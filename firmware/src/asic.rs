//! ASIC chain: brings the chips up with bm13xx-chain, feeds them jobs and checks their nonces.

use core::{cell::Cell, convert::Infallible, fmt::Write};

use bm13xx_chain::{Baud, Chain};
use bm13xx_protocol::response::ResponseType;
use defmt::{debug, error, info, warn};
use embassy_time::{Delay, Duration, Instant, Timer};
use esp_hal::{
    gpio::Output,
    uart::{self, Uart},
    Async,
};
use fugit::HertzU64;
use heapless::String;
use miner_core::{
    hashrate::HashrateMeter,
    job::{JobBook, MiningJob},
    scoreboard::Entry,
    share::check_nonce,
    timing::job_interval_ms,
};
use stratum_v1::Share;

use crate::{
    board,
    pool::{CLEAN_JOBS, JOBS, POOL_DIFFICULTY, SHARES, VERSION_MASK, VERSION_MASK_REQUEST},
    power::{self, POWER_READY, RESTART_CHAIN},
    settings, stats, status, wlog,
};

/// How often to poll the chain for responses.
const POLL_PERIOD: Duration = Duration::from_millis(5);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const RESET_DURATION: Duration = Duration::from_millis(100);
/// Baudrate of the chips after a reset.
const DEFAULT_BAUDRATE: u32 = 115_200;
/// Enough for the 16 or 32 job ids the chain cycles through.
const JOB_IDS: usize = 32;
/// The hashrate is measured over 1 minute (6 buckets of 10 s), 10 minutes (60 of 10 s) and 1
/// hour (60 of 1 minute).
const BUCKET_10S_MS: u64 = 10_000;
const BUCKET_1MIN_MS: u64 = 60_000;
/// Hashes behind a nonce at difficulty 1.
const HASHES_PER_DIFFICULTY: f64 = 4_294_967_296.0;
/// How often to publish the hashrate.
const STATUS_PERIOD: Duration = Duration::from_secs(1);

type UartError = <Uart<'static, Async> as embedded_io::ErrorType>::Error;
type ChainError = bm13xx_chain::Error<UartError, Infallible, Infallible>;

/// The serial link to the chips, with the baudrate control bm13xx-chain needs.
pub struct AsicUart {
    uart: Uart<'static, Async>,
    config: uart::Config,
}

impl AsicUart {
    /// `config` must be the configuration `uart` was created with.
    pub fn new(uart: Uart<'static, Async>, config: uart::Config) -> Self {
        Self { uart, config }
    }

    fn set_baudrate(&mut self, baudrate: u32) {
        self.config = self.config.with_baudrate(baudrate);
        if let Err(e) = self.uart.apply_config(&self.config) {
            error!("setting the ASIC baudrate to {} failed: {}", baudrate, e);
        }
    }
}

impl embedded_io::ErrorType for AsicUart {
    type Error = UartError;
}

impl embedded_io_async::Read for AsicUart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        embedded_io_async::Read::read(&mut self.uart, buf).await
    }
}

impl embedded_io::ReadReady for AsicUart {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        embedded_io::ReadReady::read_ready(&mut self.uart)
    }
}

impl embedded_io_async::Write for AsicUart {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        embedded_io_async::Write::write(&mut self.uart, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        embedded_io_async::Write::flush(&mut self.uart).await
    }
}

impl Baud for &mut AsicUart {
    fn set_baudrate(&mut self, baudrate: u32) {
        AsicUart::set_baudrate(self, baudrate);
    }
}

/// The chips' BI (busy) input isn't driven on the Bitaxe.
struct NoBusy;

impl embedded_hal::digital::ErrorType for NoBusy {
    type Error = Infallible;
}

