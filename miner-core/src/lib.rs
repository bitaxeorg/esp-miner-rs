//! Hardware independent mining logic of esp-miner-rs.
//!
//! Everything here is `no_std` and runs under `cargo test` on the host, so the parts of the
//! firmware that don't touch peripherals can be checked without a board.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod carousel;
pub mod coinbase;
pub mod hashrate;
pub mod header;
pub mod job;
pub mod json;
pub mod json_read;
pub mod pool_config;
pub mod scoreboard;
pub mod share;
pub mod sntp;
pub mod stats;
pub mod sv2;
pub mod thermal;
pub mod timing;
pub mod ui;
pub mod units;
pub mod vcore;
pub mod web;
