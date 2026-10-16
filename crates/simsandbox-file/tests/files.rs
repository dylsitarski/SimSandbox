//! Loading, saving and validating model files.

use simsandbox_core::graph::Model;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::systems::{CoupledOscillator, Lorenz};
use simsandbox_eq::{Integrator, OdeBlock};
use simsandbox_file::{load, FileError, ModelFile, SCHEMA};

const LORENZ: &str = include_str!("../../../examples/lorenz.ssbx.json");
const OSCILLATORS: &str = include_str!("../../../examples/coupled-oscillators.ssbx.json");

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    r
}

fn invalid_path(text: &str) -> String {
    match load(text, &registry()) {
        Err(FileError::Invalid { path, .. }) => path,
        Err(e) => panic!("expected an Invalid error, got {e}"),
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn examples_load_without_warnings() {
    for text in [LORENZ, OSCILLATORS] {
        let loaded = load(text, &registry()).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        Simulator::new(loaded.model, 0.0).unwrap();
    }
}

#[test]
fn examples_are_written_exactly_as_saved() {
    for text in [LORENZ, OSCILLATORS] {
        let (file, _) = ModelFile::from_json(text).unwrap();
        assert_eq!(file.to_json(), text);
    }
}

#[test]
fn lorenz_from_file_matches_lorenz_from_code() {
    let loaded = load(LORENZ, &registry()).unwrap();
    let id = loaded.ids["lorenz"];
    let mut from_file = Simulator::new(loaded.model, 0.0).unwrap();

    let mut model = Model::new();
    let code_id = model.add(Box::new(OdeBlock::new(
        Lorenz::new("lorenz"),
        Integrator::Rk4 { substeps: 1 },
    )));
    let mut from_code = Simulator::new(model, 0.0).unwrap();

    for _ in 0..1000 {
        from_file.step(0.01).unwrap();
        from_code.step(0.01).unwrap();
    }
    assert_eq!(from_file.state(id), from_code.state(code_id));
}

#[test]
fn coupled_oscillators_from_file_match_the_model_from_code() {
    let loaded = load(OSCILLATORS, &registry()).unwrap();
    let a = loaded.ids["a"];
    let mut from_file = Simulator::new(loaded.model, 0.0).unwrap();

    // The co-simulated pair as built in simsandbox-eq's verification tests.
    let rk4 = Integrator::Rk4 { substeps: 4 };
    let mut model = Model::new();
    let ca = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("a", 1.0),
        rk4,
    )));
    let cb = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("b", 0.0),
        rk4,
    )));
    model.connect((ca, "x"), (cb, "neighbor_x")).unwrap();
    model.connect((cb, "x"), (ca, "neighbor_x")).unwrap();
    let mut from_code = Simulator::new(model, 0.0).unwrap();

    for _ in 0..1000 {
        from_file.step(0.01).unwrap();
        from_code.step(0.01).unwrap();
    }
    assert_eq!(from_file.output(a, "x"), from_code.output(ca, "x"));
}

#[test]
fn captured_params_survive_a_save_and_load() {
    let mut loaded = load(LORENZ, &registry()).unwrap();
    let id = loaded.ids["lorenz"];
    loaded.model.set_param(id, "rho", 150.0).unwrap();
    loaded
        .file
        .capture_params(&loaded.model, &loaded.ids)
        .unwrap();
    let again = load(&loaded.file.to_json(), &registry()).unwrap();
    assert_eq!(
        again.model.param(again.ids["lorenz"], "rho").unwrap(),
        150.0
    );
    assert_eq!(again.file.controls, loaded.file.controls);
}

#[test]
fn omitted_params_take_defaults() {
    let text = r#"{"format": 1, "blocks": [{"name": "l", "type": "eq.lorenz", "method": {"name": "euler"}}]}"#;
    let loaded = load(text, &registry()).unwrap();
    assert_eq!(loaded.model.param(loaded.ids["l"], "rho").unwrap(), 28.0);
}

#[test]
fn syntax_errors_carry_line_and_column() {
    let text = "{\n  \"format\": 1,\n  \"blocks\": [\n    {\"name\": \"l\",}\n  ]\n}";
    match load(text, &registry()) {
        Err(FileError::Syntax { line, .. }) => assert_eq!(line, 4),
        other => panic!("unexpected {:?}", other.err()),
    }
    // Unknown fields are structural errors too, so typos are caught.
    let typo = r#"{"format": 1, "blocks": [], "conections": []}"#;
    assert!(matches!(
        load(typo, &registry()),
        Err(FileError::Syntax { .. })
    ));
}

