//! The recording: the history the views draw from (the implicit probe,
//! ADR 0062), what it keeps of each sample by what views and filters read
//! (ADR 0063), and its gaps.

use super::catalogue::Catalogue;
use super::{param_values, Session};
use simsandbox_core::graph::Model;
use simsandbox_file::{FilterEntry, ViewEntry};
use simsandbox_views::cut::Geometry;
use simsandbox_views::{FieldShape, History, SignalRef};
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The recorded history, shared with the window (ADR 0035): the session
/// writes it, the window reads it to draw. Cloning shares it.
#[derive(Clone)]
pub struct SharedHistory(Arc<RwLock<History>>);

impl SharedHistory {
    pub(super) fn new(h: History) -> Self {
        Self(Arc::new(RwLock::new(h)))
    }

    /// Read access; waits only while the session replaces or appends.
    pub fn read(&self) -> RwLockReadGuard<'_, History> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn write(&self) -> RwLockWriteGuard<'_, History> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn try_write(&self) -> Option<RwLockWriteGuard<'_, History>> {
        match self.0.try_write() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

/// A sample that found the history being read, kept to append later.
pub(super) struct Pending {
    pub(super) index: f64,
    pub(super) values: Vec<f64>,
    pub(super) frames: Vec<Vec<f64>>,
}

/// Append `pending` to `history`, waiting for its readers if `wait`, else
/// only if none is reading. Returns whether none are left: the one way a
/// probe's or a filter's recording takes the samples that waited.
pub(super) fn append(history: &SharedHistory, pending: &mut Vec<Pending>, wait: bool) -> bool {
    if pending.is_empty() {
        return true;
    }
    let guard = if wait {
        Some(history.write())
    } else {
        history.try_write()
    };
    let Some(mut h) = guard else {
        return false;
    };
    for p in pending.drain(..) {
        let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
        h.push_sample(p.index, &p.values, &frames);
    }
    true
}

/// The session's recording part: the history, one sample per step of
/// what views read from the model and what filters read (ADR 0015,
/// ADR 0063), shared with the window, with the samples waiting while the
/// window reads it, and how far back it lacks what was wired later.
pub(super) struct Recording {
    /// Recorded signals and fields; shared with the window.
    history: SharedHistory,
    /// Samples waiting for the window to stop reading.
    pending: Vec<Pending>,
    /// What views read from the model directly (ADR 0063): the history
    /// records these and what filters read, nothing else. `None` until
    /// known (a model without saved views, before the app shows any):
    /// everything.
    view_reads: Option<Vec<String>>,
    /// What the history keeps of each sample: places in the layout.
    keep_signals: Vec<usize>,
    keep_fields: Vec<usize>,
    /// The history lacks what views read now for its samples up to this
    /// time: wired after they were recorded (ADR 0063). Filled on demand
    /// by [`Session::recompute`].
    gaps_until: Option<f64>,
    /// The kept signals of the sample being recorded, in an array kept
    /// between samples.
    kept: Vec<f64>,
}

impl Recording {
    /// Nothing recorded yet, keeping what the file's views read.
    pub(super) fn new(views: &[ViewEntry]) -> Self {
        Self {
            history: SharedHistory::new(History::new(Vec::new())),
            pending: Vec::new(),
            view_reads: (!views.is_empty()).then(|| view_reads(views)),
            keep_signals: Vec::new(),
            keep_fields: Vec::new(),
            gaps_until: None,
            kept: Vec::new(),
        }
    }

    pub(super) fn shared(&self) -> &SharedHistory {
        &self.history
    }

    /// The latest time recorded, if any.
    pub(super) fn recorded_to(&self) -> Option<f64> {
        self.history.read().times().last().copied()
    }

    /// Write access to the history, with any pending samples appended
    /// first.
    pub(super) fn history_mut(&mut self) -> RwLockWriteGuard<'_, History> {
        let mut h = self.history.write();
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        h
    }

    /// Replace the history's contents (the window's handle stays valid).
    pub(super) fn replace(&mut self, h: History) -> History {
        self.pending.clear();
        std::mem::replace(&mut *self.history.write(), h)
    }

    /// The history as recorded, pending samples included, leaving it
    /// empty to be built again.
    pub(super) fn take(&mut self) -> History {
        drop(self.history_mut());
        self.replace(History::new(Vec::new()))
    }

    /// A new run: nothing recorded, no gaps.
    pub(super) fn restart(&mut self) {
        self.history_mut().clear();
        self.gaps_until = None;
    }

    pub(super) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Append pending samples: waiting for the window if `wait`, else only
    /// if it is not reading. Returns whether none are left.
    pub(super) fn flush(&mut self, wait: bool) -> bool {
        if wait {
            drop(self.history_mut());
            return true;
        }
        let Some(mut h) = self.history.try_write() else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }

