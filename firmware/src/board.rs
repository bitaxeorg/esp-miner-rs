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
    + cfg!(feature = "bitaxe-302") as usize
    + cfg!(feature = "bitaxe-303") as usize
    + cfg!(feature = "bitaxe-400") as usize
    + cfg!(feature = "bitaxe-401") as usize
    + cfg!(feature = "bitaxe-402") as usize
    + cfg!(feature = "bitaxe-403") as usize
    + cfg!(feature = "bitaxe-600") as usize
    + cfg!(feature = "bitaxe-601") as usize
    + cfg!(feature = "bitaxe-602") as usize
    + cfg!(feature = "bitaxe-603") as usize
    + cfg!(feature = "bitaxe-650") as usize
    + cfg!(feature = "bitaxe-701") as usize
    + cfg!(feature = "bitaxe-702") as usize
    + cfg!(feature = "bitaxe-801") as usize
    + cfg!(feature = "bitaxe-1201") as usize
    + cfg!(feature = "bitaxe-1300") as usize;
const _: () = assert!(
    BOARD_FEATURES == 1,
    "You must activate exactly one board feature: a board number (bitaxe-102, bitaxe-201, \
     bitaxe-202, bitaxe-205, bitaxe-207, bitaxe-302, bitaxe-303, bitaxe-400, bitaxe-401, \
     bitaxe-402, bitaxe-403, bitaxe-600, bitaxe-601, bitaxe-602, bitaxe-603, bitaxe-650, \
     bitaxe-701, bitaxe-702, bitaxe-801, bitaxe-1201, bitaxe-1300) or a family name (bitaxe-max, \
     bitaxe-ultra, bitaxe-hex, bitaxe-supra, bitaxe-gamma, bitaxe-gamma-duo, bitaxe-supra-hex, \
     bitaxe-gamma-turbo, bitaxe-naja-duo, bitaxe-gamma-hex)"
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
#[cfg(feature = "asic-bm1372")]
pub type Asic = bm1372::BM1372;

/// Core voltage (V).
#[cfg(feature = "asic-bm1397")]
pub const VCORE: f32 = 1.4;
#[cfg(feature = "asic-bm1366")]
pub const VCORE: f32 = 1.2;
#[cfg(feature = "asic-bm1368")]
pub const VCORE: f32 = 1.166;
#[cfg(all(feature = "asic-bm1370", not(feature = "bitaxe-1300")))]
pub const VCORE: f32 = 1.15;
#[cfg(feature = "bitaxe-1300")]
pub const VCORE: f32 = 1.2;
#[cfg(feature = "asic-bm1372")]
pub const VCORE: f32 = 1.0;

/// Voltage domains in series on the core supply, each holding the same number of chips.
#[cfg(any(feature = "bitaxe-1201", feature = "bitaxe-1300"))]
pub const VOLTAGE_DOMAINS: usize = 2;
#[cfg(any(
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702"
))]
pub const VOLTAGE_DOMAINS: usize = 3;
#[cfg(not(any(
    feature = "bitaxe-1201",
    feature = "bitaxe-1300",
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702"
)))]
pub const VOLTAGE_DOMAINS: usize = 1;

/// Hash frequency (MHz).
#[cfg(feature = "asic-bm1397")]
pub const HASH_FREQ_MHZ: u64 = 425;
#[cfg(feature = "asic-bm1366")]
pub const HASH_FREQ_MHZ: u64 = 485;
#[cfg(feature = "asic-bm1368")]
pub const HASH_FREQ_MHZ: u64 = 490;
#[cfg(all(
    feature = "asic-bm1370",
    not(any(feature = "bitaxe-650", feature = "bitaxe-1300"))
))]
pub const HASH_FREQ_MHZ: u64 = 525;
#[cfg(feature = "bitaxe-650")]
pub const HASH_FREQ_MHZ: u64 = 400;
#[cfg(feature = "bitaxe-1300")]
pub const HASH_FREQ_MHZ: u64 = 690;
#[cfg(feature = "asic-bm1372")]
pub const HASH_FREQ_MHZ: u64 = 327;

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
#[cfg(feature = "fan-emc2101")]
pub const ASIC_TEMP_INTERNAL: bool = cfg!(feature = "asic-temp-internal");
#[cfg(feature = "asic-temp-internal")]
pub const ASIC_TEMP_OFFSET_C: f32 = 5.0;
#[cfg(all(feature = "fan-emc2101", not(feature = "asic-temp-internal")))]
pub const ASIC_TEMP_OFFSET_C: f32 = 0.0;

