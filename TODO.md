# TODO

## Display: what still differs from AxeOS

The screens follow AxeOS (ESP-Miner) except for the following.

### Missing data

- **Stats screen: all-time best difficulty.** AxeOS shows `Best: <session>/<all time>`; we show the
  session best only. Needs settings storage (NVS) to keep the best across reboots.
- **Pool host** is the `pool::POOL_HOST` constant. AxeOS shows the URL of the pool in use, primary
  or fallback, from the settings.

### Screens not ported

- **Welcome** (no WiFi configured): setup access point name and its QR code. Needs the access point
  provisioning mode, and a QR code renderer (72 px on the LCD, 32 px on the OLED).
- **Connection**: AxeOS shows the WiFi task status messages and, while the access point is on, its
  name. Ours shows `Wi-Fi: <SSID>` and `Connecting...`.
- **Overheat** text: AxeOS says the frequency and voltage settings have been reset and to check them
  in AxeOS. Ours says the core power is cut until the next reboot, which is what this firmware does.
  Align it once the settings and the web interface exist.
- **Self-test**: results of the factory self-test, and its long-press reset.
- **ASIC status**: the error message when the ASIC chain doesn't come up.
- **Firmware update**: file name and progress of an OTA update.
- **Identify**: an image over every screen, for a time set from the web interface, until the button
  is pressed.

### Behaviour

- **Slide animation**: AxeOS slides each new carousel screen in from the right (128 refresh periods
  / 8).
- **Display timeout** setting: off after that many minutes without a button press, `0` off after
  5 s, negative never; the carousel still runs, a press wakes the display. The connection screens
  keep it on.
- **Rotation** setting (0, 90, 180, 270°). We only have the fixed `board::ST7789_FLIP`.
- **Button long press** (2 s): turns the setup access point on or off.

### Before a release

- Check the licences of the converted fonts: DigitalNumbers, AngelWish and portfolio 6×8
  (OpenSans is Apache 2.0, unscii public domain, LVGL MIT).
