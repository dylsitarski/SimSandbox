//! Strategies written as models (ADR 0078 points 5 and 7): pattern search
//! (`minimize`), Levenberg–Marquardt (`fit`), refinement (`refine`) and
//! natural-parameter continuation (`continue`). Each writes into a model
//! a study whose points come from its inputs and a strategy block, a map
//! whose rounds are its steps, wired to it: the same models as the
//! examples, for any number of parameters, which the user can open and
//! change. Python's `Model.minimize`, `fit`, `refine` and `continue_` and
//! the app's "Search…" call [`write`].

use crate::diagram::{apply_in, free_name, Edit, Scope};
use serde::{Deserialize, Serialize};
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::Registry;
use simsandbox_core::units::Unit;
use simsandbox_file::subsystems::ParamValue;
use simsandbox_file::{BlockEntry, ConnectionEntry, ModelFile};

/// A strategy and its settings, as Python and the app give them
/// (`{"kind": "minimize", …}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Strategy {
    /// Pattern search: the centre and a step either way on each parameter
    /// a round, moving to the best point or halving the step, until the
    /// step is below `tol` (relative to each parameter's `span`).
    Minimize {
        params: Vec<Varied>,
        /// What each run scores, lower being better: a reduce formula
        /// (`final(car.v)`, `first_time(abs(e) < 0.5)`).
        objective: String,
        #[serde(default = "default_search_tol")]
        tol: f64,
        #[serde(default = "default_budget")]
        budget: f64,
    },
    /// Levenberg–Marquardt least squares: a forward-difference Jacobian
    /// from the centre and a small step on each parameter, the damped
    /// step by `solve`, until the step is below `tol` (relative).
    Fit {
        params: Vec<Varied>,
        /// Each run's residuals, a vector: `at_samples(osc.x - data.x,
        /// data)` or `at_times(…)`.
        residuals: String,
        #[serde(default = "default_fit_tol")]
        tol: f64,
        #[serde(default = "default_budget")]
        budget: f64,
    },
    /// Refinement over one parameter: `points` values over `range`, an
    /// edge between neighbours, each round a new point in the middle of
    /// every interval whose ends' `result` differs by more than `change`,
    /// until intervals are narrower than `tol`, up to `max` points.
    Refine {
        param: String,
        range: [f64; 2],
        /// A reduce formula of each run (`final(bi.x) > 0`).
        result: String,
        #[serde(default = "default_points")]
        points: usize,
        #[serde(default = "default_change")]
        change: f64,
        #[serde(default = "default_refine_tol")]
        tol: f64,
        #[serde(default = "default_max")]
        max: usize,
        #[serde(default = "default_budget")]
        budget: f64,
    },
    /// Natural-parameter continuation: one run a round, `param` stepping
    /// by `step` from `range[0]` to `range[1]` and back, each run starting
    /// from the state the last settled at: its starting-value parameter
    /// `state` (`bi.x0`) set to `settled` (`final(bi.x)`) of the last run,
    /// from `state_start`.
    Continue {
        param: String,
        range: [f64; 2],
        step: f64,
        state: String,
        settled: String,
        #[serde(default)]
        state_start: f64,
        #[serde(default = "default_budget")]
        budget: f64,
    },
}

/// A parameter a strategy varies, from `start`; for pattern search its
/// `span` (the first step is a quarter of it) and bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Varied {
    pub param: String,
    pub start: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
}

fn default_search_tol() -> f64 {
    1e-3
}
fn default_fit_tol() -> f64 {
    1e-9
}
fn default_refine_tol() -> f64 {
    1e-3
}
fn default_budget() -> f64 {
    60.0
}
fn default_points() -> usize {
    9
}
fn default_change() -> f64 {
    0.5
}
fn default_max() -> usize {
    200
}

