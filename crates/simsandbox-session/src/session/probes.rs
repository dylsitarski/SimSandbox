//! Probes besides the implicit one (ADR 0062), and the filters that run
//! on their samples with clocks of their own (ADR 0063): each keeps its
//! own recording, and may write its file as it records.

use super::catalogue::Catalogue;
use super::{param_keys, param_values, Pending, Session, SharedHistory};
use crate::filters::Filters;
use crate::probe::{ProbeSpec, Trigger};
use simsandbox_file::{FilterEntry, ProbeEntry};
use simsandbox_views::{Coordinate, FieldShape, History};
use std::path::{Path, PathBuf};

/// A probe besides the implicit one (ADR 0062): a subset of what the
/// session records, on its own schedule, kept its own way.
pub(super) struct Extra {
    pub(super) spec: ProbeSpec,
    pub(super) trigger: Trigger,
    pub(super) history: SharedHistory,
    /// Where its signals and fields are in a whole sample.
    signals: Vec<usize>,
    fields: Vec<usize>,
    pub(super) pending: Vec<Pending>,
    /// Whether it keeps samples in memory: only while something reads
    /// them (views, filters, a file written when asked, a headless run),
    /// so a probe feeding a block or writing as it records costs none.
    in_memory: bool,
    /// Its file, written as it records (ADR 0063).
    pub(super) stream: Option<crate::probe::Stream>,
}

impl Extra {
    /// Its signals' names and its fields, as recorded.
    fn layout(&self) -> (Vec<String>, Vec<(String, FieldShape)>) {
        let h = self.history.read();
        (
            h.names().to_vec(),
            h.fields()
                .iter()
                .map(|f| (f.name.clone(), f.shape))
                .collect(),
        )
    }

    /// Append any pending samples, waiting for readers if `wait`.
    /// Returns whether none are left.
    pub(super) fn flush(&mut self, wait: bool) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let guard = if wait {
            Some(self.history.write())
        } else {
            self.history.try_write()
        };
        let Some(mut h) = guard else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }
}

/// Where an input of a filter with a clock of its own comes from
/// (ADR 0063).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Src {
    /// The model, at every step.
    Model,
    /// A probe, by place.
    Probe(usize),
    /// Another such filter, by place (always an earlier one).
    Derived(usize),
}

/// One input: its name, source, and where its value is: a place in the
/// whole sample (model, probe), or among that filter's outputs.
#[derive(Clone, Debug)]
struct Input {
    name: String,
    src: Src,
    at: usize,
    field: bool,
}

/// A filter with an input from a probe (ADR 0063): it runs when its
/// sources record (all of them, or any with `when: any`) and keeps its
/// own recording, bound by its outputs' names.
struct Derived {
    entry: FilterEntry,
    /// Its probes as set, so it is kept while they do not change.
    key: Vec<ProbeSpec>,
    filters: Filters,
    inputs: Vec<Input>,
    sources: Vec<Src>,
    /// Each input's latest value (one number for a signal).
    held: Vec<Option<Vec<f64>>>,
    history: SharedHistory,
    pending: Vec<Pending>,
    /// What it gave at the current step, for filters reading it.
    now: Option<(Vec<f64>, Vec<Vec<f64>>)>,
}

impl Derived {
    /// Its outputs' names: signals, then fields.
    fn outputs(&self) -> (Vec<String>, Vec<String>) {
        (
            self.filters.exprs.iter().map(|f| f.name.clone()).collect(),
            self.filters
                .cuts
                .iter()
                .flat_map(|c| c.fields().into_iter().map(|(n, _)| n))
                .collect(),
        )
    }

    fn flush(&mut self, wait: bool) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let guard = if wait {
            Some(self.history.write())
        } else {
            self.history.try_write()
        };
        let Some(mut h) = guard else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }
}

/// The session's probes part: the probes besides the implicit one (the
/// history), each on its own trigger with its own recording and file,
/// and the filters with an input from a probe, each with its own clock
/// and recording; with what decides which keep samples in memory and
/// where their files go.
#[derive(Default)]
pub(super) struct Probes {
    /// The probes, as set.
    list: Vec<Extra>,
    /// Filters with an input from a probe (ADR 0063), in dependency
    /// order.
    derived: Vec<Derived>,
    /// Their outputs' names, as views bind them.
    derived_names: Vec<String>,
    /// Probes whose recordings views read.
    view_probes: Vec<String>,
    /// Keep every probe's samples (a headless run).
    record_all: bool,
    /// Where probes write their files as they record.
    dir: Option<PathBuf>,
}

