//! Miner status, filled in by the tasks and shown on the display.

use core::cell::Cell;

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};

#[derive(Clone, Copy)]
pub struct Status {
    /// Measured from the nonces of the last minutes (GH/s).
    pub hashrate_ghs: f32,
    /// Difficulty of the best nonce since boot.
    pub best_difficulty: f64,
    /// ASIC temperature (°C), NaN until read.
    pub asic_temp_c: f32,
    /// Power (W) as the board measures it, `None` until read.
    pub power_w: Option<f32>,
    /// Shares the pool accepted and rejected in the current session.
    pub accepted: u64,
    pub rejected: u64,
    /// Whether a pool session is authorized.
    pub pool_connected: bool,
}

impl Status {
    const fn new() -> Self {
        Self {
            hashrate_ghs: 0.0,
            best_difficulty: 0.0,
            asic_temp_c: f32::NAN,
            power_w: None,
            accepted: 0,
            rejected: 0,
            pool_connected: false,
        }
    }
}

static STATUS: Mutex<CriticalSectionRawMutex, Cell<Status>> = Mutex::new(Cell::new(Status::new()));

/// The current status.
#[cfg(feature = "display-oled")]
pub fn get() -> Status {
    STATUS.lock(Cell::get)
}

/// Changes the status with `f`.
pub fn update(f: impl FnOnce(&mut Status)) {
    STATUS.lock(|status| {
        let mut s = status.get();
        f(&mut s);
        status.set(s);
    });
}
