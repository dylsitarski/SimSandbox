//! Rewind (ADR 0064): the recorder's snapshots, their budget, and going
//! back to one, or running again from one to fill the history's gaps.

use super::{Pending, Session};
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_file::RecorderEntry;
use simsandbox_views::history::thin;
use simsandbox_views::History;
use std::collections::BTreeMap;

/// When the snapshots grow past this many, or past the recorder's memory
/// budget (ADR 0064), every other one is dropped: the whole run stays
/// reachable, at coarser resolution. Gray–Scott (128 × 128, its outputs
/// computed again on rewind, ADR 0063) takes about 0.26 MB a snapshot, so
/// the default 256 MB keeps about 1 000.
const MAX_SNAPSHOTS: usize = 10_000;

/// The session's rewind part: the recorder (ADR 0064) and the snapshots
/// it takes as the run steps, thinned to its budget, which rewinding and
/// recomputing start from. Without a recorder there are none.
pub(super) struct Rewind {
    snapshots: Vec<Snapshot>,
    /// The recorder (ADR 0064): without one, no snapshots and no rewind.
    recorder: Option<RecorderEntry>,
    steps_since_snapshot: usize,
}

impl Rewind {
    /// The file's recorder, with the model as it starts.
    pub(super) fn new(recorder: Option<RecorderEntry>, start: Snapshot) -> Self {
        Self {
            snapshots: vec![start],
            recorder,
            steps_since_snapshot: 0,
        }
    }

    pub(super) fn recorder(&self) -> Option<&RecorderEntry> {
        self.recorder.as_ref()
    }

    /// After a step: a snapshot every so many steps, as the recorder says.
    pub(super) fn stepped(&mut self, sim: &Simulator, memory_mb: f64) {
        let Some(r) = &self.recorder else {
            return;
        };
        self.steps_since_snapshot += 1;
        if self.steps_since_snapshot as u64 >= r.every() {
            self.steps_since_snapshot = 0;
            self.snapshots.push(sim.snapshot());
            self.fit(memory_mb);
        }
    }

    /// A new run from `sim` as it starts.
    pub(super) fn restart(&mut self, sim: &Simulator) {
        self.snapshots = match self.recorder {
            Some(_) => vec![sim.snapshot()],
            None => Vec::new(),
        };
        self.steps_since_snapshot = 0;
    }

    /// Thin the snapshots to the recorder's budget: its own, or its share
    /// of the app's `memory_mb`.
    pub(super) fn fit(&mut self, memory_mb: f64) {
        let Some(r) = &self.recorder else {
            return;
        };
        // Its own budget, or its share of the app's.
        let mb = r.mb.unwrap_or(memory_mb / 4.0);
        let budget = (mb * 1e6 / 8.0) as usize;
        let held = |s: &[Snapshot]| s.iter().map(Snapshot::values).sum::<usize>();
        while self.snapshots.len() > 2
            && (self.snapshots.len() > MAX_SNAPSHOTS || held(&self.snapshots) > budget)
        {
            thin(&mut self.snapshots);
        }
    }

    /// Add, change or remove the recorder. Added, it starts from `sim`
    /// now; removed, its snapshots go.
    fn set_recorder(&mut self, recorder: Option<RecorderEntry>, sim: &Simulator, memory_mb: f64) {
        match (&self.recorder, &recorder) {
            (None, Some(_)) => {
                self.snapshots = vec![sim.snapshot()];
                self.steps_since_snapshot = 0;
            }
            (_, None) => self.snapshots.clear(),
            _ => {}
        }
        self.recorder = recorder;
        self.fit(memory_mb);
    }

    /// Earliest and latest time it can rewind to (`now` without
    /// snapshots).
    fn range(&self, now: f64) -> (f64, f64) {
        let first = self.snapshots.first().map_or(now, |s| s.t);
        let last = self.snapshots.last().map_or(first, |s| s.t);
        (first, last)
    }

    /// What the snapshots hold, in bytes.
    pub(super) fn bytes(&self) -> usize {
        8 * self.snapshots.iter().map(Snapshot::values).sum::<usize>()
    }

    /// The latest snapshot at or before `t`.
    fn latest_by(&self, t: f64) -> Option<&Snapshot> {
        self.snapshots.iter().rev().find(|s| s.t <= t)
    }

