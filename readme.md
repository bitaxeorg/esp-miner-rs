# ESP-Miner-rs

## Introduction
ESP-Miner-rs is bitcoin miner firmware designed to run on the ESP32-S3 of various Bitaxe:
- [BitaxeMax] with 1 Bitmain ASIC BM1397
- [BitaxeUltra] with 1 Bitmain ASIC BM1366

and more to come.

## Architecture
- Based on [esp-template](https://esp-rs.github.io/book/writing-your-own-application/generate-project-from-template.html#esp-template)

## Setup

### Nix user
```bash
nix-shell
```

### Other
- install [rust](https://esp-rs.github.io/book/installation/installation.html#rust-installation)
- follow the [espup](https://esp-rs.github.io/book/installation/installation.html#espup) installation guide
    * skip §3.2 "RISC-V Targets Only" as out target is Xtensa ESP32-S3
    * in §3.3 "RISC-V and Xtensa Targets" you can install only esp32s3 target `espup install -t esp32s3`
    * skip §3.4 "std Development Requirements" as we are `#[no_std]`
- install [probe-rs](https://docs.esp-rs.org/book/tooling/debugging/probe-rs.html) is you are using `USB-JTAG-SERIAL` peripheral (recommanded) or [espflash](https://esp-rs.github.io/book/tooling/espflash.html)

## Build and/or Run

dev profile can potentially be one or more orders of magnitude slower than release, and may cause issues with timing-senstive peripherals and/or devices of `esp-hal`

### Using Just
- edit the just file to change your local wifi SSID/PASSORD
- `just build max` to build esp-miner-rs for [BitaxeMax]
- `just build ultra` to build esp-miner-rs for [BitaxeUltra]
- `just run max` to build/flash/run esp-miner-rs for [BitaxeMax] on the connected hardware

### Manually
- `SSID="my_ssid" PASSWORD="my_password" cargo build --release --features=bitaxe-max`
- `SSID="my_ssid" PASSWORD="my_password" cargo run --release --features=bitaxe-max`

[BitaxeMax]:https://github.com/skot/bitaxe/tree/max-v2.3
[BitaxeUltra]:https://github.com/skot/bitaxe/tree/ultra-204