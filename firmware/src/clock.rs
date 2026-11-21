//! Wall-clock time from an SNTP server, which a Stratum V2 handshake checks the pool's
//! certificate against.

use core::cell::Cell;

use embassy_net::{
    dns::DnsQueryType,
    udp::{PacketMetadata, UdpSocket},
    IpAddress, Stack,
};
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::{with_timeout, Duration, Instant};
use miner_core::sntp;

use crate::wlog;

/// The NTP servers asked.
const SERVER: &str = "pool.ntp.org";
const ATTEMPTS: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(3);
/// A synchronization is used this long before the server is asked again.
const VALID: Duration = Duration::from_secs(6 * 3600);

/// The Unix time at an instant, from the last synchronization.
static SYNC: Mutex<CriticalSectionRawMutex, Cell<Option<(u32, Instant)>>> =
    Mutex::new(Cell::new(None));

/// The Unix time, if it was ever synchronized.
pub fn unix_time() -> Option<u32> {
    let (time, at) = SYNC.lock(Cell::get)?;
    Some(time.wrapping_add(at.elapsed().as_secs() as u32))
}

/// The Unix time, from a recent synchronization or else from the server; an old
/// synchronization when the server doesn't answer.
pub async fn now(stack: Stack<'_>) -> Option<u32> {
    if SYNC
        .lock(Cell::get)
        .is_some_and(|(_, at)| at.elapsed() < VALID)
    {
        return unix_time();
    }
    for _ in 0..ATTEMPTS {
        if let Some(time) = query(stack).await {
            SYNC.lock(|s| s.set(Some((time, Instant::now()))));
            wlog!(Info, "time", "synchronized with {}: {}", SERVER, time);
            return Some(time);
        }
    }
    wlog!(Warn, "time", "no answer from {}", SERVER);
    unix_time()
}

async fn query(stack: Stack<'_>) -> Option<u32> {
    let address = stack
        .dns_query(SERVER, DnsQueryType::A)
        .await
        .ok()?
        .iter()
        .find_map(|a| match a {
            IpAddress::Ipv4(a) => Some(*a),
            #[allow(unreachable_patterns)]
            _ => None,
        })?;
    let mut rx_meta = [PacketMetadata::EMPTY; 1];
    let mut tx_meta = [PacketMetadata::EMPTY; 1];
    let mut rx_buf = [0; sntp::PACKET_LEN];
    let mut tx_buf = [0; sntp::PACKET_LEN];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(0).ok()?;
    socket
        .send_to(&sntp::request(), (address, sntp::PORT))
        .await
        .ok()?;
    let mut answer = [0; sntp::PACKET_LEN];
    let (n, _) = with_timeout(TIMEOUT, socket.recv_from(&mut answer))
        .await
        .ok()?
        .ok()?;
    sntp::unix_time(&answer[..n])
}
