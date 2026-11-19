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
- The WiFi credentials are compiled in, and the pool address, worker and password are constants
  at the top of `firmware/src/pool.rs`.
- No display, web interface or settings storage yet.

## Architecture

- `firmware/`: the firmware for the ESP32-S3, built from its own directory
  - `board.rs`: the board values, selected by the board feature
  - `vcore.rs`: the core regulator, a TPS40305 moved by a DS4432 DAC or a TPS546 over PMBus
  - `cooling.rs`: the fans and the ASIC temperature, on an EMC2101, an EMC2103, or an EMC2302 with
    TMP1075 sensors
  - `power.rs`: powers the chips, then watches their temperature, drives the fan and cuts the power
    on overheat
  - `asic.rs`: brings the chain up with [bm13xx-rs], feeds it jobs and checks the nonces
  - `pool.rs`: the Stratum v1 session
- `miner-core/`: the hardware independent mining logic (block header, share check, job book, core
  voltage, fan curve), tested on the host
- drivers: [bm13xx-rs] for the ASICs, [tps546], [emc2101], [emc2103], [emc230x], [tmp1075],
  [ds4432] and [ina260]

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

### Using Just
- edit the justfile to set your WiFi SSID and password, or pass them: `just WIFI_SSID=my_ssid WIFI_PASSWORD=my_password run gamma`
- `just build gamma` to build esp-miner-rs for the [BitaxeGamma], `just build 401` for a [BitaxeSupra] 401
- `just run gamma` to build, flash and run esp-miner-rs on the connected board
- `just test` to run the `miner-core` tests on the host

### Manually
- `cd firmware`
- `WIFI_SSID="my_ssid" WIFI_PASSWORD="my_password" cargo build --release --features=bitaxe-gamma`
- `WIFI_SSID="my_ssid" WIFI_PASSWORD="my_password" cargo run --release --features=bitaxe-gamma`
- `cargo test` from the repository root runs the `miner-core` tests on the host

### CI builds

CI builds every board number and keeps its firmware (an ELF, for probe-rs or espflash) as the
artifact `esp-miner-rs-bitaxe-<number>`. These builds carry placeholder WiFi credentials, so they
can't join your network: build your own to mine.

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
[BitaxeMax]:https://github.com/bitaxeorg/bitaxeMax
[BitaxeUltra]:https://github.com/bitaxeorg/bitaxeUltra
[BitaxeHex]:https://github.com/bitaxeorg/ultraHex
[BitaxeSupra]:https://github.com/bitaxeorg/bitaxeSupra
[BitaxeGamma]:https://github.com/bitaxeorg/bitaxeGamma
[BitaxeGammaTurbo]:https://github.com/bitaxeorg/BitaxeGT
[BitaxeNajaDuo]:https://github.com/bitaxeorg/naja-duo
[BitaxeGammaHex]:https://github.com/bitaxeorg/BitaxeGammaHex
