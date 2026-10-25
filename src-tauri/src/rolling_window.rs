//! Bounded rolling window used by the latency diagnostics panel to keep the
//! most recent samples without growing without bound.
//!
//! This module is deliberately dependency-free so it can be unit-tested in
//! isolation and reused by both the backend sampler and the UI projection.

/// A fixed-capacity window of `u64` samples that drops the oldest entry once
/// it is full.
#[derive(Clone, Debug, Default)]
pub struct RollingWindow {
    capacity: usize,
    samples: std::collections::VecDeque<u64>,
}

impl RollingWindow {
    /// Create a window holding at most `capacity` samples.
    ///
    /// A capacity of zero is treated as one so the window never divides by
    /// zero when reporting an average.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            samples: std::collections::VecDeque::new(),
        }
    }

    /// Record a sample, evicting the oldest one when at capacity.
    pub fn push(&mut self, sample: u64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Number of retained samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when no sample has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Retained samples, oldest first.
    pub fn samples(&self) -> impl Iterator<Item = u64> + '_ {
        self.samples.iter().copied()
    }

    /// Mean of the retained samples, or `None` when empty.
    pub fn mean(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let total: u64 = self.samples.iter().sum();
        Some(total as f64 / self.samples.len() as f64)
    }

    /// The 95th percentile of the retained samples, or `None` when empty.
    ///
    /// Uses nearest-rank selection on a sorted copy; small windows are cheap
    /// enough that sorting beats maintaining a heap.
    pub fn p95(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let rank = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let index = rank.saturating_sub(1).min(sorted.len() - 1);
        Some(sorted[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut window = RollingWindow::new(3);
        for sample in [10, 20, 30, 40] {
            window.push(sample);
        }
        assert_eq!(window.len(), 3);
        assert_eq!(window.samples().collect::<Vec<_>>(), vec![20, 30, 40]);
    }

    #[test]
    fn zero_capacity_behaves_as_one() {
        let mut window = RollingWindow::new(0);
        window.push(5);
        window.push(9);
        assert_eq!(window.len(), 1);
        assert_eq!(window.mean(), Some(9.0));
    }

    #[test]
    fn empty_window_has_no_statistics() {
        let window = RollingWindow::new(4);
        assert!(window.is_empty());
        assert_eq!(window.mean(), None);
        assert_eq!(window.p95(), None);
    }

    #[test]
    fn p95_uses_nearest_rank() {
        let mut window = RollingWindow::new(20);
        for sample in 1..=20 {
            window.push(sample);
        }
        assert_eq!(window.p95(), Some(19));
        assert_eq!(window.mean(), Some(10.5));
    }
}