impl Probes {
    /// No probes and no filters on them: a sample is only the history's.
    pub(super) fn is_empty(&self) -> bool {
        self.list.is_empty() && self.derived.is_empty()
    }

    pub(super) fn derived_names(&self) -> &[String] {
        &self.derived_names
    }

    /// Append pending samples, waiting for readers if `wait` (and then
    /// flushing the files too). Returns whether none are left.
    pub(super) fn flush(&mut self, wait: bool) -> bool {
        let mut done = true;
        for p in &mut self.list {
            done &= p.flush(wait);
            if wait {
                if let Some(w) = &mut p.stream {
                    let _ = w.flush();
                }
            }
        }
        for d in &mut self.derived {
            done &= d.flush(wait);
        }
        done
    }

    /// Record a whole sample (`values`, `frames`, at step `step`) in each
    /// probe due, then run the filters on them; `params` gives every
    /// parameter's value, asked for only when there are such filters.
    /// A file that fails to write is noted in `warnings` and closed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        index: f64,
        values: &[f64],
        frames: &[&[f64]],
        step: u64,
        eps: f64,
        coordinates: &[Coordinate],
        params: impl FnOnce() -> Vec<f64>,
        warnings: &mut Vec<String>,
    ) {
        let mut fired = Vec::with_capacity(self.list.len());
        for p in &mut self.list {
            let due = p.trigger.due(index, step, eps);
            fired.push(due);
            if !due {
                continue;
            }
            let own: Vec<f64> = p.signals.iter().map(|&i| values[i]).collect();
            if p.stream.is_some() {
                let fr: Vec<&[f64]> = p.fields.iter().map(|&i| frames[i]).collect();
                let (_, fields) = p.layout();
                let w = p.stream.as_mut().expect("checked");
                if let Err(e) = w.write(index, &own, &fr, &fields) {
                    let note = format!("probe {}: writing its file failed: {e}", p.spec.name);
                    p.stream = None;
                    if !warnings.contains(&note) {
                        warnings.push(note);
                    }
                }
            }
            if !p.in_memory {
                continue;
            }
            p.pending.push(Pending {
                index,
                values: own,
                frames: p.fields.iter().map(|&i| frames[i].to_vec()).collect(),
            });
            p.flush(false);
        }
        self.run_derived(index, values, frames, &fired, coordinates, params);
    }

    /// Which probes keep samples in memory, and which write their files
    /// as they record (ADR 0063): call after anything those depend on
    /// changes. `indexed`: the files keep step indices (a recorder).
    pub(super) fn update_io(&mut self, indexed: bool, warnings: &mut Vec<String>) {
        let read_by_filters: Vec<usize> = self
            .derived
            .iter()
            .flat_map(|d| d.sources.iter())
            .filter_map(|s| match s {
                Src::Probe(i) => Some(*i),
                _ => None,
            })
            .collect();
        let mut notes = Vec::new();
        for (i, p) in self.list.iter_mut().enumerate() {
            let s = &p.spec;
            let in_memory = self.record_all
                || self.view_probes.contains(&s.name)
                || read_by_filters.contains(&i)
                || (s.to.is_some() && !s.stream);
            if in_memory != p.in_memory {
                // Started or stopped: from now on, nothing before.
                p.pending.clear();
                p.history.write().clear();
                p.in_memory = in_memory;
            }
            let wanted = match (&s.to, &self.dir) {
                (Some(to), Some(dir)) if s.stream => Some(dir.join(to)),
                _ => None,
            };
            let current = p.stream.as_ref().map(|w| w.paths[0].clone());
            if wanted != current {
                p.stream = None;
                if let (Some(_), Some(dir), Some(to)) = (&wanted, &self.dir, &s.to) {
                    let (names, fields) = p.layout();
                    match crate::probe::Stream::new(dir, to, &names, &fields, indexed) {
                        Ok((w, n)) => {
                            notes.extend(n.into_iter().map(|n| format!("probe {}: {n}", s.name)));
                            p.stream = Some(w);
                        }
                        Err(e) => notes.push(format!("probe {}: cannot write {to}: {e}", s.name)),
                    }
                }
            }
            if let Some(w) = &mut p.stream {
                w.set_indexed(indexed);
            }
            if s.stream && s.to.is_some() && self.dir.is_none() {
                notes.push(format!(
                    "probe {}: it writes its file once the model is saved",
                    s.name
                ));
            }
        }
        for n in notes {
            if !warnings.contains(&n) {
                warnings.push(n);
            }
        }
    }

    /// Note which probes views read; returns whether that changed.
    pub(super) fn set_view_probes(&mut self, probes: Vec<String>) -> bool {
        if probes == self.view_probes {
            return false;
        }
        self.view_probes = probes;
        true
    }

    /// Set where files go; returns whether that changed.
    fn set_dir(&mut self, dir: Option<PathBuf>) -> bool {
        if self.dir == dir {
            return false;
        }
        self.dir = dir;
        true
    }

    /// The probes as they are, without what `recorded` says a step no
    /// longer yields: notes in `warnings`, and a probe left recording
    /// nothing is dropped. The probes are cleared, to be set again.
    fn take_still_recorded(
        &mut self,
        recorded: impl Fn(&str) -> bool,
        warnings: &mut Vec<String>,
    ) -> Vec<ProbeSpec> {
        let mut specs: Vec<ProbeSpec> = Vec::new();
        for p in &self.list {
            let mut spec = p.spec.clone();
            let had = !spec.record.is_empty();
            spec.record.retain(|r| {
                let ok = recorded(r);
                if !ok {
                    warnings.push(format!("probe {}: '{r}' is no longer recorded", spec.name));
                }
                ok
            });
            if had && spec.record.is_empty() {
                warnings.push(format!(
                    "probe {}: removed, it records nothing now",
                    spec.name
                ));
            } else {
                specs.push(spec);
            }
        }
        self.list.clear();
        specs
    }

    /// Replace the probes, without the filters that read them (see
    /// [`Session::set_probes`]). Probes take from every sample a step
    /// yields (`catalogue`, ADR 0063), not from what the history keeps;
    /// `share` is each one's field budget, `recorded_to` the history's
    /// latest time.
    pub(super) fn install(
        &mut self,
        specs: Vec<ProbeSpec>,
        catalogue: &Catalogue,
        share: usize,
        recorded_to: Option<f64>,
        eps: f64,
    ) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        let (scalars, field_list) = (&catalogue.layout_scalars, &catalogue.layout_fields);
        let mut built = Vec::new();
        for spec in &specs {
            if let Err(e) = spec.check() {
                problems.push(e);
                continue;
            }
            if names.contains(&spec.name.as_str()) {
                problems.push(format!("probe {}: the name is used twice", spec.name));
            }
            names.push(&spec.name);
            let mut signals = Vec::new();
            let mut fields = Vec::new();
            for r in &spec.record {
                if let Some(i) = scalars.iter().position(|n| n == r) {
                    signals.push(i);
                } else if let Some(i) = field_list.iter().position(|(f, _)| f == r) {
                    fields.push(i);
                } else {
                    problems.push(format!("probe {}: the model has no '{r}'", spec.name));
                }
            }
            let probe_fields: Vec<(String, FieldShape)> =
                fields.iter().map(|&i| field_list[i].clone()).collect();
            let probe_signals: Vec<String> = signals.iter().map(|&i| scalars[i].clone()).collect();
            let mut ph = History::with_fields("t", probe_signals, probe_fields);
            // Coordinates of what it records, when it records them too.
            let has = |n: &str| spec.record.iter().any(|r| r == n);
            ph.set_coordinates(
                catalogue
                    .all_coordinates
                    .iter()
                    .filter(|c| has(&c.name) && has(&c.of))
                    .cloned()
                    .collect(),
            );
            ph.set_field_budget(share);
            spec.keep.apply(&mut ph);
            built.push(Extra {
                spec: spec.clone(),
                trigger: Trigger::new(spec.every.clone()),
                history: SharedHistory::new(ph),
                signals,
                fields,
                pending: Vec::new(),
                in_memory: true,
                stream: None,
            });
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        // A probe whose recording is unchanged keeps what it recorded; a
        // new one records from now on.
        for b in &mut built {
            let same = |o: &&mut Extra| {
                o.spec.name == b.spec.name
                    && o.spec.record == b.spec.record
                    && o.spec.every == b.spec.every
                    && o.spec.keep == b.spec.keep
            };
            if let Some(old) = self.list.iter_mut().find(same) {
                std::mem::swap(b, old);
                b.spec = old.spec.clone();
            } else {
                b.trigger.seek(recorded_to, eps);
            }
        }
        self.list = built;
        Ok(())
    }

    /// The filters with an input from a probe (ADR 0063), in an order
    /// where each comes after those it reads. One whose recording and
    /// probes did not change keeps what it recorded. `params`: every
    /// parameter's key; problems are noted in `warnings`, once each.
    pub(super) fn build_derived(
        &mut self,
        catalogue: &Catalogue,
        params: &[String],
        share: usize,
        recorded_to: Option<f64>,
        warnings: &mut Vec<String>,
    ) {
        let mut old: Vec<Derived> = std::mem::take(&mut self.derived);
        self.derived_names.clear();
        let mut waiting: Vec<FilterEntry> = crate::observers::clocked(&catalogue.filter_entries);
        let mut notes = Vec::new();
        while !waiting.is_empty() {
            let before = waiting.len();
            let mut k = 0;
            while k < waiting.len() {
                match self.derived_inputs(&waiting[k], &waiting, catalogue, params) {
                    Ok(None) => k += 1,
                    Ok(Some(inputs)) => {
                        let e = waiting.remove(k);
                        let built = self.compile_derived(
                            e,
                            inputs,
                            params,
                            &mut old,
                            (catalogue, share, recorded_to),
                        );
                        match built {
                            Ok(d) => self.derived.push(d),
                            Err(p) => notes.push(p),
                        }
                    }
                    Err(p) => {
                        notes.push(p);
                        waiting.remove(k);
                    }
                }
            }
            if waiting.len() == before {
                for e in waiting.drain(..) {
                    notes.push(format!(
                        "filter {}: it reads itself through other filters; ignored",
                        e.name
                    ));
                }
            }
        }
        for d in &self.derived {
            let (s, f) = d.outputs();
            self.derived_names.extend(s.into_iter().chain(f));
        }
        // Filters are set again on every probe edit: say each once.
        for n in notes {
            if !warnings.contains(&n) {
                warnings.push(n);
            }
        }
    }

    /// The inputs of filter `e`, each with its source; `Ok(None)` while a
    /// filter it reads (among `waiting`) is not built yet.
    fn derived_inputs(
        &self,
        e: &FilterEntry,
        waiting: &[FilterEntry],
        catalogue: &Catalogue,
        params: &[String],
    ) -> Result<Option<Vec<Input>>, String> {
        let reads: Vec<String> = crate::observers::filter_reads(e)
            .into_iter()
            .filter(|r| r != "t" && !params.contains(r))
            .collect();
        for p in e.probe.iter().chain(e.from.values()) {
            if !self.list.iter().any(|x| x.spec.name == *p) {
                return Err(format!(
                    "filter {}: there is no probe '{p}' to run on",
                    e.name
                ));
            }
        }
        let mut inputs = Vec::new();
        for r in reads {
            if waiting
                .iter()
                .any(|w| w.name != e.name && w.outputs().contains(&r))
            {
                return Ok(None);
            }
            if let Some(j) = self.derived.iter().position(|d| {
                let (s, f) = d.outputs();
                s.contains(&r) || f.contains(&r)
            }) {
                let (s, f) = self.derived[j].outputs();
                let (at, field) = match s.iter().position(|n| *n == r) {
                    Some(i) => (i, false),
                    None => (f.iter().position(|n| *n == r).unwrap_or(0), true),
                };
                inputs.push(Input {
                    name: r,
                    src: Src::Derived(j),
                    at,
                    field,
                });
                continue;
            }
            let src = match e.source_of(&r) {
                None => Src::Model,
                Some(p) => {
                    let i = self
                        .list
                        .iter()
                        .position(|x| x.spec.name == p)
                        .ok_or_else(|| {
                            format!("filter {}: there is no probe '{p}' to run on", e.name)
                        })?;
                    if !self.list[i].spec.record.contains(&r) {
                        return Err(format!(
                            "filter {}: probe {p} does not record '{r}'; ignored",
                            e.name
                        ));
                    }
                    Src::Probe(i)
                }
            };
            let (at, field) = match catalogue.layout_scalars.iter().position(|n| *n == r) {
                Some(i) => (i, false),
                None => match catalogue.layout_fields.iter().position(|(n, _)| *n == r) {
                    Some(i) => (i, true),
                    None => {
                        return Err(format!(
                            "filter {}: the model has no '{r}'; ignored",
                            e.name
                        ))
                    }
                },
            };
            inputs.push(Input {
                name: r,
                src,
                at,
                field,
            });
        }
        Ok(Some(inputs))
    }

    /// Compile filter `e` on its inputs, keeping the recording of an
    /// equal one in `old`. `on`: the sample's layout, the field budget
    /// and the history's latest time.
    fn compile_derived(
        &self,
        e: FilterEntry,
        inputs: Vec<Input>,
        params: &[String],
        old: &mut Vec<Derived>,
        on: (&Catalogue, usize, Option<f64>),
    ) -> Result<Derived, String> {
        let (catalogue, share, recorded_to) = on;
        let scalars: Vec<String> = inputs
            .iter()
            .filter(|i| !i.field)
            .map(|i| i.name.clone())
            .collect();
        let shape_of = |i: &Input| match i.src {
            Src::Derived(j) => {
                let h = self.derived[j].history.read();
                h.fields()
                    .iter()
                    .find(|f| f.name == i.name)
                    .map(|f| f.shape)
            }
            _ => Some(catalogue.layout_fields[i.at].1),
        };
        let grids: Vec<(String, FieldShape)> = inputs
            .iter()
            .filter(|i| i.field)
            .filter_map(|i| shape_of(i).map(|s| (i.name.clone(), s)))
            .collect();
        let (filters, problems) =
            Filters::compile(std::slice::from_ref(&e), &scalars, params, &grids);
        if let Some(p) = problems.into_iter().next() {
            return Err(format!("filter {p}; ignored"));
        }
        let mut sources: Vec<Src> = Vec::new();
        for i in &inputs {
            if !sources.contains(&i.src) {
                sources.push(i.src);
            }
        }
        let key: Vec<ProbeSpec> = sources
            .iter()
            .filter_map(|s| match s {
                Src::Probe(i) => Some(self.list[*i].spec.clone()),
                _ => None,
            })
            .collect();
        let held = vec![None; inputs.len()];
        if let Some(k) = old.iter().position(|d| d.entry == e && d.key == key) {
            let mut d = old.remove(k);
            d.filters = filters;
            d.inputs = inputs;
            d.sources = sources;
            return Ok(d);
        }
        let mut d = Derived {
            entry: e,
            key,
            filters,
            inputs,
            sources,
            held,
            history: SharedHistory::new(History::new(Vec::new())),
            pending: Vec::new(),
            now: None,
        };
        let (names, _) = d.outputs();
        let fields: Vec<(String, FieldShape)> =
            d.filters.cuts.iter().flat_map(|c| c.fields()).collect();
        let mut h = History::with_fields("t", names, fields);
        h.set_field_budget(share);
        let mut coordinates = Vec::new();
        for c in &d.filters.cuts {
            coordinates.extend(c.lines().into_iter().map(|l| Coordinate {
                name: c.coordinate(),
                of: l,
                axis: 0,
            }));
        }
        h.set_coordinates(coordinates);
        d.history = SharedHistory::new(h);
        // Mid-run, it starts from what its sources recorded so far.
        if recorded_to.is_some() {
            self.reseed(&mut d);
        }
        Ok(d)
    }

    /// Take a filter's held inputs again from its sources' recordings, as
    /// they stand (after a rewind, or when it is new).
    fn reseed(&self, d: &mut Derived) {
        for (k, i) in d.inputs.iter().enumerate() {
            let h = match i.src {
                Src::Model => None,
                Src::Probe(p) => Some(self.list[p].history.clone()),
                Src::Derived(j) => Some(self.derived[j].history.clone()),
            };
            d.held[k] = h.and_then(|h| {
                let h = h.read();
                let last = h.len().checked_sub(1)?;
                if i.field {
                    let f = h.fields().iter().find(|f| f.name == i.name)?;
                    Some(f.frame(last).to_vec()).filter(|v| !v.is_empty())
                } else {
                    let s = h.resolve(&i.name)?;
                    Some(vec![h.value(s, last)])
                }
            });
        }
        d.now = None;
    }

    /// Run the filters with clocks of their own whose sources recorded at
    /// this sample (`fired`, by probe).
    fn run_derived(
        &mut self,
        t: f64,
        values: &[f64],
        frames: &[&[f64]],
        fired: &[bool],
        coordinates: &[Coordinate],
        params: impl FnOnce() -> Vec<f64>,
    ) {
        if self.derived.is_empty() {
            return;
        }
        let params = params();
        let mut produced = vec![false; self.derived.len()];
        for j in 0..self.derived.len() {
            let (before, rest) = self.derived.split_at_mut(j);
            let d = &mut rest[0];
            d.now = None;
            let did = |s: &Src| match *s {
                Src::Model => true,
                Src::Probe(i) => fired.get(i).copied().unwrap_or(false),
                Src::Derived(k) => produced[k],
            };
            let go = if d.entry.on_any() {
                d.sources.iter().any(did)
            } else {
                d.sources.iter().all(did)
            };
            if !go {
                continue;
            }
            for (k, i) in d.inputs.iter().enumerate() {
                if !did(&i.src) {
                    continue;
                }
                d.held[k] = Some(match (i.src, i.field) {
                    (Src::Derived(m), field) => {
                        let (v, f) = before[m].now.as_ref().expect("it produced");
                        if field {
                            f[i.at].clone()
                        } else {
                            vec![v[i.at]]
                        }
                    }
                    (_, true) => frames[i.at].to_vec(),
                    (_, false) => vec![values[i.at]],
                });
            }
            // Until every input has a value (`any`), it gives NaN.
            let ready = d.held.iter().all(Option::is_some);
            let mut vals: Vec<f64> = d
                .inputs
                .iter()
                .zip(&d.held)
                .filter(|(i, _)| !i.field)
                .map(|(_, h)| h.as_ref().map_or(f64::NAN, |v| v[0]))
                .collect();
            let base = vals.len();
            derive(&d.filters.exprs, t, &mut vals, &params);
            let mut out: Vec<f64> = vals.split_off(base);
            if !ready {
                out.iter_mut().for_each(|v| *v = f64::NAN);
            }
            let field_names: Vec<String> = d
                .inputs
                .iter()
                .filter(|i| i.field)
                .map(|i| i.name.clone())
                .collect();
            let field_frames: Vec<Vec<f64>> = d
                .inputs
                .iter()
                .zip(&d.held)
                .filter(|(i, _)| i.field)
                .map(|(_, h)| h.clone().unwrap_or_default())
                .collect();
            let cut_frames = derived_cuts(&d.filters, &field_names, &field_frames, coordinates);
            d.pending.push(Pending {
                index: t,
                values: out.clone(),
                frames: cut_frames.clone(),
            });
            d.flush(false);
            d.now = Some((out, cut_frames));
            produced[j] = true;
        }
    }

    /// A new run: every recording empty, every probe's file cut back to
    /// nothing, every filter holding nothing.
    pub(super) fn restart(&mut self) {
        for p in &mut self.list {
            p.pending.clear();
            p.history.write().clear();
            p.trigger.seek(None, 0.0);
            if let Some(w) = &mut p.stream {
                let _ = w.truncate_after(None);
            }
        }
        for d in &mut self.derived {
            d.pending.clear();
            d.history.write().clear();
            d.held.iter_mut().for_each(|h| *h = None);
            d.now = None;
        }
    }

    /// Back to time `t0`: what was recorded after it goes, and filters
    /// with clocks of their own hold what their sources recorded by then
    /// (ADR 0063).
    pub(super) fn rewind(&mut self, t0: f64, eps: f64) {
        for p in &mut self.list {
            p.flush(true);
            p.history.write().truncate_after(t0);
            p.trigger.seek(Some(t0), eps);
            if let Some(w) = &mut p.stream {
                let _ = w.truncate_after(Some(t0));
            }
        }
        for j in 0..self.derived.len() {
            let mut d = std::mem::replace(
                &mut self.derived[j],
                Derived {
                    entry: FilterEntry::expression("", "0", Vec::new()),
                    key: Vec::new(),
                    filters: Filters::default(),
                    inputs: Vec::new(),
                    sources: Vec::new(),
                    held: Vec::new(),
                    history: SharedHistory::new(History::new(Vec::new())),
                    pending: Vec::new(),
                    now: None,
                },
            );
            d.flush(true);
            d.history.write().truncate_after(t0);
            self.reseed(&mut d);
            self.derived[j] = d;
        }
    }

    /// Each recording's field budget, in bytes.
    pub(super) fn set_field_budget(&self, share: usize) {
        for p in &self.list {
            p.history.write().set_field_budget(share);
        }
        for d in &self.derived {
            d.history.write().set_field_budget(share);
        }
    }

    /// What each probe and filter holds, in bytes, by `probe NAME` and
    /// `filter NAME`.
    pub(super) fn memory_parts(&self, parts: &mut Vec<(String, usize)>) {
        for p in &self.list {
            parts.push((format!("probe {}", p.spec.name), p.history.read().bytes()));
        }
        for d in &self.derived {
            parts.push((format!("filter {}", d.entry.name), d.history.read().bytes()));
        }
    }

    /// The probes in the model file's form.
    pub(super) fn entries(&self) -> Vec<ProbeEntry> {
        self.list.iter().map(|p| p.spec.to_entry()).collect()
    }
}

