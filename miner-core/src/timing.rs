//! Job scheduling.

/// Shortest interval between two jobs, so a fast chain doesn't flood the serial link.
pub const MIN_JOB_INTERVAL_MS: u32 = 10;
/// Longest interval between two jobs, so pool work changes reach the chips quickly.
pub const MAX_JOB_INTERVAL_MS: u32 = 2_000;

/// How often to send a new job so the chips never run out of work.
///
/// A job lasts as long as the chain takes to roll the full 32-bit nonce space times the number of
/// versions it rolls (hardware version rolling or midstates). The result is clamped to
/// [`MIN_JOB_INTERVAL_MS`]..=[`MAX_JOB_INTERVAL_MS`].
pub fn job_interval_ms(hashrate_ghs: f32, rolled_versions: f32) -> u32 {
    let hashes = 4_294_967_296.0 * rolled_versions;
    let ms = hashes / (hashrate_ghs * 1_000_000.0);
    if ms.is_nan() || ms >= MAX_JOB_INTERVAL_MS as f32 {
        MAX_JOB_INTERVAL_MS
    } else {
        (ms as u32).max(MIN_JOB_INTERVAL_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm1397_with_4_midstates() {
        // 425 MHz * 672 small cores
        assert_eq!(job_interval_ms(285.6, 4.0), 60);
    }

    #[test]
    fn bm1366_with_hardware_version_rolling() {
        // 485 MHz * 894 small cores, 16 rolled version bits
        assert_eq!(job_interval_ms(433.6, 65_536.0), MAX_JOB_INTERVAL_MS);
    }

    #[test]
    fn clamped() {
        assert_eq!(job_interval_ms(1_000_000.0, 1.0), MIN_JOB_INTERVAL_MS);
        assert_eq!(job_interval_ms(0.0, 1.0), MAX_JOB_INTERVAL_MS);
        assert_eq!(job_interval_ms(0.0, 0.0), MAX_JOB_INTERVAL_MS);
    }
}
