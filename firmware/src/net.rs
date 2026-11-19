//! WiFi: the station connection and the setup access point, their state for the interfaces,
//! scans, signal strength, and the services of the access point (DHCP, captive DNS) and of the
//! station (mDNS).

use core::{
    fmt::Write as _,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
};

use defmt::{debug, warn};
use edge_nal::UdpSplit;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_futures::select::{select, select3, Either, Either3};
use embassy_net::Stack;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    signal::Signal,
};
use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_radio::wifi::{
    ap::AccessPointConfig, scan::ScanConfig, sta::StationConfig, AuthenticationMethodConfig,
    Config, WifiController,
};
use heapless::{String, Vec};

use crate::{settings, status, wlog};

/// Networks a scan reports.
pub const SCAN_MAX: usize = 20;
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Address of the setup access point.
pub const AP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 4, 1);
pub const AP_PREFIX: u8 = 24;

pub struct Network {
    pub ssid: String<32>,
    pub rssi: i8,
    /// Authentication mode, numbered as the stock firmware reports it.
    pub authmode: u8,
}

static SCAN_REQUEST: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static SCAN_RESULT: Signal<CriticalSectionRawMutex, Option<Vec<Network, SCAN_MAX>>> = Signal::new();
/// Signaled when the station gets an address: the access point goes off.
static STATION_UP: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Signaled by a long press of the button: the access point goes on or off.
pub static TOGGLE_AP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// RSSI of the access point the station is connected to (dBm), `None` when not connected.
pub fn wifi_rssi() -> Option<i8> {
    let mut rssi: esp_wifi_sys_esp32s3::c_types::c_int = 0;
    // SAFETY: the Wi-Fi driver writes the RSSI to `rssi`, which outlives the call.
    let err = unsafe { esp_wifi_sys_esp32s3::include::esp_wifi_sta_get_rssi(&mut rssi) };
    (err == 0).then_some(rssi as i8)
}

/// SSID of the setup access point: `Bitaxe_` and the last two bytes of its MAC address.
pub fn ap_ssid() -> String<16> {
    let mac = esp_hal::efuse::base_mac_address();
    let b = mac.as_bytes();
    let mut ssid = String::new();
    // the access point's MAC is the base one plus one
    let _ = write!(ssid, "Bitaxe_{:02X}{:02X}", b[4], b[5].wrapping_add(1));
    ssid
}

/// The station configuration, from the settings.
pub fn station_config() -> StationConfig {
    let (ssid, password) = settings::with(|s| (s.ssid.clone(), s.wifi_pass.clone()));
    let authentication = if password.is_empty() {
        AuthenticationMethodConfig::Open
    } else {
        AuthenticationMethodConfig::WpaWpa2Personal(
            password.as_str().try_into().unwrap_or_default(),
        )
    };
    StationConfig::default()
        .with_ssid(ssid.as_str().try_into().unwrap_or_default())
        .with_authentication(authentication)
}

/// The open setup access point, on channel 1.
pub fn ap_config() -> AccessPointConfig {
    AccessPointConfig::default()
        .with_ssid(ap_ssid().as_str().try_into().unwrap_or_default())
        .with_channel(1)
        .with_authentication(AuthenticationMethodConfig::Open)
}

/// The networks in range, `None` if the scan failed.
pub async fn scan() -> Option<Vec<Network, SCAN_MAX>> {
    SCAN_RESULT.reset();
    SCAN_REQUEST.signal(());
    with_timeout(SCAN_TIMEOUT, SCAN_RESULT.wait())
        .await
        .ok()
        .flatten()
}

async fn do_scan(controller: &mut WifiController<'static>) -> Option<Vec<Network, SCAN_MAX>> {
    let found = controller
        .scan_async(&ScanConfig::default().with_max(SCAN_MAX))
        .await
        .ok()?;
    let mut networks = Vec::new();
    for ap in found.iter().filter(|ap| !ap.ssid.is_empty()) {
        let mut ssid = String::new();
        for c in ap.ssid.as_str().chars() {
            if ssid.push(c).is_err() {
                break;
            }
        }
        let authmode = ap.auth_method.map_or(0, |m| m as u8);
        if networks
            .push(Network {
                ssid,
                rssi: ap.signal_strength,
                authmode,
            })
            .is_err()
        {
            break;
        }
    }
    Some(networks)
}

