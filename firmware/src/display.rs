//! Status screens: on the I2C OLED of the boards with the original pins (`display-oled`), or on
//! the ST7789 colour LCD of the boards with the colour display (`display-st7789`).
//!
//! The screens follow [`Carousel`]: the connection screen until the network is up, then on the
//! OLED the Bitaxe and OSMU logos and a loop of the URLs, statistics, mining and Wi-Fi screens
//! (the BOOT button moves on), on the LCD the statistics alone. A notification shows top right
//! when shares are accepted (`↑`) or rejected (`x`) and when work arrives (`↓`).

use core::fmt::Write;

use defmt::info;
use embassy_futures::select::{select, Either};
use embassy_net::Stack;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::Input;
use heapless::{String, Vec};
use miner_core::{
    carousel::{Carousel, Screen},
    units,
};

use crate::{
    pool::POOL_HOST,
    status::{self, Status},
};

mod fonts;
mod images;
#[cfg(feature = "display-st7789")]
mod lcd;
#[cfg(feature = "display-oled")]
mod oled;

#[cfg(feature = "display-st7789")]
pub use lcd::{display_task, Lcd, ROW_BYTES};
#[cfg(feature = "display-oled")]
pub use oled::display_task;

/// A text of the screens, cut at its buffer size.
type Text = String<64>;

const REFRESH_PERIOD: Duration = Duration::from_millis(200);
/// How long a notification shows.
const NOTIFICATION_MS: u64 = 500;
/// Presses of the button closer than this count once.
const DEBOUNCE: Duration = Duration::from_millis(50);
/// Shown while the core power is cut.
const OVERHEAT_MESSAGE: &str = "Core power has been cut until the next reboot.";

/// A display the screens are drawn on.
trait Panel {
    /// Large displays show the statistics alone.
    const LARGE: bool;

    /// Draws `view`.
    async fn draw(&mut self, view: &View<'_>);
}

/// What a screen shows at a refresh.
struct View<'a> {
    screen: Screen,
    /// Time since the screen was shown, for the scrolling texts.
    screen_ms: u32,
    status: &'a Status,
    ip: Option<Text>,
    rssi: Option<i8>,
    uptime_s: u64,
    notification: &'static str,
}

/// A line of a screen in a column: its text, and whether it scrolls when too wide.
type Line = (Text, bool);

impl View<'_> {
    /// The lines of the column screens: connection, overheat, URLs, statistics, mining, Wi-Fi.
    fn lines(&self) -> Vec<Line, 4> {
        let s = self.status;
        let mut lines = Vec::new();
        let mut line = |scroll: bool, args: core::fmt::Arguments| {
            let mut text = Text::new();
            let _ = text.write_fmt(args);
            let _ = lines.push((text, scroll));
        };
        let ip = self.ip.as_deref().unwrap_or("");
        match self.screen {
            Screen::Connection => {
                line(true, format_args!("Wi-Fi: {}", crate::WIFI_SSID));
                line(true, format_args!("Connecting..."));
            }
            Screen::Overheat => {
                line(false, format_args!("DEVICE OVERHEAT!"));
                line(true, format_args!("{OVERHEAT_MESSAGE}"));
                line(false, format_args!("IP Address:"));
                line(false, format_args!("{ip}"));
            }
            Screen::Urls => {
                line(false, format_args!("Stratum Host:"));
                line(true, format_args!("{POOL_HOST}"));
                line(false, format_args!("IP Address:"));
                line(false, format_args!("{ip}"));
            }
            Screen::Stats => {
                line(false, format_args!("Gh/s: {:.0}", s.hashrate_ghs));
                match efficiency(s) {
                    Some(jth) => line(false, format_args!("J/Th: {jth:.2}")),
                    None => line(false, format_args!("J/Th: --")),
                }
                line(false, format_args!("Best: {}", best_text(s)));
                let [t1, t2] = s.chip_temps_c;
                if t1 > 0.0 && t2 > 0.0 {
                    line(false, format_args!("Temp: {t1:.1}°C/{t2:.1}°C"));
                } else if t1 > 0.0 {
                    line(false, format_args!("Temp: {t1:.1}°C"));
                } else {
                    line(false, format_args!("Temp: --"));
                }
            }
            Screen::Mining => {
                // the stratum client doesn't expose the coinbase: no block height nor scriptsig
                line(false, format_args!("Block: --"));
                if s.network_difficulty > 0.0 {
                    let difficulty = units::suffix_string(s.network_difficulty as u64);
                    line(false, format_args!("Difficulty: {difficulty}"));
                } else {
                    line(false, format_args!("Difficulty: --"));
                }
                line(false, format_args!("Scriptsig:"));
                line(true, format_args!("--"));
            }
            Screen::Wifi => {
                line(false, format_args!("Wi-Fi Signal"));
                match self.rssi {
                    Some(rssi) => line(false, format_args!("RSSI: {rssi} dBm")),
                    None => line(false, format_args!("RSSI: -- dBm")),
                }
                line(false, format_args!("Signal: {}", signal_text(self.rssi)));
                let uptime = units::uptime_full(self.uptime_s);
                line(false, format_args!("Uptime: {uptime}"));
            }
            Screen::BitaxeLogo | Screen::OsmuLogo => {}
        }
        lines
    }
}

