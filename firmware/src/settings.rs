//! Persistent settings, in the NVS partition, namespace `main`, with the keys and value types of
//! the stock firmware, so that both read each other's settings.

use alloc::{format, string::String};
use core::cell::RefCell;

use defmt::{error, info, warn};
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use esp_nvs::{Get, Key, Nvs};
use miner_core::scoreboard::SCOREBOARD_SIZE;

use crate::{board, flash, pool};

pub const MAX_POOLS: usize = 8;
const NAMESPACE: Key = Key::from_str("main");

/// The settings. Strings hold what the web API accepts; `pools` holds each pool as the JSON
/// object the web API exchanges, empty for an unused slot.
#[derive(Clone, PartialEq)]
pub struct Settings {
    pub ssid: String,
    pub wifi_pass: String,
    pub hostname: String,
    pub use_ntp: bool,
    pub pools: [String; MAX_POOLS],
    pub primary_pool_index: u16,
    pub secondary_pool_index: u16,
    pub use_fallback_stratum: bool,
    /// ASIC frequency (MHz) and core voltage (mV).
    pub frequency: f32,
    pub core_voltage: u16,
    pub overclock_enabled: bool,
    pub display: String,
    /// Rotation in degrees: 0, 90, 180 or 270.
    pub rotation: u16,
    pub invert_screen: bool,
    pub display_offset: u16,
    /// Minutes without a button press before the display turns off; 0 after 5 s, -1 never.
    pub display_timeout: i32,
    pub auto_fan_speed: bool,
    /// Fan duty cycle (%) when not automatic, and the lowest the automatic control goes to.
    pub manual_fan_speed: u16,
    pub min_fan_speed: u16,
    /// Chip temperature the automatic fan control holds (°C).
    pub temp_target: u16,
    /// Set when the core power was cut on overheat: the chips stay off until it is cleared.
    pub overheat_mode: bool,
    /// Seconds between statistics samples, 0 for none.
    pub stats_frequency: u16,
    /// Best share difficulty ever.
    pub best_diff: u64,
    pub theme_scheme: String,
    pub theme_color: String,
    /// The scoreboard entries, in their stored form, empty when unused.
    pub scoreboard: [String; SCOREBOARD_SIZE],
}

impl Settings {
    fn defaults() -> Self {
        let mut pools: [String; MAX_POOLS] = Default::default();
        pools[0] = format!(
            concat!(
                r#"{{"stratumProtocol":"SV1","stratumURL":"{}","stratumPort":{},"#,
                r#""stratumUser":"{}","stratumPassword":"{}","stratumSuggestedDifficulty":{},"#,
                r#""stratumExtranonceSubscribe":false,"stratumTLS":0,"stratumCert":"","#,
                r#""stratumDecodeCoinbase":true}}"#
            ),
            pool::DEFAULT_URL,
            pool::DEFAULT_PORT,
            pool::DEFAULT_USER,
            pool::DEFAULT_PASSWORD,
            pool::DEFAULT_DIFFICULTY,
        );
        Self {
            ssid: option_env!("WIFI_SSID").unwrap_or("").into(),
            wifi_pass: option_env!("WIFI_PASSWORD").unwrap_or("").into(),
            hostname: "bitaxe".into(),
            use_ntp: false,
            pools,
            primary_pool_index: 0,
            secondary_pool_index: 1,
            use_fallback_stratum: false,
            frequency: board::HASH_FREQ_MHZ as f32,
            core_voltage: (board::VCORE * 1000.0 + 0.5) as u16,
            overclock_enabled: false,
            display: board::DISPLAY.into(),
            rotation: 0,
            invert_screen: false,
            display_offset: 0,
            display_timeout: -1,
            auto_fan_speed: true,
            manual_fan_speed: 100,
            min_fan_speed: 25,
            temp_target: 60,
            overheat_mode: false,
            stats_frequency: 0,
            best_diff: 0,
            theme_scheme: "dark".into(),
            theme_color: "#F80421".into(),
            scoreboard: Default::default(),
        }
    }
}

static SETTINGS: Mutex<CriticalSectionRawMutex, RefCell<Option<Settings>>> =
    Mutex::new(RefCell::new(None));

/// The settings, read with `f`. Panics before [`load`].
pub fn with<R>(f: impl FnOnce(&Settings) -> R) -> R {
    SETTINGS.lock(|s| f(s.borrow().as_ref().expect("settings not loaded")))
}

/// A copy of the settings.
pub fn get() -> Settings {
    with(Settings::clone)
}