impl Session {
    /// What the probes record may have moved or gone: they start afresh,
    /// without what a step no longer yields.
    pub(super) fn reset_probes(&mut self) {
        let (scalars, fields) = (
            &self.catalogue.layout_scalars,
            &self.catalogue.layout_fields,
        );
        let recorded =
            |n: &str| scalars.iter().any(|m| m == n) || fields.iter().any(|(f, _)| f == n);
        let specs = self
            .probes
            .take_still_recorded(recorded, &mut self.warnings);
        if let Err(problems) = self.set_probes(specs) {
            self.warnings.extend(problems);
        }
    }

    /// Replace the probes besides the implicit one (ADR 0062). Each must
    /// record signals or fields this session records; otherwise nothing
    /// changes and the problems are returned. Probes record in time only,
    /// not in sweeps, and start empty.
    pub fn set_probes(&mut self, specs: Vec<ProbeSpec>) -> Result<(), Vec<String>> {
        self.install_probes(specs)?;
        self.build_derived();
        self.update_probe_io();
        Ok(())
    }

    /// Which probes keep samples in memory, and which write their files
    /// as they record (ADR 0063): call after anything those depend on
    /// changes.
    pub(super) fn update_probe_io(&mut self) {
        let indexed = self.rewind.recorder().is_some();
        self.probes.update_io(indexed, &mut self.warnings);
    }

