//! Status screens on the ST7789 colour LCD (320 × 170, RGB565) on the 8-bit i80 bus of the LCD_CAM
//! peripheral.
//!
//! There is no frame buffer: the screen is drawn in bands of [`ROW_HEIGHT`] rows, each into the
//! DMA buffer, then sent to its rows of the panel with one transfer.

use core::fmt::Write;

use defmt::{info, warn};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use esp_hal::{
    dma::{DmaError, DmaTxBuf},
    gpio::{Input, Output},
    lcd_cam::lcd::i8080::I8080,
    Async,
};
use heapless::Vec;
use miner_core::{
    carousel::Screen,
    ui::{column_top, draw_qr, Align, Font, Label},
    units,
};

use super::{
    best_text, efficiency,
    fonts::{
        ANGEL_WISH_40, DIGITAL_NUMBERS_16, DIGITAL_NUMBERS_28, OPEN_SANS_BOLD_13,
        OPEN_SANS_BOLD_14, UNSCII_16,
    },
    images::{self, BACKGROUND},
    run, Panel, Text, View,
};
use crate::board;

const WIDTH: usize = 320;
/// Rows of the panel, centred in the 240 rows of the controller memory.
const HEIGHT: usize = 170;
const ROW_OFFSET: usize = 35;
/// Rows drawn and sent at once.
const ROW_HEIGHT: usize = 10;
/// Bytes of a band of rows: the size of the DMA buffer.
pub const ROW_BYTES: usize = WIDTH * ROW_HEIGHT * 2;
/// Space around the column screens, and between their lines.
const PADDING: i32 = 8;
const LINE_PAD: i32 = 1;
/// Side of the QR code beside the connection screen, and the gap before it.
const QR_SIZE: i32 = 72;
const QR_GAP: i32 = 8;
const WHITE: [u8; 2] = [0xFF, 0xFF];

// ST7789 commands
const SLPOUT: u8 = 0x11;
const INVOFF: u8 = 0x20;
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

/// A label of the statistics screen: box, font and alignment.
struct Placed {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: &'static Font,
    align: Align,
}

const fn placed(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: &'static Font,
    align: Align,
) -> Placed {
    Placed {
        x,
        y,
        width,
        height,
        font,
        align,
    }
}

// The statistics screen labels, on the background.
const TITLE: Placed = placed(15, 31, 105, 42, &ANGEL_WISH_40, Align::Left);
const IP: Placed = placed(94, 1, 100, 13, &OPEN_SANS_BOLD_13, Align::Center);
const HASHRATE: Placed = placed(15, 126, 98, 36, &DIGITAL_NUMBERS_28, Align::Right);
const BEST: Placed = placed(34, 99, 84, 15, &OPEN_SANS_BOLD_14, Align::Left);
const FAN: Placed = placed(152, 70, 56, 13, &OPEN_SANS_BOLD_13, Align::Center);
const TEMP: Placed = placed(136, 101, 45, 15, &OPEN_SANS_BOLD_14, Align::Right);
const VIN: Placed = placed(234, 43, 70, 15, &OPEN_SANS_BOLD_14, Align::Left);
const VCORE: Placed = placed(234, 65, 70, 15, &OPEN_SANS_BOLD_14, Align::Left);
const CURRENT: Placed = placed(234, 87, 70, 15, &OPEN_SANS_BOLD_14, Align::Left);
const POWER: Placed = placed(234, 109, 70, 15, &OPEN_SANS_BOLD_14, Align::Left);
const EFFICIENCY: Placed = placed(217, 136, 60, 20, &DIGITAL_NUMBERS_16, Align::Right);
const UPTIME: Placed = placed(20, 78, 110, 13, &OPEN_SANS_BOLD_13, Align::Right);

/// A text to draw, with where and how.
struct Item {
    text: Text,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: &'static Font,
    align: Align,
    scroll_ms: Option<u32>,
}

impl Item {
    fn placed(spec: &Placed, text: Text) -> Self {
        Self {
            text,
            x: spec.x,
            y: spec.y,
            width: spec.width,
            height: spec.height,
            font: spec.font,
            align: spec.align,
            scroll_ms: None,
        }
    }

