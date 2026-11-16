//! Board variant, selected by exactly one `bitaxe-*` feature.
//!
//! Pins are the same on both boards (Bitaxe Max v2.2 and Ultra 205, see `main`). Values below come
//! from the boards' schematics and from ESP-Miner's device configuration.

#[cfg(any(
    all(feature = "bitaxe-max", feature = "bitaxe-ultra"),
    not(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))
))]
compile_error!(
    "You must activate exactly one of the following variant features:
    bitaxe-max,
    bitaxe-ultra,
    "
);

/// ASIC driver.
#[cfg(feature = "bitaxe-max")]
pub type Asic = bm1397::BM1397;
#[cfg(feature = "bitaxe-ultra")]
pub type Asic = bm1366::BM1366;

/// Core voltage (V).
#[cfg(feature = "bitaxe-max")]
pub const VCORE: f32 = 1.4;
#[cfg(feature = "bitaxe-ultra")]
pub const VCORE: f32 = 1.2;

/// Hash frequency (MHz).
#[cfg(feature = "bitaxe-max")]
pub const HASH_FREQ_MHZ: u64 = 425;
#[cfg(feature = "bitaxe-ultra")]
pub const HASH_FREQ_MHZ: u64 = 485;

/// Whether the chip rolls the version itself. Without it (BM1397) the host must send midstates.
#[cfg(feature = "bitaxe-max")]
pub const HW_VERSION_ROLLING: bool = false;
#[cfg(feature = "bitaxe-ultra")]
pub const HW_VERSION_ROLLING: bool = true;

/// Versions rolled per job: 4 midstates on the BM1397, 16 version bits on the BM1366.
#[cfg(feature = "bitaxe-max")]
pub const ROLLED_VERSIONS: f32 = 4.0;
#[cfg(feature = "bitaxe-ultra")]
pub const ROLLED_VERSIONS: f32 = 65_536.0;

/// The ASIC temperature comes from the EMC2101 internal sensor plus an offset on the Ultra, from
/// the chip's diode on the EMC2101 external channel on the Max.
#[cfg(feature = "bitaxe-max")]
pub const ASIC_TEMP_INTERNAL: bool = false;
#[cfg(feature = "bitaxe-ultra")]
pub const ASIC_TEMP_INTERNAL: bool = true;
#[cfg(feature = "bitaxe-max")]
pub const ASIC_TEMP_OFFSET_C: f32 = 0.0;
#[cfg(feature = "bitaxe-ultra")]
pub const ASIC_TEMP_OFFSET_C: f32 = 5.0;

/// TPS40305 feedback divider: RA (R14) and RB (R15).
pub const VCORE_RA_OHM: f32 = 4990.0;
pub const VCORE_RB_OHM: f32 = 3320.0;
/// DS4432 output 0 full-scale resistor (R16). Output 1 is not populated.
pub const DS4432_RFS0_OHM: u32 = 80_000;

/// ASIC difficulty (ticket mask): chips only report nonces at least this difficult.
pub const ASIC_DIFFICULTY: u32 = 256;

/// Serial baudrate to the chips once they are initialized.
pub const ASIC_BAUDRATE: u32 = 1_000_000;
