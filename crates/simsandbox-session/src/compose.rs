//! Running a model that uses model files headless (ADR 0066): every clock
//! domain on its own clock, in exact ticks. Between their steps the
//! domains advance apart, each with its link inputs held from the start of
//! its step; at an instant several share, they evaluate in link order,
//! each taking the latest samples of the probes feeding it, and its probes
//! sample.

use crate::run::{Keep, Output, Population, Results, Thin, DEFAULT_T_END};
use crate::session::{set_key, InputEvent, Track};
use serde_json::{json, Value};
use simsandbox_core::block::Shape;
use simsandbox_core::parallel::Parallel;
use simsandbox_core::registry::Registry;
use simsandbox_core::schedule::Trigger;
use simsandbox_core::sim::Simulator;
use simsandbox_file::compose::{load_composition, Composition};
use std::sync::mpsc;

/// What a run produced: each domain's results at its own times, by its
/// name (`.` for the top level's own blocks, else its path, `plant`), in
/// written order, depth first.
pub struct Composed {
    pub models: Vec<(String, Results)>,
    /// The common tick, in seconds.
    pub tick: f64,
    pub warnings: Vec<String>,
}

impl Composed {
    /// One domain's results.
    pub fn model(&self, name: &str) -> Option<&Results> {
        self.models.iter().find(|(n, _)| n == name).map(|(_, r)| r)
    }

    /// A full name's values (`plant/car.v`), from the domain holding it.
    pub fn values(&self, name: &str) -> Option<&[f64]> {
        self.models.iter().rev().find_map(|(_, r)| {
            r.outputs
                .iter()
                .find(|o| o.name == name)
                .map(|o| o.values.as_slice())
        })
    }

    /// JSON: each model's results as `run --format json` gives one model's.
    pub fn to_json(&self) -> Value {
        let models: serde_json::Map<String, Value> = self
            .models
            .iter()
            .map(|(n, r)| (n.clone(), r.to_json()))
            .collect();
        json!({"made_with": simsandbox_file::runs::MADE_WITH, "models": models, "tick": self.tick, "warnings": self.warnings})
    }
}