    fn label(&self) -> Label<'_> {
        Label {
            font: self.font,
            text: &self.text,
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            align: self.align,
            scroll_ms: self.scroll_ms,
        }
    }
}

/// Text of a format, cut at the [`Text`] size.
fn text(args: core::fmt::Arguments) -> Text {
    let mut t = Text::new();
    let _ = t.write_fmt(args);
    t
}

/// Text of a reading with its format, or `--`.
fn or_dashes<T>(value: Option<T>, format: impl FnOnce(T) -> Text) -> Text {
    value
        .map(format)
        .unwrap_or_else(|| text(format_args!("--")))
}

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
        // the panel shows true colours inverted; 180° adds to the board's own mounting
        let (rotation, invert) = crate::settings::with(|s| (s.rotation, s.invert_screen));
        self.command(if invert { INVOFF } else { INVON }, &[])
            .await?;
        let flipped = board::ST7789_FLIP ^ (rotation == 180 || rotation == 270);
        let mirror = if flipped { MADCTL_MX } else { MADCTL_MY };
        self.command(MADCTL, &[MADCTL_MV | mirror]).await?;
        for y in (0..HEIGHT).step_by(ROW_HEIGHT) {
            self.draw_band(y, |band| band.fill(0)).await?;
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

    /// Draws the [`ROW_HEIGHT`] panel rows from `y` with `render`, given those rows of the DMA
    /// buffer (RGB565, most significant byte first).
    async fn draw_band(
        &mut self,
        y: usize,
        render: impl FnOnce(&mut [u8]),
    ) -> Result<(), DmaError> {
        let [x0_hi, x0_lo] = 0u16.to_be_bytes();
        let [x1_hi, x1_lo] = (WIDTH as u16 - 1).to_be_bytes();
        self.command(CASET, &[x0_hi, x0_lo, x1_hi, x1_lo]).await?;
        let [y0_hi, y0_lo] = ((ROW_OFFSET + y) as u16).to_be_bytes();
        let [y1_hi, y1_lo] = ((ROW_OFFSET + y + ROW_HEIGHT - 1) as u16).to_be_bytes();
        self.command(RASET, &[y0_hi, y0_lo, y1_hi, y1_lo]).await?;

        let (i8080, mut buf) = self.bus.take().expect("LCD bus in use");
        buf.set_length(ROW_BYTES);
        render(&mut buf.as_mut_slice()[..ROW_BYTES]);
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

    /// The texts of `view`, and whether they sit on the background.
    fn items(view: &View<'_>) -> (Vec<Item, 16>, bool) {
        let mut items = Vec::new();
        if view.identify {
            // the identify image alone, drawn by `draw`
            return (items, false);
        }
        if view.screen == Screen::Stats {
            let s = view.status;
            let temp = s.chip_temps_c[0].max(s.chip_temps_c[1]);
            for item in [
                Item::placed(&TITLE, text(format_args!("Bitaxe"))),
                Item::placed(
                    &IP,
                    view.ip.clone().unwrap_or_else(|| text(format_args!("--"))),
                ),
                Item::placed(&HASHRATE, text(format_args!("{:.0}", s.hashrate_ghs))),
                Item::placed(&BEST, text(format_args!("{}", best_text(s)))),
                Item::placed(
                    &UPTIME,
                    text(format_args!("{}", units::uptime_compact(view.uptime_s))),
                ),
                Item::placed(
                    &FAN,
                    or_dashes(s.fan_rpm.filter(|&r| r > 0), |r| text(format_args!("{r}"))),
                ),
                Item::placed(
                    &TEMP,
                    or_dashes((temp > 0.0).then_some(temp), |t| {
                        text(format_args!("{t:.0}"))
                    }),
                ),
                Item::placed(&VIN, or_dashes(s.vin_v, |v| text(format_args!("{v:.1}V")))),
                Item::placed(
                    &VCORE,
                    or_dashes(s.vcore_v, |v| text(format_args!("{v:.2}V"))),
                ),
                Item::placed(
                    &CURRENT,
                    or_dashes(s.current_a, |a| text(format_args!("{a:.1}A"))),
                ),
                Item::placed(
                    &POWER,
                    or_dashes(s.power_w, |w| text(format_args!("{w:.1}W"))),
                ),
                Item::placed(
                    &EFFICIENCY,
                    or_dashes(efficiency(s), |e| text(format_args!("{e:.1}"))),
                ),
            ] {
                let _ = items.push(item);
            }
        } else {
            let mut width = WIDTH as i32 - 2 * PADDING;
            if view.qr().is_some() {
                width -= QR_SIZE + QR_GAP;
            }
            let lines = view.lines();
            let line_height = i32::from(UNSCII_16.line_height);
            let top = column_top(
                lines.len(),
                line_height,
                LINE_PAD,
                PADDING,
                HEIGHT as i32 - 2 * PADDING,
            );
            for (i, (line, scroll)) in lines.into_iter().enumerate() {
                let _ = items.push(Item {
                    text: line,
                    x: PADDING,
                    y: top + i as i32 * (line_height + LINE_PAD),
                    width,
                    height: line_height,
                    font: &UNSCII_16,
                    align: Align::Left,
                    scroll_ms: scroll.then_some(view.screen_ms),
                });
            }
        }
        let notification = text(format_args!("{}", view.notification));
        let width = UNSCII_16.text_width(&notification);
        let _ = items.push(Item {
            text: notification,
            x: WIDTH as i32 - width,
            y: 0,
            width,
            height: i32::from(UNSCII_16.line_height),
            font: &UNSCII_16,
            align: Align::Left,
            scroll_ms: None,
        });
        (items, view.screen == Screen::Stats)
    }
}

impl Panel for Lcd {
    const LARGE: bool = true;

    async fn draw(&mut self, view: &View<'_>) {
        let (items, background) = Self::items(view);
        let qr = view.qr();
        let qr_x = WIDTH as i32 - PADDING - QR_SIZE;
        let qr_y = (HEIGHT as i32 - QR_SIZE) / 2;
        for y0 in (0..HEIGHT).step_by(ROW_HEIGHT) {
            let result = self
                .draw_band(y0, |band| {
                    let rows = ROW_HEIGHT.min(HEIGHT - y0);
                    if background {
                        let from = y0 * WIDTH * 2;
                        band[..rows * WIDTH * 2]
                            .copy_from_slice(&BACKGROUND[from..from + rows * WIDTH * 2]);
                        band[rows * WIDTH * 2..].fill(0);
                    } else {
                        band.fill(0);
                    }
                    let band_top = y0 as i32;
                    let mut plot = |x: i32, y: i32| {
                        let row = y - band_top;
                        if (0..WIDTH as i32).contains(&x) && (0..rows as i32).contains(&row) {
                            let i = (row as usize * WIDTH + x as usize) * 2;
                            band[i..i + 2].copy_from_slice(&WHITE);
                        }
                    };
                    if let Some(qr) = &qr {
                        if qr_y < band_top + ROW_HEIGHT as i32 && qr_y + QR_SIZE > band_top {
                            draw_qr(qr, qr_x, qr_y, QR_SIZE, &mut plot);
                        }
                    }
                    if view.identify {
                        let image = &images::IDENTIFY;
                        image.draw(
                            (WIDTH as i32 - image.width) / 2,
                            (HEIGHT as i32 - image.height) / 2,
                            &mut plot,
                        );
                    }
                    for item in &items {
                        if item.y < band_top + ROW_HEIGHT as i32 && item.y + item.height > band_top
                        {
                            item.label().draw(&mut plot);
                        }
                    }
                })
                .await;
            if let Err(e) = result {
                warn!("LCD update failed: {}", e);
                return;
            }
        }
    }

    async fn set_on(&mut self, on: bool) {
        if on {
            self.backlight.set_high();
        } else {
            self.backlight.set_low();
        }
    }
}

/// Sets the LCD up, then shows the status screens on it. Ends when the LCD can't be set up.
#[embassy_executor::task]
pub async fn display_task(mut lcd: Lcd, stack: Stack<'static>, button: Input<'static>) {
    if let Err(e) = lcd.init().await {
        warn!("LCD setup failed: {}", e);
        return;
    }
    info!("LCD ready");
    run(&mut lcd, stack, button).await;
}
