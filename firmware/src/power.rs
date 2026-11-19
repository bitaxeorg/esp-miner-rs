//! Core power, mining control, ASIC temperature and fan.
//!
//! The chips are powered while mining. Mining stops when paused from the web interface, and on
//! overheat: then the core power is cut and the fan runs at full speed, and once cooled down the
//! chips restart 100 mV and 100 MHz lower.

use core::cell::Cell;

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_futures::select::select;
use embassy_sync::{
    blocking_mutex::{
        raw::{CriticalSectionRawMutex, NoopRawMutex},
        Mutex,
    },
    signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{i2c::master::I2c, Async};
use miner_core::thermal::{self, FanPid};

use crate::{cooling::Cooling, settings, status, vcore::Vcore, wlog};

/// A device on the shared I²C bus.
pub type BusDevice = I2cDevice<'static, NoopRawMutex, I2c<'static, Async>>;

/// Signaled each time the core voltage comes up, so the chips can be brought up.
pub static POWER_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Signaled when the chips must be brought up again, at a new frequency.
pub static RESTART_CHAIN: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Pause and resume, from the web interface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Pause,
    Resume,
}

pub static COMMAND: Signal<CriticalSectionRawMutex, Command> = Signal::new();

/// Whether the chips are powered and may mine.
static MINING: Mutex<CriticalSectionRawMutex, Cell<bool>> = Mutex::new(Cell::new(false));

pub fn mining() -> bool {
    MINING.lock(Cell::get)
}

/// Temperatures and fan are handled this often, the regulator read every
/// [`READINGS_EVERY`] periods.
const PERIOD: Duration = Duration::from_secs(1);
const READINGS_EVERY: u32 = 5;
/// Time for the regulator to settle after it is enabled.
const VCORE_SETTLE: Duration = Duration::from_millis(100);
/// Regulator temperature above which the core power is cut, and below which it may restart.
const VR_OVERHEAT_C: f32 = 105.0;
const VR_COOLED_C: f32 = 95.0;
/// Chip temperature below which the chips may restart after an overheat, and the least time
/// they stay off.
const COOLED_C: f32 = 45.0;
const COOLING_MIN: Duration = Duration::from_secs(30);
/// Steps down after an overheat, and the floors.
const OVERHEAT_VOLTAGE_STEP_MV: u16 = 100;
const OVERHEAT_FREQUENCY_STEP_MHZ: f32 = 100.0;
const MIN_VOLTAGE_MV: u16 = 1000;
const MIN_FREQUENCY_MHZ: f32 = 400.0;
/// Fan duty while paused.
const PAUSED_FAN_PERCENT: f32 = 30.0;

enum State {
    Mining,
    Paused,
    /// Off after an overheat, since then.
    Cooling(Instant),
}

/// Powers the chips at the core voltage of the settings, and lets them mine.
async fn power_on(vcore: &mut Vcore, millivolts: u16) -> bool {
    if !vcore.on(f32::from(millivolts) / 1000.0).await {
        wlog!(Error, "power", "core power stays off");
        return false;
    }
    Timer::after(VCORE_SETTLE).await;
    wlog!(Info, "power", "core power on at {} mV", millivolts);
    MINING.lock(|m| m.set(true));
    POWER_READY.signal(());
    true
}

/// Stops mining and cuts the core power.
async fn power_off(vcore: &mut Vcore) {
    MINING.lock(|m| m.set(false));
    vcore.off().await;
}

