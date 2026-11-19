//! Stratum v1 pool session: owns the client, hands jobs to the ASIC task and submits its shares.

use core::{cell::Cell, convert::Infallible, net::Ipv4Addr, str::FromStr};

use defmt::{info, warn};
use embassy_futures::select::select;
use embassy_net::{
    dns::{DnsQueryType, Error as DnsError},
    tcp::{ConnectError, TcpSocket},
    IpAddress, Stack,
};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, Mutex},
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use heapless::{Deque, String};
use miner_core::{
    coinbase::coinbase_info, job::MiningJob, pool_config::PoolConfig, units::network_difficulty,
};
use stratum_v1::{Client, Extensions, Message, Share, VersionRolling};

use crate::{board, settings, status, wlog};

/// Jobs rolled from the pool work, ready for the chips.
pub static JOBS: Channel<CriticalSectionRawMutex, MiningJob, 2> = Channel::new();
/// Shares found by the chips, to submit.
pub static SHARES: Channel<CriticalSectionRawMutex, Share, 8> = Channel::new();
/// Signaled when the pool settings change: the session ends and the pool is chosen again.
pub static RECONNECT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Signaled when the pool asks to drop the current work.
pub static CLEAN_JOBS: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Share difficulty set by the pool.
pub static POOL_DIFFICULTY: Mutex<CriticalSectionRawMutex, Cell<f64>> =
    Mutex::new(Cell::new(board::ASIC_DIFFICULTY as f64));
/// Version rolling mask allowed by the pool.
pub static VERSION_MASK: Mutex<CriticalSectionRawMutex, Cell<u32>> =
    Mutex::new(Cell::new(VERSION_MASK_REQUEST));

/// Version rolling mask asked to the pool, and enabled in the chips.
pub const VERSION_MASK_REQUEST: u32 = 0x1fff_e000;

/// The pool of the default settings.
pub const DEFAULT_URL: &str = "public-pool.io";
pub const DEFAULT_PORT: u16 = 21496;
pub const DEFAULT_USER: &str = "1HLQGxzAQWnLore3fWHc2W8UP1CgMv1GKQ.miner1";
pub const DEFAULT_PASSWORD: &str = "x";
pub const DEFAULT_DIFFICULTY: u32 = 1000;
const CLIENT_NAME: &str = "esp-miner-rs";
/// Sessions in a row that end before the authorization, after which the fallback pool is used.
const FAILURES_BEFORE_FALLBACK: u32 = 3;

/// How long to wait for the pool when there is nothing to do.
const IDLE_POLL: Duration = Duration::from_millis(10);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
/// A silent pool connection is dropped after this long.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(300);
const SOCKET_KEEP_ALIVE: Duration = Duration::from_secs(60);

type StratumClient<'a> = Client<TcpSocket<'a>, 1480, 512>;

enum SessionError {
    Dns(DnsError),
    NoAddress,
    Connect(ConnectError),
    Stratum(stratum_v1::Error),
    /// The user or password is longer than the client takes.
    Credentials,
    /// The pool settings changed.
    Reconfigured,
}

impl defmt::Format for SessionError {
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            Self::Dns(e) => defmt::write!(fmt, "DNS error: {}", e),
            Self::NoAddress => defmt::write!(fmt, "no IPv4 address for the pool"),
            Self::Connect(e) => defmt::write!(fmt, "connect error: {}", e),
            Self::Credentials => defmt::write!(fmt, "user or password too long"),
            Self::Reconfigured => defmt::write!(fmt, "pool settings changed"),
            Self::Stratum(e) => defmt::write!(fmt, "stratum error: {}", e),
        }
    }
}

impl core::fmt::Display for SessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::Dns(e) => write!(f, "DNS error: {e:?}"),
            Self::NoAddress => f.write_str("no IPv4 address for the pool"),
            Self::Connect(e) => write!(f, "connect error: {e:?}"),
            Self::Stratum(e) => write!(f, "stratum error: {e:?}"),
            Self::Credentials => f.write_str("user or password too long"),
            Self::Reconfigured => f.write_str("pool settings changed"),
        }
    }
}

impl From<stratum_v1::Error> for SessionError {
    fn from(e: stratum_v1::Error) -> Self {
        Self::Stratum(e)
    }
}