/// EMC2101 external diode settings for the chip's diode: ideality factor and beta compensation
/// register values. `None` keeps the device defaults.
#[cfg(any(
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603",
    feature = "bitaxe-650"
))]
pub const EMC2101_DIODE: Option<(u8, u8)> = Some((0x24, 0x00));
#[cfg(all(
    feature = "fan-emc2101",
    not(any(
        feature = "bitaxe-600",
        feature = "bitaxe-601",
        feature = "bitaxe-602",
        feature = "bitaxe-603",
        feature = "bitaxe-650"
    ))
))]
pub const EMC2101_DIODE: Option<(u8, u8)> = None;

/// I2C addresses of the EMC2302 and of the two TMP1075 board sensors.
#[cfg(feature = "fan-emc2302")]
pub const EMC2302_ADDRESS: u8 = 0x2F;
#[cfg(feature = "fan-emc2302")]
pub const TMP1075_ADDRESSES: [u8; 2] = [0x4A, 0x4B];
/// The ASIC temperature is the hotter TMP1075 board sensor plus this offset.
#[cfg(feature = "fan-emc2302")]
pub const TMP1075_OFFSET_C: f32 = 10.0;

/// Whether the EMC2103 PWM output is push-pull at 26 kHz, instead of open drain at the default
/// base frequency.
#[cfg(feature = "bitaxe-1300")]
pub const EMC2103_PUSH_PULL_26KHZ: bool = true;
#[cfg(all(feature = "fan-emc2103", not(feature = "bitaxe-1300")))]
pub const EMC2103_PUSH_PULL_26KHZ: bool = false;

/// EMC2103 settings for the chips' diodes, on both external channels: ideality factor register
/// value and beta compensation. `None` keeps the device defaults.
#[cfg(any(feature = "bitaxe-801", feature = "bitaxe-1300"))]
pub const EMC2103_DIODE: Option<(u8, emc2103::BetaCompensation)> = None;
#[cfg(feature = "bitaxe-1201")]
pub const EMC2103_DIODE: Option<(u8, emc2103::BetaCompensation)> =
    Some((0x17, emc2103::BetaCompensation::Auto));

/// Whether the ST7789 is mounted upside down.
#[cfg(feature = "bitaxe-1201")]
pub const ST7789_FLIP: bool = false;
#[cfg(feature = "bitaxe-1300")]
pub const ST7789_FLIP: bool = true;

/// The display, as the settings name it.
#[cfg(feature = "display-st7789")]
pub const DISPLAY: &str = "ST7789 (320x170)";
#[cfg(not(feature = "display-st7789"))]
pub const DISPLAY: &str = "SSD1306 (128x32)";

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

/// TPS546 settings for the 12 V input, single phase, for three voltage domains.
#[cfg(any(
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702"
))]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    phase: 0x00,
    stack: crate::vcore::PhaseCount::Single,
    sync: 0x10,
    compensation: None,
    vin_on: 11.5,
    vin_off: 11.0,
    vin_uv_warn: Some(11.0),
    vin_ov_fault: 14.8,
    vout_scale_loop: 0.125,
    vout_min: 2.5,
    vout_max: 4.5,
    iout_oc_warn: 25.0,
    iout_oc_fault: 30.0,
};
/// TPS546 settings for the 5 V input, single phase.
#[cfg(all(
    feature = "vcore-tps546",
    not(any(
        feature = "bitaxe-302",
        feature = "bitaxe-303",
        feature = "bitaxe-701",
        feature = "bitaxe-702",
        feature = "bitaxe-801",
        feature = "bitaxe-1201",
        feature = "bitaxe-1300"
    ))
))]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    phase: 0x00,
    stack: crate::vcore::PhaseCount::Single,
    sync: 0x10,
    compensation: None,
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
/// TPS546 settings for the 12 V input, two phases with SYNC auto-detected.
#[cfg(feature = "bitaxe-801")]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    phase: 0xFF,
    stack: crate::vcore::PhaseCount::TwoPhase,
    sync: 0xD0,
    compensation: Some([0x12, 0x34, 0x42, 0x21, 0x04]),
    vin_on: 11.0,
    vin_off: 10.5,
    vin_uv_warn: Some(11.0),
    vin_ov_fault: 14.8,
    vout_scale_loop: 0.25,
    vout_min: 1.0,
    vout_max: 3.0,
    iout_oc_warn: 50.0,
    iout_oc_fault: 55.0,
};

/// TPS546 settings for the 12 V input, two phases with SYNC auto-detected, for two voltage domains.
#[cfg(feature = "bitaxe-1201")]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    phase: 0xFF,
    stack: crate::vcore::PhaseCount::TwoPhase,
    sync: 0xD0,
    compensation: Some([0x12, 0x34, 0x42, 0x25, 0x04]),
    vin_on: 11.0,
    vin_off: 10.5,
    vin_uv_warn: Some(11.0),
    vin_ov_fault: 14.0,
    vout_scale_loop: 0.125,
    vout_min: 1.8,
    vout_max: 3.0,
    iout_oc_warn: 61.0,
    iout_oc_fault: 68.0,
};

