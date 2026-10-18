//! Loading, saving and validating model files.

use simsandbox_core::graph::BlockId;
use simsandbox_core::graph::Model;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::integrators::GroupIntegrator;
use simsandbox_eq::systems::{CoupledOscillator, Lorenz};
use simsandbox_eq::{Integrator, OdeBlock};
use simsandbox_file::{load, Binding, FileError, FusedGroup, ModelFile, SCHEMA};

const LORENZ: &str = include_str!("../../../examples/lorenz.ssbx.json");
const OSCILLATORS: &str = include_str!("../../../examples/coupled-oscillators.ssbx.json");
const PREDATOR_PREY: &str = include_str!("../../../examples/predator-prey.ssbx.json");
const LOGISTIC: &str = include_str!("../../../examples/logistic-map.ssbx.json");
const RESONANCE: &str = include_str!("../../../examples/resonance.ssbx.json");
const HEAT_ROD: &str = include_str!("../../../examples/heat-rod.ssbx.json");
const ORBITS: &str = include_str!("../../../examples/orbits.ssbx.json");
const PENDULUM: &str = include_str!("../../../examples/pendulum.ssbx.json");
const RESONANCE_MAP: &str = include_str!("../../../examples/resonance-map.ssbx.json");

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
    for text in [
        LORENZ,
        OSCILLATORS,
        PREDATOR_PREY,
        LOGISTIC,
        RESONANCE,
        HEAT_ROD,
        ORBITS,
        PENDULUM,
        RESONANCE_MAP,
    ] {
        let loaded = load(text, &registry()).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        Simulator::new(loaded.model, 0.0).unwrap();
    }
}

#[test]
fn examples_are_written_exactly_as_saved() {
    for text in [
        LORENZ,
        OSCILLATORS,
        PREDATOR_PREY,
        LOGISTIC,
        RESONANCE,
        HEAT_ROD,
        ORBITS,
        PENDULUM,
        RESONANCE_MAP,
    ] {
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

/// The oscillator pair built in code, fused by the master or not.
fn oscillators_from_code(fused: bool) -> (Simulator, BlockId) {
    let rk4 = Integrator::Rk4 { substeps: 4 };
    let mut model = Model::new();
    let a = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("a", 1.0),
        rk4,
    )));
    let b = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("b", 0.0),
        rk4,
    )));
    model.connect((a, "x"), (b, "neighbor_x")).unwrap();
    model.connect((b, "x"), (a, "neighbor_x")).unwrap();
    if fused {
        model
            .fuse(&[a, b], Box::new(GroupIntegrator::new(rk4)))
            .unwrap();
    }
    (Simulator::new(model, 0.0).unwrap(), a)
}

fn run_both(text: &str, fused: bool) -> (Option<Vec<f64>>, Option<Vec<f64>>) {
    let loaded = load(text, &registry()).unwrap();
    let a = loaded.ids["a"];
    let mut from_file = Simulator::new(loaded.model, 0.0).unwrap();
    let (mut from_code, ca) = oscillators_from_code(fused);
    for _ in 0..1000 {
        from_file.step(0.01).unwrap();
        from_code.step(0.01).unwrap();
    }
    (
        from_file.output(a, "x").map(<[f64]>::to_vec),
        from_code.output(ca, "x").map(<[f64]>::to_vec),
    )
}

#[test]
fn connected_equation_blocks_with_one_method_are_fused_automatically() {
    let loaded = load(OSCILLATORS, &registry()).unwrap();
    assert_eq!(
        loaded.groups,
        [FusedGroup {
            blocks: vec!["a".into(), "b".into()],
            method: "RK4 (4 substeps)".into(),
            explicit: false,
        }]
    );
    let (file, code) = run_both(OSCILLATORS, true);
    assert_eq!(file, code);
}

#[test]
fn fusion_off_co_simulates_and_explicit_groups_choose_the_method() {
    let off = OSCILLATORS.replacen(
        "\"connections\"",
        "\"fusion\": \"off\",\n  \"connections\"",
        1,
    );
    assert!(load(&off, &registry()).unwrap().groups.is_empty());
    let (file, code) = run_both(&off, false);
    assert_eq!(file, code);

    // An explicit group overrides the blocks' own methods.
    let grouped = off.replacen(
        "\"connections\"",
        "\"groups\": [{\"blocks\": [\"b\", \"a\"], \"method\": {\"name\": \"dopri5\"}}],\n  \"connections\"",
        1,
    );
    let loaded = load(&grouped, &registry()).unwrap();
    assert_eq!(loaded.groups.len(), 1);
    assert!(loaded.groups[0].explicit);
    assert!(
        loaded.groups[0].method.starts_with("Dormand"),
        "{}",
        loaded.groups[0].method
    );
    // Saving keeps both settings.
    let saved = loaded.file.to_json();
    assert!(saved.contains("\"fusion\": \"off\""), "{saved}");
    assert!(saved.contains("\"groups\""), "{saved}");
}

#[test]
fn blocks_with_different_methods_stay_separate() {
    let mixed = OSCILLATORS.replacen("\"substeps\": 4.0", "\"substeps\": 2.0", 1);
    assert!(load(&mixed, &registry()).unwrap().groups.is_empty());
}

