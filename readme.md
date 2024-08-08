# ESP-Miner-rs

## Introduction
ESP-Miner-rs is bitcoin miner firmware designed to run on the ESP32-S3 of various Bitaxe:
- [BitaxeMax](https://github.com/skot/bitaxe/tree/max-v2.3) with 1 Bitmain ASIC BM1397
- [BitaxeUltra](https://github.com/skot/bitaxe/tree/ultra-204) with 1 Bitmain ASIC BM1366

and more to come.

## Architecture
- Based on [esp-template](https://esp-rs.github.io/book/writing-your-own-application/generate-project-from-template.html#esp-template)

## Setup
- install [rust](https://esp-rs.github.io/book/installation/installation.html#rust-installation)
- follow the [espup](https://esp-rs.github.io/book/installation/installation.html#espup) installation guide
    * skip §3.2 "RISC-V Targets Only" as out target is Xtensa ESP32-S3
    * in §3.3 "RISC-V and Xtensa Targets" you can install only esp32s3 target `espup install -t esp32s3`
    * skip §3.4 "std Development Requirements" as we are `#[no_std]`
- install [probe-rs](https://docs.esp-rs.org/book/tooling/debugging/probe-rs.html) is you are using `USB-JTAG-SERIAL` peripheral (recommanded) or [espflash](https://esp-rs.github.io/book/tooling/espflash.html)

## Build
`cargo build --release` (dev profile can potentially be one or more orders of magnitude
  slower than release, and may cause issues with timing-senstive
  peripherals and/or devices of `esp-hal`)

## Flash and Run
`SSID="my_ssid" PASSWORD="my_password" cargo build --release`