/// TPS546 settings for the 12 V input, four phases with SYNC auto-detected, for two voltage domains.
#[cfg(feature = "bitaxe-1300")]
pub const TPS546: crate::vcore::Tps546Config = crate::vcore::Tps546Config {
    phase: 0xFF,
    stack: crate::vcore::PhaseCount::FourPhase,
    sync: 0xD0,
    compensation: Some([0x02, 0x64, 0x82, 0x11, 0x06]),
    vin_on: 11.0,
    vin_off: 10.5,
    vin_uv_warn: Some(11.0),
    vin_ov_fault: 14.0,
    vout_scale_loop: 0.125,
    vout_min: 2.0,
    vout_max: 3.0,
    iout_oc_warn: 150.0,
    iout_oc_fault: 160.0,
};

/// Power of the rest of the board, added to the TPS546 output power (W).
#[cfg(any(
    feature = "bitaxe-207",
    feature = "bitaxe-402",
    feature = "bitaxe-403",
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603",
    feature = "bitaxe-650"
))]
pub const POWER_OFFSET_W: f32 = 5.0;
#[cfg(any(feature = "bitaxe-302", feature = "bitaxe-303"))]
pub const POWER_OFFSET_W: f32 = 12.0;
#[cfg(any(
    feature = "bitaxe-701",
    feature = "bitaxe-702",
    feature = "bitaxe-1300"
))]
pub const POWER_OFFSET_W: f32 = 25.0;
#[cfg(feature = "bitaxe-801")]
pub const POWER_OFFSET_W: f32 = 10.0;
#[cfg(feature = "bitaxe-1201")]
pub const POWER_OFFSET_W: f32 = 0.0;

/// ASIC difficulty (ticket mask): chips only report nonces at least this difficult.
pub const ASIC_DIFFICULTY: u32 = 256;

/// Serial baudrate to the chips once they are initialized.
pub const ASIC_BAUDRATE: u32 = 1_000_000;

/// ASIC model name, cores per chip and hash domains per chip.
#[cfg(feature = "asic-bm1397")]
pub const ASIC_MODEL: &str = "BM1397";
#[cfg(feature = "asic-bm1366")]
pub const ASIC_MODEL: &str = "BM1366";
#[cfg(feature = "asic-bm1368")]
pub const ASIC_MODEL: &str = "BM1368";
#[cfg(all(feature = "asic-bm1370", not(feature = "bitaxe-1201")))]
pub const ASIC_MODEL: &str = "BM1370";
#[cfg(feature = "bitaxe-1201")]
pub const ASIC_MODEL: &str = "BM1372/BM1373";

#[cfg(feature = "asic-bm1397")]
pub const SMALL_CORE_COUNT: u16 = 672;
#[cfg(feature = "asic-bm1366")]
pub const SMALL_CORE_COUNT: u16 = 894;
#[cfg(feature = "asic-bm1368")]
pub const SMALL_CORE_COUNT: u16 = 1276;
#[cfg(all(feature = "asic-bm1370", not(feature = "bitaxe-1201")))]
pub const SMALL_CORE_COUNT: u16 = 2040;
#[cfg(feature = "bitaxe-1201")]
pub const SMALL_CORE_COUNT: u16 = 6725;

#[cfg(feature = "asic-bm1397")]
pub const HASH_DOMAINS: u8 = 1;
#[cfg(any(feature = "asic-bm1366", feature = "asic-bm1368"))]
pub const HASH_DOMAINS: u8 = 4;
#[cfg(all(feature = "asic-bm1370", not(feature = "bitaxe-1201")))]
pub const HASH_DOMAINS: u8 = 4;
#[cfg(feature = "bitaxe-1201")]
pub const HASH_DOMAINS: u8 = 8;

/// Frequencies (MHz) and core voltages (mV) offered in the settings.
#[cfg(feature = "asic-bm1397")]
pub const FREQUENCY_OPTIONS: &[u16] = &[400, 425, 450, 475, 485, 500, 525, 550, 575, 600];
#[cfg(feature = "asic-bm1366")]
pub const FREQUENCY_OPTIONS: &[u16] = &[400, 425, 450, 475, 485, 500, 525, 550, 575];
#[cfg(feature = "asic-bm1368")]
pub const FREQUENCY_OPTIONS: &[u16] = &[400, 425, 450, 475, 485, 490, 500, 525, 550, 575];
#[cfg(all(
    feature = "asic-bm1370",
    not(any(feature = "bitaxe-650", feature = "bitaxe-1201"))
))]
pub const FREQUENCY_OPTIONS: &[u16] = &[400, 490, 525, 550, 600, 625, 690];
#[cfg(feature = "bitaxe-650")]
pub const FREQUENCY_OPTIONS: &[u16] = &[350, 375, 380, 400, 410];
#[cfg(feature = "bitaxe-1201")]
pub const FREQUENCY_OPTIONS: &[u16] = &[327, 350, 375, 380, 400, 410];

