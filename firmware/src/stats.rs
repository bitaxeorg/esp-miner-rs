//! Statistics history, scoreboard, and the saving of the all-time best difficulty and the
//! scoreboard.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::{Duration, Instant, Timer};
use miner_core::{
    scoreboard::{Entry, Scoreboard},
    stats::{Column, StatsRing, Values, COLUMNS},
};

use crate::{net, settings, status};

/// Samples kept.
pub const STATS_LIMIT: usize = 300;
const PERIOD: Duration = Duration::from_secs(1);
/// How often the best difficulty and the scoreboard are saved, when they changed.
const SAVE_PERIOD: Duration = Duration::from_secs(60);

static RING: Mutex<CriticalSectionRawMutex, RefCell<StatsRing<STATS_LIMIT>>> =
    Mutex::new(RefCell::new(StatsRing::new()));

struct Board {
    board: Scoreboard,
    changed: bool,
}

static BOARD: Mutex<CriticalSectionRawMutex, RefCell<Board>> = Mutex::new(RefCell::new(Board {
    board: Scoreboard::new(),
    changed: false,
}));

/// Offers a valid nonce to the scoreboard.
pub fn score(entry: Entry) {
    BOARD.lock(|b| {
        let mut b = b.borrow_mut();
        if b.board.consider(entry) {
            b.changed = true;
        }
    });
}

pub fn with_scoreboard<R>(f: impl FnOnce(&Scoreboard) -> R) -> R {
    BOARD.lock(|b| f(&b.borrow().board))
}

pub fn with_ring<R>(f: impl FnOnce(&StatsRing<STATS_LIMIT>) -> R) -> R {
    RING.lock(|r| f(&r.borrow()))
}

/// Free heap (bytes).
pub fn free_heap() -> usize {
    esp_alloc::HEAP.free()
}

/// The values of a sample, from the status.
fn sample() -> Values {
    let mut v: Values = [None; COLUMNS];
    let rssi = net::wifi_rssi();
    status::with(|s| {
        let mut set = |c: Column, value: Option<f32>| v[c as usize] = value;
        set(Column::Hashrate, Some(s.hashrate_ghs));
        set(Column::Hashrate1m, Some(s.hashrate_1m));
        set(Column::Hashrate10m, Some(s.hashrate_10m));
        set(Column::Hashrate1h, Some(s.hashrate_1h));
        set(Column::ErrorPercentage, Some(s.error_percentage));
        set(
            Column::AsicTemp,
            Some(s.chip_temps_c[0]).filter(|t| !t.is_nan()),
        );
        set(
            Column::AsicTemp2,
            Some(s.chip_temps_c[1]).filter(|t| !t.is_nan()),
        );
        set(Column::VrTemp, s.vr_temp_c);
        set(Column::AsicVoltage, s.vcore_v.map(|v| v * 1000.0));
        set(Column::Voltage, s.vin_v.map(|v| v * 1000.0));
        set(Column::Power, s.power_w);
        set(Column::Current, s.current_a.map(|a| a * 1000.0));
        set(Column::FanSpeed, Some(s.fan_percent));
        set(Column::FanRpm, s.fan_rpm.map(f32::from));
        set(Column::Fan2Rpm, None);
        set(Column::WifiRssi, rssi.map(f32::from));
        set(Column::FreeHeap, Some(free_heap() as f32));
        set(Column::ResponseTimeMin, s.response_time_min_ms);
        set(Column::ResponseTimeMax, s.response_time_max_ms);
    });
    v
}

/// Takes a sample every `statsFrequency` seconds, and saves the best difficulty and the
/// scoreboard when they changed.
#[embassy_executor::task]
pub async fn stats_task() {
    // the saved best difficulty and scoreboard
    let (best, entries) = settings::with(|s| (s.best_diff, s.scoreboard.clone()));
    status::update(|s| s.best_difficulty_ever = best as f64);
    BOARD.lock(|b| {
        let mut b = b.borrow_mut();
        for e in entries.iter().filter_map(|e| Entry::decode(e)) {
            b.board.consider(e);
        }
    });

    let mut last_sample: Option<Instant> = None;
    let mut last_save = Instant::now();
    loop {
        Timer::after(PERIOD).await;
        let frequency = settings::with(|s| s.stats_frequency);
        if frequency == 0 {
            RING.lock(|r| r.borrow_mut().clear());
            last_sample = None;
        } else if last_sample
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(u64::from(frequency)))
        {
            let values = sample();
            let now = Instant::now();
            RING.lock(|r| r.borrow_mut().push(now.as_millis() as u32, &values));
            status::update(|s| {
                s.response_time_min_ms = None;
                s.response_time_max_ms = None;
            });
            last_sample = Some(now);
        }

        if last_save.elapsed() >= SAVE_PERIOD {
            last_save = Instant::now();
            let best = status::with(|s| s.best_difficulty_ever) as u64;
            let changed = BOARD.lock(|b| core::mem::take(&mut b.borrow_mut().changed));
            let mut stored: [alloc::string::String; miner_core::scoreboard::SCOREBOARD_SIZE] =
                Default::default();
            if changed {
                with_scoreboard(|board| {
                    for (slot, e) in stored.iter_mut().zip(board.entries()) {
                        *slot = e.encode().as_str().into();
                    }
                });
            }
            if changed || best > settings::with(|s| s.best_diff) {
                let _ = settings::update(|s| {
                    s.best_diff = s.best_diff.max(best);
                    if changed {
                        s.scoreboard = stored;
                    }
                })
                .await;
            }
        }
    }
}