impl Strategy {
    fn kind(&self) -> &'static str {
        match self {
            Strategy::Minimize { .. } => "search",
            Strategy::Fit { .. } => "fit",
            Strategy::Refine { .. } => "refine",
            Strategy::Continue { .. } => "continuation",
        }
    }

    fn budget(&self) -> f64 {
        match self {
            Strategy::Minimize { budget, .. }
            | Strategy::Fit { budget, .. }
            | Strategy::Refine { budget, .. }
            | Strategy::Continue { budget, .. } => *budget,
        }
    }

    /// The parameters the study varies, in order.
    fn keys(&self) -> Vec<String> {
        match self {
            Strategy::Minimize { params, .. } | Strategy::Fit { params, .. } => {
                params.iter().map(|p| p.param.clone()).collect()
            }
            Strategy::Refine { param, .. } => vec![param.clone()],
            Strategy::Continue { param, state, .. } => vec![param.clone(), state.clone()],
        }
    }

    /// The study's design, its number of points (none for rows) and its
    /// results, by name.
    fn sample(
        &self,
    ) -> (
        simsandbox_file::Design,
        Option<usize>,
        Vec<(String, String)>,
    ) {
        use simsandbox_file::Design;
        match self {
            Strategy::Minimize {
                params, objective, ..
            } => (
                Design::Inputs,
                Some(2 * params.len() + 1),
                vec![("score".into(), objective.clone())],
            ),
            Strategy::Fit {
                params, residuals, ..
            } => (
                Design::Inputs,
                Some(params.len() + 2),
                vec![("res".into(), residuals.clone())],
            ),
            Strategy::Refine { result, .. } => {
                (Design::Rows, None, vec![("result".into(), result.clone())])
            }
            Strategy::Continue { settled, .. } => (
                Design::Inputs,
                Some(1),
                vec![("settled".into(), settled.clone())],
            ),
        }
    }

    fn check(&self) -> Result<(), String> {
        let positive = |v: f64, what: &str| {
            (v.is_finite() && v > 0.0)
                .then_some(())
                .ok_or(format!("{what} must be a positive number, not {v}"))
        };
        positive(self.budget(), "budget")?;
        match self {
            Strategy::Minimize { params, tol, .. } | Strategy::Fit { params, tol, .. } => {
                positive(*tol, "tol")?;
                if params.is_empty() {
                    return Err("give at least one parameter to vary".into());
                }
                for p in params {
                    if !p.start.is_finite() {
                        return Err(format!("{}'s start must be a number", p.param));
                    }
                    if let Some(s) = p.span {
                        positive(s, &format!("{}'s span", p.param))?;
                    }
                }
            }
            Strategy::Refine {
                range,
                points,
                tol,
                max,
                change,
                ..
            } => {
                positive(*tol, "tol")?;
                positive(*change, "change")?;
                if range[0].partial_cmp(&range[1]) != Some(std::cmp::Ordering::Less) {
                    return Err(format!("range [{}, {}] must increase", range[0], range[1]));
                }
                if *points < 2 || points > max {
                    return Err(format!("points must be 2 to max ({max}), not {points}"));
                }
            }
            Strategy::Continue { range, step, .. } => {
                positive(*step, "step")?;
                if range[0].partial_cmp(&range[1]) != Some(std::cmp::Ordering::Less) {
                    return Err(format!("range [{}, {}] must increase", range[0], range[1]));
                }
            }
        }
        let keys = self.keys();
        if keys.iter().enumerate().any(|(i, k)| keys[..i].contains(k)) {
            return Err("each parameter is varied once".into());
        }
        Ok(())
    }
}

/// A study input's port name for a swept key, as the study names it.
fn port(key: &str) -> String {
    key.replace(['.', '/'], "_")
}

/// ` [unit]`, or nothing when dimensionless.
fn bracket(u: &Unit) -> String {
    if *u == Unit::DIMENSIONLESS {
        String::new()
    } else {
        format!(" [{u}]")
    }
}

