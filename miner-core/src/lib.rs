//! Hardware independent mining logic of esp-miner-rs.
//!
//! Everything here is `no_std` and runs under `cargo test` on the host, so the parts of the
//! firmware that don't touch peripherals can be checked without a board.

#![cfg_attr(not(test), no_std)]

pub mod header;
pub mod job;
pub mod share;
pub mod thermal;
pub mod timing;
pub mod vcore;
