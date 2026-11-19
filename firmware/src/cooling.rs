//! ASIC temperature and fan.
//!
//! - `fan-emc2101`: one ASIC, whose temperature is the EMC2101 internal sensor plus an offset
//!   (`asic-temp-internal`) or the chip's diode on the external channel.
//! - `fan-emc2103`: a chip diode on each external channel, with the ideality and beta of
//!   `board::EMC2103_DIODE` when set, the fan driven by direct setting.
//! - `fan-emc2302`: two fans on an EMC2302 driven together, the ASIC temperature from two TMP1075
//!   board sensors plus an offset.

#[cfg(feature = "fan-emc2101")]
pub use emc2101_fan::Cooling;
#[cfg(feature = "fan-emc2103")]
pub use emc2103_fan::Cooling;
#[cfg(feature = "fan-emc2302")]
pub use emc2302_fan::Cooling;

/// ASIC temperatures (°C).
#[derive(Clone, Copy)]
pub struct Temps {
    /// What the board measures, NaN when absent or unread.
    pub chips: [f32; 2],
    /// The hottest, NaN when a sensor can't be read.
    pub hottest: f32,
}

#[cfg(feature = "fan-emc2302")]
mod emc2302_fan {
    use defmt::{debug, error, Debug2Format};
    use emc230x::{Emc230x, FanSelect};
    use tmp1075::Tmp1075;

    use super::{hottest, Temps};
    use crate::{board, power::BusDevice};

    /// The fans of the EMC2302.
    const FANS: [u16; 2] = [1, 2];

    pub struct Cooling {
        fans: Emc230x<BusDevice>,
        sensors: [Tmp1075<BusDevice>; 2],
    }

    impl Cooling {
        pub fn new(fans: Emc230x<BusDevice>, sensors: [Tmp1075<BusDevice>; 2]) -> Self {
            Self { fans, sensors }
        }

        /// Sets the fan outputs to open drain (the driver sets them push-pull) and the fans to
        /// full speed.
        pub async fn init(&mut self) {
            match self.fans.pwm_output_config().await {
                Ok(mut config) => {
                    for fan in FANS {
                        config.open_drain(fan);
                    }
                    if let Err(e) = self.fans.set_pwm_output_config(config).await {
                        error!("fan PWM setup failed: {}", Debug2Format(&e));
                    }
                }
                Err(e) => error!("reading the fan PWM setup failed: {}", Debug2Format(&e)),
            }
            self.set_fan(100).await;
        }

        /// The board sensors plus [`board::TMP1075_OFFSET_C`].
        pub async fn asic_temp(&mut self) -> Temps {
            let mut chips = [f32::NAN; 2];
            for ((sensor, address), chip) in self
                .sensors
                .iter_mut()
                .zip(board::TMP1075_ADDRESSES)
                .zip(chips.iter_mut())
            {
                match sensor.get_temperature_float().await {
                    Ok(t) => {
                        debug!("TMP1075 {:x}: {} °C", address, t);
                        *chip = t + board::TMP1075_OFFSET_C;
                    }
                    Err(e) => error!("reading the TMP1075 at {:x} failed: {}", address, e),
                }
            }
            Temps {
                chips,
                hottest: hottest(chips),
            }
        }

        /// Speed of the first fan, `None` when it can't be read.
        pub async fn fan_rpm(&mut self) -> Option<u16> {
            self.fans
                .rpm(FanSelect(FANS[0]))
                .await
                .ok()
                .filter(|&rpm| rpm != u16::MAX)
        }

        pub async fn set_fan(&mut self, percent: u8) {
            let percent = percent.min(100);
            for fan in FANS {
                if let Err(e) = self.fans.set_duty_cycle(FanSelect(fan), percent).await {
                    error!(
                        "setting fan {} to {}% failed: {}",
                        fan,
                        percent,
                        Debug2Format(&e)
                    );
                }
            }
        }
    }
}

