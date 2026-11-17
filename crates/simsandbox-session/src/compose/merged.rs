//! A headless run of a model that uses model files as one simulator with
//! clocks (ADR 0090): [`simsandbox_file::compose::load_merged_with`]'s
//! model stepped from instant to instant, each model's outputs recorded
//! at its own instants under its own name.

use super::Composed;
use crate::run::{Keep, Output, Population, Results, Thin, DEFAULT_T_END};
use crate::session::{set_key, InputEvent, Track};
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::parallel::Parallel;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_file::compose::{load_merged_with, Composition};
use simsandbox_file::schedule::Trigger;

/// One model of the run: where its clock stands and what it records.
struct Part {
    on_data: bool,
    dt: f64,
    /// Its step in ticks, the steps it takes, and those taken; all 0 for
    /// one that runs on new data.
    every: u128,
    total: u128,
    taken: u128,
    /// Its listed instants (ADR 0067 step 4) in ticks and seconds;
    /// `taken` counts those passed.
    listed: Vec<(u128, f64)>,
    /// Its own time: its steps added up, or, on new data, when it was
    /// last evaluated.
    time: f64,
    /// On new data: how many times it has been evaluated.
    evaluations: u64,
    fixed: Vec<(BlockId, String)>,
    variable: Vec<(BlockId, String)>,
    results: Results,
    times: Vec<f64>,
    /// Which of its steps it keeps (ADR 0076).
    thin: Thin,
    events: Track,
}

impl Part {
    /// Record the instant just reached, if kept.
    fn record(&mut self, sim: &Simulator) {
        if self.thin.keep() {
            self.record_now(sim);
        }
    }

    /// Record the instant just reached.
    fn record_now(&mut self, sim: &Simulator) {
        self.times.push(self.time);
        for (o, (id, port)) in self.results.outputs.iter_mut().zip(&self.fixed) {
            o.values
                .extend_from_slice(sim.output(*id, port).unwrap_or(&[]));
        }
        for (p, (id, port)) in self.results.populations.iter_mut().zip(&self.variable) {
            p.frames.push(sim.output(*id, port).unwrap_or(&[]).to_vec());
        }
    }

    /// Its next instant up to `end`: its next step, or its next listed
    /// time.
    fn next(&self, end: u128) -> Option<(u128, f64)> {
        if !self.listed.is_empty() {
            return self
                .listed
                .get(self.taken as usize)
                .copied()
                .filter(|&(k, _)| k <= end);
        }
        (!self.on_data && self.taken < self.total)
            .then(|| ((self.taken + 1) * self.every, f64::NAN))
    }

    /// How close to `now` scheduled input and probes count as due.
    fn eps(&self, now: f64) -> f64 {
        1e-9 * self.dt.max(now.abs()).max(1e-300)
    }
}

