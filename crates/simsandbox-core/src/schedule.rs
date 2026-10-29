//! When something samples, step by step (ADR 0062, ADR 0063): every N
//! steps, every so much simulated time, or at listed times, at the first
//! communication point at or after each. Probes record by it, and a probe
//! feeding its model holds its input by it, so both sample alike.

/// When a probe records a sample.
#[derive(Debug, Clone, PartialEq)]
pub enum Every {
    /// Every `n` steps of the run (counted from t = 0), the start included.
    Steps(u64),
    /// Every `dt` of simulated time, from t = 0: at the first step at or
    /// after each multiple.
    Seconds(f64),
    /// At the first step at or after each of these times, in order.
    Times(Vec<f64>),
}

/// Decides, step by step, when a probe records.
#[derive(Debug, Clone)]
pub struct Trigger {
    every: Every,
    /// For `Seconds`: the next multiple due. For `Times`: the next index.
    next_time: f64,
    next_index: usize,
}

impl Trigger {
    pub fn new(every: Every) -> Self {
        let mut t = Self {
            every,
            next_time: 0.0,
            next_index: 0,
        };
        t.seek(None, 0.0);
        t
    }

    /// Whether to record the sample at time `t` after `step` steps; `eps`
    /// absorbs rounding in the times. Call once per step, in order.
    pub fn due(&mut self, t: f64, step: u64, eps: f64) -> bool {
        match &self.every {
            Every::Steps(n) => step % *n == 0,
            Every::Seconds(dt) => {
                if t + eps < self.next_time {
                    return false;
                }
                // The next multiple strictly after t.
                self.next_time = ((t + eps) / dt).floor() * dt + dt;
                true
            }
            Every::Times(ts) => {
                let mut due = false;
                while self.next_index < ts.len() && ts[self.next_index] <= t + eps {
                    self.next_index += 1;
                    due = true;
                }
                due
            }
        }
    }

    /// After a rewind or restart to `t`: the samples at `t` and before are
    /// recorded (`Some(t)`), or nothing is (`None`, a fresh start).
    pub fn seek(&mut self, recorded_up_to: Option<f64>, eps: f64) {
        match (&self.every, recorded_up_to) {
            (Every::Seconds(_), None) | (Every::Times(_), None) => {
                self.next_time = 0.0;
                self.next_index = 0;
            }
            (Every::Seconds(dt), Some(t)) => {
                self.next_time = ((t + eps) / dt).floor() * dt + dt;
            }
            (Every::Times(ts), Some(t)) => {
                self.next_index = ts.partition_point(|&x| x <= t + eps);
            }
            (Every::Steps(_), _) => {}
        }
    }
}

impl Trigger {
    /// Where it stands (the next time due, the next listed time), to be
    /// saved with the state it decides for.
    pub fn position(&self) -> (f64, usize) {
        (self.next_time, self.next_index)
    }

    /// Put it back where it stood.
    pub fn set_position(&mut self, (next_time, next_index): (f64, usize)) {
        self.next_time = next_time;
        self.next_index = next_index;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The times a trigger records at over `steps` steps of `dt`.
    fn fired(every: Every, dt: f64, steps: u64) -> Vec<f64> {
        let mut tr = Trigger::new(every);
        (0..=steps)
            .map(|k| (k, k as f64 * dt))
            .filter(|&(k, t)| tr.due(t, k, 1e-9))
            .map(|(_, t)| t)
            .collect()
    }

    #[test]
    fn triggers_fire_by_steps_seconds_and_times() {
        assert_eq!(fired(Every::Steps(3), 0.1, 7).len(), 3);
        // Every 0.25 s with 0.1 s steps: the first step at or after each.
        let t = fired(Every::Seconds(0.25), 0.1, 10);
        let r: Vec<f64> = t.iter().map(|x| (x * 10.0).round() / 10.0).collect();
        assert_eq!(r, [0.0, 0.3, 0.5, 0.8, 1.0]);
        // Listed times: each once, even if two fall in one step.
        let t = fired(Every::Times(vec![0.0, 0.31, 0.32, 0.7]), 0.1, 10);
        let r: Vec<f64> = t.iter().map(|x| (x * 10.0).round() / 10.0).collect();
        assert_eq!(r, [0.0, 0.4, 0.7]);
    }

    #[test]
    fn a_rewound_trigger_fires_again_where_it_did() {
        let mut tr = Trigger::new(Every::Seconds(0.25));
        let first: Vec<u64> = (0..=10)
            .filter(|&k| tr.due(k as f64 * 0.1, k, 1e-9))
            .collect();
        // Back to t = 0.4 (recorded up to there): fires at 0.5 and 0.8 again.
        tr.seek(Some(0.4), 1e-9);
        let again: Vec<u64> = (5..=10)
            .filter(|&k| tr.due(k as f64 * 0.1, k, 1e-9))
            .collect();
        assert_eq!(
            again,
            first.into_iter().filter(|&k| k >= 5).collect::<Vec<_>>()
        );
        let mut times = Trigger::new(Every::Times(vec![0.2, 0.6]));
        assert!(times.due(0.2, 2, 1e-9));
        times.seek(Some(0.1), 1e-9);
        assert!(times.due(0.2, 2, 1e-9));
    }
}
