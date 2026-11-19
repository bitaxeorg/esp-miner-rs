//! What the coinbase of a job tells: block height and miner tag.

use heapless::String;

/// Longest scriptsig text kept.
pub const SCRIPTSIG_LEN: usize = 99;

/// Offset of the scriptsig length in the coinbase: after the version (4), the input count (1),
/// the previous output hash (32) and index (4).
const SCRIPTSIG_LEN_OFFSET: usize = 41;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoinbaseInfo {
    /// Block height (BIP 34), at the start of the scriptsig.
    pub block_height: u32,
    /// The rest of the scriptsig as text, non-printable bytes shown as `.`, cut at
    /// [`SCRIPTSIG_LEN`]. Empty when the scriptsig runs past the extranonces in `coinb2` further
    /// than `coinb2` goes.
    pub scriptsig: String<SCRIPTSIG_LEN>,
}

/// Block height and scriptsig of the coinbase made of `coinb1`, the extranonces (of
/// `extranonce1_len` and `extranonce2_len` bytes) and `coinb2`. `None` when `coinb1` is too short
/// or the height doesn't take 1 to 4 bytes.
pub fn coinbase_info(
    coinb1: &[u8],
    coinb2: &[u8],
    extranonce1_len: usize,
    extranonce2_len: usize,
) -> Option<CoinbaseInfo> {
    let scriptsig_len = usize::from(*coinb1.get(SCRIPTSIG_LEN_OFFSET)?);
    let height_len = usize::from(*coinb1.get(SCRIPTSIG_LEN_OFFSET + 1)?);
    if !(1..=4).contains(&height_len) {
        return None;
    }
    let height_start = SCRIPTSIG_LEN_OFFSET + 2;
    let height_bytes = coinb1.get(height_start..height_start + height_len)?;
    let block_height = height_bytes
        .iter()
        .rev()
        .fold(0u32, |height, &b| height << 8 | u32::from(b));

    let tag_start = height_start + height_len;
    let mut tag_len = scriptsig_len as isize - 1 - height_len as isize;
    let in_coinb1 = coinb1.len() - tag_start;
    if (in_coinb1 as isize) < tag_len {
        // the scriptsig covers the extranonces, which carry no text
        tag_len -= (extranonce1_len + extranonce2_len) as isize;
    }
    let mut scriptsig = String::new();
    if tag_len > 0 {
        let tag_len = (tag_len as usize).min(SCRIPTSIG_LEN);
        let from_coinb1 = in_coinb1.min(tag_len);
        let from_coinb2 = tag_len - from_coinb1;
        if coinb2.len() >= from_coinb2 {
            let bytes = coinb1[tag_start..tag_start + from_coinb1]
                .iter()
                .chain(&coinb2[..from_coinb2]);
            for &b in bytes {
                let c = if (0x20..0x7f).contains(&b) {
                    char::from(b)
                } else {
                    '.'
                };
                let _ = scriptsig.push(c);
            }
        }
    }
    Some(CoinbaseInfo {
        block_height,
        scriptsig,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// A coinbase start up to the scriptsig: version, one input, null previous output.
    fn coinb1_with(scriptsig_len: u8, scriptsig_start: &[u8]) -> Vec<u8> {
        let mut c = Vec::from([1, 0, 0, 0, 1]);
        c.extend([0; 32]);
        c.extend([0xff; 4]);
        c.push(scriptsig_len);
        c.extend(scriptsig_start);
        c
    }

    #[test]
    fn height_and_tag_across_the_extranonces() {
        // height 870000 (3 bytes), "ab" in coinb1, 4 + 8 bytes of extranonces, "cd" in coinb2
        let coinb1 = coinb1_with(1 + 3 + 2 + 12 + 2, &[3, 0x70, 0x46, 0x0d, b'a', b'b']);
        let coinb2 = [b'c', b'd', 0xff, 0xff, 0xff, 0xff];
        let info = coinbase_info(&coinb1, &coinb2, 4, 8).unwrap();
        assert_eq!(info.block_height, 870_000);
        assert_eq!(info.scriptsig, "abcd");
    }

    #[test]
    fn non_printable_bytes_show_as_dots() {
        let coinb1 = coinb1_with(1 + 1 + 3, &[1, 42, b'x', 0x00, b'y']);
        let info = coinbase_info(&coinb1, &[], 4, 8).unwrap();
        assert_eq!(info.block_height, 42);
        assert_eq!(info.scriptsig, "x.y");
    }

    #[test]
    fn short_coinbases_tell_nothing() {
        assert_eq!(coinbase_info(&[0; 41], &[], 4, 8), None);
        let coinb1 = coinb1_with(5, &[5, 1, 2, 3, 4, 5]);
        assert_eq!(coinbase_info(&coinb1, &[], 4, 8), None);
    }
}