    /// Record what it keeps of a whole sample (ADR 0063). It never waits
    /// for the window: a sample that finds the history being read is
    /// appended later.
    pub(super) fn push_kept(&mut self, index: f64, values: &[f64], frames: &[&[f64]]) {
        self.kept.clear();
        self.kept
            .extend(self.keep_signals.iter().map(|&i| values[i]));
        let values = &self.kept;
        let frames: Vec<&[f64]> = self
            .keep_fields
            .iter()
            .map(|&i| frames.get(i).copied().unwrap_or(&[]))
            .collect();
        if let Some(mut h) = self.history.try_write() {
            for p in self.pending.drain(..) {
                let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
                h.push_sample(p.index, &p.values, &frames);
            }
            h.push_sample(index, values, &frames);
            return;
        }
        self.pending.push(Pending {
            index,
            values: values.clone(),
            frames: frames.iter().map(|f| f.to_vec()).collect(),
        });
    }

    /// Whether it keeps the field at place `i` in the layout.
    pub(super) fn keeps_field(&self, i: usize) -> bool {
        self.keep_fields.contains(&i)
    }

    /// Note what views read from the model; returns whether what it keeps
    /// changes, so it must be built again.
    fn set_view_reads(&mut self, reads: Vec<String>, catalogue: &Catalogue) -> bool {
        let reads = Some(model_reads(reads));
        if self.view_reads == reads {
            return false;
        }
        self.view_reads = reads;
        let (signals, fields) = self.kept(catalogue);
        signals != self.keep_signals || fields != self.keep_fields
    }

    /// The places in the layout the history keeps: everything when no
    /// views are known, else what views read from the model
    /// and what filters read, with the coordinates cuts need.
    fn kept(&self, catalogue: &Catalogue) -> (Vec<usize>, Vec<usize>) {
        let Some(reads) = &self.view_reads else {
            return (
                (0..catalogue.layout_scalars.len()).collect(),
                (0..catalogue.layout_fields.len()).collect(),
            );
        };
        let mut want: std::collections::BTreeSet<String> = reads.iter().cloned().collect();
        // Filters with clocks of their own read probes and each other.
        let derived = crate::observers::clocked(&catalogue.filter_entries);
        let model = |f: &&FilterEntry| !derived.iter().any(|d| d.name == f.name);
        for f in catalogue.filter_entries.iter().filter(model) {
            want.extend(crate::observers::filter_reads(f));
        }
        for c in &catalogue.filters.cuts {
            want.extend(
                catalogue
                    .all_coordinates
                    .iter()
                    .filter(|k| k.of == c.input)
                    .map(|k| k.name.clone()),
            );
        }
        let signals = (0..catalogue.layout_scalars.len())
            .filter(|&i| want.contains(&catalogue.layout_scalars[i]))
            .collect();
        let fields = (0..catalogue.layout_fields.len())
            .filter(|&i| want.contains(&catalogue.layout_fields[i].0))
            .collect();
        (signals, fields)
    }

    /// An empty history keeping what is wired, of the sample `catalogue`
    /// lays out (with the cuts' `labels`); `share` is its field budget.
    fn rebuild(
        &mut self,
        catalogue: &Catalogue,
        labels: Vec<(String, String)>,
        share: usize,
        model: &Model,
    ) {
        (self.keep_signals, self.keep_fields) = self.kept(catalogue);
        let kept_fields: Vec<(String, FieldShape)> = self
            .keep_fields
            .iter()
            .map(|&i| catalogue.layout_fields[i].clone())
            .collect();
        let mut h = History::with_fields(
            "t",
            self.keep_signals
                .iter()
                .map(|&i| catalogue.layout_scalars[i].clone())
                .collect(),
            kept_fields,
        );
        h.set_field_budget(share);
        catalogue.describe(&mut h, labels, model);
        self.replace(h);
    }