    /// Drop the snapshots after the latest at or before `t` (or the
    /// first) and return it; `None` without snapshots.
    fn back_to(&mut self, t: f64) -> Option<&Snapshot> {
        if self.snapshots.is_empty() {
            return None;
        }
        let keep = self.snapshots.iter().rposition(|s| s.t <= t).unwrap_or(0);
        self.snapshots.truncate(keep + 1);
        self.steps_since_snapshot = 0;
        Some(&self.snapshots[keep])
    }
}

impl Session {
    /// The recorder, if the model has one (ADR 0064).
    pub fn recorder(&self) -> Option<&RecorderEntry> {
        self.rewind.recorder()
    }

    /// Add, change or remove the recorder (ADR 0064). Added during a run,
    /// it can rewind to now and later; removed, its snapshots go.
    pub fn set_recorder(&mut self, recorder: Option<RecorderEntry>) {
        self.rewind
            .set_recorder(recorder, &self.sim, self.memory_mb);
        self.update_probe_io();
    }

    /// Earliest and latest time the run can be rewound to.
    pub fn history_range(&self) -> (f64, f64) {
        self.rewind.range(self.time())
    }

    /// Fill the history's gaps (ADR 0062, ADR 0063: on demand, never
    /// automatically): run again from the latest snapshot before them,
    /// replaying the input as recorded, and record what views read now at
    /// the times of the samples held. The model is put back as it was, at
    /// the same time; it computes the same values bit for bit, so samples
    /// recorded before are unchanged. Probes and filters with clocks of
    /// their own are not filled. Needs the recorder's snapshots.
    pub fn recompute(&mut self) -> Result<(), String> {
        if !self.has_gaps() {
            return Ok(());
        }
        let until = self.recording.gaps_until().expect("has gaps");
        let first = self.recording.shared().read().times()[0];
        let Some(snap) = self.rewind.latest_by(first) else {
            return Err("recomputing needs the recorder's snapshots (ADR 0064)".into());
        };
        let snap = snap.clone();
        drop(self.recording.history_mut());
        let times: Vec<f64> = self.recording.shared().read().times().to_vec();
        let now = self.sim.snapshot();
        let track = self.track.position();
        let plan = self.plan.clone();
        let warnings = self.warnings.len();
        let eps = self.eps();
        self.sim.restore(&snap);
        self.track.seek(snap.t, eps);
        let mut fills: BTreeMap<usize, Pending> = BTreeMap::new();
        let mut k = times.partition_point(|&t| t < snap.t - eps);
        let mut failed = None;
        while k < times.len() && times[k] <= until + eps {
            let t = self.sim.time();
            if (times[k] - t).abs() <= eps {
                fills.insert(k, self.sample_now());
                k += 1;
                continue;
            }
            if times[k] < t - eps {
                // A sample between steps (none are, but never loop).
                k += 1;
                continue;
            }
            self.apply_due();
            if let Err(e) = self.sim.step(self.dt) {
                failed = Some(e.to_string());
                break;
            }
        }
        self.sim.restore(&now);
        self.track.set_position(track);
        self.plan = plan;
        self.warnings.truncate(warnings);
        if let Some(e) = failed {
            return Err(format!("recomputing failed: {e}"));
        }
        let old = self.recording.replace(History::new(Vec::new()));
        self.recording.clear_gaps();
        self.build_history();
        self.replay_history(&old, false, &fills);
        Ok(())
    }

    /// Rewind to the latest snapshot at or before `t`. Everything recorded
    /// after it is discarded, so running on starts a new branch.
    /// Without a recorder (ADR 0064) there is nothing to rewind to.
    pub fn rewind_to(&mut self, t: f64) {
        let Some(snap) = self.rewind.back_to(t) else {
            return;
        };
        self.sim.restore(snap);
        let t0 = snap.t;
        // Input from then on is ahead again: running on replays it.
        let eps = self.eps();
        self.track.seek(t0, eps);
        self.stop_at = self
            .file
            .run
            .t_end
            .or_else(|| crate::run::data_end(self.sim.model()))
            .filter(|&end| end > t0);
        self.recording.history_mut().truncate_after(t0);
        // Filters with clocks of their own hold what their sources
        // recorded by then (ADR 0063).
        self.probes.rewind(t0, eps);
    }
}
