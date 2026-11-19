//! Status screen: on the I2C OLED of the boards with the original pins (`display-oled`), or on the
//! ST7789 colour LCD of the boards with the colour display (`display-st7789`).

use core::fmt::Write;

use embassy_net::Stack;
use heapless::String;
use miner_core::units;

use crate::status::Status;

#[cfg(feature = "display-st7789")]
mod lcd;
#[cfg(feature = "display-oled")]
mod oled;

#[cfg(feature = "display-st7789")]
pub use lcd::{display_task, Lcd, ROW_BYTES};
#[cfg(feature = "display-oled")]
pub use oled::display_task;

/// A piece of text of the status screen, cut at its buffer size.
type Text = String<24>;

/// Measured hashrate, e.g. `1.21 TH/s`.
fn hashrate_text(s: &Status) -> Text {
    let (value, unit) = units::hashrate(f64::from(s.hashrate_ghs));
    let mut text = Text::new();
    let _ = write!(text, "{value:.2} {unit}");
    text
}

/// ASIC temperature, e.g. `62C`, `--C` until read.
fn temperature_text(s: &Status) -> Text {
    let mut text = Text::new();
    if s.asic_temp_c.is_nan() {
        let _ = write!(text, "--C");
    } else {
        let _ = write!(text, "{:.0}C", s.asic_temp_c);
    }
    text
}

/// Power, e.g. `14.2W`, `--W` until read.
fn power_text(s: &Status) -> Text {
    let mut text = Text::new();
    match s.power_w {
        Some(w) => {
            let _ = write!(text, "{w:.1}W");
        }
        None => {
            let _ = write!(text, "--W");
        }
    }
    text
}

/// Best nonce difficulty, e.g. `4.2M`.
fn best_text(s: &Status) -> Text {
    let (value, prefix) = units::si_prefix(s.best_difficulty);
    let mut text = Text::new();
    let _ = write!(text, "{value:.1}{prefix}");
    text
}

/// IP address, `None` while the network is down.
fn ip_text(stack: Stack<'_>) -> Option<Text> {
    let config = stack.config_v4()?;
    let mut text = Text::new();
    let _ = write!(text, "{}", config.address.address());
    Some(text)
}
