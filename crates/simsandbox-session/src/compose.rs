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
        .zip(&c.ticks.steps)
        .map(|(d, steps)| {
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
                "dt": d.dt,
                "ticks_per_step": steps.to_string(),
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
        "models": models,
        "links": links,
        "tick": c.ticks.seconds(),
        "order": c.order.iter().map(|&d| c.domains[d].name()).collect::<Vec<_>>(),
        "loops": c.loops,
        "model_hash": c.hash(),
    })
}

/// One used model while it runs.
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
    /// Its step in ticks, the steps it takes, and those taken.
    every: u128,
    total: u128,
    taken: u128,
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

    /// Apply scheduled input due now; whether any was.
    fn apply(&mut self) -> Result<bool, String> {
        let now = self.sim.time();
        let mut any = false;
        while let Some(e) = self.events.take_due(now, 1e-9 * self.dt.max(now.abs())) {
            set_key(&mut self.sim, &mut self.plan, &self.ids, &e.key, e.value)
                .map_err(|err| format!("input at t = {}: {err}", e.t))?;
            any = true;
        }
        Ok(any)
    }
}

/// Run a composition to `t_end` (the file's, if not given). Used files
/// are found from the composition's folder (see
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

    // The probes that feed links: when they sample, and what each link
    // last received.
    let mut triggers: Vec<Option<Trigger>> = c
        .probes
        .iter()
        .map(|p| Some(Trigger::new(p.every.clone())))
        .collect();
    let probe_use: Vec<Option<usize>> = c.probes.iter().map(|p| Some(p.domain)).collect();
    let mut latest: Vec<f64> = vec![f64::NAN; c.links.len()];
    let mut delayed = false;

    // The start: every model has a point at t = 0.
    for r in &mut runs {
        if r.apply()? {
            r.sim.restart(0.0).map_err(|e| e.to_string())?;
        }
    }
    let all: Vec<usize> = (0..runs.len()).collect();
    point(
        &c,
        &mut runs,
        &all,
        &mut triggers,
        &probe_use,
        &mut latest,
        &mut delayed,
    )?;
    loop {
        let next = runs
            .iter()
            .filter(|r| r.taken < r.total)
            .map(|r| (r.taken + 1) * r.every)
            .min();
        let Some(tick) = next else { break };
        let reaching: Vec<usize> = (0..runs.len())
            .filter(|&u| {
                runs[u].taken < runs[u].total && (runs[u].taken + 1) * runs[u].every == tick
            })
            .collect();
        // Between points the models step apart, their link inputs held.
        for &u in &reaching {
            let r = &mut runs[u];
            r.apply()?;
            let dt = r.dt;
            r.sim
                .step(dt)
                .map_err(|e| format!("{}: {e}", c.domains[u].name()))?;
            r.taken += 1;
        }
        point(
            &c,
            &mut runs,
            &reaching,
            &mut triggers,
            &probe_use,
            &mut latest,
            &mut delayed,
        )?;
    }
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
                r.results.index = vec![("t".into(), std::mem::take(&mut r.times))];
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
        let every = c.ticks.steps[k];
        let total = c.ticks.steps_within(k, t_end).map_err(|e| e.to_string())?;
        let prefix = if u.path.is_empty() {
            String::new()
        } else {
            format!("{}/", u.path)
        };
        let name = u.name().to_string();
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
        });
    }
    Ok(runs)
}

/// A communication point of the models in `reaching`, all at the same
/// tick: in link order, each takes its link inputs, evaluates, and its
/// probes sample. A model that read a model evaluated after it (a loop)
/// then takes those samples too, for its next step.
#[allow(clippy::too_many_arguments)]
fn point(
    c: &Composition,
    runs: &mut [Running],
    reaching: &[usize],
    triggers: &mut [Option<Trigger>],
    probe_use: &[Option<usize>],
    latest: &mut [f64],
    delayed: &mut bool,
) -> Result<(), String> {
    let order: Vec<usize> = c
        .order
        .iter()
        .copied()
        .filter(|u| reaching.contains(u))
        .collect();
    for &u in &order {
        take_links(c, runs, u, latest)?;
        let r = &mut runs[u];
        let (t, step) = (r.sim.time(), r.sim.step_count());
        let eps = 1e-9 * r.dt.max(t.abs());
        for (p, trigger) in triggers.iter_mut().enumerate() {
            let Some(trigger) = trigger.as_mut().filter(|_| probe_use[p] == Some(u)) else {
                continue;
            };
            if trigger.due(t, step, eps) {
                for (k, l) in c.links.iter().enumerate().filter(|(_, l)| l.probe == p) {
                    latest[k] = value(&r.sim, &l.source);
                }
            }
        }
    }
    // Loops: samples taken after a model evaluated reach it now.
    for &u in &order {
        let sampled: Vec<f64> = c
            .links
            .iter()
            .filter(|l| l.from == u)
            .map(|l| value(&runs[u].sim, &l.source))
            .collect();
        take_links(c, runs, u, latest)?;
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
            *delayed = true;
        }
    }
    for &u in reaching {
        runs[u].record();
    }
    Ok(())
}

/// Set a model's linked input blocks to the latest samples and apply them.
fn take_links(
    c: &Composition,
    runs: &mut [Running],
    u: usize,
    latest: &[f64],
) -> Result<(), String> {
    let r = &mut runs[u];
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
        }
    }
    r.sim
        .refresh()
        .map_err(|e| format!("{}: {e}", c.domains[u].name()))
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
        write(
            &dir,
            "outer.ssbx.json",
            &json!({"format": 1,
            "blocks": [use_of("plant", "car.ssbx.json")]}),
        );
        refused(
            &|v| v["blocks"][0]["file"] = "outer.ssbx.json".into(),
            "cannot use models on their own clocks",
        );
        write(
            &dir,
            "deep.ssbx.json",
            &json!({"format": 1, "blocks": [shared("plant", "car.ssbx.json")]}),
        );
        refused(
            &|v| {
                v["blocks"][0] = shared("deep", "deep.ssbx.json");
                v["connections"] = json!([{"from": "deep/plant/car.v", "to": "controller/speed"}]);
            },
            "models inside it are wired in its own file",
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
}
