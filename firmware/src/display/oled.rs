//! Status screens on the I2C OLED (SSD1306, 128 × 32), in the 6 × 8 font.

use defmt::{info, warn, Debug2Format};
use embassy_net::Stack;
use embedded_hal_async::i2c::I2c;
use esp_hal::gpio::Input;
use miner_core::{
    carousel::Screen,
    ui::{column_top, Align, Label},
};
use ssd1306::{
    mode::BufferedGraphicsModeAsync, prelude::*, size::DisplaySize128x32, I2CDisplayInterface,
    Ssd1306Async,
};

use super::{fonts::PORTFOLIO_6X8, images, run, Panel, View};
use crate::power::BusDevice;

/// I2C addresses an OLED can answer at.
const ADDRESS: u8 = 0x3C;
const ALTERNATE_ADDRESS: u8 = 0x3D;
/// Display off: control byte (command), then the command.
const DISPLAY_OFF: [u8; 2] = [0x00, 0xAE];
const WIDTH: i32 = 128;
const HEIGHT: i32 = 32;
/// Space between the logo screen texts and the right edge.
const LOGO_TEXT_MARGIN: i32 = 8;

type Display = Ssd1306Async<
    I2CInterface<BusDevice>,
    DisplaySize128x32,
    BufferedGraphicsModeAsync<DisplaySize128x32>,
>;

struct Oled(Display);

impl Panel for Oled {
    const LARGE: bool = false;

    async fn draw(&mut self, view: &View<'_>) {
        let display = &mut self.0;
        display.clear_buffer();
        let mut plot = |x: i32, y: i32| {
            if (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) {
                display.set_pixel(x as u32, y as u32, true);
            }
        };
        let font = &PORTFOLIO_6X8;
        let line_height = i32::from(font.line_height);
        if view.identify {
            // "Hi!" three times the font size, centred
            const SCALE: i32 = 3;
            let text = "Hi!";
            let x0 = (WIDTH - font.text_width(text) * SCALE) / 2;
            let y0 = (HEIGHT - line_height * SCALE) / 2;
            font.draw(text, 0, 0, &mut |x, y| {
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        plot(x0 + x * SCALE + dx, y0 + y * SCALE + dy);
                    }
                }
            });
            if let Err(e) = display.flush().await {
                warn!("OLED display update failed: {}", Debug2Format(&e));
            }
            return;
        }
        match view.screen {
            Screen::BitaxeLogo => {
                let logo = &images::BITAXE_LOGO;
                logo.draw(
                    (WIDTH - logo.width) / 2,
                    (HEIGHT - logo.height) / 2 + 1,
                    &mut plot,
                );
                // name and board number right aligned, above the vertical middle
                for (text, dy) in [(env!("BOARD_NAME"), -12), (env!("BOARD_VERSION"), -4)] {
                    let x = WIDTH - LOGO_TEXT_MARGIN - font.text_width(text);
                    font.draw(text, x, (HEIGHT - line_height) / 2 + dy, &mut plot);
                }
            }
            Screen::OsmuLogo => {
                let logo = &images::OSMU_LOGO;
                logo.draw(
                    (WIDTH - logo.width) / 2,
                    (HEIGHT - logo.height) / 2,
                    &mut plot,
                );
            }
            _ => {
                let lines = view.lines();
                let top = column_top(lines.len(), line_height, 0, 0, HEIGHT);
                for (i, (text, scroll)) in lines.iter().enumerate() {
                    Label {
                        font,
                        text,
                        x: 0,
                        y: top + i as i32 * line_height,
                        width: WIDTH,
                        height: line_height,
                        align: Align::Left,
                        scroll_ms: scroll.then_some(view.screen_ms),
                    }
                    .draw(&mut plot);
                }
            }
        }
        let x = WIDTH - font.text_width(view.notification);
        font.draw(view.notification, x, 0, &mut plot);

        if let Err(e) = display.flush().await {
            warn!("OLED display update failed: {}", Debug2Format(&e));
        }
    }

    async fn set_on(&mut self, on: bool) {
        if let Err(e) = self.0.set_display_on(on).await {
            warn!("OLED display power failed: {}", Debug2Format(&e));
        }
    }
}

/// Finds the OLED, then shows the status screens on it. Ends at once when no OLED answers.
#[embassy_executor::task]
pub async fn display_task(mut bus: BusDevice, stack: Stack<'static>, button: Input<'static>) {
    let interface = if bus.write(ADDRESS, &DISPLAY_OFF).await.is_ok() {
        I2CDisplayInterface::new(bus)
    } else if bus.write(ALTERNATE_ADDRESS, &DISPLAY_OFF).await.is_ok() {
        I2CDisplayInterface::new_alternate_address(bus)
    } else {
        info!("no OLED display found");
        return;
    };
    // 180° turns the display over; 90° and 270° don't fit its shape
    let (rotation, invert) = crate::settings::with(|s| (s.rotation, s.invert_screen));
    let rotation = if rotation == 180 || rotation == 270 {
        DisplayRotation::Rotate180
    } else {
        DisplayRotation::Rotate0
    };
    let mut display =
        Ssd1306Async::new(interface, DisplaySize128x32, rotation).into_buffered_graphics_mode();
    if let Err(e) = display.init().await {
        warn!("OLED display setup failed: {}", Debug2Format(&e));
        return;
    }
    if invert {
        let _ = display.set_invert(true).await;
    }
    info!("OLED display found");
    run(&mut Oled(display), stack, button).await;
}