/// The pool to use, and whether it is the fallback: the primary, or the fallback after
/// [`FAILURES_BEFORE_FALLBACK`] failed sessions in a row on the primary when the fallback is
/// enabled.
fn choose_pool(failures: u32) -> Option<(PoolConfig, bool)> {
    settings::with(|s| {
        let primary = s.pools.get(usize::from(s.primary_pool_index));
        let secondary = s.pools.get(usize::from(s.secondary_pool_index));
        let fallback = s.use_fallback_stratum
            && failures >= FAILURES_BEFORE_FALLBACK
            && s.secondary_pool_index != s.primary_pool_index;
        if fallback {
            if let Some(pool) = secondary.and_then(|p| PoolConfig::parse(p)) {
                return Some((pool, true));
            }
        }
        primary
            .and_then(|p| PoolConfig::parse(p))
            .map(|p| (p, false))
    })
}

/// Keeps a session with the pool of the settings open, reconnecting after any error. After a
/// session on the fallback pool, the primary is tried again.
#[embassy_executor::task]
pub async fn pool_task(
    stack: Stack<'static>,
    rx_buf: &'static mut [u8; 1536],
    tx_buf: &'static mut [u8; 1536],
) {
    let mut failures = 0;
    loop {
        stack.wait_config_up().await;
        RECONNECT.reset();
        let Some((pool, fallback)) = choose_pool(failures) else {
            wlog!(Error, "pool", "no pool configured");
            Timer::after(RECONNECT_DELAY).await;
            continue;
        };
        status::update(|s| {
            s.pool_url = String::try_from(pool.host()).unwrap_or_default();
            s.pool_port = pool.effective_port();
            s.pool_user = String::try_from(pool.user.as_str()).unwrap_or_default();
            s.using_fallback = fallback;
            if fallback {
                status::set_str(&mut s.primary_pool_error, "Pool unreachable");
            } else {
                s.primary_pool_error.clear();
            }
        });
        let mut authorized = false;
        let Err(e) = session(
            stack,
            &mut rx_buf[..],
            &mut tx_buf[..],
            &pool,
            &mut authorized,
        )
        .await;
        wlog!(Error, "pool", "session ended: {}", e);
        status::update(|s| s.pool_connected = false);
        if fallback || authorized {
            failures = 0;
        } else {
            failures += 1;
        }
        // jobs and shares of the old session are stale
        JOBS.clear();
        SHARES.clear();
        CLEAN_JOBS.signal(());
        Timer::after(RECONNECT_DELAY).await;
    }
}

