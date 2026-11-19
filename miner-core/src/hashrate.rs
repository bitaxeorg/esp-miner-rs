//! Hashrate measured from the nonces the chips return.

/// Hashes behind one nonce at difficulty 1, on average.
const HASHES_PER_DIFFICULTY: f64 = 4_294_967_296.0;

/// Nonces counted over a sliding window of `N` buckets of `bucket_ms` each.
///
/// The chips only return nonces at least as difficult as their ticket mask, so each nonce stands
/// for `difficulty × 2^32` hashes on average. The hashrate is the hashes of the nonces in the
/// window divided by the window, or by the time since the start while the window isn't full yet.
pub struct HashrateMeter<const N: usize> {
    counts: [u32; N],
    /// Bucket of `counts` the current time falls in.
    current: usize,
    /// Start of the current bucket (ms).
    bucket_start_ms: u64,
    bucket_ms: u64,
    start_ms: u64,
}

impl<const N: usize> HashrateMeter<N> {
    /// A meter starting at `now_ms`, its window `N × bucket_ms` long.
    pub fn new(now_ms: u64, bucket_ms: u64) -> Self {
        Self {
            counts: [0; N],
            current: 0,
            bucket_start_ms: now_ms,
            bucket_ms: bucket_ms.max(1),
            start_ms: now_ms,
        }
    }

    /// Counts a nonce returned at `now_ms`.
    pub fn record(&mut self, now_ms: u64) {
        self.advance(now_ms);
        self.counts[self.current] = self.counts[self.current].saturating_add(1);
    }

    /// Hashrate (GH/s) at `now_ms` of chips returning nonces at `difficulty`.
    pub fn ghs(&mut self, now_ms: u64, difficulty: u32) -> f64 {
        self.advance(now_ms);
        let window_ms = (now_ms - self.start_ms).min(N as u64 * self.bucket_ms);
        if window_ms == 0 {
            return 0.0;
        }
        let nonces: u64 = self.counts.iter().map(|&c| u64::from(c)).sum();
        nonces as f64 * f64::from(difficulty) * HASHES_PER_DIFFICULTY
            / (window_ms as f64 / 1000.0)
            / 1e9
    }

    /// Moves the current bucket to `now_ms`, emptying the buckets left behind.
    fn advance(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.bucket_start_ms) / self.bucket_ms;
        for _ in 0..elapsed.min(N as u64) {
            self.current = (self.current + 1) % N;
            self.counts[self.current] = 0;
        }
        self.bucket_start_ms += elapsed * self.bucket_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn no_nonce_no_hashrate() {
        let mut meter = HashrateMeter::<6>::new(1_000, 10_000);
        assert_eq!(meter.ghs(1_000, 256), 0.0);
        assert_eq!(meter.ghs(31_000, 256), 0.0);
    }

    #[test]
    fn partial_window_uses_the_elapsed_time() {
        let mut meter = HashrateMeter::<6>::new(0, 10_000);
        for t in 0..10 {
            meter.record(t * 2_000);
        }
        // 10 nonces × 256 × 2^32 hashes in 20 s
        assert_close(meter.ghs(20_000, 256), 549.76);
    }

    #[test]
    fn full_window_drops_the_oldest_buckets() {
        let mut meter = HashrateMeter::<6>::new(0, 10_000);
        for t in 0..60 {
            meter.record(t * 1_000);
        }
        // 60 nonces in the 59.999 s since the start
        assert_close(meter.ghs(60_000 - 1, 256), 1_099.53);
        // 30 s later the first 30 nonces have left the window
        assert_close(meter.ghs(90_000 - 1, 256), 549.76);
        // and after a whole window without nonces nothing is left
        assert_eq!(meter.ghs(200_000, 256), 0.0);
    }
}
