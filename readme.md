# ESP-Miner-rs

## Introduction

ESP-Miner-rs is a `#[no_std]` Rust Bitcoin miner firmware for the ESP32-S3 of the Bitaxe boards,
built on [esp-hal] and [embassy].

| Board | Board numbers | Name feature | ASIC | Core regulator | Fan and ASIC temperature |
|---|---|---|---|---|---|
| [BitaxeMax] | `102` (v2.2) | `bitaxe-max` | 1 × BM1397 | TPS40305 + DS4432 DAC, INA260 | EMC2101 |
| [BitaxeUltra] | `201`, `202`, `205` | | 1 × BM1366 | TPS40305 + DS4432 DAC, INA260 | EMC2101 (internal sensor) |
| [BitaxeUltra] | `207` | `bitaxe-ultra` | 1 × BM1366 | TPS546 | EMC2101 |
| [BitaxeHex] | `302`, `303` | `bitaxe-hex` (303) | 6 × BM1366 in 3 domains | TPS546, 12 V | EMC2302 (2 fans), 2 × TMP1075 |
| [BitaxeSupra] | `400`, `401` | | 1 × BM1368 | TPS40305 + DS4432 DAC, INA260 | EMC2101 (internal sensor) |
| [BitaxeSupra] | `402`, `403` | `bitaxe-supra` (403) | 1 × BM1368 | TPS546 | EMC2101 |
| [BitaxeGamma] | `600` to `603` | `bitaxe-gamma` (603) | 1 × BM1370 | TPS546 | EMC2101 |
| BitaxeGammaDuo | `650` | `bitaxe-gamma-duo` | 2 × BM1370 | TPS546 | EMC2101 |
| BitaxeSupraHex | `701`, `702` | `bitaxe-supra-hex` (702) | 6 × BM1368 in 3 domains | TPS546, 12 V | EMC2302 (2 fans), 2 × TMP1075 |
| [BitaxeGammaTurbo] | `801` | `bitaxe-gamma-turbo` | 2 × BM1370 | 2-phase TPS546, 12 V | EMC2103 |
| [BitaxeNajaDuo] | `1201` | `bitaxe-naja-duo` | 2 × BM1373 in series | 2-phase TPS546, 12 V | EMC2103 |
| [BitaxeGammaHex] | `1300` | `bitaxe-gamma-hex` | 6 × BM1370 in 2 domains | 4-phase TPS546, 12 V | EMC2103 |

### Status

This is a proof of concept:
- The firmware has not run on hardware since the ASIC, power and pool tasks were written: treat
  every board as untested.
- The BitaxeMax can't mine yet: the BM1397 needs midstates, which [bm13xx-rs] doesn't compute yet.
- The web interface follows AxeOS: dashboard, scoreboard, swarm, logs, system, pool, network,
  theme, settings and update pages, rendered on the device with [htmx], and the AxeOS JSON API
  (`/api/system/info`, `PATCH /api/system`, pools, statistics, scoreboard, OTA update…), so the
  tools that monitor AxeOS miners work with it. Only clients on a private network may use it.
- Settings live in the NVS partition with the keys and types of AxeOS: switching between both
  firmwares keeps the WiFi, pools and tuning. WiFi credentials given at build time are only
  defaults.
- Without WiFi, or until it connects, the miner opens the setup access point `Bitaxe_XXXX` (open,
  192.168.4.1, captive portal); a 2 s press of the BOOT button turns it on or off. Once connected,
  the miner answers as `<hostname>.local`.
- The firmware updates itself from the update page (an app image, `.bin`), into the other OTA
  partition of the AxeOS partition table.
- The display shows the AxeOS screens: on the SSD1306 OLED (128 × 32) of the boards with the
  original pins, the connection screen, the Bitaxe and OSMU logos, then the URLs, statistics,
  mining and Wi-Fi screens in a loop (the BOOT button moves on); on the ST7789 colour LCD
  (320 × 170) of the NajaDuo and GammaHex, the connection screen then the statistics screen on its
  background. Display timeout, rotation and inversion follow the settings.
- What still differs from AxeOS, on the display and in the web interface, is listed in
  [TODO.md](TODO.md).

## Architecture

- `firmware/`: the firmware for the ESP32-S3, built from its own directory
  - `board.rs`: the board values, selected by the board feature
  - `vcore.rs`: the core regulator, a TPS40305 moved by a DS4432 DAC or a TPS546 over PMBus
  - `cooling.rs`: the fans and the ASIC temperature, on an EMC2101, an EMC2103, or an EMC2302 with
    TMP1075 sensors
  - `power.rs`: powers the chips, pauses and resumes them, follows the core voltage and frequency
    of the settings, drives the fan (automatic PID or manual), and on overheat cuts the power,
    then restarts 100 mV and 100 MHz lower once cooled down
  - `asic.rs`: brings the chain up with [bm13xx-rs], feeds it jobs and checks the nonces
  - `pool.rs`: the Stratum v1 session, with the primary and fallback pools of the settings
  - `settings.rs`, `flash.rs`: the settings in the NVS partition, on the shared flash
  - `net.rs`: the WiFi station and setup access point, DHCP server, captive DNS and mDNS
  - `web.rs`: the web interface on [picoserve]: the JSON API (`web/api.rs`, `web/patch.rs`), the
    pages (`web/pages.rs`) and the static files (`web/assets.rs`, from `firmware/web/`)
  - `ota.rs`: the app partitions, the firmware checksum, updates and the boot partition
  - `stats.rs`, `weblog.rs`: the statistics history, the scoreboard and the text log of the web
    interface
  - `status.rs`: the status the tasks fill in, and `display.rs`: the screens, drawn on the OLED
    (`display/oled.rs`) or on the colour LCD over LCD_CAM and DMA (`display/lcd.rs`), with their
    fonts and images (`display/fonts.rs`, `display/images.rs`)