/// `check` of a model that uses model files (ADR 0066): its domains with
/// their files, clocks, outputs and the input blocks links can drive, the
/// links, the common tick, the order domains evaluate in at a shared
/// instant, the loops that order breaks, and the model hash.
pub fn check(text: &str, registry: &Registry) -> Value {
    let c = match load_composition(text, registry, None) {
        Ok(c) => c,
        Err(e) => {
            return json!({"ok": false, "errors": [crate::describe::file_error(&e)], "warnings": []})
        }
    };
    let models: Vec<Value> = c
        .domains
        .iter()
        .enumerate()
        .map(|(k, d)| {
            let steps = c.clocks[k].map(|i| c.ticks.steps[i].to_string());
            let flat = &d.loaded.flat;
            let mut outputs = Vec::new();
            for b in &flat.blocks {
                if let Ok(block) = d.loaded.model.block(d.loaded.ids[&b.name]) {
                    for p in block.spec().outputs() {
                        outputs.push(d.full(&format!("{}.{}", b.name, p.name)));
                    }
                }
            }
            let inputs: Vec<String> = flat
                .blocks
                .iter()
                .filter(|b| b.type_name == simsandbox_eq::input::VALUE)
                .map(|b| d.full(&b.name))
                .collect();
            json!({
                "name": d.name(),
                "file": d.file.as_ref().map(|f| f.display().to_string()),
                "dt": (!d.on_data).then_some(d.dt),
                "ticks_per_step": steps,
                "on_new_data": d.on_data,
                "declared_outputs": d.declared.iter().map(|o| json!({"name": o.name, "from": o.from})).collect::<Vec<_>>(),
                "outputs": outputs,
                "inputs": inputs,
                // Parameters driven by wires (ADR 0075), by full name.
                "driven": crate::session::drives(&d.loaded.model)
                    .into_iter()
                    .map(|(to, from)| json!({"param": d.full(&to), "from": d.full(&from)}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let links: Vec<Value> = c
        .links
        .iter()
        .map(|l| json!({"probe": c.probes[l.probe].name, "from": l.signal, "to": l.block}))
        .collect();
    json!({
        "ok": true,
        "errors": [],
        "warnings": c.warnings,
        "composition": true,
        "declared_outputs": c.file.outputs.iter().map(|o| json!({"name": o.name, "from": o.from})).collect::<Vec<_>>(),
        // Files of uses replaced by this file, by full path (ADR 0066
        // step 4c); each model's `file` is the one read.
        "replaced_files": c
            .file
            .blocks
            .iter()
            .flat_map(|b| b.files.iter().map(move |(k, f)| (format!("{}/{k}", b.name), f.clone())))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "definitions": definitions(&c.file),
        // Every parameter by full name, as the app lists them (ADR 0068).
        "params": crate::Session::load_in("check", text, registry, None)
            .map(|s| {
                s.params()
                    .iter()
                    .map(|p| json!({"key": p.key, "value": p.value, "formula": p.formula, "unit": p.unit, "driven_by": p.driven}))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "models": models,
        "links": links,
        "tick": c.ticks.seconds(),
        "order": c.order.iter().map(|&d| c.domains[d].name()).collect::<Vec<_>>(),
        "loops": c.loops,
        "model_hash": c.hash(),
    })
}

/// The models defined in `file` (ADR 0068), each with its ports (input
/// blocks, declared outputs) and the uses naming it at its level.
pub fn definitions(file: &simsandbox_file::ModelFile) -> Value {
    let mut out = serde_json::Map::new();
    for (name, v) in &file.models {
        let Ok(d) = simsandbox_file::compose::read_definition(name, v) else {
            continue;
        };
        // Its exposed inputs (ADR 0074), then its input blocks.
        let inputs: Vec<&str> = d
            .inputs
            .iter()
            .map(|i| i.name.as_str())
            .chain(
                d.blocks
                    .iter()
                    .filter(|b| b.type_name == simsandbox_eq::input::VALUE)
                    .map(|b| b.name.as_str()),
            )
            .collect();
        let uses: Vec<&str> = file
            .blocks
            .iter()
            .filter(|b| b.model.as_deref() == Some(name.as_str()))
            .map(|b| b.name.as_str())
            .collect();
        out.insert(
            name.clone(),
            json!({
                "inputs": inputs,
                "outputs": d.outputs.iter().map(|o| json!({"name": o.name, "from": o.from})).collect::<Vec<_>>(),
                "uses": uses,
                "dt": d.run.dt_written.then_some(d.run.dt),
            }),
        );
    }
    Value::Object(out)
}

/// One clock domain while it runs.
struct Running {
    sim: Simulator,
    plan: simsandbox_file::subsystems::ParamPlan,
    ids: std::collections::BTreeMap<String, simsandbox_core::graph::BlockId>,
    fixed: Vec<(simsandbox_core::graph::BlockId, String)>,
    variable: Vec<(simsandbox_core::graph::BlockId, String)>,
    results: Results,
    times: Vec<f64>,
    /// Which of its steps it keeps (ADR 0076).
    thin: Thin,
    events: Track,
    dt: f64,
    /// Its step in ticks, the steps it takes, and those taken; all 0 for
    /// one that runs on new data.
    every: u128,
    total: u128,
    taken: u128,
    /// Run on new data (ADR 0066 step 3): how many times it has been
    /// evaluated, and when last.
    on_data: bool,
    evaluations: u64,
    /// Its listed instants (ADR 0067 step 4) in ticks and seconds: its
    /// data's samples on new data, else the times it steps to; `taken`
    /// counts those passed.
    listed: Vec<(u128, f64)>,
}

impl Running {
    /// Record the step just reached, if kept.
    fn record(&mut self) {
        if self.thin.keep() {
            self.record_now();
        }
    }

    /// Take back the latest step's record, to record it again.
    fn unrecord(&mut self) {
        if !self.thin.undo() {
            return;
        }
        self.times.pop();
        for o in &mut self.results.outputs {
            let n = o.shape.len();
            o.values.truncate(o.values.len() - n);
        }
        for p in &mut self.results.populations {
            p.frames.pop();
        }
    }

    /// Record the step just reached.
    fn record_now(&mut self) {
        self.times.push(self.sim.time());
        for (o, (id, port)) in self.results.outputs.iter_mut().zip(&self.fixed) {
            o.values
                .extend_from_slice(self.sim.output(*id, port).unwrap_or(&[]));
        }
        for (p, (id, port)) in self.results.populations.iter_mut().zip(&self.variable) {
            p.frames
                .push(self.sim.output(*id, port).unwrap_or(&[]).to_vec());
        }
    }

    /// Apply scheduled input due at `now`; whether any was.
    fn apply(&mut self, now: f64) -> Result<bool, String> {
        let mut any = false;
        while let Some(e) = self
            .events
            .take_due(now, 1e-9 * self.dt.max(now.abs()).max(1e-300))
        {
            set_key(&mut self.sim, &mut self.plan, &self.ids, &e.key, e.value)
                .map_err(|err| format!("input at t = {}: {err}", e.t))?;
            any = true;
        }
        Ok(any)
    }

    /// Evaluate a model that runs on new data at `t`, recorded.
    fn evaluate(&mut self, t: f64) -> Result<(), String> {
        self.apply(t)?;
        self.evaluations += 1;
        self.sim.restart(t).map_err(|e| e.to_string())?;
        self.record();
        Ok(())
    }
}

/// What a point pass carries from one instant to the next: each probe's
/// trigger, and what each link last received.
struct Links {
    triggers: Vec<Trigger>,
    latest: Vec<f64>,
    /// The time of the model each link's latest sample came from.
    time: Vec<f64>,
    delayed: bool,
}

/// A link end in another part of the run (ADR 0066 step 6): its samples
/// come from, or go to, a part on another thread.
const OUTSIDE: usize = usize::MAX;

/// A sample passed on: (link, value, the sampling model's time), the link
/// by its place in the whole run.
type Sample = (usize, f64, f64);

/// What one part of a run passed on since it last did: from part `from`,
/// the instants with samples and how many each has, the samples in that
/// order (one array, not one per instant: each costs an allocation, and
/// freeing it on another thread costs more), and the last instant it has
/// passed (`u128::MAX`: it has finished).
#[derive(Default)]
struct Batch {
    from: usize,
    upto: u128,
    instants: Vec<(u128, usize)>,
    samples: Vec<Sample>,
}

/// Microseconds each part sleeps before passing samples on (tests).
#[cfg(test)]
static LAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Samples received from one part, not yet reached: its instants with
/// their counts, and the samples in order.
type Pending = (
    std::collections::VecDeque<(u128, usize)>,
    std::collections::VecDeque<Sample>,
);

/// A part passes its samples on every this many instants (and when it
/// finishes): passing them on wakes the part they reach, which costs more
/// than a small model's step.
const FLUSH: usize = 64;

/// How a part of the run meets the parts it is linked to one way: the
/// samples it receives, and those it passes on.
#[derive(Default)]
struct Wire {
    me: usize,
    inbox: Option<mpsc::Receiver<Batch>>,
    /// The parts feeding this one, and the last tick each has passed
    /// (`None`: none yet).
    upstream: Vec<usize>,
    passed: Vec<Option<u128>>,
    /// Samples received, not yet reached, from each part feeding this one:
    /// its instants with their counts, and the samples.
    pending: Vec<Pending>,
    /// Each link into this part by its place in the whole run: its place
    /// here.
    local: std::collections::HashMap<usize, usize>,
    /// The parts this one feeds, and for each link leaving it (by its
    /// place here) the run's place and which of `downstream` it goes to;
    /// the instants not yet passed on to each, and how many there were.
    downstream: Vec<mpsc::Sender<Batch>>,
    out: Vec<(usize, usize, usize)>,
    held: Vec<Batch>,
    since: usize,
}

impl Wire {
    /// Receive until every part feeding this one has passed `until` (or,
    /// with `None`, finished).
    fn wait(&mut self, until: Option<u128>) {
        let Some(inbox) = &self.inbox else { return };
        let behind = |p: &Option<u128>| match (p, until) {
            (None, _) => true,
            (Some(p), _) if *p == u128::MAX => false,
            (Some(_), None) => true,
            (Some(p), Some(u)) => *p < u,
        };
        while self.passed.iter().any(behind) {
            let Ok(b) = inbox.recv() else {
                // Every sender gone: every part feeding this one finished.
                self.passed.iter_mut().for_each(|p| *p = Some(u128::MAX));
                break;
            };
            let k = self
                .upstream
                .iter()
                .position(|&u| u == b.from)
                .expect("a part feeding this one");
            self.passed[k] = Some(b.upto);
            self.pending[k].0.extend(b.instants);
            self.pending[k].1.extend(b.samples);
        }
    }

    /// The earliest instant received samples are for.
    fn next(&self) -> Option<u128> {
        self.pending
            .iter()
            .filter_map(|q| q.0.front())
            .map(|i| i.0)
            .min()
    }

    /// Take the samples for `tick` into `links`: the links they reach,
    /// fresh in this instant.
    fn apply(&mut self, tick: u128, links: &mut Links, mut fresh: Vec<bool>) -> Vec<bool> {
        fresh.clear();
        fresh.resize(links.latest.len(), false);
        for (instants, samples) in &mut self.pending {
            if instants.front().is_none_or(|i| i.0 != tick) {
                continue;
            }
            let (_, n) = instants.pop_front().expect("checked");
            for (g, v, t) in samples.drain(..n) {
                let k = self.local[&g];
                links.latest[k] = v;
                links.time[k] = t;
                fresh[k] = true;
            }
        }
        fresh
    }

    /// Keep the samples taken at `tick` for the parts they reach, and
    /// pass them on every [`FLUSH`] instants.
    fn send(&mut self, tick: u128, fresh: &[bool], links: &Links) {
        // Tests make a feeding part lag, so the parts it feeds must wait.
        #[cfg(test)]
        {
            let lag = LAG.load(std::sync::atomic::Ordering::Relaxed);
            if lag > 0 {
                std::thread::sleep(std::time::Duration::from_micros(lag));
            }
        }
        for &(k, g, d) in &self.out {
            if fresh[k] {
                let held = &mut self.held[d];
                match held.instants.last_mut() {
                    Some((t, n)) if *t == tick => *n += 1,
                    _ => held.instants.push((tick, 1)),
                }
                held.samples.push((g, links.latest[k], links.time[k]));
            }
        }
        self.since += 1;
        if self.since >= FLUSH {
            self.flush(tick);
        }
    }

    /// Pass on what is held, the instants up to `upto` passed.
    fn flush(&mut self, upto: u128) {
        self.since = 0;
        for (d, held) in self.held.iter_mut().enumerate() {
            let mut b = std::mem::take(held);
            b.from = self.me;
            b.upto = upto;
            // A part that stopped early (an error) no longer listens.
            let _ = self.downstream[d].send(b);
        }
    }
}

impl Drop for Wire {
    /// Pass on what is held and say this part has finished, however it
    /// ended.
    fn drop(&mut self) {
        self.flush(u128::MAX);
    }
}

/// Run a model that uses model files to `t_end` (the file's, if not
/// given). Used files are found from its folder (see
/// [`simsandbox_core::registry::in_folder`]).
pub fn run_composition(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
) -> Result<Composed, String> {
    run_composition_with(text, registry, t_end, Parallel::Auto)
}

/// [`run_composition`], the models stepping between instants on several
/// cores as `parallel` says (ADR 0066 step 6); the same bits either way.
pub fn run_composition_with(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    parallel: Parallel,
) -> Result<Composed, String> {
    run_composition_keeping(text, registry, t_end, parallel, &Keep::default())
}

/// [`run_composition_with`], keeping only what `keep` asks for (ADR
/// 0076): outputs by full name or model, every few steps of each model.
pub fn run_composition_keeping(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    parallel: Parallel,
    keep: &Keep,
) -> Result<Composed, String> {
    run_in_parts(text, registry, t_end, parallel, keep, true)
}

/// [`run_composition_keeping`]; with `split` false, linked models all
/// meet at every instant (one part per group), for tests to compare.
fn run_in_parts(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    parallel: Parallel,
    keep: &Keep,
    split: bool,
) -> Result<Composed, String> {
    if let Some(t) = t_end.filter(|t| !(t.is_finite() && *t > 0.0)) {
        return Err(format!("the end time must be positive, got {t}"));
    }
    let mut c = load_composition(text, registry, None).map_err(|e| e.to_string())?;
    // With no clock every `dt`, a run only data drives ends at its last
    // sample (ADR 0067 step 4).
    let data_end = c
        .clocks
        .iter()
        .all(Option::is_none)
        .then(|| c.times.iter().flatten().map(|t| t.1).reduce(f64::max))
        .flatten();
    let t_end = t_end
        .or(c.file.run.t_end)
        .or(data_end)
        .unwrap_or(DEFAULT_T_END);
    let end_tick = c.ticks.within(t_end).map_err(|e| e.to_string())?;
    let mut warnings = c.warnings.clone();
    warnings.extend(c.loops.iter().map(|l| format!("loop between models: {l}")));
    let runs = start(&mut c, t_end, keep)?;
    // The run in parts (ADR 0066 step 6): models that feed each other run
    // together, meeting at every instant they share; a part linked to
    // another one way only runs ahead of it on a thread of its own,
    // passing its samples on; parts no link joins never meet.
    let plan = Plan::of(&c);
    let parts = if split {
        plan.parts(runs.len())
    } else {
        plan.groups(runs.len())
    };
    let mut slots: Vec<Option<Running>> = runs.into_iter().map(Some).collect();
    let part_of = |u: usize| parts.iter().position(|p| p.contains(&u)).expect("a part");
    let mut wires: Vec<Wire> = (0..parts.len())
        .map(|me| {
            let mut w = Wire::default();
            w.me = me;
            w
        })
        .collect();
    let mut work: Vec<(Plan, Vec<Running>)> = Vec::new();
    for (me, members) in parts.iter().enumerate() {
        let (sub, global) = plan.part(members);
        for (k, l) in sub.links.iter().enumerate() {
            let g = global[k];
            if l.from == OUTSIDE {
                wires[me].local.insert(g, k);
                let from = part_of(plan.links[g].from);
                if !wires[me].upstream.contains(&from) {
                    wires[me].upstream.push(from);
                }
            }
        }
        let mine = members
            .iter()
            .map(|&u| slots[u].take().expect("each model in one part"))
            .collect();
        work.push((sub, mine));
    }
    // A channel into each part fed by another, a sender from each feeder.
    for me in 0..parts.len() {
        if wires[me].upstream.is_empty() {
            continue;
        }
        let (tx, rx) = mpsc::channel();
        wires[me].passed = vec![None; wires[me].upstream.len()];
        wires[me].pending = vec![Default::default(); wires[me].upstream.len()];
        wires[me].inbox = Some(rx);
        for from in wires[me].upstream.clone() {
            let d = wires[from].downstream.len();
            wires[from].downstream.push(tx.clone());
            wires[from].held.push(Batch::default());
            let (sub, global) = (&work[from].0, plan.part(&parts[from]).1);
            for (k, l) in sub.links.iter().enumerate() {
                if l.to == OUTSIDE && part_of(plan.links[global[k]].to) == me {
                    wires[from].out.push((k, global[k], d));
                }
            }
        }
    }
    let apart = parts.len() > 1 && parallel != Parallel::Never;
    let items: Vec<((Plan, Vec<Running>), Wire)> = work.into_iter().zip(wires).collect();
    let run_part = |((sub, mine), mut wire): ((Plan, Vec<Running>), Wire)| {
        let result = timeline(&sub, mine, end_tick, parallel, &mut wire);
        drop(wire);
        result
    };
    // Each part on a thread of its own when apart (a part waiting for
    // another must not hold the thread that one needs); else upstream
    // first, its samples queued.
    let done: Vec<Result<(Vec<Running>, bool), String>> = if apart {
        std::thread::scope(|s| {
            let handles: Vec<_> = items
                .into_iter()
                .map(|item| s.spawn(move || run_part(item)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("a part of the run panicked"))
                .collect()
        })
    } else {
        items.into_iter().map(run_part).collect()
    };
    let mut delayed = false;
    let mut failed = None;
    for (members, result) in parts.iter().zip(done) {
        match result {
            Ok((mine, late)) => {
                delayed |= late;
                for (&u, r) in members.iter().zip(mine) {
                    slots[u] = Some(r);
                }
            }
            Err(e) => {
                failed.get_or_insert(e);
            }
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    let runs: Vec<Running> = slots
        .into_iter()
        .map(|r| r.expect("every model ran"))
        .collect();
    if delayed {
        warnings.push(
            "an output depending at the same instant on a model evaluated after it saw that \
             model's samples one step late (see the loop above)"
                .into(),
        );
    }
    Ok(Composed {
        models: c
            .domains
            .iter()
            .zip(runs)
            .map(|(d, mut r)| {
                // Its last step, kept whatever `every` says.
                if r.thin.behind {
                    r.record_now();
                }
                r.results.index = vec![("t".into(), std::mem::take(&mut r.times))];
                (d.name().to_string(), r.results)
            })
            .collect(),
        tick: c.ticks.seconds(),
        warnings,
    })
}

/// What the instant loop needs of a composition, for all its models or
/// one group of them renumbered from 0 (ADR 0066 step 6).
#[derive(Clone)]
struct Plan {
    links: Vec<simsandbox_file::compose::Link>,
    probes: Vec<simsandbox_file::compose::LinkProbe>,
    order: Vec<usize>,
    names: Vec<String>,
}

impl Plan {
    fn of(c: &Composition) -> Self {
        Self {
            links: c.links.clone(),
            probes: c.probes.clone(),
            order: c.order.clone(),
            names: c.domains.iter().map(|d| d.name().to_string()).collect(),
        }
    }

    /// The models links join, each group in model order, the groups by
    /// their first model.
    fn groups(&self, n: usize) -> Vec<Vec<usize>> {
        let mut root: Vec<usize> = (0..n).collect();
        fn find(root: &mut [usize], u: usize) -> usize {
            let mut u = u;
            while root[u] != u {
                root[u] = root[root[u]];
                u = root[u];
            }
            u
        }
        for l in &self.links {
            let (a, b) = (find(&mut root, l.from), find(&mut root, l.to));
            root[a.max(b)] = a.min(b);
        }
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut first: Vec<Option<usize>> = vec![None; n];
        for u in 0..n {
            let r = find(&mut root, u);
            match first[r] {
                Some(g) => groups[g].push(u),
                None => {
                    first[r] = Some(groups.len());
                    groups.push(vec![u]);
                }
            }
        }
        groups
    }

    /// The parts a run splits into, upstream first: models that feed
    /// each other, directly or around, in one part; parts joined only one
    /// way. Every link between parts must run from earlier to later in
    /// `order` (it does, unless a loop broken in written order puts a
    /// model before one feeding it); where one would not, its group of
    /// linked models is one part.
    fn parts(&self, n: usize) -> Vec<Vec<usize>> {
        // Which models each reaches through links.
        let mut reach = vec![vec![false; n]; n];
        for (u, row) in reach.iter_mut().enumerate() {
            let mut stack = vec![u];
            while let Some(v) = stack.pop() {
                for l in self.links.iter().filter(|l| l.from == v) {
                    if !row[l.to] {
                        row[l.to] = true;
                        stack.push(l.to);
                    }
                }
            }
        }
        let place = |u: usize| self.order.iter().position(|&o| o == u).unwrap_or(u);
        let together = |u: usize, v: usize| u == v || (reach[u][v] && reach[v][u]);
        let mut parts: Vec<Vec<usize>> = Vec::new();
        for group in self.groups(n) {
            let split_ok = self.links.iter().all(|l| {
                !group.contains(&l.from) || together(l.from, l.to) || place(l.from) < place(l.to)
            });
            if !split_ok {
                parts.push(group);
                continue;
            }
            let mut left = group.clone();
            while let Some(&u) = left.first() {
                let part: Vec<usize> = left.iter().copied().filter(|&v| together(u, v)).collect();
                left.retain(|v| !part.contains(v));
                parts.push(part);
            }
        }
        parts.sort_by_key(|p| p.iter().map(|&u| place(u)).min());
        parts
    }

    /// The plan of the models `members`, numbered by their place there,
    /// with the links reaching them from other parts or leaving them for
    /// other parts, their other end [`OUTSIDE`]; and each link's place in
    /// this plan.
    fn part(&self, members: &[usize]) -> (Self, Vec<usize>) {
        let local = |u: usize| members.iter().position(|&m| m == u);
        let probes: Vec<usize> = (0..self.probes.len())
            .filter(|&p| local(self.probes[p].domain).is_some())
            .collect();
        let mut global = Vec::new();
        let links = self
            .links
            .iter()
            .enumerate()
            .filter(|(_, l)| local(l.from).is_some() || local(l.to).is_some())
            .map(|(g, l)| {
                global.push(g);
                simsandbox_file::compose::Link {
                    probe: probes.iter().position(|&p| p == l.probe).unwrap_or(OUTSIDE),
                    from: local(l.from).unwrap_or(OUTSIDE),
                    to: local(l.to).unwrap_or(OUTSIDE),
                    ..l.clone()
                }
            })
            .collect();
        let plan = Self {
            links,
            probes: probes
                .iter()
                .map(|&p| simsandbox_file::compose::LinkProbe {
                    domain: local(self.probes[p].domain).expect("kept"),
                    ..self.probes[p].clone()
                })
                .collect(),
            order: self.order.iter().filter_map(|&u| local(u)).collect(),
            names: members.iter().map(|&u| self.names[u].clone()).collect(),
        };
        (plan, global)
    }
}

/// Run the models of `c` from the start to `end_tick`, instant by
/// instant, stepping those due at an instant on several cores when worth
/// it; the models as run, and whether a loop delivered a sample late.
fn timeline(
    c: &Plan,
    mut runs: Vec<Running>,
    end_tick: u128,
    parallel: Parallel,
    wire: &mut Wire,
) -> Result<(Vec<Running>, bool), String> {
    let mut links = Links {
        triggers: c
            .probes
            .iter()
            .map(|p| Trigger::new(p.every.clone()))
            .collect(),
        latest: vec![f64::NAN; c.links.len()],
        time: vec![f64::NAN; c.links.len()],
        delayed: false,
    };
    // The start: every model has a point at t = 0.
    for r in runs.iter_mut().filter(|r| !r.on_data) {
        if r.apply(0.0)? {
            r.sim.restart(0.0).map_err(|e| e.to_string())?;
        }
    }
    let clocked: Vec<usize> = (0..runs.len()).filter(|&u| !runs[u].on_data).collect();
    let names = &c.names;
    let mut costs = simsandbox_core::parallel::Costs::pooled(runs.len());
    // Threads kept for the part's run, started the first time models are
    // worth stepping on several cores.
    let mut pool: Option<simsandbox_core::parallel::Pool> = None;
    // Samples from parts feeding this one at the start come first.
    wire.wait(Some(0));
    let fresh = wire.apply(0, &mut links, Vec::new());
    let fresh = point(c, &mut runs, &clocked, true, &mut links, &[], None, fresh)?;
    wire.send(0, &fresh, &links);
    // Which links are fresh at an instant, kept between instants.
    let mut buffer = fresh;
    // Each model's next instant: its next step, or its next listed time
    // up to the end.
    let next_of = |r: &Running| -> Option<(u128, f64)> {
        if !r.listed.is_empty() {
            return r
                .listed
                .get(r.taken as usize)
                .copied()
                .filter(|&(k, _)| k <= end_tick);
        }
        (!r.on_data && r.taken < r.total).then(|| ((r.taken + 1) * r.every, f64::NAN))
    };
    loop {
        // The next instant: this part's own, or one at which a part
        // feeding it passed samples on, once those have all passed it.
        let own = runs.iter().filter_map(next_of).map(|n| n.0).min();
        wire.wait(own);
        let tick = match (own, wire.next()) {
            (None, None) => break,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => a.min(b),
        };
        let fresh = wire.apply(tick, &mut links, std::mem::take(&mut buffer));
        if own != Some(tick) {
            // Only received samples now: models on new data they reach
            // are evaluated; the rest take them at their own instants.
            buffer = if runs.iter().any(|r| r.on_data) {
                let fresh = point(c, &mut runs, &[], false, &mut links, &[], None, fresh)?;
                wire.send(tick, &fresh, &links);
                fresh
            } else {
                fresh
            };
            continue;
        }
        let reaching: Vec<usize> = (0..runs.len())
            .filter(|&u| !runs[u].on_data && next_of(&runs[u]).map(|n| n.0) == Some(tick))
            .collect();
        // Models on new data whose recorded data has a sample now.
        let due: Vec<(usize, f64)> = (0..runs.len())
            .filter(|&u| runs[u].on_data)
            .filter_map(|u| next_of(&runs[u]).filter(|n| n.0 == tick).map(|n| (u, n.1)))
            .collect();
        for &(u, _) in &due {
            runs[u].taken += 1;
        }
        // Between points the models step apart, their link inputs held:
        // by `dt`, or to their next listed time; on several cores when
        // worth it (ADR 0066 step 6).
        let cores = costs.worth(parallel, &reaching);
        if reaching.len() == 1 {
            // One model: nothing to share or to measure for sharing.
            let u = reaching[0];
            step_model(&mut runs[u]).map_err(|e| named(&names[u], e))?;
        } else if cores {
            // On the part's pool of threads, kept for the run: each model
            // lent to it, moved out and back (ADR 0066 step 6).
            let threads = simsandbox_core::parallel::threads();
            let pool = pool.get_or_insert_with(|| {
                simsandbox_core::parallel::Pool::new(threads.saturating_sub(1))
            });
            for (u, (stepped, took)) in lend(pool, &mut runs, &reaching, step_lent) {
                costs.record(u, took);
                stepped.map_err(|e| named(&names[u], e))?;
            }
        } else {
            for &u in &reaching {
                let start = std::time::Instant::now();
                let stepped = step_model(&mut runs[u]);
                costs.record(u, start.elapsed());
                stepped.map_err(|e| named(&names[u], e))?;
            }
        }
        let fresh = point(
            c,
            &mut runs,
            &reaching,
            false,
            &mut links,
            &due,
            pool.as_ref().filter(|_| cores),
            fresh,
        )?;
        wire.send(tick, &fresh, &links);
        buffer = fresh;
    }
    Ok((runs, links.delayed))
}

/// Each domain's simulator and recording, before the first step.
fn start(c: &mut Composition, t_end: f64, keep: &Keep) -> Result<Vec<Running>, String> {
    // Scheduled input by full name goes to the domain holding it.
    let mut schedules: Vec<Vec<InputEvent>> = c.domains.iter().map(|_| Vec::new()).collect();
    for e in &c.file.run.inputs {
        let (d, key) = c
            .domain_of(&e.set)
            .ok_or_else(|| format!("run.inputs: {} is in no model", e.set))?;
        schedules[d].push(InputEvent {
            t: e.t,
            key: key.to_string(),
            value: e.to,
        });
    }
    let mut runs = Vec::new();
    let mut all = Vec::new();
    for (k, (u, schedule)) in c.domains.iter_mut().zip(schedules).enumerate() {
        let (every, total) = match c.clocks[k] {
            Some(i) => (
                c.ticks.steps[i],
                c.ticks.steps_within(i, t_end).map_err(|e| e.to_string())?,
            ),
            None => (0, 0),
        };
        let prefix = if u.path.is_empty() {
            String::new()
        } else {
            format!("{}/", u.path)
        };
        let name = u.name().to_string();
        let model = name.clone();
        let on_data = u.on_data;
        let loaded = &mut u.loaded;
        let mut fixed = Vec::new();
        let mut variable = Vec::new();
        let mut outputs = Vec::new();
        let mut populations = Vec::new();
        for entry in &loaded.flat.blocks {
            let id = loaded.ids[&entry.name];
            let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
            for port in spec.outputs() {
                let name = format!("{prefix}{}.{}", entry.name, port.name);
                all.push((model.clone(), name.clone()));
                if !keep.wants(&model, &name) {
                    continue;
                }
                if let Shape::Population { dim } = port.shape {
                    variable.push((id, port.name.clone()));
                    populations.push(Population {
                        name,
                        dim,
                        frames: Vec::new(),
                    });
                } else {
                    fixed.push((id, port.name.clone()));
                    outputs.push(Output {
                        name,
                        shape: port.shape,
                        axes: port.axis_names.clone(),
                        // Its rows, when it steps every `dt`: room from the
                        // start, not grown as they come.
                        values: Vec::with_capacity(
                            (total as usize / keep.every.max(1) + 2) * port.shape.len(),
                        ),
                    });
                }
            }
        }
        // The simulator takes the built model; the composition keeps the
        // rest (names, links) for the run.
        let model = std::mem::take(&mut loaded.model);
        let sim = Simulator::new(model, 0.0).map_err(|e| format!("{name}: {e}"))?;
        let results = Results {
            index: Vec::new(),
            outputs,
            populations,
            labels: sim.labels(),
            notes: crate::session::loop_notes(&sim),
            warnings: Vec::new(),
        };
        runs.push(Running {
            sim,
            plan: loaded.params.clone(),
            ids: loaded.ids.clone(),
            fixed,
            variable,
            results,
            times: Vec::new(),
            thin: keep.thin(),
            events: Track::new(schedule, Vec::new()),
            dt: u.dt,
            every,
            total,
            taken: 0,
            on_data,
            evaluations: 0,
            listed: c.times[k].clone(),
        });
    }
    keep.check(&all)?;
    Ok(runs)
}

/// One instant: the clocked models in `reaching` (all at the same tick)
/// and the models on new data, in link order. A clocked model takes its
/// link inputs and evaluates; a model on new data is evaluated if a link
/// into it delivered a sample during this pass (or at the start), at the
/// time of the model that delivered it; then its probes sample. A model
/// that read one evaluated after it (a loop) then takes those samples
/// too: for its next step, or, on new data, evaluated again.
#[allow(clippy::too_many_arguments)]
fn point(
    c: &Plan,
    runs: &mut Vec<Running>,
    reaching: &[usize],
    start: bool,
    links: &mut Links,
    due: &[(usize, f64)],
    pool: Option<&simsandbox_core::parallel::Pool>,
    fresh: Vec<bool>,
) -> Result<Vec<bool>, String> {
    let order: Vec<usize> = c
        .order
        .iter()
        .copied()
        .filter(|&u| reaching.contains(&u) || runs[u].on_data)
        .collect();
    // Which links received a sample in this pass (those from other parts
    // given), and which models on new data were evaluated, at what time.
    let mut fresh = fresh;
    let mut evaluated: Vec<Option<f64>> = vec![None; runs.len()];
    // When a link delivered to `u`: the time of the model it sampled.
    let delivery = |time: &[f64], fresh: &[bool], u: usize| -> Option<f64> {
        c.links
            .iter()
            .enumerate()
            .find(|(k, l)| l.to == u && fresh[*k])
            .map(|(k, _)| time[k])
    };
    for &u in &order {
        if runs[u].on_data {
            // At the start, at a sample of its own data, or when a link
            // delivers.
            let own = due.iter().find(|d| d.0 == u).map(|d| d.1);
            let at = if start {
                Some(0.0)
            } else {
                own.or_else(|| delivery(&links.time, &fresh, u))
            };
            let Some(t) = at else { continue };
            take_links(c, runs, u, &links.latest)?;
            runs[u]
                .evaluate(t)
                .map_err(|e| format!("{}: {e}", c.names[u]))?;
            evaluated[u] = Some(t);
        } else {
            take_links(c, runs, u, &links.latest)?;
        }
        sample(c, runs, u, links, &mut fresh);
    }
    // Loops: samples taken after a model evaluated reach it now.
    for &u in &order {
        let sampled: Vec<f64> = c
            .links
            .iter()
            .filter(|l| l.from == u)
            .map(|l| value(&runs[u].sim, &l.source))
            .collect();
        let changed = take_links(c, runs, u, &links.latest)?;
        if runs[u].on_data && changed {
            let Some(t) = evaluated[u].or_else(|| delivery(&links.time, &fresh, u)) else {
                continue;
            };
            // Evaluated again with what arrived: its record for this
            // instant is the new one.
            if evaluated[u].is_some() {
                runs[u].unrecord();
            }
            runs[u]
                .evaluate(t)
                .map_err(|e| format!("{}: {e}", c.names[u]))?;
            evaluated[u] = Some(t);
        }
        let again: Vec<f64> = c
            .links
            .iter()
            .filter(|l| l.from == u)
            .map(|l| value(&runs[u].sim, &l.source))
            .collect();
        if sampled
            .iter()
            .zip(&again)
            .any(|(a, b)| a.to_bits() != b.to_bits())
        {
            links.delayed = true;
        }
    }
    // Each model records its own outputs: on several cores when its
    // stepping was (ADR 0066 step 6).
    if let Some(pool) = pool.filter(|_| reaching.len() > 1) {
        lend(pool, runs, reaching, record_lent);
    } else {
        for &u in reaching {
            runs[u].record();
        }
    }
    Ok(fresh)
}

/// Step one model between instants, its link inputs held: by `dt`, or to
/// its next listed time. An error from the step itself is `Err((true,
/// …))`, to be named by its model; one from scheduled input `(false, …)`,
/// which names itself.
fn step_model(r: &mut Running) -> Result<(), (bool, String)> {
    let now = r.sim.time();
    r.apply(now).map_err(|e| (false, e))?;
    let dt = match r.listed.get(r.taken as usize) {
        Some(&(_, t)) => t - now,
        None => r.dt,
    };
    r.sim.step(dt).map_err(|e| (true, e.to_string()))?;
    r.taken += 1;
    Ok(())
}

/// A stepping error as the instant loop reports it.
fn named(name: &str, (of_step, e): (bool, String)) -> String {
    if of_step {
        format!("{name}: {e}")
    } else {
        e
    }
}

/// How a model's step went, and what it took.
type Stepped = (Result<(), (bool, String)>, std::time::Duration);

/// [`step_model`] on a model lent to a pool, timed.
fn step_lent((u, mut r): (usize, Running)) -> (usize, Running, Stepped) {
    let start = std::time::Instant::now();
    let stepped = step_model(&mut r);
    (u, r, (stepped, start.elapsed()))
}

/// [`Running::record`] on a model lent to a pool.
fn record_lent((u, mut r): (usize, Running)) -> (usize, Running, ()) {
    r.record();
    (u, r, ())
}

/// Run `f` on the models `which` on `pool`'s threads: each moved out of
/// `runs`, worked on and moved back; what each gave, by model.
fn lend<R: Send + 'static>(
    pool: &simsandbox_core::parallel::Pool,
    runs: &mut Vec<Running>,
    which: &[usize],
    f: fn((usize, Running)) -> (usize, Running, R),
) -> Vec<(usize, R)> {
    let mut slots: Vec<Option<Running>> = std::mem::take(runs).into_iter().map(Some).collect();
    let items: Vec<(usize, Running)> = which
        .iter()
        .map(|&u| (u, slots[u].take().expect("each model lent once")))
        .collect();
    let done = pool.each(items, f);
    let mut out = Vec::with_capacity(done.len());
    for (u, r, result) in done {
        slots[u] = Some(r);
        out.push((u, result));
    }
    *runs = slots
        .into_iter()
        .map(|r| r.expect("every model back"))
        .collect();
    out
}

/// The probes on model `u` sample if due: each link they feed takes the
/// value, marked as fresh in this pass.
fn sample(c: &Plan, runs: &[Running], u: usize, links: &mut Links, fresh: &mut [bool]) {
    let r = &runs[u];
    let t = r.sim.time();
    // A model on new data counts its evaluations as its steps.
    let step = if r.on_data {
        r.evaluations.saturating_sub(1)
    } else {
        r.sim.step_count()
    };
    let eps = 1e-9 * r.dt.max(t.abs()).max(1e-300);
    for (p, probe) in c.probes.iter().enumerate() {
        if probe.domain != u || !links.triggers[p].due(t, step, eps) {
            continue;
        }
        for (k, l) in c.links.iter().enumerate().filter(|(_, l)| l.probe == p) {
            links.latest[k] = value(&r.sim, &l.source);
            links.time[k] = t;
            fresh[k] = true;
        }
    }
}

/// Set a model's linked input blocks to the latest samples and apply them;
/// whether any changed.
fn take_links(c: &Plan, runs: &mut [Running], u: usize, latest: &[f64]) -> Result<bool, String> {
    let r = &mut runs[u];
    let mut changed = false;
    for (k, l) in c.links.iter().enumerate().filter(|(_, l)| l.to == u) {
        // Until its first sample, an input block keeps its own value.
        if latest[k].is_nan() {
            continue;
        }
        let now = r.sim.param(l.id, "y").map_err(|e| e.to_string())?;
        if latest[k].to_bits() != now.to_bits() {
            r.sim
                .set_param(l.id, "y", latest[k])
                .map_err(|e| format!("{}: {e}", l.block))?;
            changed = true;
        }
    }
    r.sim
        .refresh()
        .map_err(|e| format!("{}: {e}", c.names[u]))?;
    Ok(changed)
}

fn value(sim: &Simulator, s: &simsandbox_file::compose::Source) -> f64 {
    sim.output(s.block, &s.port)
        .and_then(|v| v.get(s.index))
        .copied()
        .unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use serde_json::json;
    use std::path::{Path, PathBuf};

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("simsandbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, name: &str, v: &Value) {
        std::fs::write(dir.join(name), v.to_string()).unwrap();
    }

    fn run_in(dir: &Path, model: &Value) -> Composed {
        simsandbox_core::registry::in_folder(Some(dir), || {
            run_composition(&model.to_string(), &registry(), None)
        })
        .unwrap()
    }

    /// Each model's rows as kept with `every` (and its last), and its
    /// outputs `signals` keeps, equal bit for bit to the whole run's.
    fn keeps_the_same(dir: &Path, model: &Value, signals: Option<Vec<String>>, every: usize) {
        let all = run_in(dir, model);
        let keep = Keep {
            signals: signals.clone(),
            every,
        };
        let some = simsandbox_core::registry::in_folder(Some(dir), || {
            run_composition_keeping(&model.to_string(), &registry(), None, Parallel::Auto, &keep)
        })
        .unwrap();
        for ((name, full), (_, kept)) in all.models.iter().zip(&some.models) {
            let rows = full.rows();
            let mut want: Vec<usize> = (0..rows).step_by(every).collect();
            if rows > 0 && want.last() != Some(&(rows - 1)) {
                want.push(rows - 1);
            }
            let pick = |v: &[f64], n: usize| -> Vec<u64> {
                want.iter()
                    .flat_map(|&r| v[r * n..(r + 1) * n].iter().map(|x| x.to_bits()))
                    .collect()
            };
            assert_eq!(bits(&kept.index[0].1), pick(&full.index[0].1, 1), "{name}");
            let names: Vec<&str> = full
                .outputs
                .iter()
                .map(|o| o.name.as_str())
                .filter(|n| keep.wants(name, n))
                .collect();
            let got: Vec<&str> = kept.outputs.iter().map(|o| o.name.as_str()).collect();
            assert_eq!(got, names, "{name}");
            for o in &kept.outputs {
                let f = full.outputs.iter().find(|f| f.name == o.name).unwrap();
                assert_eq!(
                    bits(&o.values),
                    pick(&f.values, o.shape.len()),
                    "{}",
                    o.name
                );
            }
        }
    }

    fn check_in(dir: &Path, v: &Value) -> Value {
        simsandbox_core::registry::in_folder(Some(dir), || check(&v.to_string(), &registry()))
    }

    fn series<'a>(r: &'a Results, name: &str) -> &'a [f64] {
        &r.outputs.iter().find(|o| o.name == name).unwrap().values
    }

    fn bits(v: &[f64]) -> Vec<u64> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// `vehicle.ssbx.json`: the car and the controller, linked.
    fn write_file(dir: &Path) {
        write(
            dir,
            "vehicle.ssbx.json",
            &with(
                json!({"format": 1,
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
    }

    fn use_of(name: &str, file: &str) -> Value {
        json!({"name": name, "file": file})
    }

    /// The cruise control's car and controller as two model files in `dir`,
    /// each reading the other through an input block.
    fn cruise_files(dir: &Path) -> Value {
        let text = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap()
            .text;
        let whole: Value = serde_json::from_str(text).unwrap();
        let (car, cruise) = (&whole["blocks"][0], &whole["blocks"][1]);
        let input = |name: &str, unit: &str| json!({"name": name, "type": "input.value", "settings": {"unit": unit}});
        let run = json!({"dt": 0.05, "t_end": 20});
        write(
            dir,
            "car.ssbx.json",
            &json!({
            "format": 1, "run": run, "fusion": "off",
            "blocks": [input("force", "N"), car],
            "connections": [{"from": "force.y", "to": "car.F"}]}),
        );
        write(
            dir,
            "cruise.ssbx.json",
            &json!({
            "format": 1, "run": run, "fusion": "off",
            "blocks": [input("speed", "m/s"), cruise],
            "connections": [{"from": "speed.y", "to": "cruise.v"}]}),
        );
        whole
    }

    /// Two uses linked both ways every step, under `prefix` (`plant` or
    /// `vehicle/plant`).
    fn cruise_links(plant: &str, controller: &str) -> Value {
        json!({
            "connections": [
                {"from": format!("probe:speed.{plant}/car.v"), "to": format!("{controller}/speed")},
                {"from": format!("probe:force.{controller}/cruise.F"), "to": format!("{plant}/force")}],
            "controls": {"probes": [
                {"name": "speed", "record": [format!("{plant}/car.v")]},
                {"name": "force", "record": [format!("{controller}/cruise.F")]}]}})
    }

    fn with(mut base: Value, more: Value) -> Value {
        for (k, v) in more.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    #[test]
    fn a_model_split_in_two_runs_as_the_model_with_a_hold() {
        let dir = temp_dir("split");
        let whole = cruise_files(&dir);
        let composed = run_in(
            &dir,
            &with(
                json!({"format": 1, "run": {"t_end": 20},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        // The model in one file, the controller reading the speed through
        // a probe every step (ADR 0063 step 3), solved apart.
        let mut one = whole.clone();
        one["fusion"] = "off".into();
        one["controls"]["probes"] = json!([{"name": "sampled", "record": ["car.v"]}]);
        one["connections"][0]["from"] = "probe:sampled.car.v".into();
        let single = crate::run::run(&one.to_string(), &registry(), Some(20.0), None).unwrap();
        let (plant, controller) = (
            composed.model("plant").unwrap(),
            composed.model("controller").unwrap(),
        );
        assert_eq!(plant.rows(), 401);
        assert_eq!(
            bits(series(plant, "plant/car.v")),
            bits(series(&single, "car.v"))
        );
        for s in ["e", "I", "F"] {
            assert_eq!(
                bits(series(controller, &format!("controller/cruise.{s}"))),
                bits(series(&single, &format!("cruise.{s}"))),
                "cruise.{s}"
            );
        }
        // The loop is broken without delay: neither output depends on the
        // other at the same instant.
        assert!(composed
            .warnings
            .iter()
            .any(|w| w.starts_with("loop between models")));
        assert!(!composed
            .warnings
            .iter()
            .any(|w| w.contains("saw that model")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_use_replaces_the_files_of_uses_inside_it() {
        let dir = temp_dir("replace");
        cruise_files(&dir);
        // A heavier copy of the car, in a folder of its own.
        let mut heavy: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("car.ssbx.json")).unwrap())
                .unwrap();
        heavy["blocks"][1]["params"]["m"] = json!(2000.0);
        std::fs::create_dir_all(dir.join("copies")).unwrap();
        write(&dir.join("copies"), "car-2.ssbx.json", &heavy);
        let vehicle = |plant: Value| {
            with(
                json!({"format": 1, "blocks": [plant, use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            )
        };
        write(
            &dir,
            "vehicle.ssbx.json",
            &vehicle(use_of("plant", "car.ssbx.json")),
        );
        let top = |entry: Value| {
            json!({"format": 1, "run": {"t_end": 5}, "blocks": [entry],
                   "controls": {"probes": [{"name": "v", "record": ["vehicle/plant/car.v"]}]}})
        };
        let replaced = run_in(
            &dir,
            &top(json!({"name": "vehicle", "file": "vehicle.ssbx.json",
                        "files": {"plant": "copies/car-2.ssbx.json"}})),
        );
        // As a vehicle file using the copy, and as the car given m = 2000.
        let by_params = run_in(
            &dir,
            &top(json!({"name": "vehicle", "file": "vehicle.ssbx.json",
                        "params": {"plant/car.m": 2000.0}})),
        );
        let original = run_in(&dir, &top(use_of("vehicle", "vehicle.ssbx.json")));
        let v = |c: &Composed| bits(c.values("vehicle/plant/car.v").unwrap());
        assert_eq!(v(&replaced), v(&by_params));
        assert_ne!(v(&replaced), v(&original));
        // The vehicle file is left alone.
        assert!(std::fs::read_to_string(dir.join("vehicle.ssbx.json"))
            .unwrap()
            .contains("\"car.ssbx.json\""));
        // On a shared clock too, and from two levels up.
        write(
            &dir,
            "fleet.ssbx.json",
            &json!({"format": 1,
            "blocks": [{"name": "vehicle", "file": "vehicle.ssbx.json", "clock": "shared"}]}),
        );
        let deep = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 5},
                    "blocks": [{"name": "fleet", "file": "fleet.ssbx.json",
                                "files": {"vehicle/plant": "copies/car-2.ssbx.json"}}]}),
        );
        let deep_params = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 5},
                    "blocks": [{"name": "fleet", "file": "fleet.ssbx.json",
                                "params": {"vehicle/plant/car.m": 2000.0}}]}),
        );
        let fv = |c: &Composed| bits(c.values("fleet/vehicle/plant/car.v").unwrap());
        assert_eq!(fv(&deep), fv(&deep_params));
        // The nearer replacement wins over one written further down.
        let mut vehicle_b = vehicle(use_of("plant", "car.ssbx.json"));
        vehicle_b["blocks"][0]["files"] = json!({});
        write(
            &dir,
            "fleet.ssbx.json",
            &json!({"format": 1,
            "blocks": [{"name": "vehicle", "file": "vehicle.ssbx.json", "clock": "shared",
                        "files": {"plant": "car.ssbx.json"}}]}),
        );
        let nearer = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 5},
                    "blocks": [{"name": "fleet", "file": "fleet.ssbx.json",
                                "files": {"vehicle/plant": "copies/car-2.ssbx.json"}}]}),
        );
        assert_eq!(fv(&nearer), fv(&deep_params));
        // check lists them; the hash follows the file read.
        let c = check_in(
            &dir,
            &top(json!({"name": "vehicle", "file": "vehicle.ssbx.json",
                        "files": {"plant": "copies/car-2.ssbx.json"}})),
        );
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["replaced_files"]["vehicle/plant"], "copies/car-2.ssbx.json",
            "{c}"
        );
        assert!(c["models"][0]["file"]
            .as_str()
            .unwrap()
            .ends_with("car-2.ssbx.json"));
        let o = check_in(&dir, &top(use_of("vehicle", "vehicle.ssbx.json")));
        assert_ne!(c["model_hash"], o["model_hash"]);
        // Refused: a path that is not a used model, and files on a block.
        for (entry, at, says) in [
            (
                json!({"name": "vehicle", "file": "vehicle.ssbx.json", "files": {"wheel": "car.ssbx.json"}}),
                "blocks[0]",
                "not a use of a model file",
            ),
            (
                json!({"name": "vehicle", "file": "vehicle.ssbx.json", "files": {"plant/../x": "car.ssbx.json"}}),
                "blocks[0].files",
                "a path of used models' names",
            ),
            (
                json!({"name": "k", "type": "input.value", "files": {"plant": "car.ssbx.json"}}),
                "blocks[0].files",
                "only a use of a model file",
            ),
        ] {
            let c = check_in(&dir, &top(entry));
            assert_eq!(c["ok"], false, "{c}");
            let e = c["errors"][0].to_string();
            assert!(e.contains(at) && e.contains(says), "{e}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_used_model_is_edited_three_ways() {
        use crate::edit_use::{prepare, read, save, write, Edited, How};
        let dir = temp_dir("edit-use");
        cruise_files(&dir);
        write_file(&dir);
        let top_text = json!({"format": 1, "run": {"t_end": 5}, "blocks": [use_of("vehicle", "vehicle.ssbx.json")],
            "controls": {"probes": [{"name": "v", "record": ["vehicle/plant/car.v"]}]}});
        let top = || {
            let (mut f, _) = simsandbox_file::ModelFile::from_json(&top_text.to_string()).unwrap();
            f.folder = Some(dir.clone());
            f
        };
        let path = ["vehicle".to_string(), "plant".to_string()];
        let v = |f: &simsandbox_file::ModelFile| {
            let c = run_in(&dir, &serde_json::from_str(&f.to_json()).unwrap());
            bits(c.values("vehicle/plant/car.v").unwrap())
        };
        let mut heavier = top_text.clone();
        heavier["blocks"][0]["params"] = json!({"plant/car.m": 2000.0});
        let (heavier, _) = simsandbox_file::ModelFile::from_json(&heavier.to_string()).unwrap();
        let expected = v(&heavier);
        let before = v(&top());
        let disk = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        let (car, vehicle) = (disk("car.ssbx.json"), disk("vehicle.ssbx.json"));
        // The edit: the car's mass.
        let heavy = |t: &mut simsandbox_file::ModelFile| {
            let car = t.blocks.iter_mut().find(|b| b.name == "car").unwrap();
            car.params
                .insert("m".into(), simsandbox_file::ParamValue::Number(2000.0));
        };
        for how in [
            How::Original,
            How::NewCopy(dir.join("car-2.ssbx.json")),
            How::LocalCopy,
        ] {
            let mut file = top();
            let mut edited = Edited::new();
            let target = prepare(&mut file, Some(&dir), &mut edited, &path, how.clone()).unwrap();
            // Unedited, it runs as before.
            assert_eq!(v(&file), before, "{how:?}");
            let mut inside = read(&file, &edited, &target).unwrap();
            heavy(&mut inside);
            write(&mut file, &mut edited, &target, &inside).unwrap();
            assert_eq!(v(&file), expected, "{how:?}");
            // Nothing on disk changes until saved.
            assert_eq!(
                (disk("car.ssbx.json"), disk("vehicle.ssbx.json")),
                (car.clone(), vehicle.clone())
            );
            let text = file.to_json();
            match &how {
                How::Original => {
                    assert_eq!(
                        edited.keys().collect::<Vec<_>>(),
                        [&dir.join("car.ssbx.json")]
                    );
                    assert_eq!(text, top().to_json());
                }
                How::NewCopy(p) => {
                    assert_eq!(edited.keys().collect::<Vec<_>>(), [p]);
                    assert!(text.contains("\"plant\": \"car-2.ssbx.json\""), "{text}");
                }
                How::LocalCopy => {
                    assert!(edited.is_empty());
                    assert!(
                        text.contains("\"model\"") && !text.contains("vehicle.ssbx.json"),
                        "{text}"
                    );
                }
            }
            let written = save(&mut edited).unwrap();
            assert_eq!(v(&file), expected, "{how:?} saved");
            match how {
                How::Original => {
                    assert_ne!(disk("car.ssbx.json"), car);
                    std::fs::write(dir.join("car.ssbx.json"), &car).unwrap();
                }
                How::NewCopy(p) => {
                    assert_eq!(written, std::slice::from_ref(&p));
                    std::fs::remove_file(p).unwrap();
                }
                How::LocalCopy => assert!(written.is_empty()),
            }
        }
        // A name already taken is refused.
        let mut file = top();
        let e = prepare(
            &mut file,
            Some(&dir),
            &mut Edited::new(),
            &path,
            How::NewCopy(dir.join("car.ssbx.json")),
        );
        assert!(e.unwrap_err().contains("exists"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wires_drawn_between_used_models_link_them_across_clocks() {
        use crate::diagram::{apply_in, Edit, Scope};
        let dir = temp_dir("wire-uses");
        cruise_files(&dir);
        let r = registry();
        let wired = |shared: bool| {
            let clock = |v: Value| {
                if shared {
                    with(v, json!({"clock": "shared"}))
                } else {
                    v
                }
            };
            let (mut f, _) = simsandbox_file::ModelFile::from_json(
                &json!({"format": 1, "run": {"t_end": 5, "dt": 0.05}, "fusion": "off",
                        "blocks": [clock(use_of("plant", "car.ssbx.json")), clock(use_of("controller", "cruise.ssbx.json"))]})
                .to_string(),
            )
            .unwrap();
            let mut notes = Vec::new();
            for (from, to) in [
                ("plant.car.v", "controller.speed"),
                ("controller.cruise.F", "plant.force"),
            ] {
                let e = Edit::Connect {
                    from: from.into(),
                    to: to.into(),
                };
                notes.push(apply_in(&mut f, &Scope::Model, &r, &e).unwrap());
            }
            (f, notes)
        };
        // On their own clocks: linked through probes, as written by hand.
        let (linked, notes) = wired(false);
        assert!(notes[0].contains("through the probe"), "{notes:?}");
        assert_eq!(
            linked.connections[0].from,
            "probe:link_plant_car_v.plant/car.v"
        );
        assert_eq!(linked.connections[0].to, "controller/speed");
        let by_hand = run_in(
            &dir,
            &with(
                json!({"format": 1, "run": {"t_end": 5},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let drawn = run_in(&dir, &serde_json::from_str(&linked.to_json()).unwrap());
        assert_eq!(
            bits(drawn.values("plant/car.v").unwrap()),
            bits(by_hand.values("plant/car.v").unwrap())
        );
        // Disconnecting removes the link and the probe made for it.
        let mut f = linked.clone();
        apply_in(
            &mut f,
            &Scope::Model,
            &r,
            &Edit::Disconnect {
                to: "controller.speed".into(),
            },
        )
        .unwrap();
        assert_eq!(f.connections.len(), 1);
        assert_eq!(f.controls.probes.len(), 1);
        // On a shared clock: plain wires.
        let (shared, _) = wired(true);
        assert_eq!(shared.connections[0].from, "plant/car.v");
        assert!(shared.controls.probes.is_empty());
        let c = check_in(&dir, &serde_json::from_str(&shared.to_json()).unwrap());
        assert_eq!(c["ok"], true, "{c}");
        // Across clocks into a block that is not an input block: refused.
        let (mut f, _) = simsandbox_file::ModelFile::from_json(
            &json!({"format": 1, "blocks": [use_of("plant", "car.ssbx.json"),
                {"name": "odo", "type": "eq.equations", "equations": ["input v [m/s]", "state d [m]", "dd/dt = v"]}]})
            .to_string(),
        )
        .unwrap();
        let e = apply_in(
            &mut f,
            &Scope::Model,
            &r,
            &Edit::Connect {
                from: "plant.car.v".into(),
                to: "odo.v".into(),
            },
        );
        assert!(e.unwrap_err().contains("input block"));
        // Add model…: named after its file, kept relative to the folder.
        let (mut f, _) =
            simsandbox_file::ModelFile::from_json(r#"{"format": 1, "blocks": []}"#).unwrap();
        f.folder = Some(dir.clone());
        for _ in 0..2 {
            let e = Edit::AddUse {
                file: dir.join("car.ssbx.json").to_string_lossy().into_owned(),
                at: [10.0, 20.0],
            };
            apply_in(&mut f, &Scope::Model, &r, &e).unwrap();
        }
        let names: Vec<(&str, Option<&str>)> = f
            .blocks
            .iter()
            .map(|b| (b.name.as_str(), b.file.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("car", Some("car.ssbx.json")),
                ("car_2", Some("car.ssbx.json"))
            ]
        );
        assert_eq!(f.blocks[0].canvas, Some([10.0, 20.0]));
        let c = check_in(&dir, &serde_json::from_str(&f.to_json()).unwrap());
        assert_eq!(c["ok"], true, "{c}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_platoon_uses_one_model_defined_in_the_file() {
        let written = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap()
            .text;
        assert!(written.contains("\"models\"") && !written.contains("\"subsystems\""));
        // One clock, wired through its exposed input. (Its bits equal the
        // subsystem it was, tested before subsystems were removed.)
        let two = run_composition(written, &registry(), Some(20.0)).unwrap();
        assert_eq!(two.models.len(), 1, "one clock");
        assert!(two.values("car3/control.I").is_some());
        // check lists the definition, its ports and its uses.
        let c = check(written, &registry());
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(c["definitions"]["follower"]["inputs"], json!(["lead"]));
        assert_eq!(
            c["definitions"]["follower"]["uses"],
            json!(["car1", "car2", "car3"])
        );
        // The session runs it as the app would.
        let s = crate::Session::load_in("p", written, &registry(), None).unwrap();
        assert!(s.signals().resolve("car3/car.v").is_some());
    }

    #[test]
    fn make_model_moves_a_selection_into_a_definition_that_runs_the_same() {
        use crate::diagram::{apply_in, Edit, Scope};
        let r = registry();
        let text = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap()
            .text;
        let (mut f, _) = simsandbox_file::ModelFile::from_json(text).unwrap();
        let note = apply_in(
            &mut f,
            &Scope::Model,
            &r,
            &Edit::GroupModel {
                blocks: vec!["car".into()],
            },
        )
        .unwrap();
        assert!(note.contains("model1"), "{note}");
        assert!(f.subsystems.is_empty());
        let def = simsandbox_file::compose::read_definition("model1", &f.models["model1"]).unwrap();
        // Crossing wires became an input block and a declared output.
        assert!(
            def.blocks
                .iter()
                .any(|b| b.name == "F" && b.type_name == "input.value"),
            "{def:?}"
        );
        assert_eq!(def.outputs[0].from, "car.v");
        let use_ = f
            .blocks
            .iter()
            .find(|b| b.model.as_deref() == Some("model1"))
            .unwrap();
        assert_eq!(use_.clock, None, "wired: it shares the clock");
        let written = f.to_json();
        let one = crate::run::run(text, &r, Some(20.0), None).unwrap();
        let two = run_composition(&written, &r, Some(20.0)).unwrap();
        assert_eq!(two.models.len(), 1);
        let inst = &use_.name;
        assert_eq!(
            bits(series(&one, "car.v")),
            bits(two.values(&format!("{inst}/car.v")).unwrap())
        );
        // Not wired, it is put on the user's clock explicitly.
        let (mut g, _) = simsandbox_file::ModelFile::from_json(text).unwrap();
        apply_in(
            &mut g,
            &Scope::Model,
            &r,
            &Edit::GroupModel {
                blocks: vec!["car".into(), "cruise".into()],
            },
        )
        .unwrap();
        let use_ = g.blocks.iter().find(|b| b.model.is_some()).unwrap();
        assert_eq!(use_.clock, Some(simsandbox_file::UseClock::Shared));
        let three = run_composition(&g.to_json(), &r, Some(20.0)).unwrap();
        assert_eq!(
            bits(series(&one, "car.v")),
            bits(three.values(&format!("{}/car.v", use_.name)).unwrap())
        );
    }

    #[test]
    fn a_used_file_moves_into_the_model_and_back_out() {
        use crate::edit_use::{definition_to_file, file_to_definition, save, Edited};
        let dir = temp_dir("move-def");
        cruise_files(&dir);
        let text = with(
            json!({"format": 1, "run": {"t_end": 5},
               "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
            cruise_links("plant", "controller"),
        );
        let before = run_in(&dir, &text);
        let v = |c: &Composed| bits(c.values("plant/car.v").unwrap());
        let (mut f, _) = simsandbox_file::ModelFile::from_json(&text.to_string()).unwrap();
        f.folder = Some(dir.clone());
        // In: the file becomes a definition named after it.
        let key = file_to_definition(&mut f, Some(&dir), "plant").unwrap();
        assert_eq!(key, "car");
        assert_eq!(f.blocks[0].model.as_deref(), Some("car"));
        let inside = run_in(&dir, &serde_json::from_str(&f.to_json()).unwrap());
        assert_eq!(v(&inside), v(&before));
        // Out: written to a new file, kept apart until saved.
        let mut edited = Edited::new();
        let to = dir.join("car-out.ssbx.json");
        definition_to_file(&mut f, Some(&dir), &mut edited, "car", &to).unwrap();
        assert!(f.models.is_empty());
        assert_eq!(f.blocks[0].file.as_deref(), Some("car-out.ssbx.json"));
        assert!(!to.exists() && edited.contains_key(&to));
        let outside = run_in(&dir, &serde_json::from_str(&f.to_json()).unwrap());
        assert_eq!(v(&outside), v(&before));
        save(&mut edited).unwrap();
        assert!(to.exists());
        assert!(definition_to_file(&mut f, Some(&dir), &mut edited, "car", &to).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_platoon_as_one_array_block_runs_as_its_three_uses() {
        use simsandbox_core::sim::Simulator;
        // ADR 0069: the followers as arrays (the same bits), each car reading the one
        // ahead with prev(v); the first reads the leader through v's edge.
        let platoon = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap()
            .text;
        let whole: Value = serde_json::from_str(platoon).unwrap();
        let leader = whole["blocks"][0].clone();
        let cars = json!({"name": "cars", "type": "eq.equations", "method": {"name": "rk4"},
            "equations": [
                "grid k = 0 .. 3, 3",
                "input lead [m/s]",
                "param kp [N*s/m] = 400",
                "param ki [N/m] = 40",
                "param c [N*s/m] = 50",
                "param mass[k] [kg] = 1000",
                "state v[k] [m/s] = 0",
                "state I[k] [m] = 0",
                "edge v: k- = lead",
                "e = prev(v) - v",
                "F = kp*e + ki*I [N]",
                "dv/dt = (F - c*v)/mass",
                "dI/dt = e"],
            "params": {"kp": "kp", "ki": "ki", "mass[1]": 1500.0, "mass[2]": 2500.0}});
        let arrays = json!({"format": 1, "run": {"dt": 0.05}, "params": whole["params"],
            "blocks": [leader, cars], "connections": [{"from": "leader.v", "to": "cars.lead"}]});
        let one = simsandbox_file::load(&arrays.to_string(), &registry()).unwrap();
        // One parameter per car, as files and sliders name them.
        assert_eq!(one.model.param(one.ids["cars"], "mass[2]").unwrap(), 2500.0);
        assert_eq!(one.model.param(one.ids["cars"], "mass[0]").unwrap(), 1000.0);
        let three = simsandbox_file::compose::load_merged(platoon, &registry(), None).unwrap();
        let (a, b) = (one.ids["cars"], three.ids.clone());
        let mut s1 = Simulator::new(one.model, 0.0).unwrap();
        let mut s3 = Simulator::new(three.model, 0.0).unwrap();
        for step in 0..400 {
            s1.step(0.05).unwrap();
            s3.step(0.05).unwrap();
            let v = s1.output(a, "v").unwrap().to_vec();
            for (k, car) in ["car1", "car2", "car3"].iter().enumerate() {
                let w = s3.output(b[&format!("{car}/car")], "v").unwrap()[0];
                assert_eq!(
                    v[k].to_bits(),
                    w.to_bits(),
                    "step {step}, {car}: {} vs {w}",
                    v[k]
                );
            }
        }
        // next() reads the car behind; past the end, insulated: itself.
        let tail = json!({"format": 1, "blocks": [{"name": "s", "type": "eq.equations",
            "equations": ["grid k = 0 .. 3, 3", "param x[k] = 1", "y = next(x) - x"],
            "params": {"x[0]": 1.0, "x[1]": 4.0, "x[2]": 9.0}}]});
        let l = simsandbox_file::load(&tail.to_string(), &registry()).unwrap();
        let id = l.ids["s"];
        let sim = Simulator::new(l.model, 0.0).unwrap();
        assert_eq!(sim.output(id, "y").unwrap(), [3.0, 5.0, 0.0]);
    }

    #[test]
    fn models_using_models_take_inputs_probes_and_run_files() {
        use crate::run::{model_hash, run_probes, run_with};
        use crate::session::InputEvent;
        let dir = temp_dir("step5");
        cruise_files(&dir);
        write(
            &dir,
            "sum.ssbx.json",
            &json!({"format": 1, "run": {"dt": 0.1},
            "blocks": [{"name": "u", "type": "input.value"},
                       {"name": "acc", "type": "eq.equations", "method": {"name": "euler"},
                        "equations": ["input u", "state z", "dz/dt = u"], "params": {"z0": 0}}],
            "connections": [{"from": "u.y", "to": "acc.u"}]}),
        );
        let top = with(
            json!({"format": 1, "run": {"t_end": 1, "inputs": [{"t": 0.3, "set": "slow/u.y", "to": 2.0}]},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json"),
                              use_of("slow", "sum.ssbx.json")]}),
            cruise_links("plant", "controller"),
        );
        let mut top = top;
        top["controls"]["probes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "rec", "record": ["slow/acc.z"]}));
        let text = top.to_string();
        simsandbox_core::registry::in_folder(Some(&dir), || {
            // The schedule reaches an input block inside a use, by path.
            let r = run_with(&text, &registry(), None, None, Vec::new()).unwrap();
            let z = series(&r, "slow/acc.z");
            assert_eq!(*z.last().unwrap(), 2.0 * 0.7, "{z:?}");
            // So does a recorded track, as a run file replays it.
            let track = vec![InputEvent {
                t: 0.5,
                key: "slow/u.y".into(),
                value: 1.0,
            }];
            let r = run_with(&text, &registry(), None, None, track).unwrap();
            assert!((series(&r, "slow/acc.z").last().unwrap() - (0.4 + 0.5)).abs() < 1e-12);
            // The merged run gives each model's values at its own instants.
            let apart = run_composition(&text, &registry(), None).unwrap();
            let t: Vec<f64> = r.index[0].1.clone();
            let v = series(&r, "plant/car.v");
            let own = apart.model("plant").unwrap();
            for (k, ti) in own.index[0].1.iter().enumerate() {
                let j = t.iter().position(|x| (x - ti).abs() < 1e-9).unwrap();
                assert_eq!(
                    v[j].to_bits(),
                    series(own, "plant/car.v")[k].to_bits(),
                    "t = {ti}"
                );
            }
            // A probe inside a use records, and writes its file.
            let out = dir.join("probes");
            std::fs::create_dir_all(&out).unwrap();
            let p = run_probes(
                &text,
                &registry(),
                None,
                None,
                Vec::new(),
                &["rec".into()],
                Some(&out),
            )
            .unwrap();
            let (spec, h) = p.probes.iter().find(|(s, _)| s.name == "rec").unwrap();
            assert_eq!(spec.record, ["slow/acc.z"]);
            assert!(h.read().len() > 5);
            // A run file's hash covers the files used: changing one changes it.
            let before = model_hash(&text).unwrap();
            let mut sum: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("sum.ssbx.json")).unwrap())
                    .unwrap();
            sum["blocks"][1]["params"]["z0"] = json!(1.0);
            write(&dir, "sum.ssbx.json", &sum);
            assert_ne!(model_hash(&text).unwrap(), before);
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_experiment_runs_each_point_as_the_model_alone_on_any_number_of_threads() {
        use crate::run::run;
        let ex = |name: &str| -> Value {
            serde_json::from_str(
                EXAMPLES
                    .iter()
                    .find(|e| e.name.contains(name))
                    .unwrap()
                    .text,
            )
            .unwrap()
        };
        let r = registry();
        let mut cruise = ex("Cruise control");
        cruise["sample"] = json!({"design": "list", "params": [{"param": "cruise.kp"}],
                                  "values": [[200.0], [400.0], [800.0]]});
        let text = cruise.to_string();
        let one = crate::run::run_sampled(&text, &r, Some(5.0), Some(1)).unwrap();
        let three = crate::run::run_sampled(&text, &r, Some(5.0), Some(3)).unwrap();
        assert_eq!(one.rows(), 3);
        let finals = |x: &crate::run::Results| bits(series(x, "car.v"));
        assert_eq!(
            finals(&one),
            finals(&three),
            "the same on any number of threads"
        );
        // Each point is the model run alone with that value.
        for (k, kp) in [200.0, 400.0, 800.0].iter().enumerate() {
            let mut alone = ex("Cruise control");
            alone["blocks"][1]["params"]["kp"] = json!(kp);
            let a = run(&alone.to_string(), &r, Some(5.0), None).unwrap();
            assert_eq!(
                series(&one, "car.v")[k].to_bits(),
                series(&a, "car.v").last().unwrap().to_bits()
            );
        }
        assert_eq!(one.index[1].0, "cruise.kp");
        // Points from a CSV, columns named by path.
        let dir = temp_dir("experiment");
        std::fs::write(
            dir.join("gains.csv"),
            "cruise.kp,cruise.ki\n200,10\n400,20\n",
        )
        .unwrap();
        let mut from_file = ex("Cruise control");
        from_file["sample"] = json!({"design": "file", "file": "gains.csv"});
        let f = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::run::run_sampled(&from_file.to_string(), &r, Some(2.0), None)
        })
        .unwrap();
        assert_eq!(f.rows(), 2);
        assert_eq!(f.index[2], ("cruise.ki".to_string(), vec![10.0, 20.0]));
        std::fs::remove_dir_all(&dir).unwrap();
        // A declared parameter of a model that uses models, by its name.
        let mut platoon = ex("Platoon");
        platoon["sample"] =
            json!({"design": "list", "params": [{"param": "kp"}], "values": [[300.0], [900.0]]});
        let p = crate::run::run_sampled(&platoon.to_string(), &r, Some(3.0), None).unwrap();
        let v = series(&p, "car3/car.v");
        assert!(v[0] != v[1], "{v:?}");
        // Seeds: common by default, so equal points agree; per point and
        // replicates draw apart.
        let mut grazing = ex("grazing");
        grazing["sample"] =
            json!({"design": "list", "params": [{"param": "size"}], "values": [[40.0], [40.0]]});
        let count = |g: &Value| {
            series(
                &crate::run::run_sampled(&g.to_string(), &r, Some(20.0), None).unwrap(),
                "sheep.count",
            )
            .to_vec()
        };
        let c = count(&grazing);
        assert_eq!(c[0], c[1]);
        grazing["sample"]["seeds"] = json!("per point");
        let c = count(&grazing);
        assert_ne!(c[0], c[1], "{c:?}");
        grazing["sample"]["seeds"] = json!("common");
        grazing["sample"]["replicates"] = json!(2);
        let g = crate::run::run_sampled(&grazing.to_string(), &r, Some(20.0), None).unwrap();
        assert_eq!(g.index[1].0, "replicate");
        let c = series(&g, "sheep.count");
        assert_eq!(
            (c[0], c[1]),
            (c[2], c[3]),
            "replicate j draws alike at every point"
        );
        assert_ne!(c[0], c[1], "{c:?}");
    }

    #[test]
    fn a_points_ids_decide_its_seed_not_its_place() {
        // ADR 0070 step 4: with ids from the points file, a point keeps its
        // random draws however the file is ordered; without, its place
        // decides them, as before.
        let r = registry();
        let mut grazing: Value = serde_json::from_str(
            EXAMPLES
                .iter()
                .find(|e| e.name.contains("grazing"))
                .unwrap()
                .text,
        )
        .unwrap();
        grazing["sample"] = json!({"design": "file", "file": "points.csv", "seeds": "per point"});
        let dir = temp_dir("point-ids");
        let rows = |csv: &str| {
            std::fs::write(dir.join("points.csv"), csv).unwrap();
            simsandbox_core::registry::in_folder(Some(&dir), || {
                crate::run::run_sampled(&grazing.to_string(), &r, Some(20.0), None)
            })
            .unwrap()
        };
        let by = |x: &crate::run::Results, column: &str| -> Vec<(u64, u64)> {
            let ids = &x.index.iter().find(|(n, _)| n == column).unwrap().1;
            let mut pairs: Vec<(u64, u64)> = ids
                .iter()
                .zip(series(x, "sheep.count"))
                .map(|(i, c)| (*i as u64, c.to_bits()))
                .collect();
            pairs.sort_unstable();
            pairs
        };
        let ahead = rows("id,size\n7,40\n3,40\n12,40\n");
        let behind = rows("size,id\n40,12\n40,7\n40,3\n");
        assert_eq!(ahead.index[1].0, "id");
        assert_eq!(by(&ahead, "id"), by(&behind, "id"));
        // Three equal points drawing apart: the seeds are per point.
        let counts = by(&ahead, "id");
        assert!(
            counts[0].1 != counts[1].1 || counts[1].1 != counts[2].1,
            "{counts:?}"
        );
        // Without ids, a point's place is its id: equal to before.
        let places = rows("size\n40\n40\n40\n");
        assert!(places.index.iter().all(|(n, _)| n != "id"));
        // Ids are whole, from 0, each once.
        for (csv, says) in [
            ("id,size\n1,40\n1,40\n", "given twice"),
            ("id,size\n1.5,40\n", "whole number"),
            ("id,size\n-1,40\n", "whole number"),
        ] {
            std::fs::write(dir.join("points.csv"), csv).unwrap();
            let e = simsandbox_core::registry::in_folder(Some(&dir), || {
                crate::run::run_sampled(&grazing.to_string(), &r, Some(1.0), None)
            })
            .unwrap_err();
            assert!(e.contains(says), "{csv}: {e}");
        }
        // The points file in the run hash: `a_study_gives_the_ids_its_points_file_gives`.
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_experiment_reduces_each_run_to_named_values() {
        let ex = |name: &str| -> Value {
            serde_json::from_str(
                EXAMPLES
                    .iter()
                    .find(|e| e.name.contains(name))
                    .unwrap()
                    .text,
            )
            .unwrap()
        };
        let r = registry();
        let mut cruise = ex("Cruise control");
        cruise["sample"] = json!({"design": "list", "params": [{"param": "cruise.kp"}],
            "values": [[200.0], [800.0]],
            "reduce": {"v_max": "max(car.v)", "v_end": "final(car.v)",
                       "settle": "first_time(abs(cruise.e) < 0.5)", "e_mean": "mean(cruise.e)"}});
        let x = crate::run::run_sampled(&cruise.to_string(), &r, Some(30.0), None).unwrap();
        // The rows hold the reductions, in name order.
        let names: Vec<&str> = x.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["e_mean", "settle", "v_end", "v_max"]);
        for (k, kp) in [200.0, 800.0].into_iter().enumerate() {
            // The run of that point alone.
            let mut alone = ex("Cruise control");
            alone["blocks"][1]["params"]["kp"] = json!(kp);
            let run = &crate::run::run(&alone.to_string(), &r, Some(30.0), None).unwrap();
            let v = series(run, "car.v");
            let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(series(&x, "v_max")[k], max);
            assert_eq!(series(&x, "v_end")[k], *v.last().unwrap());
            let e = series(run, "cruise.e");
            let t = &run.index[0].1;
            let first = (0..e.len())
                .find(|&i| e[i].abs() < 0.5)
                .map(|i| t[i])
                .unwrap();
            assert_eq!(series(&x, "settle")[k], first);
        }
        // A firmer controller settles sooner.
        let settle = series(&x, "settle");
        assert!(settle[1] < settle[0], "{settle:?}");
        // By path, in a model using models.
        let mut platoon = ex("Platoon");
        platoon["sample"] = json!({"design": "list", "params": [{"param": "kp"}], "values": [[300.0], [900.0]],
            "reduce": {"lag": "max(abs(leader.v - car3/car.v))"}});
        let p = crate::run::run_sampled(&platoon.to_string(), &r, Some(20.0), None).unwrap();
        assert_eq!(p.outputs.len(), 1);
        assert!(series(&p, "lag").iter().all(|v| *v > 0.0));
        // Refused with where and why.
        cruise["sample"]["reduce"] = json!({"bad": "max(car.w)"});
        let e = crate::run::run_sampled(&cruise.to_string(), &r, Some(1.0), None).unwrap_err();
        assert!(
            e.contains("sample.reduce.bad") && e.contains("car.w"),
            "{e}"
        );
    }

    #[test]
    fn exposed_inputs_are_wired_where_a_model_is_used() {
        // ADR 0074: an input exposed from a block's port, with no input
        // block, runs as the input block did, and has no default.
        let r = registry();
        let gain = |exposed: bool| {
            let mut d = json!({"blocks": [
                {"name": "g", "type": "eq.equations", "equations": ["input x [m]", "param a = 2", "y = a*x [m]"]}],
              "outputs": [{"name": "y", "from": "g.y"}]});
            if exposed {
                d["inputs"] = json!([{"name": "u", "to": ["g.x"], "unit": "m"}]);
            } else {
                d["blocks"].as_array_mut().unwrap().insert(
                    0,
                    json!({"name": "u", "type": "input.value", "settings": {"unit": "m"}}),
                );
                d["connections"] = json!([{"from": "u.y", "to": "g.x"}]);
            }
            d
        };
        let top = |exposed: bool, wired: bool| {
            let mut t = json!({"format": 1, "run": {"dt": 0.1, "t_end": 1.0},
                "models": {"gain": gain(exposed)},
                "blocks": [{"name": "src", "type": "eq.equations", "method": {"name": "rk4"},
                            "equations": ["param r [1/s] = 1", "state s [m] = 1", "ds/dt = r*s [m/s]"]},
                           {"name": "a", "model": "gain"}]});
            if wired {
                t["connections"] = json!([{"from": "src.s", "to": "a/u"}]);
            }
            t.to_string()
        };
        let value = |text: &str| {
            let res = crate::run::run(text, &r, None, None).unwrap();
            res.outputs
                .iter()
                .find(|o| o.name == "a/g.y")
                .unwrap()
                .values
                .clone()
        };
        assert_eq!(
            bits(&value(&top(true, true))),
            bits(&value(&top(false, true)))
        );
        // Unwired: refused, naming it.
        let e = crate::run::run(&top(true, false), &r, None, None).unwrap_err();
        assert!(e.contains("'u' is not wired"), "{e}");
        // Alone: refused, saying why.
        let mut alone = gain(true);
        alone["format"] = json!(1);
        let e = crate::run::run(&alone.to_string(), &r, None, None).unwrap_err();
        assert!(e.contains("no default"), "{e}");
        // A study takes it as an input, as an input block.
        let study = json!({"format": 1, "models": {"gain": gain(true)},
            "blocks": [{"name": "k", "type": "input.value", "params": {"y": 3.0}, "settings": {"unit": "m"}},
                {"name": "s", "type": "study", "model": "gain",
                 "sample": {"design": "list", "params": [{"param": "g.a"}], "values": [[1.0]]}}],
            "connections": [{"from": "k.y", "to": "s.u"}]});
        let check = crate::describe::check(&study.to_string(), &r);
        assert_eq!(check["ok"], true, "{check}");
        // Its names: apart, and feeding block inputs.
        for (inputs, says) in [
            (json!([{"name": "g", "to": ["g.x"]}]), "apart"),
            (json!([{"name": "u", "to": []}]), "at least one"),
            (
                json!([{"name": "u", "to": ["nope.x"]}]),
                "one of its blocks",
            ),
        ] {
            let mut d = gain(true);
            d["format"] = json!(1);
            d["inputs"] = inputs;
            let e = simsandbox_file::ModelFile::from_json(&d.to_string())
                .unwrap_err()
                .to_string();
            assert!(e.contains(says), "{e}");
        }
    }

    #[test]
    fn models_defined_in_a_file_are_found_used_and_refused() {
        let gain = |k: f64| {
            json!({"blocks": [
            {"name": "u", "type": "input.value"},
            {"name": "g", "type": "eq.equations", "equations": ["input x", format!("y = {k}*x")]}],
            "connections": [{"from": "u.y", "to": "g.x"}],
            "outputs": [{"name": "y", "from": "g.y"}]})
        };
        let src = json!({"name": "src", "type": "eq.equations", "method": {"name": "euler"},
                         "equations": ["state s", "ds/dt = 1"], "params": {"s0": 1}});
        let model = |models: Value, blocks: Value, conns: Value| {
            json!({"format": 1, "run": {"dt": 0.1, "t_end": 0.5}, "models": models,
                   "blocks": blocks, "connections": conns})
            .to_string()
        };
        // Found in the file; a definition inside a definition finds the
        // outer ones; wired, it shares the user's clock.
        let twice = json!({"blocks": [{"name": "a", "model": "gain"}, {"name": "b", "model": "gain"},
                                      {"name": "u", "type": "input.value"}],
                           "connections": [{"from": "u.y", "to": "a/u"}, {"from": "a/y", "to": "b/u"}],
                           "outputs": [{"name": "y", "from": "b/y"}]});
        let text = model(
            json!({"gain": gain(2.0), "twice": twice}),
            json!([src, {"name": "t", "model": "twice"}]),
            json!([{"from": "src.s", "to": "t/u"}]),
        );
        let r = run_composition(&text, &registry(), None).unwrap();
        assert_eq!(r.models.len(), 1);
        let s = r.values("src.s").unwrap();
        let y = r.values("t/b/g.y").unwrap();
        for (s, y) in s.iter().zip(y) {
            assert_eq!(*y, 4.0 * s);
        }
        // An inner definition of the same name wins over the outer one.
        let mut shadowed = twice.clone();
        shadowed["models"] = json!({"gain": gain(3.0)});
        let text = model(
            json!({"gain": gain(2.0), "twice": shadowed}),
            json!([src, {"name": "t", "model": "twice"}]),
            json!([{"from": "src.s", "to": "t/u"}]),
        );
        let r = run_composition(&text, &registry(), None).unwrap();
        assert_eq!(r.values("t/b/g.y").unwrap()[0], 9.0);
        // Not wired, with states, it runs on its own clock at its run.dt.
        let stepped = json!({"run": {"dt": 0.25}, "blocks": [{"name": "c", "type": "eq.equations",
            "method": {"name": "euler"}, "equations": ["state x", "dx/dt = 1"], "params": {"x0": 0}}]});
        let text = json!({"format": 1, "run": {"dt": 0.1, "t_end": 0.5}, "models": {"g": stepped},
            "blocks": [src, {"name": "k", "model": "g"}]})
        .to_string();
        let r = run_composition(&text, &registry(), None).unwrap();
        assert_eq!(r.models.len(), 2);
        assert_eq!(r.model("k").unwrap().index[0].1[1], 0.25);
        // Refused: an unknown name, a model using itself, controls other
        // than probes.
        let looped = json!({"blocks": [{"name": "again", "model": "me"}]});
        for (text, says) in [
            (
                model(
                    json!({}),
                    json!([{"name": "t", "model": "nothing"}]),
                    json!([]),
                ),
                "no model named 'nothing'",
            ),
            (
                model(
                    json!({"me": looped}),
                    json!([{"name": "t", "model": "me"}]),
                    json!([]),
                ),
                "uses itself",
            ),
            (
                model(
                    json!({"g": {"blocks": [], "controls": {"views": []}}}),
                    json!([src]),
                    json!([]),
                ),
                "only probes",
            ),
        ] {
            let c = check(&text, &registry());
            assert_eq!(c["ok"], false, "{c}");
            assert!(c["errors"][0].to_string().contains(says), "{c}");
        }
    }

    #[test]
    fn a_use_sets_start_states_inside_by_path() {
        let def = json!({"blocks": [{"name": "c", "type": "eq.equations", "method": {"name": "euler"},
                                     "equations": ["state x", "dx/dt = 1"], "params": {"x0": 0}}]});
        let text = json!({"format": 1, "run": {"dt": 0.1, "t_end": 0.2}, "models": {"count": def},
            "blocks": [{"name": "a", "model": "count", "starts": {"c": {"state": [5.0]}}},
                       {"name": "b", "model": "count"}]})
        .to_string();
        let r = run_composition(&text, &registry(), None).unwrap();
        assert_eq!(r.values("a/c.x").unwrap()[0], 5.0);
        assert_eq!(r.values("b/c.x").unwrap()[0], 0.0);
    }

    #[test]
    fn a_nested_use_runs_as_its_models_side_by_side() {
        // vehicle.ssbx.json uses the car and the controller and links them;
        // the top level uses the vehicle and adds a block of its own.
        let dir = temp_dir("nested");
        cruise_files(&dir);
        write(
            &dir,
            "vehicle.ssbx.json",
            &with(
                json!({"format": 1,
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let flat = run_in(
            &dir,
            &with(
                json!({"format": 1, "run": {"t_end": 5},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let top = json!({"format": 1, "run": {"t_end": 5, "dt": 0.1},
            "blocks": [
                {"name": "watch", "type": "input.value", "settings": {"unit": "m/s"}},
                {"name": "odo", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["input v [m/s]", "state d [m]", "dd/dt = v"], "params": {"d0": 0}},
                use_of("vehicle", "vehicle.ssbx.json")],
            "connections": [
                {"from": "watch.y", "to": "odo.v"},
                {"from": "probe:v.vehicle/plant/car.v", "to": "watch"}],
            "controls": {"probes": [{"name": "v", "record": ["vehicle/plant/car.v"]}]}});
        let nested = run_in(&dir, &top);
        let names: Vec<&str> = nested.models.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [".", "vehicle/plant", "vehicle/controller"]);
        assert_eq!(
            bits(nested.values("vehicle/plant/car.v").unwrap()),
            bits(flat.values("plant/car.v").unwrap())
        );
        assert_eq!(
            bits(nested.values("vehicle/controller/cruise.F").unwrap()),
            bits(flat.values("controller/cruise.F").unwrap())
        );
        // The top level's own block reads the car every 0.1 s.
        let watch = nested.values("watch.y").unwrap();
        let v = flat.values("plant/car.v").unwrap();
        assert_eq!(watch.len(), 51);
        assert_eq!(watch[10], v[20]);
        let report = check_in(&dir, &top);
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["links"][2]["to"], "watch");
        assert!(report["models"][1]["file"]
            .as_str()
            .unwrap()
            .ends_with("car.ssbx.json"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two models in `dir`: `a` counts (`count.x`, and `count.y` showing
    /// what it reads into `w`), `b` integrates (`sum.z` from `u`), Euler,
    /// so every value can be computed by hand.
    fn pair(dir: &Path, dt_a: f64, dt_b: f64) -> Value {
        write(
            dir,
            "a.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": dt_a},
            "blocks": [
                {"name": "w", "type": "input.value", "settings": {"unit": "m*s"}},
                {"name": "count", "type": "eq.equations", "method": {"name": "euler"}, "equations": [
                    "input w [m*s]", "param r [m/s] = 1", "state x [m]", "dx/dt = r", "y = w"],
                 "params": {"x0": 0}}],
            "connections": [{"from": "w.y", "to": "count.w"}]}),
        );
        write(
            dir,
            "b.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": dt_b},
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "sum", "type": "eq.equations", "method": {"name": "euler"}, "equations": [
                    "input u [m]", "state z [m*s]", "dz/dt = u"], "params": {"z0": 0}}],
            "connections": [{"from": "u.y", "to": "sum.u"}]}),
        );
        json!({
            "format": 1,
            "blocks": [use_of("a", "a.ssbx.json"), use_of("b", "b.ssbx.json")],
            "connections": [
                {"from": "probe:p.a/count.x", "to": "b/u"},
                {"from": "probe:q.b/sum.z", "to": "a/w"}],
            "controls": {"probes": [
                {"name": "p", "record": ["a/count.x"]},
                {"name": "q", "record": ["b/sum.z"]}]}})
    }

    #[test]
    fn two_clocks_hold_each_others_samples() {
        let dir = temp_dir("clocks");
        let mut c = pair(&dir, 0.1, 0.3);
        c["run"] = json!({"t_end": 3});
        let composed = run_in(&dir, &c);
        assert_eq!(composed.tick, 0.1);
        let (a, b) = (composed.model("a").unwrap(), composed.model("b").unwrap());
        assert_eq!((a.rows(), b.rows()), (31, 11));
        // By hand: x after n steps of a; z after k steps of b, each adding
        // 0.3 × the x it took at 0.3 k.
        let mut x = vec![0.0f64];
        for _ in 0..30 {
            x.push(x.last().unwrap() + 0.1 * 1.0);
        }
        let mut z = vec![0.0f64];
        for k in 0..10 {
            z.push(z[k] + 0.3 * x[3 * k]);
        }
        assert_eq!(bits(series(a, "a/count.x")), bits(&x));
        assert_eq!(bits(series(b, "b/sum.z")), bits(&z));
        // a reads b's latest sample at or before each of its steps.
        let y: Vec<f64> = (0..=30).map(|n| z[n / 3]).collect();
        assert_eq!(bits(series(a, "a/count.y")), bits(&y));
        assert_eq!(series(b, "b/u.y")[1], x[3]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn models_stepping_on_several_cores_give_the_same_bits() {
        let dir = temp_dir("cores");
        let mut c = pair(&dir, 0.1, 0.1);
        c["run"] = json!({"t_end": 3});
        let with = |p: Parallel| {
            simsandbox_core::registry::in_folder(Some(&dir), || {
                run_composition_with(&c.to_string(), &registry(), None, p)
            })
            .unwrap()
        };
        let all = |r: &Composed| -> Vec<u64> {
            r.models
                .iter()
                .flat_map(|(_, m)| m.outputs.iter().flat_map(|o| bits(&o.values)))
                .collect()
        };
        let apart = with(Parallel::Never);
        assert_eq!(all(&with(Parallel::Always)), all(&apart));
        assert_eq!(all(&with(Parallel::Auto)), all(&apart));
        // Unlinked: two groups, each run on a thread of its own from start
        // to end.
        let mut unlinked = c.clone();
        unlinked["connections"] = json!([]);
        unlinked["controls"] = json!({});
        let text = unlinked.to_string();
        let loaded = simsandbox_core::registry::in_folder(Some(&dir), || {
            load_composition(&text, &registry(), None)
        })
        .unwrap();
        assert_eq!(Plan::of(&loaded).groups(2), [vec![0], vec![1]]);
        let with = |p: Parallel| {
            simsandbox_core::registry::in_folder(Some(&dir), || {
                run_composition_with(&text, &registry(), None, p)
            })
            .unwrap()
        };
        let apart = with(Parallel::Never);
        assert_eq!(all(&with(Parallel::Always)), all(&apart));
        assert_eq!(all(&with(Parallel::Auto)), all(&apart));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn groups_follow_links_and_keep_their_numbering() {
        let dir = temp_dir("groups");
        let c = pair(&dir, 0.1, 0.3);
        let text = c.to_string();
        let loaded = simsandbox_core::registry::in_folder(Some(&dir), || {
            load_composition(&text, &registry(), None)
        })
        .unwrap();
        let plan = Plan::of(&loaded);
        // Linked both ways: one group, all of it.
        assert_eq!(plan.groups(2), [vec![0, 1]]);
        // Feeding each other: one part.
        assert_eq!(plan.parts(2), [vec![0, 1]]);
        let (all, global) = plan.part(&[0, 1]);
        assert_eq!(all.links.len(), plan.links.len());
        assert_eq!(global, (0..plan.links.len()).collect::<Vec<_>>());
        assert_eq!(all.order, plan.order);
        // One model alone keeps its links to the other, that end outside.
        let (one, _) = plan.part(&[1]);
        assert_eq!(one.links.len(), plan.links.len());
        assert!(one
            .links
            .iter()
            .all(|l| (l.from == OUTSIDE) != (l.to == OUTSIDE)));
        assert_eq!(one.names, [plan.names[1].clone()]);
        assert!(one.probes.iter().all(|p| p.domain == 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clocks_that_rarely_meet_run_long_and_stop_at_the_end() {
        // A tick of 10⁻¹⁶ s: 64-bit counts would overflow after about
        // 1 800 s.
        let dir = temp_dir("rarely");
        let mut c = pair(&dir, 0.1, 0.3333333333333333);
        c["run"] = json!({"t_end": 3000});
        let composed = run_in(&dir, &c);
        let (a, b) = (composed.model("a").unwrap(), composed.model("b").unwrap());
        assert_eq!((a.rows(), b.rows()), (30_001, 9_001));
        // b's step at 1 (three of 0.3333333333333333) comes just before a's,
        // so it takes a's sample at 0.9.
        assert_eq!(series(b, "b/u.y")[3], series(a, "a/count.x")[9]);
        // The last step at or before the end, never past it.
        let mut c = pair(&dir, 0.4, 0.2);
        c["run"] = json!({"t_end": 1.0});
        let composed = run_in(&dir, &c);
        let t = &composed.model("a").unwrap().index[0].1;
        assert_eq!((t.len(), *t.last().unwrap()), (3, 0.8));
        assert_eq!(composed.model("b").unwrap().rows(), 6);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_uses_of_one_file_draw_differently() {
        let dir = temp_dir("seeds");
        let mut c = pair(&dir, 0.1, 0.2);
        c["blocks"] = json!([
            use_of("a", "a.ssbx.json"),
            use_of("b", "b.ssbx.json"),
            use_of("b2", "b.ssbx.json")
        ]);
        c["run"] = json!({"seed": 7});
        let built = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::compose::load_composition(&c.to_string(), &registry(), None)
        })
        .unwrap();
        let seeds: Vec<u64> = built
            .domains
            .iter()
            .map(|d| d.loaded.model.seed())
            .collect();
        assert_ne!(seeds[1], seeds[2]);
        // From the top level's seed: another seed, other draws.
        c["run"] = json!({"seed": 8});
        let other = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::compose::load_composition(&c.to_string(), &registry(), None)
        })
        .unwrap();
        assert_ne!(other.domains[1].loaded.model.seed(), seeds[1]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_lists_models_links_and_the_tick() {
        let dir = temp_dir("compose-check");
        let c = pair(&dir, 0.1, 0.2);
        let report = check_in(&dir, &c);
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["tick"], 0.1);
        assert_eq!(report["order"], json!(["a", "b"]));
        assert_eq!(report["models"][1]["ticks_per_step"], "2");
        assert_eq!(report["models"][1]["inputs"], json!(["b/u"]));
        assert!(report["models"][0]["outputs"]
            .as_array()
            .unwrap()
            .contains(&json!("a/count.x")));
        assert_eq!(report["links"][0]["to"], "b/u");
        let via = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::describe::check(&c.to_string(), &registry())
        });
        assert_eq!(via["composition"], true);
        // The hash follows the used files, parameter values and clocks.
        let hash = |v: &Value| {
            check_in(&dir, v)["model_hash"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let before = hash(&c);
        let mut faster = c.clone();
        faster["blocks"][1]["clock"] = json!({"dt": 0.1});
        assert_ne!(hash(&faster), before);
        let mut set = c.clone();
        set["blocks"][1]["params"] = json!({"sum.z0": 2});
        assert_ne!(hash(&set), before);
        let a = std::fs::read_to_string(dir.join("a.ssbx.json")).unwrap();
        std::fs::write(
            dir.join("a.ssbx.json"),
            a.replace("r [m/s] = 1", "r [m/s] = 2"),
        )
        .unwrap();
        assert_ne!(hash(&c), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_model_using_files_refuses() {
        let dir = temp_dir("compose-refuse");
        let c = pair(&dir, 0.1, 0.2);
        let refused = |edit: &dyn Fn(&mut Value), says: &str| {
            let mut v = c.clone();
            edit(&mut v);
            let report = check_in(&dir, &v);
            assert_eq!(report["ok"], false, "{v}");
            let message = report["errors"][0].to_string();
            assert!(message.contains(says), "{message}");
        };
        refused(
            &|v| v["connections"][0]["from"] = "a/count.x".into(),
            "only through probes",
        );
        refused(
            &|v| v["connections"][0]["to"] = "b/sum".into(),
            "not an input block",
        );
        refused(
            &|v| v["connections"][0]["from"] = "probe:p.a/count.y".into(),
            "does not record",
        );
        refused(
            &|v| v["blocks"][1]["file"] = "missing.ssbx.json".into(),
            "missing.ssbx.json",
        );
        refused(
            &|v| v["blocks"][1]["name"] = "b/c".into(),
            "letters, digits",
        );
        refused(
            &|v| v["blocks"][1]["params"] = json!({"sum.nope": 1}),
            "has no parameter 'nope'",
        );
        refused(
            &|v| v["blocks"][1]["type"] = "input.value".into(),
            "blocks[1].type",
        );
        refused(&|v| v["blocks"][0]["clock"] = "fast".into(), "a clock is");
        refused(
            &|v| v["controls"]["probes"][0]["record"] = json!(["a/count.x", "b/sum.z"]),
            "records from several models",
        );
        refused(
            &|v| v["connections"][0]["to"] = "a/w".into(),
            "its own model",
        );
        refused(
            &|v| {
                v["connections"] = json!([{"from": "probe:r.a/count.y", "to": "b/u"}]);
                v["controls"]["probes"] = json!([{"name": "r", "record": ["a/count.y"]}]);
            },
            "units must match",
        );
        // A file that uses itself, through another.
        write(
            &dir,
            "loop.ssbx.json",
            &json!({"format": 1, "blocks": [use_of("again", "top.ssbx.json")]}),
        );
        write(
            &dir,
            "top.ssbx.json",
            &json!({"format": 1, "blocks": [use_of("inner", "loop.ssbx.json")]}),
        );
        let text = std::fs::read_to_string(dir.join("top.ssbx.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let report = check_in(&dir, &v);
        assert!(
            report["errors"][0].to_string().contains("uses itself"),
            "{report}"
        );
        // The app does not open such a model yet; it says so.
        let e = simsandbox_file::load(&c.to_string(), &registry())
            .err()
            .unwrap();
        assert!(e.to_string().contains("uses model files"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn shared(name: &str, file: &str) -> Value {
        json!({"name": name, "file": file, "clock": "shared"})
    }

    /// The car and the controller used on the top level's clock, wired
    /// directly into each other's input blocks.
    fn wired(plant: &str, controller: &str) -> Value {
        json!([
            {"from": format!("{plant}/car.v"), "to": format!("{controller}/speed")},
            {"from": format!("{controller}/cruise.F"), "to": format!("{plant}/force")}])
    }

    #[test]
    fn shared_clock_uses_run_as_the_model_in_one_file() {
        let dir = temp_dir("shared");
        let whole = cruise_files(&dir);
        let top = json!({"format": 1, "run": {"dt": 0.05, "t_end": 20},
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "connections": wired("plant", "controller")});
        let one = crate::run::run(&whole.to_string(), &registry(), Some(20.0), None).unwrap();
        let composed = run_in(&dir, &top);
        // One clock: the top level's, with both models flattened into it.
        assert_eq!(composed.models.len(), 1);
        for (inner, alone) in [
            ("plant/car.v", "car.v"),
            ("controller/cruise.F", "cruise.F"),
        ] {
            assert_eq!(
                bits(composed.values(inner).unwrap()),
                bits(series(&one, alone)),
                "{inner}"
            );
        }
        // Fused across the boundary, the input blocks dropped.
        let built = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::compose::load_composition(&top.to_string(), &registry(), None)
        })
        .unwrap();
        let groups = &built.domains[0].loaded.groups;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].blocks, ["plant/car", "controller/cruise"]);
        assert!(!built.domains[0].loaded.ids.contains_key("plant/force"));
        // One level deeper: a vehicle file wiring the two, used shared.
        write(
            &dir,
            "vehicle.ssbx.json",
            &json!({"format": 1,
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "connections": wired("plant", "controller")}),
        );
        let nested = run_in(
            &dir,
            &json!({"format": 1, "run": {"dt": 0.05, "t_end": 20},
            "blocks": [shared("vehicle", "vehicle.ssbx.json")]}),
        );
        assert_eq!(
            bits(nested.values("vehicle/plant/car.v").unwrap()),
            bits(series(&one, "car.v"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_shared_use_beside_one_on_its_own_clock() {
        // The plant on the top level's clock, the controller on its own,
        // linked by probes: as both on their own clocks.
        let dir = temp_dir("mixed");
        cruise_files(&dir);
        let apart = run_in(
            &dir,
            &with(
                json!({"format": 1, "run": {"t_end": 20},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let mixed = run_in(
            &dir,
            &with(
                json!({"format": 1, "run": {"dt": 0.05, "t_end": 20}, "fusion": "off",
                   "blocks": [shared("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let names: Vec<&str> = mixed.models.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [".", "controller"]);
        for name in ["plant/car.v", "controller/cruise.F"] {
            assert_eq!(
                bits(mixed.values(name).unwrap()),
                bits(apart.values(name).unwrap()),
                "{name}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A model with no states reading speeds recorded at irregular times
    /// and doubling them.
    fn data_files(dir: &Path) -> Value {
        std::fs::write(
            dir.join("speed.csv"),
            "t,v [m/s]\n0,1\n0.3,2\n0.35,4\n1.2,8\n",
        )
        .unwrap();
        let data = json!({"format": 1,
            "blocks": [
                {"name": "data", "type": "input.file", "settings": {"file": "speed.csv"}},
                {"name": "f", "type": "eq.equations", "equations": ["input v [m/s]", "y = 2*v [m/s]"]}],
            "connections": [{"from": "data.v", "to": "f.v"}]});
        write(dir, "data.ssbx.json", &data);
        data
    }

    fn index_t(r: &Results) -> Vec<f64> {
        r.index.iter().find(|(n, _)| n == "t").unwrap().1.clone()
    }

    #[test]
    fn a_model_driven_by_data_steps_at_its_samples_and_ends_with_them() {
        // ADR 0067 step 4: no dt, no t_end: the data's times.
        let dir = temp_dir("data-times");
        let data = data_files(&dir);
        let r = registry();
        let run = |t_end: Option<f64>| {
            simsandbox_core::registry::in_folder(Some(&dir), || {
                crate::run::run(&data.to_string(), &r, t_end, None)
            })
            .unwrap()
        };
        let all = run(None);
        assert_eq!(index_t(&all), [0.0, 0.3, 0.35, 1.2]);
        assert_eq!(series(&all, "f.y"), [2.0, 4.0, 8.0, 16.0]);
        // An end given stops before the last sample.
        assert_eq!(index_t(&run(Some(1.0))), [0.0, 0.3, 0.35]);
        // It runs in time; the app pauses at the last sample.
        let s = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::Session::load("data", &data.to_string(), &r)
        })
        .unwrap();
        assert_eq!(s.signals().index(), "t");
        let loaded = simsandbox_file::load_in(&data.to_string(), &r, Some(&dir)).unwrap();
        assert_eq!(crate::run::data_end(&loaded.model), Some(1.2));
        // A step of its own written: every dt, as before.
        let mut stepped = data.clone();
        stepped["run"] = json!({"dt": 0.5, "t_end": 1.0});
        let r2 = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::run::run(&stepped.to_string(), &r, None, None)
        })
        .unwrap();
        assert_eq!(index_t(&r2), [0.0, 0.5, 1.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_used_model_on_data_is_evaluated_at_its_own_samples() {
        let dir = temp_dir("data-use");
        data_files(&dir);
        let r = registry();
        // Beside a model stepping every 0.5 s, to 1.5 s.
        let top = json!({"format": 1, "run": {"dt": 0.5, "t_end": 1.5},
            "blocks": [use_of("d", "data.ssbx.json"),
                {"name": "clock", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["state s [s]", "ds/dt = 1"]}]});
        let apart = run_in(&dir, &top);
        let d = apart.model("d").unwrap();
        assert_eq!(index_t(d), [0.0, 0.3, 0.35, 1.2]);
        assert_eq!(series(d, "d/f.y"), [2.0, 4.0, 8.0, 16.0]);
        // As one model: every instant of either, the same values.
        let one = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::run::run(&top.to_string(), &r, None, None)
        })
        .unwrap();
        assert_eq!(index_t(&one), [0.0, 0.3, 0.35, 0.5, 1.0, 1.2, 1.5]);
        assert_eq!(series(&one, "d/f.y"), [2.0, 4.0, 8.0, 8.0, 8.0, 16.0, 16.0]);
        // Only data drives it: it ends at the last sample.
        let alone = json!({"format": 1, "blocks": [use_of("d", "data.ssbx.json")]});
        let apart = run_in(&dir, &alone);
        assert_eq!(index_t(apart.model("d").unwrap()), [0.0, 0.3, 0.35, 1.2]);
        let one = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::run::run(&alone.to_string(), &r, None, None)
        })
        .unwrap();
        assert_eq!(index_t(&one), [0.0, 0.3, 0.35, 1.2]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_use_steps_at_listed_times() {
        // clock: {"times"} (ADR 0067 step 4): Euler from one to the next.
        let dir = temp_dir("listed-times");
        write(
            &dir,
            "decay.ssbx.json",
            &json!({"format": 1, "run": {"dt": 0.1},
                "blocks": [{"name": "p", "type": "eq.equations", "method": {"name": "euler"},
                            "equations": ["state x", "dx/dt = -x"], "params": {"x0": 1}}]}),
        );
        let r = registry();
        let top = json!({"format": 1, "run": {"t_end": 2},
            "blocks": [{"name": "u", "file": "decay.ssbx.json", "clock": {"times": [0.2, 0.5, 1.0]}}]});
        let mut x = 1.0f64;
        let mut expect = vec![x];
        for dt in [0.2, 0.5 - 0.2, 1.0 - 0.5] {
            x += dt * -x;
            expect.push(x);
        }
        let apart = run_in(&dir, &top);
        let u = apart.model("u").unwrap();
        assert_eq!(index_t(u), [0.0, 0.2, 0.5, 1.0]);
        assert_eq!(bits(series(u, "u/p.x")), bits(&expect));
        let one = simsandbox_core::registry::in_folder(Some(&dir), || {
            crate::run::run(&top.to_string(), &r, None, None)
        })
        .unwrap();
        assert_eq!(index_t(&one), [0.0, 0.2, 0.5, 1.0]);
        assert_eq!(bits(series(&one, "u/p.x")), bits(&expect));
        // The node says so; times must ascend.
        let mut bad = top.clone();
        bad["blocks"][0]["clock"] = json!({"times": [0.5, 0.2]});
        let e = check_in(&dir, &bad)["errors"][0].to_string();
        assert!(e.contains("ascending"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_shared_use_reads_its_data_from_its_own_folder() {
        let dir = temp_dir("shared-data");
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join("parts/push.csv"), "t,F [N]\n0,1\n1,3\n").unwrap();
        write(
            &dir,
            "parts/push.ssbx.json",
            &json!({"format": 1,
            "blocks": [{"name": "data", "type": "input.file", "settings": {"file": "push.csv"}}]}),
        );
        let composed = run_in(
            &dir,
            &json!({"format": 1, "run": {"dt": 0.5, "t_end": 2},
            "blocks": [
                shared("push", "parts/push.ssbx.json"),
                {"name": "sum", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["input u [N]", "state z [N*s]", "dz/dt = u"], "params": {"z0": 0}}],
            "connections": [{"from": "push/data.F", "to": "sum.u"}]}),
        );
        assert_eq!(
            composed.values("push/data.F").unwrap(),
            [1.0, 1.0, 3.0, 3.0, 3.0]
        );
        assert_eq!(composed.values("sum.z").unwrap(), [0.0, 0.5, 1.0, 2.5, 4.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_shared_use_refuses() {
        let dir = temp_dir("shared-refuse");
        cruise_files(&dir);
        let top = json!({"format": 1,
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "connections": wired("plant", "controller")});
        assert_eq!(check_in(&dir, &top)["ok"], true, "{}", check_in(&dir, &top));
        let refused = |edit: &dyn Fn(&mut Value), says: &str| {
            let mut v = top.clone();
            edit(&mut v);
            let report = check_in(&dir, &v);
            assert_eq!(report["ok"], false, "{v}");
            let message = report["errors"][0].to_string();
            assert!(message.contains(says), "{message}");
        };
        refused(
            &|v| v["connections"][1]["to"] = "plant/car".into(),
            "one of its input blocks",
        );
        refused(
            &|v| {
                v["connections"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"from": "controller/cruise.F", "to": "plant/force"}))
            },
            "wired twice",
        );
        // A model on its own clock inside a shared use takes a probe link.
        write(
            &dir,
            "own.ssbx.json",
            &json!({"format": 1, "blocks": [use_of("plant", "car.ssbx.json")]}),
        );
        refused(
            &|v| {
                v["blocks"][0] = shared("deep", "own.ssbx.json");
                v["connections"] =
                    json!([{"from": "controller/cruise.F", "to": "deep/plant/force"}]);
            },
            "a model on its own clock takes a probe link",
        );
        refused(
            &|v| {
                v["blocks"][0] = shared("deep", "own.ssbx.json");
                v["connections"] = json!([{"from": "deep/car.v", "to": "controller/speed"}]);
            },
            "no output 'car.v'",
        );
        write(
            &dir,
            "self.ssbx.json",
            &json!({"format": 1, "blocks": [shared("me", "self.ssbx.json")]}),
        );
        refused(
            &|v| v["blocks"][0] = shared("me", "self.ssbx.json"),
            "uses itself",
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn run_alone(dir: &Path, file: &str, t_end: f64) -> Results {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        crate::run::run(&text, &registry(), Some(t_end), None).unwrap()
    }

    #[test]
    fn any_file_can_be_used_on_a_shared_clock() {
        let dir = temp_dir("any-shared");
        let whole = cruise_files(&dir);
        // A file whose models run on their own clocks, linked by probes.
        write(
            &dir,
            "apart.ssbx.json",
            &with(
                json!({"format": 1,
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let alone = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 20},
            "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")],
            "connections": cruise_links("plant", "controller")["connections"],
            "controls": cruise_links("plant", "controller")["controls"]}),
        );
        let used = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 20},
            "blocks": [shared("vehicle", "apart.ssbx.json")]}),
        );
        for name in ["plant/car.v", "controller/cruise.F"] {
            assert_eq!(
                bits(used.values(&format!("vehicle/{name}")).unwrap()),
                bits(alone.values(name).unwrap()),
                "{name}"
            );
        }
        // A file with a probe feeding its own blocks (ADR 0063 step 3).
        let mut held = whole.clone();
        held["fusion"] = "off".into();
        held["controls"]["probes"] =
            json!([{"name": "sampled", "record": ["car.v"], "every": {"seconds": 1}}]);
        held["connections"][0]["from"] = "probe:sampled.car.v".into();
        write(&dir, "held.ssbx.json", &held);
        let used = run_in(
            &dir,
            &json!({"format": 1, "run": {"dt": 0.05, "t_end": 20}, "fusion": "off",
            "blocks": [shared("c", "held.ssbx.json")]}),
        );
        let direct = run_alone(&dir, "held.ssbx.json", 20.0);
        for name in ["car.v", "cruise.e"] {
            assert_eq!(
                bits(used.values(&format!("c/{name}")).unwrap()),
                bits(series(&direct, name)),
                "{name}"
            );
        }
        // A file with groups.
        let mut grouped = whole.clone();
        grouped["fusion"] = "off".into();
        grouped["groups"] = json!([{"blocks": ["car", "cruise"], "method": {"name": "rk4"}}]);
        write(&dir, "grouped.ssbx.json", &grouped);
        let top = json!({"format": 1, "run": {"dt": 0.05, "t_end": 20}, "fusion": "off",
            "blocks": [shared("g", "grouped.ssbx.json")]});
        let used = run_in(&dir, &top);
        let direct = run_alone(&dir, "grouped.ssbx.json", 20.0);
        assert_eq!(
            bits(used.values("g/car.v").unwrap()),
            bits(series(&direct, "car.v"))
        );
        let built = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::compose::load_composition(&top.to_string(), &registry(), None)
        })
        .unwrap();
        assert_eq!(
            built.domains[0].loaded.groups[0].blocks,
            ["g/car", "g/cruise"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wires_and_values_reach_any_depth() {
        let dir = temp_dir("deep");
        let whole = cruise_files(&dir);
        // The vehicle uses both on its clock but wires nothing; the top
        // level wires them two levels down and sets the car's mass there.
        write(
            &dir,
            "parts.ssbx.json",
            &json!({"format": 1,
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")]}),
        );
        let top = |mass: Value| {
            json!({"format": 1, "run": {"dt": 0.05, "t_end": 20},
            "params": {"mass": {"value": 1200, "unit": "kg"}},
            "blocks": [{"name": "vehicle", "file": "parts.ssbx.json", "clock": "shared",
                        "params": {"plant/car.m": mass}}],
            "connections": wired("vehicle/plant", "vehicle/controller")})
        };
        let mut heavier = whole.clone();
        heavier["blocks"][0]["params"]["m"] = 1200.into();
        let one = crate::run::run(&heavier.to_string(), &registry(), Some(20.0), None).unwrap();
        for mass in [json!(1200), json!("mass")] {
            let deep = run_in(&dir, &top(mass.clone()));
            assert_eq!(
                bits(deep.values("vehicle/plant/car.v").unwrap()),
                bits(series(&one, "car.v")),
                "mass {mass}"
            );
        }
        // A formula is live on a shared clock: the mass follows the
        // declared parameter.
        let built = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::compose::load_composition(
                &top(json!("mass")).to_string(),
                &registry(),
                None,
            )
        })
        .unwrap();
        assert!(built.domains[0]
            .loaded
            .params
            .derived
            .iter()
            .any(|d| d.block == "vehicle/plant/car" && d.param == "m"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn values_for_models_on_their_own_clocks() {
        // Paths and formulas into a model on its own clock are evaluated
        // once, from the declared parameters where they are written.
        let dir = temp_dir("own-values");
        cruise_files(&dir);
        let car: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("car.ssbx.json")).unwrap())
                .unwrap();
        let mut declared = car.clone();
        declared["params"] = json!({"m": {"value": 1000, "unit": "kg"}});
        declared["blocks"][1]["params"]["m"] = "m".into();
        write(&dir, "car-m.ssbx.json", &declared);
        let run = |plant: Value, params: Value| {
            run_in(
                &dir,
                &with(
                    json!({"format": 1, "run": {"t_end": 10}, "params": params,
                       "blocks": [plant, use_of("controller", "cruise.ssbx.json")]}),
                    cruise_links("plant", "controller"),
                ),
            )
            .values("plant/car.v")
            .unwrap()
            .to_vec()
        };
        let numbers = run(
            json!({"name": "plant", "file": "car.ssbx.json", "params": {"car.m": 1500}}),
            json!({}),
        );
        let half = json!({"half": {"value": 750, "unit": "kg"}});
        // A block parameter set by a formula of this file's parameters.
        let block = run(
            json!({"name": "plant", "file": "car.ssbx.json", "params": {"car.m": "2*half"}}),
            half.clone(),
        );
        // A declared parameter of the used file set by a formula.
        let decl = run(
            json!({"name": "plant", "file": "car-m.ssbx.json", "params": {"m": "2*half"}}),
            half.clone(),
        );
        assert_eq!(bits(&block), bits(&numbers));
        assert_eq!(bits(&decl), bits(&numbers));
        // A path into a model the used file uses.
        write(
            &dir,
            "wrap.ssbx.json",
            &with(
                json!({"format": 1, "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        let wrapped = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 10}, "params": half,
            "blocks": [{"name": "w", "file": "wrap.ssbx.json", "params": {"plant/car.m": "2*half"}}]}),
        );
        assert_eq!(
            bits(wrapped.values("w/plant/car.v").unwrap()),
            bits(&numbers)
        );
        // A formula naming what is not declared where it is written.
        let report = check_in(
            &dir,
            &json!({"format": 1,
            "blocks": [{"name": "plant", "file": "car.ssbx.json", "params": {"car.m": "2*nothing"}}]}),
        );
        assert!(
            report["errors"][0]
                .to_string()
                .contains("'nothing' is not a parameter"),
            "{report}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `a` counting time every 0.1 s (from [`pair`]) and `double.ssbx.json`,
    /// a model without states: `y = 2·u`, `u` an input block.
    fn on_data_files(dir: &Path) {
        pair(dir, 0.1, 0.3);
        write(
            dir,
            "double.ssbx.json",
            &json!({"format": 1,
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "f", "type": "eq.equations", "equations": [
                    "input u [m]", "param k = 2", "y = k*u [m]"]}],
            "connections": [{"from": "u.y", "to": "f.u"}]}),
        );
    }

    /// x of `a` after n steps of 0.1 s, as Euler computes it.
    fn counted(n: usize) -> Vec<f64> {
        let mut x = vec![0.0f64];
        for _ in 0..n {
            x.push(x.last().unwrap() + 0.1 * 1.0);
        }
        x
    }

    #[test]
    fn a_model_without_states_is_evaluated_when_a_link_delivers() {
        let dir = temp_dir("on-data");
        on_data_files(&dir);
        let feeds = |every: Value| {
            json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("d", "double.ssbx.json")],
            "connections": [{"from": "probe:p.a/count.x", "to": "d/u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": every}]}})
        };
        let composed = run_in(&dir, &feeds(json!({"seconds": 0.3})));
        let x = counted(12);
        let d = composed.model("d").unwrap();
        // At the start and at each delivery, at the delivery's time.
        let t: Vec<f64> = [0, 3, 6, 9, 12]
            .iter()
            .map(|&n| composed.model("a").unwrap().index[0].1[n])
            .collect();
        assert_eq!(d.index[0].1, t);
        let y: Vec<f64> = [0, 3, 6, 9, 12].iter().map(|&n| 2.0 * x[n]).collect();
        assert_eq!(bits(series(d, "d/f.y")), bits(&y));
        let report = check_in(&dir, &feeds(json!({"seconds": 0.3})));
        assert_eq!(report["models"][1]["on_new_data"], true, "{report}");
        assert_eq!(report["models"][1]["dt"], Value::Null);
        // The top level's own blocks without states run on new data too.
        let top = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "f", "type": "eq.equations", "equations": ["input u [m]", "y = 3*u [m]"]},
                use_of("a", "a.ssbx.json")],
            "connections": [{"from": "u.y", "to": "f.u"}, {"from": "probe:p.a/count.x", "to": "u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.6}}]}});
        let composed = run_in(&dir, &top);
        let y: Vec<f64> = [0, 6, 12].iter().map(|&n| 3.0 * x[n]).collect();
        assert_eq!(bits(composed.values("f.y").unwrap()), bits(&y));
        // Given a clock, it is evaluated at each of its steps instead.
        let mut clocked = feeds(json!({"steps": 1}));
        clocked["blocks"][1]["clock"] = json!({"dt": 0.2});
        let composed = run_in(&dir, &clocked);
        let d = composed.model("d").unwrap();
        assert_eq!(d.rows(), 7);
        let y: Vec<f64> = (0..=6).map(|k| 2.0 * x[2 * k]).collect();
        assert_eq!(bits(series(d, "d/f.y")), bits(&y));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Linked one way, models run ahead of the models they feed, in parts
    /// on threads of their own (ADR 0066 step 6): the same bits as all
    /// meeting at every instant.
    #[test]
    fn models_linked_one_way_run_ahead_with_the_same_bits() {
        let dir = temp_dir("one-way");
        on_data_files(&dir);
        write(
            &dir,
            "add.ssbx.json",
            &json!({"format": 1,
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "w", "type": "input.value", "settings": {"unit": "m*s"}},
                {"name": "f", "type": "eq.equations", "equations": [
                    "input u [m]", "input w [m*s]", "param c [1/s] = 0.25", "y = u + c*w [m]"]}],
            "connections": [{"from": "u.y", "to": "f.u"}, {"from": "w.y", "to": "f.w"}]}),
        );
        // a → b, a and b → d (on new data), d → e; and f, g feeding each
        // other beside them. Clocks 0.1, 0.3, 0.2, 0.3, 0.1 s.
        let mut e = use_of("e", "b.ssbx.json");
        e["clock"] = json!({"dt": 0.2});
        let model = json!({"format": 1, "run": {"t_end": 6.1},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("b", "b.ssbx.json"),
                use_of("d", "add.ssbx.json"), e,
                use_of("f", "a.ssbx.json"), use_of("g", "b.ssbx.json")],
            "connections": [
                {"from": "probe:p.a/count.x", "to": "b/u"},
                {"from": "probe:p.a/count.x", "to": "d/u"},
                {"from": "probe:q.b/sum.z", "to": "d/w"},
                {"from": "probe:r.d/f.y", "to": "e/u"},
                {"from": "probe:s.f/count.x", "to": "g/u"},
                {"from": "probe:t.g/sum.z", "to": "f/w"}],
            "controls": {"probes": [
                {"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.3}},
                {"name": "q", "record": ["b/sum.z"]},
                {"name": "r", "record": ["d/f.y"]},
                {"name": "s", "record": ["f/count.x"]},
                {"name": "t", "record": ["g/sum.z"], "every": {"steps": 2}}]}});
        let text = model.to_string();
        let run = |parallel, split| {
            simsandbox_core::registry::in_folder(Some(&dir), || {
                run_in_parts(&text, &registry(), None, parallel, &Keep::default(), split)
            })
            .unwrap()
        };
        let loaded = simsandbox_core::registry::in_folder(Some(&dir), || {
            load_composition(&text, &registry(), None)
        })
        .unwrap();
        let plan = Plan::of(&loaded);
        let names = &plan.names;
        let parts: Vec<Vec<&str>> = plan
            .parts(names.len())
            .iter()
            .map(|p| p.iter().map(|&u| names[u].as_str()).collect())
            .collect();
        assert_eq!(
            parts,
            [vec!["a"], vec!["b"], vec!["d"], vec!["e"], vec!["f", "g"]]
        );
        let together = run(Parallel::Never, false);
        for (parallel, lag) in [
            (Parallel::Never, 0),
            (Parallel::Auto, 0),
            (Parallel::Always, 200),
        ] {
            // With feeders lagging, the parts they feed must wait for them.
            LAG.store(lag, std::sync::atomic::Ordering::Relaxed);
            let apart = run(parallel, true);
            LAG.store(0, std::sync::atomic::Ordering::Relaxed);
            for ((name, one), (_, other)) in together.models.iter().zip(&apart.models) {
                assert_eq!(bits(&one.index[0].1), bits(&other.index[0].1), "{name} t");
                for (o, p) in one.outputs.iter().zip(&other.outputs) {
                    assert_eq!(bits(&o.values), bits(&p.values), "{}", o.name);
                }
            }
            assert_eq!(together.warnings, apart.warnings);
        }
        // d answers when either feeder delivers, so it ran.
        assert!(together.model("e").unwrap().rows() > 1);
        assert!(series(together.model("d").unwrap(), "d/f.y")
            .iter()
            .any(|&y| y > 0.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_model_on_new_data_answers_in_the_same_instant() {
        // a → d (every 0.3 s) → a's w: a shows d's answer at the instant
        // it delivered, and holds it, with no delay.
        let dir = temp_dir("on-data-loop");
        on_data_files(&dir);
        let mut double: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("double.ssbx.json")).unwrap())
                .unwrap();
        double["blocks"][1]["equations"] = json!(["input u [m]", "param k [s] = 2", "y = k*u"]);
        write(&dir, "double.ssbx.json", &double);
        let composed = run_in(
            &dir,
            &json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("d", "double.ssbx.json")],
            "connections": [
                {"from": "probe:p.a/count.x", "to": "d/u"},
                {"from": "probe:q.d/f.y", "to": "a/w"}],
            "controls": {"probes": [
                {"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.3}},
                {"name": "q", "record": ["d/f.y"]}]}}),
        );
        let x = counted(12);
        let w: Vec<f64> = (0..=12).map(|n| 2.0 * x[3 * (n / 3)]).collect();
        assert_eq!(bits(composed.values("a/count.y").unwrap()), bits(&w));
        // Keeping less (ADR 0076), d evaluated again within instants.
        let model = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("d", "double.ssbx.json")],
            "connections": [
                {"from": "probe:p.a/count.x", "to": "d/u"},
                {"from": "probe:q.d/f.y", "to": "a/w"}],
            "controls": {"probes": [
                {"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.3}},
                {"name": "q", "record": ["d/f.y"]}]}});
        for every in [1, 2, 5] {
            keeps_the_same(&dir, &model, None, every);
        }
        keeps_the_same(&dir, &model, Some(vec!["a/count.y".into(), "d".into()]), 3);
        // Two models on new data in a loop: d is evaluated again within an
        // instant once e answers, its row for the instant taken back.
        write(
            &dir,
            "add.ssbx.json",
            &json!({"format": 1,
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "w", "type": "input.value", "settings": {"unit": "m*s"}},
                {"name": "f", "type": "eq.equations", "equations": [
                    "input u [m]", "input w [m*s]", "param c [1/s] = 0.25", "y = u + c*w [m]"]}],
            "connections": [{"from": "u.y", "to": "f.u"}, {"from": "w.y", "to": "f.w"}]}),
        );
        let looped = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("d", "add.ssbx.json"),
                use_of("e", "double.ssbx.json")],
            "connections": [
                {"from": "probe:p.a/count.x", "to": "d/u"},
                {"from": "probe:q.e/f.y", "to": "d/w"},
                {"from": "probe:r.d/f.y", "to": "e/u"}],
            "controls": {"probes": [
                {"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.3}},
                {"name": "q", "record": ["e/f.y"]},
                {"name": "r", "record": ["d/f.y"]}]}});
        for every in [1, 2, 3] {
            keeps_the_same(&dir, &looped, None, every);
        }
        assert!(!composed
            .warnings
            .iter()
            .any(|w| w.contains("saw that model")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_use_runs_its_model_as_written_and_a_study_sweeps_it() {
        let dir = temp_dir("on-data-sweep");
        on_data_files(&dir);
        let double: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("double.ssbx.json")).unwrap())
                .unwrap();
        let sample =
            json!({"design": "grid", "params": [{"param": "f.k", "range": [1, 3], "points": 3}]});
        write(&dir, "sweep.ssbx.json", &double);
        // A use of it runs it plain (k = 2): only a study sweeps (ADR 0071).
        let top = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("s", "sweep.ssbx.json")],
            "connections": [{"from": "probe:p.a/count.x", "to": "s/u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.6}}]}});
        let composed = run_in(&dir, &top);
        let x = counted(12);
        let s = composed.model("s").unwrap();
        assert_eq!(series(s, "s/f.y").last(), Some(&(2.0 * x[12])));
        // It feeds links like any model.
        let mut feeding = top.clone();
        feeding["blocks"][0] = use_of("a", "a.ssbx.json");
        feeding["controls"]["probes"] = json!([
            {"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.6}},
            {"name": "q", "record": ["s/f.y"]}]);
        assert_eq!(check_in(&dir, &feeding)["ok"], true);
        // A study block sweeps it: over k = 1, 2, 3 at the latest delivery.
        let study = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"),
                {"name": "x", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "s", "type": "study", "file": "sweep.ssbx.json", "sample": sample}],
            "connections": [{"from": "probe:p.a/count.x", "to": "x"}, {"from": "x.y", "to": "s.u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.6}}]}});
        let composed = run_in(&dir, &study);
        let top = composed.model(".").unwrap();
        let y = series(top, "s.f_y");
        let expect: Vec<f64> = [1.0, 2.0, 3.0].iter().map(|k| k * x[12]).collect();
        assert_eq!(bits(&y[y.len() - 3..]), bits(&expect));
        assert_eq!(series(top, "s.f_k")[..3], [1.0, 2.0, 3.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_model_with_nothing_in_time_is_evaluated_once_in_link_order() {
        // features → model → post-processing, no states anywhere (ADR 0067).
        let dir = temp_dir("pipeline");
        on_data_files(&dir);
        let composed = run_in(
            &dir,
            &json!({"format": 1,
            "blocks": [
                {"name": "x", "type": "input.value", "settings": {"unit": "m", "min": "0", "max": "10"}, "params": {"y": 1.5}},
                use_of("first", "double.ssbx.json"),
                use_of("second", "double.ssbx.json")],
            "connections": [
                {"from": "probe:p.x.y", "to": "first/u"},
                {"from": "probe:q.first/f.y", "to": "second/u"}],
            "controls": {"probes": [
                {"name": "p", "record": ["x.y"]},
                {"name": "q", "record": ["first/f.y"]}]}}),
        );
        assert_eq!(composed.model("second").unwrap().rows(), 1);
        assert_eq!(composed.values("first/f.y").unwrap(), [3.0]);
        assert_eq!(composed.values("second/f.y").unwrap(), [6.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uses_are_nodes_on_the_diagram() {
        use crate::diagram::{Diagram, NodeKind};
        let dir = temp_dir("use-nodes");
        cruise_files(&dir);
        let diagram = |v: &Value| {
            let (file, _) = simsandbox_file::ModelFile::from_json(&v.to_string()).unwrap();
            simsandbox_core::registry::in_folder(Some(&dir), || {
                let mut d = Diagram::new(&file, &registry());
                d.observe(&file, &crate::examples::view_registry());
                d
            })
        };
        let linked = with(
            json!({"format": 1, "blocks": [
                use_of("plant", "car.ssbx.json"),
                {"name": "controller", "file": "cruise.ssbx.json", "clock": {"dt": 0.5}}]}),
            cruise_links("plant", "controller"),
        );
        let d = diagram(&linked);
        let plant = &d.nodes[d.node("plant").unwrap()];
        assert_eq!(plant.kind, NodeKind::Use);
        assert_eq!(plant.method, "every 0.05 s");
        assert_eq!(plant.inputs[0].name, "force");
        assert!(plant
            .outputs
            .iter()
            .any(|p| p.name == "car.v" && p.unit == "m/s"));
        assert_eq!(d.nodes[d.node("controller").unwrap()].method, "every 0.5 s");
        // The probe records the car and drives the controller's input.
        let wired = |from: &str, to: &str| {
            d.wires
                .iter()
                .any(|w| w.from_name == from && w.to_name == to)
        };
        assert!(wired("plant.car.v", "probe:speed.record"), "{:?}", d.wires);
        assert!(
            wired("probe:speed.plant/car.v", "controller/speed"),
            "{:?}",
            d.wires
        );
        // Shared: wired directly.
        let shared_top = json!({"format": 1,
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "connections": wired_pair()});
        let d = diagram(&shared_top);
        assert_eq!(d.nodes[d.node("plant").unwrap()].method, "shared clock");
        assert!(d
            .wires
            .iter()
            .any(|w| w.from_name == "plant/car.v" && w.to_name == "controller/speed"));
        assert!(d.dangling.is_empty(), "{:?}", d.dangling);
        // A file that cannot be read shows its error on the node.
        let missing = json!({"format": 1, "blocks": [use_of("gone", "nothing.ssbx.json")]});
        let d = diagram(&missing);
        assert!(d.nodes[0]
            .error
            .as_ref()
            .unwrap()
            .contains("nothing.ssbx.json"));
        // A model without states on new data says so.
        on_data_files(&dir);
        let d = diagram(&json!({"format": 1, "blocks": [use_of("d", "double.ssbx.json")]}));
        assert_eq!(d.nodes[0].method, "on new data");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn wired_pair() -> Value {
        wired("plant", "controller")
    }

    /// The car and the controller declaring what they offer: `speed` and
    /// `force` (ADR 0066 step 4b).
    fn declared_files(dir: &Path) -> Value {
        let whole = cruise_files(dir);
        for (file, name, from) in [
            ("car.ssbx.json", "speed", "car.v"),
            ("cruise.ssbx.json", "force", "cruise.F"),
        ] {
            let mut v: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
            v["outputs"] = json!([{"name": name, "from": from}]);
            write(dir, file, &v);
        }
        whole
    }

    #[test]
    fn declared_outputs_are_reached_by_name() {
        let dir = temp_dir("declared");
        let whole = declared_files(&dir);
        // Linked across clocks by name: as by the inner names.
        let by_name = |plant: &str, controller: &str| {
            json!({
            "format": 1, "run": {"t_end": 20},
            "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")],
            "connections": [
                {"from": format!("probe:speed.{plant}"), "to": "controller/speed"},
                {"from": format!("probe:force.{controller}"), "to": "plant/force"}],
            "controls": {"probes": [
                {"name": "speed", "record": [plant]},
                {"name": "force", "record": [controller]}]}})
        };
        let named = run_in(&dir, &by_name("plant/speed", "controller/force"));
        let inner = run_in(&dir, &by_name("plant/car.v", "controller/cruise.F"));
        assert_eq!(
            bits(named.values("plant/car.v").unwrap()),
            bits(inner.values("plant/car.v").unwrap())
        );
        // Wired by name on a shared clock: as the model in one file.
        let shared_named = run_in(
            &dir,
            &json!({"format": 1, "run": {"dt": 0.05, "t_end": 20},
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "connections": [
                {"from": "plant/speed", "to": "controller/speed"},
                {"from": "controller/force", "to": "plant/force"}]}),
        );
        let one = crate::run::run(&whole.to_string(), &registry(), Some(20.0), None).unwrap();
        assert_eq!(
            bits(shared_named.values("plant/car.v").unwrap()),
            bits(series(&one, "car.v"))
        );
        // Through a file that declares what a model inside it offers.
        write(
            &dir,
            "vehicle.ssbx.json",
            &json!({"format": 1, "run": {"dt": 0.05},
            "blocks": [shared("plant", "car.ssbx.json"), shared("controller", "cruise.ssbx.json")],
            "outputs": [{"name": "v", "from": "plant/speed"}],
            "connections": [
                {"from": "plant/speed", "to": "controller/speed"},
                {"from": "controller/force", "to": "plant/force"}]}),
        );
        let watched = run_in(
            &dir,
            &json!({"format": 1, "run": {"dt": 0.05, "t_end": 5},
            "blocks": [
                {"name": "watch", "type": "input.value", "settings": {"unit": "m/s"}},
                {"name": "odo", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["input v [m/s]", "state d [m]", "dd/dt = v"], "params": {"d0": 0}},
                use_of("vehicle", "vehicle.ssbx.json")],
            "connections": [
                {"from": "watch.y", "to": "odo.v"},
                {"from": "probe:p.vehicle/v", "to": "watch"}],
            "controls": {"probes": [{"name": "p", "record": ["vehicle/v"]}]}}),
        );
        assert_eq!(
            bits(watched.values("watch.y").unwrap()),
            bits(&series(&one, "car.v")[..101])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_used_models_node_shows_its_interface() {
        use crate::diagram::Diagram;
        let dir = temp_dir("interface");
        declared_files(&dir);
        let diagram = |v: &Value| {
            let (file, _) = simsandbox_file::ModelFile::from_json(&v.to_string()).unwrap();
            simsandbox_core::registry::in_folder(Some(&dir), || {
                let mut d = Diagram::new(&file, &registry());
                d.observe(&file, &crate::examples::view_registry());
                d
            })
        };
        let alone = diagram(&json!({"format": 1, "blocks": [use_of("plant", "car.ssbx.json")]}));
        let plant = &alone.nodes[0];
        let names = |ports: &[crate::diagram::Port]| {
            ports.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
        };
        // Its input block and its declared output; nothing else, and
        // never an input block's own y.
        assert_eq!(names(&plant.inputs), ["force"]);
        assert_eq!(names(&plant.outputs), ["speed"]);
        assert_eq!(plant.outputs[0].unit, "m/s");
        // `speed` shows car.v: it is not listed again (ADR 0073).
        assert!(!plant
            .reachable
            .iter()
            .any(|(i, p)| !*i && p.name == "car.v"));
        assert!(!plant.reachable.iter().any(|(_, p)| p.name == "force.y"));
        // What is wired by an inner name shows too.
        let wired = diagram(&json!({"format": 1,
            "blocks": [use_of("plant", "car.ssbx.json")],
            "controls": {"probes": [{"name": "p", "record": ["plant/car.v"]}]}}));
        // Wired by an inner name its declared output shows: drawn from
        // that port, nothing more revealed.
        assert_eq!(names(&wired.nodes[0].outputs), ["speed"]);
        // A car that offers nothing shows nothing: its ports are exposed
        // inside it (ADR 0074), not chosen on the node.
        let mut bare: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("car.ssbx.json")).unwrap())
                .unwrap();
        bare.as_object_mut().unwrap().remove("outputs");
        write(&dir, "bare.ssbx.json", &bare);
        let bare = diagram(&json!({"format": 1, "blocks": [use_of("plant", "bare.ssbx.json")]}));
        assert!(bare.nodes[0].outputs.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_declared_outputs_refuse() {
        let dir = temp_dir("declared-refuse");
        cruise_files(&dir);
        let car: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("car.ssbx.json")).unwrap())
                .unwrap();
        let refused = |outputs: Value, says: &str| {
            let mut v = car.clone();
            v["outputs"] = outputs;
            let report = crate::describe::check(&v.to_string(), &registry());
            assert_eq!(report["ok"], false, "{v}");
            let message = report["errors"][0].to_string();
            assert!(message.contains(says), "{message}");
        };
        refused(
            json!([{"name": "s", "from": "car.v"}, {"name": "s", "from": "car.v"}]),
            "unique",
        );
        refused(
            json!([{"name": "force", "from": "car.v"}]),
            "an input block's name",
        );
        refused(
            json!([{"name": "s", "from": "wheel.v"}]),
            "one of its blocks' outputs",
        );
        refused(
            json!([{"name": "s", "from": "car.w"}]),
            "is not an output of the model",
        );
        let mut ok = car.clone();
        ok["outputs"] = json!([{"name": "speed", "from": "car.v"}]);
        let report = crate::describe::check(&ok.to_string(), &registry());
        assert_eq!(report["declared_outputs"][0]["from"], "car.v", "{report}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An instant of a merged run: its time, each clock's steps, and every
    /// block's outputs then.
    type Instant = (f64, Vec<u128>, Vec<Vec<f64>>);

    /// Run `model` merged into one simulator with clocks (ADR 0066 step
    /// 4b) and check every value the headless runner records, at each
    /// model's own instants, bit for bit.
    fn merged_matches_runner(dir: &Path, model: &Value) {
        let apart = run_in(dir, model);
        let loaded = simsandbox_core::registry::in_folder(Some(dir), || {
            simsandbox_file::compose::load_merged(&model.to_string(), &registry(), None)
        })
        .unwrap();
        let t_end = model["run"]["t_end"].as_f64().unwrap();
        let ids = loaded.ids.clone();
        let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
        // Every instant: its time, each clock's steps, and the
        // simulator's outputs then.
        let mut instants: Vec<Instant> = Vec::new();
        let grab = |sim: &Simulator| -> Vec<Vec<f64>> {
            ids.values()
                .map(|&id| {
                    let spec = sim.model().block(id).unwrap().spec().clone();
                    spec.outputs()
                        .flat_map(|p| sim.output(id, &p.name).unwrap().to_vec())
                        .collect()
                })
                .collect()
        };
        instants.push((sim.time(), sim.clock_steps().to_vec(), grab(&sim)));
        while sim
            .next_instant()
            .is_some_and(|t| t <= t_end * (1.0 + 1e-12))
        {
            sim.step(0.0).unwrap();
            instants.push((sim.time(), sim.clock_steps().to_vec(), grab(&sim)));
        }
        let value = |sim_values: &Vec<Vec<f64>>, name: &str| -> Option<Vec<f64>> {
            let (block, port) = name.rsplit_once('.')?;
            let k = ids.keys().position(|b| b == block)?;
            let id = ids[block];
            let spec = sim.model().block(id).unwrap().spec().clone();
            let mut at = 0;
            for p in spec.outputs() {
                let n = p.shape.len();
                if p.name == port {
                    return Some(sim_values[k][at..at + n].to_vec());
                }
                at += n;
            }
            None
        };
        let mut compared = 0;
        for (d, (domain, r)) in apart.models.iter().enumerate() {
            let times = &r.index[0].1;
            let on_data = model_on_data(&apart, d, &instants);
            for o in &r.outputs {
                for (row, &t) in times.iter().enumerate() {
                    let n = o.shape.len();
                    let want = &o.values[row * n..(row + 1) * n];
                    // A clocked model's row k is the instant it took its
                    // k-th step; one on new data's, the instant at its
                    // delivery's time.
                    let (_, _, vals) = instants
                        .iter()
                        .find(|(ti, steps, _)| {
                            if on_data {
                                ti.to_bits() == t.to_bits()
                            } else {
                                steps[d] == row as u128
                            }
                        })
                        .unwrap_or_else(|| panic!("{domain}: no instant for row {row} (t = {t})"));
                    let Some(got) = value(vals, &o.name) else {
                        continue;
                    };
                    assert_eq!(bits(&got), bits(want), "{} at t = {t}", o.name);
                    compared += 1;
                }
            }
        }
        assert!(compared > 20, "compared {compared}");
    }

    /// Whether domain `d` runs on new data: its clock takes no steps.
    fn model_on_data(_apart: &Composed, d: usize, instants: &[Instant]) -> bool {
        instants.last().is_some_and(|(_, steps, _)| steps[d] == 0)
    }

    #[test]
    fn one_simulator_with_clocks_runs_as_models_apart() {
        let dir = temp_dir("merged");
        cruise_files(&dir);
        // The cruise control from two files, linked both ways.
        merged_matches_runner(
            &dir,
            &with(
                json!({"format": 1, "run": {"t_end": 20},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        // Clocks of 0.1 s and 0.3 s, each reading the other.
        let mut two = pair(&dir, 0.1, 0.3);
        two["run"] = json!({"t_end": 3});
        merged_matches_runner(&dir, &two);
        // A shared plant beside a controller on its own clock.
        merged_matches_runner(
            &dir,
            &with(
                json!({"format": 1, "run": {"dt": 0.05, "t_end": 20}, "fusion": "off",
                   "blocks": [shared("plant", "car.ssbx.json"), {"name": "controller", "file": "cruise.ssbx.json", "clock": {"dt": 0.5}}]}),
                cruise_links("plant", "controller"),
            ),
        );
        // Nested: a vehicle file of linked models, used with a top level
        // reading it.
        write(
            &dir,
            "vehicle.ssbx.json",
            &with(
                json!({"format": 1,
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
                cruise_links("plant", "controller"),
            ),
        );
        merged_matches_runner(
            &dir,
            &json!({"format": 1, "run": {"t_end": 5, "dt": 0.1},
            "blocks": [
                {"name": "watch", "type": "input.value", "settings": {"unit": "m/s"}},
                {"name": "odo", "type": "eq.equations", "method": {"name": "euler"},
                 "equations": ["input v [m/s]", "state d [m]", "dd/dt = v"], "params": {"d0": 0}},
                use_of("vehicle", "vehicle.ssbx.json")],
            "connections": [
                {"from": "watch.y", "to": "odo.v"},
                {"from": "probe:v.vehicle/plant/car.v", "to": "watch"}],
            "controls": {"probes": [{"name": "v", "record": ["vehicle/plant/car.v"]}]}}),
        );
        // A model on new data: the same values at its deliveries.
        on_data_files(&dir);
        merged_matches_runner(
            &dir,
            &json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("d", "double.ssbx.json")],
            "connections": [{"from": "probe:p.a/count.x", "to": "d/u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.3}}]}}),
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn session_values(s: &crate::Session, name: &str) -> Vec<f64> {
        let h = s.signals();
        h.values(h.resolve(name).unwrap())
    }

    #[test]
    fn the_session_runs_models_that_use_files() {
        let dir = temp_dir("session-uses");
        cruise_files(&dir);
        let model = with(
            json!({"format": 1, "run": {"t_end": 20},
                   "blocks": [use_of("plant", "car.ssbx.json"), use_of("controller", "cruise.ssbx.json")]}),
            cruise_links("plant", "controller"),
        );
        let apart = run_in(&dir, &model);
        let mut s =
            crate::Session::load_in("uses", &model.to_string(), &registry(), Some(&dir)).unwrap();
        s.set_view_reads(vec!["plant/car.v".into(), "controller/cruise.F".into()]);
        s.restart().unwrap();
        s.run_until(20.0).unwrap();
        s.flush(true);
        for name in ["plant/car.v", "controller/cruise.F"] {
            assert_eq!(
                bits(&session_values(&s, name)),
                bits(apart.values(name).unwrap()),
                "{name}"
            );
        }
        // Sliders by full name.
        assert!(s.params().iter().any(|p| p.key == "plant/car.m"));
        // The file as written stays the session's file.
        assert!(s.file.blocks.iter().any(|b| b.file.is_some()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rewinding_two_clocks_replays_exactly() {
        let dir = temp_dir("session-rewind");
        let mut two = pair(&dir, 0.1, 0.3);
        two["run"] = json!({"t_end": 6});
        let mut s =
            crate::Session::load_in("two", &two.to_string(), &registry(), Some(&dir)).unwrap();
        s.set_recorder(Some(Default::default()));
        s.set_view_reads(vec!["a/count.y".into(), "b/sum.z".into()]);
        s.restart().unwrap();
        s.run_until(6.0).unwrap();
        s.flush(true);
        let first = (
            session_values(&s, "a/count.y"),
            session_values(&s, "b/sum.z"),
        );
        // Between b's instants, with a's input latched from b.
        s.rewind_to(2.75);
        s.run_until(6.0).unwrap();
        s.flush(true);
        let again = (
            session_values(&s, "a/count.y"),
            session_values(&s, "b/sum.z"),
        );
        assert_eq!(bits(&first.0), bits(&again.0));
        assert_eq!(bits(&first.1), bits(&again.1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
