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

/// Fan duty before the first valid temperature.
pub const FAN_STARTUP_PERCENT: f32 = 70.0;
const PID_KP: f32 = 5.0;
const PID_KI: f32 = 0.1;
const PID_KD: f32 = 2.0;
/// Weight of a new temperature in the smoothed input.
const PID_SMOOTHING: f32 = 0.2;

/// Automatic fan control: a PID holding the hottest chip temperature at a target, its output a
/// fan duty between a minimum and 100 %. Derivative on the measurement; the integral is clamped
/// to the output range.
pub struct FanPid {
    integral: f32,
    smoothed: Option<f32>,
    last_input: Option<f32>,
}

impl Default for FanPid {
    fn default() -> Self {
        Self::new()
    }
}

impl FanPid {
    pub const fn new() -> Self {
        Self {
            integral: FAN_STARTUP_PERCENT,
            smoothed: None,
            last_input: None,
        }
    }

    /// Fan duty (%) for the hottest chip temperature `temp_c`, `dt_s` seconds after the previous
    /// call. Before the first valid temperature, [`FAN_STARTUP_PERCENT`]; for an invalid one
    /// after that, 100 %.
    pub fn update(&mut self, temp_c: f32, target_c: f32, min_percent: f32, dt_s: f32) -> f32 {
        let min = min_percent.clamp(0.0, 100.0);
        if temp_c.is_nan() {
            return if self.smoothed.is_none() {
                FAN_STARTUP_PERCENT
            } else {
                100.0
            };
        }
        let input = match self.smoothed {
            Some(s) => s + PID_SMOOTHING * (temp_c - s),
            None => temp_c,
        };
        self.smoothed = Some(input);
        let dt = dt_s.max(0.001);
        let error = input - target_c;
        self.integral = (self.integral + PID_KI * dt * error).clamp(min, 100.0);
        let derivative = self.last_input.map_or(0.0, |last| (input - last) / dt);
        self.last_input = Some(input);
        (PID_KP * error + self.integral + PID_KD * derivative).clamp(min, 100.0)
    }
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
    fn pid_starts_at_the_startup_duty_and_stays_in_range() {
        let mut pid = FanPid::new();
        assert_eq!(pid.update(f32::NAN, 60.0, 25.0, 1.0), FAN_STARTUP_PERCENT);
        // at the target, no error and no derivative: the integral, still at the startup duty
        assert_eq!(pid.update(60.0, 60.0, 25.0, 1.0), FAN_STARTUP_PERCENT);
        // far below the target, down to the minimum
        let mut out = 0.0;
        for _ in 0..200 {
            out = pid.update(40.0, 60.0, 25.0, 1.0);
        }
        assert_eq!(out, 25.0);
        // far above, up to 100 %
        for _ in 0..200 {
            out = pid.update(80.0, 60.0, 25.0, 1.0);
        }
        assert_eq!(out, 100.0);
        assert_eq!(pid.update(f32::NAN, 60.0, 25.0, 1.0), 100.0);
    }

    #[test]
    fn pid_settles_to_hold_the_target() {
        // a plant whose temperature falls with the fan duty
        let mut pid = FanPid::new();
        let mut temp = 70.0;
        let mut duty = FAN_STARTUP_PERCENT;
        for _ in 0..2000 {
            duty = pid.update(temp, 60.0, 20.0, 1.0);
            let steady = 90.0 - 0.5 * duty;
            temp += 0.1 * (steady - temp);
        }
        assert!((temp - 60.0).abs() < 0.5, "{temp}");
        assert!((duty - 60.0).abs() < 1.0, "{duty}");
    }

    #[test]
    fn fan_steps() {
        assert_eq!(emc2101_fan_step(0), 0);
        assert_eq!(emc2101_fan_step(50), 31);
        assert_eq!(emc2101_fan_step(100), 63);
        assert_eq!(emc2101_fan_step(200), 63);
    }
}