/// [`super::run_composition_keeping`] on the merged simulator.
pub(super) fn run(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    parallel: Parallel,
    keep: &Keep,
) -> Result<Composed, String> {
    if let Some(t) = t_end.filter(|t| !(t.is_finite() && *t > 0.0)) {
        return Err(format!("the end time must be positive, got {t}"));
    }
    let (loaded, c) = load_merged_with(text, registry, None).map_err(|e| e.to_string())?;
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
    let end = c.ticks.within(t_end).map_err(|e| e.to_string())?;
    let mut warnings = c.warnings.clone();
    warnings.extend(c.loops.iter().map(|l| format!("loop between models: {l}")));
    let mut plan = loaded.params.clone();
    let ids = loaded.ids;
    let mut parts = start(&c, &ids, t_end, keep)?;
    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    sim.set_parallel(parallel);
    for (d, part) in parts.iter_mut().enumerate() {
        part.results.labels = labels(sim.model(), d);
    }
    for (d, note) in notes(&sim, &c) {
        parts[d].results.notes.push(note);
    }
    // Scheduled input due for model `u` at `now`, set by full name;
    // whether any was.
    let mut apply = |sim: &mut Simulator, part: &mut Part, now: f64| -> Result<bool, String> {
        let mut any = false;
        while let Some(e) = part.events.take_due(now, part.eps(now)) {
            set_key(sim, &mut plan, &ids, &e.key, e.value)
                .map_err(|err| format!("input at t = {}: {err}", e.t))?;
            any = true;
        }
        Ok(any)
    };
    // The start: every model has a point at t = 0, input then setting
    // where the run starts.
    let mut any = false;
    for part in &mut parts {
        any |= apply(&mut sim, part, 0.0)?;
    }
    if any {
        sim.restart(0.0).map_err(|e| e.to_string())?;
    }
    let mut triggers: Vec<Trigger> = c
        .probes
        .iter()
        .map(|p| Trigger::new(p.every.clone()))
        .collect();
    // When each link last took a sample (its model's time then), and
    // which did at the present instant.
    let mut sampled = vec![f64::NAN; c.links.len()];
    let mut fresh = vec![false; c.links.len()];
    for &u in &c.order {
        if parts[u].on_data {
            parts[u].evaluations += 1;
        }
        sample(&c, &parts[u], u, &mut triggers, &mut sampled, &mut fresh);
    }
    for part in &mut parts {
        part.record(&sim);
    }
    // From instant to instant: the earliest next one of any model.
    while let Some(tick) = parts.iter().filter_map(|p| p.next(end)).map(|n| n.0).min() {
        // The models stepping to this instant take the input due at the
        // start of their step, then stand at its end.
        let mut reaching = vec![false; parts.len()];
        let mut due: Vec<Option<f64>> = vec![None; parts.len()];
        for (u, part) in parts.iter_mut().enumerate() {
            let Some((_, t)) = part.next(end).filter(|n| n.0 == tick) else {
                continue;
            };
            part.taken += 1;
            if part.on_data {
                // A sample of its own recorded data.
                due[u] = Some(t);
                continue;
            }
            let now = part.time;
            apply(&mut sim, part, now)?;
            reaching[u] = true;
            // By `dt`, or to its listed time.
            part.time += if t.is_nan() { part.dt } else { t - now };
        }
        // In link order: a model on new data is evaluated at a sample of
        // its own data or when a link delivers, at the time of the model
        // that delivered; then each model's probes sample.
        fresh.fill(false);
        let mut evaluated = vec![false; parts.len()];
        for &u in &c.order {
            if parts[u].on_data {
                let delivered = c
                    .links
                    .iter()
                    .enumerate()
                    .find(|(k, l)| l.to == u && fresh[*k])
                    .map(|(k, _)| sampled[k]);
                if let Some(t) = due[u].or(delivered) {
                    apply(&mut sim, &mut parts[u], t)?;
                    parts[u].evaluations += 1;
                    parts[u].time = t;
                    evaluated[u] = true;
                }
            } else if !reaching[u] {
                continue;
            }
            sample(&c, &parts[u], u, &mut triggers, &mut sampled, &mut fresh);
        }
        sim.step(0.0).map_err(|e| e.to_string())?;
        for (u, part) in parts.iter_mut().enumerate() {
            if reaching[u] || evaluated[u] {
                part.record(&sim);
            }
        }
    }
    Ok(Composed {
        models: c
            .domains
            .iter()
            .zip(parts)
            .map(|(d, mut part)| {
                // Its last step, kept whatever `every` says.
                if part.thin.behind {
                    part.record_now(&sim);
                }
                part.results.index = vec![("t".into(), std::mem::take(&mut part.times))];
                (d.name().to_string(), part.results)
            })
            .collect(),
        tick: c.ticks.seconds(),
        warnings,
    })
}