    /// Where probes write their files as they record: beside the model
    /// (ADR 0063). `None` while the model has no file.
    pub fn set_probe_dir(&mut self, dir: Option<PathBuf>) {
        if self.probes.set_dir(dir) {
            self.update_probe_io();
        }
    }

    /// Keep every probe's samples in memory, whatever reads them: a
    /// headless run returns them all.
    pub fn record_all_probes(&mut self) {
        self.probes.record_all = true;
        self.update_probe_io();
    }

    /// [`Session::set_probes`] without the filters that read probes.
    fn install_probes(&mut self, specs: Vec<ProbeSpec>) -> Result<(), Vec<String>> {
        let (share, recorded_to, eps) = (self.share(), self.recording.recorded_to(), self.eps());
        self.probes
            .install(specs, &self.catalogue, share, recorded_to, eps)
    }

    /// The filters with an input from a probe (ADR 0063), built again.
    fn build_derived(&mut self) {
        let params = param_keys(&self.flat, &self.ids, self.sim.model());
        let (share, recorded_to) = (self.share(), self.recording.recorded_to());
        self.probes.build_derived(
            &self.catalogue,
            &params,
            share,
            recorded_to,
            &mut self.warnings,
        );
    }

    /// Record a whole sample in the probes and the filters on them, if
    /// there are any.
    pub(super) fn push_probes(&mut self, index: f64, values: &[f64], frames: &[&[f64]]) {
        if self.probes.is_empty() {
            return;
        }
        let (step, eps) = (self.sim.step_count(), self.eps());
        let (sim, flat, ids) = (&self.sim, &self.flat, &self.ids);
        self.probes.push(
            index,
            values,
            frames,
            step,
            eps,
            &self.catalogue.all_coordinates,
            || param_values(sim, flat, ids),
            &mut self.warnings,
        );
    }

