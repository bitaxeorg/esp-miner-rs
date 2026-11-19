//! The best nonces ever found, most difficult first.

use core::fmt::Write;

use heapless::{String, Vec};

/// Entries kept.
pub const SCOREBOARD_SIZE: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub difficulty: f64,
    pub job_id: String<32>,
    /// Hex.
    pub extranonce2: String<16>,
    pub ntime: u32,
    pub nonce: u32,
    pub version_bits: u32,
}

impl Entry {
    /// The stored form: `difficulty;job;extranonce2;ntime;nonce;version bits`, difficulty with
    /// one decimal, nonce and version bits in 8 uppercase hex digits.
    pub fn encode(&self) -> String<96> {
        let mut s = String::new();
        let _ = write!(
            s,
            "{:.1};{};{};{};{:08X};{:08X}",
            self.difficulty,
            self.job_id,
            self.extranonce2,
            self.ntime,
            self.nonce,
            self.version_bits
        );
        s
    }

    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.split(';');
        let difficulty = parts.next()?.parse().ok()?;
        let job_id = String::try_from(parts.next()?).ok()?;
        let extranonce2 = String::try_from(parts.next()?).ok()?;
        let ntime = parts.next()?.parse().ok()?;
        let nonce = u32::from_str_radix(parts.next()?, 16).ok()?;
        let version_bits = u32::from_str_radix(parts.next()?, 16).ok()?;
        Some(Self {
            difficulty,
            job_id,
            extranonce2,
            ntime,
            nonce,
            version_bits,
        })
    }
}

/// The [`SCOREBOARD_SIZE`] most difficult entries, sorted by difficulty, highest first.
#[derive(Default)]
pub struct Scoreboard {
    entries: Vec<Entry, SCOREBOARD_SIZE>,
}

impl Scoreboard {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether an entry of `difficulty` would make it to the board.
    pub fn qualifies(&self, difficulty: f64) -> bool {
        !self.entries.is_full()
            || self
                .entries
                .last()
                .is_some_and(|e| difficulty > e.difficulty)
    }

    /// Adds `entry` if it makes it to the board. Returns whether it did.
    pub fn consider(&mut self, entry: Entry) -> bool {
        if !self.qualifies(entry.difficulty) {
            return false;
        }
        if self.entries.is_full() {
            self.entries.pop();
        }
        let at = self
            .entries
            .iter()
            .position(|e| entry.difficulty > e.difficulty)
            .unwrap_or(self.entries.len());
        self.entries.insert(at, entry).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(difficulty: f64) -> Entry {
        Entry {
            difficulty,
            job_id: String::try_from("6a1f").unwrap(),
            extranonce2: String::try_from("00000001").unwrap(),
            ntime: 1_700_000_000,
            nonce: 0x1234_abcd,
            version_bits: 0x0020_0000,
        }
    }

    #[test]
    fn keeps_the_best_sorted() {
        let mut board = Scoreboard::new();
        for d in 1..=25 {
            board.consider(entry(d as f64));
        }
        assert_eq!(board.entries().len(), SCOREBOARD_SIZE);
        assert_eq!(board.entries()[0].difficulty, 25.0);
        assert_eq!(board.entries()[19].difficulty, 6.0);
        assert!(!board.consider(entry(5.0)));
        assert!(board.consider(entry(7.5)));
        assert_eq!(board.entries()[18].difficulty, 7.5);
        assert_eq!(board.entries()[19].difficulty, 7.0);
    }

    #[test]
    fn stored_form_round_trips() {
        let e = entry(123456.75);
        assert_eq!(
            e.encode(),
            "123456.8;6a1f;00000001;1700000000;1234ABCD;00200000"
        );
        let back = Entry::decode(&e.encode()).unwrap();
        assert_eq!(back.difficulty, 123456.8);
        assert_eq!(back.nonce, e.nonce);
        assert_eq!(back.version_bits, e.version_bits);
        assert_eq!(Entry::decode("garbage"), None);
    }
}
