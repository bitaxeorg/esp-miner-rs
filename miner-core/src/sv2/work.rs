//! The work of a Stratum V2 mining channel: the jobs the pool sent, the block they build on, and
//! the [`MiningJob`]s rolled from the active one.
//!
//! A job with an `ntime_start` is for the current block and becomes active at once. A job without
//! one is for a block to come: it waits for the `SetNewPrevHash` that names it, which also makes
//! every other waiting job stale.

use alloc::vec::Vec as AllocVec;
use core::fmt::Write as _;

use bitcoin_hashes::sha256d;
use heapless::{String, Vec};

use crate::job::{MiningJob, EXTRANONCE_LEN_MAX};

/// Jobs for a coming block kept at most; the oldest goes first.
const FUTURE_JOBS: usize = 4;

/// Where the merkle root of a job comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merkle {
    /// Standard channel: the pool's merkle root, in internal byte order.
    Root([u8; 32]),
    /// Extended channel: the coinbase around the extranonce (its serialization without witness),
    /// and the merkle path of the coinbase, in internal byte order.
    Coinbase {
        prefix: AllocVec<u8>,
        suffix: AllocVec<u8>,
        path: AllocVec<[u8; 32]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: u32,
    pub version: u32,
    /// The pool lets the version bits be rolled for this job.
    pub version_rolling: bool,
    pub merkle: Merkle,
}

/// The block jobs build on, from `SetNewPrevHash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrevHash {
    /// The job it was sent for.
    pub job_id: u32,
    /// Previous block hash, in internal byte order.
    pub hash: [u8; 32],
    /// Earliest ntime of the block.
    pub ntime: u32,
    pub nbits: u32,
}

#[derive(Debug, Default)]
pub struct Work {
    future: AllocVec<Job>,
    prev: Option<PrevHash>,
    /// The active job and its ntime.
    active: Option<(Job, u32)>,
    /// The active job was handed out once (a standard job goes to the chips once).
    sent: bool,
    /// Next extranonce of the active extended job.
    extranonce: u64,
}

impl Work {
    /// A job from the pool, for the current block with its `ntime_start`, or for a block to
    /// come without. Returns whether it is the active job now.
    pub fn new_job(&mut self, job: Job, ntime_start: Option<u32>) -> bool {
        match ntime_start {
            Some(ntime) => {
                self.activate(job, ntime);
                self.prev.is_some()
            }
            None => {
                if self.future.len() == FUTURE_JOBS {
                    self.future.remove(0);
                }
                self.future.push(job);
                false
            }
        }
    }

    /// A new block: the job it names becomes the active job, the other jobs are stale. Returns
    /// whether a job is active now.
    pub fn new_prev_hash(&mut self, prev: PrevHash) -> bool {
        self.prev = Some(prev);
        if let Some(i) = self.future.iter().position(|j| j.id == prev.job_id) {
            let job = self.future.remove(i);
            self.activate(job, prev.ntime);
        } else if let Some((job, _)) = self.active.take() {
            if job.id == prev.job_id {
                self.activate(job, prev.ntime);
            }
        }
        self.future.clear();
        self.active.is_some()
    }

    fn activate(&mut self, job: Job, ntime: u32) {
        self.active = Some((job, ntime));
        self.sent = false;
        self.extranonce = 0;
    }

    /// The active job, and the block it builds on.
    pub fn active(&self) -> Option<(&Job, &PrevHash)> {
        Some((&self.active.as_ref()?.0, self.prev.as_ref()?))
    }

    /// Forgets the jobs, keeping the block.
    pub fn clear_jobs(&mut self) {
        self.future.clear();
        self.active = None;
    }

    /// The next job for the chips, `None` without active work. An extended job comes with a new
    /// extranonce of `extranonce_size` bytes each time, after `extranonce_prefix`; a standard
    /// job, which can't be rolled here, comes once.
    pub fn roll(&mut self, extranonce_prefix: &[u8], extranonce_size: usize) -> Option<MiningJob> {
        let prev = self.prev?;
        let (job, ntime) = self.active.as_ref()?;
        let size = extranonce_size.min(EXTRANONCE_LEN_MAX);
        let (merkle_root, extranonce2) = match &job.merkle {
            Merkle::Coinbase {
                prefix,
                suffix,
                path,
            } if size > 0 => {
                let counter = self.extranonce.to_le_bytes();
                let extranonce: Vec<u8, EXTRANONCE_LEN_MAX> = (0..size)
                    .map(|i| counter.get(i).copied().unwrap_or(0))
                    .collect();
                self.extranonce = self.extranonce.wrapping_add(1);
                let root = merkle_root(prefix, extranonce_prefix, &extranonce, suffix, path);
                (root, extranonce)
            }
            _ if self.sent => return None,
            Merkle::Root(root) => (*root, Vec::new()),
            Merkle::Coinbase {
                prefix,
                suffix,
                path,
            } => (
                merkle_root(prefix, extranonce_prefix, &[], suffix, path),
                Vec::new(),
            ),
        };
        self.sent = true;
        let mut pool_job_id = String::new();
        let _ = write!(pool_job_id, "{}", job.id);
        Some(MiningJob {
            pool_job_id,
            extranonce2,
            version: job.version,
            prev_blockhash: prev.hash,
            merkle_root,
            ntime: *ntime,
            nbits: prev.nbits,
        })
    }
}