/// Runs one session, until the connection fails or the client can't send. A pool message the
/// client can't parse is dropped, and the session goes on.
async fn session(
    stack: Stack<'_>,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
    pool: &PoolConfig,
    authorized: &mut bool,
) -> Result<Infallible, SessionError> {
    let user = String::<64>::try_from(pool.user.as_str()).map_err(|_| SessionError::Credentials)?;
    let password =
        String::<64>::try_from(pool.password.as_str()).map_err(|_| SessionError::Credentials)?;
    let address = match Ipv4Addr::from_str(pool.host()) {
        Ok(address) => address,
        Err(_) => stack
            .dns_query(pool.host(), DnsQueryType::A)
            .await
            .map_err(SessionError::Dns)?
            .iter()
            .find_map(|a| match a {
                IpAddress::Ipv4(a) => Some(*a),
                #[allow(unreachable_patterns)]
                _ => None,
            })
            .ok_or(SessionError::NoAddress)?,
    };
    let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
    socket.set_timeout(Some(SOCKET_TIMEOUT));
    socket.set_keep_alive(Some(SOCKET_KEEP_ALIVE));
    wlog!(
        Info,
        "pool",
        "connecting to {}:{}",
        pool.host(),
        pool.effective_port()
    );
    socket
        .connect((address, pool.effective_port()))
        .await
        .map_err(SessionError::Connect)?;
    info!("connected!");

    let mut client = StratumClient::new(socket);
    client.enable_software_rolling(false, true, false);
    client
        .send_configure(Extensions {
            version_rolling: Some(VersionRolling {
                mask: Some(VERSION_MASK_REQUEST),
                min_bit_count: Some(16),
            }),
            minimum_difficulty: (pool.suggested_difficulty > 0)
                .then_some(pool.suggested_difficulty),
            subscribe_extranonce: pool.extranonce_subscribe.then_some(()),
            info: None,
        })
        .await?;

    let credentials = (user, password);
    // when each share awaiting the pool's answer was submitted
    let mut submitted = Deque::<Instant, 16>::new();
    loop {
        if RECONNECT.signaled() {
            RECONNECT.reset();
            return Err(SessionError::Reconfigured);
        }
        let mut busy = false;
        match client.poll_message().await {
            Ok(Some(msg)) => {
                busy = true;
                handle_message(&mut client, msg, authorized, &credentials, &mut submitted).await?;
            }
            Ok(None) => {}
            Err(stratum_v1::Error::Network) => return Err(stratum_v1::Error::Network.into()),
            Err(e) => {
                busy = true;
                warn!("pool message dropped: {}", e);
            }
        }
        if *authorized {
            while let Ok(share) = SHARES.try_receive() {
                busy = true;
                client.send_submit(share).await?;
                if submitted.is_full() {
                    submitted.pop_front();
                }
                let _ = submitted.push_back(Instant::now());
                status::update(|s| s.submitted += 1);
            }
            while !JOBS.is_full() {
                match client.roll_job().await {
                    Ok(job) => {
                        busy = true;
                        let _ = JOBS.try_send(MiningJob::from_stratum_v1(
                            job.job_id,
                            job.extranonce2,
                            job.header.version,
                            job.header.prev_blockhash,
                            job.header.merkle_root,
                            job.header.ntime,
                            job.header.nbits,
                        ));
                    }
                    // no work received yet
                    Err(stratum_v1::Error::NoWork) => break,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if !busy {
            if *authorized {
                select(Timer::after(IDLE_POLL), SHARES.ready_to_receive()).await;
            } else {
                // shares wait for the authorization: waiting on them would not yield
                Timer::after(IDLE_POLL).await;
            }
        }
    }
}

async fn handle_message(
    client: &mut StratumClient<'_>,
    msg: Message,
    authorized: &mut bool,
    credentials: &(String<64>, String<64>),
    submitted: &mut Deque<Instant, 16>,
) -> Result<(), SessionError> {
    match msg {
        Message::Configured => {
            client
                .send_connect(Some(String::from_str(CLIENT_NAME).unwrap()))
                .await?
        }
        Message::Connected => {
            client
                .send_authorize(credentials.0.clone(), credentials.1.clone())
                .await?
        }
        Message::Authorized => {
            *authorized = true;
            wlog!(Info, "pool", "authorized");
            status::update(|s| {
                s.pool_connected = true;
                s.submitted = 0;
                s.accepted = 0;
                s.rejected = 0;
                s.work_received = 0;
            });
        }
        Message::Share { accepted, rejected } => {
            let response_ms = submitted
                .pop_front()
                .map(|t| t.elapsed().as_micros() as f32 / 1000.0);
            let was_rejected = status::with(|s| rejected > s.rejected);
            if was_rejected {
                wlog!(
                    Warn,
                    "pool",
                    "share rejected ({} accepted, {} rejected)",
                    accepted,
                    rejected
                );
            } else {
                wlog!(
                    Info,
                    "pool",
                    "share accepted ({} accepted, {} rejected)",
                    accepted,
                    rejected
                );
            }
            status::update(|s| {
                s.accepted = accepted;
                s.rejected = rejected;
                if let Some(ms) = response_ms {
                    s.response_time_ms = Some(ms);
                    s.response_time_min_ms = Some(s.response_time_min_ms.map_or(ms, |m| m.min(ms)));
                    s.response_time_max_ms = Some(s.response_time_max_ms.map_or(ms, |m| m.max(ms)));
                }
            });
        }
        Message::VersionMask(mask) => VERSION_MASK.lock(|m| m.set(mask)),
        Message::Difficulty(diff) => {
            wlog!(Info, "pool", "difficulty {}", diff);
            POOL_DIFFICULTY.lock(|d| d.set(diff));
            status::update(|s| s.pool_difficulty = diff);
        }
        Message::Work { clean_jobs } => {
            if clean_jobs {
                // the new work is already in the client: the jobs rolled next use it
                JOBS.clear();
                CLEAN_JOBS.signal(());
            }
            let difficulty = client
                .work()
                .map_or(0.0, |work| network_difficulty(work.nbits));
            let coinbase = client.work().and_then(|work| {
                coinbase_info(
                    &work.coinb1,
                    &work.coinb2,
                    client.extranonce1().len(),
                    client.extranonce2_size(),
                )
            });
            status::update(|s| {
                s.work_received += 1;
                s.network_difficulty = difficulty;
                s.coinbase = coinbase;
            });
        }
    }
    Ok(())
}
