//! Turns sensor readings into a monotonic energy counter.

/// Readings above this are treated as sensor glitches.
const MAX_WATTS: f64 = 10_000.0;
/// A gap longer than this many intervals (suspend, stalled sensor) is not integrated.
const MAX_GAP_INTERVALS: u64 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// Instantaneous power (macOS SMC).
    Watts(f64),
    /// Cumulative energy counters in joules (RAPL); each one wraps on its own.
    Joules(Vec<f64>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub t_ms: u64,
    pub reading: Reading,
}

pub struct Integrator {
    interval_ms: u64,
    energy_mj: f64,
    last_t: Option<u64>,
    last_watts: f64,
    last_counters: Vec<f64>,
}

impl Integrator {
    pub fn new(interval_ms: u64, start_energy_mj: u64) -> Self {
        Self {
            interval_ms,
            energy_mj: start_energy_mj as f64,
            last_t: None,
            last_watts: 0.0,
            last_counters: Vec::new(),
        }
    }

    /// Forget the previous sample (the sensor was restarted); energy is kept.
    pub fn rebase(&mut self) {
        self.last_t = None;
        self.last_counters.clear();
    }

    /// Returns (energy_mJ, power_mW) after taking this sample into account.
    pub fn push(&mut self, s: &Sample) -> (u64, u64) {
        let dt_ms = match self.last_t {
            Some(t) if s.t_ms > t && s.t_ms - t <= MAX_GAP_INTERVALS * self.interval_ms => {
                Some(s.t_ms - t)
            }
            _ => None,
        };
        match &s.reading {
            Reading::Watts(w) => {
                let w = if valid(*w) { *w } else { self.last_watts };
                if let Some(dt) = dt_ms {
                    self.energy_mj += (self.last_watts + w) / 2.0 * dt as f64;
                }
                self.last_watts = w;
            }
            Reading::Joules(counters) => {
                if let (Some(dt), true) = (dt_ms, counters.len() == self.last_counters.len()) {
                    let dt_s = dt as f64 / 1000.0;
                    let delta: f64 = counters
                        .iter()
                        .zip(&self.last_counters)
                        .map(|(now, before)| now - before)
                        .sum();
                    let wrapped = counters
                        .iter()
                        .zip(&self.last_counters)
                        .any(|(now, before)| now < before);
                    let watts = delta / dt_s;
                    if wrapped || !valid(watts) {
                        // Counter wrap or glitch: assume power did not change.
                        self.energy_mj += self.last_watts * dt as f64;
                    } else {
                        self.energy_mj += delta * 1000.0;
                        self.last_watts = watts;
                    }
                }
                self.last_counters.clone_from(counters);
            }
        }
        self.last_t = Some(s.t_ms);
        (self.energy_mj as u64, (self.last_watts * 1000.0) as u64)
    }
}

fn valid(watts: f64) -> bool {
    watts.is_finite() && (0.0..=MAX_WATTS).contains(&watts)
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

    fn joules(t_ms: u64, c: &[f64]) -> Sample {
        Sample {
            t_ms,
            reading: Reading::Joules(c.to_vec()),
        }
    }

    #[test]
    fn trapezoid_for_watts() {
        let mut i = Integrator::new(500, 0);
        assert_eq!(i.push(&watts(1000, 10.0)), (0, 10_000));
        // (10 + 20) / 2 W for 0.5 s = 7.5 J
        assert_eq!(i.push(&watts(1500, 20.0)), (7_500, 20_000));
        assert_eq!(i.push(&watts(2000, 20.0)), (17_500, 20_000));
    }

    #[test]
    fn absurd_watts_are_replaced_by_last_value() {
        let mut i = Integrator::new(500, 0);
        i.push(&watts(0, 10.0));
        assert_eq!(i.push(&watts(500, 1e9)), (5_000, 10_000));
        assert_eq!(i.push(&watts(1000, -3.0)), (10_000, 10_000));
        assert_eq!(i.push(&watts(1500, f64::NAN)), (15_000, 10_000));
    }

    #[test]
    fn joule_deltas_sum_all_counters() {
        let mut i = Integrator::new(500, 0);
        i.push(&joules(0, &[100.0, 10.0]));
        // 4 J package + 1 J DRAM in 0.5 s = 10 W
        assert_eq!(i.push(&joules(500, &[104.0, 11.0])), (5_000, 10_000));
    }

    #[test]
    fn wrap_falls_back_to_last_power() {
        let mut i = Integrator::new(500, 0);
        i.push(&joules(0, &[262_130.0]));
        assert_eq!(i.push(&joules(500, &[262_135.0])), (5_000, 10_000));
        // The counter wrapped: 10 W for 0.5 s is assumed.
        assert_eq!(i.push(&joules(1000, &[1.0])), (10_000, 10_000));
        // Normal deltas resume from the new counter value.
        assert_eq!(i.push(&joules(1500, &[4.0])), (13_000, 6_000));
    }

    #[test]
    fn absurd_joule_jump_falls_back_to_last_power() {
        let mut i = Integrator::new(500, 0);
        i.push(&joules(0, &[0.0]));
        i.push(&joules(500, &[5.0]));
        assert_eq!(i.push(&joules(1000, &[900_000.0])), (10_000, 10_000));
    }

    #[test]
    fn long_gaps_are_not_integrated() {
        let mut i = Integrator::new(500, 0);
        i.push(&watts(0, 10.0));
        i.push(&watts(500, 10.0));
        // One hour asleep must not count as one hour at 10 W.
        assert_eq!(i.push(&watts(3_600_000, 10.0)), (5_000, 10_000));
        assert_eq!(i.push(&watts(3_600_500, 10.0)), (10_000, 10_000));
    }

    #[test]
    fn energy_survives_a_sensor_restart() {
        let mut i = Integrator::new(500, 42_000);
        i.push(&joules(0, &[50.0]));
        i.push(&joules(500, &[55.0]));
        i.rebase();
        // The new sensor process starts from a different counter base.
        assert_eq!(i.push(&joules(900, &[3.0])).0, 47_000);
        assert_eq!(i.push(&joules(1400, &[5.0])).0, 49_000);
    }

    #[test]
    fn time_going_backwards_adds_nothing() {
        let mut i = Integrator::new(500, 0);
        i.push(&watts(1000, 10.0));
        assert_eq!(i.push(&watts(900, 10.0)).0, 0);
    }
}
