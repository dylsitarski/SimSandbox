//! One open model: the running simulator, its snapshots for rewind, the
//! recorded signals the views draw from, and its parameters.

use simsandbox_core::block::{MethodLabels, TimeSemantics};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::{FilterEntry, FusedGroup, ModelFile, ParamPlan, ViewEntry};
use std::collections::BTreeMap;

mod catalogue;
mod inputs;
mod probes;
mod recording;
mod rewind;

use catalogue::Catalogue;
pub use inputs::InputEvent;
pub(crate) use inputs::{set_key, Track};
use probes::Probes;
pub use recording::{cut_geometry, cut_inputs, default_cut, view_reads, SharedHistory};
use recording::{Pending, Recording};
use rewind::Rewind;

/// The app's memory budget by default, in MB (ADR 0062 step 5): shared by
/// snapshots, the history and probes, a quarter each by default.
pub const DEFAULT_MEMORY_MB: f64 = 1024.0;

/// What a session's recordings hold against its memory budget (ADR 0062
/// step 5), in bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryUse {
    pub budget: usize,
    pub held: usize,
    /// Each part: `snapshots`, `history`, `probe NAME`, `filter NAME`.
    pub parts: Vec<(String, usize)>,
}

impl MemoryUse {
    /// The part holding most, if any holds anything.
    pub fn largest(&self) -> Option<&(String, usize)> {
        self.parts.iter().filter(|p| p.1 > 0).max_by_key(|p| p.1)
    }

    pub fn over(&self) -> bool {
        self.held > self.budget
    }
}

/// One parameter as the panel shows it.
#[derive(Clone, Debug)]
pub struct ParamRow {
    /// `"block.param"`, the key used in `controls.params`; for a declared
    /// parameter (ADR 0043) `"name"` (the model's) or `"instance.name"`.
    pub key: String,
    /// The Block it belongs to; `None` for a declared parameter.
    pub block: Option<BlockId>,
    /// The flat block name (`"pasture/grass"`), or for a declared
    /// parameter the instance (`""` for the model's own).
    pub owner: String,
    pub name: String,
    /// Set when a formula computes it: it is shown, not edited.
    pub formula: Option<String>,
    pub unit: String,
    pub description: String,
    pub suggested: [f64; 2],
    pub initial_condition: bool,
    pub value: f64,
    /// The signal driving it (ADR 0075), `block.port`: then its value is
    /// that signal's, not set by hand.
    pub driven: Option<String>,
}

/// The parameters of `model` driven by wires, as `block.param` to the
/// `block.port` driving each (ADR 0075).
pub fn drives(model: &simsandbox_core::graph::Model) -> Vec<(String, String)> {
    let name = |id: BlockId| {
        model
            .block(id)
            .map(|b| b.spec().name.clone())
            .unwrap_or_default()
    };
    model
        .drives()
        .iter()
        .map(|c| {
            (
                format!("{}.{}", name(c.to.block), c.to.port),
                format!("{}.{}", name(c.from.block), c.from.port),
            )
        })
        .collect()
}

pub struct Session {
    /// Display name, e.g. the file name.
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded; the source for saving.
    pub file: ModelFile,
    /// The file flattened: one entry per Block, subsystems expanded
    /// (ADR 0043).
    pub flat: ModelFile,
    /// Declared parameters and the formulas they feed.
    plan: ParamPlan,
    pub ids: BTreeMap<String, BlockId>,
    pub warnings: Vec<String>,
    /// Older forms the file was written in, converted on reading
    /// (ADR 0044).
    pub conversions: Vec<String>,
    /// Blocks solved together as one system (ADR 0023).
    pub groups: Vec<FusedGroup>,
    pub dt: f64,
    /// The history the views draw from, and what it keeps (ADR 0015,
    /// ADR 0063).
    recording: Recording,
    /// Probes besides the implicit one, which is `history` (ADR 0062),
    /// and the filters on them.
    probes: Probes,
    /// What a step yields, the filters, and what views can bind.
    catalogue: Catalogue,
    /// Where the app pauses (the file's `run.t_end`); cleared once reached so
    /// Run continues past it (ADR 0013).
    stop_at: Option<f64>,
    sim: Simulator,
    /// The app's memory budget in MB (ADR 0062 step 5).
    memory_mb: f64,
    /// Built without running its studies ([`crate::study::without_running_studies`]),
    /// or (evaluated later) changed since last evaluated: [`Session::evaluate`]
    /// brings it up to date.
    unevaluated: bool,
    /// Nothing in it runs in time (ADR 0072): evaluated, not stepped.
    timeless: bool,
    /// Only maps, and nothing reads time (ADR 0083): it iterates.
    iterates: bool,
    /// An iterating model has rounds still to run: since it opened or last
    /// changed, it has neither held still nor reached its last round.
    rounds_pending: bool,
    /// Its last round changed no state.
    held_still: bool,
    /// Changes to a model with nothing in time leave it unevaluated, for
    /// the caller to [`Session::evaluate`] (the app's worker, ADR 0072),
    /// instead of evaluating it at once.
    evaluate_later: bool,
    /// The recorder and its snapshots (ADR 0064).
    rewind: Rewind,
    /// The schedule and the recorded live input (ADR 0058).
    track: Track,
    /// The file as it was when the current run started (its parameters at
    /// t = 0), which a run file is made with (ADR 0059).
    start_file: ModelFile,
    /// Digests of the files blocks read, for the model hash (ADR 0063).
    data: BTreeMap<String, String>,
}