- `miner-core/`: the hardware independent logic (block header, share check, job book, hashrate
  meter, core voltage, fan PID, coinbase, pool entries, statistics, scoreboard, JSON, access
  rules, units, screen carousel and text drawing), tested on the host
- `tools/display-assets/`: converts the LVGL fonts and images of ESP-Miner's `main/` (and LVGL's
  `lv_font_unscii_16.c`) into the display assets of the firmware
- drivers: [bm13xx-rs] for the ASICs, [tps546], [emc2101], [emc2103], [emc230x], [tmp1075],
  [ds4432], [ina260] and [ssd1306]

## Setup

### Nix user
`flake.nix` provides everything: the Xtensa `esp` Rust toolchain, `espflash`, `probe-rs` and
`just`. CI builds with it too.
```bash
nix develop
```
or, with [direnv](https://direnv.net), `use flake` in your `.envrc`, next to your machine-specific
values (`WIFI_SSID`, `WIFI_PASSWORD`, `PROBE_RS_PROBE`).

### Other
- install [rust](https://esp-rs.github.io/book/installation/installation.html#rust-installation)
- follow the [espup](https://esp-rs.github.io/book/installation/installation.html#espup) installation guide
    * skip §3.2 "RISC-V Targets Only", as our target is the Xtensa ESP32-S3
    * in §3.3 "RISC-V and Xtensa Targets" you can install only the esp32s3 target: `espup install -t esp32s3`
    * skip §3.4 "std Development Requirements", as we are `#[no_std]`
- install [probe-rs](https://docs.esp-rs.org/book/tooling/debugging/probe-rs.html) if you use the
  `USB-JTAG-SERIAL` peripheral (recommended), or [espflash](https://esp-rs.github.io/book/tooling/espflash.html)

## Build and run

Every build needs exactly one board feature: the number printed on the board (`bitaxe-401`), or
the board name for its latest revision (`bitaxe-supra`, see the table above).

The dev profile can be one or more orders of magnitude slower than release, and may cause issues
with timing-sensitive peripherals of `esp-hal`: use `--release` for anything you flash.

Flashing writes the AxeOS partition table (`firmware/partitions.csv`, 16 MB): the probe-rs runner
and `espflash flash --partition-table partitions.csv` both take it. After an update from the web
interface, the bootloader starts the OTA partition: erase `otadata` (`espflash erase-parts
otadata`) for a newly flashed image to run.

WiFi credentials are optional at build time: they are the defaults of the settings, used until the
network page changes them. Without them, set the WiFi up from the access point.

### Using Just
- set default WiFi credentials if you want: `just WIFI_SSID=my_ssid WIFI_PASSWORD=my_password run gamma`
- `just build gamma` to build esp-miner-rs for the [BitaxeGamma], `just build 401` for a [BitaxeSupra] 401
- `just run gamma` to build, flash and run esp-miner-rs on the connected board
- `just test` to run the `miner-core` tests on the host
- `just display-assets <ESP-Miner>/main <LVGL>/src/font/lv_font_unscii_16.c` to regenerate the
  display fonts and images

### Manually
- `cd firmware`
- `cargo build --release --features=bitaxe-gamma` (with `WIFI_SSID` and `WIFI_PASSWORD` set for
  default credentials)
- `cargo run --release --features=bitaxe-gamma`
- `cargo test` from the repository root runs the `miner-core` tests on the host

### CI builds

CI builds every board number and keeps its firmware as the artifact `esp-miner-rs-bitaxe-<number>`:
the ELF, for probe-rs or espflash, and the app image `.bin`, for the update page. These builds have
no WiFi credentials: they start the setup access point.

[esp-hal]:https://github.com/esp-rs/esp-hal
[embassy]:https://github.com/embassy-rs/embassy
[bm13xx-rs]:https://github.com/GPTechinno/bm13xx-rs
[tps546]:https://github.com/GPTechinno/tps546
[emc2101]:https://github.com/Georges760/emc2101-rs
[emc2103]:https://github.com/Georges760/emc2103-rs
[emc230x]:https://github.com/retroctrl/emc230x
[tmp1075]:https://github.com/JanekGraff/tmp1075-rs
[ds4432]:https://github.com/w3irdrobot/ds4432
[ina260]:https://github.com/therealprof/ina260
[ssd1306]:https://github.com/rust-embedded-community/ssd1306
[htmx]:https://htmx.org
[picoserve]:https://github.com/sammhicks/picoserve
[BitaxeMax]:https://github.com/bitaxeorg/bitaxeMax
[BitaxeUltra]:https://github.com/bitaxeorg/bitaxeUltra
[BitaxeHex]:https://github.com/bitaxeorg/ultraHex
[BitaxeSupra]:https://github.com/bitaxeorg/bitaxeSupra
[BitaxeGamma]:https://github.com/bitaxeorg/bitaxeGamma
[BitaxeGammaTurbo]:https://github.com/bitaxeorg/BitaxeGT
[BitaxeNajaDuo]:https://github.com/bitaxeorg/naja-duo
[BitaxeGammaHex]:https://github.com/bitaxeorg/BitaxeGammaHex
