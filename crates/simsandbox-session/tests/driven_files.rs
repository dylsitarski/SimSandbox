//! Parameters driven by wires in model files (ADR 0075): blocks, used
//! models' declared parameters, studies, and what is refused.

use serde_json::{json, Value};
use simsandbox_session::compose::run_composition;
use simsandbox_session::describe::check;
use simsandbox_session::examples::{registry, EXAMPLES};
use simsandbox_session::run::run;

fn example(part: &str) -> Value {
    let text = EXAMPLES
        .iter()
        .find(|e| e.name.contains(part))
        .unwrap()
        .text;
    serde_json::from_str(text).unwrap()
}

fn values(r: &simsandbox_session::run::Results, name: &str) -> Vec<f64> {
    r.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .clone()
}

fn refused(file: &Value, says: &str) {
    let report = check(&file.to_string(), &registry());
    assert_eq!(report["ok"], false, "{report}");
    let message = report["errors"][0]["message"].as_str().unwrap_or_default();
    assert!(message.contains(says), "{message}");
}

#[test]
fn a_wire_into_a_block_parameter_drives_it() {
    let file = json!({"format": 1,
        "blocks": [
            {"name": "clock", "type": "eq.equations", "equations": [
                "param a [1/s^2] = 0.5", "state tau [s] = 0", "dtau/dt = 1", "k = a*tau [1/s]"]},
            {"name": "decay", "type": "eq.equations", "equations": [
                "param k [1/s] = 100", "state x = 1", "dx/dt = -k*x"]}],
        "connections": [{"from": "clock.k", "to": "decay.k"}],
        "run": {"dt": 0.01, "t_end": 2.0}});
    let report = check(&file.to_string(), &registry());
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(
        report["driven"],
        json!([{"param": "decay.k", "from": "clock.k"}])
    );
    let k = report["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "decay.k")
        .unwrap();
    assert_eq!(k["driven_by"], "clock.k");
    let r = run(&file.to_string(), &registry(), None, None).unwrap();
    let x = values(&r, "decay.x");
    let t = &r.index[0].1;
    let err = x
        .iter()
        .zip(t)
        .map(|(x, t)| (x - (-0.25 * t * t).exp()).abs())
        .fold(0.0, f64::max);
    assert!(err < 1e-8, "{err}");
    // Set and wired at once is refused.
    let mut set = file.clone();
    set["blocks"][1]["params"] = json!({"k": 2.0});
    refused(&set, "set or wired, not both");
}

#[test]
fn a_wire_into_a_used_models_parameter_drives_what_is_set_to_it() {
    let mut platoon = example("Platoon");
    platoon["blocks"].as_array_mut().unwrap().push(
        json!({"name": "load", "type": "eq.equations", "equations": [
            "param m0 [kg] = 1000", "param rate [kg/s] = 100", "state s [s] = 0",
            "ds/dt = 1", "mass = m0 + rate*s"]}),
    );
    platoon["connections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"from": "load.mass", "to": "car2/mass"}));
    // car2 sets its own mass: refused until it does not.
    refused(&platoon, "set or wired, not both");
    for b in platoon["blocks"].as_array_mut().unwrap() {
        if b["name"] == "car2" {
            b["params"].as_object_mut().unwrap().remove("mass");
        }
    }
    let report = check(&platoon.to_string(), &registry());
    assert_eq!(report["ok"], true, "{report}");
    let driven: Vec<(String, String)> = report["params"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| !p["driven_by"].is_null())
        .map(|p| {
            (
                p["key"].as_str().unwrap().into(),
                p["driven_by"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(
        driven.contains(&("car2/car.m".into(), "load.mass".into())),
        "{driven:?}"
    );
    assert!(
        driven.contains(&("car2.mass".into(), "load.mass".into())),
        "{driven:?}"
    );
    let c = run_composition(&platoon.to_string(), &registry(), Some(5.0)).unwrap();
    assert!(c.values("car2/car.v").is_some());
    // A formula reading the parameter otherwise: not driven, refused.
    let mut doubled = platoon.clone();
    doubled["models"]["follower"]["blocks"][0]["params"]["m"] = json!("2*mass");
    refused(&doubled, "cannot drive its parameter mass");
}

#[test]
fn a_wire_into_a_studys_parameter_runs_it_again() {
    let mut curve = example("Resonance curve");
    let study = curve["blocks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|b| b["name"] == "curve")
        .unwrap();
    study["params"].as_object_mut().unwrap().remove("m");
    curve["blocks"].as_array_mut().unwrap().push(
        json!({"name": "mass", "type": "eq.equations", "equations": [
            "param m0 [kg] = 2", "m = m0*1"]}),
    );
    curve["connections"] = json!([{"from": "mass.m", "to": "curve.m"}]);
    let r = run(&curve.to_string(), &registry(), None, None).unwrap();
    let (w, a) = (values(&r, "curve.w"), values(&r, "curve.spring_amplitude"));
    let peak = a
        .iter()
        .enumerate()
        .max_by(|x, y| x.1.total_cmp(y.1))
        .unwrap()
        .0;
    // Twice the mass: the resonance at 1/sqrt(2) of the spring's own.
    assert!((w[peak] - 0.5f64.sqrt()).abs() < 0.01, "{}", w[peak]);
}

#[test]
fn a_models_inputs_outputs_and_parameters_share_their_names() {
    let file = json!({"format": 1,
        "params": {"g": {"value": 1.0}},
        "blocks": [{"name": "b", "type": "eq.equations", "equations": ["input u", "y = u*g"],
                    "params": {"g": "g"}}],
        "inputs": [{"name": "g", "to": ["b.u"]}]});
    refused(&file, "is a parameter of the model already");
}

#[test]
fn the_forced_spring_swings_as_its_steady_state_predicts() {
    // One spring model: `held` sets F, `shaken` has F driven by a sine.
    let text = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Forced spring"))
        .unwrap()
        .text;
    let report = check(text, &registry());
    assert_eq!(report["ok"], true, "{report}");
    let c = run_composition(text, &registry(), Some(80.0)).unwrap();
    // One shared clock: one model's results, indexed by time.
    let t = c.models[0].1.index[0].1.clone();
    let x = c.values("shaken/mass.x").unwrap();
    // F0 = 1, w = 1, k = 1, m = 1, c = 0.2: 1/sqrt((k - m w^2)^2 + (c w)^2).
    let swing = x
        .iter()
        .zip(&t)
        .filter(|(_, t)| **t > 70.0)
        .map(|(x, _)| x.abs())
        .fold(0.0, f64::max);
    assert!((swing - 5.0).abs() < 0.01, "{swing}");
    let held = c.values("held/mass.x").unwrap();
    assert!((held.last().unwrap() - 1.0).abs() < 1e-3);
}