#[test]
fn unsupported_format_is_rejected() {
    assert!(matches!(
        load(r#"{"format": 2, "blocks": []}"#, &registry()),
        Err(FileError::UnsupportedFormat(2))
    ));
}

#[test]
fn meaning_errors_name_the_offending_path() {
    let block = |extra: &str| {
        format!(
            r#"{{"format": 1, "blocks": [{{"name": "l", "type": "eq.lorenz", "method": {{"name": "rk4"}}{extra}}}]}}"#
        )
    };
    assert_eq!(
        invalid_path(&block(r#", "params": {"rhoo": 1.0}"#)),
        "blocks[0].params.rhoo"
    );
    assert_eq!(
        invalid_path(&block("").replace("eq.lorenz", "eq.lorentz")),
        "blocks[0].type"
    );
    assert_eq!(
        invalid_path(&block("").replace(r#""rk4""#, r#""rk5""#)),
        "blocks[0].method"
    );
    assert_eq!(
        invalid_path(&block("").replace(r#""l""#, r#""l.x""#)),
        "blocks[0].name"
    );

    let two = |conn: &str| {
        format!(
            r#"{{"format": 1, "blocks": [
              {{"name": "a", "type": "eq.coupled_oscillator", "method": {{"name": "rk4"}}}},
              {{"name": "a2", "type": "eq.coupled_oscillator", "method": {{"name": "rk4"}}}}
            ], "connections": [{conn}]}}"#
        )
    };
    assert_eq!(
        invalid_path(&two(r#"{"from": "a.x", "to": "nope.neighbor_x"}"#)),
        "connections[0].to"
    );
    // v [m/s] into neighbor_x [m]: unit mismatch.
    assert_eq!(
        invalid_path(&two(r#"{"from": "a.v", "to": "a2.neighbor_x"}"#)),
        "connections[0]"
    );
    assert_eq!(
        invalid_path(&two(r#"{"from": "a.x", "to": "a2.neighbor_x"}"#).replace("\"a2\"", "\"a\"")),
        "blocks[1].name"
    );
}

#[test]
fn bad_controls_are_dropped_with_warnings() {
    let malformed = LORENZ.replace(r#""speed": 1.0"#, r#""speed": "fast""#);
    let loaded = load(&malformed, &registry()).unwrap();
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert!(loaded.file.controls.is_empty());

    let dangling = LORENZ
        .replace("lorenz.sigma", "lorenz.sigmaa")
        .replace(r#""speed": 1.0"#, r#""speed": -1.0"#)
        .replace(r#""lorenz.xyz""#, r#""lorenz.nope""#)
        .replace("0.0,\n          30.0", "30.0,\n          0.0");
    let loaded = load(&dangling, &registry()).unwrap();
    assert_eq!(loaded.warnings.len(), 4, "{:?}", loaded.warnings);
    let c = &loaded.file.controls;
    assert_eq!(c.params.len(), 1);
    assert!(c.params["lorenz.rho"].slider);
    assert_eq!(c.params["lorenz.rho"].range, None);
    assert_eq!(c.speed, None);
    assert!(c.plots.is_empty());
}

#[test]
fn schema_is_valid_json_and_lists_the_file_fields() {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let mut props: Vec<_> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    props.sort();
    assert_eq!(
        props,
        [
            "blocks",
            "connections",
            "controls",
            "description",
            "format",
            "run"
        ]
    );
    assert_eq!(schema["properties"]["format"]["const"], 1);
}

#[test]
fn run_settings_default_validate_and_round_trip() {
    let bare = r#"{"format": 1, "blocks": [{"name": "l", "type": "eq.lorenz", "method": {"name": "rk4"}}]}"#;
    let (file, _) = ModelFile::from_json(bare).unwrap();
    assert_eq!(file.run.dt, simsandbox_file::DEFAULT_DT);
    assert_eq!(file.run.t_end, None);
    // Saving always writes the run section.
    assert!(file.to_json().contains("\"run\""));

    let with = bare.replace(
        r#""blocks""#,
        r#""run": {"dt": 0.005, "t_end": 12.5}, "blocks""#,
    );
    let (file, _) = ModelFile::from_json(&with).unwrap();
    assert_eq!((file.run.dt, file.run.t_end), (0.005, Some(12.5)));
    let (again, _) = ModelFile::from_json(&file.to_json()).unwrap();
    assert_eq!(again.run, file.run);

    for (bad, path) in [
        (r#""run": {"dt": 0}, "blocks""#, "run.dt"),
        (r#""run": {"dt": -0.1}, "blocks""#, "run.dt"),
        (r#""run": {"t_end": 0}, "blocks""#, "run.t_end"),
    ] {
        assert_eq!(invalid_path(&bare.replace(r#""blocks""#, bad)), path);
    }
    // Unknown run fields are errors: run settings change results.
    assert!(matches!(
        load(
            &bare.replace(r#""blocks""#, r#""run": {"step": 0.1}, "blocks""#),
            &registry()
        ),
        Err(FileError::Syntax { .. })
    ));
}

#[test]
fn views_are_validated_leniently() {
    let loaded = load(LORENZ, &registry()).unwrap();
    let view = loaded.file.controls.view.as_ref().unwrap();
    assert_eq!(view.kind, "3d");
    assert_eq!(view.axes.len(), 3);

    for (from, to) in [
        (r#""kind": "3d""#, r#""kind": "4d""#),
        (r#""lorenz.xyz[2]""#, r#""lorenz.xyz[3]""#),
        (r#""lorenz.xyz[2]""#, r#""lorenz.xyz""#),
        (r#""kind": "3d""#, r#""kind": "phase""#),
    ] {
        let loaded = load(&LORENZ.replace(from, to), &registry()).unwrap();
        assert_eq!(loaded.warnings.len(), 1, "{to}: {:?}", loaded.warnings);
        assert!(loaded.file.controls.view.is_none());
        // The rest of controls survives a bad view.
        assert!(loaded.file.controls.params["lorenz.rho"].slider);
    }

    let bad_camera = LORENZ.replace(r#""zoom": 1.0"#, r#""zoom": 0.0"#);
    let loaded = load(&bad_camera, &registry()).unwrap();
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.file.controls.view.as_ref().unwrap().camera.is_none());

    // Scalar outputs are named without an index.
    let osc = OSCILLATORS.replace(
        r#""speed": 1.0,"#,
        r#""speed": 1.0, "view": {"kind": "phase", "axes": ["a.x", "b.x"]},"#,
    );
    let loaded = load(&osc, &registry()).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.file.controls.view.unwrap().axes, ["a.x", "b.x"]);
}
