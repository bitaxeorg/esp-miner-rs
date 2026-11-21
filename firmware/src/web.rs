//! The web interface: the JSON API of the stock firmware, and the htmx pages.

mod api;
mod assets;
mod http;
mod pages;
mod patch;
mod routes;

use core::fmt::Write;

use defmt::{debug, warn};
use embassy_net::{tcp::TcpSocket, IpAddress, Ipv4Cidr, Stack};
use embassy_time::Duration;
use miner_core::web::is_private_ipv4;
use picoserve::{AppBuilder, AppRouter, Router};

pub use api::restart_task;

/// Connections served at once on the station, and on the setup access point.
pub const STATION_TASKS: usize = 3;
pub const AP_TASKS: usize = 1;
const PORT: u16 = 80;
const TCP_BUFFER: usize = 1024;
const HTTP_BUFFER: usize = 2048;

/// The station's IPv4 address, once it has one.
pub fn station_ip() -> Option<Ipv4Cidr> {
    crate::status::with(|s| s.ipv4)
}

/// The MAC address, `AA:BB:CC:DD:EE:FF`.
pub fn mac_address() -> heapless::String<17> {
    let mac = esp_hal::efuse::base_mac_address();
    let mut s = heapless::String::new();
    for (i, b) in mac.as_bytes().iter().enumerate() {
        let _ = write!(s, "{}{b:02X}", if i == 0 { "" } else { ":" });
    }
    s
}

pub struct App;

impl AppBuilder for App {
    type PathRouter = impl picoserve::routing::PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::from_service(routes::Routes).layer(api::OriginCheck)
    }
}

static CONFIG: picoserve::Config = picoserve::Config::new(picoserve::Timeouts {
    start_read_request: picoserve::time::Duration::from_secs(5),
    persistent_start_read_request: picoserve::time::Duration::from_secs(5),
    read_request: picoserve::time::Duration::from_secs(10),
    write: picoserve::time::Duration::from_secs(10),
})
.keep_connection_alive();

/// Whether a client may connect: from a private network, or from the setup access point.
fn allowed(socket: &TcpSocket, ap: bool) -> bool {
    ap || match socket.remote_endpoint() {
        Some(endpoint) => match endpoint.addr {
            IpAddress::Ipv4(address) => is_private_ipv4(address.octets()),
            #[allow(unreachable_patterns)]
            _ => false,
        },
        None => false,
    }
}

/// Serves one connection at a time on port 80 of `stack`.
async fn serve(id: usize, stack: Stack<'static>, app: &'static AppRouter<App>, ap: bool) -> ! {
    let mut rx_buffer = [0; TCP_BUFFER];
    let mut tx_buffer = [0; TCP_BUFFER];
    let mut http_buffer = [0; HTTP_BUFFER];
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        if let Err(e) = socket.accept(PORT).await {
            warn!("web {}: accept error: {}", id, e);
            continue;
        }
        if !allowed(&socket, ap) {
            warn!("web {}: refused {}", id, socket.remote_endpoint());
            socket.abort();
            let _ = socket.flush().await;
            continue;
        }
        socket.set_keep_alive(Some(Duration::from_secs(30)));
        socket.set_timeout(Some(Duration::from_secs(45)));
        match picoserve::Server::new(app, &CONFIG, &mut http_buffer)
            .serve(socket)
            .await
        {
            Ok(info) => debug!("web {}: {} requests", id, info.handled_requests_count),
            Err(e) => debug!("web {}: {}", id, defmt::Debug2Format(&e)),
        }
    }
}

#[embassy_executor::task(pool_size = STATION_TASKS)]
pub async fn station_task(id: usize, stack: Stack<'static>, app: &'static AppRouter<App>) -> ! {
    serve(id, stack, app, false).await
}

#[embassy_executor::task(pool_size = AP_TASKS)]
pub async fn ap_task(id: usize, stack: Stack<'static>, app: &'static AppRouter<App>) -> ! {
    serve(id, stack, app, true).await
}
