//! Stratum v1 pool session: owns the client, hands jobs to the ASIC task and submits its shares.

use core::{cell::Cell, convert::Infallible, net::Ipv4Addr, str::FromStr};

use defmt::{error, info, Debug2Format};
use embassy_futures::select::select;
use embassy_net::{
    tcp::{ConnectError, TcpSocket},
    Stack,
};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, Mutex},
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use heapless::String;
use miner_core::job::MiningJob;
use stratum_v1::{Client, Extensions, Message, Share, VersionRolling};

use crate::{board, io06::Io06};

/// Jobs rolled from the pool work, ready for the chips.
pub static JOBS: Channel<CriticalSectionRawMutex, MiningJob, 2> = Channel::new();
/// Shares found by the chips, to submit.
pub static SHARES: Channel<CriticalSectionRawMutex, Share, 8> = Channel::new();
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

const POOL_ADDR: (Ipv4Addr, u16) = (Ipv4Addr::new(68, 235, 52, 36), 21496); // public-pool
const POOL_USER: &str = "1HLQGxzAQWnLore3fWHc2W8UP1CgMv1GKQ.miner1";
const POOL_PASS: &str = "x";
const CLIENT_NAME: &str = "esp-miner-rs";
const MINIMUM_DIFFICULTY: u32 = 256;

/// How long to wait for the pool when there is nothing to do.
const IDLE_POLL: Duration = Duration::from_millis(10);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
/// A silent pool connection is dropped after this long.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(300);
const SOCKET_KEEP_ALIVE: Duration = Duration::from_secs(60);

type StratumClient<'a> = Client<Io06<TcpSocket<'a>>, 1480, 512>;

enum SessionError {
    Connect(ConnectError),
    Stratum(stratum_v1::Error),
}

impl defmt::Format for SessionError {
    fn format(&self, fmt: defmt::Formatter) {
        match self {
            Self::Connect(e) => defmt::write!(fmt, "connect error: {}", e),
            // stratum-v1 0.1 can't be built with defmt
            Self::Stratum(e) => defmt::write!(fmt, "stratum error: {}", Debug2Format(e)),
        }
    }
}

impl From<stratum_v1::Error> for SessionError {
    fn from(e: stratum_v1::Error) -> Self {
        Self::Stratum(e)
    }
}

/// Keeps a session with the pool open, reconnecting after any error.
#[embassy_executor::task]
pub async fn pool_task(
    stack: Stack<'static>,
    rx_buf: &'static mut [u8; 1536],
    tx_buf: &'static mut [u8; 1536],
) {
    loop {
        stack.wait_config_up().await;
        let Err(e) = session(stack, &mut rx_buf[..], &mut tx_buf[..]).await;
        error!("pool session ended: {}", e);
        // jobs and shares of the old session are stale
        JOBS.clear();
        SHARES.clear();
        CLEAN_JOBS.signal(());
        Timer::after(RECONNECT_DELAY).await;
    }
}

/// Runs one session. stratum-v1 0.1 leaves a line it failed to parse in its buffer, so any error
/// ends the session.
async fn session(
    stack: Stack<'_>,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
) -> Result<Infallible, SessionError> {
    let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
    socket.set_timeout(Some(SOCKET_TIMEOUT));
    socket.set_keep_alive(Some(SOCKET_KEEP_ALIVE));
    info!("connecting to pool...");
    socket
        .connect(POOL_ADDR)
        .await
        .map_err(SessionError::Connect)?;
    info!("connected!");

    let mut client = StratumClient::new(Io06(socket));
    client.enable_software_rolling(false, true, false);
    client
        .send_configure(Extensions {
            version_rolling: Some(VersionRolling {
                mask: Some(VERSION_MASK_REQUEST),
                min_bit_count: Some(16),
            }),
            minimum_difficulty: Some(MINIMUM_DIFFICULTY),
            subscribe_extranonce: None,
            info: None,
        })
        .await?;

    let mut authorized = false;
    loop {
        let mut busy = false;
        if let Some(msg) = client.poll_message().await? {
            busy = true;
            handle_message(&mut client, msg, &mut authorized).await?;
        }
        if authorized {
            while let Ok(share) = SHARES.try_receive() {
                busy = true;
                client.send_submit(share).await?;
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
            if authorized {
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
) -> Result<(), SessionError> {
    match msg {
        Message::Configured => {
            client
                .send_connect(Some(String::from_str(CLIENT_NAME).unwrap()))
                .await?
        }
        Message::Connected => {
            client
                .send_authorize(
                    String::from_str(POOL_USER).unwrap(),
                    String::from_str(POOL_PASS).unwrap(),
                )
                .await?
        }
        Message::Authorized => *authorized = true,
        Message::Share { accepted, rejected } => {
            info!("shares accepted: {}, rejected: {}", accepted, rejected)
        }
        Message::VersionMask(mask) => VERSION_MASK.lock(|m| m.set(mask)),
        Message::Difficulty(diff) => POOL_DIFFICULTY.lock(|d| d.set(diff)),
        Message::CleanJobs => {
            // the new work is already in the client: the jobs rolled next use it
            JOBS.clear();
            CLEAN_JOBS.signal(());
        }
    }
    Ok(())
}
