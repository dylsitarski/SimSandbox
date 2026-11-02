//! Running a model that uses model files headless (ADR 0066): every clock
//! domain on its own clock, in exact ticks. Between their steps the
//! domains advance apart, each with its link inputs held from the start of
//! its step; at an instant several share, they evaluate in link order,
//! each taking the latest samples of the probes feeding it, and its probes
//! sample.

use crate::run::{Output, Population, Results, DEFAULT_T_END};
use crate::session::{set_key, InputEvent, Track};
use serde_json::{json, Value};
use simsandbox_core::block::Shape;
use simsandbox_core::registry::Registry;
use simsandbox_core::schedule::Trigger;
use simsandbox_core::sim::Simulator;
use simsandbox_file::compose::{load_composition, Composition};

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
        json!({"models": models, "tick": self.tick, "warnings": self.warnings})
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
                "swept": d.swept(),
                "outputs": outputs,
                "inputs": inputs,
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
        "models": models,
        "links": links,
        "tick": c.ticks.seconds(),
        "order": c.order.iter().map(|&d| c.domains[d].name()).collect::<Vec<_>>(),
        "loops": c.loops,
        "model_hash": c.hash(),
    })
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
    /// For one that sweeps at each delivery: its sweep, and what indexes
    /// the latest one's rows.
    sweep: Option<simsandbox_file::Plan>,
    sweep_index: Vec<(String, Vec<f64>)>,
    swept_at: Option<f64>,
    /// Its path and a slash, prefixing its names.
    prefix: String,
}

impl Running {
    fn record(&mut self) {
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

    /// Evaluate a model that runs on new data at `t`: once, recorded, or
    /// its whole sweep, which replaces the last.
    fn evaluate(&mut self, t: f64) -> Result<(), String> {
        self.apply(t)?;
        self.evaluations += 1;
        let Some(plan) = self.sweep.clone() else {
            self.sim.restart(t).map_err(|e| e.to_string())?;
            self.record();
            return Ok(());
        };
        for o in &mut self.results.outputs {
            o.values.clear();
        }
        for p in &mut self.results.populations {
            p.frames.clear();
        }
        self.times.clear();
        let mut index = vec![("sample".to_string(), Vec::new())];
        index.extend(
            plan.keys
                .iter()
                .map(|k| (format!("{}{k}", self.prefix), Vec::new())),
        );
        let held: Vec<(simsandbox_core::graph::BlockId, String, f64)> = plan
            .keys
            .iter()
            .map(|k| {
                let (block, param) = k.split_once('.').expect("plan keys are block.param");
                let id = self.ids[block];
                (
                    id,
                    param.to_string(),
                    self.sim.param(id, param).unwrap_or(f64::NAN),
                )
            })
            .collect();
        for (k, point) in plan.points.iter().enumerate() {
            for ((id, param, _), &v) in held.iter().zip(point) {
                self.sim
                    .set_param(*id, param, v)
                    .map_err(|e| e.to_string())?;
            }
            self.sim.restart(t).map_err(|e| e.to_string())?;
            index[0].1.push(k as f64);
            for (column, &v) in index[1..].iter_mut().zip(point) {
                column.1.push(v);
            }
            self.record();
        }
        // The swept parameters go back to their values.
        for (id, param, v) in &held {
            self.sim
                .set_param(*id, param, *v)
                .map_err(|e| e.to_string())?;
        }
        self.sweep_index = index;
        self.swept_at = Some(t);
        Ok(())
    }
}

/// What a point pass carries from one instant to the next: each probe's
/// trigger, and what each link last received.
struct Links {
    triggers: Vec<Trigger>,
    latest: Vec<f64>,
    delayed: bool,
}

/// Run a model that uses model files to `t_end` (the file's, if not
/// given). Used files are found from its folder (see
/// [`simsandbox_core::registry::in_folder`]).
pub fn run_composition(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
) -> Result<Composed, String> {
    if let Some(t) = t_end.filter(|t| !(t.is_finite() && *t > 0.0)) {
        return Err(format!("the end time must be positive, got {t}"));
    }
    let mut c = load_composition(text, registry, None).map_err(|e| e.to_string())?;
    let t_end = t_end.or(c.file.run.t_end).unwrap_or(DEFAULT_T_END);
    let mut warnings = c.warnings.clone();
    warnings.extend(c.loops.iter().map(|l| format!("loop between models: {l}")));
    let mut runs = start(&mut c, t_end)?;
    let mut links = Links {
        triggers: c
            .probes
            .iter()
            .map(|p| Trigger::new(p.every.clone()))
            .collect(),
        latest: vec![f64::NAN; c.links.len()],
        delayed: false,
    };
    // The start: every model has a point at t = 0.
    for r in runs.iter_mut().filter(|r| !r.on_data) {
        if r.apply(0.0)? {
            r.sim.restart(0.0).map_err(|e| e.to_string())?;
        }
    }
    let clocked: Vec<usize> = (0..runs.len()).filter(|&u| !runs[u].on_data).collect();
    point(&c, &mut runs, &clocked, true, &mut links)?;
    loop {
        let next = runs
            .iter()
            .filter(|r| !r.on_data && r.taken < r.total)
            .map(|r| (r.taken + 1) * r.every)
            .min();
        let Some(tick) = next else { break };
        let reaching: Vec<usize> = (0..runs.len())
            .filter(|&u| {
                let r = &runs[u];
                !r.on_data && r.taken < r.total && (r.taken + 1) * r.every == tick
            })
            .collect();
        // Between points the models step apart, their link inputs held.
        for &u in &reaching {
            let r = &mut runs[u];
            let now = r.sim.time();
            r.apply(now)?;
            let dt = r.dt;
            r.sim
                .step(dt)
                .map_err(|e| format!("{}: {e}", c.domains[u].name()))?;
            r.taken += 1;
        }
        point(&c, &mut runs, &reaching, false, &mut links)?;
    }
    if links.delayed {
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
                r.results.index = if r.sweep.is_some() {
                    if let Some(t) = r.swept_at {
                        r.results
                            .notes
                            .push(format!("the latest sweep, evaluated at t = {t}"));
                    }
                    std::mem::take(&mut r.sweep_index)
                } else {
                    vec![("t".into(), std::mem::take(&mut r.times))]
                };
                (d.name().to_string(), r.results)
            })
            .collect(),
        tick: c.ticks.seconds(),
        warnings,
    })
}

