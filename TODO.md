# TODO

## Display: what still differs from AxeOS

The screens follow AxeOS (ESP-Miner) except for the following.

### Screens not ported

- **Welcome** (no WiFi configured): the QR code of the setup access point. The connection screen
  names the access point, without the QR code; it needs a QR code renderer (72 px on the LCD, 32 px
  on the OLED).
- **Self-test**: results of the factory self-test, and its long-press reset.
- **ASIC status**: the error message when the ASIC chain doesn't come up.
- **Firmware update**: file name and progress of an OTA update.
- **Identify**: AxeOS shows an image; ours writes "Hi!".
- **Paused**: AxeOS shows `▐▐`; ours `||` in the notification corner.

### Behaviour

- **Slide animation**: AxeOS slides each new carousel screen in from the right (128 refresh periods
  / 8).
- **Rotation** 90° and 270°: the OLED takes 0° and 180° only, the LCD turns 180°.
- **Display type**: the setting is kept, the board's display is used.

## Web interface: what still differs from AxeOS

The pages are rendered on the device with htmx instead of the Angular app; they follow its layout,
labels and formats, not its exact look.

### Pages

- **Dashboard**: no layout editing (widgets, wide view), no Hashrate Registers card (per-domain
  hashrate), no sensitive-data toggle, no confetti. The chart is drawn on the device: hashrate and
  ASIC temperature only, no series choice, no tooltips.
- **Pool**: one pool saved at a time, Stratum V1 fields only (no SV2, TLS or certificate), no
  quick links to the pool dashboards.
- **Update**: no GitHub release check, no firmware integrity check against the release (the
  checksum endpoint exists), no custom web UI (`useCustomWWW`, `OTAWWW`: the pages are part of the
  firmware).
- **Swarm**: list view only, no sorting, no edit modal for the other devices, no family colours.
- **Theme**: 20 accent colours instead of 50.
- **Logs**: the lines of the firmware's own text log (48 lines), polled every second, not the
  ESP-IDF log stream.

### API

- **WebSockets** `/api/ws` (logs) and `/api/ws/live` (live data): not implemented; the pages poll.
- **Missing data** in `/api/system/info`: rejected share reasons, coinbase outputs, value and payout
  status, pool banner, share batching, CPU usage, heap statistics beyond the free heap, reset
  reason, IPv6, total uptime across reboots (= uptime), power and hardware faults, second fan.
- **Statistics**: 300 samples taken at the data logging interval; AxeOS keeps 720, sampled every
  second and thinned.
- **Hostname**: applied at the next restart; AxeOS applies it at once.
- **Frequency and voltage** changes apply at once by bringing the chips up again, or by powering
  them again for the voltage; AxeOS changes them without interrupting.
- **NTP** time sync: the setting is kept, not used.

### Network

- **mDNS**: answers `<hostname>.local` only; AxeOS also announces `_http._tcp` with its TXT
  records, and renames itself on a name conflict.
- **Setup access point**: goes off when the station gets an address, even with a client connected
  to it.

### Before a release

- Check the licences of the converted fonts: DigitalNumbers, AngelWish and portfolio 6×8
  (OpenSans is Apache 2.0, unscii public domain, LVGL MIT), and of the Nippo font of the web pages.
- Run it on hardware: nothing of the web interface, settings storage, OTA or access point has run
  on a board yet.
