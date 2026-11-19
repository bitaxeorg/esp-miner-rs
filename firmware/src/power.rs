//! Core power, ASIC temperature and fan.

use bounded_integer::BoundedU8;
use defmt::{error, info};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use emc2101::{AsyncEMC2101, BetaCompensation};
use esp_hal::{i2c::master::I2c, Async};
use fugit::HertzU32;
use miner_core::thermal;

use crate::{board, vcore::Vcore};

/// A device on the shared I²C bus.
pub type BusDevice = I2cDevice<'static, NoopRawMutex, I2c<'static, Async>>;

/// Signaled once the core voltage is up, so the chips can be brought up.
pub static POWER_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

const MONITOR_PERIOD: Duration = Duration::from_secs(5);
/// Time for the regulator to settle after it is enabled.
const VCORE_SETTLE: Duration = Duration::from_millis(100);
const FAN_PWM_FREQ: HertzU32 = HertzU32::Hz(5806);

/// Turns the core power on, then watches the ASIC temperature: drives the fan and cuts the core
/// power above [`thermal::OVERHEAT_C`].
#[embassy_executor::task]
pub async fn power_task(mut vcore: Vcore, mut sensor: AsyncEMC2101<BusDevice>) {
    // full speed until the first temperature reading
    if let Err(e) = sensor.set_fan_pwm(FAN_PWM_FREQ, false).await {
        error!("fan PWM setup failed: {}", e);
    }
    set_fan(&mut sensor, 100).await;
    if let Some((ideality, beta)) = board::EMC2101_DIODE {
        configure_diode(&mut sensor, ideality, beta).await;
    }

    if !vcore.on(board::VCORE).await {
        error!("core power stays off");
        return;
    }
    Timer::after(VCORE_SETTLE).await;
    POWER_READY.signal(());

    loop {
        Timer::after(MONITOR_PERIOD).await;

        let temp = asic_temp(&mut sensor).await;
        vcore.log(temp).await;

        if temp > thermal::OVERHEAT_C {
            vcore.off().await;
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

/// Sets the EMC2101 external diode ideality factor and beta compensation to the chip's diode.
async fn configure_diode(sensor: &mut AsyncEMC2101<BusDevice>, ideality: u8, beta: u8) {
    let (Some(ideality), Ok(beta)) = (BoundedU8::new(ideality), BetaCompensation::try_from(beta))
    else {
        error!(
            "invalid EMC2101 diode settings: ideality {:x}, beta {:x}",
            ideality, beta
        );
        return;
    };
    if let Err(e) = sensor.set_ideality_factor(ideality).await {
        error!("setting the EMC2101 ideality factor failed: {}", e);
    }
    if let Err(e) = sensor.set_beta_compensation(beta).await {
        error!("setting the EMC2101 beta compensation failed: {}", e);
    }
    info!(
        "EMC2101 diode: ideality {:x}, beta {:x}",
        ideality.get(),
        beta as u8
    );
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
