//! Idle baseline: a low percentile of power while no shell is running a command.

use std::collections::VecDeque;

const WINDOW_MS: u64 = 5 * 60 * 1000;
const PERCENTILE: usize = 10;
/// Idle-only samples needed before they replace the provisional estimate.
const MIN_IDLE_SAMPLES: usize = 10;
const RECOMPUTE_EVERY: u32 = 10;

pub struct IdleEstimator {
    idle: VecDeque<(u64, u32)>,
    any: VecDeque<(u64, u32)>,
    seed_mw: Option<u32>,
    current_mw: u32,
    since_recompute: u32,
}

impl IdleEstimator {
    /// `seed_mw` is the baseline saved by the previous sampler, if any.
    pub fn new(seed_mw: Option<u32>) -> Self {
        Self {
            idle: VecDeque::new(),
            any: VecDeque::new(),
            seed_mw,
            current_mw: seed_mw.unwrap_or(0),
            since_recompute: RECOMPUTE_EVERY,
        }
    }

    pub fn push(&mut self, t_ms: u64, power_mw: u32, is_idle: bool) -> u32 {
        let cutoff = t_ms.saturating_sub(WINDOW_MS);
        for (queue, take) in [(&mut self.any, true), (&mut self.idle, is_idle)] {
            if take {
                queue.push_back((t_ms, power_mw));
            }
            while queue.front().is_some_and(|(t, _)| *t < cutoff) {
                queue.pop_front();
            }
        }
        self.since_recompute += 1;
        // Every sample while there are few, then every tenth.
        if self.since_recompute >= RECOMPUTE_EVERY || self.any.len() <= 2 * MIN_IDLE_SAMPLES {
            self.since_recompute = 0;
            self.current_mw = self.estimate();
        }
        self.current_mw
    }

    pub fn current_mw(&self) -> u32 {
        self.current_mw
    }

    fn estimate(&self) -> u32 {
        if self.idle.len() >= MIN_IDLE_SAMPLES {
            percentile(&self.idle)
        } else if let Some(seed) = self.seed_mw {
            seed
        } else {
            // Nothing better yet: the quietest moments seen so far, busy or not.
            percentile(&self.any)
        }
    }
}

fn percentile(samples: &VecDeque<(u64, u32)>) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    let mut values: Vec<u32> = samples.iter().map(|(_, p)| *p).collect();
    let k = (values.len() - 1) * PERCENTILE / 100;
    *values.select_nth_unstable(k).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p10_of_idle_samples() {
        let mut est = IdleEstimator::new(None);
        let mut idle = 0;
        // Idle samples at 1000..=1099 mW in scrambled order (1000 appears twice).
        for i in 0..=100u64 {
            idle = est.push(i * 500, 1000 + ((i * 37) % 100) as u32, true);
        }
        assert_eq!(idle, 1009);
    }

    #[test]
    fn busy_samples_do_not_raise_the_baseline() {
        let mut est = IdleEstimator::new(None);
        for i in 0..50u64 {
            est.push(i * 500, 8_000, true);
        }
        let mut idle = 0;
        for i in 50..400u64 {
            idle = est.push(i * 500, 45_000, false);
        }
        assert_eq!(idle, 8_000);
    }

    #[test]
    fn seed_is_used_until_enough_idle_samples() {
        let mut est = IdleEstimator::new(Some(7_000));
        assert_eq!(est.current_mw(), 7_000);
        for i in 0..5u64 {
            assert_eq!(est.push(i * 500, 30_000, false), 7_000);
        }
        let mut idle = 0;
        for i in 5..25u64 {
            idle = est.push(i * 500, 9_000, true);
        }
        assert_eq!(idle, 9_000);
    }

    #[test]
    fn without_seed_or_idle_samples_uses_lowest_power_seen() {
        let mut est = IdleEstimator::new(None);
        let mut idle = 0;
        for i in 0..20u64 {
            let p = if i < 5 { 6_000 } else { 30_000 };
            idle = est.push(i * 500, p, false);
        }
        assert_eq!(idle, 6_000);
    }

    #[test]
    fn old_samples_leave_the_window() {
        let mut est = IdleEstimator::new(None);
        for i in 0..20u64 {
            est.push(i * 500, 5_000, true);
        }
        // Ten minutes later the machine idles higher; the old 5 W is forgotten.
        let later = 600_000;
        let mut idle = 0;
        for i in 0..20u64 {
            idle = est.push(later + i * 500, 12_000, true);
        }
        assert_eq!(idle, 12_000);
    }
}