/// Each domain's simulator and recording, before the first step.
fn start(c: &mut Composition, t_end: f64) -> Result<Vec<Running>, String> {
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
    for (k, (u, schedule)) in c.domains.iter_mut().zip(schedules).enumerate() {
        let (every, total) = match c.clocks[k] {
            Some(i) => (
                c.ticks.steps[i],
                c.ticks.steps_within(i, t_end).map_err(|e| e.to_string())?,
            ),
            None => (0, 0),
        };
        let sweep = if u.swept() {
            u.loaded.plan.clone()
        } else {
            None
        };
        let prefix = if u.path.is_empty() {
            String::new()
        } else {
            format!("{}/", u.path)
        };
        let name = u.name().to_string();
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
                        values: Vec::new(),
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
            events: Track::new(schedule, Vec::new()),
            dt: u.dt,
            every,
            total,
            taken: 0,
            on_data,
            evaluations: 0,
            sweep,
            sweep_index: Vec::new(),
            swept_at: None,
            prefix: prefix.clone(),
        });
    }
    Ok(runs)
}

/// One instant: the clocked models in `reaching` (all at the same tick)
/// and the models on new data, in link order. A clocked model takes its
/// link inputs and evaluates; a model on new data is evaluated if a link
/// into it delivered a sample during this pass (or at the start), at the
/// time of the model that delivered it; then its probes sample. A model
/// that read one evaluated after it (a loop) then takes those samples
/// too: for its next step, or, on new data, evaluated again.
fn point(
    c: &Composition,
    runs: &mut [Running],
    reaching: &[usize],
    start: bool,
    links: &mut Links,
) -> Result<(), String> {
    let order: Vec<usize> = c
        .order
        .iter()
        .copied()
        .filter(|&u| reaching.contains(&u) || runs[u].on_data)
        .collect();
    // When each link last received a sample in this pass, and which
    // models on new data were evaluated, at what time.
    let mut fresh = vec![false; c.links.len()];
    let mut evaluated: Vec<Option<f64>> = vec![None; runs.len()];
    let delivery = |runs: &[Running], fresh: &[bool], u: usize| -> Option<f64> {
        c.links
            .iter()
            .enumerate()
            .find(|(k, l)| l.to == u && fresh[*k])
            .map(|(_, l)| runs[l.from].sim.time())
    };
    for &u in &order {
        if runs[u].on_data {
            let at = if start {
                Some(0.0)
            } else {
                delivery(runs, &fresh, u)
            };
            let Some(t) = at else { continue };
            take_links(c, runs, u, &links.latest)?;
            runs[u]
                .evaluate(t)
                .map_err(|e| format!("{}: {e}", c.domains[u].name()))?;
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
            let Some(t) = evaluated[u].or_else(|| delivery(runs, &fresh, u)) else {
                continue;
            };
            // Evaluated again with what arrived: its record for this
            // instant is the new one.
            if evaluated[u].is_some() && runs[u].sweep.is_none() {
                let r = &mut runs[u];
                r.times.pop();
                for o in &mut r.results.outputs {
                    let n = o.shape.len();
                    o.values.truncate(o.values.len() - n);
                }
                for p in &mut r.results.populations {
                    p.frames.pop();
                }
            }
            runs[u]
                .evaluate(t)
                .map_err(|e| format!("{}: {e}", c.domains[u].name()))?;
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
    for &u in reaching {
        runs[u].record();
    }
    Ok(())
}

/// The probes on model `u` sample if due: each link they feed takes the
/// value, marked as fresh in this pass.
fn sample(c: &Composition, runs: &[Running], u: usize, links: &mut Links, fresh: &mut [bool]) {
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
            fresh[k] = true;
        }
    }
}

