use defmt::debug;
use embassy_time::{Duration, Timer};

const TPS40305_VFB: f32 = 0.6;
#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
const TPS40305_RA: f32 = 4990.0; // R14
#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
const TPS40305_RB: f32 = 3320.0; // R15

trait SetVCore {
    async fn set_vcore(&mut self, vcore: f32);
}

#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
impl<I: embedded_hal_async::i2c::I2c + embedded_hal_async::i2c::ErrorType> SetVCore
    for ds4432::AsyncDS4432<I>
{
    async fn set_vcore(&mut self, vcore: f32) {
        let irb_ua = TPS40305_VFB * 1_000_000.0 / TPS40305_RB;
        let ira_ua = (vcore - TPS40305_VFB) * 1_000_000.0 / TPS40305_RA;
        let status = if irb_ua > ira_ua {
            ds4432::Status::SourceMicroAmp(irb_ua - ira_ua)
        } else {
            ds4432::Status::SinkMicroAmp(ira_ua - irb_ua)
        };
        let _ = self.set_status(ds4432::Output::Zero, status).await;
    }
}

trait MeasureVCore {
    async fn measure_vcore(&mut self) -> Option<f32>;
}

#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
impl<I: embedded_hal_async::i2c::I2c + embedded_hal_async::i2c::ErrorType> MeasureVCore
    for ina260::AsyncINA260<I>
{
    async fn measure_vcore(&mut self) -> Option<f32> {
        self.voltage().await.map(|v| (v as f32) / 1_000_000.0).ok()
    }
}

// trait MeasureICore {
//     async fn measure_icore(&mut self) -> Option<f32>;
// }

// #[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
// impl<I: embedded_hal_async::i2c::I2c + embedded_hal_async::i2c::ErrorType> MeasureICore
//     for ina260::AsyncINA260<I>
// {
//     async fn measure_icore(&mut self) -> Option<f32> {
//         self.current().await.map(|v| (v as f32) / 1_000_000.0).ok()
//     }
// }

#[embassy_executor::task]
pub async fn vcore_task(
    target_vcore: f32,
    mut setter: impl SetVCore + 'static,
    mut measurer: Option<impl MeasureVCore + 'static>,
) {
    debug!("start vcore task");
    let mut delta = 0.0;
    loop {
        if let Some(measurer) = measurer.as_mut() {
            if let Some(measured_vcore) = measurer.measure_vcore().await {
                delta = measured_vcore - target_vcore;
            }
        }
        setter.set_vcore(target_vcore + delta).await;
        Timer::after(Duration::from_millis(1_000)).await;
    }
}
