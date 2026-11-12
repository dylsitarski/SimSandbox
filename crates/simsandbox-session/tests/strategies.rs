//! Strategies written as models (ADR 0078 point 7): each generator,
//! given the model an example studies, writes a model that reaches what
//! the hand-written example reaches.

use serde_json::{json, Value};
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};
use simsandbox_session::strategies::{write, Strategy};
use simsandbox_session::ModelFile;

fn example(name: &str) -> Value {
    let path = format!(
        "{}/../../examples/{name}.ssbx.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The model an example studies, as a model file of its own.
fn studied(name: &str, model: &str) -> ModelFile {
    let mut m = example(name)["models"][model].clone();
    m["format"] = json!(1);
    ModelFile::from_json(&m.to_string()).unwrap().0
}

fn written(name: &str, model: &str, s: Value) -> (String, String) {
    let r = registry();
    let mut f = studied(name, model);
    let s: Strategy = serde_json::from_value(s).unwrap();
    let note = write(&mut f, &r, None, &s).unwrap();
    (f.to_json(), note)
}

fn last(y: &Results, name: &str) -> f64 {
    *y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .last()
        .unwrap()
}

fn series(y: &Results, name: &str) -> Vec<u64> {
    y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .iter()
        .map(|v| v.to_bits())
        .collect()
}

#[test]
fn minimize_writes_the_pattern_search_of_the_example() {
    let r = registry();
    let (text, note) = written(
        "cruise-search",
        "cruise",
        json!({"kind": "minimize", "budget": 40,
            "objective": "first_time(abs(cruise.e) < 0.5) + (max(car.v) > 26)*1000",
            "params": [
                {"param": "cruise.kp", "start": 400, "span": 1000, "range": [1, 1e300]},
                {"param": "cruise.ki", "start": 40, "span": 400, "range": [1, 1e300]}]}),
    );
    assert!(note.contains("search"), "{note}");
    let mine = run(&text, &r, None, None).unwrap();
    let theirs = run(&example("cruise-search").to_string(), &r, None, None).unwrap();
    assert_eq!(
        series(&mine, "search.best_cruise_kp"),
        series(&theirs, "search.kp")
    );
    assert_eq!(
        series(&mine, "search.best_cruise_ki"),
        series(&theirs, "search.ki")
    );
    assert_eq!(last(&mine, "search.step"), last(&theirs, "search.step"));
}

#[test]
fn fit_writes_levenberg_marquardt_reaching_the_least_squares_fit() {
    let r = registry();
    let (text, _) = written(
        "oscillator-fit",
        "oscillator",
        json!({"kind": "fit", "residuals": "at_samples(osc.x - data.x, data)",
            "params": [{"param": "osc.k", "start": 2}, {"param": "osc.c", "start": 1}]}),
    );
    let mine = run(&text, &r, None, None).unwrap();
    let theirs = run(&example("oscillator-fit").to_string(), &r, None, None).unwrap();
    // SciPy's least_squares: k = 3.991282, c = 0.501779.
    let (k, c) = (last(&mine, "fit.fit_osc_k"), last(&mine, "fit.fit_osc_c"));
    assert!(
        (k - 3.991282).abs() < 1e-5 && (c - 0.501779).abs() < 1e-5,
        "{k} {c}"
    );
    assert_eq!(k, last(&theirs, "fit.k"));
    assert_eq!(c, last(&theirs, "fit.c"));
    assert_eq!(last(&mine, "fit.done"), 1.0);
}

#[test]
fn refine_writes_the_refinement_of_the_example() {
    let r = registry();
    let (text, _) = written(
        "bistable-refinement",
        "bistable",
        json!({"kind": "refine", "param": "bi.x0", "range": [-2, 2],
            "result": "final(bi.x) > 0", "budget": 15}),
    );
    let mine = run(&text, &r, None, None).unwrap();
    let theirs = run(&example("bistable-refinement").to_string(), &r, None, None).unwrap();
    for n in ["lo", "hi", "points"] {
        assert_eq!(
            series(&mine, &format!("refine.{n}")),
            series(&theirs, &format!("refine.{n}")),
            "{n}"
        );
    }
}

#[test]
fn continue_writes_the_continuation_of_the_example() {
    let r = registry();
    let (text, _) = written(
        "bistable-continuation",
        "bistable",
        json!({"kind": "continue", "param": "bi.r", "range": [-1, 1], "step": 0.02,
            "state": "bi.x0", "settled": "final(bi.x)", "state_start": -1.3, "budget": 200}),
    );
    let mine = run(&text, &r, None, None).unwrap();
    let theirs = run(
        &example("bistable-continuation").to_string(),
        &r,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        series(&mine, "continuation.value"),
        series(&theirs, "cont.r")
    );
    assert_eq!(
        series(&mine, "continuation.settled"),
        series(&theirs, "cont.settled")
    );
}

#[test]
fn strategies_drive_a_used_model_or_a_study_and_refuse_mistakes() {
    let r = registry();
    // A used model: a study of it is added beside it, the use kept.
    let mut f = ModelFile::from_json(
        &json!({"format": 1, "models": {"bistable": example("bistable-refinement")["models"]["bistable"]},
            "blocks": [{"name": "b", "model": "bistable"}]})
        .to_string(),
    )
    .unwrap()
    .0;
    let s: Strategy = serde_json::from_value(json!({"kind": "refine", "param": "bi.x0",
        "range": [-2, 2], "result": "final(bi.x) > 0", "budget": 12}))
    .unwrap();
    write(&mut f, &r, Some("b"), &s).unwrap();
    assert!(f.blocks.iter().any(|b| b.name == "b"));
    assert!(f.blocks.iter().any(|b| b.name == "b_study"));
    let y = run(&f.to_json(), &r, None, None).unwrap();
    assert!((last(&y, "refine.hi") - last(&y, "refine.lo")) <= 1e-3);
    // Mistakes leave the model as it was.
    let before = f.clone();
    let bad = |s: Value| -> String {
        let mut g = before.clone();
        let e = write(
            &mut g,
            &r,
            Some("b_study"),
            &serde_json::from_value(s).unwrap(),
        )
        .unwrap_err();
        assert_eq!(g.to_json(), before.to_json());
        e
    };
    assert!(
        bad(json!({"kind": "minimize", "objective": "final(bi.x)", "params": []}))
            .contains("at least one")
    );
    assert!(
        bad(json!({"kind": "refine", "param": "bi.x0", "range": [2, -2],
        "result": "final(bi.x)"}))
        .contains("must increase")
    );
    assert!(bad(json!({"kind": "fit", "residuals": "final(bi.x)",
        "params": [{"param": "bi.x0", "start": 1}]}))
    .contains("vector per run"));
    let mut g = before.clone();
    assert!(write(&mut g, &r, Some("nothing"), &s)
        .unwrap_err()
        .contains("no block"));
}

#[test]
fn the_search_edit_writes_a_strategy_for_a_study() {
    // The app's "Search…" (ADR 0078 point 7): the same writer, by an edit.
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let r = registry();
    let mut f = ModelFile::from_json(
        &json!({"format": 1, "models": {"bistable": example("bistable-refinement")["models"]["bistable"]},
            "blocks": [{"name": "trials", "type": "study", "model": "bistable",
                "sample": {"design": "grid", "params": [{"param": "bi.x0", "range": [-2, 2]}]}}]})
        .to_string(),
    )
    .unwrap()
    .0;
    let spec = json!({"kind": "refine", "param": "bi.x0", "range": [-2, 2],
        "result": "final(bi.x) > 0", "budget": 12});
    let note = apply_in(
        &mut f,
        &Scope::Model,
        &r,
        &Edit::Search {
            study: "trials".into(),
            strategy: spec.to_string(),
        },
    )
    .unwrap();
    assert!(
        note.contains("refinement strategy driving trials"),
        "{note}"
    );
    let y = run(&f.to_json(), &r, None, None).unwrap();
    assert!(last(&y, "refine.hi") - last(&y, "refine.lo") <= 1e-3);
}

/// Refinement over two and three parameters (ADR 0085 point 7): the
/// strategy written for a whole model whose result is whether a point
/// lies inside a circle (a sphere) settles with the cells across the
/// boundary narrow, and every point's result right.
#[test]
fn refine_writes_a_mesh_over_two_or_three_parameters() {
    let r = registry();
    for (names, points, tol) in [(vec!["u", "v"], 5, 0.02), (vec!["u", "v", "w"], 3, 0.15)] {
        let centre = [0.1, 0.05, -0.07];
        let dist: Vec<String> = names
            .iter()
            .zip(centre)
            .map(|(n, c)| format!("({n} - {c})^2"))
            .collect();
        let mut eqs: Vec<String> = names.iter().map(|n| format!("param {n} = 0")).collect();
        eqs.push(format!("inside = ({}) < 0.36", dist.join(" + ")));
        let model = json!({"format": 1, "run": {"dt": 1, "t_end": 1},
            "blocks": [{"name": "f", "type": "eq.equations", "equations": eqs}]});
        let mut f = ModelFile::from_json(&model.to_string()).unwrap().0;
        let params: Vec<Value> = names
            .iter()
            .map(|n| json!({"param": format!("f.{n}"), "range": [-1.0, 1.0]}))
            .collect();
        let s: Strategy = serde_json::from_value(json!({"kind": "refine", "params": params,
            "result": "final(f.inside)", "points": points, "tol": tol, "max": 5000, "budget": 40}))
        .unwrap();
        let note = write(&mut f, &r, None, &s).unwrap();
        assert!(note.contains("refinement"), "{note}");
        let y = run(&f.to_json(), &r, None, None).unwrap();
        let count: Vec<f64> = y
            .outputs
            .iter()
            .find(|o| o.name == "refine.points")
            .unwrap()
            .values
            .clone();
        let n = *count.last().unwrap();
        assert!(
            n > count[0] && count[count.len() - 2] == n,
            "{names:?}: {count:?}"
        );
        // Every point's result as the circle (sphere) has it.
        let rows = |name: &str| y.populations.iter().find(|p| p.name == name).unwrap();
        let at = count.len() - 1;
        let coords: Vec<Vec<f64>> = names
            .iter()
            .map(|nm| {
                rows(&format!("refine.f_{nm}"))
                    .rows_at(at)
                    .unwrap()
                    .2
                    .to_vec()
            })
            .collect();
        let study = y
            .populations
            .iter()
            .find(|p| p.name.ends_with(".result") && !p.name.starts_with("refine"))
            .unwrap();
        let (_, _, results) = study.rows_at(at).unwrap();
        assert_eq!(results.len() as f64, n);
        for (k, &got) in results.iter().enumerate() {
            let d2: f64 = (0..names.len())
                .map(|i| (coords[i][k] - centre[i]).powi(2))
                .sum();
            assert_eq!(got, f64::from(u8::from(d2 < 0.36)));
        }
        assert!(last(&y, "refine.changing") > 0.0);
    }
    // Mistakes are explained.
    let refuse = |s: Value| {
        let s: Strategy = serde_json::from_value(s).unwrap();
        let mut f = studied("bistable-refinement", "bistable");
        write(&mut f, &r, None, &s).unwrap_err()
    };
    let one = json!({"param": "bi.x0", "range": [0.0, 1.0]});
    assert!(
        refuse(json!({"kind": "refine", "result": "1", "params": [one]})).contains("two or more")
    );
    let two =
        json!([{"param": "bi.x0", "range": [0.0, 1.0]}, {"param": "bi.r", "range": [0.0, 1.0]}]);
    assert!(
        refuse(json!({"kind": "refine", "result": "1", "params": two, "points": 30}))
            .contains("more than max")
    );
    assert!(
        refuse(json!({"kind": "refine", "result": "1", "param": "bi.x0"}))
            .contains("'param' and 'range'")
    );
}
