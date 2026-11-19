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
use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_hal::gpio::Input;
use heapless::{String, Vec};
use miner_core::{
    carousel::{Carousel, Screen},
    units,
};

use crate::{
    settings,
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

/// A text of the screens, long enough for the longest: a scriptsig.
type Text = String<{ miner_core::coinbase::SCRIPTSIG_LEN + 1 }>;

const REFRESH_PERIOD: Duration = Duration::from_millis(200);
/// How long a notification shows.
const NOTIFICATION_MS: u64 = 500;
/// Presses of the button closer than this count once.
const DEBOUNCE: Duration = Duration::from_millis(50);
/// How long the button keeps a sleeping display on, and how long a press that toggles the setup
/// access point lasts.
const WAKE_TIME: Duration = Duration::from_secs(5);
const LONG_PRESS: Duration = Duration::from_secs(2);
/// Shown while the core power is cut.
const OVERHEAT_MESSAGE: &str =
    "Power, frequency and fan settings have been changed. Go to the web interface to check them.";

/// A display the screens are drawn on.
trait Panel {
    /// Large displays show the statistics alone.
    const LARGE: bool;

    /// Draws `view`.
    async fn draw(&mut self, view: &View<'_>);

    /// Turns the display on or off.
    async fn set_on(&mut self, on: bool);
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
    /// Saying hello, for the identify request.
    identify: bool,
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
                let ssid =
                    crate::settings::with(|s| Text::try_from(s.ssid.as_str()).unwrap_or_default());
                line(true, format_args!("Wi-Fi: {ssid}"));
                line(true, format_args!("{}", s.wifi_status));
                if s.ap_enabled {
                    line(false, format_args!("Setup Wi-Fi:"));
                    line(true, format_args!("{}", s.ap_ssid));
                }
            }
            Screen::Overheat => {
                line(false, format_args!("DEVICE OVERHEAT!"));
                line(true, format_args!("{OVERHEAT_MESSAGE}"));
                line(false, format_args!("IP Address:"));
                line(false, format_args!("{ip}"));
            }
            Screen::Urls => {
                line(false, format_args!("Stratum Host:"));
                line(true, format_args!("{}", s.pool_url));
                line(false, format_args!("IP Address:"));
                line(false, format_args!("{ip}"));
            }
            Screen::Stats => {
                line(false, format_args!("Gh/s: {:.0}", s.hashrate_ghs));
                match efficiency(s) {
                    Some(jth) => line(false, format_args!("J/Th: {jth:.2}")),
                    None => line(false, format_args!("J/Th: --")),
                }
                if s.show_new_block {
                    line(
                        true,
                        format_args!("Best: {}   !!! BLOCK FOUND !!!", best_text(s)),
                    );
                } else {
                    let ever = if s.best_difficulty_ever > 0.0 {
                        units::suffix_string(s.best_difficulty_ever as u64)
                    } else {
                        String::try_from("--").unwrap_or_default()
                    };
                    line(false, format_args!("Best: {}/{ever}", best_text(s)));
                }
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
                match &s.coinbase {
                    Some(c) => line(false, format_args!("Block: {}", c.block_height)),
                    None => line(false, format_args!("Block: --")),
                }
                if s.network_difficulty > 0.0 {
                    let difficulty = units::suffix_string(s.network_difficulty as u64);
                    line(false, format_args!("Difficulty: {difficulty}"));
                } else {
                    line(false, format_args!("Difficulty: --"));
                }
                line(false, format_args!("Scriptsig:"));
                match &s.coinbase {
                    Some(c) => line(true, format_args!("{}", c.scriptsig)),
                    None => line(true, format_args!("--")),
                }
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
    let mut last_press = Instant::now();
    let mut counters = (0, 0, 0);
    let mut shown = ("", Instant::now());
    let mut on = true;
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
        let notification = if status.mining_paused {
            "||"
        } else if (now - shown.1).as_millis() < NOTIFICATION_MS {
            shown.0
        } else {
            ""
        };

        // the display sleeps after the timeout of the settings, but not before mining starts
        let identify = status.identify_until_ms > now.as_millis();
        let timeout = settings::with(|s| s.display_timeout);
        let idle = now - last_press;
        let before_carousel = matches!(screen, Screen::Connection | Screen::Overheat);
        let want_on = match timeout {
            t if t < 0 => true,
            0 => before_carousel || idle < WAKE_TIME,
            minutes => {
                before_carousel || identify || idle < Duration::from_secs(minutes as u64 * 60)
            }
        };
        if want_on != on {
            on = want_on;
            panel.set_on(on).await;
        }

        if on {
            let view = View {
                screen,
                screen_ms: (now - screen_start).as_millis() as u32,
                status: &status,
                rssi: ip.as_ref().and_then(|_| crate::net::wifi_rssi()),
                ip,
                uptime_s: now.as_secs(),
                notification,
                identify,
            };
            panel.draw(&view).await;
        }

        if let Either::Second(_) =
            select(Timer::after(REFRESH_PERIOD), button.wait_for_falling_edge()).await
        {
            Timer::after(DEBOUNCE).await;
            if button.is_low() {
                last_press = Instant::now();
                let held = with_timeout(LONG_PRESS - DEBOUNCE, button.wait_for_high()).await;
                if held.is_err() {
                    info!("button: long press");
                    crate::net::TOGGLE_AP.signal(());
                    button.wait_for_high().await;
                } else if identify {
                    status::update(|s| s.identify_until_ms = 0);
                } else if on {
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
}