impl Session {
    /// Load a model file. Errors are formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        Self::load_in(title, text, registry, None)
    }

    /// Load a model file saved in `folder`, whose blocks read the files
    /// they name from there (ADR 0063).
    pub fn load_in(
        title: &str,
        text: &str,
        registry: &Registry,
        folder: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        // A model that uses model files runs as one simulator with clocks
        // (ADR 0066 step 4b).
        let loaded = if simsandbox_file::compose::is_composition(text) {
            simsandbox_file::compose::load_merged(text, registry, folder)
        } else {
            simsandbox_file::load_in(text, registry, folder)
        }
        .map_err(|e| e.to_string())?;
        let catalogue = Catalogue::of(&loaded)?;
        // Where a run only data drives ends (ADR 0067 step 4).
        let data_end = crate::run::data_end(&loaded.model);
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
        let schedule: Vec<InputEvent> = loaded
            .file
            .run
            .inputs
            .iter()
            .map(|c| InputEvent {
                t: c.t,
                key: c.set.clone(),
                value: c.to,
            })
            .collect();
        let start_file = loaded.file.clone();
        let data = loaded.data;
        let mut warnings = loaded.warnings;
        warnings.extend(loop_notes(&sim));
        let entries = loaded.file.controls.filters.clone();
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end.or(data_end),
            conversions: loaded.conversions,
            file: loaded.file,
            flat: loaded.flat,
            plan: loaded.params,
            ids: loaded.ids,
            groups: loaded.groups,
            warnings,
            recording: Recording::new(&start_file.controls.views),
            probes: Probes::default(),
            catalogue,
            rewind: Rewind::new(start_file.controls.recorder.clone(), sim.snapshot()),
            memory_mb: DEFAULT_MEMORY_MB,
            sim,
            track: Track::new(schedule, Vec::new()),
            start_file,
            data,
            unevaluated: false,
            timeless: false,
            iterates: false,
            rounds_pending: false,
            held_still: false,
            evaluate_later: false,
        };
        session.timeless = session.nothing_in_time();
        session.iterates = !session.timeless && session.only_rounds();
        session.rounds_pending = session.iterates;
        session.unevaluated = !crate::study::studies_run()
            && session
                .catalogue
                .block_types
                .iter()
                .any(|t| t == crate::study::STUDY);
        session.check_schedule()?;
        let (filters, problems) = session.compile_filters(&entries);
        session.catalogue.set_filters(filters, &entries);
        for p in problems {
            session
                .warnings
                .push(format!("controls.filters.{p}; ignored"));
        }
        // No probes yet: they are loaded next, and the filters that read
        // them with them.
        session.build_history();
        session.load_probes();
        // Scheduled changes at t = 0 set where the run starts.
        session.restart().map_err(|e| e.to_string())?;
        Ok(session)
    }

    /// How close two times must be to count as the same step boundary.
    fn eps(&self) -> f64 {
        1e-9 * self.dt.max(self.time().abs())
    }

    /// The file as it was when the current run started.
    pub fn start_file(&self) -> &ModelFile {
        &self.start_file
    }

    /// Append pending samples: waiting for the window if `wait`, else only
    /// if it is not reading. Returns whether none are left.
    pub fn flush(&mut self, wait: bool) -> bool {
        let probes_done = self.probes.flush(wait);
        if !self.recording.has_pending() {
            return probes_done;
        }
        self.recording.flush(wait)
    }

    /// Record one sample: scalar values, the frames of the fields before
    /// the cuts, then the cuts' frames. It never waits for the window: a
    /// sample that finds the history being read is appended later.
    fn push(&mut self, index: f64, values: &[f64], before: &[&[f64]], cuts: &[Vec<f64>]) {
        let frames: Vec<&[f64]> = before
            .iter()
            .copied()
            .chain(cuts.iter().map(Vec::as_slice))
            .collect();
        self.push_probes(index, values, &frames);
        self.recording.push_kept(index, values, &frames);
    }

    pub fn time(&self) -> f64 {
        self.sim.time()
    }

    pub fn labels(&self) -> Option<MethodLabels> {
        self.sim.labels()
    }

    /// Advance one communication step and record it. A step whose study
    /// was given up (ADR 0071 step 2, [`crate::run::given_up`]) leaves the
    /// model as it was before it.
    pub fn step(&mut self) -> Result<(), SimError> {
        self.apply_due();
        let studied = self
            .catalogue
            .block_types
            .iter()
            .any(|t| t == simsandbox_file::STUDY);
        let before = studied.then(|| self.sim.snapshot());
        let states_before = self.iterates.then(|| self.state_bits());
        if let Err(e) = self.sim.step(self.dt) {
            if let Some(b) = before.filter(|_| crate::run::given_up(&e)) {
                self.sim.restore(&b);
            }
            return Err(e);
        }
        // A round that changes no state is the last that changes anything
        // (ADR 0083).
        if let Some(b) = states_before {
            if b == self.state_bits() {
                self.held_still = true;
                self.rounds_pending = false;
            }
        }
        self.record();
        self.rewind.stepped(&self.sim, self.memory_mb);
        Ok(())
    }

    /// Step until the time reaches `t` (within rounding), recording every
    /// step: how headless interfaces run a model (ADR 0039).
    /// How long the next step is in simulated time: `dt`, or with several
    /// clocks the time to the next instant of any (ADR 0066 step 4b).
    pub fn step_dt(&self) -> f64 {
        self.sim
            .next_instant()
            .map_or(self.dt, |next| (next - self.time()).max(0.0))
    }

    pub fn run_until(&mut self, t: f64) -> Result<(), SimError> {
        while self.time() < t - 1e-9 * self.dt.max(t.abs()) {
            self.step()?;
        }
        Ok(())
    }

    /// The run has reached the file's `t_end`. Returns true once, then the
    /// run may continue past it.
    pub fn take_end_reached(&mut self) -> bool {
        let reached = self
            .stop_at
            .is_some_and(|end| self.time() >= end - 1e-9 * self.dt.max(end.abs()));
        if reached {
            self.stop_at = None;
        }
        reached
    }

    /// The kernel model being run.
    pub fn sim_model(&self) -> &simsandbox_core::graph::Model {
        self.sim.model()
    }

    /// Each activated use (ADR 0088), by path: whether it is held now
    /// and how often it has started again since the run began.
    pub fn activations(&self) -> Vec<(String, bool, u64)> {
        self.sim
            .activations()
            .into_iter()
            .map(|(name, held, starts)| (name.to_string(), held, starts))
            .collect()
    }

    /// Every parameter: the declared ones (the model's, then each
    /// instance's), then every block's in flat block order.
    pub fn params(&self) -> Vec<ParamRow> {
        let model = self.sim.model();
        let mut rows = Vec::new();
        for d in &self.plan.declared {
            let (owner, name) = d.key.rsplit_once('.').unwrap_or(("", &d.key));
            let formula = match (&d.formula, d.editable) {
                (Some(f), _) => Some(f.text.clone()),
                // Set inside another subsystem: fixed by its definition.
                (None, false) => Some(short(d.value)),
                (None, true) => None,
            };
            rows.push(ParamRow {
                key: d.key.clone(),
                block: None,
                owner: owner.into(),
                name: name.into(),
                formula,
                unit: d.unit.clone(),
                description: d.description.clone(),
                suggested: d.range.unwrap_or_else(|| suggested_range(d.value)),
                initial_condition: false,
                value: d.value,
                driven: None,
            });
        }
        for entry in &self.flat.blocks {
            let id = self.ids[&entry.name];
            let (Ok(block), Ok(values)) = (model.block(id), model.params(id)) else {
                continue;
            };
            for (p, &value) in block.spec().params.iter().zip(values) {
                rows.push(ParamRow {
                    key: format!("{}.{}", entry.name, p.name),
                    block: Some(id),
                    owner: entry.name.clone(),
                    formula: self
                        .plan
                        .formula_of(&entry.name, &p.name)
                        .map(|f| f.text.clone()),
                    name: p.name.clone(),
                    unit: if p.unit == Unit::DIMENSIONLESS {
                        String::new()
                    } else {
                        p.unit.to_string()
                    },
                    description: p.description.clone(),
                    suggested: [p.suggested_min, p.suggested_max],
                    initial_condition: p.initial_condition,
                    value,
                    driven: None,
                });
            }
        }
        // What wires drive (ADR 0075); a declared parameter of a use is
        // driven when the parameters set to it are.
        let driven = drives(model);
        for r in &mut rows {
            r.driven = driven
                .iter()
                .find(|(to, _)| *to == r.key)
                .map(|(_, from)| from.clone());
        }
        let blocks: Vec<(String, Option<String>, Option<String>)> = rows
            .iter()
            .filter(|r| r.block.is_some())
            .map(|r| (r.owner.clone(), r.formula.clone(), r.driven.clone()))
            .collect();
        for r in rows
            .iter_mut()
            .filter(|r| r.block.is_none() && !r.owner.is_empty())
        {
            let inside = format!("{}/", r.owner);
            r.driven = blocks
                .iter()
                .find(|(owner, formula, d)| {
                    owner.starts_with(&inside) && formula.as_deref() == Some(&r.name) && d.is_some()
                })
                .and_then(|(_, _, d)| d.clone());
        }
        rows
    }

    fn after_param_change(&mut self) -> Result<(), String> {
        if self.timeless && self.evaluate_later {
            self.unevaluated = true;
            return Ok(());
        }
        // An iteration starts again from round 0 (ADR 0083).
        if self.iterates && self.evaluate_later {
            return self.restart().map_err(|e| e.to_string());
        }
        if self.time() == 0.0 && self.sim.step_count() == 0 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Whether it was built without running its studies, or changed since
    /// last evaluated with evaluation left for later: [`Session::evaluate`]
    /// brings it up to date.
    pub fn unevaluated(&self) -> bool {
        self.unevaluated
    }

    /// Nothing in it runs in time (ADR 0072): no block has states or
    /// tables or reads recorded data, there are no clocks and no scheduled
    /// inputs. The app evaluates such a model and never steps it.
    pub fn timeless(&self) -> bool {
        self.timeless
    }

    /// Only maps, with a last round, and nothing reads time or happens at a
    /// time (ADR 0083): a `t_end`; no continuous states, tables, clocks,
    /// data or scheduled inputs; and every block says it never reads `t`. The app iterates such a model
    /// as fast as it can instead of pacing it.
    pub fn iterates(&self) -> bool {
        self.iterates
    }

    /// An iterating model still has rounds to run: it has neither held
    /// still nor reached its last round since it opened or last changed.
    pub fn rounds_pending(&self) -> bool {
        self.rounds_pending
    }

    /// Stop iterating (its last round was reached, or a step failed).
    pub fn stop_rounds(&mut self) {
        self.rounds_pending = false;
    }

    /// Its last round changed no state, so no later round changes anything.
    pub fn held_still(&self) -> bool {
        self.held_still
    }

    /// Rounds run since it started.
    pub fn round(&self) -> u64 {
        self.sim.step_count()
    }

    /// Every block's state and tables (rows, ids and values), by bits.
    fn state_bits(&self) -> Vec<u64> {
        let mut bits = Vec::new();
        for &id in self.ids.values() {
            bits.extend(self.sim.state(id).iter().map(|v| v.to_bits()));
            for t in self.sim.tables(id) {
                bits.push(t.len() as u64);
                bits.extend(t.ids());
                for c in 0..t.columns() {
                    bits.extend(t.column(c).iter().map(|v| v.to_bits()));
                }
            }
        }
        bits
    }

    fn only_rounds(&self) -> bool {
        let model = self.sim.model();
        // A last round: an open-ended map (the logistic map) is watched as
        // it goes, paced as any model.
        self.file.run.t_end.is_some()
            && model.clocks().0.is_empty()
            && self.track.schedule().is_empty()
            && self.ids.values().all(|&id| {
                model.block(id).is_ok_and(|b| {
                    // Tables only of blocks that say they never read
                    // time: rows that change by rules each round. A block
                    // integrated in time runs in time, whatever holds its
                    // states (rows of tables, ADR 0085 step 3).
                    b.reads_time() == Some(false)
                        && b.instants().is_none()
                        && b.spec().time != TimeSemantics::Continuous
                        && (b.spec().states.is_empty()
                            || b.spec().time == TimeSemantics::DiscreteEvent)
                })
            })
    }

    fn nothing_in_time(&self) -> bool {
        let model = self.sim.model();
        model.clocks().0.is_empty()
            && self.track.schedule().is_empty()
            && self.ids.values().all(|&id| {
                model.block(id).is_ok_and(|b| {
                    b.spec().states.is_empty()
                        && b.spec().tables.is_empty()
                        && b.instants().is_none()
                })
            })
    }

    /// Leave changes to a model with nothing in time unevaluated, for the
    /// caller to [`Session::evaluate`] where it can show progress and give
    /// a study up (the app's worker, ADR 0072).
    pub fn evaluate_later(&mut self) {
        self.evaluate_later = true;
    }

    /// Bring the model up to date (a restart: the history starts over with
    /// its values): run the studies of a model built without running them,
    /// or evaluate a model with nothing in time after a change. A study
    /// given up ([`crate::run::given_up`]) leaves it unevaluated, to run
    /// again.
    pub fn evaluate(&mut self) -> Result<(), SimError> {
        let r = self.restart();
        // A study that failed is reported once, not run again and again.
        if r.as_ref().is_err_and(|e| !crate::run::given_up(e)) {
            self.unevaluated = false;
        }
        r
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        // A new run: recorded input is dropped; what the schedule sets
        // starts from the model's own values and is set again.
        self.track.clear_recorded();
        for (key, value) in self.track.base().to_vec() {
            if let Err(e) = self.set_value(&key, value) {
                self.warnings.push(e);
            }
        }
        let r = self.restart_keeping_recorded();
        self.rounds_pending = self.iterates;
        self.held_still = false;
        // Any restart outside `without_running_studies` runs them.
        if r.is_ok() && crate::study::studies_run() {
            self.unevaluated = false;
        }
        r
    }

    /// Restart at t = 0 with the events at t = 0 applied first, so they
    /// set where the run starts, and the rest still ahead.
    fn restart_keeping_recorded(&mut self) -> Result<(), SimError> {
        self.track.seek(0.0, 0.0);
        // Back to t = 0 so the events there are due.
        self.sim.restart(0.0)?;
        // The model as it starts, before the events: a run's hash and the
        // saved model agree, and replaying applies the events again.
        if let Err(e) = self.capture_start() {
            self.warnings.push(e);
        }
        self.apply_due();
        self.sim.restart(0.0)?;
        self.rewind.restart(&self.sim);
        self.stop_at = self
            .file
            .run
            .t_end
            .or_else(|| crate::run::data_end(self.sim.model()));
        self.recording.restart();
        self.probes.restart();
        self.record();
        Ok(())
    }

    /// Start from the current state (ADR 0042): every block's state now
    /// becomes where the run starts, then the run restarts at t = 0 from
    /// it. Blocks whose initial-condition parameters can say it exactly
    /// (`x0` for a state `x`) get those parameters; the others get a
    /// `start` entry in the file with their state and tables. Returns a
    /// note per block.
    pub fn start_from_now(&mut self) -> Result<Vec<String>, String> {
        let t = self.time();
        let mut notes = Vec::new();
        for i in 0..self.flat.blocks.len() {
            let name = self.flat.blocks[i].name.clone();
            let id = self.ids[&name];
            let now = self.sim.current(id);
            if now.state.is_empty() && now.tables.is_empty() {
                continue;
            }
            let model = self.sim.model();
            let block = model.block(id).map_err(|e| e.to_string())?;
            let spec = block.spec();
            let params = model.params(id).map_err(|e| e.to_string())?;
            let by_params = now
                .tables
                .is_empty()
                .then(|| block.initial_params(&now.state, params))
                .flatten();
            if let Some(p) = by_params {
                let changed: Vec<(String, f64)> = spec
                    .params
                    .iter()
                    .zip(p.iter().zip(params))
                    .filter(|(_, (new, old))| new != old)
                    .map(|(q, (new, _))| (q.name.clone(), *new))
                    .collect();
                let shown: Vec<String> = changed
                    .iter()
                    .map(|(n, v)| format!("{n} = {}", crate::session::short(*v)))
                    .collect();
                for (n, v) in changed {
                    self.sim.set_param(id, &n, v).map_err(|e| e.to_string())?;
                }
                self.sim.set_start(id, None).map_err(|e| e.to_string())?;
                self.flat.blocks[i].start = None;
                notes.push(format!("{name}: {}", shown.join(", ")));
            } else {
                let entry = simsandbox_file::StartEntry::from_start(&now, spec);
                let rows: usize = now.tables.iter().map(Vec::len).sum();
                let what = if rows > 0 {
                    format!("{} values and {rows} rows", now.state.len())
                } else {
                    format!("{} values", now.state.len())
                };
                self.sim
                    .set_start(id, Some(now))
                    .map_err(|e| e.to_string())?;
                self.flat.blocks[i].start = Some(entry);
                notes.push(format!("{name}: start state saved ({what})"));
            }
        }
        if notes.is_empty() {
            return Err("no block has a state to start from".into());
        }
        self.sync_file()?;
        self.restart().map_err(|e| e.to_string())?;
        notes.insert(0, format!("Starting from the state at t = {}", short(t)));
        Ok(notes)
    }

    /// Where a block sits on the node canvas (ADR 0042); layout only, so
    /// the run goes on.
    pub fn set_position(&mut self, block: &str, at: [f64; 2]) -> Result<(), String> {
        self.set_position_in(&crate::diagram::Scope::Model, block, at)
    }

    /// Where a block, or a subsystem's port node, sits on the canvas of one
    /// level (ADR 0043); layout only.
    pub fn set_position_in(
        &mut self,
        scope: &crate::diagram::Scope,
        block: &str,
        at: [f64; 2],
    ) -> Result<(), String> {
        let mut file = std::mem::take(&mut self.file);
        let edit = crate::diagram::Edit::Move {
            block: block.into(),
            at,
        };
        let result = crate::diagram::apply_in(&mut file, scope, &Registry::new(), &edit);
        self.file = file;
        result.map(|_| ())
    }

    /// Whether any block starts from a saved state instead of its
    /// parameters.
    pub fn has_start_states(&self) -> bool {
        self.flat.blocks.iter().any(|b| b.start.is_some())
    }

    /// Bring the file as written up to date with what runs.
    fn sync_file(&mut self) -> Result<(), String> {
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        Ok(())
    }

    /// Start every block from its parameters again, and restart.
    pub fn forget_start_states(&mut self) -> Result<(), String> {
        for b in &mut self.flat.blocks {
            if b.start.take().is_some() {
                self.sim
                    .set_start(self.ids[&b.name], None)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.sync_file()?;
        self.restart().map_err(|e| e.to_string())
    }

    /// The model file with the current parameters, step, default speed and
    /// views (ADR 0012, ADR 0013, ADR 0015), ready to write.
    pub fn save_json(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
    ) -> Result<String, String> {
        self.save_json_with(speed, views, None)
    }

    /// As [`Self::save_json`], with the app's arrangement of its panes
    /// (ADR 0053) if given; otherwise the file's is kept.
    pub fn save_json_with(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
        layout: Option<Option<simsandbox_file::layout::Layout>>,
    ) -> Result<String, String> {
        if let Some(l) = layout {
            self.file.controls.layout = l;
        }
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        self.file.run.dt = self.dt;
        self.file.controls.speed = Some(speed);
        if let Some(v) = views {
            self.file.controls.views = v;
        }
        self.file.controls.filters = self.catalogue.filter_entries.clone();
        self.file.controls.recorder = self.rewind.recorder().cloned();
        self.file.controls.probes = self.probe_entries();
        Ok(self.file.to_json())
    }

    /// Keep the file as it is now, with the current parameters, as the
    /// file this run starts from.
    fn capture_start(&mut self) -> Result<(), String> {
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        self.file.run.dt = self.dt;
        self.start_file = self.file.clone();
        Ok(())
    }

    /// Whether dragged sliders apply while dragged (ADR 0037): every method
    /// is real-time.
    pub fn real_time(&self) -> bool {
        self.labels()
            .is_some_and(|l| l.speed == simsandbox_core::block::SpeedClass::RealTime)
    }

    /// A quarter of the memory budget, in bytes: the default share of
    /// snapshots, the history and each probe (ADR 0062 step 5).
    fn share(&self) -> usize {
        (self.memory_mb * 1e6 / 4.0) as usize
    }

    /// The app's memory budget in MB (ADR 0062 step 5).
    pub fn memory_budget(&self) -> f64 {
        self.memory_mb
    }

    /// Set the memory budget in MB (ADR 0062 step 5): snapshots (unless
    /// the recorder sets its own), the history, each probe that keeps
    /// what it records by default, and each filter thin past a quarter of
    /// it. What is held now thins at the next sample.
    pub fn set_memory_budget(&mut self, mb: f64) {
        if !(mb.is_finite() && mb > 0.0) {
            return;
        }
        self.memory_mb = mb;
        let share = self.share();
        self.recording.set_field_budget(share);
        self.probes.set_field_budget(share);
        self.rewind.fit(self.memory_mb);
    }

    /// What the recordings hold against the memory budget (ADR 0062 step
    /// 5): snapshots, the history, each probe and filter.
    pub fn memory_use(&self) -> MemoryUse {
        let mut parts = vec![
            ("snapshots".to_string(), self.rewind.bytes()),
            ("history".to_string(), self.recording.bytes()),
        ];
        self.probes.memory_parts(&mut parts);
        MemoryUse {
            budget: (self.memory_mb * 1e6) as usize,
            held: parts.iter().map(|p| p.1).sum(),
            parts,
        }
    }

    fn record(&mut self) {
        let t = self.sim.time();
        let values = self.sample(t);
        let owned: Vec<Vec<f64>> = self
            .catalogue
            .frames(&self.sim)
            .into_iter()
            .map(<[f64]>::to_vec)
            .collect();
        let frames: Vec<&[f64]> = owned.iter().map(Vec::as_slice).collect();
        let cuts = self.catalogue.sample_cuts(&frames);
        self.push(t, &values, &frames, &cuts);
    }

    /// Replace the filters. All must be usable, or nothing changes and the
    /// problems are returned. The recorded history is kept and the filters
    /// are recomputed for it, using current parameter values.
    pub fn set_filters(&mut self, entries: Vec<FilterEntry>) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut names = Vec::new();
        for e in &entries {
            if names.contains(&e.name) || self.catalogue.output_names.contains(&e.name) {
                problems.push(format!("{}: the name is already used", e.name));
            }
            names.push(e.name.clone());
        }
        let (filters, more) = self.compile_filters(&entries);
        problems.extend(more);
        if !problems.is_empty() {
            return Err(problems);
        }
        self.catalogue.set_filters(filters, &entries);
        // Replay the recorded outputs and fields through the new filters;
        // probes start afresh from now.
        let old = self.recording.take();
        self.build_history();
        self.replay_history(&old, true, &BTreeMap::new());
        self.reset_probes();
        Ok(())
    }
}

/// One note per algebraic loop (ADR 0040), saying how it is solved, and
/// one per loop of equation blocks stepped apart ([`apart_notes`]).
pub fn loop_notes(sim: &Simulator) -> Vec<String> {
    let method = sim.model().loop_method().name();
    let mut notes: Vec<String> = sim
        .loops()
        .into_iter()
        .map(|l| algebraic_note(&l, method))
        .collect();
    notes.extend(apart_notes(sim.model()));
    notes
}

/// The note of [`loop_notes`] for one algebraic loop, by its outputs.
pub(crate) fn algebraic_note(outputs: &[String], method: &str) -> String {
    format!(
        "algebraic loop {}: solved at every step by '{method}' (run.loops)",
        outputs.join(" -> ")
    )
}

/// One note per loop of equation blocks that could be solved together
/// and are not (ADR 0095): blocks wired in a circle, continuous or
/// stateless, at least one with states, not all in one fused group
/// (different methods, `fusion: off`, different clocks, an activated
/// part's edge). Each then reads the others' outputs as they were at the
/// start of the step, an error of the order of the step whatever the
/// methods' own accuracy, which their labels do not show.
pub fn apart_notes(model: &simsandbox_core::graph::Model) -> Vec<String> {
    apart_loops(model).iter().map(|l| apart_note(l)).collect()
}

/// The note of [`apart_notes`] for one loop, by its blocks' names.
pub(crate) fn apart_note(names: &[String]) -> String {
    format!(
        "{} depend on each other and are stepped apart: each reads the others' outputs \
         from the start of the step, an error of the order of run.dt that the accuracy \
         label does not show. To solve them together give them one method and leave \
         fusion on, or list them in 'groups'",
        names.join(", ")
    )
}

/// The loops [`apart_notes`] reports, each as its blocks' names.
pub(crate) fn apart_loops(model: &simsandbox_core::graph::Model) -> Vec<Vec<String>> {
    let n = model.len();
    let mut next = vec![Vec::new(); n];
    let mut prev = vec![Vec::new(); n];
    for c in model.connections() {
        let (a, b) = (c.from.block.0, c.to.block.0);
        if a != b && model.is_fusable(c.from.block) && model.is_fusable(c.to.block) {
            next[a].push(b);
            prev[b].push(a);
        }
    }
    // Strongly connected parts (Kosaraju, without recursion): blocks in
    // the order they are finished going forward, then gathered going
    // backward from the last finished.
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((v, k)) = stack.last_mut() {
            if let Some(&w) = next[*v].get(*k) {
                *k += 1;
                if !seen[w] {
                    seen[w] = true;
                    stack.push((w, 0));
                }
            } else {
                order.push(*v);
                stack.pop();
            }
        }
    }
    let mut part = vec![usize::MAX; n];
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for &start in order.iter().rev() {
        if part[start] != usize::MAX {
            continue;
        }
        let k = parts.len();
        part[start] = k;
        let mut members = vec![start];
        let mut at = 0;
        while at < members.len() {
            for &w in &prev[members[at]] {
                if part[w] == usize::MAX {
                    part[w] = k;
                    members.push(w);
                }
            }
            at += 1;
        }
        parts.push(members);
    }
    let id = simsandbox_core::graph::BlockId;
    let mut notes = Vec::new();
    for mut members in parts {
        let together = model
            .group_of(id(members[0]))
            .is_some_and(|g| members.iter().all(|&m| model.group_of(id(m)) == Some(g)));
        if members.len() < 2 || together || members.iter().all(|&m| model.is_algebraic(id(m))) {
            continue;
        }
        members.sort_unstable();
        let names: Vec<String> = members
            .iter()
            .filter_map(|&m| model.block(id(m)).ok().map(|b| b.spec().name.clone()))
            .collect();
        notes.push(names);
    }
    notes
}

