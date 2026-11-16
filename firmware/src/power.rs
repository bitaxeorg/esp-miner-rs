//! Core power, ASIC temperature and fan.

use bounded_integer::BoundedU8;
use defmt::{error, info, warn};
use ds4432::{AsyncDS4432, Status};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use emc2101::AsyncEMC2101;
use esp_hal::{gpio::Output, i2c::master::I2c, Async};
use fugit_06::HertzU32;
use ina260::AsyncINA260;
use miner_core::{thermal, vcore};

use crate::board;

/// A device on the shared I²C bus.
pub type BusDevice = I2cDevice<'static, NoopRawMutex, I2c<'static, Async>>;

/// Signaled once the core voltage is up, so the chips can be brought up.
pub static POWER_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

const MONITOR_PERIOD: Duration = Duration::from_secs(5);
/// Time for the regulator to settle after it is enabled.
const VCORE_SETTLE: Duration = Duration::from_millis(100);
const FAN_PWM_FREQ: HertzU32 = HertzU32::Hz(5806);

/// Sets the core voltage, enables the regulator, then watches the ASIC temperature: drives the fan
/// and cuts the core power above [`thermal::OVERHEAT_C`].
///
/// `asic_enable` is the regulator enable (PWR_EN), active low.
#[embassy_executor::task]
pub async fn power_task(
    mut dac: AsyncDS4432<BusDevice>,
    mut monitor: AsyncINA260<BusDevice>,
    mut sensor: AsyncEMC2101<BusDevice>,
    mut asic_enable: Output<'static>,
) {
    // full speed until the first temperature reading
    if let Err(e) = sensor.set_fan_pwm(FAN_PWM_FREQ, false).await {
        error!("fan PWM setup failed: {}", e);
    }
    set_fan(&mut sensor, 100).await;

    // Set the DAC before enabling the regulator, so it starts at the target voltage instead of
    // its nominal ~1.5 V.
    let current_ua =
        vcore::feedback_current_ua(board::VCORE, board::VCORE_RA_OHM, board::VCORE_RB_OHM);
    let status = if current_ua >= 0.0 {
        Status::SourceMicroAmp(current_ua)
    } else {
        Status::SinkMicroAmp(-current_ua)
    };
    if let Err(e) = dac.set_status(ds4432::Output::Zero, status).await {
        error!(
            "setting the core voltage failed, core power stays off: {}",
            e
        );
        return;
    }
    asic_enable.set_low();
    info!("core voltage set to {} V", board::VCORE);
    Timer::after(VCORE_SETTLE).await;
    POWER_READY.signal(());

    loop {
        Timer::after(MONITOR_PERIOD).await;

        let temp = asic_temp(&mut sensor).await;
        match (
            monitor.power().await,
            monitor.current().await,
            monitor.voltage().await,
        ) {
            (Ok(mw), Ok(ua), Ok(uv)) => info!(
                "ASIC {} °C, input {} mW ({} mV, {} mA)",
                temp,
                mw,
                uv / 1000,
                ua / 1000
            ),
            _ => warn!("ASIC {} °C, reading the INA260 failed", temp),
        }

        if temp > thermal::OVERHEAT_C {
            asic_enable.set_high();
            set_fan(&mut sensor, 100).await;
            error!(
                "ASIC overheat ({} °C > {} °C): core power cut until reboot",
                temp,
                thermal::OVERHEAT_C
            );
            return;
        }
        set_fan(&mut sensor, thermal::fan_percent(temp)).await;
    }
}

/// ASIC temperature (°C), NaN when it can't be read.
async fn asic_temp(sensor: &mut AsyncEMC2101<BusDevice>) -> f32 {
    let temp = if board::ASIC_TEMP_INTERNAL {
        sensor.temp_internal().await.map(f32::from)
    } else {
        sensor.temp_external_precise().await
    };
    match temp {
        Ok(t) => t + board::ASIC_TEMP_OFFSET_C,
        Err(e) => {
            error!("reading the ASIC temperature failed: {}", e);
            f32::NAN
        }
    }
}

async fn set_fan(sensor: &mut AsyncEMC2101<BusDevice>, percent: u8) {
    let step = BoundedU8::new(thermal::emc2101_fan_step(percent)).unwrap_or(BoundedU8::MAX);
    if let Err(e) = sensor.set_fan_power(step).await {
        error!("setting the fan to {}% failed: {}", percent, e);
    }
}
