//! Numbers shown to a person.

/// `value` scaled down by powers of 1000 below 1000, with its SI prefix (`""`, `"k"`, `"M"`,
/// `"G"`, `"T"`, `"P"`, `"E"`).
pub fn si_prefix(value: f64) -> (f64, &'static str) {
    const PREFIXES: [&str; 7] = ["", "k", "M", "G", "T", "P", "E"];
    let mut scaled = value;
    let mut prefix = 0;
    while scaled.abs() >= 1000.0 && prefix < PREFIXES.len() - 1 {
        scaled /= 1000.0;
        prefix += 1;
    }
    (scaled, PREFIXES[prefix])
}

/// Hashrate in GH/s as a value and an SI-prefixed unit (`"GH/s"`, `"TH/s"`, ...).
pub fn hashrate(ghs: f64) -> (f64, &'static str) {
    const UNITS: [&str; 7] = ["GH/s", "TH/s", "PH/s", "EH/s", "ZH/s", "YH/s", "RH/s"];
    let mut scaled = ghs;
    let mut unit = 0;
    while scaled.abs() >= 1000.0 && unit < UNITS.len() - 1 {
        scaled /= 1000.0;
        unit += 1;
    }
    (scaled, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_values_keep_no_prefix() {
        assert_eq!(si_prefix(0.0), (0.0, ""));
        assert_eq!(si_prefix(999.0), (999.0, ""));
    }

    #[test]
    fn large_values_get_a_prefix() {
        assert_eq!(si_prefix(1000.0), (1.0, "k"));
        assert_eq!(si_prefix(4_200_000.0), (4.2, "M"));
        assert_eq!(si_prefix(1.5e12), (1.5, "T"));
        assert_eq!(si_prefix(2e21), (2000.0, "E"));
    }

    #[test]
    fn hashrate_units() {
        assert_eq!(hashrate(485.0), (485.0, "GH/s"));
        assert_eq!(hashrate(1_210.0), (1.21, "TH/s"));
    }
}
