//! Core voltage regulator.
//!
//! - `vcore-dac` (Max, Ultra up to 205, Supra 400/401): a TPS40305 buck whose output a DS4432
//!   current DAC moves through its feedback node, enabled by PWR_EN (active low), with an INA260
//!   on the 5 V input.
//! - `vcore-tps546` (Ultra 207, Supra 402 and later, Gamma, GammaTurbo): a TPS546D24A buck, or a
//!   stack of them, driven over PMBus, which also measures its input, output and temperature.

#[cfg(feature = "vcore-dac")]
pub use dac::Vcore;
#[cfg(feature = "vcore-tps546")]
pub use pmbus::{PhaseCount, Tps546Config, Vcore};

#[cfg(feature = "vcore-dac")]
mod dac {
    use defmt::{error, info, warn};
    use ds4432::{AsyncDS4432, Status};
    use esp_hal::gpio::Output;
    use ina260::AsyncINA260;
    use miner_core::vcore;

    use crate::{board, power::BusDevice};

    pub struct Vcore {
        dac: AsyncDS4432<BusDevice>,
        monitor: AsyncINA260<BusDevice>,
        /// PWR_EN, active low.
        enable: Output<'static>,
    }

    impl Vcore {
        pub fn new(
            dac: AsyncDS4432<BusDevice>,
            monitor: AsyncINA260<BusDevice>,
            enable: Output<'static>,
        ) -> Self {
            Self {
                dac,
                monitor,
                enable,
            }
        }

        /// Sets the DAC for `volts`, then enables the regulator: set before, it starts at the target
        /// voltage instead of its nominal ~1.5 V. False when the core power stays off.
        pub async fn on(&mut self, volts: f32) -> bool {
            let current_ua =
                vcore::feedback_current_ua(volts, board::VCORE_RA_OHM, board::VCORE_RB_OHM);
            let status = if current_ua >= 0.0 {
                Status::SourceMicroAmp(current_ua)
            } else {
                Status::SinkMicroAmp(-current_ua)
            };
            if let Err(e) = self.dac.set_status(ds4432::Output::Zero, status).await {
                error!("setting the core voltage failed: {}", e);
                return false;
            }
            self.enable.set_low();
            info!("core voltage set to {} V", volts);
            true
        }

        pub async fn off(&mut self) {
            self.enable.set_high();
        }

        /// Logs the input power with the ASIC temperature.
        pub async fn log(&mut self, asic_temp: f32) {
            match (
                self.monitor.power().await,
                self.monitor.current().await,
                self.monitor.voltage().await,
            ) {
                (Ok(mw), Ok(ua), Ok(uv)) => info!(
                    "ASIC {} °C, input {} mW ({} mV, {} mA)",
                    asic_temp,
                    mw,
                    uv / 1000,
                    ua / 1000
                ),
                _ => warn!("ASIC {} °C, reading the INA260 failed", asic_temp),
            }
        }
    }
}

#[cfg(feature = "vcore-tps546")]
mod pmbus {
    use defmt::{error, info, warn};
    use embassy_time::{Duration, Timer};
    use esp_hal::gpio::Output;
    use pmbus_adapter::PmbusAdaptor;
    use smbus_adapter::SmbusAdaptor;
    pub use tps546::PhaseCount;
    use tps546::{
        CompensationConfig, OnOffConfig, Operation, PinDetectOverride, StackConfig, SyncConfig,
        Tps546,
    };

    use crate::{board, power::BusDevice};

    /// PMBus address of the TPS546.
    const ADDRESS: u8 = 0x24;
    /// IC_DEVICE_ID of the TPS546D24A and of the TPS546D24S.
    const DEVICE_IDS: [[u8; 6]; 2] = [
        [0x54, 0x49, 0x54, 0x6D, 0x24, 0x41],
        [0x54, 0x49, 0x54, 0x6D, 0x24, 0x62],
    ];
    /// Time for the TPS546 to answer after power up, then between IC_DEVICE_ID attempts.
    const ID_FIRST_DELAY: Duration = Duration::from_millis(15);
    const ID_RETRY_DELAY: Duration = Duration::from_millis(3);
    const ID_ATTEMPTS: usize = 6;
    /// Time for the ASIC power enable to settle before the regulator is used.
    const ENABLE_SETTLE: Duration = Duration::from_millis(20);

