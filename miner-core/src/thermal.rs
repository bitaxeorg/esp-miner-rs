//! Fan curve and thermal protection.

/// ASIC temperature above which the core power is cut (ESP-Miner's `THROTTLE_TEMP`).
pub const OVERHEAT_C: f32 = 75.0;

/// Below this temperature the fan runs at [`FAN_MIN_PERCENT`].
const FAN_LOW_C: f32 = 40.0;
/// From this temperature the fan runs at 100 %.
const FAN_HIGH_C: f32 = 70.0;
const FAN_MIN_PERCENT: f32 = 30.0;

/// Fan duty for an ASIC temperature: [`FAN_MIN_PERCENT`] up to 40 °C, 100 % from 70 °C, linear in
/// between. An invalid (NaN) temperature gives 100 %.
pub fn fan_percent(temp_c: f32) -> u8 {
    let percent = if temp_c.is_nan() || temp_c >= FAN_HIGH_C {
        100.0
    } else if temp_c <= FAN_LOW_C {
        FAN_MIN_PERCENT
    } else {
        FAN_MIN_PERCENT
            + (100.0 - FAN_MIN_PERCENT) * (temp_c - FAN_LOW_C) / (FAN_HIGH_C - FAN_LOW_C)
    };
    percent as u8
}

/// EMC2101 fan setting step (0 to 63, 63 being 100 % in PWM mode) for a duty in percent.
pub fn emc2101_fan_step(percent: u8) -> u8 {
    (percent.min(100) as u16 * 63 / 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_curve() {
        assert_eq!(fan_percent(-10.0), 30);
        assert_eq!(fan_percent(40.0), 30);
        assert_eq!(fan_percent(55.0), 65);
        assert_eq!(fan_percent(70.0), 100);
        assert_eq!(fan_percent(90.0), 100);
        assert_eq!(fan_percent(f32::NAN), 100);
    }

    #[test]
    fn fan_steps() {
        assert_eq!(emc2101_fan_step(0), 0);
        assert_eq!(emc2101_fan_step(50), 31);
        assert_eq!(emc2101_fan_step(100), 63);
        assert_eq!(emc2101_fan_step(200), 63);
    }
}