    /// Append the samples of `old` to the (new, empty) history, by name:
    /// what it did not record is NaN for signals and absent for fields.
    /// With `recompute`, the filters are computed again from the outputs
    /// (`params`: every parameter's value).
    ///
    /// Samples in `fills` (by index in `old`: every scalar, and the kept
    /// fields' frames) are taken from there instead. Names `old` lacks
    /// mark the samples before as gaps, unless recomputed.
    fn replay(
        &mut self,
        old: &History,
        recompute: bool,
        fills: &BTreeMap<usize, Pending>,
        catalogue: &Catalogue,
        params: impl Fn() -> Vec<f64>,
    ) {
        let signal_at: Vec<Option<usize>> = catalogue
            .layout_scalars
            .iter()
            .map(|n| old.names().iter().position(|m| m == n))
            .collect();
        let field_at: Vec<Option<usize>> = catalogue
            .layout_fields
            .iter()
            .map(|(n, _)| old.fields().iter().position(|f| &f.name == n))
            .collect();
        let outputs = catalogue.output_names.len();
        // Recorded parameters (ADR 0102) come last, after the signals: a
        // value at a past sample is had only from the model then.
        let signals = catalogue.signal_count();
        let base = catalogue.base_fields.len();
        let lacks = self
            .keep_signals
            .iter()
            .any(|&i| signal_at[i].is_none() && (!recompute || i < outputs || i >= signals))
            || self
                .keep_fields
                .iter()
                .any(|&i| field_at[i].is_none() && (!recompute || i < base));
        // The latest sample neither filled nor complete.
        let unfilled = (0..old.len()).rev().find(|k| !fills.contains_key(k));
        if let (true, Some(k)) = (lacks, unfilled) {
            let last = old.times()[k];
            self.gaps_until = Some(self.gaps_until.map_or(last, |g| g.max(last)));
        }
        for k in 0..old.len() {
            let t = old.times()[k];
            if let Some(f) = fills.get(&k) {
                let frames: Vec<&[f64]> = f.frames.iter().map(Vec::as_slice).collect();
                self.push_kept(t, &f.values, &frames);
                continue;
            }
            let mut values: Vec<f64> = signal_at
                .iter()
                .map(|i| i.map_or(f64::NAN, |i| old.value(SignalRef::Signal(i), k)))
                .collect();
            let mut frames: Vec<&[f64]> = field_at
                .iter()
                .map(|i| i.map_or(&[][..], |i| old.fields()[i].frame(k)))
                .collect();
            let cuts;
            if recompute {
                let recorded_params = values.split_off(signals);
                values.truncate(outputs);
                catalogue.append_derived(t, &mut values, &params);
                values.extend(recorded_params);
                frames.truncate(base);
                cuts = catalogue.sample_cuts(&frames);
                frames.extend(cuts.iter().map(Vec::as_slice));
            }
            self.push_kept(t, &values, &frames);
        }
    }

    /// Whether it lacks, for samples it holds, something views read now.
    fn has_gaps(&self) -> bool {
        let first = self.history.read().times().first().copied();
        matches!((self.gaps_until, first), (Some(g), Some(f)) if g >= f)
    }

    /// Up to when it lacks what views read now.
    pub(super) fn gaps_until(&self) -> Option<f64> {
        self.gaps_until
    }

    /// Its gaps are being filled.
    pub(super) fn clear_gaps(&mut self) {
        self.gaps_until = None;
    }

    pub(super) fn set_field_budget(&self, share: usize) {
        self.history.write().set_field_budget(share);
    }

    pub(super) fn bytes(&self) -> usize {
        self.history.read().bytes()
    }
}

impl Session {
    /// The recorded signals, for reading.
    pub fn signals(&self) -> RwLockReadGuard<'_, History> {
        self.recording.shared().read()
    }

    /// The recorded signals, shared, for the window to read.
    pub fn shared_history(&self) -> SharedHistory {
        self.recording.shared().clone()
    }

    /// Tell the session what the views read (their bindings as written):
    /// the history records what they read from the model directly and
    /// what filters read, from now on (ADR 0063). What it recorded of
    /// those before is kept; what it no longer records is dropped.
    pub fn set_view_reads(&mut self, reads: Vec<String>) {
        let mut probes: Vec<String> = reads
            .iter()
            .filter_map(|r| {
                simsandbox_views::history::split_source(r)
                    .0
                    .map(String::from)
            })
            .collect();
        probes.sort();
        probes.dedup();
        if self.probes.set_view_probes(probes) {
            self.update_probe_io();
        }
        // The parameters a scene binds are recorded too (ADR 0102).
        let params = self.catalogue.set_param_reads(&model_reads(reads.clone()));
        if !self.recording.set_view_reads(reads, &self.catalogue) && !params {
            return;
        }
        let old = self.recording.take();
        self.build_history();
        // The sample at now is had again from the model as it is.
        let mut fills = BTreeMap::new();
        if let Some(&last) = old.times().last() {
            if (last - self.time()).abs() <= self.eps() {
                fills.insert(old.len() - 1, self.sample_now());
            }
        }
        self.replay_history(&old, false, &fills);
    }

    /// The layout of a sample and an empty history keeping what is wired.
    pub(super) fn build_history(&mut self) {
        let labels = self.catalogue.lay_out();
        let share = self.share();
        self.recording
            .rebuild(&self.catalogue, labels, share, self.sim.model());
    }

    /// Append the samples of `old` to the new, empty history (see
    /// [`Recording::replay`]).
    pub(super) fn replay_history(
        &mut self,
        old: &History,
        recompute: bool,
        fills: &BTreeMap<usize, Pending>,
    ) {
        let (sim, flat, ids) = (&self.sim, &self.flat, &self.ids);
        self.recording
            .replay(old, recompute, fills, &self.catalogue, || {
                param_values(sim, flat, ids)
            });
    }

    /// Every scalar and every field's frame of the model now, `keep`
    /// fields only (the rest empty), as a sample at its time.
    pub(super) fn sample_now(&self) -> Pending {
        let t = self.sim.time();
        let values = self.sample(t);
        let before = self.catalogue.frames(&self.sim);
        let cuts = self.catalogue.sample_cuts(&before);
        let mut all: Vec<Vec<f64>> = before.iter().map(|f| f.to_vec()).chain(cuts).collect();
        for (i, f) in all.iter_mut().enumerate() {
            if !self.recording.keeps_field(i) {
                *f = Vec::new();
            }
        }
        Pending {
            index: t,
            values,
            frames: all,
        }
    }

    /// Whether the history lacks, for samples it holds, something views
    /// read now (wired after those samples were recorded, ADR 0063):
    /// [`Session::recompute`] fills it.
    pub fn has_gaps(&self) -> bool {
        self.recording.has_gaps()
    }
}