    const SWITCH_FREQ_KHZ: f32 = 650.0;
    /// Output limits and margins, relative to VOUT_COMMAND (the TPS546 default format).
    const VOUT_OV_FAULT: f32 = 1.25;
    const VOUT_OV_WARN: f32 = 1.16;
    const VOUT_MARGIN_HIGH: f32 = 1.1;
    const VOUT_MARGIN_LOW: f32 = 0.9;
    const VOUT_UV_WARN: f32 = 0.9;
    const VOUT_UV_FAULT: f32 = 0.75;
    /// Fault responses: input overvoltage, output overcurrent (shut down, no retry), over
    /// temperature (wait for cooling, then retry) and TON_MAX.
    const VIN_OV_FAULT_RESPONSE: u8 = 0xB7;
    const IOUT_OC_FAULT_RESPONSE: u8 = 0xC0;
    const OT_FAULT_RESPONSE: u8 = 0xFF;
    const TON_MAX_FAULT_RESPONSE: u8 = 0x3B;
    const OT_WARN_C: f32 = 105.0;
    const OT_FAULT_C: f32 = 145.0;
    /// Turn-on rise time (ms). The turn-on and turn-off delays, the turn-off fall time and
    /// TON_MAX_FAULT_LIMIT are 0.
    const TON_RISE_MS: f32 = 3.0;

    /// Board-specific TPS546 settings (V, A).
    pub struct Tps546Config {
        /// PHASE: 0x00 for a single TPS546, 0xFF for every phase of a stack.
        pub phase: u8,
        /// Phases of the stack.
        pub stack: PhaseCount,
        /// SYNC_CONFIG raw value.
        pub sync: u8,
        /// COMPENSATION_CONFIG, `None` to keep the pin-detected one.
        pub compensation: Option<[u8; 5]>,
        pub vin_on: f32,
        pub vin_off: f32,
        pub vin_uv_warn: Option<f32>,
        pub vin_ov_fault: f32,
        pub vout_scale_loop: f32,
        pub vout_min: f32,
        pub vout_max: f32,
        pub iout_oc_warn: f32,
        pub iout_oc_fault: f32,
    }

    pub struct Vcore {
        tps: Tps546<BusDevice>,
        /// ASIC power enable, active high, on the boards that have one.
        enable: Option<Output<'static>>,
    }

    impl Vcore {
        pub fn new(bus: BusDevice, enable: Option<Output<'static>>) -> Self {
            Self {
                tps: Tps546::new(PmbusAdaptor::new(SmbusAdaptor::new(bus)), ADDRESS),
                enable,
            }
        }

        /// Enables the ASIC power, configures the regulator with the output off, then sets the
        /// output to `volts` for each of the [`board::VOLTAGE_DOMAINS`] in series and turns it on.
        /// False when the core power stays off.
        pub async fn on(&mut self, volts: f32) -> bool {
            let config = &board::TPS546;
            let vout = volts * board::VOLTAGE_DOMAINS as f32;
            if !(config.vout_min..=config.vout_max).contains(&vout) {
                error!(
                    "regulator output {} V outside of {} V to {} V",
                    vout, config.vout_min, config.vout_max
                );
                return false;
            }
            if let Some(enable) = &mut self.enable {
                enable.set_high();
                Timer::after(ENABLE_SETTLE).await;
            }
            if !self.identify().await {
                error!("no TPS546 found");
                return false;
            }
            if let Err(e) = self.configure(config, vout).await {
                error!("configuring the TPS546 failed: {}", e);
                return false;
            }
            if let Err(e) = self.tps.set_operation(Operation::from_raw(0x80)).await {
                error!("turning the TPS546 output on failed: {}", e);
                return false;
            }
            match self.tps.get_operation().await {
                Ok(op) if op.on => {}
                Ok(op) => warn!("TPS546 OPERATION reads {:x} after turning on", op.to_raw()),
                Err(e) => warn!("reading the TPS546 OPERATION failed: {}", e),
            }
            info!(
                "core voltage set to {} V (regulator output {} V)",
                volts, vout
            );
            true
        }

        pub async fn off(&mut self) {
            if let Some(enable) = &mut self.enable {
                enable.set_low();
            }
            if let Err(e) = self.tps.set_operation(Operation::from_raw(0x00)).await {
                error!("turning the TPS546 output off failed: {}", e);
            }
        }

