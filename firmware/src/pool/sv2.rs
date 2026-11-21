//! Stratum V2 pool session, through `miner_core::sv2::Client`: the same role as the Stratum V1
//! session, on a standard or an extended channel.

use core::{convert::Infallible, fmt::Write as _};

use defmt::warn;
use embassy_futures::select::select;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Instant, Timer};
use heapless::{Deque, String};
use miner_core::{
    coinbase::coinbase_info,
    pool_config::PoolConfig,
    sv2::{self, work::Merkle, ChannelType, Client, Event},
    units::network_difficulty,
};

use super::{
    connect, SessionError, CLEAN_JOBS, IDLE_POLL, JOBS, POOL_DIFFICULTY, RECONNECT, SHARES,
    VERSION_MASK, VERSION_MASK_REQUEST,
};
use crate::{clock, status, wlog};

/// Hashrate announced while the chips haven't said theirs yet, in hashes per second.
const DEFAULT_HASHRATE: f32 = 1.0e12;
const VENDOR: &str = "bitaxe";

/// The hardware random number generator, random from the radio's noise while Wi-Fi runs.
struct HwRng(esp_hal::rng::Rng);

impl rand_core::RngCore for HwRng {
    fn next_u32(&mut self) -> u32 {
        self.0.random()
    }

    fn next_u64(&mut self) -> u64 {
        u64::from(self.0.random()) << 32 | u64::from(self.0.random())
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.read(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.0.read(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for HwRng {}

type Sv2Client<'a> = Client<TcpSocket<'a>>;

/// Runs one session, until the connection fails, the pool closes the channel or asks to
/// reconnect.
pub(super) async fn session(
    stack: Stack<'_>,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
    pool: &PoolConfig,
    authorized: &mut bool,
) -> Result<Infallible, SessionError> {
    let authority_key = match pool.sv2_authority_key.trim() {
        "" if pool.sv2_require_auth => return Err(SessionError::NoAuthorityKey),
        "" => None,
        key => Some(sv2::authority_key(key).ok_or(SessionError::AuthorityKey)?),
    };
    // the time is only checked with an authority key
    let now = match authority_key {
        Some(_) => clock::now(stack).await.ok_or(SessionError::NoTime)?,
        None => 0,
    };
    let socket = connect(stack, rx_buf, tx_buf, pool).await?;
    let mut hardware = String::<32>::new();
    let _ = write!(hardware, "{} {}", env!("BOARD_NAME"), env!("BOARD_VERSION"));
    let expected_ghs = status::with(|s| s.expected_hashrate_ghs);
    let channel = ChannelType::from_setting(&pool.sv2_channel_type);
    let config = sv2::Config {
        host: pool.host(),
        port: pool.effective_port(),
        user: &pool.user,
        authority_key,
        channel,
        nominal_hashrate: if expected_ghs > 0.0 {
            expected_ghs * 1.0e9
        } else {
            DEFAULT_HASHRATE
        },
        vendor: VENDOR,
        hardware_version: &hardware,
        firmware: concat!("esp-miner-rs ", env!("CARGO_PKG_VERSION")),
        device_id: "",
    };
    let mut rng = HwRng(esp_hal::rng::Rng::new());
    let mut client = Client::connect(socket, &config, &mut rng, now)
        .await
        .map_err(SessionError::Sv2)?;
    *authorized = true;
    let difficulty = client.difficulty();
    wlog!(
        Info,
        "pool",
        "stratum v2 {} channel {} open, difficulty {}",
        match channel {
            ChannelType::Standard => "standard",
            ChannelType::Extended => "extended",
        },
        client.channel_id(),
        difficulty
    );
    if !client.version_rolling() {
        wlog!(Warn, "pool", "the pool doesn't allow version rolling");
    }
    VERSION_MASK.lock(|m| {
        m.set(if client.version_rolling() {
            VERSION_MASK_REQUEST
        } else {
            0
        })
    });
    POOL_DIFFICULTY.lock(|d| d.set(difficulty));
    status::update(|s| {
        s.pool_connected = true;
        s.pool_difficulty = difficulty;
        s.submitted = 0;
        s.accepted = 0;
        s.rejected = 0;
        s.work_received = 0;
    });

    // when each share awaiting the pool's answer was submitted
    let mut submitted = Deque::<Instant, 16>::new();
    loop {
        if RECONNECT.signaled() {
            RECONNECT.reset();
            return Err(SessionError::Reconfigured);
        }
        let mut busy = false;
        match client.poll_message().await {
            Ok(Some(event)) => {
                busy = true;
                handle_event(&client, event, &mut submitted)?;
            }
            Ok(None) => {}
            Err(e) => return Err(SessionError::Sv2(e)),
        }
        while let Ok(share) = SHARES.try_receive() {
            busy = true;
            client.submit(&share).await.map_err(SessionError::Sv2)?;
            if submitted.is_full() {
                submitted.pop_front();
            }
            let _ = submitted.push_back(Instant::now());
            status::update(|s| s.submitted += 1);
        }
        while !JOBS.is_full() {
            let Some(job) = client.roll_job() else {
                break;
            };
            busy = true;
            let _ = JOBS.try_send(job);
        }
        if !busy {
            select(Timer::after(IDLE_POLL), SHARES.ready_to_receive()).await;
        }
    }
}

fn handle_event(
    client: &Sv2Client<'_>,
    event: Event,
    submitted: &mut Deque<Instant, 16>,
) -> Result<(), SessionError> {
    match event {
        Event::Work { clean } => {
            if clean {
                // the jobs rolled before are stale
                JOBS.clear();
                CLEAN_JOBS.signal(());
            }
            let (network, coinbase) = match client.active_job() {
                Some((job, prev)) => {
                    if !job.version_rolling {
                        warn!("job {} doesn't allow version rolling", job.id);
                    }
                    let coinbase = match &job.merkle {
                        Merkle::Coinbase { prefix, suffix, .. } => {
                            let (extranonce_prefix, size) = client.extranonce();
                            coinbase_info(prefix, suffix, extranonce_prefix.len(), size)
                        }
                        Merkle::Root(_) => None,
                    };
                    (network_difficulty(prev.nbits), coinbase)
                }
                None => (0.0, None),
            };
            status::update(|s| {
                s.work_received += 1;
                s.network_difficulty = network;
                s.coinbase = coinbase;
            });
        }
        Event::Difficulty(difficulty) => {
            wlog!(Info, "pool", "difficulty {}", difficulty);
            POOL_DIFFICULTY.lock(|d| d.set(difficulty));
            status::update(|s| s.pool_difficulty = difficulty);
        }
        Event::Accepted(count) => {
            let mut response_ms = None;
            for _ in 0..count {
                if let Some(t) = submitted.pop_front() {
                    response_ms = Some(t.elapsed().as_micros() as f32 / 1000.0);
                }
            }
            wlog!(Info, "pool", "{} share(s) accepted", count);
            status::update(|s| {
                s.accepted += u64::from(count);
                if let Some(ms) = response_ms {
                    s.response_time_ms = Some(ms);
                    s.response_time_min_ms = Some(s.response_time_min_ms.map_or(ms, |m| m.min(ms)));
                    s.response_time_max_ms = Some(s.response_time_max_ms.map_or(ms, |m| m.max(ms)));
                }
            });
        }
        Event::Rejected(code) => {
            submitted.pop_front();
            wlog!(Warn, "pool", "share rejected: {}", code.as_str());
            status::update(|s| s.rejected += 1);
        }
        Event::Reconnect => return Err(SessionError::Reconnect),
    }
    Ok(())
}
