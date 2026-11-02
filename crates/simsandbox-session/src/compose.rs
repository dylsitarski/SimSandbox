//! Running a composition headless (ADR 0065): every used model on its own
//! clock, in exact ticks. Between communication points the models step
//! apart, each with its link inputs held from the start of its step; at a
//! point, the models reaching it evaluate in link order, each taking the
//! latest sample of the probes feeding it, and its probes sample.

use crate::run::{Output, Population, Results, DEFAULT_T_END};
use crate::session::{set_key, InputEvent, Track};
use serde_json::{json, Value};
use simsandbox_core::block::Shape;
use simsandbox_core::registry::Registry;
use simsandbox_core::schedule::Trigger;
use simsandbox_core::sim::Simulator;
use simsandbox_file::compose::{load_composition, Composition};

/// What a composition's run produced: each model's results at its own
/// times, in written order.
pub struct Composed {
    pub models: Vec<(String, Results)>,
    /// The common tick, in seconds.
    pub tick: f64,
    pub warnings: Vec<String>,
}

impl Composed {
    /// One model's results.
    pub fn model(&self, name: &str) -> Option<&Results> {
        self.models.iter().find(|(n, _)| n == name).map(|(_, r)| r)
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

/// `check` of a composition (ADR 0065): its models with their files,
/// clocks and parameter values, what each records and the input blocks
/// links can drive, the links, the common tick, the order models evaluate
/// in at a shared point and the loops that order breaks.
pub fn check(text: &str, registry: &Registry) -> Value {
    let c = match load_composition(text, registry, None) {
        Ok(c) => c,
        Err(e) => {
            return json!({"ok": false, "errors": [crate::describe::file_error(&e)], "warnings": []})
        }
    };
    let models: Vec<Value> = c
        .uses
        .iter()
        .zip(&c.file.models)
        .zip(&c.ticks.steps)
        .map(|((u, entry), steps)| {
            let flat = &u.loaded.flat;
            let mut outputs = Vec::new();
            for b in &flat.blocks {
                if let Ok(block) = u.loaded.model.block(u.loaded.ids[&b.name]) {
                    for p in block.spec().outputs() {
                        outputs.push(format!("{}/{}.{}", u.name, b.name, p.name));
                    }
                }
            }
            let inputs: Vec<String> = flat
                .blocks
                .iter()
                .filter(|b| b.type_name == simsandbox_eq::input::VALUE)
                .map(|b| format!("{}/{}", u.name, b.name))
                .collect();
            json!({
                "name": u.name,
                "file": entry.file,
                "dt": u.dt,
                "ticks_per_step": steps,
                "params": entry.params,
                "outputs": outputs,
                "inputs": inputs,
            })
        })
        .collect();
    let links: Vec<Value> = c
        .links
        .iter()
        .map(|l| {
            json!({
                "probe": c.file.controls.probes[l.probe].name,
                "from": l.signal,
                "to": format!("{}/{}", c.uses[l.to].name, l.block),
            })
        })
        .collect();
    json!({
        "ok": true,
        "errors": [],
        "warnings": c.warnings,
        "composition": true,
        "models": models,
        "links": links,
        "tick": c.ticks.seconds(),
        "order": c.order.iter().map(|&u| &c.uses[u].name).collect::<Vec<_>>(),
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
    every: u64,
    total: u64,
    taken: u64,
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
    let mut triggers: Vec<Option<Trigger>> = c.file.controls.probes.iter().map(|_| None).collect();
    for l in &c.links {
        triggers[l.probe]
            .get_or_insert_with(|| Trigger::new(c.file.controls.probes[l.probe].schedule()));
    }
    let probe_use: Vec<Option<usize>> = (0..triggers.len())
        .map(|p| c.links.iter().find(|l| l.probe == p).map(|l| l.from))
        .collect();
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
                .map_err(|e| format!("{}: {e}", c.uses[u].name))?;
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
            .uses
            .iter()
            .zip(runs)
            .map(|(u, mut r)| {
                r.results.index = vec![("t".into(), std::mem::take(&mut r.times))];
                (u.name.clone(), r.results)
            })
            .collect(),
        tick: c.ticks.seconds(),
        warnings,
    })
}

/// Each used model's simulator and recording, before the first point.
fn start(c: &mut Composition, t_end: f64) -> Result<Vec<Running>, String> {
    let mut runs = Vec::new();
    for (u, every) in c.uses.iter_mut().zip(&c.ticks.steps) {
        let prefix = format!("{}/", u.name);
        // Scheduled input by full name goes to the model it names.
        let schedule = c
            .file
            .run
            .inputs
            .iter()
            .filter_map(|e| {
                Some(InputEvent {
                    t: e.t,
                    key: e.set.strip_prefix(&prefix)?.to_string(),
                    value: e.to,
                })
            })
            .collect();
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
        let sim = Simulator::new(model, 0.0).map_err(|e| format!("{}: {e}", u.name))?;
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
            every: *every,
            total: (t_end / u.dt).round() as u64,
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
                .map_err(|e| format!("{}/{}: {e}", c.uses[u].name, l.block))?;
        }
    }
    r.sim
        .refresh()
        .map_err(|e| format!("{}: {e}", c.uses[u].name))
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

    fn run_in(dir: &Path, composition: &Value) -> Composed {
        simsandbox_core::registry::in_folder(Some(dir), || {
            run_composition(&composition.to_string(), &registry(), None)
        })
        .unwrap()
    }

    fn series<'a>(r: &'a Results, name: &str) -> &'a [f64] {
        &r.outputs.iter().find(|o| o.name == name).unwrap().values
    }

    fn bits(v: &[f64]) -> Vec<u64> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    #[test]
    fn a_model_split_in_two_runs_as_the_model_with_a_hold() {
        let text = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap()
            .text;
        let whole: Value = serde_json::from_str(text).unwrap();
        let (car, cruise) = (&whole["blocks"][0], &whole["blocks"][1]);
        let dir = temp_dir("split");
        let input = |name: &str, unit: &str| json!({"name": name, "type": "input.value", "settings": {"unit": unit}});
        let run = json!({"dt": 0.05, "t_end": 20});
        write(
            &dir,
            "car.ssbx.json",
            &json!({
            "format": 1, "run": run, "fusion": "off",
            "blocks": [input("force", "N"), car],
            "connections": [{"from": "force.y", "to": "car.F"}]}),
        );
        write(
            &dir,
            "cruise.ssbx.json",
            &json!({
            "format": 1, "run": run, "fusion": "off",
            "blocks": [input("speed", "m/s"), cruise],
            "connections": [{"from": "speed.y", "to": "cruise.v"}]}),
        );
        let composed = run_in(
            &dir,
            &json!({
            "format": 1, "run": {"t_end": 20},
            "models": [{"name": "plant", "file": "car.ssbx.json"},
                       {"name": "controller", "file": "cruise.ssbx.json"}],
            "connections": [
                {"from": "probe:speed.plant/car.v", "to": "controller/speed"},
                {"from": "probe:force.controller/cruise.F", "to": "plant/force"}],
            "controls": {"probes": [
                {"name": "speed", "record": ["plant/car.v"]},
                {"name": "force", "record": ["controller/cruise.F"]}]}}),
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
    fn two_clocks_hold_each_others_samples() {
        // fast (0.1 s) counts time, x' = 1, and shows what it reads from
        // slow, y = w; slow (0.3 s) integrates what it reads from fast,
        // z' = u. Euler, so every value can be computed by hand.
        let dir = temp_dir("clocks");
        write(
            &dir,
            "fast.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": 0.1},
            "blocks": [
                {"name": "w", "type": "input.value"},
                {"name": "count", "type": "eq.equations", "method": {"name": "euler"}, "equations": [
                    "input w", "state x", "dx/dt = 1", "y = w"], "params": {"x0": 0}}],
            "connections": [{"from": "w.y", "to": "count.w"}]}),
        );
        write(
            &dir,
            "slow.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": 0.3},
            "blocks": [
                {"name": "u", "type": "input.value"},
                {"name": "sum", "type": "eq.equations", "method": {"name": "euler"}, "equations": [
                    "input u", "state z", "dz/dt = u"], "params": {"z0": 0}}],
            "connections": [{"from": "u.y", "to": "sum.u"}]}),
        );
        let composed = run_in(
            &dir,
            &json!({
            "format": 1, "run": {"t_end": 3},
            "models": [{"name": "fast", "file": "fast.ssbx.json"},
                       {"name": "slow", "file": "slow.ssbx.json"}],
            "connections": [
                {"from": "probe:x.fast/count.x", "to": "slow/u"},
                {"from": "probe:z.slow/sum.z", "to": "fast/w"}],
            "controls": {"probes": [
                {"name": "x", "record": ["fast/count.x"]},
                {"name": "z", "record": ["slow/sum.z"]}]}}),
        );
        assert_eq!(composed.tick, 0.1);
        let fast = composed.model("fast").unwrap();
        let slow = composed.model("slow").unwrap();
        assert_eq!((fast.rows(), slow.rows()), (31, 11));
        // By hand: x after n fast steps; z after k slow steps, each
        // adding 0.3 × x where slow last took it (at 0.3 k).
        let mut x = vec![0.0f64];
        for _ in 0..30 {
            x.push(x.last().unwrap() + 0.1 * 1.0);
        }
        let mut z = vec![0.0f64];
        for k in 0..10 {
            z.push(z[k] + 0.3 * x[3 * k]);
        }
        assert_eq!(bits(series(fast, "fast/count.x")), bits(&x));
        assert_eq!(bits(series(slow, "slow/sum.z")), bits(&z));
        // fast reads slow's latest sample at or before each of its points.
        let y: Vec<f64> = (0..=30).map(|n| z[n / 3]).collect();
        assert_eq!(bits(series(fast, "fast/count.y")), bits(&y));
        assert_eq!(series(slow, "slow/u.y")[1], x[3]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two models in `dir`: `a` counts (`count.x`, `count.y` showing what
    /// it reads into `w`), `b` integrates (`sum.z` from `u`).
    fn pair(dir: &Path) -> Value {
        write(
            dir,
            "a.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": 0.1},
            "blocks": [
                {"name": "w", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "count", "type": "eq.equations", "equations": [
                    "input w [m]", "param r [m/s] = 1", "state x [m]", "dx/dt = r", "y = w"]}],
            "connections": [{"from": "w.y", "to": "count.w"}]}),
        );
        write(
            dir,
            "b.ssbx.json",
            &json!({
            "format": 1, "run": {"dt": 0.2},
            "blocks": [
                {"name": "u", "type": "input.value", "settings": {"unit": "m"}},
                {"name": "sum", "type": "eq.equations", "equations": [
                    "input u [m]", "state z [m*s]", "dz/dt = u"]}],
            "connections": [{"from": "u.y", "to": "sum.u"}]}),
        );
        json!({
            "format": 1,
            "models": [{"name": "a", "file": "a.ssbx.json"}, {"name": "b", "file": "b.ssbx.json"}],
            "connections": [{"from": "probe:p.a/count.x", "to": "b/u"}],
            "controls": {"probes": [{"name": "p", "record": ["a/count.x"]}]}})
    }

    fn check_in(dir: &Path, v: &Value) -> Value {
        simsandbox_core::registry::in_folder(Some(dir), || check(&v.to_string(), &registry()))
    }

    #[test]
    fn check_lists_models_links_and_the_tick() {
        let dir = temp_dir("compose-check");
        let c = pair(&dir);
        let report = check_in(&dir, &c);
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["tick"], 0.1);
        assert_eq!(report["order"], json!(["a", "b"]));
        assert_eq!(report["models"][1]["ticks_per_step"], 2);
        assert_eq!(report["models"][1]["inputs"], json!(["b/u"]));
        assert!(report["models"][0]["outputs"]
            .as_array()
            .unwrap()
            .contains(&json!("a/count.x")));
        assert_eq!(report["links"][0]["to"], "b/u");
        // describe's check sends compositions here.
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
        faster["models"][1]["run"] = json!({"dt": 0.1});
        assert_ne!(hash(&faster), before);
        let mut set = c.clone();
        set["models"][1]["params"] = json!({"sum.z0": 2});
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
    fn what_a_composition_refuses() {
        let dir = temp_dir("compose-refuse");
        let c = pair(&dir);
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
            "no input block",
        );
        refused(
            &|v| v["connections"][0]["from"] = "probe:p.a/count.y".into(),
            "does not record",
        );
        refused(
            &|v| v["models"][1]["file"] = "missing.ssbx.json".into(),
            "missing.ssbx.json",
        );
        refused(&|v| v["models"][1]["name"] = "a".into(), "used twice");
        refused(
            &|v| v["models"][1]["name"] = "b/c".into(),
            "letters, digits",
        );
        refused(
            &|v| v["models"][1]["params"] = json!({"sum.nope": 1}),
            "has no parameter 'nope'",
        );
        refused(
            &|v| v["blocks"] = json!([{"name": "x", "type": "input.value"}]),
            "no blocks of its own",
        );
        refused(
            &|v| v["controls"]["probes"][0]["record"] = json!(["a/count.x", "b/sum.z"]),
            "one model",
        );
        refused(
            &|v| v["connections"][0]["to"] = "a/w".into(),
            "its own model",
        );
        // Units must match: z is m*s, w takes m.
        refused(
            &|v| {
                v["connections"] = json!([{"from": "probe:q.b/sum.z", "to": "a/w"}]);
                v["controls"]["probes"] = json!([{"name": "q", "record": ["b/sum.z"]}]);
            },
            "units must match",
        );
        // A model alone still runs as a model; a composition is not one.
        let e = simsandbox_file::load(&c.to_string(), &registry())
            .err()
            .unwrap();
        assert!(e.to_string().contains("composition"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