        /// Logs the regulator readings with the ASIC temperature. The output power leaves out the
        /// rest of the board.
        pub async fn log(&mut self, asic_temp: f32) {
            match (
                self.tps.read_vin().await,
                self.tps.read_vout().await,
                self.tps.read_iout().await,
                self.tps.read_temperature().await,
            ) {
                (Ok(vin), Ok(vout), Ok(iout), Ok(temp)) => info!(
                    "ASIC {} °C, core {} mW ({} mV, {} mA), input {} mV, regulator {} °C",
                    asic_temp,
                    (vout * iout * 1000.0) as u32,
                    (vout * 1000.0) as u32,
                    (iout * 1000.0) as u32,
                    (vin * 1000.0) as u32,
                    temp
                ),
                _ => warn!("ASIC {} °C, reading the TPS546 failed", asic_temp),
            }
        }

        /// Whether IC_DEVICE_ID is a TPS546D24A or TPS546D24S.
        async fn identify(&mut self) -> bool {
            Timer::after(ID_FIRST_DELAY).await;
            for _ in 0..ID_ATTEMPTS {
                if let Ok(id) = self.tps.get_ic_device_id().await {
                    if DEVICE_IDS.iter().any(|known| id.as_slice() == known) {
                        return true;
                    }
                }
                Timer::after(ID_RETRY_DELAY).await;
            }
            false
        }

        /// Writes every setting with the output at `volts`, the output staying off. Nothing is
        /// stored in the NVM.
        async fn configure(
            &mut self,
            config: &Tps546Config,
            volts: f32,
        ) -> Result<(), <BusDevice as embedded_hal_async::i2c::ErrorType>::Error> {
            let tps = &mut self.tps;
            tps.set_operation(Operation::from_raw(0x00)).await?;
            // power conversion follows OPERATION and the CONTROL pin until configured
            tps.set_on_off_config(OnOffConfig::from_raw(0x1F)).await?;
            tps.init().await?;
            // power conversion follows OPERATION only
            tps.set_on_off_config(OnOffConfig::from_raw(0x1B)).await?;
            tps.set_stack_config(StackConfig {
                phase_count: config.stack,
            })
            .await?;
            tps.set_sync_config(SyncConfig::from_raw(config.sync))
                .await?;
            tps.set_phase(config.phase).await?;
            tps.set_frequency_switch(SWITCH_FREQ_KHZ).await?;
            if let Some(compensation) = config.compensation {
                tps.set_compensation_config(CompensationConfig::from_bytes(compensation))
                    .await?;
            }

            if let Some(limit) = config.vin_uv_warn {
                tps.set_vin_uv_warn_limit(limit).await?;
            }
            tps.set_vin_on(config.vin_on).await?;
            tps.set_vin_off(config.vin_off).await?;
            tps.set_vin_ov_fault_limit(config.vin_ov_fault).await?;
            tps.set_vin_ov_fault_response(VIN_OV_FAULT_RESPONSE).await?;

            tps.set_vout_scale_loop(config.vout_scale_loop).await?;
            tps.set_vout_command(volts).await?;
            tps.set_vout_max(config.vout_max).await?;
            tps.set_vout_min(config.vout_min).await?;
            tps.set_vout_ov_fault_limit(VOUT_OV_FAULT).await?;
            tps.set_vout_ov_warn_limit(VOUT_OV_WARN).await?;
            tps.set_vout_margin_high(VOUT_MARGIN_HIGH).await?;
            tps.set_vout_margin_low(VOUT_MARGIN_LOW).await?;
            tps.set_vout_uv_warn_limit(VOUT_UV_WARN).await?;
            tps.set_vout_uv_fault_limit(VOUT_UV_FAULT).await?;

            tps.set_iout_oc_warn_limit(config.iout_oc_warn).await?;
            tps.set_iout_oc_fault_limit(config.iout_oc_fault).await?;
            tps.set_iout_oc_fault_response(IOUT_OC_FAULT_RESPONSE)
                .await?;

            tps.set_ot_warn_limit(OT_WARN_C).await?;
            tps.set_ot_fault_limit(OT_FAULT_C).await?;
            tps.set_ot_fault_response(OT_FAULT_RESPONSE).await?;

            tps.set_ton_delay(0.0).await?;
            tps.set_ton_rise(TON_RISE_MS).await?;
            tps.set_ton_max_fault_limit(0.0).await?;
            tps.set_ton_max_fault_response(TON_MAX_FAULT_RESPONSE)
                .await?;
            tps.set_toff_delay(0.0).await?;
            tps.set_toff_fall(0.0).await?;

            // power up with the pin-detected settings, not the NVM ones
            tps.set_pin_detect_override(PinDetectOverride::from_raw(0xFFFF))
                .await?;
            tps.clear_faults().await
        }
    }
}