    /// The outputs of filters with clocks of their own, as views bind
    /// them (ADR 0063).
    pub fn derived_names(&self) -> &[String] {
        self.probes.derived_names()
    }

    /// What each filter with a clock of its own recorded, by its name.
    pub fn derived_histories(&self) -> Vec<(String, SharedHistory)> {
        self.probes
            .derived
            .iter()
            .map(|d| (d.entry.name.clone(), d.history.clone()))
            .collect()
    }

    /// Set the probes from the model file's form (ADR 0062), as the
    /// canvas edits them: what each probe recorded is kept if its
    /// recording did not change.
    pub fn set_probe_entries(&mut self, entries: &[ProbeEntry]) -> Result<(), Vec<String>> {
        self.set_probes(entries.iter().map(ProbeSpec::from_entry).collect())
    }

    /// The probes in the model file's form.
    pub fn probe_entries(&self) -> Vec<ProbeEntry> {
        self.probes.entries()
    }

    /// Write the files of the probes that have one (`to`) into `dir`, as
    /// recorded so far: the files written, and notes on what was left out.
    pub fn write_probe_files(
        &mut self,
        dir: &Path,
    ) -> std::io::Result<(Vec<PathBuf>, Vec<String>)> {
        self.flush(true);
        let mut written = Vec::new();
        let mut notes = Vec::new();
        for p in &mut self.probes.list {
            // Written as it records: already there, flushed above.
            if let Some(w) = &p.stream {
                written.extend(w.paths.iter().cloned());
                continue;
            }
            if let Some(to) = p.spec.to.as_ref().filter(|_| !p.spec.stream) {
                let (w, n) = crate::probe::write_files(&p.history.read(), dir, to)?;
                written.extend(w);
                notes.extend(n.into_iter().map(|n| format!("probe {}: {n}", p.spec.name)));
            }
        }
        Ok((written, notes))
    }

