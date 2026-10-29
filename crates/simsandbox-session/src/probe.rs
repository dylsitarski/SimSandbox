//! Probes (ADR 0062): what a run records, when, and how long it is kept.
//!
//! Every session has an implicit probe: everything its views and filters
//! can show, at every step, thinned when long, which is how recording
//! always worked. Further probes record a chosen subset on their own
//! schedule ([`Every`]) and keep it their own way ([`Keep`]), each into a
//! history of its own. A probe never changes results.

use simsandbox_views::History;

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

/// How long a probe keeps its samples.
#[derive(Debug, Clone, PartialEq)]
pub enum Keep {
    /// The whole run, thinned (every other sample dropped) when it holds
    /// more than `mb` megabytes, or by the fields' size if not given: the
    /// default.
    Thin { mb: Option<f64> },
    /// Only the newest `samples`, forgetting the oldest.
    Last { samples: usize },
    /// Every sample, never thinned.
    All,
}

impl Default for Keep {
    fn default() -> Self {
        Keep::Thin { mb: None }
    }
}

impl Keep {
    /// Apply to a history.
    pub(crate) fn apply(&self, h: &mut History) {
        match *self {
            Keep::Thin { mb: None } => {}
            Keep::Thin { mb: Some(mb) } => {
                let per = h.values_per_sample().max(1) as f64 * 8.0;
                h.thin_past((mb * 1e6 / per) as usize);
            }
            Keep::Last { samples } => h.keep_last(samples),
            Keep::All => h.keep_all(),
        }
    }
}

/// A probe: its name, what it records (signal and field names as views
/// bind them), when and how long.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeSpec {
    pub name: String,
    pub record: Vec<String>,
    pub every: Every,
    pub keep: Keep,
}

impl ProbeSpec {
    /// Check the settings; an error says what is wrong.
    pub fn check(&self) -> Result<(), String> {
        let ok = match &self.every {
            Every::Steps(n) => *n >= 1,
            Every::Seconds(dt) => dt.is_finite() && *dt > 0.0,
            Every::Times(ts) => ts.iter().all(|t| t.is_finite() && *t >= 0.0),
        };
        if !ok {
            return Err(format!(
                "probe {}: 'every' is a whole number of steps of at least 1, a positive time, or times of 0 or more",
                self.name
            ));
        }
        let keep_ok = match self.keep {
            Keep::Thin { mb } => mb.is_none_or(|mb| mb.is_finite() && mb > 0.0),
            Keep::Last { samples } => samples >= 1,
            Keep::All => true,
        };
        if !keep_ok {
            return Err(format!(
                "probe {}: 'keep' is a positive size in MB or at least 1 sample",
                self.name
            ));
        }
        if self.record.is_empty() {
            return Err(format!("probe {}: it records nothing", self.name));
        }
        Ok(())
    }
}

/// Decides, step by step, when a probe records.
#[derive(Debug, Clone)]
pub(crate) struct Trigger {
    every: Every,
    /// For `Seconds`: the next multiple due. For `Times`: the next index.
    next_time: f64,
    next_index: usize,
}

impl Trigger {
    pub(crate) fn new(every: Every) -> Self {
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
    pub(crate) fn due(&mut self, t: f64, step: u64, eps: f64) -> bool {
        match &self.every {
            Every::Steps(n) => step.is_multiple_of(*n),
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
    pub(crate) fn seek(&mut self, recorded_up_to: Option<f64>, eps: f64) {
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

    #[test]
    fn probe_settings_are_checked() {
        let p = |every, keep| ProbeSpec {
            name: "p".into(),
            record: vec!["a.x".into()],
            every,
            keep,
        };
        assert!(p(Every::Steps(1), Keep::default()).check().is_ok());
        assert!(p(Every::Steps(0), Keep::All).check().is_err());
        assert!(p(Every::Seconds(-1.0), Keep::All).check().is_err());
        assert!(p(Every::Times(vec![f64::NAN]), Keep::All).check().is_err());
        assert!(p(Every::Steps(1), Keep::Last { samples: 0 })
            .check()
            .is_err());
        assert!(p(Every::Steps(1), Keep::Thin { mb: Some(0.0) })
            .check()
            .is_err());
        let mut none = p(Every::Steps(1), Keep::All);
        none.record.clear();
        assert!(none.check().is_err());
    }
}
