//! The run in parts (ADR 0066 step 6): every clock domain a `Simulator`
//! of its own, in exact ticks. Between their steps the domains advance
//! apart, each with its link inputs held from the start of its step; at
//! an instant several share, they evaluate in link order, each taking the
//! latest samples of the probes feeding it, and its probes sample.
//!
//! No run uses it any more: headless runs go through the merged simulator
//! ([`super::merged`], ADR 0090 point 1). Kept until deleted per ADR 0090
//! point 3, and built only for the tests that compare the two engines bit
//! for bit (point 2).

use super::Composed;
use crate::run::{Keep, Output, Population, Results, Thin, DEFAULT_T_END};
use crate::session::{set_key, InputEvent, Track};
use simsandbox_core::block::Shape;
use simsandbox_core::parallel::Parallel;
use simsandbox_core::registry::Registry;
use simsandbox_core::schedule::Trigger;
use simsandbox_core::sim::Simulator;
use simsandbox_file::compose::{load_composition, Composition};
use std::sync::mpsc;

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
pub(super) const OUTSIDE: usize = usize::MAX;

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
pub(super) static LAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

/// The run in parts, keeping only what `keep` asks for (ADR 0076); with
/// `split` false, linked models all meet at every instant (one part per
/// group), for tests to compare.
pub(super) fn run_in_parts(
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
pub(super) struct Plan {
    pub(super) links: Vec<simsandbox_file::compose::Link>,
    pub(super) probes: Vec<simsandbox_file::compose::LinkProbe>,
    pub(super) order: Vec<usize>,
    pub(super) names: Vec<String>,
}

impl Plan {
    pub(super) fn of(c: &Composition) -> Self {
        Self {
            links: c.links.clone(),
            probes: c.probes.clone(),
            order: c.order.clone(),
            names: c.domains.iter().map(|d| d.name().to_string()).collect(),
        }
    }

    /// The models links join, each group in model order, the groups by
    /// their first model.
    pub(super) fn groups(&self, n: usize) -> Vec<Vec<usize>> {
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
    pub(super) fn parts(&self, n: usize) -> Vec<Vec<usize>> {
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
    pub(super) fn part(&self, members: &[usize]) -> (Self, Vec<usize>) {
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
                if let Shape::Population { dim } | Shape::Rows { dim } = port.shape {
                    variable.push((id, port.name.clone()));
                    populations.push(Population {
                        name,
                        dim,
                        frames: Vec::new(),
                        rows: matches!(port.shape, Shape::Rows { .. }),
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
