//! Core voltage of the Bitaxe Max/Ultra.
//!
//! The TPS40305 buck regulator holds its feedback node at [`TPS40305_VFB`], fed by a divider
//! between the output (RA, R14) and ground (RB, R15). A DS4432 current DAC sources current into
//! (or sinks it from) that node to move the output voltage.

/// Feedback voltage of the TPS40305.
pub const TPS40305_VFB: f32 = 0.6;

/// Full-scale reference voltage of the DS4432.
const DS4432_VRFS: f32 = 0.997;

/// Current (µA) the DAC must source into the feedback node for `vout`, negative when it must sink.
///
/// Kirchhoff at the feedback node: `(vout - VFB) / RA + I = VFB / RB`.
pub fn feedback_current_ua(vout: f32, ra_ohm: f32, rb_ohm: f32) -> f32 {
    (TPS40305_VFB / rb_ohm - (vout - TPS40305_VFB) / ra_ohm) * 1_000_000.0
}

/// Output voltage with no DAC current: `VFB * (1 + RA / RB)`.
pub fn nominal_vout(ra_ohm: f32, rb_ohm: f32) -> f32 {
    TPS40305_VFB * (1.0 + ra_ohm / rb_ohm)
}

/// Full-scale current (µA) of a DS4432 output with its `rfs` resistor: `VRFS / RFS * 127 / 16`.
pub fn ds4432_full_scale_ua(rfs_ohm: f32) -> f32 {
    DS4432_VRFS / rfs_ohm * 127.0 / 16.0 * 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bitaxe Max v2.2 / Ultra 205: R14 = 4.99k, R15 = 3.32k, R16 = 80k
    const RA: f32 = 4990.0;
    const RB: f32 = 3320.0;
    const RFS: f32 = 80_000.0;

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn nominal() {
        assert_close(nominal_vout(RA, RB), 1.5018);
        assert_close(feedback_current_ua(nominal_vout(RA, RB), RA, RB), 0.0);
    }

    #[test]
    fn ultra_and_max_targets_need_sourcing() {
        assert_close(feedback_current_ua(1.2, RA, RB), 60.48);
        assert_close(feedback_current_ua(1.4, RA, RB), 20.40);
    }

    #[test]
    fn above_nominal_needs_sinking() {
        assert!(feedback_current_ua(1.6, RA, RB) < 0.0);
    }

    #[test]
    fn targets_fit_the_dac_range() {
        let full_scale = ds4432_full_scale_ua(RFS);
        assert_close(full_scale, 98.92);
        for vout in [1.1, 1.2, 1.4] {
            assert!(feedback_current_ua(vout, RA, RB).abs() <= full_scale);
        }
    }
}
