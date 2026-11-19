//! Status screen on the I2C OLED (SSD1306, 128 × 32).

use core::fmt::Write;

use defmt::{info, warn, Debug2Format};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use embedded_hal_async::i2c::I2c;
use heapless::String;
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306Async};

use super::{best_text, hashrate_text, ip_text, power_text, temperature_text};
use crate::{power::BusDevice, status};

/// I2C addresses an OLED can answer at.
const ADDRESS: u8 = 0x3C;
const ALTERNATE_ADDRESS: u8 = 0x3D;
/// Display off: control byte (command), then the command.
const DISPLAY_OFF: [u8; 2] = [0x00, 0xAE];
const REFRESH_PERIOD: Duration = Duration::from_secs(2);
/// Height of a text line of [`FONT_6X10`], plus one pixel.
const LINE_HEIGHT: i32 = 11;

/// Finds the OLED, then shows the status on it every [`REFRESH_PERIOD`]. Ends at once when no
/// OLED answers.
#[embassy_executor::task]
pub async fn display_task(mut bus: BusDevice, stack: Stack<'static>) {
    let interface = if bus.write(ADDRESS, &DISPLAY_OFF).await.is_ok() {
        I2CDisplayInterface::new(bus)
    } else if bus.write(ALTERNATE_ADDRESS, &DISPLAY_OFF).await.is_ok() {
        I2CDisplayInterface::new_alternate_address(bus)
    } else {
        info!("no OLED display found");
        return;
    };
    let mut display = Ssd1306Async::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    if let Err(e) = display.init().await {
        warn!("OLED display setup failed: {}", Debug2Format(&e));
        return;
    }
    info!("OLED display found");

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    loop {
        display.clear_buffer();
        for (i, line) in status_lines(stack).iter().enumerate() {
            let position = Point::new(0, i as i32 * LINE_HEIGHT);
            // drawing to the frame buffer can't fail
            let _ = Text::with_baseline(line, position, style, Baseline::Top).draw(&mut display);
        }
        if let Err(e) = display.flush().await {
            warn!("OLED display update failed: {}", Debug2Format(&e));
        }
        Timer::after(REFRESH_PERIOD).await;
    }
}

/// The three lines of 21 characters of the status screen: hashrate, temperature and power; the
/// pool shares and best difficulty; the IP address.
fn status_lines(stack: Stack<'_>) -> [String<24>; 3] {
    let s = status::get();
    let mut lines: [String<24>; 3] = Default::default();
    // a line too long for its buffer is cut, which writing ignores
    let _ = write!(
        lines[0],
        "{} {} {}",
        hashrate_text(&s),
        temperature_text(&s),
        power_text(&s)
    );
    if s.pool_connected {
        let _ = write!(
            lines[1],
            "A{} R{} B{}",
            s.accepted,
            s.rejected,
            best_text(&s)
        );
    } else {
        let _ = write!(lines[1], "pool: connecting");
    }
    match ip_text(stack) {
        Some(ip) => {
            let _ = write!(lines[2], "{ip}");
        }
        None => {
            let _ = write!(lines[2], "WiFi: connecting");
        }
    }
    lines
}