#[test]
fn bad_groups_are_load_errors() {
    let with = |groups: &str| {
        OSCILLATORS.replacen(
            "\"connections\"",
            &format!("\"groups\": {groups},\n  \"connections\""),
            1,
        )
    };
    assert_eq!(
        invalid_path(&with(
            r#"[{"blocks": ["a", "z"], "method": {"name": "rk4"}}]"#
        )),
        "groups[0].blocks[1]"
    );
    assert_eq!(
        invalid_path(&with(
            r#"[{"blocks": ["a"], "method": {"name": "rk4"}}, {"blocks": ["a"], "method": {"name": "rk4"}}]"#
        )),
        "groups[1].blocks[0]"
    );
    assert_eq!(
        invalid_path(&with(r#"[{"blocks": ["a"], "method": {"name": "rk5"}}]"#)),
        "groups[0].method"
    );
    assert_eq!(
        invalid_path(&with(r#"[{"blocks": [], "method": {"name": "rk4"}}]"#)),
        "groups[0].blocks"
    );
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
            "fusion",
            "groups",
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
    let v = &loaded.file.controls.views[0];
    assert_eq!(v.type_name, "plot.3d");
    assert_eq!(v.bind["colour"], Binding::One("t".into()));

    let views_before = loaded.file.controls.views.len();
    // (replacement, views dropped): both Lorenz views use xyz[2]; only the
    // 3D view has a colour slot.
    for (from, to, dropped) in [
        (r#""lorenz.xyz[2]""#, r#""lorenz.xyz[3]""#, 2),
        (r#""lorenz.xyz[2]""#, r#""lorenz.xyz""#, 2),
        (r#""colour": "t""#, r#""colour": "time""#, 1),
    ] {
        let loaded = load(&LORENZ.replace(from, to), &registry()).unwrap();
        assert_eq!(
            loaded.warnings.len(),
            dropped,
            "{to}: {:?}",
            loaded.warnings
        );
        assert_eq!(loaded.file.controls.views.len(), views_before - dropped);
        // The rest of controls survives a bad view.
        assert!(loaded.file.controls.params["lorenz.rho"].slider);
    }

    let bad_camera = LORENZ.replace(r#""zoom": 1.0"#, r#""zoom": 0.0"#);
    let loaded = load(&bad_camera, &registry()).unwrap();
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.file.controls.views[0].camera.is_none());

    // Slots that take several signals use a list; scalar outputs have no index.
    let osc = OSCILLATORS.replace(
        r#""type": "plot.xy",
        "bind": {
          "x": "t","#,
        r#""type": "plot.xy", "style": {"w": 2.0},
        "bind": {
          "x": "t","#,
    );
    let loaded = load(&osc, &registry()).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let v = &loaded.file.controls.views[0];
    assert_eq!(v.bind["y"].signals(), ["a.x", "b.x"]);
    assert_eq!(v.style["w"], 2.0);
}

#[test]
fn the_single_view_of_adr_0014_is_converted() {
    let with_view = |view: &str| {
        format!(
            r#"{{"format": 1, "blocks": [
              {{"name": "a", "type": "eq.coupled_oscillator", "method": {{"name": "rk4"}}}},
              {{"name": "b", "type": "eq.coupled_oscillator", "method": {{"name": "rk4"}}}}
            ], "connections": [
              {{"from": "a.x", "to": "b.neighbor_x"}}, {{"from": "b.x", "to": "a.neighbor_x"}}
            ], "controls": {{"view": {view}}}}}"#
        )
    };
    let loaded = load(
        &with_view(r#"{"kind": "phase", "axes": ["a.x", "b.x"]}"#),
        &registry(),
    )
    .unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let c = &loaded.file.controls;
    assert!(c.view.is_none());
    assert_eq!(c.views.len(), 1);
    assert_eq!(c.views[0].type_name, "plot.xy");
    assert_eq!(c.views[0].bind["x"], Binding::One("a.x".into()));
    assert_eq!(c.views[0].bind["colour"], Binding::One("t".into()));
    // Saving writes the new form only.
    let saved = loaded.file.to_json();
    assert!(saved.contains("\"views\"") && !saved.contains("\"view\""));

    let bad = load(&with_view(r#"{"kind": "4d", "axes": []}"#), &registry()).unwrap();
    assert_eq!(bad.warnings.len(), 1);
    assert!(bad.file.controls.views.is_empty());

    // When a file has both, `views` wins.
    let both = OSCILLATORS.replace(
        r#""speed": 1.0,"#,
        r#""speed": 1.0, "view": {"kind": "time", "axes": []},"#,
    );
    let loaded = load(&both, &registry()).unwrap();
    assert_eq!(loaded.file.controls.views.len(), 3);
}

#[test]
fn typed_equations_load_and_report_mistakes_at_their_line() {
    let loaded = load(PREDATOR_PREY, &registry()).unwrap();
    let id = loaded.ids["ecosystem"];
    assert_eq!(loaded.model.param(id, "alpha").unwrap(), 1.1);
    let spec = loaded.model.block(id).unwrap().spec();
    let outs: Vec<_> = spec.outputs().map(|p| p.name.as_str()).collect();
    assert_eq!(outs, ["prey", "predators", "total"]);

    let broken = PREDATOR_PREY.replace("alpha*prey - beta", "alpha*prey -- beta *");
    match load(&broken, &registry()) {
        Err(FileError::Invalid { path, message }) => {
            assert_eq!(path, "blocks[0].equations");
            assert!(message.starts_with("line 2, column"), "{message}");
        }
        other => panic!("unexpected {:?}", other.err()),
    }
    // Built-in types take no equations.
    let fixed = LORENZ.replace(
        r#""type": "eq.lorenz","#,
        r#""type": "eq.lorenz", "equations": ["dx/dt = 1"],"#,
    );
    assert!(matches!(
        load(&fixed, &registry()),
        Err(FileError::Invalid { path, .. }) if path == "blocks[0].equations"
    ));
}

#[test]
fn time_and_phase_views_are_converted_to_xy_plots() {
    let old = OSCILLATORS
        .replacen(
            r#""type": "plot.xy",
        "bind": {
          "x": "t","#,
            r#""type": "plot.time",
        "bind": {"#,
            1,
        )
        .replacen(r#""type": "plot.xy""#, r#""type": "plot.phase""#, 1);
    assert!(old.contains("plot.time") && old.contains("plot.phase"));
    let loaded = load(&old, &registry()).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    // Saving writes exactly the example, which uses plot.xy.
    assert_eq!(loaded.file.to_json(), OSCILLATORS);
}

#[test]
fn old_heat_map_placement_is_converted_to_axes() {
    let new = r#""x": "t",
          "y": "rod.position""#;
    assert!(HEAT_ROD.contains(new), "the example uses the axes");
    let old = HEAT_ROD.replace(new, r#""position": "rod.position""#);
    let loaded = load(&old, &registry()).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.file.to_json(), HEAT_ROD);
    // Time upwards: the position runs across instead.
    let upwards = old.replace(
        r#""position": "rod.position""#,
        r#""position": "rod.position"
        },
        "style": {
          "time_axis": 1.0"#,
    );
    let loaded = load(&upwards, &registry()).unwrap();
    let bind = &loaded.file.controls.views[0].bind;
    assert_eq!(bind["x"], Binding::One("rod.position".into()));
    assert_eq!(bind["y"], Binding::One("t".into()));
    assert!(loaded.file.controls.views[0].style.is_empty());
}

#[test]
fn sweeps_are_read_and_checked_leniently() {
    let loaded = load(RESONANCE, &registry()).unwrap();
    let [sw] = loaded.file.controls.sweep.as_slice() else {
        panic!("one swept parameter");
    };
    assert_eq!(
        (sw.param.as_str(), sw.range, sw.points),
        ("spring.w", Some([0.0, 3.0]), Some(600))
    );

    let unknown = load(
        &RESONANCE.replace(r#""param": "spring.w""#, r#""param": "spring.q""#),
        &registry(),
    )
    .unwrap();
    assert_eq!(unknown.warnings.len(), 1);
    assert!(unknown.file.controls.sweep.is_empty());
    let bad_range = RESONANCE.replace("0.0,\n        3.0", "3.0,\n        0.0");
    let loaded = load(&bad_range, &registry()).unwrap();
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    assert_eq!(loaded.file.controls.sweep[0].range, None);

    // Two parameters are a list; a third, a repeat or too many points are
    // dropped with warnings.
    let one = r#""sweep": {
      "param": "spring.w",
      "range": [
        0.0,
        3.0
      ],
      "points": 600
    }"#;
    assert!(RESONANCE.contains(one));
    let with = |sweep: &str| {
        load(
            &RESONANCE.replace(one, &format!("\"sweep\": {sweep}")),
            &registry(),
        )
        .unwrap()
    };
    let two = with(
        r#"[{"param": "spring.w", "range": [0.0, 3.0]}, {"param": "spring.c", "points": 50}]"#,
    );
    assert!(two.warnings.is_empty(), "{:?}", two.warnings);
    assert_eq!(two.file.controls.sweep.len(), 2);
    assert!(two.file.to_json().contains("\"sweep\": [\n"));
    let extra = with(
        r#"[{"param": "spring.w"}, {"param": "spring.w"}, {"param": "spring.c"}, {"param": "spring.k"}]"#,
    );
    assert_eq!(extra.warnings.len(), 2, "{:?}", extra.warnings);
    assert_eq!(extra.file.controls.sweep.len(), 2);
    let huge =
        with(r#"[{"param": "spring.w", "points": 1000}, {"param": "spring.c", "points": 1000}]"#);
    assert_eq!(huge.warnings.len(), 1, "{:?}", huge.warnings);
    assert_eq!(huge.file.controls.sweep[0].points, None);

    // Maps and formulas need the matching method.
    let wrong = LOGISTIC.replace(r#""name": "iterate""#, r#""name": "rk4""#);
    assert!(matches!(
        load(&wrong, &registry()),
        Err(FileError::Invalid { path, .. }) if path == "blocks[0].method"
    ));
}
