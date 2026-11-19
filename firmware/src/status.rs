//! Miner status, filled in by the tasks and shown on the display.

use core::cell::Cell;

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};

#[derive(Clone, Copy)]
pub struct Status {
    /// Measured from the nonces of the last minutes (GH/s).
    pub hashrate_ghs: f32,
    /// Difficulty of the best nonce since boot.
    pub best_difficulty: f64,
    /// Temperature of the hottest chip (°C), NaN until read.
    pub asic_temp_c: f32,
    /// The chip temperatures the board measures (°C), NaN when absent or unread.
    pub chip_temps_c: [f32; 2],
    pub fan_rpm: Option<u16>,
    /// Power (W) as the board measures it.
    pub power_w: Option<f32>,
    /// Input voltage, core voltage of a chip (V) and current (A) as the board measures them.
    pub vin_v: Option<f32>,
    pub vcore_v: Option<f32>,
    pub current_a: Option<f32>,
    /// Core power cut on overheat.
    pub overheat: bool,
    /// Shares the pool accepted and rejected, and work it sent, in the current session.
    pub accepted: u64,
    pub rejected: u64,
    pub work_received: u64,
    /// Whether a pool session is authorized.
    pub pool_connected: bool,
    /// Network difficulty of the current work, 0 until known.
    pub network_difficulty: f64,
}

impl Status {
    const fn new() -> Self {
        Self {
            hashrate_ghs: 0.0,
            best_difficulty: 0.0,
            asic_temp_c: f32::NAN,
            chip_temps_c: [f32::NAN; 2],
            fan_rpm: None,
            power_w: None,
            vin_v: None,
            vcore_v: None,
            current_a: None,
            overheat: false,
            accepted: 0,
            rejected: 0,
            work_received: 0,
            pool_connected: false,
            network_difficulty: 0.0,
        }
    }
}

static STATUS: Mutex<CriticalSectionRawMutex, Cell<Status>> = Mutex::new(Cell::new(Status::new()));

/// The current status.
#[cfg(any(feature = "display-oled", feature = "display-st7789"))]
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