    /// Where a probe's node sits on the data layer, without restarting
    /// its recording.
    pub fn place_probe(&mut self, name: &str, at: Option<[f64; 2]>) -> Result<(), String> {
        let p = self
            .probes
            .list
            .iter_mut()
            .find(|p| p.spec.name == name)
            .ok_or_else(|| format!("no probe '{name}'"))?;
        p.spec.canvas = at.map(|p| p.map(|v| (v * 10.0).round() / 10.0));
        Ok(())
    }

    /// Build the file's probes, one by one: one that records what this
    /// session does not is dropped with a warning.
    pub(super) fn load_probes(&mut self) {
        let mut kept: Vec<ProbeSpec> = Vec::new();
        for (k, e) in self.file.controls.probes.clone().iter().enumerate() {
            let mut specs = kept.clone();
            specs.push(ProbeSpec::from_entry(e));
            match self.install_probes(specs.clone()) {
                Ok(()) => kept = specs,
                Err(problems) => {
                    for p in problems {
                        self.warnings
                            .push(format!("controls.probes[{k}]: {p}; ignored"));
                    }
                }
            }
        }
        if let Err(problems) = self.set_probes(kept) {
            self.warnings.extend(problems);
        }
    }

    /// The probes besides the implicit one, by name.
    pub fn probe_names(&self) -> Vec<&str> {
        self.probes
            .list
            .iter()
            .map(|p| p.spec.name.as_str())
            .collect()
    }

