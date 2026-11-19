//! Status screen on the ST7789 colour LCD (320 × 170, RGB565) on the 8-bit i80 bus of the LCD_CAM
//! peripheral.
//!
//! There is no frame buffer: each text row is drawn into the DMA buffer, then sent to its rows of
//! the panel with one transfer.

use core::{convert::Infallible, fmt::Write};

use defmt::{info, warn};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyle},
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    text::{Baseline, Text},
};
use esp_hal::{
    dma::{DmaError, DmaTxBuf},
    gpio::Output,
    lcd_cam::lcd::i8080::I8080,
    Async,
};

use super::{best_text, hashrate_text, ip_text, power_text, temperature_text};
use crate::{board, status};

const WIDTH: usize = 320;
/// Rows of the panel, centred in the 240 rows of the controller memory.
const HEIGHT: usize = 170;
const ROW_OFFSET: usize = 35;
/// Height of a text row of [`FONT_10X20`].
const ROW_HEIGHT: usize = 20;
/// Bytes of a text row: the size of the DMA buffer.
pub const ROW_BYTES: usize = WIDTH * ROW_HEIGHT * 2;
/// Width of the label column of the status rows.
const LABEL_WIDTH: i32 = 100;
const REFRESH_PERIOD: Duration = Duration::from_secs(2);

// ST7789 commands
const SLPOUT: u8 = 0x11;
const INVON: u8 = 0x21;
const DISPON: u8 = 0x29;
const CASET: u8 = 0x2A;
const RASET: u8 = 0x2B;
const RAMWR: u8 = 0x2C;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3A;
const RAMCTRL: u8 = 0xB0;
/// MADCTL: rows and columns swapped (landscape), then mirrored on one axis or the other.
const MADCTL_MY: u8 = 0x80;
const MADCTL_MX: u8 = 0x40;
const MADCTL_MV: u8 = 0x20;
/// COLMOD: 16 bits per pixel (RGB565).
const COLMOD_RGB565: u8 = 0x55;
/// RAMCTRL: frame memory written over the MCU interface, RGB565 bytes most significant first.
const RAMCTRL_BIG_ENDIAN: [u8; 2] = [0x00, 0xF0];

const RESET_PULSE: Duration = Duration::from_millis(10);
/// Time the ST7789 needs after a reset and after leaving sleep.
const RESET_WAIT: Duration = Duration::from_millis(120);
const SLEEP_OUT_WAIT: Duration = Duration::from_millis(120);

/// The ST7789 on the i80 bus, with its control lines.
pub struct Lcd {
    /// The bus and its DMA buffer, taken by a transfer while it runs.
    bus: Option<(I8080<'static, Async>, DmaTxBuf)>,
    reset: Output<'static>,
    backlight: Output<'static>,
    /// RD (held high: the panel is only written) and the panel power, kept on.
    _read: Output<'static>,
    _power: Output<'static>,
}

impl Lcd {
    /// `buf` must hold [`ROW_BYTES`]. `reset` is active low, `backlight` and `power` active high,
    /// `read` held high.
    pub fn new(
        i8080: I8080<'static, Async>,
        buf: DmaTxBuf,
        reset: Output<'static>,
        backlight: Output<'static>,
        read: Output<'static>,
        power: Output<'static>,
    ) -> Self {
        Self {
            bus: Some((i8080, buf)),
            reset,
            backlight,
            _read: read,
            _power: power,
        }
    }

    /// Resets and sets up the panel, clears it, then turns it and its backlight on.
    async fn init(&mut self) -> Result<(), DmaError> {
        self.backlight.set_low();
        self.reset.set_low();
        Timer::after(RESET_PULSE).await;
        self.reset.set_high();
        Timer::after(RESET_WAIT).await;

        self.command(SLPOUT, &[]).await?;
        Timer::after(SLEEP_OUT_WAIT).await;
        self.command(COLMOD, &[COLMOD_RGB565]).await?;
        self.command(RAMCTRL, &RAMCTRL_BIG_ENDIAN).await?;
        self.command(INVON, &[]).await?;
        let mirror = if board::ST7789_FLIP {
            MADCTL_MX
        } else {
            MADCTL_MY
        };
        self.command(MADCTL, &[MADCTL_MV | mirror]).await?;
        for y in (0..HEIGHT).step_by(ROW_HEIGHT) {
            self.draw_row(y, |row| {
                let _ = row.clear(Rgb565::BLACK);
            })
            .await?;
        }
        self.command(DISPON, &[]).await?;
        self.backlight.set_high();
        Ok(())
    }

    /// Sends `cmd` with its `params`.
    async fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), DmaError> {
        let (i8080, mut buf) = self.bus.take().expect("LCD bus in use");
        buf.fill(params);
        self.transfer(i8080, buf, cmd).await
    }

