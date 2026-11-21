//! Miner status, filled in by the tasks and shown on the display and the web interface.

use core::cell::RefCell;

use bm13xx_chain::MAX_HASH_DOMAINS;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use heapless::{String, Vec};
use miner_core::coinbase::CoinbaseInfo;

use crate::board;

/// Hashrate of one chip from its hash counters (GH/s), in total and per hash domain.
#[derive(Clone, Default)]
pub struct ChipHashrate {
    pub total: f32,
    pub domains: Vec<f32, MAX_HASH_DOMAINS>,
}

#[derive(Clone)]
pub struct Status {
    /// Measured from the nonces (GH/s): over the last 10 minutes, and over 1 minute, 10 minutes
    /// and 1 hour.
    pub hashrate_ghs: f32,
    pub hashrate_1m: f32,
    pub hashrate_10m: f32,
    pub hashrate_1h: f32,
    /// What the chips should hash at their frequency (GH/s).
    pub expected_hashrate_ghs: f32,
    /// Invalid nonces among all those returned in the last 10 minutes (%).
    pub error_percentage: f32,
    /// Difficulty of the best nonce since boot.
    pub best_difficulty: f64,
    /// Best nonce difficulty ever.
    pub best_difficulty_ever: f64,
    /// Hashes done since boot, from the nonces.
    pub total_hashes: f64,
    /// Chips found on the chain, and the frequency they run at (MHz).
    pub asic_count: u8,
    pub frequency_mhz: f32,
    /// Why the chain did not come up, until it does.
    pub asic_status: Option<String<128>>,
    /// Hashrate of each chip from its hash counters, over the last reading period.
    pub chip_hashrates: Vec<ChipHashrate, { board::ASIC_COUNT as usize }>,
    /// Temperature of the hottest chip (°C), NaN until read.
    pub asic_temp_c: f32,
    /// The chip temperatures the board measures (°C), NaN when absent or unread.
    pub chip_temps_c: [f32; 2],
    /// Regulator temperature (°C).
    pub vr_temp_c: Option<f32>,
    pub fan_rpm: Option<u16>,
    /// Fan duty (%).
    pub fan_percent: f32,
    /// Power (W) as the board measures it.
    pub power_w: Option<f32>,
    /// Input voltage, core voltage of a chip (V) and current (A) as the board measures them.
    pub vin_v: Option<f32>,
    pub vcore_v: Option<f32>,
    pub current_a: Option<f32>,
    /// Core power cut on overheat, the chips cooling down.
    pub overheat: bool,
    /// Mining paused from the web interface.
    pub mining_paused: bool,
    /// Shares submitted, accepted and rejected, and work received, in the current session.
    pub submitted: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub work_received: u64,
    /// Whether a pool session is authorized.
    pub pool_connected: bool,
    /// Share difficulty the pool set.
    pub pool_difficulty: f64,
    /// Pool response time to the last share, and its extremes since the last statistics sample
    /// (ms).
    pub response_time_ms: Option<f32>,
    pub response_time_min_ms: Option<f32>,
    pub response_time_max_ms: Option<f32>,
    /// Network difficulty of the current work, 0 until known.
    pub network_difficulty: f64,
    /// Block height and scriptsig of the current work, when its coinbase could be read.
    pub coinbase: Option<CoinbaseInfo>,
    /// Host and port of the pool in use, its user, and whether it is the fallback pool.
    pub pool_url: String<128>,
    pub pool_port: u16,
    pub pool_user: String<128>,
    pub using_fallback: bool,
    /// Why the primary pool is not in use, when the fallback is.
    pub primary_pool_error: String<64>,
    /// Blocks found since boot, and whether the news is still to be dismissed.
    pub block_found: u32,
    pub show_new_block: bool,
    /// The station's address, once it has one.
    pub ipv4: Option<embassy_net::Ipv4Cidr>,
    /// WiFi state in words, and the setup access point.
    pub wifi_status: String<48>,
    pub ap_enabled: bool,
    pub ap_ssid: String<16>,
    /// Uptime (ms) until which the display says hello, 0 when it doesn't.
    pub identify_until_ms: u64,
}

impl Status {
    const fn new() -> Self {
        Self {
            hashrate_ghs: 0.0,
            hashrate_1m: 0.0,
            hashrate_10m: 0.0,
            hashrate_1h: 0.0,
            expected_hashrate_ghs: 0.0,
            error_percentage: 0.0,
            best_difficulty: 0.0,
            best_difficulty_ever: 0.0,
            total_hashes: 0.0,
            asic_count: 0,
            asic_status: None,
            chip_hashrates: Vec::new(),
            frequency_mhz: 0.0,
            asic_temp_c: f32::NAN,
            chip_temps_c: [f32::NAN; 2],
            vr_temp_c: None,
            fan_rpm: None,
            fan_percent: 0.0,
            power_w: None,
            vin_v: None,
            vcore_v: None,
            current_a: None,
            overheat: false,
            mining_paused: false,
            submitted: 0,
            accepted: 0,
            rejected: 0,
            work_received: 0,
            pool_connected: false,
            pool_difficulty: 0.0,
            response_time_ms: None,
            response_time_min_ms: None,
            response_time_max_ms: None,
            network_difficulty: 0.0,
            coinbase: None,
            pool_url: String::new(),
            pool_port: 0,
            pool_user: String::new(),
            using_fallback: false,
            primary_pool_error: String::new(),
            block_found: 0,
            show_new_block: false,
            ipv4: None,
            wifi_status: String::new(),
            ap_enabled: false,
            ap_ssid: String::new(),
            identify_until_ms: 0,
        }
    }
}

static STATUS: Mutex<CriticalSectionRawMutex, RefCell<Status>> =
    Mutex::new(RefCell::new(Status::new()));

/// The current status.
pub fn get() -> Status {
    STATUS.lock(|status| status.borrow().clone())
}

/// Reads the status with `f`, without copying it.
pub fn with<R>(f: impl FnOnce(&Status) -> R) -> R {
    STATUS.lock(|status| f(&status.borrow()))
}

/// Changes the status with `f`.
pub fn update(f: impl FnOnce(&mut Status)) {
    STATUS.lock(|status| f(&mut status.borrow_mut()));
}

/// Sets a status string, cut to its capacity.
pub fn set_str<const N: usize>(target: &mut String<N>, value: &str) {
    target.clear();
    for c in value.chars() {
        if target.push(c).is_err() {
            break;
        }
    }
}