/// What views read, as their bindings are written (ADR 0063), a scene's
/// layers and frames included (ADR 0093).
pub fn view_reads(views: &[ViewEntry]) -> Vec<String> {
    model_reads(
        views
            .iter()
            .flat_map(|v| {
                let scene = (v.type_name == simsandbox_views::scene::SCENE)
                    .then(|| simsandbox_views::scene::reads(&v.scene.to_map()));
                v.bind
                    .values()
                    .flat_map(simsandbox_file::Binding::signals)
                    .chain(scene.into_iter().flatten())
            })
            .collect(),
    )
}

/// Of these bindings, what is read from the model directly: not `t` and
/// not a probe's recording; sorted, once each.
fn model_reads(mut reads: Vec<String>) -> Vec<String> {
    reads.retain(|r| r != "t" && !r.starts_with("probe:"));
    reads.sort();
    reads.dedup();
    reads
}

/// The 2D grids a cut filter can take: recorded grids that are not
/// themselves coordinates (in a two-parameter sweep, every result).
pub fn cut_inputs(h: &History) -> Vec<String> {
    h.fields()
        .iter()
        .filter(|f| matches!(f.shape, FieldShape::Grid { nx, ny } if nx >= 2 && ny >= 2))
        .filter(|f| !h.coordinates().iter().any(|c| c.name == f.name))
        .map(|f| f.name.clone())
        .collect()
}

/// Where the cells of grid `input` lie, from the newest sample.
pub fn cut_geometry(h: &History, input: &str) -> Geometry {
    let (nx, ny) = match h.fields().iter().find(|f| f.name == input).map(|f| f.shape) {
        Some(FieldShape::Grid { nx, ny }) => (nx, ny),
        _ => (1, 1),
    };
    let newest = |n: &str| {
        h.fields()
            .iter()
            .find(|f| f.name == n)
            .filter(|_| !h.is_empty())
            .map(|f| f.frame(h.len() - 1))
    };
    h.coordinates()
        .iter()
        .filter(|c| c.of == input && c.axis < 2)
        .fold(Geometry::cells(nx, ny), |g, c| match newest(&c.name) {
            Some(f) => g.with_coordinate(c.axis, &c.name, f),
            None => g,
        })
}

/// A new cut filter through `input`: along x at four values spread over
/// its rows, each on a row so it reads it exactly.
pub fn default_cut(h: &History, name: &str, input: &str) -> FilterEntry {
    let g = cut_geometry(h, input);
    let ny = g.ny;
    let mut rows: Vec<usize> = (0..4)
        .map(|k| (((k as f64 + 0.5) / 4.0 * ny as f64) as usize).min(ny.max(1) - 1))
        .collect();
    rows.dedup();
    let at: Vec<f64> = rows.iter().map(|&j| g.centres[1][j]).collect();
    FilterEntry {
        name: name.into(),
        type_name: "cut".into(),
        inputs: vec![input.into()],
        canvas: None,
        probe: None,
        from: Default::default(),
        when: None,
        settings: [
            ("mode".to_string(), serde_json::Value::from("along_x")),
            ("at".to_string(), serde_json::Value::from(at)),
        ]
        .into(),
    }
}
