//! Numbers shown to a person.

use core::fmt::Write;

use heapless::String;

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

/// Text of `value` with a suffix: below 1000 as an integer, else divided by the largest power of
/// 1000 it reaches, rounded down to 3 digits, with 2 decimals and `k`, `M`, `G`, `T`, `P` or `E`
/// (`4234567` gives `4.23M`).
pub fn suffix_string(value: u64) -> String<16> {
    const SUFFIXES: [(u64, &str); 6] = [
        (1_000_000_000_000_000_000, "E"),
        (1_000_000_000_000_000, "P"),
        (1_000_000_000_000, "T"),
        (1_000_000_000, "G"),
        (1_000_000, "M"),
        (1_000, "k"),
    ];
    let mut text = String::new();
    match SUFFIXES.iter().find(|(unit, _)| value >= *unit) {
        Some(&(unit, suffix)) => {
            let thousandths = value / (unit / 1000);
            let _ = write!(text, "{:.2}{suffix}", thousandths as f64 / 1000.0);
        }
        None => {
            let _ = write!(text, "{value}");
        }
    }
    text
}

/// Uptime as `1d 2h 3m`, `2h 3m` or `3m 4s`.
pub fn uptime_compact(seconds: u64) -> String<24> {
    let (days, hours, minutes, seconds) = split_uptime(seconds);
    let mut text = String::new();
    let _ = if days > 0 {
        write!(text, "{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        write!(text, "{hours}h {minutes}m")
    } else {
        write!(text, "{minutes}m {seconds}s")
    };
    text
}

/// Uptime as `1d 2h 3m 4s`, `2h 3m 4s`, `3m 4s` or `4s`.
pub fn uptime_full(seconds: u64) -> String<24> {
    let (days, hours, minutes, seconds) = split_uptime(seconds);
    let mut text = String::new();
    let _ = if days > 0 {
        write!(text, "{days}d {hours}h {minutes}m {seconds}s")
    } else if hours > 0 {
        write!(text, "{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        write!(text, "{minutes}m {seconds}s")
    } else {
        write!(text, "{seconds}s")
    };
    text
}

fn split_uptime(seconds: u64) -> (u64, u64, u64, u64) {
    (
        seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    )
}

/// Network difficulty of the target encoded by `nbits` (difficulty 1: `0x1d00ffff`).
pub fn network_difficulty(nbits: u32) -> f64 {
    let exponent = (nbits >> 24) as i32;
    let mantissa = f64::from(nbits & 0x00ff_ffff);
    if mantissa == 0.0 {
        return 0.0;
    }
    let mut difficulty = f64::from(0xffff_u32) / mantissa;
    let mut shift = 0x1d - exponent;
    while shift > 0 {
        difficulty *= 256.0;
        shift -= 1;
    }
    while shift < 0 {
        difficulty /= 256.0;
        shift += 1;
    }
    difficulty
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

    #[test]
    fn suffix_strings() {
        assert_eq!(suffix_string(0), "0");
        assert_eq!(suffix_string(999), "999");
        assert_eq!(suffix_string(1_000), "1.00k");
        assert_eq!(suffix_string(4_234_567), "4.23M");
        assert_eq!(suffix_string(127_005_820_000_000), "127.00T");
    }

    #[test]
    fn uptimes() {
        assert_eq!(uptime_compact(64), "1m 4s");
        assert_eq!(uptime_compact(2 * 3_600 + 180), "2h 3m");
        assert_eq!(uptime_compact(86_400 + 7_384), "1d 2h 3m");
        assert_eq!(uptime_full(4), "4s");
        assert_eq!(uptime_full(64), "1m 4s");
        assert_eq!(uptime_full(86_400 + 7_384), "1d 2h 3m 4s");
    }

    #[test]
    fn network_difficulties() {
        assert_eq!(network_difficulty(0x1d00_ffff), 1.0);
        // 0xffff / 0x02c4e4 × 256^6
        let d = network_difficulty(0x1702_c4e4);
        assert!((d - 101.65e12).abs() < 0.01e12, "{d}");
        assert_eq!(network_difficulty(0), 0.0);
    }
}