/// Merkle root, in internal byte order, of a block whose coinbase is `prefix`, the extranonce
/// prefix and extranonce, then `suffix`, with the merkle `path` of the coinbase.
pub fn merkle_root(
    prefix: &[u8],
    extranonce_prefix: &[u8],
    extranonce: &[u8],
    suffix: &[u8],
    path: &[[u8; 32]],
) -> [u8; 32] {
    let mut coinbase = AllocVec::with_capacity(
        prefix.len() + extranonce_prefix.len() + extranonce.len() + suffix.len(),
    );
    for part in [prefix, extranonce_prefix, extranonce, suffix] {
        coinbase.extend_from_slice(part);
    }
    let mut root = sha256d::Hash::hash(&coinbase).to_byte_array();
    for branch in path {
        let mut pair = [0u8; 64];
        pair[..32].copy_from_slice(&root);
        pair[32..].copy_from_slice(branch);
        root = sha256d::Hash::hash(&pair).to_byte_array();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::reversed;
    use hex_literal::hex;

    fn prev(job_id: u32) -> PrevHash {
        PrevHash {
            job_id,
            hash: [0x11; 32],
            ntime: 1_700_000_000,
            nbits: 0x1703_0ecd,
        }
    }

    fn standard(id: u32) -> Job {
        Job {
            id,
            version: 0x2000_0000,
            version_rolling: true,
            merkle: Merkle::Root([id as u8; 32]),
        }
    }

    fn extended(id: u32) -> Job {
        Job {
            id,
            version: 0x2000_0000,
            version_rolling: true,
            merkle: Merkle::Coinbase {
                prefix: alloc::vec![1, 2, 3],
                suffix: alloc::vec![4, 5],
                path: alloc::vec![[7; 32]],
            },
        }
    }

    #[test]
    fn a_future_job_waits_for_its_block() {
        let mut work = Work::default();
        assert!(!work.new_job(standard(1), None));
        assert!(!work.new_job(standard(2), None));
        assert!(work.roll(&[], 0).is_none());
        assert!(work.new_prev_hash(prev(2)));
        let job = work.roll(&[], 0).unwrap();
        assert_eq!(job.pool_job_id.as_str(), "2");
        assert_eq!(job.merkle_root, [2; 32]);
        assert_eq!(
            (job.prev_blockhash, job.ntime, job.nbits),
            ([0x11; 32], 1_700_000_000, 0x1703_0ecd)
        );
        // a standard job goes to the chips once
        assert!(work.roll(&[], 0).is_none());
        // job 1 was dropped with the new block
        assert!(!work.new_prev_hash(prev(1)));
    }

    #[test]
    fn a_job_for_the_current_block_is_active_at_once() {
        let mut work = Work::default();
        // no block yet
        assert!(!work.new_job(standard(5), Some(1_700_000_100)));
        assert!(work.new_prev_hash(prev(5)));
        assert!(work.new_job(standard(6), Some(1_700_000_200)));
        let job = work.roll(&[], 0).unwrap();
        assert_eq!((job.pool_job_id.as_str(), job.ntime), ("6", 1_700_000_200));
        // a new block for another job leaves no work
        assert!(!work.new_prev_hash(prev(9)));
        assert!(work.roll(&[], 0).is_none());
    }

    #[test]
    fn an_extended_job_rolls_its_extranonce() {
        let mut work = Work::default();
        work.new_job(extended(3), None);
        work.new_prev_hash(prev(3));
        let a = work.roll(&[0xaa, 0xbb], 4).unwrap();
        let b = work.roll(&[0xaa, 0xbb], 4).unwrap();
        assert_eq!(a.extranonce2.as_slice(), &[0, 0, 0, 0]);
        assert_eq!(b.extranonce2.as_slice(), &[1, 0, 0, 0]);
        assert_ne!(a.merkle_root, b.merkle_root);
        let coinbase = [1, 2, 3, 0xaa, 0xbb, 1, 0, 0, 0, 4, 5];
        let txid = sha256d::Hash::hash(&coinbase).to_byte_array();
        let mut pair = [7u8; 64];
        pair[..32].copy_from_slice(&txid);
        assert_eq!(b.merkle_root, sha256d::Hash::hash(&pair).to_byte_array());
    }

    #[test]
    fn the_merkle_root_of_a_block_with_one_transaction_is_its_txid() {
        // the genesis coinbase, split around 4 bytes of its scriptsig
        let coinbase = hex!(
            "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff"
            "4d04ffff001d0104455468652054696d65732030332f4a616e2f32303039204368616e63656c6c6f72"
            "206f6e206272696e6b206f66207365636f6e64206261696c6f757420666f722062616e6b73ffffffff"
            "0100f2052a01000000434104678afdb0fe5548271967f1a67130b7105cd6a828e03909a67962e0ea1f"
            "61deb649f6bc3f4cef38c4f35504e51ec112de5c384df7ba0b8d578a4c702b6bf11d5fac00000000"
        );
        let root = merkle_root(
            &coinbase[..42],
            &coinbase[42..44],
            &coinbase[44..46],
            &coinbase[46..],
            &[],
        );
        assert_eq!(
            reversed(root),
            hex!("4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b")
        );
    }
}