/// Each model's clock and recording, before the first step: its outputs
/// as its own file gives them, read from the merged model's blocks.
fn start(
    c: &Composition,
    ids: &std::collections::BTreeMap<String, BlockId>,
    t_end: f64,
    keep: &Keep,
) -> Result<Vec<Part>, String> {
    // Scheduled input by full name goes to the model holding it.
    let mut schedules: Vec<Vec<InputEvent>> = c.domains.iter().map(|_| Vec::new()).collect();
    for e in &c.file.run.inputs {
        let (d, _) = c
            .domain_of(&e.set)
            .ok_or_else(|| format!("run.inputs: {} is in no model", e.set))?;
        schedules[d].push(InputEvent {
            t: e.t,
            key: e.set.clone(),
            value: e.to,
        });
    }
    let mut parts = Vec::new();
    let mut all = Vec::new();
    for (k, (u, schedule)) in c.domains.iter().zip(schedules).enumerate() {
        let (every, total) = match c.clocks[k] {
            Some(i) => (
                c.ticks.steps[i],
                c.ticks.steps_within(i, t_end).map_err(|e| e.to_string())?,
            ),
            None => (0, 0),
        };
        let model = u.name().to_string();
        let mut fixed = Vec::new();
        let mut variable = Vec::new();
        let mut outputs = Vec::new();
        let mut populations = Vec::new();
        for entry in &u.loaded.flat.blocks {
            let own = u.loaded.ids[&entry.name];
            let spec = u.loaded.model.block(own).map_err(|e| e.to_string())?.spec();
            let block = u.full(&entry.name);
            let id = *ids
                .get(&block)
                .ok_or_else(|| format!("{block} is not in the model as one"))?;
            for port in spec.outputs() {
                let name = format!("{block}.{}", port.name);
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
        parts.push(Part {
            on_data: u.on_data,
            dt: u.dt,
            every,
            total,
            taken: 0,
            listed: c.times[k].clone(),
            time: 0.0,
            evaluations: 0,
            fixed,
            variable,
            results: Results {
                index: Vec::new(),
                outputs,
                populations,
                labels: None,
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            times: Vec::new(),
            thin: keep.thin(),
            events: Track::new(schedule, Vec::new()),
        });
    }
    keep.check(&all)?;
    Ok(parts)
}

/// The probes on model `u` sample if due: each link they feed is fresh at
/// this instant, sampled at the model's time.
fn sample(
    c: &Composition,
    part: &Part,
    u: usize,
    triggers: &mut [Trigger],
    sampled: &mut [f64],
    fresh: &mut [bool],
) {
    let t = part.time;
    // A model on new data counts its evaluations as its steps.
    let step = if part.on_data {
        part.evaluations.saturating_sub(1)
    } else {
        part.taken as u64
    };
    for (p, probe) in c.probes.iter().enumerate() {
        if probe.domain != u || !triggers[p].due(t, step, part.eps(t)) {
            continue;
        }
        for (k, _) in c.links.iter().enumerate().filter(|(_, l)| l.probe == p) {
            sampled[k] = t;
            fresh[k] = true;
        }
    }
}

/// Speed and accuracy labels of the blocks on clock `d`: one model's, as
/// [`Simulator::labels`] gives the whole model's.
fn labels(m: &Model, d: usize) -> Option<MethodLabels> {
    let clock_of = m.clocks().1;
    let single = m
        .block_ids()
        .filter(|&id| clock_of[id.0] == d && m.group_of(id).is_none())
        .filter_map(|id| m.block(id).ok().map(|b| b.method().labels));
    let groups = m
        .groups()
        .iter()
        .filter(|g| g.members().first().is_some_and(|id| clock_of[id.0] == d))
        .map(|g| g.method().labels);
    MethodLabels::combine(single.chain(groups))
}

/// The loop notes ([`crate::session::loop_notes`]) of each model, by its
/// place: those of loops inside it, named as in its own file.
fn notes(sim: &Simulator, c: &Composition) -> Vec<(usize, String)> {
    // The model all of `names` lie in, and their names inside it.
    let inside = |names: &[String]| -> Option<(usize, Vec<String>)> {
        let mut found = None;
        let mut local = Vec::new();
        for n in names {
            let (d, rest) = c.domain_of(n)?;
            if found.is_some_and(|f| f != d) {
                return None;
            }
            found = Some(d);
            local.push(rest.to_string());
        }
        found.map(|d| (d, local))
    };
    let method = sim.model().loop_method().name();
    let mut notes: Vec<(usize, String)> = sim
        .loops()
        .iter()
        .filter_map(|l| inside(l))
        .map(|(d, l)| (d, crate::session::algebraic_note(&l, method)))
        .collect();
    notes.extend(
        crate::session::apart_loops(sim.model())
            .iter()
            .filter_map(|l| inside(l))
            .map(|(d, l)| (d, crate::session::apart_note(&l))),
    );
    notes
}