/// Turns the core power on, then watches temperatures, drives the fan and follows the pause,
/// resume and settings changes.
#[embassy_executor::task]
pub async fn power_task(mut vcore: Vcore, mut cooling: Cooling) {
    // full speed until the first temperature reading
    cooling.init().await;
    let mut pid = FanPid::new();
    let (overheat_mode, mut voltage_mv, mut frequency) =
        settings::with(|s| (s.overheat_mode, s.core_voltage, s.frequency));
    let mut state = if overheat_mode {
        wlog!(Warn, "power", "overheat mode set: cooling down first");
        State::Cooling(Instant::now())
    } else if power_on(&mut vcore, voltage_mv).await {
        State::Mining
    } else {
        State::Paused
    };
    let mut reading = Default::default();
    let mut periods: u32 = 0;
    let mut last = Instant::now();
    loop {
        if let embassy_futures::select::Either::Second(command) =
            select(Timer::after(PERIOD), COMMAND.wait()).await
        {
            match (command, &state) {
                (Command::Pause, State::Mining) => {
                    power_off(&mut vcore).await;
                    state = State::Paused;
                    wlog!(Info, "power", "mining paused");
                }
                (Command::Resume, State::Paused) => {
                    voltage_mv = settings::with(|s| s.core_voltage);
                    if power_on(&mut vcore, voltage_mv).await {
                        state = State::Mining;
                        wlog!(Info, "power", "mining resumed");
                    }
                }
                _ => {}
            }
        }
        let dt = last.elapsed();
        last = Instant::now();

        let temps = cooling.asic_temp().await;
        let temp = temps.hottest;
        if periods.is_multiple_of(READINGS_EVERY) {
            reading = vcore.log(temp).await;
        }
        periods = periods.wrapping_add(1);
        let fan_rpm = cooling.fan_rpm().await;
        let vr_temp = reading.vr_temp_c;

        let (s_voltage, s_frequency, s_overheat_mode, auto_fan, manual, min_fan, target) =
            settings::with(|s| {
                (
                    s.core_voltage,
                    s.frequency,
                    s.overheat_mode,
                    s.auto_fan_speed,
                    s.manual_fan_speed,
                    s.min_fan_speed,
                    s.temp_target,
                )
            });

        match state {
            State::Mining => {
                let hot = temp > thermal::OVERHEAT_C || vr_temp.is_some_and(|t| t > VR_OVERHEAT_C);
                if hot {
                    power_off(&mut vcore).await;
                    wlog!(
                        Error,
                        "power",
                        "overheat (chips {} °C, regulator {} °C): core power cut",
                        temp,
                        vr_temp.unwrap_or(f32::NAN)
                    );
                    let _ = settings::update(|s| {
                        s.auto_fan_speed = false;
                        s.manual_fan_speed = 100;
                        s.overheat_mode = true;
                    })
                    .await;
                    state = State::Cooling(Instant::now());
                } else if s_voltage != voltage_mv {
                    // a new core voltage: power the chips again at it
                    power_off(&mut vcore).await;
                    voltage_mv = s_voltage;
                    if !power_on(&mut vcore, voltage_mv).await {
                        state = State::Paused;
                    }
                } else if s_frequency != frequency {
                    frequency = s_frequency;
                    wlog!(Info, "power", "frequency now {} MHz", frequency);
                    RESTART_CHAIN.signal(());
                }
            }
            State::Paused => {
                voltage_mv = s_voltage;
                frequency = s_frequency;
            }
            State::Cooling(since) => {
                let cooled = since.elapsed() >= COOLING_MIN
                    && (temp.is_nan() || temp <= COOLED_C)
                    && vr_temp.is_none_or(|t| t <= VR_COOLED_C);
                if !s_overheat_mode {
                    // cleared from the web interface: restart at the settings as they are
                    voltage_mv = s_voltage;
                    frequency = s_frequency;
                    if power_on(&mut vcore, voltage_mv).await {
                        state = State::Mining;
                    }
                } else if cooled {
                    voltage_mv = s_voltage
                        .saturating_sub(OVERHEAT_VOLTAGE_STEP_MV)
                        .max(MIN_VOLTAGE_MV);
                    frequency = (s_frequency - OVERHEAT_FREQUENCY_STEP_MHZ).max(MIN_FREQUENCY_MHZ);
                    let (v, f) = (voltage_mv, frequency);
                    let _ = settings::update(|s| {
                        s.core_voltage = v;
                        s.frequency = f;
                        s.overheat_mode = false;
                    })
                    .await;
                    wlog!(
                        Warn,
                        "power",
                        "cooled down: restarting at {} mV, {} MHz",
                        voltage_mv,
                        frequency
                    );
                    if power_on(&mut vcore, voltage_mv).await {
                        state = State::Mining;
                    }
                }
            }
        }

        let fan = match state {
            State::Cooling(_) => 100.0,
            State::Paused => PAUSED_FAN_PERCENT,
            State::Mining if auto_fan => pid.update(
                temp,
                f32::from(target),
                f32::from(min_fan),
                dt.as_millis() as f32 / 1000.0,
            ),
            State::Mining => f32::from(manual.min(100)),
        };
        cooling.set_fan(fan as u8).await;

        status::update(|s| {
            s.asic_temp_c = temp;
            s.chip_temps_c = temps.chips;
            s.vr_temp_c = vr_temp;
            s.fan_rpm = fan_rpm;
            s.fan_percent = fan;
            s.power_w = reading.power_w;
            s.vin_v = reading.vin_v;
            s.vcore_v = reading.vcore_v;
            s.current_a = reading.current_a;
            s.overheat = matches!(state, State::Cooling(_));
            s.mining_paused = matches!(state, State::Paused);
        });
    }
}
