//! Core power, ASIC temperature and fan.

use defmt::error;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use esp_hal::{i2c::master::I2c, Async};
use miner_core::thermal;

use crate::{board, cooling::Cooling, vcore::Vcore};

/// A device on the shared I²C bus.
pub type BusDevice = I2cDevice<'static, NoopRawMutex, I2c<'static, Async>>;

/// Signaled once the core voltage is up, so the chips can be brought up.
pub static POWER_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

const MONITOR_PERIOD: Duration = Duration::from_secs(5);
/// Time for the regulator to settle after it is enabled.
const VCORE_SETTLE: Duration = Duration::from_millis(100);

/// Turns the core power on, then watches the ASIC temperature: drives the fan and cuts the core
/// power above [`thermal::OVERHEAT_C`].
#[embassy_executor::task]
pub async fn power_task(mut vcore: Vcore, mut cooling: Cooling) {
    // full speed until the first temperature reading
    cooling.init().await;

    if !vcore.on(board::VCORE).await {
        error!("core power stays off");
        return;
    }
    Timer::after(VCORE_SETTLE).await;
    POWER_READY.signal(());

    loop {
        Timer::after(MONITOR_PERIOD).await;

        let temp = cooling.asic_temp().await;
        vcore.log(temp).await;

        if temp > thermal::OVERHEAT_C {
            vcore.off().await;
            cooling.set_fan(100).await;
            error!(
                "ASIC overheat ({} °C > {} °C): core power cut until reboot",
                temp,
                thermal::OVERHEAT_C
            );
            return;
        }
        cooling.set_fan(thermal::fan_percent(temp)).await;
    }
}
