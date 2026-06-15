//! Rewind: a bounded ring of machine-state snapshots. During normal play the
//! frontend records a snapshot every few frames; holding the rewind key pops
//! them newest-first to step gameplay backward. Memory is bounded by a byte
//! budget — the snapshot-count cap is derived from the first snapshot's size,
//! so small (GB) states get a long window and large (GBA) states a shorter one.

use std::collections::VecDeque;

/// Frames between snapshots during normal play (~0.1s at 59.73 Hz).
const INTERVAL: u32 = 6;
/// RAM ceiling for the snapshot ring. 64 MiB keeps termboy's footprint modest
/// while still giving a useful window — minutes on GB/GBC (tiny states), ~12 s
/// on GBA (~0.5 MiB states). This is the bulk of the process's memory, so the
/// cap is deliberately conservative.
const BUDGET_BYTES: usize = 64 * 1024 * 1024;

pub struct Rewind {
    ring: VecDeque<Vec<u8>>,
    budget: usize,
    interval: u32,
    frame: u32,
    cap: usize, // 0 until the first snapshot sizes the ring
}

impl Default for Rewind {
    fn default() -> Self {
        Self::new()
    }
}

impl Rewind {
    pub fn new() -> Self {
        Self::with(BUDGET_BYTES, INTERVAL)
    }

    fn with(budget: usize, interval: u32) -> Self {
        Self { ring: VecDeque::new(), budget, interval, frame: 0, cap: 0 }
    }

    /// Record a snapshot on the interval. `snapshot` is only invoked on the
    /// frames a snapshot is actually taken, so its cost is paid ~1/INTERVAL.
    pub fn record(&mut self, snapshot: impl FnOnce() -> Vec<u8>) {
        if self.frame == 0 {
            let snap = snapshot();
            if self.cap == 0 {
                // Size the ring from the first snapshot so memory is bounded
                // regardless of which core (GB states are tiny, GBA large).
                self.cap = (self.budget / snap.len().max(1)).max(1);
            }
            self.ring.push_back(snap);
            while self.ring.len() > self.cap {
                self.ring.pop_front();
            }
        }
        self.frame = (self.frame + 1) % self.interval.max(1);
    }

    /// Pop the most recent snapshot to rewind to, or `None` if exhausted.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.ring.pop_back()
    }

    #[cfg(test)]
    fn len_for_test(&self) -> usize {
        self.ring.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_first_frame_then_every_interval() {
        let mut r = Rewind::with(BUDGET_BYTES, 6);
        r.record(|| vec![0u8; 8]); // frame 0 -> capture
        assert_eq!(r.len_for_test(), 1);
        for _ in 0..5 {
            r.record(|| vec![0u8; 8]); // frames 1..=5 -> no capture
        }
        assert_eq!(r.len_for_test(), 1);
        r.record(|| vec![0u8; 8]); // frame 6 -> capture
        assert_eq!(r.len_for_test(), 2);
    }

    #[test]
    fn byte_budget_caps_snapshot_count_dropping_oldest() {
        // budget 25 / 10-byte snapshots -> cap 2. Interval 1 captures every call.
        let mut r = Rewind::with(25, 1);
        r.record(|| vec![1u8; 10]);
        r.record(|| vec![2u8; 10]);
        r.record(|| vec![3u8; 10]); // oldest (the 1s) drops
        assert_eq!(r.len_for_test(), 2);
        assert_eq!(r.pop(), Some(vec![3u8; 10])); // newest first
        assert_eq!(r.pop(), Some(vec![2u8; 10]));
        assert_eq!(r.pop(), None); // the 1s were dropped
    }

    #[test]
    fn pop_drains_newest_first_then_empties() {
        let mut r = Rewind::with(BUDGET_BYTES, 1);
        r.record(|| vec![1]);
        r.record(|| vec![2]);
        assert_eq!(r.len_for_test(), 2);
        assert_eq!(r.pop(), Some(vec![2]));
        assert_eq!(r.pop(), Some(vec![1]));
        assert_eq!(r.pop(), None); // drained
    }
}