/// Power over hashrate (J/TH), when both are known.
fn efficiency(s: &Status) -> Option<f32> {
    match s.power_w {
        Some(w) if w > 0.0 && s.hashrate_ghs > 0.0 => Some(w / (s.hashrate_ghs / 1000.0)),
        _ => None,
    }
}

/// Best nonce difficulty, `--` before the first.
fn best_text(s: &Status) -> String<16> {
    if s.best_difficulty > 0.0 {
        units::suffix_string(s.best_difficulty as u64)
    } else {
        String::try_from("--").unwrap_or_default()
    }
}

/// Wi-Fi signal quality from its RSSI (dBm).
fn signal_text(rssi: Option<i8>) -> &'static str {
    match rssi {
        Some(r) if r > -50 => "Excellent",
        Some(r) if r > -60 => "Good",
        Some(r) if r > -70 => "Fair",
        Some(_) => "Weak",
        None => "--",
    }
}

/// IP address, `None` while the network is down.
fn ip_text(stack: Stack<'_>) -> Option<Text> {
    let config = stack.config_v4()?;
    let mut text = Text::new();
    let _ = write!(text, "{}", config.address.address());
    Some(text)
}

/// RSSI of the access point the station is connected to (dBm).
fn wifi_rssi() -> Option<i8> {
    let mut rssi: esp_wifi_sys_esp32s3::c_types::c_int = 0;
    // SAFETY: the Wi-Fi driver writes the RSSI to `rssi`, which outlives the call.
    let err = unsafe { esp_wifi_sys_esp32s3::include::esp_wifi_sta_get_rssi(&mut rssi) };
    (err == 0).then_some(rssi as i8)
}

/// The notification symbol for what changed since the last check.
fn notification(accepted: bool, rejected: bool, received: bool) -> &'static str {
    match (received, rejected, accepted) {
        (false, false, false) => "",
        (false, false, true) => "↑",
        (false, true, false) => "x",
        (false, true, true) => "x↑",
        (true, false, false) => "↓",
        (true, false, true) => "↕",
        (true, true, false) => "x↓",
        (true, true, true) => "x↕",
    }
}

/// Shows the screens on `panel` every [`REFRESH_PERIOD`], the button moving the carousel on.
async fn run<P: Panel>(panel: &mut P, stack: Stack<'static>, mut button: Input<'static>) {
    let mut carousel = Carousel::new(P::LARGE);
    let mut screen = Screen::Connection;
    let mut screen_start = Instant::now();
    let mut last_update = Instant::now();
    let mut counters = (0, 0, 0);
    let mut shown = ("", Instant::now());
    loop {
        let now = Instant::now();
        let status = status::get();
        let ip = ip_text(stack);
        let dt = (now - last_update).as_millis() as u32;
        last_update = now;
        let next = carousel.update(dt, ip.is_some(), status.overheat);
        if next != screen {
            screen = next;
            screen_start = now;
        }

        let latest = (status.accepted, status.rejected, status.work_received);
        if latest != counters {
            let symbol = notification(
                latest.0 > counters.0,
                latest.1 > counters.1,
                latest.2 > counters.2,
            );
            shown = (symbol, now);
            counters = latest;
        }
        let notification = if (now - shown.1).as_millis() < NOTIFICATION_MS {
            shown.0
        } else {
            ""
        };

        let view = View {
            screen,
            screen_ms: (now - screen_start).as_millis() as u32,
            status: &status,
            rssi: ip.as_ref().and_then(|_| wifi_rssi()),
            ip,
            uptime_s: now.as_secs(),
            notification,
        };
        panel.draw(&view).await;

        if let Either::Second(_) =
            select(Timer::after(REFRESH_PERIOD), button.wait_for_falling_edge()).await
        {
            Timer::after(DEBOUNCE).await;
            if button.is_low() {
                let next = carousel.button();
                info!("button: next screen");
                if next != screen {
                    screen = next;
                    screen_start = Instant::now();
                }
            }
        }
    }
}
