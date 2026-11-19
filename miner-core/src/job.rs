//! Mining jobs, and the book that matches the chips' results back to them.

use heapless::{String, Vec};

use crate::header::{reversed, BlockHeader};

/// A job ready to be sent to the chips.
///
/// Hashes are kept in internal byte order (as serialized in a block header).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MiningJob {
    /// Pool job id, echoed back in `mining.submit`.
    pub pool_job_id: String<32>,
    /// Extranonce2 the coinbase of this job was built with, echoed back in `mining.submit`.
    pub extranonce2: Vec<u8, 8>,
    pub version: u32,
    pub prev_blockhash: [u8; 32],
    pub merkle_root: [u8; 32],
    pub ntime: u32,
    pub nbits: u32,
}

impl MiningJob {
    /// Builds a job from the fields of a `stratum_v1::Job`.
    ///
    /// stratum-v1 gives `prev_blockhash` in display order and `merkle_root` in internal byte
    /// order.
    pub fn from_stratum_v1(
        pool_job_id: String<32>,
        extranonce2: Vec<u8, 8>,
        version: i32,
        prev_blockhash_display: [u8; 32],
        merkle_root: [u8; 32],
        ntime: u32,
        nbits: u32,
    ) -> Self {
        Self {
            pool_job_id,
            extranonce2,
            version: version as u32,
            prev_blockhash: reversed(prev_blockhash_display),
            merkle_root,
            ntime,
            nbits,
        }
    }

    /// Previous block hash in display order, as `bm13xx_chain::Chain::send_job` expects it.
    pub fn prev_blockhash_display(&self) -> [u8; 32] {
        reversed(self.prev_blockhash)
    }

    /// Merkle root in display order, as `bm13xx_chain::Chain::send_job` expects it.
    pub fn merkle_root_display(&self) -> [u8; 32] {
        reversed(self.merkle_root)
    }

    /// The header hashed by a chip that found `nonce` with `version`.
    pub fn header(&self, version: u32, nonce: u32) -> BlockHeader {
        BlockHeader {
            version,
            prev_blockhash: self.prev_blockhash,
            merkle_root: self.merkle_root,
            ntime: self.ntime,
            nbits: self.nbits,
            nonce,
        }
    }
}

/// The jobs sent to the chain, indexed by the job id the chain gave them.
pub struct JobBook<const N: usize> {
    jobs: [Option<MiningJob>; N],
}

impl<const N: usize> JobBook<N> {
    pub const fn new() -> Self {
        Self {
            jobs: [const { None }; N],
        }
    }

    /// Records the job sent with `id`, replacing the previous job with that id.
    ///
    /// An `id` outside the book is ignored: its results will be reported for an unknown job.
    pub fn insert(&mut self, id: u8, job: MiningJob) {
        if let Some(slot) = self.jobs.get_mut(id as usize) {
            *slot = Some(job);
        }
    }

    pub fn get(&self, id: u8) -> Option<&MiningJob> {
        self.jobs.get(id as usize)?.as_ref()
    }

    /// Forgets all jobs, when the pool asks to drop the current work.
    pub fn clear(&mut self) {
        self.jobs.iter_mut().for_each(|slot| *slot = None);
    }
}

impl<const N: usize> Default for JobBook<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use hex_literal::hex;

    /// The genesis block header as stratum-v1 would hand it.
    pub(crate) fn genesis_job() -> MiningJob {
        MiningJob::from_stratum_v1(
            String::try_from("genesis").unwrap(),
            Vec::from_slice(&[0, 0, 0, 1]).unwrap(),
            1,
            [0; 32],
            reversed(hex!(
                "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
            )),
            1_231_006_505,
            0x1d00_ffff,
        )
    }

    #[test]
    fn stratum_v1_byte_orders() {
        let prev_display =
            hex!("00000000000000000002 86ff0ea5a45bfc00d35556d50eae1ad876d71f99e138");
        let merkle_internal = [7; 32];
        let job = MiningJob::from_stratum_v1(
            String::new(),
            Vec::new(),
            0x2000_0000,
            prev_display,
            merkle_internal,
            0,
            0,
        );
        assert_eq!(job.prev_blockhash, reversed(prev_display));
        assert_eq!(job.prev_blockhash_display(), prev_display);
        assert_eq!(job.merkle_root, merkle_internal);
        assert_eq!(job.merkle_root_display(), reversed(merkle_internal));
    }

    #[test]
    fn header_of_genesis_job() {
        let job = genesis_job();
        assert_eq!(
            reversed(job.header(1, 2_083_236_893).hash()),
            hex!("000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f")
        );
    }

    #[test]
    fn job_book() {
        let mut book = JobBook::<16>::new();
        assert!(book.get(3).is_none());
        book.insert(3, genesis_job());
        assert_eq!(book.get(3), Some(&genesis_job()));
        // out of range ids are ignored
        book.insert(16, genesis_job());
        assert!(book.get(16).is_none());
        book.clear();
        assert!(book.get(3).is_none());
    }
}
