//! ASIC temperature and fan.
//!
//! - `fan-emc2101`: one ASIC, whose temperature is the EMC2101 internal sensor plus an offset
//!   (`asic-temp-internal`) or the chip's diode on the external channel.
//! - `fan-emc2103`: one ASIC on each external diode channel, with the ideality and beta of
//!   `board::EMC2103_DIODE` when set, the fan driven by direct setting.

#[cfg(feature = "fan-emc2101")]
pub use emc2101_fan::Cooling;
#[cfg(feature = "fan-emc2103")]
pub use emc2103_fan::Cooling;

#[cfg(feature = "fan-emc2101")]
mod emc2101_fan {
    use bounded_integer::BoundedU8;
    use defmt::{error, info};
    use emc2101::{AsyncEMC2101, BetaCompensation};
    use fugit::HertzU32;
    use miner_core::thermal;

    use crate::{board, power::BusDevice};

    const FAN_PWM_FREQ: HertzU32 = HertzU32::Hz(5806);

    pub struct Cooling {
        sensor: AsyncEMC2101<BusDevice>,
    }

    impl Cooling {
        pub fn new(sensor: AsyncEMC2101<BusDevice>) -> Self {
            Self { sensor }
        }

        /// Sets up the fan output at full speed and the temperature channel.
        pub async fn init(&mut self) {
            if let Err(e) = self.sensor.set_fan_pwm(FAN_PWM_FREQ, false).await {
                error!("fan PWM setup failed: {}", e);
            }
            self.set_fan(100).await;
            if let Some((ideality, beta)) = board::EMC2101_DIODE {
                self.configure_diode(ideality, beta).await;
            }
        }

        /// ASIC temperature (°C), NaN when it can't be read.
        pub async fn asic_temp(&mut self) -> f32 {
            let temp = if board::ASIC_TEMP_INTERNAL {
                self.sensor.temp_internal().await.map(f32::from)
            } else {
                self.sensor.temp_external_precise().await
            };
            match temp {
                Ok(t) => t + board::ASIC_TEMP_OFFSET_C,
                Err(e) => {
                    error!("reading the ASIC temperature failed: {}", e);
                    f32::NAN
                }
            }
        }

        pub async fn set_fan(&mut self, percent: u8) {
            let step = BoundedU8::new(thermal::emc2101_fan_step(percent)).unwrap_or(BoundedU8::MAX);
            if let Err(e) = self.sensor.set_fan_power(step).await {
                error!("setting the fan to {}% failed: {}", percent, e);
            }
        }

        /// Sets the external diode ideality factor and beta compensation to the chip's diode.
        async fn configure_diode(&mut self, ideality: u8, beta: u8) {
            let (Some(ideality), Ok(beta)) =
                (BoundedU8::new(ideality), BetaCompensation::try_from(beta))
            else {
                error!(
                    "invalid EMC2101 diode settings: ideality {:x}, beta {:x}",
                    ideality, beta
                );
                return;
            };
            if let Err(e) = self.sensor.set_ideality_factor(ideality).await {
                error!("setting the EMC2101 ideality factor failed: {}", e);
            }
            if let Err(e) = self.sensor.set_beta_compensation(beta).await {
                error!("setting the EMC2101 beta compensation failed: {}", e);
            }
            info!(
                "EMC2101 diode: ideality {:x}, beta {:x}",
                ideality.get(),
                beta as u8
            );
        }
    }
}

#[cfg(feature = "fan-emc2103")]
mod emc2103_fan {
    use bounded_integer::BoundedU8;
    use defmt::{debug, error, info};
    use emc2103::{AsyncEMC2103, BetaCompensation, Channel, ExternalDiode};

    use crate::{board, power::BusDevice};

    pub struct Cooling {
        sensor: AsyncEMC2103<BusDevice>,
    }

    impl Cooling {
        pub fn new(sensor: AsyncEMC2103<BusDevice>) -> Self {
            Self { sensor }
        }

        /// Sets up the fan output (open drain, normal polarity, direct setting) at full speed and
        /// the temperature channels.
        pub async fn init(&mut self) {
            if let Err(e) = self.sensor.set_pwm_output(false, false).await {
                error!("fan PWM setup failed: {}", e);
            }
            if let Err(e) = self.sensor.disable_lut().await {
                error!("disabling the fan Look-Up Table failed: {}", e);
            }
            if let Err(e) = self.sensor.set_fan_direct().await {
                error!("setting the fan to direct drive failed: {}", e);
            }
            self.set_fan(100).await;
            if let Some((ideality, beta)) = board::EMC2103_DIODE {
                self.configure_diodes(ideality, beta).await;
            }
        }

        /// Temperature of the hottest ASIC (°C), NaN when one can't be read.
        pub async fn asic_temp(&mut self) -> f32 {
            // External Diode 1 is on the first chip of the chain, or the second one when flipped.
            let (first, second) = if board::EMC2103_TEMP_FLIP {
                (Channel::External2, Channel::External1)
            } else {
                (Channel::External1, Channel::External2)
            };
            let first = self.chip_temp(first).await;
            let second = self.chip_temp(second).await;
            debug!("ASIC 0 {} °C, ASIC 1 {} °C", first, second);
            if first.is_nan() || second.is_nan() {
                f32::NAN
            } else {
                first.max(second)
            }
        }

        pub async fn set_fan(&mut self, percent: u8) {
            let percent = BoundedU8::new(percent).unwrap_or(BoundedU8::MAX);
            if let Err(e) = self.sensor.set_fan_percent(percent).await {
                error!("setting the fan to {}% failed: {}", percent.get(), e);
            }
        }

        /// Sets the ideality factor and beta compensation of both external channels to the chips'
        /// diodes.
        async fn configure_diodes(&mut self, ideality: u8, beta: BetaCompensation) {
            let Some(ideality) = BoundedU8::new(ideality) else {
                error!("invalid EMC2103 ideality factor {:x}", ideality);
                return;
            };
            for diode in [ExternalDiode::External1, ExternalDiode::External2] {
                if let Err(e) = self.sensor.set_ideality_factor(diode, ideality).await {
                    error!(
                        "setting the EMC2103 {} ideality factor failed: {}",
                        diode, e
                    );
                }
                if let Err(e) = self.sensor.set_beta_compensation(diode, beta).await {
                    error!(
                        "setting the EMC2103 {} beta compensation failed: {}",
                        diode, e
                    );
                }
            }
            info!(
                "EMC2103 diodes: ideality {:x}, beta {}",
                ideality.get(),
                beta
            );
        }

        /// Temperature of the chip on `channel` (°C), NaN when it can't be read.
        async fn chip_temp(&mut self, channel: Channel) -> f32 {
            match self.sensor.temperature(channel).await {
                Ok(t) => t,
                Err(e) => {
                    error!("reading the ASIC temperature on {} failed: {}", channel, e);
                    f32::NAN
                }
            }
        }
    }
}