/// Changes the settings with `f` and saves what changed. The new settings are in use even if
/// saving fails, which is reported.
pub async fn update(f: impl FnOnce(&mut Settings)) -> Result<(), ()> {
    let (old, new) = SETTINGS.lock(|s| {
        let mut s = s.borrow_mut();
        let current = s.as_mut().expect("settings not loaded");
        let old = current.clone();
        f(current);
        (old, current.clone())
    });
    if old == new {
        return Ok(());
    }
    save(&old, &new).await
}

/// Value types of the settings, as they are stored.
enum Value<'a> {
    Str(&'a str),
    U16(u16),
    I32(i32),
    U64(u64),
    /// Stored as a `u16`, 0 or 1.
    Bool(bool),
    /// Stored as a string, `%f`.
    Float(f32),
}

/// Calls `f` with the NVS key and value of each setting.
fn for_each(s: &Settings, mut f: impl FnMut(&str, Value)) {
    f("wifissid", Value::Str(&s.ssid));
    f("wifipass", Value::Str(&s.wifi_pass));
    f("hostname", Value::Str(&s.hostname));
    f("usentp", Value::Bool(s.use_ntp));
    for (i, p) in s.pools.iter().enumerate() {
        f(&pool_key(i), Value::Str(p));
    }
    f("prim_idx", Value::U16(s.primary_pool_index));
    f("sec_idx", Value::U16(s.secondary_pool_index));
    f("usefbstartum", Value::Bool(s.use_fallback_stratum));
    f("asicfrequency_f", Value::Float(s.frequency));
    f("asicvoltage", Value::U16(s.core_voltage));
    f("oc_enabled", Value::Bool(s.overclock_enabled));
    f("display", Value::Str(&s.display));
    f("rotation", Value::U16(s.rotation));
    f("invertscreen", Value::Bool(s.invert_screen));
    f("displayOffset", Value::U16(s.display_offset));
    f("displayTimeout", Value::I32(s.display_timeout));
    f("autofanspeed", Value::Bool(s.auto_fan_speed));
    f("manualfanspeed", Value::U16(s.manual_fan_speed));
    f("minfanspeed", Value::U16(s.min_fan_speed));
    f("temptarget", Value::U16(s.temp_target));
    f("overheat_mode", Value::Bool(s.overheat_mode));
    f("statsFrequency", Value::U16(s.stats_frequency));
    f("bestdiff", Value::U64(s.best_diff));
    f("themescheme", Value::Str(&s.theme_scheme));
    f("themecolor", Value::Str(&s.theme_color));
    for (i, e) in s.scoreboard.iter().enumerate() {
        f(&indexed_key("scoreboard", i, 2), Value::Str(e));
    }
}

/// Key of pool slot `i`: `pool_1` to `pool_8`.
fn pool_key(i: usize) -> heapless::String<16> {
    indexed_key("pool", i, 1)
}

/// Key of entry `i` of an array setting: `name_` then `i + 1` on `width` digits.
fn indexed_key(name: &str, i: usize, width: usize) -> heapless::String<16> {
    let mut key = heapless::String::new();
    let _ = core::fmt::write(&mut key, format_args!("{name}_{:0width$}", i + 1));
    key
}

/// Reads the settings, keeping the default of each one missing or unreadable.
pub async fn load() {
    let mut s = Settings::defaults();
    if let Some(region) = flash::nvs_partition() {
        let mut guard = flash::lock().await;
        let storage = guard.as_mut().expect("flash not initialized");
        match Nvs::new(
            region.offset as usize,
            region.size as usize,
            flash::NvsFlash(storage),
        ) {
            Ok(mut nvs) => {
                let mut found = 0;
                let defaults = s.clone();
                // read into a copy, then write each value found into `s`
                let mut read = |key: &str, value: Value| {
                    let k = Key::from_str(key);
                    let ok = match value {
                        Value::Str(_) => Get::<String>::get(&mut nvs, &NAMESPACE, &k)
                            .map(|v| set_str(&mut s, key, v))
                            .is_ok(),
                        Value::U16(_) | Value::Bool(_) => Get::<u16>::get(&mut nvs, &NAMESPACE, &k)
                            .map(|v| set_u16(&mut s, key, v))
                            .is_ok(),
                        Value::I32(_) => Get::<i32>::get(&mut nvs, &NAMESPACE, &k)
                            .map(|v| set_i32(&mut s, key, v))
                            .is_ok(),
                        Value::U64(_) => Get::<u64>::get(&mut nvs, &NAMESPACE, &k)
                            .map(|v| set_u64(&mut s, key, v))
                            .is_ok(),
                        Value::Float(_) => Get::<String>::get(&mut nvs, &NAMESPACE, &k)
                            .ok()
                            .and_then(|v| v.trim().parse::<f32>().ok())
                            .map(|v| set_f32(&mut s, key, v))
                            .is_some(),
                    };
                    found += ok as usize;
                };
                for_each(&defaults, &mut read);
                info!("settings: {} values read", found);
            }
            Err(e) => error!("NVS unreadable, default settings: {}", e),
        }
    } else {
        warn!("no NVS partition, default settings");
    }
    SETTINGS.lock(|cell| *cell.borrow_mut() = Some(s));
}