/// Set a model's linked input blocks to the latest samples and apply them;
/// whether any changed.
fn take_links(
    c: &Composition,
    runs: &mut [Running],
    u: usize,
    latest: &[f64],
) -> Result<bool, String> {
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
        .map_err(|e| format!("{}: {e}", c.domains[u].name()))?;
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

    fn check_in(dir: &Path, v: &Value) -> Value {
        simsandbox_core::registry::in_folder(Some(dir), || check(&v.to_string(), &registry()))
    }

    fn series<'a>(r: &'a Results, name: &str) -> &'a [f64] {
        &r.outputs.iter().find(|o| o.name == name).unwrap().values
    }

    fn bits(v: &[f64]) -> Vec<u64> {
        v.iter().map(|x| x.to_bits()).collect()
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
        assert!(!composed
            .warnings
            .iter()
            .any(|w| w.contains("saw that model")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_swept_model_runs_its_sweep_at_each_delivery() {
        let dir = temp_dir("on-data-sweep");
        on_data_files(&dir);
        let mut double: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("double.ssbx.json")).unwrap())
                .unwrap();
        double["sample"] =
            json!({"design": "grid", "params": [{"param": "f.k", "range": [1, 3], "points": 3}]});
        write(&dir, "sweep.ssbx.json", &double);
        let top = json!({"format": 1, "run": {"t_end": 1.2},
            "blocks": [use_of("a", "a.ssbx.json"), use_of("s", "sweep.ssbx.json")],
            "connections": [{"from": "probe:p.a/count.x", "to": "s/u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"], "every": {"seconds": 0.6}}]}});
        let composed = run_in(&dir, &top);
        let s = composed.model("s").unwrap();
        // The latest sweep: at 1.2 s, over k = 1, 2, 3.
        let x = counted(12);
        assert_eq!(s.index[1], ("s/f.k".to_string(), vec![1.0, 2.0, 3.0]));
        let y: Vec<f64> = [1.0, 2.0, 3.0].iter().map(|k| k * x[12]).collect();
        assert_eq!(bits(series(s, "s/f.y")), bits(&y));
        assert!(
            s.notes.iter().any(|n| n.contains("latest sweep")),
            "{:?}",
            s.notes
        );
        let report = check_in(&dir, &top);
        assert_eq!(report["models"][1]["swept"], true);
        // A sweep feeds no link, and runs on new data only.
        let mut feeding = top.clone();
        feeding["connections"] = json!([{"from": "probe:q.s/f.y", "to": "a/w"}]);
        feeding["controls"]["probes"] = json!([{"name": "q", "record": ["s/f.y"]}]);
        assert!(check_in(&dir, &feeding)["errors"][0]
            .to_string()
            .contains("cannot feed a link"));
        let mut clocked = top.clone();
        clocked["blocks"][1]["clock"] = json!({"dt": 0.1});
        assert!(check_in(&dir, &clocked)["errors"][0]
            .to_string()
            .contains("give its use no clock"));
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
        assert!(plant
            .reachable
            .iter()
            .any(|(i, p)| !*i && p.name == "car.v"));
        assert!(!plant.reachable.iter().any(|(_, p)| p.name == "force.y"));
        // What is wired by an inner name shows too.
        let wired = diagram(&json!({"format": 1,
            "blocks": [use_of("plant", "car.ssbx.json")],
            "controls": {"probes": [{"name": "p", "record": ["plant/car.v"]}]}}));
        assert_eq!(names(&wired.nodes[0].outputs), ["speed", "car.v"]);
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
}
