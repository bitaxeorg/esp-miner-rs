//! ASIC chain: brings the chips up with bm13xx-chain, feeds them jobs and checks their nonces.

use core::{cell::Cell, convert::Infallible};

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
use miner_core::{job::JobBook, share::check_nonce, timing::job_interval_ms};
use stratum_v1::Share;

use crate::{
    board,
    pool::{CLEAN_JOBS, JOBS, POOL_DIFFICULTY, SHARES, VERSION_MASK, VERSION_MASK_REQUEST},
    power::POWER_READY,
};

/// How often to poll the chain for responses.
const POLL_PERIOD: Duration = Duration::from_millis(5);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const RESET_DURATION: Duration = Duration::from_millis(100);
/// Baudrate of the chips after a reset.
const DEFAULT_BAUDRATE: u32 = 115_200;
/// Enough for the 16 or 32 job ids the chain cycles through.
const JOB_IDS: usize = 32;

type UartError = <Uart<'static, Async> as embedded_io_06::ErrorType>::Error;
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

impl embedded_io_06::ErrorType for AsicUart {
    type Error = UartError;
}

impl embedded_io_async_06::Read for AsicUart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        embedded_io_async_06::Read::read(&mut self.uart, buf).await
    }
}

impl embedded_io_06::ReadReady for AsicUart {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        embedded_io_06::ReadReady::read_ready(&mut self.uart)
    }
}

impl embedded_io_async_06::Write for AsicUart {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        embedded_io_async_06::Write::write(&mut self.uart, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        embedded_io_async_06::Write::flush(&mut self.uart).await
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

/// Brings the chain up once the core power is ready, then mines. Starts over after any chain
/// error.
///
/// `reset` is the chips' NRST, active low.
#[embassy_executor::task]
pub async fn asic_task(mut uart: AsicUart, mut reset: Output<'static>) {
    POWER_READY.wait().await;
    loop {
        let Err(e) = run_chain(&mut uart, &mut reset).await;
        error!("ASIC chain error: {}", e);
        Timer::after(RETRY_DELAY).await;
    }
}

async fn run_chain(
    uart: &mut AsicUart,
    reset: &mut Output<'static>,
) -> Result<Infallible, ChainError> {
    // Start from reset chips at their default baudrate, also after a chain error.
    // `enumerate` releases the reset.
    reset.set_low();
    uart.set_baudrate(DEFAULT_BAUDRATE);
    Timer::after(RESET_DURATION).await;
    let mut chain = Chain::enumerate(board::Asic::default(), uart, NoBusy, reset, Delay).await?;
    info!("enumerated {} ASIC(s)", chain.asic_cnt);
    chain.init(board::ASIC_DIFFICULTY).await?;
    chain.change_baudrate(board::ASIC_BAUDRATE).await?;
    chain.reset_all_cores().await?;
    chain
        .set_hash_freq(HertzU64::MHz(board::HASH_FREQ_MHZ))
        .await?;
    if board::HW_VERSION_ROLLING {
        chain.enable_version_rolling(VERSION_MASK_REQUEST).await?;
    } else {
        warn!("bm13xx-chain doesn't compute midstates yet: the chips will hash wrong jobs");
    }
    let job_interval = Duration::from_millis(job_interval_ms(
        chain.theoretical_hashrate_ghs(),
        board::ROLLED_VERSIONS,
    ) as u64);
    info!(
        "mining at {} GH/s (theoretical), a new job every {} ms",
        chain.theoretical_hashrate_ghs(),
        job_interval.as_millis()
    );

    let mut book = JobBook::<JOB_IDS>::new();
    let mut next_job = Instant::now();
    loop {
        while let Some(resp) = chain.poll_response().await? {
            handle_response(&book, resp);
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

/// Checks a nonce returned by a chip and queues it as a share if the pool wants it.
fn handle_response(book: &JobBook<JOB_IDS>, resp: ResponseType) {
    let (job_id, nonce, version_bits) = match resp {
        ResponseType::JobVer(r) => (r.job_id, r.nonce, Some(r.version_bit)),
        ResponseType::Job(r) => (r.job_id, r.nonce, None),
        ResponseType::Reg(r) => {
            debug!("register response: {}", r);
            return;
        }
    };
    let Some(job) = book.get(job_id as u8) else {
        warn!("nonce {:#010x} for unknown job id {}", nonce, job_id);
        return;
    };
    let checked = check_nonce(job, nonce, version_bits, VERSION_MASK.lock(Cell::get));
    let pool_difficulty = POOL_DIFFICULTY.lock(Cell::get);
    if checked.difficulty < board::ASIC_DIFFICULTY as f64 {
        // the chips only report nonces above their ticket mask
        warn!(
            "invalid nonce {:#010x} (difficulty {}): wrong job data or hardware error",
            nonce, checked.difficulty
        );
        return;
    }
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
}