/// Every parameter's value, in [`Session::params`] order.
fn param_values(sim: &Simulator, flat: &ModelFile, ids: &BTreeMap<String, BlockId>) -> Vec<f64> {
    let model = sim.model();
    flat.blocks
        .iter()
        .flat_map(|e| model.params(ids[&e.name]).unwrap_or(&[]))
        .copied()
        .collect()
}

/// Every parameter as `"block.param"`, in the order [`Session::params`]
/// lists them (file block order, then each block's parameters).
fn param_keys(file: &ModelFile, ids: &BTreeMap<String, BlockId>, model: &Model) -> Vec<String> {
    let mut keys = Vec::new();
    for entry in &file.blocks {
        if let Ok(b) = model.block(ids[&entry.name]) {
            for p in &b.spec().params {
                keys.push(format!("{}.{}", entry.name, p.name));
            }
        }
    }
    keys
}

/// A slider range for a declared parameter that gives none: from zero to
/// twice its value (or ±1 around zero).
fn suggested_range(v: f64) -> [f64; 2] {
    if v == 0.0 {
        [-1.0, 1.0]
    } else if v > 0.0 {
        [0.0, 2.0 * v]
    } else {
        [2.0 * v, 0.0]
    }
}

/// A number in its shortest form, at most 6 significant digits.
pub(crate) fn short(v: f64) -> String {
    let r = format!("{v:.6e}").parse::<f64>().unwrap_or(v);
    let s = format!("{r}");
    if s.len() > 12 {
        format!("{r:.6e}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests;