fn set_wifi_status(text: &str) {
    status::update(|s| status::set_str(&mut s.wifi_status, text));
}

/// Turns the access point on or off, keeping the station.
fn set_ap(controller: &mut WifiController<'static>, on: bool) {
    let config = if on {
        Config::AccessPointStation(station_config(), ap_config())
    } else {
        Config::Station(station_config())
    };
    match controller.set_config(&config) {
        Ok(()) => {
            wlog!(
                Info,
                "wifi",
                "access point {}",
                if on { "on" } else { "off" }
            );
            status::update(|s| s.ap_enabled = on);
        }
        Err(e) => warn!("switching the access point failed: {}", e),
    }
}

/// Follows the station's address.
#[embassy_executor::task]
pub async fn address_task(stack: Stack<'static>) {
    loop {
        stack.wait_config_up().await;
        let address = stack.config_v4().map(|c| c.address);
        if let Some(a) = address {
            wlog!(Info, "wifi", "address {}", a);
        }
        status::update(|s| s.ipv4 = address);
        STATION_UP.signal(());
        stack.wait_config_down().await;
        status::update(|s| s.ipv4 = None);
    }
}

/// Keeps the station connected, retrying every 5 s, runs the scans asked for, and keeps the
/// setup access point on while the station has no address.
#[embassy_executor::task]
pub async fn connection_task(mut controller: WifiController<'static>, configured: bool) {
    status::update(|s| {
        s.ap_enabled = true;
        status::set_str(&mut s.ap_ssid, &ap_ssid());
    });
    let mut retries: u32 = 0;
    let mut ap = true;
    loop {
        if !configured {
            set_wifi_status("No Wi-Fi configured");
            match select(SCAN_REQUEST.wait(), TOGGLE_AP.wait()).await {
                Either::First(()) => SCAN_RESULT.signal(do_scan(&mut controller).await),
                Either::Second(()) => {}
            }
            continue;
        }
        set_wifi_status("Connecting...");
        match select3(
            controller.connect_async(),
            SCAN_REQUEST.wait(),
            TOGGLE_AP.wait(),
        )
        .await
        {
            Either3::First(Ok(_)) => {
                retries = 0;
                set_wifi_status("Connected!");
                wlog!(Info, "wifi", "connected");
                loop {
                    match select3(
                        controller.wait_for_disconnect_async(),
                        SCAN_REQUEST.wait(),
                        select(STATION_UP.wait(), TOGGLE_AP.wait()),
                    )
                    .await
                    {
                        Either3::First(_) => break,
                        Either3::Second(()) => {
                            let result = do_scan(&mut controller).await;
                            SCAN_RESULT.signal(result);
                        }
                        Either3::Third(Either::First(())) => {
                            if ap {
                                ap = false;
                                set_ap(&mut controller, false);
                            }
                        }
                        Either3::Third(Either::Second(())) => {
                            ap = !ap;
                            set_ap(&mut controller, ap);
                        }
                    }
                }
                wlog!(Warn, "wifi", "disconnected");
                if !ap {
                    ap = true;
                    set_ap(&mut controller, true);
                }
            }
            Either3::First(Err(e)) => {
                retries += 1;
                let mut text = String::<48>::new();
                let _ = write!(text, "{:?} (retry #{})", defmt::Debug2Format(&e), retries);
                debug!("wifi connection failed: {}", text.as_str());
                set_wifi_status(&text);
                Timer::after(RETRY_DELAY).await;
            }
            Either3::Second(()) => {
                let result = do_scan(&mut controller).await;
                SCAN_RESULT.signal(result);
            }
            Either3::Third(()) => {
                ap = !ap;
                set_ap(&mut controller, ap);
            }
        }
    }
}

