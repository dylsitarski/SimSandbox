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