#[cfg(feature = "fan-emc2101")]
mod emc2101_fan {
    use bounded_integer::BoundedU8;
    use defmt::{error, info};
    use emc2101::{AsyncEMC2101, BetaCompensation};
    use fugit::HertzU32;
    use miner_core::thermal;

    use super::Temps;
    use crate::{board, power::BusDevice};

    const FAN_PWM_FREQ: HertzU32 = HertzU32::Hz(5806);

    pub struct Cooling {
        sensor: AsyncEMC2101<BusDevice>,
    }

    impl Cooling {
        pub fn new(sensor: AsyncEMC2101<BusDevice>) -> Self {
            Self { sensor }
        }

        /// Sets up the fan output at full speed, the TACH input and the temperature channel.
        pub async fn init(&mut self) {
            if let Err(e) = self.sensor.set_fan_pwm(FAN_PWM_FREQ, false).await {
                error!("fan PWM setup failed: {}", e);
            }
            if let Err(e) = self.sensor.enable_tach_input().await {
                error!("enabling the fan TACH input failed: {}", e);
            }
            self.set_fan(100).await;
            if let Some((ideality, beta)) = board::EMC2101_DIODE {
                self.configure_diode(ideality, beta).await;
            }
        }

        /// The ASIC temperature.
        pub async fn asic_temp(&mut self) -> Temps {
            let temp = if board::ASIC_TEMP_INTERNAL {
                self.sensor.temp_internal().await.map(f32::from)
            } else {
                self.sensor.temp_external_precise().await
            };
            let temp = match temp {
                Ok(t) => t + board::ASIC_TEMP_OFFSET_C,
                Err(e) => {
                    error!("reading the ASIC temperature failed: {}", e);
                    f32::NAN
                }
            };
            Temps {
                chips: [temp, f32::NAN],
                hottest: temp,
            }
        }

        /// Fan speed, `None` when it can't be read.
        pub async fn fan_rpm(&mut self) -> Option<u16> {
            self.sensor.fan_rpm().await.ok()
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
    use emc2103::{AsyncEMC2103, BetaCompensation, Channel, ExternalDiode, PwmBase};

    use super::{hottest, Temps};
    use crate::{board, power::BusDevice};

    pub struct Cooling {
        sensor: AsyncEMC2103<BusDevice>,
    }

    impl Cooling {
        pub fn new(sensor: AsyncEMC2103<BusDevice>) -> Self {
            Self { sensor }
        }

        /// Sets up the fan output (normal polarity, direct setting; open drain at the default base
        /// frequency, or push-pull at 26 kHz) at full speed and the temperature channels.
        pub async fn init(&mut self) {
            let push_pull = board::EMC2103_PUSH_PULL_26KHZ;
            if let Err(e) = self.sensor.set_pwm_output(push_pull, false).await {
                error!("fan PWM setup failed: {}", e);
            }
            if push_pull {
                if let Err(e) = self
                    .sensor
                    .set_pwm_frequency(PwmBase::Base26kHz, BoundedU8::MIN)
                    .await
                {
                    error!("fan PWM frequency setup failed: {}", e);
                }
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

        /// The two chips with a diode.
        pub async fn asic_temp(&mut self) -> Temps {
            let first = self.chip_temp(Channel::External1).await;
            let second = self.chip_temp(Channel::External2).await;
            debug!(
                "ASIC diodes: External1 {} °C, External2 {} °C",
                first, second
            );
            let chips = [first, second];
            Temps {
                chips,
                hottest: hottest(chips),
            }
        }

        /// Fan speed, `None` when it can't be read.
        pub async fn fan_rpm(&mut self) -> Option<u16> {
            self.sensor.fan_rpm().await.ok()
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

/// The hotter of two temperatures, NaN when one is NaN.
#[cfg(any(feature = "fan-emc2103", feature = "fan-emc2302"))]
fn hottest(chips: [f32; 2]) -> f32 {
    if chips.iter().any(|t| t.is_nan()) {
        f32::NAN
    } else {
        chips[0].max(chips[1])
    }
}