/// Writes the settings of `new` that differ from `old`.
async fn save(old: &Settings, new: &Settings) -> Result<(), ()> {
    let Some(region) = flash::nvs_partition() else {
        return Err(());
    };
    let mut old_values = heapless::Vec::<(heapless::String<16>, u64), 64>::new();
    for_each(old, |key, value| {
        let mut k = heapless::String::new();
        let _ = k.push_str(key);
        let _ = old_values.push((k, fingerprint(&value)));
    });
    let mut guard = flash::lock().await;
    let storage = guard.as_mut().expect("flash not initialized");
    let mut nvs = Nvs::new(
        region.offset as usize,
        region.size as usize,
        flash::NvsFlash(storage),
    )
    .map_err(|e| error!("NVS unreadable: {}", e))?;
    let mut result = Ok(());
    for_each(new, |key, value| {
        let unchanged = old_values
            .iter()
            .any(|(k, f)| k.as_str() == key && *f == fingerprint(&value));
        if unchanged {
            return;
        }
        let k = Key::from_str(key);
        let written = match value {
            Value::Str(v) => nvs.set(&NAMESPACE, &k, v),
            Value::U16(v) => nvs.set(&NAMESPACE, &k, v),
            Value::Bool(v) => nvs.set(&NAMESPACE, &k, v as u16),
            Value::I32(v) => nvs.set(&NAMESPACE, &k, v),
            Value::U64(v) => nvs.set(&NAMESPACE, &k, v),
            Value::Float(v) => nvs.set(&NAMESPACE, &k, format!("{v:.6}").as_str()),
        };
        match written {
            Ok(()) => info!("setting {} saved", key),
            Err(e) => {
                error!("setting {} not saved: {}", key, e);
                result = Err(());
            }
        }
    });
    result
}

/// A value reduced to a number that changes when the value does.
fn fingerprint(value: &Value) -> u64 {
    match *value {
        Value::Str(s) => {
            // FNV-1a
            s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
            })
        }
        Value::U16(v) => v.into(),
        Value::Bool(v) => v.into(),
        Value::I32(v) => v as u32 as u64,
        Value::U64(v) => v,
        Value::Float(v) => v.to_bits().into(),
    }
}

fn set_str(s: &mut Settings, key: &str, v: String) {
    match key {
        "wifissid" => s.ssid = v,
        "wifipass" => s.wifi_pass = v,
        "hostname" => s.hostname = v,
        "display" => s.display = v,
        "themescheme" => s.theme_scheme = v,
        "themecolor" => s.theme_color = v,
        _ => {
            if let Some(i) = key
                .strip_prefix("scoreboard_")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=SCOREBOARD_SIZE).contains(n))
            {
                s.scoreboard[i - 1] = v;
            } else if let Some(i) = key
                .strip_prefix("pool_")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=MAX_POOLS).contains(n))
            {
                s.pools[i - 1] = v;
            }
        }
    }
}

fn set_u16(s: &mut Settings, key: &str, v: u16) {
    match key {
        "usentp" => s.use_ntp = v != 0,
        "prim_idx" => s.primary_pool_index = v,
        "sec_idx" => s.secondary_pool_index = v,
        "usefbstartum" => s.use_fallback_stratum = v != 0,
        "asicvoltage" => s.core_voltage = v,
        "oc_enabled" => s.overclock_enabled = v != 0,
        "rotation" => s.rotation = v,
        "invertscreen" => s.invert_screen = v != 0,
        "displayOffset" => s.display_offset = v,
        "autofanspeed" => s.auto_fan_speed = v != 0,
        "manualfanspeed" => s.manual_fan_speed = v,
        "minfanspeed" => s.min_fan_speed = v,
        "temptarget" => s.temp_target = v,
        "overheat_mode" => s.overheat_mode = v != 0,
        "statsFrequency" => s.stats_frequency = v,
        _ => {}
    }
}

fn set_i32(s: &mut Settings, key: &str, v: i32) {
    if key == "displayTimeout" {
        s.display_timeout = v;
    }
}

fn set_u64(s: &mut Settings, key: &str, v: u64) {
    if key == "bestdiff" {
        s.best_diff = v;
    }
}

fn set_f32(s: &mut Settings, key: &str, v: f32) {
    if key == "asicfrequency_f" {
        s.frequency = v;
    }
}