#[cfg(feature = "asic-bm1397")]
pub const VOLTAGE_OPTIONS: &[u16] = &[1100, 1150, 1200, 1250, 1300, 1350, 1400, 1450, 1500];
#[cfg(feature = "asic-bm1366")]
pub const VOLTAGE_OPTIONS: &[u16] = &[1100, 1150, 1200, 1250, 1300];
#[cfg(feature = "asic-bm1368")]
pub const VOLTAGE_OPTIONS: &[u16] = &[1100, 1150, 1166, 1200, 1250, 1300];
#[cfg(any(feature = "asic-bm1370", feature = "asic-bm1372"))]
pub const VOLTAGE_OPTIONS: &[u16] = &[1000, 1060, 1100, 1150, 1200, 1250];

/// Chips on the board, the most power it draws (W), its nominal input voltage (V), and the
/// colour of its family in the swarm view.
#[cfg(any(
    feature = "bitaxe-102",
    feature = "bitaxe-201",
    feature = "bitaxe-202",
    feature = "bitaxe-205",
    feature = "bitaxe-207",
    feature = "bitaxe-400",
    feature = "bitaxe-401",
    feature = "bitaxe-402",
    feature = "bitaxe-403",
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603"
))]
pub const ASIC_COUNT: u8 = 1;
#[cfg(any(
    feature = "bitaxe-650",
    feature = "bitaxe-801",
    feature = "bitaxe-1201"
))]
pub const ASIC_COUNT: u8 = 2;
#[cfg(any(
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702",
    feature = "bitaxe-1300"
))]
pub const ASIC_COUNT: u8 = 6;

#[cfg(any(
    feature = "bitaxe-102",
    feature = "bitaxe-201",
    feature = "bitaxe-202",
    feature = "bitaxe-205",
    feature = "bitaxe-207"
))]
pub const MAX_POWER_W: u16 = 25;
#[cfg(any(
    feature = "bitaxe-400",
    feature = "bitaxe-401",
    feature = "bitaxe-402",
    feature = "bitaxe-403",
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603",
    feature = "bitaxe-650"
))]
pub const MAX_POWER_W: u16 = 40;
#[cfg(any(feature = "bitaxe-801", feature = "bitaxe-1201"))]
pub const MAX_POWER_W: u16 = 60;
#[cfg(any(feature = "bitaxe-302", feature = "bitaxe-303"))]
pub const MAX_POWER_W: u16 = 90;
#[cfg(any(feature = "bitaxe-701", feature = "bitaxe-702"))]
pub const MAX_POWER_W: u16 = 120;
#[cfg(feature = "bitaxe-1300")]
pub const MAX_POWER_W: u16 = 180;

#[cfg(any(
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702",
    feature = "bitaxe-801",
    feature = "bitaxe-1201",
    feature = "bitaxe-1300"
))]
pub const NOMINAL_VOLTAGE_V: u8 = 12;
#[cfg(not(any(
    feature = "bitaxe-302",
    feature = "bitaxe-303",
    feature = "bitaxe-701",
    feature = "bitaxe-702",
    feature = "bitaxe-801",
    feature = "bitaxe-1201",
    feature = "bitaxe-1300"
)))]
pub const NOMINAL_VOLTAGE_V: u8 = 5;

#[cfg(feature = "bitaxe-102")]
pub const SWARM_COLOR: &str = "red";
#[cfg(any(
    feature = "bitaxe-201",
    feature = "bitaxe-202",
    feature = "bitaxe-205",
    feature = "bitaxe-207"
))]
pub const SWARM_COLOR: &str = "purple";
#[cfg(any(feature = "bitaxe-302", feature = "bitaxe-303"))]
pub const SWARM_COLOR: &str = "orange";
#[cfg(any(
    feature = "bitaxe-400",
    feature = "bitaxe-401",
    feature = "bitaxe-402",
    feature = "bitaxe-403"
))]
pub const SWARM_COLOR: &str = "blue";
#[cfg(any(
    feature = "bitaxe-600",
    feature = "bitaxe-601",
    feature = "bitaxe-602",
    feature = "bitaxe-603",
    feature = "bitaxe-650"
))]
pub const SWARM_COLOR: &str = "green";
#[cfg(any(feature = "bitaxe-701", feature = "bitaxe-702"))]
pub const SWARM_COLOR: &str = "darkblue";
#[cfg(any(feature = "bitaxe-801", feature = "bitaxe-1300"))]
pub const SWARM_COLOR: &str = "cyan";
#[cfg(feature = "bitaxe-1201")]
pub const SWARM_COLOR: &str = "magenta";
