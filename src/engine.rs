//! Samples in, state file contents out.

use crate::idle::IdleEstimator;
use crate::integrate::{Integrator, Reading, Sample};
use crate::state::{History, State};

/// The sensor's real update period is never reported above this many intervals.
const MAX_RES_INTERVALS: u64 = 4;
const RES_GAPS: usize = 9;

pub struct Engine {
    integrator: Integrator,
    idle: IdleEstimator,
    history: History,
    resolution: Resolution,
    was_busy: bool,
    state: State,
}

impl Engine {
    /// `previous` is the last state of an earlier sampler: energy and seq carry on from it.
    pub fn new(interval_ms: u64, previous: Option<&State>, idle_seed_mw: Option<u32>) -> Self {
        let start_energy = previous.map_or(0, |p| p.energy_mj);
        Self {
            integrator: Integrator::new(interval_ms, start_energy),
            idle: IdleEstimator::new(idle_seed_mw),
            history: History::default(),
            resolution: Resolution::new(interval_ms),
            was_busy: true,
            state: State {
                seq: previous.map_or(0, |p| p.seq),
                energy_mj: start_energy,
                interval_ms,
                res_ms: interval_ms,
                ..Default::default()
            },
        }
    }

    /// The sensor process was restarted.
    pub fn rebase(&mut self, source: &str) {
        self.integrator.rebase();
        self.state.source = source.to_string();
    }

    /// `busy`: some registered shell is running a command right now.
    pub fn push(&mut self, sample: &Sample, busy: bool) -> &State {
        let (energy_mj, power_mw) = self.integrator.push(sample);
        let power = power_mw.min(u64::from(u32::MAX)) as u32;
        // A sample counts as idle only if no command ran during any part of it.
        let is_idle = !busy && !self.was_busy;
        self.was_busy = busy;
        // The first sample of a joule counter has no delta yet, so no power:
        // keep it out of the history and the baseline.
        let known = power > 0;
        if known {
            self.history.push(power);
        }
        if let Reading::Watts(_) = sample.reading {
            self.state.res_ms = self.resolution.push(sample.t_ms, power);
        }
        self.state.seq += 1;
        self.state.t_ms = sample.t_ms;
        self.state.energy_mj = energy_mj;
        self.state.power_mw = power_mw;
        if known {
            self.state.idle_mw = u64::from(self.idle.push(sample.t_ms, power, is_idle));
        }
        self.state.history = self.history.snapshot();
        &self.state
    }

    pub fn idle_mw(&self) -> u32 {
        self.idle.current_mw()
    }
}

/// Estimates how often a watts sensor really produces a new value, as the
/// median time between changes.
struct Resolution {
    interval_ms: u64,
    last_value: Option<u32>,
    last_change_ms: u64,
    gaps: Vec<u64>,
}

impl Resolution {
    fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            last_value: None,
            last_change_ms: 0,
            gaps: Vec::with_capacity(RES_GAPS),
        }
    }

    fn push(&mut self, t_ms: u64, power_mw: u32) -> u64 {
        if self.last_value != Some(power_mw) {
            if self.last_value.is_some() {
                if self.gaps.len() == RES_GAPS {
                    self.gaps.remove(0);
                }
                self.gaps.push(t_ms.saturating_sub(self.last_change_ms));
            }
            self.last_value = Some(power_mw);
            self.last_change_ms = t_ms;
        }
        let mut sorted = self.gaps.clone();
        sorted.sort_unstable();
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
        // Round to the sampling grid so jitter does not move the threshold.
        let steps = (median + self.interval_ms / 2) / self.interval_ms;
        steps.clamp(1, MAX_RES_INTERVALS) * self.interval_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watts(t_ms: u64, w: f64) -> Sample {
        Sample {
            t_ms,
            reading: Reading::Watts(w),
        }
    }

    #[test]
    fn state_tracks_samples() {
        let mut e = Engine::new(500, None, None);
        e.rebase("smc-system");
        e.push(&watts(1000, 10.0), false);
        let s = e.push(&watts(1500, 12.0), false).clone();
        assert_eq!(
            (s.seq, s.t_ms, s.energy_mj, s.power_mw),
            (2, 1500, 5500, 12_000)
        );
        assert_eq!(s.source, "smc-system");
        assert_eq!(s.history[0], vec![12_000, 10_000]);
    }

    #[test]
    fn energy_and_seq_continue_from_previous_sampler() {
        let previous = State {
            seq: 40,
            energy_mj: 9_000,
            ..Default::default()
        };
        let mut e = Engine::new(500, Some(&previous), None);
        e.push(&watts(1000, 10.0), false);
        let s = e.push(&watts(1500, 10.0), false);
        assert_eq!((s.seq, s.energy_mj), (42, 14_000));
    }

    #[test]
    fn samples_next_to_a_command_are_not_idle() {
        let mut e = Engine::new(500, None, None);
        let mut t = 0;
        let mut step = |e: &mut Engine, w: f64, busy: bool| {
            t += 500;
            e.push(&watts(t, w), busy).idle_mw
        };
        for _ in 0..30 {
            step(&mut e, 8.0, false);
        }
        // A long, heavy command; the sample right after it still holds its power.
        for _ in 0..200 {
            step(&mut e, 50.0, true);
        }
        step(&mut e, 50.0, false);
        let mut idle = 0;
        for _ in 0..30 {
            idle = step(&mut e, 8.0, false);
        }
        assert_eq!(idle, 8_000);
    }

    #[test]
    fn first_joule_sample_is_not_a_zero_watt_reading() {
        let mut e = Engine::new(500, None, None);
        let joules = |t_ms, j| Sample {
            t_ms,
            reading: Reading::Joules(vec![j]),
        };
        let s = e.push(&joules(0, 100.0), false).clone();
        assert_eq!((s.power_mw, s.idle_mw), (0, 0));
        assert!(s.history[0].is_empty());
        e.push(&joules(500, 104.0), false);
        let s = e.push(&joules(1000, 108.0), false);
        assert_eq!((s.power_mw, s.idle_mw), (8_000, 8_000));
        assert_eq!(s.history[0], vec![8_000, 8_000]);
    }

    #[test]
    fn resolution_follows_a_slow_sensor() {
        let mut e = Engine::new(500, None, None);
        let mut res = 0;
        // Sampled every 500 ms, but the value only changes once a second.
        for i in 0..20u64 {
            let w = 10.0 + (i / 2) as f64;
            res = e.push(&watts(i * 500 + (i % 3), w), false).res_ms;
        }
        assert_eq!(res, 1000);
    }

    #[test]
    fn resolution_equals_interval_for_a_fast_sensor() {
        let mut e = Engine::new(200, None, None);
        let mut res = 0;
        for i in 0..20u64 {
            res = e.push(&watts(i * 200, 10.0 + i as f64), false).res_ms;
        }
        assert_eq!(res, 200);
    }

    #[test]
    fn resolution_is_capped_for_a_flat_signal() {
        let mut e = Engine::new(500, None, None);
        e.push(&watts(0, 10.0), false);
        e.push(&watts(60_000, 11.0), false);
        assert_eq!(e.push(&watts(60_500, 11.0), false).res_ms, 2000);
    }
}