    /// The probes besides the implicit one, as set.
    pub fn probe_specs(&self) -> Vec<&ProbeSpec> {
        self.probes.list.iter().map(|p| &p.spec).collect()
    }

    /// What probe `name` has recorded, shared for reading (`None` if
    /// there is no such probe). The implicit probe is [`Session::signals`].
    pub fn probe(&self, name: &str) -> Option<SharedHistory> {
        self.probes
            .list
            .iter()
            .find(|p| p.spec.name == name)
            .map(|p| p.history.clone())
    }
}

/// A filter's cut frames for one sample, from its input fields' frames,
/// named by `names`.
fn derived_cuts(
    filters: &Filters,
    names: &[String],
    frames: &[Vec<f64>],
    coordinates: &[Coordinate],
) -> Vec<Vec<f64>> {
    let frame = |name: &str| -> Option<&[f64]> {
        names
            .iter()
            .position(|n| n == name)
            .and_then(|i| frames.get(i))
            .map(Vec::as_slice)
    };
    filters
        .cuts
        .iter()
        .flat_map(|c| match frame(&c.input) {
            Some(input) => {
                let g = c.geometry(coordinates, frame);
                c.frames(&g, input)
            }
            None => vec![Vec::new(); c.settings.count() + 1],
        })
        .collect()
}

/// Evaluate expression filters into `values`, which holds `base` signals
/// first: slots are `t`, those signals, the filters, then `params`.
fn derive(exprs: &[crate::filters::ExprFilter], t: f64, values: &mut Vec<f64>, params: &[f64]) {
    if exprs.is_empty() {
        return;
    }
    let base = values.len();
    let mut vars = Vec::with_capacity(1 + base + exprs.len() + params.len());
    vars.push(t);
    vars.extend_from_slice(values);
    vars.extend(std::iter::repeat_n(0.0, exprs.len()));
    vars.extend_from_slice(params);
    for (k, f) in exprs.iter().enumerate() {
        let v = f.expr.eval(&vars);
        vars[1 + base + k] = v;
        values.push(v);
    }
}
