//! Inputs (ADR 0058): the schedule, the live input recorded in a run, and
//! replaying it after a rewind or from a run file (ADR 0059).

use super::{short, ParamRow, Session};
use simsandbox_core::graph::BlockId;
use simsandbox_core::sim::Simulator;
use simsandbox_file::runs::RunFile;
use simsandbox_file::ParamPlan;
use std::collections::BTreeMap;

/// Set a value by its key on a running model: a declared parameter (the
/// formulas it feeds follow, ADR 0043) or `"block.param"`, unless a formula
/// sets it.
pub(crate) fn set_key(
    sim: &mut Simulator,
    plan: &mut ParamPlan,
    ids: &BTreeMap<String, BlockId>,
    key: &str,
    value: f64,
) -> Result<(), String> {
    if plan.declared(key).is_some() {
        for (block, param, v) in plan.set(key, value)? {
            // A driven parameter takes its signal instead (ADR 0075).
            if sim.model().is_driven(ids[&block], &param) {
                continue;
            }
            sim.set_param(ids[&block], &param, v)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let (block, name) = key
        .rsplit_once('.')
        .ok_or_else(|| format!("'{key}' is not 'block.param'"))?;
    let id = *ids
        .get(block)
        .ok_or_else(|| format!("no block '{block}'"))?;
    if let Some(f) = plan.formula_of(block, name) {
        return Err(format!("{key} is computed from '{}'", f.text));
    }
    sim.set_param(id, name, value).map_err(|e| e.to_string())
}

/// One change of a value during a run (ADR 0058): `key` is a parameter
/// key (`"block.param"` or a declared parameter), applied at the first step
/// boundary at or after `t`.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub t: f64,
    pub key: String,
    pub value: f64,
}

/// The input track, the session's inputs part: the schedule (kept by
/// restarts) and the recorded live input (dropped by restarts, kept by
/// rewinds and replayed), each with how much of it has been applied.
/// Headless runs (`run`, `compose`) step through one too.
#[derive(Debug, Clone, Default)]
pub(crate) struct Track {
    schedule: Vec<InputEvent>,
    /// The model's own value of each key the schedule sets, which every
    /// run starts from.
    base: Vec<(String, f64)>,
    recorded: Vec<InputEvent>,
    next_schedule: usize,
    next_recorded: usize,
}

impl Track {
    /// A schedule and recorded input, each put in time order.
    pub(crate) fn new(mut schedule: Vec<InputEvent>, mut recorded: Vec<InputEvent>) -> Self {
        schedule.sort_by(|a, b| a.t.total_cmp(&b.t));
        recorded.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self {
            schedule,
            recorded,
            ..Self::default()
        }
    }

    /// The next event due by `now`: the earlier of the two lists, the
    /// schedule first at equal times (as it was applied live).
    pub(crate) fn take_due(&mut self, now: f64, eps: f64) -> Option<InputEvent> {
        let s = self
            .schedule
            .get(self.next_schedule)
            .filter(|e| e.t <= now + eps);
        let r = self
            .recorded
            .get(self.next_recorded)
            .filter(|e| e.t <= now + eps);
        match (s, r) {
            (Some(a), Some(b)) if b.t < a.t - eps => {
                self.next_recorded += 1;
                Some(b.clone())
            }
            (Some(a), _) => {
                self.next_schedule += 1;
                Some(a.clone())
            }
            (None, Some(b)) => {
                self.next_recorded += 1;
                Some(b.clone())
            }
            (None, None) => None,
        }
    }

    /// Start applying again from time `t`: events at `t` or later are
    /// still ahead.
    pub(super) fn seek(&mut self, t: f64, eps: f64) {
        self.next_schedule = self.schedule.partition_point(|e| e.t < t - eps);
        self.next_recorded = self.recorded.partition_point(|e| e.t < t - eps);
    }

    /// How much of each list has been applied, to go back to with
    /// [`Track::set_position`].
    pub(super) fn position(&self) -> (usize, usize) {
        (self.next_schedule, self.next_recorded)
    }

    pub(super) fn set_position(&mut self, (schedule, recorded): (usize, usize)) {
        (self.next_schedule, self.next_recorded) = (schedule, recorded);
    }

    pub(super) fn schedule(&self) -> &[InputEvent] {
        &self.schedule
    }

    pub(super) fn recorded(&self) -> &[InputEvent] {
        &self.recorded
    }

    /// The schedule's keys and values, for checking.
    fn scheduled_values(&self) -> Vec<(String, f64)> {
        self.schedule
            .iter()
            .map(|e| (e.key.clone(), e.value))
            .collect()
    }

    /// Note the model's own value (in `rows`) of each key the schedule
    /// sets, which every run starts from.
    fn set_base(&mut self, rows: &[ParamRow]) {
        let mut base: Vec<(String, f64)> = Vec::new();
        for e in &self.schedule {
            if !base.iter().any(|(k, _)| *k == e.key) {
                if let Some(r) = rows.iter().find(|r| r.key == e.key) {
                    base.push((e.key.clone(), r.value));
                }
            }
        }
        self.base = base;
    }

    /// The values a run starts from, before its events.
    pub(super) fn base(&self) -> &[(String, f64)] {
        &self.base
    }

    /// A new run: recorded input is dropped.
    pub(super) fn clear_recorded(&mut self) {
        self.recorded.clear();
    }

    /// Record a change made live at `t`: input recorded ahead of now is
    /// dropped (a new branch). Before the run starts (`at_start`), a value
    /// the schedule also sets becomes the model's own.
    fn record_live(&mut self, t: f64, key: &str, value: f64, at_start: bool) {
        if at_start {
            for (k, v) in &mut self.base {
                if k == key {
                    *v = value;
                }
            }
        }
        let ahead = self.next_recorded;
        self.recorded.truncate(ahead);
        self.recorded.push(InputEvent {
            t,
            key: key.to_string(),
            value,
        });
        self.next_recorded = self.recorded.len();
    }
}

impl Session {
    /// Check the schedule as loaded and note the values its keys start
    /// from.
    pub(super) fn check_schedule(&mut self) -> Result<(), String> {
        self.check_events(&self.track.scheduled_values(), "run.inputs")?;
        let rows = self.params();
        self.track.set_base(&rows);
        Ok(())
    }

    /// Events must name parameters a person sets (not ones a formula
    /// computes); values outside a parameter's suggested range are noted.
    fn check_events(&mut self, events: &[(String, f64)], what: &str) -> Result<(), String> {
        let rows = self.params();
        for (k, (key, value)) in events.iter().enumerate() {
            let Some(row) = rows.iter().find(|r| r.key == *key) else {
                return Err(format!("{what}[{k}]: no parameter '{key}'"));
            };
            if row.formula.is_some() {
                return Err(format!(
                    "{what}[{k}]: {key} is set by a formula, so it cannot be changed"
                ));
            }
            let [lo, hi] = row.suggested;
            if *value < lo || *value > hi {
                self.warnings.push(format!(
                    "{what}[{k}]: {key} = {value} is outside its suggested range {lo} to {hi}"
                ));
            }
        }
        Ok(())
    }

    /// Apply every event due by now (ADR 0058). An event that no longer
    /// applies is noted, not fatal.
    pub(super) fn apply_due(&mut self) {
        let (now, eps) = (self.time(), self.eps());
        while let Some(e) = self.track.take_due(now, eps) {
            if let Err(err) = self.set_value(&e.key, e.value) {
                self.warnings
                    .push(format!("input at t = {}: {err}", short(e.t)));
            }
        }
    }

    /// The scheduled changes in effect (ADR 0058).
    pub fn schedule(&self) -> &[InputEvent] {
        self.track.schedule()
    }

    /// The live input recorded in this run, including any ahead of now
    /// after a rewind (ADR 0058).
    pub fn recorded(&self) -> &[InputEvent] {
        self.track.recorded()
    }

    /// Recorded input lies ahead of now: running on replays it.
    pub fn replaying(&self) -> bool {
        self.track.next_recorded < self.track.recorded.len()
    }

    /// Restart and replay `track` as recorded live input (a run file's,
    /// ADR 0059). Errors name the event that does not apply.
    pub fn replay(&mut self, mut track: Vec<InputEvent>) -> Result<(), String> {
        let events: Vec<(String, f64)> = track.iter().map(|e| (e.key.clone(), e.value)).collect();
        self.check_events(&events, "track")?;
        track.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.restart().map_err(|e| e.to_string())?;
        self.track.recorded = track;
        self.track.next_recorded = 0;
        // Recorded input at t = 0 sets where the run starts, as live.
        if self
            .track
            .recorded
            .first()
            .is_some_and(|e| e.t <= self.eps())
        {
            self.restart_keeping_recorded().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// This run as a run file (ADR 0059): the model hash of the file it
    /// started from, its seed and settings, the schedule and the recorded
    /// input. `model` is the model file's path relative to where the run
    /// file goes.
    pub fn run_file(&self, model: &str, name: &str, note: &str, created: &str) -> RunFile {
        RunFile {
            format: simsandbox_file::runs::RUN_FORMAT,
            model: model.into(),
            model_hash: simsandbox_file::runs::model_hash_with(&self.start_file, &self.data),
            made_with: simsandbox_file::runs::MADE_WITH.into(),
            name: name.into(),
            note: note.into(),
            created: created.into(),
            seed: Some(self.file.run.seed.unwrap_or(1)),
            run: Some(simsandbox_file::runs::RunOverrides {
                dt: Some(self.dt),
                t_end: self.file.run.t_end,
            }),
            inputs: self
                .track
                .schedule
                .iter()
                .map(|e| simsandbox_file::ScheduledChange {
                    t: e.t,
                    set: e.key.clone(),
                    to: e.value,
                })
                .collect(),
            track: self
                .track
                .recorded
                .iter()
                .map(|e| (e.t, e.key.clone(), e.value))
                .collect(),
        }
    }

    /// Change a parameter by its key (`"block.param"`, or a declared
    /// parameter's), recomputing what formulas set from it (ADR 0043).
    /// The change is live input: it is recorded in the run's input track
    /// (ADR 0058), after any events due now, and drops recorded input
    /// ahead (running on from a rewind with new input is a new branch).
    pub fn set_param_key(&mut self, key: &str, value: f64) -> Result<(), String> {
        self.apply_due();
        self.set_value(key, value)?;
        // Before the run starts, a value the schedule also sets is the
        // model's own: the run starts from it.
        let at_start = self.time() == 0.0 && self.sim.step_count() == 0;
        let t = self.time();
        self.track.record_live(t, key, value, at_start);
        self.after_param_change()
    }

    /// Live input from a device (keys, a view's pointer, ADR 0058): as
    /// [`Session::set_param_key`], but nothing happens (and nothing is
    /// recorded) when an input block's value is already `value`, so a held
    /// pointer or key sends only changes. Returns whether it changed.
    pub fn set_input(&mut self, key: &str, value: f64) -> Result<bool, String> {
        self.apply_due();
        if let Some((block, name)) = key.rsplit_once('.') {
            if let Some(&id) = self.ids.get(block) {
                if self.sim.param(id, name).ok() == Some(value) {
                    return Ok(false);
                }
            }
        }
        self.set_param_key(key, value).map(|()| true)
    }

    /// Set a value by key, not recorded.
    pub(super) fn set_value(&mut self, key: &str, value: f64) -> Result<(), String> {
        set_key(&mut self.sim, &mut self.plan, &self.ids, key, value)
    }

    /// Change a parameter while running, as live input (recorded, like
    /// [`Session::set_param_key`]). Initial conditions apply on
    /// [`Session::restart`], or at once if the run has not started.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<(), String> {
        let flat = self
            .ids
            .iter()
            .find(|(_, &id)| id == block)
            .map(|(n, _)| n.clone())
            .ok_or("no such block")?;
        self.set_param_key(&format!("{flat}.{name}"), value)
    }
}
