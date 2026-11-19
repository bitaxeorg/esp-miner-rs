//! Board variant, selected by exactly one `bitaxe-*` feature.
//!
//! Pins are the same on every board (see `main`).

const BOARD_FEATURES: usize = cfg!(feature = "bitaxe-max") as usize
    + cfg!(feature = "bitaxe-ultra") as usize
    + cfg!(feature = "bitaxe-supra") as usize
    + cfg!(feature = "bitaxe-gamma") as usize;
const _: () = assert!(
    BOARD_FEATURES == 1,
    "You must activate exactly one of the following variant features: bitaxe-max, bitaxe-ultra, \
     bitaxe-supra, bitaxe-gamma"
);

/// ASIC driver.
#[cfg(feature = "bitaxe-max")]
pub type Asic = bm1397::BM1397;
#[cfg(feature = "bitaxe-ultra")]
pub type Asic = bm1366::BM1366;
#[cfg(feature = "bitaxe-supra")]
pub type Asic = bm1368::BM1368;
#[cfg(feature = "bitaxe-gamma")]
pub type Asic = bm1370::BM1370;

/// Core voltage (V).
#[cfg(feature = "bitaxe-max")]
pub const VCORE: f32 = 1.4;
#[cfg(feature = "bitaxe-ultra")]
pub const VCORE: f32 = 1.2;
#[cfg(feature = "bitaxe-supra")]
pub const VCORE: f32 = 1.166;
#[cfg(feature = "bitaxe-gamma")]
pub const VCORE: f32 = 1.15;

/// Hash frequency (MHz).
#[cfg(feature = "bitaxe-max")]
pub const HASH_FREQ_MHZ: u64 = 425;
#[cfg(feature = "bitaxe-ultra")]
pub const HASH_FREQ_MHZ: u64 = 485;
#[cfg(feature = "bitaxe-supra")]
pub const HASH_FREQ_MHZ: u64 = 490;
#[cfg(feature = "bitaxe-gamma")]
pub const HASH_FREQ_MHZ: u64 = 525;

/// Whether the chip rolls the version itself. Without it (BM1397) the host must send midstates.
#[cfg(feature = "bitaxe-max")]
pub const HW_VERSION_ROLLING: bool = false;
#[cfg(not(feature = "bitaxe-max"))]
pub const HW_VERSION_ROLLING: bool = true;

/// Versions rolled per job: 4 midstates on the BM1397, 16 version bits on the others.
#[cfg(feature = "bitaxe-max")]
pub const ROLLED_VERSIONS: f32 = 4.0;
#[cfg(not(feature = "bitaxe-max"))]
pub const ROLLED_VERSIONS: f32 = 65_536.0;

/// The ASIC temperature comes from the EMC2101 internal sensor plus an offset on the Ultra, from
/// the chip's diode on the EMC2101 external channel on the others.
#[cfg(feature = "bitaxe-ultra")]
pub const ASIC_TEMP_INTERNAL: bool = true;
#[cfg(not(feature = "bitaxe-ultra"))]
pub const ASIC_TEMP_INTERNAL: bool = false;
#[cfg(feature = "bitaxe-ultra")]
pub const ASIC_TEMP_OFFSET_C: f32 = 5.0;
#[cfg(not(feature = "bitaxe-ultra"))]
pub const ASIC_TEMP_OFFSET_C: f32 = 0.0;

/// EMC2101 external diode settings for the chip's diode: ideality factor and beta compensation
/// register values. `None` keeps the device defaults.
#[cfg(not(feature = "bitaxe-gamma"))]
pub const EMC2101_DIODE: Option<(u8, u8)> = None;
#[cfg(feature = "bitaxe-gamma")]
pub const EMC2101_DIODE: Option<(u8, u8)> = Some((0x24, 0x00));

/// TPS40305 feedback divider: RA (R14) and RB (R15).
#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
pub const VCORE_RA_OHM: f32 = 4990.0;
#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
pub const VCORE_RB_OHM: f32 = 3320.0;
/// DS4432 output 0 full-scale resistor (R16). Output 1 is not populated.
#[cfg(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))]
pub const DS4432_RFS0_OHM: u32 = 80_000;

/// TPS546 settings for the 5 V input, single phase.
#[cfg(any(feature = "bitaxe-supra", feature = "bitaxe-gamma"))]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    vin_on: 4.8,
    vin_off: 4.5,
    vin_uv_warn: None,
    vin_ov_fault: 6.5,
    vout_scale_loop: 0.25,
    vout_min: 1.0,
    vout_max: 2.0,
    iout_oc_warn: 25.0,
    iout_oc_fault: 30.0,
};

/// ASIC difficulty (ticket mask): chips only report nonces at least this difficult.
pub const ASIC_DIFFICULTY: u32 = 256;

/// Serial baudrate to the chips once they are initialized.
pub const ASIC_BAUDRATE: u32 = 1_000_000;
