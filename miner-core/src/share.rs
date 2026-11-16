//! Checking the nonces returned by the chips.

use crate::{header::difficulty, job::MiningJob};

/// The version a chip hashed: the job version with the rolled bits in `mask` (BIP 310).
pub fn rolled_version(job_version: u32, version_bits: u32, mask: u32) -> u32 {
    (job_version & !mask) | (version_bits & mask)
}

/// A nonce returned by a chip, checked against its job.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CheckedNonce {
    /// Version of the hashed header.
    pub version: u32,
    /// `version_bits` of `mining.submit` (BIP 310), `None` without version rolling.
    pub version_bits: Option<u32>,
    /// Header hash, in internal byte order.
    pub hash: [u8; 32],
    pub difficulty: f64,
}

/// Rebuilds the header hashed by the chip that returned `nonce` for `job`, and measures it.
///
/// `version_bits` are the version bits the chip rolled, `None` without hardware version rolling.
pub fn check_nonce(
    job: &MiningJob,
    nonce: u32,
    version_bits: Option<u32>,
    version_mask: u32,
) -> CheckedNonce {
    let version = match version_bits {
        Some(bits) => rolled_version(job.version, bits, version_mask),
        None => job.version,
    };
    let hash = job.header(version, nonce).hash();
    CheckedNonce {
        version,
        version_bits: version_bits.map(|_| version & version_mask),
        hash,
        difficulty: difficulty(&hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::tests::genesis_job;

    #[test]
    fn rolled_version_keeps_bits_outside_mask() {
        assert_eq!(
            rolled_version(0x2000_0000, 0x0001_e000, 0x1fff_e000),
            0x2001_e000
        );
        // bits the chip set outside the mask are ignored
        assert_eq!(
            rolled_version(0x2000_0000, 0xe000_1fff, 0x1fff_e000),
            0x2000_0000
        );
        // bits of the job version inside the mask are replaced
        assert_eq!(
            rolled_version(0x2000_e000, 0x0000_2000, 0x1fff_e000),
            0x2000_2000
        );
    }

    #[test]
    fn genesis_nonce_without_version_rolling() {
        let checked = check_nonce(&genesis_job(), 2_083_236_893, None, 0x1fff_e000);
        assert_eq!(checked.version, 1);
        assert_eq!(checked.version_bits, None);
        assert!(checked.difficulty > 2536.0);
    }

    #[test]
    fn wrong_nonce_is_low_difficulty() {
        let checked = check_nonce(&genesis_job(), 2_083_236_894, None, 0x1fff_e000);
        assert!(checked.difficulty < 1.0);
    }

    #[test]
    fn version_rolling_changes_the_hash() {
        let job = genesis_job();
        let checked = check_nonce(&job, 2_083_236_893, Some(0x0000_2000), 0x1fff_e000);
        assert_eq!(checked.version, 0x0000_2001);
        assert_eq!(checked.version_bits, Some(0x0000_2000));
        assert_eq!(checked.hash, job.header(0x0000_2001, 2_083_236_893).hash());
        assert!(checked.difficulty < 1.0);
    }
}
