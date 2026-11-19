//! Board variant, selected by exactly one `bitaxe-*` feature: a board number (`bitaxe-401`) or a
//! family name for its latest revision (`bitaxe-supra`). The board feature enables the board's
//! parts (`asic-*`, `vcore-*`, ...), which the code below and the other modules select on.
//!
//! Pins are the same on every board (see `main`).

const BOARD_FEATURES: usize = cfg!(feature = "bitaxe-102") as usize
    + cfg!(feature = "bitaxe-201") as usize
    + cfg!(feature = "bitaxe-202") as usize
    + cfg!(feature = "bitaxe-205") as usize
    + cfg!(feature = "bitaxe-207") as usize
    + cfg!(feature = "bitaxe-400") as usize
    + cfg!(feature = "bitaxe-401") as usize
    + cfg!(feature = "bitaxe-402") as usize
    + cfg!(feature = "bitaxe-403") as usize
    + cfg!(feature = "bitaxe-600") as usize
    + cfg!(feature = "bitaxe-601") as usize
    + cfg!(feature = "bitaxe-602") as usize
    + cfg!(feature = "bitaxe-603") as usize;
const _: () = assert!(
    BOARD_FEATURES == 1,
    "You must activate exactly one board feature: a board number (bitaxe-102, bitaxe-201, \
     bitaxe-202, bitaxe-205, bitaxe-207, bitaxe-400, bitaxe-401, bitaxe-402, bitaxe-403, \
     bitaxe-600, bitaxe-601, bitaxe-602, bitaxe-603) or a family name (bitaxe-max, bitaxe-ultra, \
     bitaxe-supra, bitaxe-gamma)"
);

/// ASIC driver.
#[cfg(feature = "asic-bm1397")]
pub type Asic = bm1397::BM1397;
#[cfg(feature = "asic-bm1366")]
pub type Asic = bm1366::BM1366;
#[cfg(feature = "asic-bm1368")]
pub type Asic = bm1368::BM1368;
#[cfg(feature = "asic-bm1370")]
pub type Asic = bm1370::BM1370;

/// Core voltage (V).
#[cfg(feature = "asic-bm1397")]
pub const VCORE: f32 = 1.4;
#[cfg(feature = "asic-bm1366")]
pub const VCORE: f32 = 1.2;
#[cfg(feature = "asic-bm1368")]
pub const VCORE: f32 = 1.166;
#[cfg(feature = "asic-bm1370")]
pub const VCORE: f32 = 1.15;

/// Hash frequency (MHz).
#[cfg(feature = "asic-bm1397")]
pub const HASH_FREQ_MHZ: u64 = 425;
#[cfg(feature = "asic-bm1366")]
pub const HASH_FREQ_MHZ: u64 = 485;
#[cfg(feature = "asic-bm1368")]
pub const HASH_FREQ_MHZ: u64 = 490;
#[cfg(feature = "asic-bm1370")]
pub const HASH_FREQ_MHZ: u64 = 525;

/// Whether the chip rolls the version itself. Without it (BM1397) the host must send midstates.
#[cfg(feature = "asic-bm1397")]
pub const HW_VERSION_ROLLING: bool = false;
#[cfg(not(feature = "asic-bm1397"))]
pub const HW_VERSION_ROLLING: bool = true;

/// Versions rolled per job: 4 midstates on the BM1397, 16 version bits on the others.
#[cfg(feature = "asic-bm1397")]
pub const ROLLED_VERSIONS: f32 = 4.0;
#[cfg(not(feature = "asic-bm1397"))]
pub const ROLLED_VERSIONS: f32 = 65_536.0;

/// The ASIC temperature comes from the EMC2101 internal sensor plus an offset, or from the chip's
/// diode on the EMC2101 external channel.
pub const ASIC_TEMP_INTERNAL: bool = cfg!(feature = "asic-temp-internal");
#[cfg(feature = "asic-temp-internal")]
pub const ASIC_TEMP_OFFSET_C: f32 = 5.0;
#[cfg(not(feature = "asic-temp-internal"))]
pub const ASIC_TEMP_OFFSET_C: f32 = 0.0;

/// EMC2101 external diode settings for the chip's diode: ideality factor and beta compensation
/// register values. `None` keeps the device defaults.
#[cfg(any(
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603"
))]
pub const EMC2101_DIODE: Option<(u8, u8)> = Some((0x24, 0x00));
#[cfg(not(any(
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603"
)))]
pub const EMC2101_DIODE: Option<(u8, u8)> = None;

/// TPS40305 feedback divider: RA (R14) and RB (R15).
#[cfg(feature = "vcore-dac")]
pub const VCORE_RA_OHM: f32 = 4990.0;
#[cfg(feature = "vcore-dac")]
pub const VCORE_RB_OHM: f32 = 3320.0;
/// DS4432 output 0 full-scale resistor (R16). Output 1 is not populated.
#[cfg(any(feature = "bitaxe-102", feature = "bitaxe-201"))]
pub const DS4432_RFS0_OHM: u32 = 80_000;
#[cfg(any(
    feature = "bitaxe-202",
    feature = "bitaxe-205",
    feature = "bitaxe-400",
    feature = "bitaxe-401"
))]
pub const DS4432_RFS0_OHM: u32 = 80_600;

/// TPS546 settings for the 5 V input, single phase.
#[cfg(feature = "vcore-tps546")]
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