/// Write `strategy` into `file`, driving the study `target` names: a
/// study block (its model kept, its sample replaced), a used model (a
/// study of it added beside it) or, with none, the whole model (moved
/// into a definition the study runs; the top level becomes the study and
/// the strategy). The model's run becomes one round a step, `budget`
/// rounds. Returns a note naming what it wrote.
pub fn write(
    file: &mut ModelFile,
    registry: &Registry,
    target: Option<&str>,
    strategy: &Strategy,
) -> Result<String, String> {
    strategy.check()?;
    let before = file.clone();
    let result = write_into(file, registry, target, strategy);
    if result.is_err() {
        *file = before;
    }
    result
}

fn write_into(
    file: &mut ModelFile,
    registry: &Registry,
    target: Option<&str>,
    strategy: &Strategy,
) -> Result<String, String> {
    let names =
        |f: &ModelFile| -> Vec<String> { f.blocks.iter().map(|b| b.name.clone()).collect() };
    let study = match target {
        Some(t) => {
            let b = file
                .blocks
                .iter()
                .find(|b| b.name == t)
                .ok_or_else(|| format!("no block '{t}'"))?;
            if b.type_name == simsandbox_file::STUDY {
                t.to_string()
            } else if b.is_use() {
                let had = names(file);
                apply_in(
                    file,
                    &Scope::Model,
                    registry,
                    &Edit::Study {
                        block: t.to_string(),
                    },
                )?;
                names(file)
                    .into_iter()
                    .find(|n| !had.contains(n))
                    .ok_or("the study was not made")?
            } else {
                return Err(format!(
                    "'{t}' is neither a study nor a used model: name one, or none for the whole model"
                ));
            }
        }
        None => {
            // The whole model into a definition, studied; the top level
            // keeps only the study (and the strategy, below).
            let had = names(file);
            let run = file.run.clone();
            apply_in(file, &Scope::Model, registry, &Edit::StudyModel)?;
            let study = file
                .blocks
                .iter()
                .find(|b| b.type_name == simsandbox_file::STUDY && !had.contains(&b.name))
                .map(|b| b.name.clone())
                .ok_or("the study was not made")?;
            // The model keeps its own run (its step and end) inside the
            // definition; the top level's becomes the strategy's rounds.
            let key = file
                .blocks
                .iter()
                .find(|b| b.name == study)
                .and_then(|b| b.model.clone())
                .ok_or("the study names no model")?;
            if let Some(def) = file.models.get_mut(&key) {
                def["run"] = serde_json::to_value(&run).map_err(|e| e.to_string())?;
            }
            let uses: Vec<String> = file
                .blocks
                .iter()
                .filter(|b| b.name != study)
                .map(|b| b.name.clone())
                .collect();
            let node = |key: &str| key.split(['.', '/']).next().unwrap_or(key).to_string();
            file.blocks.retain(|b| b.name == study);
            file.connections
                .retain(|c| !uses.contains(&node(&c.from)) && !uses.contains(&node(&c.to)));
            file.controls = Default::default();
            study
        }
    };
    // The study: its points from the strategy, its results as named.
    let (design, count, reduce) = strategy.sample();
    let keys = strategy.keys();
    let sample: simsandbox_file::Sampling = serde_json::from_value(serde_json::json!({
        "design": design,
        "count": count,
        "params": keys.iter().map(|k| serde_json::json!({"param": k})).collect::<Vec<_>>(),
        "reduce": reduce.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
    }))
    .map_err(|e| e.to_string())?;
    let entry = file
        .blocks
        .iter_mut()
        .find(|b| b.name == study)
        .expect("found above");
    entry.sample = Some(sample);
    // The swept parameters' inputs leave the wires that fed them.
    let ports: Vec<String> = keys.iter().map(|k| port(k)).collect();
    file.connections
        .retain(|c| !ports.iter().any(|p| c.to == format!("{study}.{p}")));
    file.resolve_models().map_err(|e| e.to_string())?;
    // Its ports' units, as the study gives them.
    let units = study_units(file, registry, &study)?;
    let unit = |name: &str| units.get(name).copied().unwrap_or(Unit::DIMENSIONLESS);
    let taken = |n: &str| file.blocks.iter().any(|b| b.name == n);
    let name = match taken(strategy.kind()) {
        true => free_name(strategy.kind(), taken),
        false => strategy.kind().to_string(),
    };
    let at = file
        .blocks
        .iter()
        .find(|b| b.name == study)
        .and_then(|b| b.canvas)
        .map(|[x, y]| [x, y + 220.0]);
    let (equations, params, sent, read, views) = match strategy {
        Strategy::Minimize { params, tol, .. } => minimize(params, *tol, &unit),
        Strategy::Fit { params, tol, .. } => {
            let width = residual_count(file, registry, &study)?;
            fit(params, *tol, width, &unit)
        }
        Strategy::Refine {
            param,
            range,
            points,
            change,
            tol,
            max,
            ..
        } => refine(param, *range, *points, *change, *tol, *max, &unit),
        Strategy::Continue {
            param,
            range,
            step,
            state,
            state_start,
            ..
        } => continuation(param, *range, *step, state, *state_start, &unit),
    };
    let mut block = BlockEntry {
        name: name.clone(),
        type_name: "eq.equations".into(),
        equations,
        canvas: at,
        ..Default::default()
    };
    block.params = params
        .into_iter()
        .map(|(k, v)| (k, ParamValue::Number(v)))
        .collect();
    file.blocks.push(block);
    for (out, input) in sent {
        file.connections.push(ConnectionEntry {
            from: format!("{name}.{out}"),
            to: format!("{study}.{input}"),
        });
    }
    for (out, input) in read {
        file.connections.push(ConnectionEntry {
            from: format!("{study}.{out}"),
            to: format!("{name}.{input}"),
        });
    }
    file.run.dt = 1.0;
    file.run.t_end = Some(strategy.budget());
    for v in views {
        let view: simsandbox_file::ViewEntry = serde_json::from_value(
            v.replace("{s}", &name)
                .parse::<serde_json::Value>()
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        file.controls.views.push(view);
    }
    file.resolve_models().map_err(|e| e.to_string())?;
    Ok(format!(
        "Added {name}, a {} strategy driving {study} ({} rounds at most)",
        match strategy {
            Strategy::Minimize { .. } => "pattern-search",
            Strategy::Fit { .. } => "Levenberg–Marquardt",
            Strategy::Refine { .. } => "refinement",
            Strategy::Continue { .. } => "continuation",
        },
        strategy.budget()
    ))
}

/// The study block as built: its ports' units by name.
fn study_units(
    file: &ModelFile,
    registry: &Registry,
    study: &str,
) -> Result<std::collections::HashMap<String, Unit>, String> {
    let spec = study_spec(file, registry, study)?;
    Ok(spec
        .ports
        .iter()
        .map(|p| (p.name.clone(), p.unit))
        .collect())
}

fn study_spec(
    file: &ModelFile,
    registry: &Registry,
    study: &str,
) -> Result<simsandbox_core::block::BlockSpec, String> {
    let entry = file
        .blocks
        .iter()
        .find(|b| b.name == study)
        .ok_or("no study")?;
    let config = entry.config(file.folder.as_deref());
    let block = crate::study::without_running_studies(|| {
        simsandbox_core::registry::in_folder(file.folder.as_deref(), || {
            registry.build(simsandbox_file::STUDY, study, &config)
        })
    })
    .map_err(|e| format!("{study}: {e}"))?;
    Ok(block.spec().clone())
}

/// How many residuals each run gives: the length of the study's `res`
/// per run.
fn residual_count(file: &ModelFile, registry: &Registry, study: &str) -> Result<usize, String> {
    let spec = study_spec(file, registry, study)?;
    let res = spec
        .ports
        .iter()
        .find(|p| p.name == "res" && p.causality == Causality::Output)
        .ok_or("the study gives no residuals")?;
    match res.shape {
        Shape::Grid { nx, ny } if ny > 1 => Ok(nx),
        _ => Err(
            "the residuals must be a vector per run, e.g. at_samples(osc.x - data.x, data)".into(),
        ),
    }
}

/// What a strategy writes: its equations, parameter values, the outputs
/// it sends (to the study's inputs), the results it reads (from the
/// study's outputs to its inputs), and views (`{s}` its name).
type Written = (
    Vec<String>,
    Vec<(String, f64)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<String>,
);

/// `sum(v*(abs(p - i) < 0.5), p)`: the `i`th of `v` along `p`.
fn nth(v: &str, i: usize) -> String {
    format!("sum({v}*(abs(p - {i}) < 0.5), p)")
}

fn minimize(params: &[Varied], tol: f64, unit: &dyn Fn(&str) -> Unit) -> Written {
    let d = params.len();
    let list: Vec<&str> = params.iter().map(|p| p.param.as_str()).collect();
    let mut eq = vec![
        format!("# Pattern search over {} (ADR 0078): point 0 is the centre, points 2i+1 and 2i+2 a step up and down on parameter i", list.join(", ")),
        format!("grid p = {d}"),
        format!("grid k = {}", 2 * d + 1),
        format!("input score[k]{}", bracket(&unit("score"))),
        "# each parameter's start, span (the first step is a quarter of it) and bounds, by place along p".into(),
        "param x_start[p] = 0".into(),
        "param span[p] = 1".into(),
        "param lo[p] = -1e300".into(),
        "param hi[p] = 1e300".into(),
        format!("param tol = {tol}"),
        "state x[p] = x_start".into(),
        "state step".into(),
        "dir = (abs(k - 2*p - 1) < 0.5) - (abs(k - 2*p - 2) < 0.5)".into(),
        "tx = min(max(x + step*span*dir, lo), hi)".into(),
        "# the best point; the centre once the step is below the tolerance".into(),
        "best = argminof(score)*(step > tol)".into(),
        format!("best_score = minof(score){}", bracket(&unit("score"))),
        "x[n+1] = sum(tx*(abs(k - best) < 0.5), k)".into(),
        "step[n+1] = step*(1 - 0.5*(best < 0.5)*(step > tol))".into(),
        "# the points sent to the study, one input per parameter, in its unit".into(),
    ];
    let mut values = vec![("step0".to_string(), 0.25)];
    let mut sent = Vec::new();
    for (i, v) in params.iter().enumerate() {
        let name = port(&v.param);
        let u = unit(&name);
        let span = v
            .span
            .unwrap_or_else(|| v.range.map_or(4.0 * v.start.abs().max(1.0), |[a, b]| b - a));
        values.push((format!("x_start[{i}]"), v.start));
        values.push((format!("span[{i}]"), span));
        if let Some([a, b]) = v.range {
            values.push((format!("lo[{i}]"), a));
            values.push((format!("hi[{i}]"), b));
        }
        eq.push(unit_line(&name, &u));
        eq.push(format!(
            "{name} = {}*{}{}",
            unit_name(&name),
            nth("tx", i),
            bracket(&u)
        ));
        eq.push(format!(
            "best_{name} = {}*{}{}",
            unit_name(&name),
            nth("x", i),
            bracket(&u)
        ));
        sent.push((name.clone(), name));
    }
    let shown: Vec<String> = params
        .iter()
        .map(|v| format!("\"{{s}}.best_{}\"", port(&v.param)))
        .collect();
    let views = vec![
        r#"{"type": "plot.xy", "bind": {"x": "t", "y": "{s}.best_score"}}"#.to_string(),
        format!(
            r#"{{"type": "plot.values", "bind": {{"values": [{}, "{{s}}.step", "{{s}}.best_score"]}}}}"#,
            shown.join(", ")
        ),
    ];
    (
        eq,
        values,
        sent,
        vec![("score".into(), "score".into())],
        views,
    )
}

/// A parameter carrying an output's unit, so that a dimensionless
/// strategy can send values in it (`param in_cruise_kp [N*s/m] = 1`).
fn unit_name(port: &str) -> String {
    format!("in_{port}")
}

fn unit_line(port: &str, u: &Unit) -> String {
    format!("param {}{} = 1", unit_name(port), bracket(u))
}

fn fit(params: &[Varied], tol: f64, width: usize, unit: &dyn Fn(&str) -> Unit) -> Written {
    let d = params.len();
    let list: Vec<&str> = params.iter().map(|p| p.param.as_str()).collect();
    let mut eq = vec![
        format!("# Levenberg-Marquardt fitting of {} (ADR 0078): point 0 is the centre, point i+1 a small step on parameter i (a forward-difference Jacobian), point {} the candidate", list.join(", "), d + 1),
        format!("grid r = {width}"),
        format!("grid j = {}", d + 2),
        format!("grid p = {d}"),
        format!("grid q = {d}"),
        format!("input res[r, j]{}", bracket(&unit("res"))),
        "param x_start[p] = 0".into(),
        "state x[p] = x_start".into(),
        "state dx[p] = 0".into(),
        "state lam".into(),
        "state done".into(),
        "param rel = 1e-6".into(),
        format!("param tol = {tol}"),
        "param up = 4".into(),
        "param down = 0.25".into(),
        "h = rel*max(abs(x), 1)".into(),
        "pick = abs(j - p - 1) < 0.5".into(),
        format!("cand = j > {}", d as f64 + 0.5),
        "tx = x + h*pick + dx*cand".into(),
        "# residuals at the centre and the candidate, the Jacobian, the costs".into(),
        "r0 = sum(res*(j < 0.5), j)".into(),
        "rc = sum(res*cand, j)".into(),
        "J = (sum(res*pick, j) - r0)/h".into(),
        "cost = sum(r0*r0)".into(),
        "cost_cand = sum(rc*rc)".into(),
        "moved = sum(dx*dx) > 0".into(),
        "better = moved*(cost_cand < cost)".into(),
        "# from the better point, a damped Gauss-Newton step: (JTJ + lam diag JTJ) step = -JT r".into(),
        "base = rc*better + r0*(1 - better)".into(),
        "lam_next = lam*(1 + moved*(better*(down - 1) + (1 - better)*(up - 1)))".into(),
        "JTJ = sum(J*on(J, p, q), r)".into(),
        "A = JTJ*(1 + lam_next*(abs(p - q) < 0.5))".into(),
        "s = on(solve(A, -sum(J*base, r)), q, p)".into(),
        "conditioning = rcond(A)".into(),
        "# done once the step is below the tolerance where nothing improves: then it holds still".into(),
        "small = maxof(abs(s) - tol*max(abs(x), 1)) < 0".into(),
        "stop = max(done, small*(1 - better))".into(),
        "x[n+1] = x + (1 - done)*better*dx".into(),
        "dx[n+1] = (1 - stop)*s".into(),
        "lam[n+1] = lam + (1 - done)*(lam_next - lam)".into(),
        "done[n+1] = stop".into(),
        "# the points sent to the study, one input per parameter, in its unit".into(),
    ];
    let mut values = vec![("lam0".to_string(), 1e-3), ("done0".to_string(), 0.0)];
    let mut sent = Vec::new();
    for (i, v) in params.iter().enumerate() {
        let name = port(&v.param);
        let u = unit(&name);
        values.push((format!("x_start[{i}]"), v.start));
        eq.push(unit_line(&name, &u));
        eq.push(format!(
            "{name} = {}*{}{}",
            unit_name(&name),
            nth("tx", i),
            bracket(&u)
        ));
        eq.push(format!(
            "fit_{name} = {}*{}{}",
            unit_name(&name),
            nth("x", i),
            bracket(&u)
        ));
        sent.push((name.clone(), name));
    }
    let shown: Vec<String> = params
        .iter()
        .map(|v| format!("\"{{s}}.fit_{}\"", port(&v.param)))
        .collect();
    let views = vec![
        r#"{"type": "plot.xy", "bind": {"x": "t", "y": "{s}.cost"}}"#.to_string(),
        format!(
            r#"{{"type": "plot.values", "bind": {{"values": [{}, "{{s}}.cost", "{{s}}.done"]}}}}"#,
            shown.join(", ")
        ),
    ];
    (eq, values, sent, vec![("res".into(), "res".into())], views)
}

fn refine(
    param: &str,
    range: [f64; 2],
    points: usize,
    change: f64,
    tol: f64,
    max: usize,
    unit: &dyn Fn(&str) -> Unit,
) -> Written {
    let name = port(param);
    let (u, ru) = (bracket(&unit(&name)), bracket(&unit("result")));
    let eq = vec![
        format!("# Refinement over {param} (ADR 0078, ADR 0079 stage 2): its values one a row, an edge joining each two neighbours"),
        format!("table pts max {max}"),
        format!("start pts = {points}"),
        format!("param lo_end{u} = {}", range[0]),
        format!("param hi_end{u} = {}", range[1]),
        format!("state {name}[pts]{u} = lo_end + id(pts)*(hi_end - lo_end)/(rows(pts) - 1)"),
        format!("{name}[n+1] = {name}"),
        "edges segs of pts".into(),
        "start segs = chain".into(),
        "# each run's result, by row id, from the study".into(),
        format!("input result[pts]{ru}"),
        format!("param change{ru} = {change}"),
        format!("param tol{u} = {tol}"),
        format!("width = to_end({name}, segs) - from_end({name}, segs)"),
        "across = abs(to_end(result, segs) - from_end(result, segs)) > change".into(),
        "# a new point in the middle of each interval whose ends' results differ by more than change, until it is narrower than tol".into(),
        format!("split segs where across*(width > tol): {name} = mean"),
        format!("lo = sum(across*from_end({name}, segs)){u}"),
        format!("hi = sum(across*to_end({name}, segs)){u}"),
        "points = rows(pts)".into(),
    ];
    let views = vec![
        r#"{"type": "plot.xy", "bind": {"x": "t", "y": ["{s}.lo", "{s}.hi"]}}"#.to_string(),
        r#"{"type": "plot.xy", "bind": {"x": "t", "y": "{s}.points"}}"#.to_string(),
    ];
    (
        eq,
        Vec::new(),
        vec![(name.clone(), name)],
        vec![("result".into(), "result".into())],
        views,
    )
}

fn continuation(
    param: &str,
    range: [f64; 2],
    step: f64,
    state: &str,
    state_start: f64,
    unit: &dyn Fn(&str) -> Unit,
) -> Written {
    let (p, s) = (port(param), port(state));
    let (pu, su) = (bracket(&unit(&p)), bracket(&unit(&s)));
    let eq = vec![
        format!("# Natural-parameter continuation in {param} (ADR 0078): one run a round at the next value, starting {state} where the last run settled, so it follows the branch it is on"),
        "grid k = 1".into(),
        format!("input settled_in[k]{}", bracket(&unit("settled"))),
        format!("state value{pu}"),
        "state dir".into(),
        format!("state x{su}"),
        format!("param v_min{pu} = {}", range[0]),
        format!("param v_max{pu} = {}", range[1]),
        format!("param dv{pu} = {step}"),
        "# the point sent to the study: this value, from the last settled state".into(),
        format!("{p} = value + 0*k{pu}"),
        format!("{s} = x + 0*k{su}"),
        format!("settled = at(settled_in, 0){su}"),
        "# up to v_max, then back down to v_min, and so on".into(),
        "turn = (value + dir*dv > v_max + dv/2) + (value + dir*dv < v_min - dv/2)".into(),
        "dir[n+1] = dir*(1 - 2*turn)".into(),
        "value[n+1] = value + dir*(1 - 2*turn)*dv".into(),
        "x[n+1] = settled".into(),
    ];
    let values = vec![
        ("value0".into(), range[0]),
        ("dir0".into(), 1.0),
        ("x0".into(), state_start),
    ];
    let views = vec![
        r#"{"type": "plot.xy", "bind": {"x": "{s}.value", "y": "{s}.settled"}}"#.to_string(),
        r#"{"type": "plot.xy", "bind": {"x": "t", "y": "{s}.value"}}"#.to_string(),
    ];
    (
        eq,
        values,
        vec![(p.clone(), p), (s.clone(), s)],
        vec![("settled".into(), "settled_in".into())],
        views,
    )
}