impl embedded_hal::digital::OutputPin for NoBusy {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Brings the chain up each time the core power comes up, then mines until mining stops. Starts
/// over after any chain error, and when the frequency changes. The chips are held in reset while
/// not mining.
///
/// `reset` is the chips' NRST, active low.
#[embassy_executor::task]
pub async fn asic_task(mut uart: AsicUart, mut reset: Output<'static>) {
    loop {
        POWER_READY.wait().await;
        while power::mining() {
            RESTART_CHAIN.reset();
            if let Err(e) = run_chain(&mut uart, &mut reset).await {
                wlog!(Error, "asic", "chain error: {:?}", e);
                Timer::after(RETRY_DELAY).await;
            }
        }
        reset.set_low();
        status::update(|s| {
            s.hashrate_ghs = 0.0;
            s.hashrate_1m = 0.0;
            s.expected_hashrate_ghs = 0.0;
        });
    }
}

async fn run_chain(uart: &mut AsicUart, reset: &mut Output<'static>) -> Result<(), ChainError> {
    let frequency = settings::with(|s| s.frequency);
    // Start from reset chips at their default baudrate, also after a chain error.
    // `enumerate` releases the reset.
    reset.set_low();
    uart.set_baudrate(DEFAULT_BAUDRATE);
    Timer::after(RESET_DURATION).await;
    let mut chain = Chain::enumerate(board::Asic::default(), uart, NoBusy, reset, Delay).await?;
    wlog!(Info, "asic", "{} ASIC(s) found", chain.asic_cnt);
    chain.set_domain_cnt(board::VOLTAGE_DOMAINS);
    chain.init(board::ASIC_DIFFICULTY).await?;
    chain.change_baudrate(board::ASIC_BAUDRATE).await?;
    chain.reset_all_cores().await?;
    chain.set_hash_freq(HertzU64::MHz(frequency as u64)).await?;
    if board::HW_VERSION_ROLLING {
        chain.enable_version_rolling(VERSION_MASK_REQUEST).await?;
    } else {
        warn!("bm13xx-chain doesn't compute midstates yet: the chips will hash wrong jobs");
    }
    let job_interval = Duration::from_millis(job_interval_ms(
        chain.theoretical_hashrate_ghs(),
        board::ROLLED_VERSIONS,
    ) as u64);
    wlog!(
        Info,
        "asic",
        "mining at {} MHz, {} GH/s expected, a new job every {} ms",
        frequency,
        chain.theoretical_hashrate_ghs(),
        job_interval.as_millis()
    );
    let expected = chain.theoretical_hashrate_ghs() as f32;
    let asics = chain.asic_cnt as u8;
    status::update(|s| {
        s.expected_hashrate_ghs = expected;
        s.asic_count = asics;
        s.frequency_mhz = frequency;
    });

    let mut book = JobBook::<JOB_IDS>::new();
    let mut next_job = Instant::now();
    let now = Instant::now().as_millis();
    let mut meter_1m = HashrateMeter::<6>::new(now, BUCKET_10S_MS);
    let mut meter_10m = HashrateMeter::<60>::new(now, BUCKET_10S_MS);
    let mut meter_1h = HashrateMeter::<60>::new(now, BUCKET_1MIN_MS);
    let mut invalid_10m = HashrateMeter::<60>::new(now, BUCKET_10S_MS);
    let mut next_status = Instant::now();
    loop {
        if !power::mining() || RESTART_CHAIN.signaled() {
            return Ok(());
        }
        while let Some(resp) = chain.poll_response().await? {
            let now = Instant::now().as_millis();
            match handle_response(&book, resp) {
                Some(Ok(difficulty)) => {
                    meter_1m.record(now);
                    meter_10m.record(now);
                    meter_1h.record(now);
                    status::update(|s| {
                        s.best_difficulty = s.best_difficulty.max(difficulty);
                        s.best_difficulty_ever = s.best_difficulty_ever.max(difficulty);
                        s.total_hashes += board::ASIC_DIFFICULTY as f64 * HASHES_PER_DIFFICULTY;
                        if s.network_difficulty > 0.0 && difficulty >= s.network_difficulty {
                            s.block_found += 1;
                            s.show_new_block = true;
                        }
                    });
                }
                Some(Err(())) => invalid_10m.record(now),
                None => {}
            }
        }
        if Instant::now() >= next_status {
            let now = Instant::now().as_millis();
            let d = board::ASIC_DIFFICULTY;
            let (h1, h10, h60) = (
                meter_1m.ghs(now, d),
                meter_10m.ghs(now, d),
                meter_1h.ghs(now, d),
            );
            let invalid = invalid_10m.ghs(now, d);
            let errors = if h10 + invalid > 0.0 {
                invalid / (h10 + invalid) * 100.0
            } else {
                0.0
            };
            status::update(|s| {
                s.hashrate_ghs = h10 as f32;
                s.hashrate_1m = h1 as f32;
                s.hashrate_10m = h10 as f32;
                s.hashrate_1h = h60 as f32;
                s.error_percentage = errors as f32;
            });
            next_status = Instant::now() + STATUS_PERIOD;
        }
        if CLEAN_JOBS.signaled() {
            CLEAN_JOBS.reset();
            book.clear();
            next_job = Instant::now();
        }
        if Instant::now() >= next_job {
            if let Ok(job) = JOBS.try_receive() {
                let id = chain
                    .send_job(
                        job.version,
                        job.prev_blockhash_display(),
                        job.merkle_root_display(),
                        job.nbits,
                        job.ntime,
                    )
                    .await?;
                book.insert(id, job);
                next_job = Instant::now() + job_interval;
            }
        }
        Timer::after(POLL_PERIOD).await;
    }
}

/// Checks a nonce returned by a chip and queues it as a share if the pool wants it. Returns the
/// nonce difficulty when it is valid, an error when it is not, `None` for other responses.
fn handle_response(book: &JobBook<JOB_IDS>, resp: ResponseType) -> Option<Result<f64, ()>> {
    let (job_id, nonce, version_bits) = match resp {
        ResponseType::JobVer(r) => (r.job_id, r.nonce, Some(r.version_bit)),
        ResponseType::Job(r) => (r.job_id, r.nonce, None),
        ResponseType::Reg(r) => {
            debug!("register response: {}", r);
            return None;
        }
    };
    let Some(job) = book.get(job_id as u8) else {
        warn!("nonce {:#010x} for unknown job id {}", nonce, job_id);
        return None;
    };
    let checked = check_nonce(job, nonce, version_bits, VERSION_MASK.lock(Cell::get));
    let pool_difficulty = POOL_DIFFICULTY.lock(Cell::get);
    if checked.difficulty < board::ASIC_DIFFICULTY as f64 {
        // the chips only report nonces above their ticket mask
        warn!(
            "invalid nonce {:#010x} (difficulty {}): wrong job data or hardware error",
            nonce, checked.difficulty
        );
        return Some(Err(()));
    }
    stats::score(score_entry(
        job,
        nonce,
        checked.difficulty,
        checked.version_bits,
    ));
    info!(
        "nonce {:#010x}: difficulty {} (pool {})",
        nonce, checked.difficulty, pool_difficulty
    );
    if checked.difficulty >= pool_difficulty {
        let share = Share {
            job_id: String::try_from(job.pool_job_id.as_str()).unwrap(),
            extranonce2: job.extranonce2.clone(),
            ntime: job.ntime,
            nonce,
            version_bits: checked.version_bits,
        };
        if SHARES.try_send(share).is_err() {
            warn!("share queue full, dropping a share");
        }
    }
    Some(Ok(checked.difficulty))
}

fn score_entry(job: &MiningJob, nonce: u32, difficulty: f64, version_bits: Option<u32>) -> Entry {
    let mut extranonce2 = String::new();
    for b in &job.extranonce2 {
        let _ = write!(extranonce2, "{b:02x}");
    }
    Entry {
        difficulty,
        job_id: String::try_from(job.pool_job_id.as_str()).unwrap_or_default(),
        extranonce2,
        ntime: job.ntime,
        nonce,
        version_bits: version_bits.unwrap_or(0),
    }
}