/// Leases the setup access point's clients an address, pointing them to it for DNS.
#[embassy_executor::task]
pub async fn dhcp_task(stack: Stack<'static>) {
    let buffers: UdpBuffers<1, 600, 600, 2> = UdpBuffers::new();
    let udp = Udp::new(stack, &buffers);
    let start = Instant::now();
    let mut server =
        edge_dhcp::server::Server::<_, 8>::new(move || start.elapsed().as_secs(), AP_ADDRESS);
    let mut gateway = [AP_ADDRESS; 1];
    let mut options = edge_dhcp::server::ServerOptions::new(AP_ADDRESS, Some(&mut gateway));
    let dns = [AP_ADDRESS];
    options.dns = &dns;
    let mut buffer = [0u8; 600];
    loop {
        let bound = edge_nal::UdpBind::bind(
            &udp,
            SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                edge_dhcp::io::DEFAULT_SERVER_PORT,
            )),
        )
        .await;
        match bound {
            Ok(mut socket) => {
                if let Err(e) =
                    edge_dhcp::io::server::run(&mut server, &options, &mut socket, &mut buffer)
                        .await
                {
                    warn!("DHCP server: {}", defmt::Debug2Format(&e));
                }
            }
            Err(e) => warn!("DHCP server bind: {}", defmt::Debug2Format(&e)),
        }
        Timer::after(RETRY_DELAY).await;
    }
}

/// Answers every name with the access point's address, so that clients open the setup page.
#[embassy_executor::task]
pub async fn captive_dns_task(stack: Stack<'static>) {
    let buffers: UdpBuffers<1, 512, 512, 2> = UdpBuffers::new();
    let udp = Udp::new(stack, &buffers);
    let mut tx = [0u8; 512];
    let mut rx = [0u8; 512];
    loop {
        if let Err(e) = edge_captive::io::run(
            &udp,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 53)),
            &mut tx,
            &mut rx,
            AP_ADDRESS,
            core::time::Duration::from_secs(60),
        )
        .await
        {
            warn!("captive DNS: {}", defmt::Debug2Format(&e));
        }
        Timer::after(RETRY_DELAY).await;
    }
}

/// Answers `<hostname>.local` with the station's address.
#[embassy_executor::task]
pub async fn mdns_task(stack: Stack<'static>, rng: esp_hal::rng::Rng) {
    let buffers: UdpBuffers<1, 768, 768, 2> = UdpBuffers::new();
    let udp = Udp::new(stack, &buffers);
    loop {
        stack.wait_config_up().await;
        let Some(ip) = stack.config_v4().map(|c| c.address.address()) else {
            continue;
        };
        let hostname = settings::with(|s| s.hostname.clone());
        let socket = edge_mdns::io::bind(
            &udp,
            edge_mdns::io::IPV4_DEFAULT_SOCKET,
            Some(Ipv4Addr::UNSPECIFIED),
            None,
        )
        .await;
        let mut socket = match socket {
            Ok(socket) => socket,
            Err(e) => {
                warn!("mDNS bind: {}", defmt::Debug2Format(&e));
                Timer::after(RETRY_DELAY).await;
                continue;
            }
        };
        let (recv, send) = socket.split();
        let host = edge_mdns::host::Host {
            hostname: &hostname,
            ipv4: ip,
            ipv6: Ipv6Addr::UNSPECIFIED,
            ttl: edge_mdns::domain::base::Ttl::from_secs(60),
        };
        let signal = Signal::<NoopRawMutex, ()>::new();
        let recv_buf = edge_mdns::buf::VecBufAccess::<NoopRawMutex, 768>::new();
        let send_buf = edge_mdns::buf::VecBufAccess::<NoopRawMutex, 768>::new();
        let mdns = edge_mdns::io::Mdns::<_, _, _, _, _, NoopRawMutex>::new(
            Some(Ipv4Addr::UNSPECIFIED),
            None,
            recv,
            send,
            &recv_buf,
            &send_buf,
            rng,
            &signal,
        );
        wlog!(Info, "mdns", "answering {}.local", hostname.as_str());
        if let Either::First(Err(e)) = select(
            mdns.run(edge_mdns::HostAnswersMdnsHandler::new(&host)),
            stack.wait_config_down(),
        )
        .await
        {
            warn!("mDNS: {}", defmt::Debug2Format(&e));
        }
    }
}