    /// Draws the [`ROW_HEIGHT`] panel rows from `y` with `render`.
    async fn draw_row(
        &mut self,
        y: usize,
        render: impl FnOnce(&mut RowBuffer<'_>),
    ) -> Result<(), DmaError> {
        let [x0_hi, x0_lo] = 0u16.to_be_bytes();
        let [x1_hi, x1_lo] = (WIDTH as u16 - 1).to_be_bytes();
        self.command(CASET, &[x0_hi, x0_lo, x1_hi, x1_lo]).await?;
        let [y0_hi, y0_lo] = ((ROW_OFFSET + y) as u16).to_be_bytes();
        let [y1_hi, y1_lo] = ((ROW_OFFSET + y + ROW_HEIGHT - 1) as u16).to_be_bytes();
        self.command(RASET, &[y0_hi, y0_lo, y1_hi, y1_lo]).await?;

        let (i8080, mut buf) = self.bus.take().expect("LCD bus in use");
        buf.set_length(ROW_BYTES);
        render(&mut RowBuffer(&mut buf.as_mut_slice()[..ROW_BYTES]));
        self.transfer(i8080, buf, RAMWR).await
    }

    /// Sends `cmd` then the data of `buf`, waiting for the transfer without blocking.
    async fn transfer(
        &mut self,
        i8080: I8080<'static, Async>,
        buf: DmaTxBuf,
        cmd: u8,
    ) -> Result<(), DmaError> {
        let mut transfer = match i8080.send(cmd, 0, buf) {
            Ok(transfer) => transfer,
            Err((e, i8080, buf)) => {
                self.bus = Some((i8080, buf));
                return Err(e);
            }
        };
        transfer.wait_for_done().await;
        let (result, i8080, buf) = transfer.wait();
        self.bus = Some((i8080, buf));
        result
    }
}

/// A text row of the panel in the DMA buffer, RGB565 most significant byte first.
struct RowBuffer<'a>(&'a mut [u8]);

impl OriginDimensions for RowBuffer<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, ROW_HEIGHT as u32)
    }
}

impl DrawTarget for RowBuffer<'_> {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            let (Ok(x), Ok(y)) = (usize::try_from(point.x), usize::try_from(point.y)) else {
                continue;
            };
            if x < WIDTH && y < ROW_HEIGHT {
                let i = (y * WIDTH + x) * 2;
                self.0[i..i + 2].copy_from_slice(&RawU16::from(color).into_inner().to_be_bytes());
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        let bytes = RawU16::from(color).into_inner().to_be_bytes();
        for pixel in self.0.chunks_exact_mut(2) {
            pixel.copy_from_slice(&bytes);
        }
        Ok(())
    }
}

/// Sets the LCD up, then shows the status on it every [`REFRESH_PERIOD`]. Ends when the LCD can't
/// be set up.
#[embassy_executor::task]
pub async fn display_task(mut lcd: Lcd, stack: Stack<'static>) {
    if let Err(e) = lcd.init().await {
        warn!("LCD setup failed: {}", e);
        return;
    }
    info!("LCD ready");

    let title = MonoTextStyle::new(&FONT_10X20, Rgb565::YELLOW);
    let label = MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN);
    let value = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    loop {
        let rows = status_rows(stack);
        if let Err(e) = lcd
            .draw_row(0, |row| {
                let _ = row.clear(Rgb565::BLACK);
                let _ = Text::with_baseline("esp-miner-rs", Point::zero(), title, Baseline::Top)
                    .draw(row);
            })
            .await
        {
            warn!("LCD update failed: {}", e);
        }
        for (i, (name, text)) in rows.iter().enumerate() {
            let result = lcd
                .draw_row((i + 1) * ROW_HEIGHT, |row| {
                    let _ = row.clear(Rgb565::BLACK);
                    let _ =
                        Text::with_baseline(name, Point::zero(), label, Baseline::Top).draw(row);
                    let _ =
                        Text::with_baseline(text, Point::new(LABEL_WIDTH, 0), value, Baseline::Top)
                            .draw(row);
                })
                .await;
            if let Err(e) = result {
                warn!("LCD update failed: {}", e);
            }
        }
        Timer::after(REFRESH_PERIOD).await;
    }
}

/// The labelled rows under the title: hashrate, ASIC temperature, power, shares, best difficulty,
/// pool and IP address.
fn status_rows(stack: Stack<'_>) -> [(&'static str, super::Text); 7] {
    let s = status::get();
    let mut shares = super::Text::new();
    let _ = write!(shares, "{} / {}", s.accepted, s.rejected);
    let mut pool = super::Text::new();
    let _ = write!(
        pool,
        "{}",
        if s.pool_connected {
            "connected"
        } else {
            "connecting"
        }
    );
    let ip = ip_text(stack).unwrap_or_else(|| {
        let mut text = super::Text::new();
        let _ = write!(text, "connecting");
        text
    });
    [
        ("Hashrate", hashrate_text(&s)),
        ("ASIC", temperature_text(&s)),
        ("Power", power_text(&s)),
        ("Shares", shares),
        ("Best", best_text(&s)),
        ("Pool", pool),
        ("IP", ip),
    ]
}
