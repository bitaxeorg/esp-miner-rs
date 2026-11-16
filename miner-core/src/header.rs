//! Block header serialization, hashing and difficulty.

use bitcoin_hashes::{sha256d, Hash};

/// 2^208, built from its bits to stay exact.
const TWO_POW_208: f64 = f64::from_bits((1023u64 + 208) << 52);

/// The difficulty 1 target, `0x00000000ffff0000…0000`.
const DIFF1_TARGET: f64 = 65_535.0 * TWO_POW_208;

/// A block header, with hashes in internal byte order (as serialized in a block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BlockHeader {
    pub version: u32,
    pub prev_blockhash: [u8; 32],
    pub merkle_root: [u8; 32],
    pub ntime: u32,
    pub nbits: u32,
    pub nonce: u32,
}

impl BlockHeader {
    /// The 80 bytes of the serialized header.
    pub fn to_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        bytes[0..4].copy_from_slice(&self.version.to_le_bytes());
        bytes[4..36].copy_from_slice(&self.prev_blockhash);
        bytes[36..68].copy_from_slice(&self.merkle_root);
        bytes[68..72].copy_from_slice(&self.ntime.to_le_bytes());
        bytes[72..76].copy_from_slice(&self.nbits.to_le_bytes());
        bytes[76..80].copy_from_slice(&self.nonce.to_le_bytes());
        bytes
    }

    /// Double SHA-256 of the serialized header, in internal byte order.
    pub fn hash(&self) -> [u8; 32] {
        sha256d::Hash::hash(&self.to_bytes()).to_byte_array()
    }
}

/// Difficulty of a hash given in internal byte order: the difficulty 1 target divided by the hash.
pub fn difficulty(hash: &[u8; 32]) -> f64 {
    // internal byte order is little endian
    let value = hash
        .iter()
        .rev()
        .fold(0.0, |acc, &byte| acc * 256.0 + byte as f64);
    if value == 0.0 {
        f64::INFINITY
    } else {
        DIFF1_TARGET / value
    }
}

/// Switches a hash between internal byte order and display order.
pub fn reversed(mut hash: [u8; 32]) -> [u8; 32] {
    hash.reverse();
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    /// The genesis block header.
    pub(crate) const GENESIS: BlockHeader = BlockHeader {
        version: 1,
        prev_blockhash: [0; 32],
        merkle_root: reversed_const(hex!(
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
        )),
        ntime: 1_231_006_505,
        nbits: 0x1d00_ffff,
        nonce: 2_083_236_893,
    };

    const fn reversed_const(hash: [u8; 32]) -> [u8; 32] {
        let mut out = [0; 32];
        let mut i = 0;
        while i < 32 {
            out[i] = hash[31 - i];
            i += 1;
        }
        out
    }

    #[test]
    fn genesis_hash() {
        assert_eq!(
            reversed(GENESIS.hash()),
            hex!("000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f")
        );
    }

    #[test]
    fn genesis_serialization() {
        let bytes = GENESIS.to_bytes();
        assert_eq!(bytes[0..4], [1, 0, 0, 0]);
        assert_eq!(bytes[36..40], hex!("3ba3edfd"));
        assert_eq!(bytes[68..80], hex!("29ab5f49 ffff001d 1dac2b7c"));
    }

    #[test]
    fn difficulty_of_diff1_target_is_one() {
        let mut target = [0u8; 32];
        // 0x00000000ffff0000…, internal byte order
        target[26] = 0xff;
        target[27] = 0xff;
        assert_eq!(difficulty(&target), 1.0);
    }

    #[test]
    fn genesis_difficulty() {
        let diff = difficulty(&GENESIS.hash());
        assert!((2536.0..2537.0).contains(&diff), "{diff}");
    }

    #[test]
    fn zero_hash_is_infinitely_difficult() {
        assert_eq!(difficulty(&[0; 32]), f64::INFINITY);
    }
}
