use super::files::*;
use super::*;

#[test]
fn save_names_get_the_model_extension() {
    assert_eq!(
        default_file_name("Lorenz system"),
        "lorenz-system.ssbx.json"
    );
    assert_eq!(
        default_file_name("Coupled oscillators"),
        "coupled-oscillators.ssbx.json"
    );
    assert_eq!(default_file_name("my.ssbx.json"), "my.ssbx.json");
    assert_eq!(default_file_name("!!!"), "model.ssbx.json");
    assert_eq!(
        with_model_extension("a/b".into()),
        PathBuf::from("a/b.ssbx.json")
    );
    assert_eq!(
        with_model_extension("a/b.json".into()),
        PathBuf::from("a/b.json")
    );
    assert_eq!(
        with_model_extension("a/b.JSON".into()),
        PathBuf::from("a/b.JSON")
    );
}

#[test]
fn saving_an_example_then_reopening_it_keeps_the_path() {
    let dir = std::env::temp_dir().join(format!("simsandbox-app-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lorenz.ssbx.json");

    let mut app = SimApp::new(None, None, None, None);
    // Without a file the app opens on its landing page.
    app.open_example(0);
    app.mark_saved();
    assert!(app.path.is_none(), "built-in examples have no path");
    app.save_to(&path);
    assert_eq!(app.error, None);
    assert_eq!(app.path.as_deref(), Some(path.as_path()));

    let mut reopened = SimApp::new(Some(path.to_string_lossy().into_owned()), None, None, None);
    assert_eq!(reopened.error, None);
    assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
    assert_eq!(reopened.views.tiles[0].state.type_name, "plot.3d");

    // A failed open keeps the previous file as the save target.
    reopened.open_path(&dir.join("missing.ssbx.json").to_string_lossy());
    assert!(reopened.error.is_some());
    assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}

fn editor(block: Option<&str>, text: &str) -> Option<Editor> {
    Some(Editor {
        block: block.map(str::to_string),
        text: text.into(),
        error: None,
    })
}

#[test]
fn new_models_run_from_typed_equations() {
    let mut app = SimApp::new(None, None, None, None);
    app.editor = editor(None, NEW_MODEL);
    app.apply_equations();
    assert!(app.editor.is_none(), "applied editors close");
    let live = app.session.as_ref().unwrap();
    assert_eq!(live.model().title, "Untitled");
    assert_eq!(
        live.history().names(),
        ["model.x", "model.v", "model.energy"]
    );
    assert!(app.path.is_none());
}

#[test]
fn fusion_can_be_switched_off_and_on_keeping_parameters() {
    let mut app = SimApp::new(None, None, None, None);
    let e = &EXAMPLES[1];
    assert!(e.name.contains("oscillators"));
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    assert_eq!(m.groups.len(), 1, "the example is fused by default");
    let id = live.call(|s, _| s.ids["a"]).unwrap();
    live.call(move |s, _| s.set_param(id, "kappa", 0.8))
        .unwrap()
        .unwrap();

    app.set_fusion(false);
    let m = app.session.as_ref().unwrap().model();
    assert!(m.groups.is_empty());
    assert_eq!(m.file.fusion, simsandbox_file::Fusion::Off);
    assert_eq!(
        m.params.iter().find(|r| r.key == "a.kappa").unwrap().value,
        0.8
    );
    assert!(app.error.is_none());

    app.set_fusion(true);
    assert_eq!(app.session.as_ref().unwrap().model().groups.len(), 1);
}

#[test]
fn new_maps_and_formulas_get_the_right_method() {
    let mut app = SimApp::new(None, None, None, None);
    app.editor = editor(None, "x[n+1] = r*x*(1 - x)");
    app.apply_equations();
    assert!(app.editor.is_none());
    let method = |app: &SimApp| {
        app.session.as_ref().unwrap().model().file.blocks[0]
            .method
            .name
            .clone()
    };
    assert_eq!(method(&app), "iterate");

    app.editor = editor(None, "y = a*x^2");
    app.apply_equations();
    assert_eq!(method(&app), "evaluate");

    // Editing a formula block into a continuous one switches the method.
    app.editor = editor(Some("model"), "dy/dt = -a*y");
    app.apply_equations();
    assert_eq!(method(&app), "rk4");
}

#[test]
fn mistakes_keep_the_editor_open_and_the_model_running() {
    let mut app = SimApp::new(None, None, None, None);
    // Without a file the app opens on its landing page.
    app.open_example(0);
    app.mark_saved();
    app.editor = editor(None, "dx/dt = -k*");
    app.apply_equations();
    let e = app.editor.as_ref().unwrap().error.as_deref().unwrap();
    assert!(e.contains("line 1, column 12"), "{e}");
    assert_eq!(app.session.as_ref().unwrap().model().title, "Lorenz system");
    assert_eq!(app.error, None);
}

#[test]
fn editing_a_block_keeps_parameter_values_that_still_exist() {
    let mut app = SimApp::new(None, None, None, None);
    let (name, text) = (EXAMPLES[2].name, EXAMPLES[2].text);
    app.open_text(name, text);
    let live = app.session.as_ref().unwrap();
    let id = live.call(|s, _| s.ids["ecosystem"]).unwrap();
    live.call(move |s, _| s.set_param(id, "alpha", 2.0))
        .unwrap()
        .unwrap();

    let edited = "dprey/dt = alpha*prey - beta*prey*predators\n\
                  dpredators/dt = delta*prey*predators - gamma*predators";
    app.editor = editor(Some("ecosystem"), edited);
    app.apply_equations();
    assert!(
        app.editor.is_none(),
        "{:?}",
        app.editor.as_ref().map(|e| &e.error)
    );
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    assert_eq!(m.title, name);
    assert_eq!(
        m.params.iter().find(|r| r.name == "alpha").unwrap().value,
        2.0
    );
    assert_eq!(
        live.history().names(),
        ["ecosystem.prey", "ecosystem.predators"]
    );
    // Both saved views still fit.
    assert_eq!(app.views.tiles.len(), 2);
}

#[test]
fn sliders_apply_while_dragged_only_for_real_time_models() {
    // (changed, dragged, drag stopped)
    let dragging = (true, true, false);
    let released = (false, false, true);
    let clicked = (true, false, false);
    for (real_time, case, expected) in [
        (true, dragging, true),
        (true, released, false),
        (true, clicked, true),
        (false, dragging, false),
        (false, released, true),
        (false, clicked, true),
    ] {
        let (c, d, s) = case;
        assert_eq!(
            apply_now(real_time, c, d, s),
            expected,
            "{real_time} {case:?}"
        );
    }
    // The examples: the Lorenz system is real-time.
    let mut app = SimApp::new(None, None, None, None);
    // Without a file the app opens on its landing page.
    app.open_example(0);
    app.mark_saved();
    let live = app.session.as_ref().unwrap();
    assert!(live.model().real_time);
}

#[test]
fn agent_blocks_change_their_update_order() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("grazing"))
        .unwrap();
    app.open_text(e.name, e.text);
    app.rebuild(
        |f| f.blocks[1].update = Some("synchronous".into()),
        "changed",
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    let m = app.session.as_ref().unwrap().model();
    assert_eq!(m.file.blocks[1].update.as_deref(), Some("synchronous"));
    // An unknown order is refused, naming where it is written.
    app.rebuild(|f| f.blocks[1].update = Some("bogus".into()), "changed");
    let err = app.error.clone().unwrap();
    assert!(err.contains("blocks[1].update"), "{err}");
}

#[test]
fn values_display_compactly() {
    assert_eq!(format_value(28.0), "28");
    assert_eq!(format_value(8.0 / 3.0), "2.66667");
    assert_eq!(format_value(-0.5), "-0.5");
    assert_eq!(format_value(0.0), "0");
    assert_eq!(format_value(123456.0), "123456");
    assert_eq!(format_value(1e-7), "1e-7");
    assert_eq!(format_value(2.5e9), "2.5e9");
}

/// Apply canvas edits to the open model as the Diagram tab does.
fn canvas_edits(app: &mut SimApp, edits: Vec<simsandbox_session::diagram::Edit>) {
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    let (speed, views, layout) = (app.speed, app.views.entries(), app.layout());
    let current = || {
        let (views, layout) = (views.clone(), layout.clone());
        live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
    };
    let outcomes = app.diagram.handle(edits, &app.registry, &current);
    app.diagram_outcomes(&m, outcomes);
}

/// Undo (`back`) or redo a canvas edit, as Ctrl+Z and Ctrl+Shift+Z do.
fn canvas_step(app: &mut SimApp, back: bool) {
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    let (speed, views, layout) = (app.speed, app.views.entries(), app.layout());
    let current = || {
        let (views, layout) = (views.clone(), layout.clone());
        live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
    };
    let outcomes = app.diagram.step(back, &app.registry, &current);
    app.diagram_outcomes(&m, outcomes);
}

#[test]
fn the_arrangement_is_saved_and_survives_rebuilds() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("oscillators"))
        .unwrap();
    app.open_text(e.name, e.text);
    // The standard arrangement is not written.
    assert!(!app.saved_text().unwrap().contains("\"layout\""));
    let tiles = app.tile_ids();
    assert_eq!(tiles.len(), 3);
    // The second view in a tab of its own; the diagram in a window.
    app.dock.own_tab(tiles[1]);
    app.dock.pop_out(&Pane::Diagram);
    let text = app.saved_text().unwrap();
    let (f, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
    let l = f.controls.layout.clone().expect("saved");
    let panes: Vec<String> = l.panes().iter().map(|p| format!("{p:?}")).collect();
    assert!(panes.iter().any(|p| p.contains("view:1")), "{panes:?}");
    assert!(panes.iter().any(|p| p.contains("diagram")), "{panes:?}");
    // Drawn, the popped-out diagram is a window of its own (here, with
    // no native windows, one inside the app).
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.docked(ui));
    out.textures_delta.clear();
    assert_eq!(app.dock.popped, [Pane::Diagram]);
    // A canvas edit rebuilds the model: the window and the tab stay.
    canvas_edits(
        &mut app,
        vec![Edit::Rename {
            from: "b".into(),
            to: "c".into(),
        }],
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.dock.popped, [Pane::Diagram]);
    assert!(app.dock.panes().iter().any(|p| matches!(p, Pane::View(_))));
    // Opened again, the view has its own tab and the diagram is docked.
    app.open_text("again", &text);
    assert!(app.dock.popped.is_empty());
    assert!(app.dock.panes().iter().any(|p| matches!(p, Pane::View(_))));
    assert!(app.dock.panes().contains(&Pane::Diagram));
}

/// Draw one frame of the inspector.
fn inspect(app: &mut SimApp) {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(500.0, 900.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.param_panel(ui));
    out.textures_delta.clear();
}

#[test]
fn a_studys_runs_are_counted_as_its_design_makes_them() {
    let s =
        |v: serde_json::Value| -> simsandbox_file::Sampling { serde_json::from_value(v).unwrap() };
    let one = serde_json::json!({"param": "a.x"});
    let two = serde_json::json!({"param": "a.y", "points": 3});
    assert_eq!(study_runs(&s(serde_json::json!({"params": [one]}))), 400);
    assert_eq!(
        study_runs(&s(serde_json::json!({"params": [one, two]}))),
        300
    );
    assert_eq!(
        study_runs(&s(
            serde_json::json!({"design": "sobol", "params": [one], "count": 64, "replicates": 2})
        )),
        128
    );
    assert_eq!(
        study_runs(&s(
            serde_json::json!({"design": "list", "params": [one], "values": [[1.0], [2.0]]})
        )),
        2
    );
}

#[test]
fn a_block_inside_a_study_shows_its_models_values() {
    // ADR 0070: open the tuning study, pick its car: the cruise
    // model's values, as written, in the Inspector and the tree.
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Tuning"))
        .unwrap();
    app.open_text(e.name, e.text);
    let file = app.session.as_ref().unwrap().model().file.clone();
    let st = app.studied(&file, "tuning").unwrap();
    assert_eq!(st.model, "cruise");
    let car = st.blocks.iter().find(|(b, _)| b == "car").unwrap();
    assert!(car.1.iter().any(|v| v.name == "m"), "{:?}", car.1);
    app.diagram.inside = vec!["tuning".into()];
    let o = simsandbox_session::render::RenderOptions {
        width: 900.0,
        height: 600.0,
        ..Default::default()
    };
    simsandbox_session::render::paint(&o, |ui| app.diagram_tab(ui));
    let values = app
        .diagram
        .inside_values(&["tuning".to_string()], "car")
        .unwrap();
    assert!(values.iter().any(|v| v.name == "m"), "{values:?}");
    app.diagram.canvas.selected = Some(crate::canvas::Selection::Block("car".into()));
    // SIMSANDBOX_SHOT=FILE.png writes the Inspector as drawn, to look at.
    if let Ok(path) = std::env::var("SIMSANDBOX_SHOT_INSIDE") {
        let o = simsandbox_session::render::RenderOptions {
            width: 520.0,
            height: 900.0,
            ..Default::default()
        };
        let image = simsandbox_session::render::paint(&o, |ui| app.param_panel(ui));
        std::fs::write(path, simsandbox_session::render::png(&image).unwrap()).unwrap();
    }
    inspect(&mut app);
    assert_eq!(
        app.picked,
        [(Some("tuning".to_string()), "car".to_string())]
    );
}

#[test]
fn a_study_selected_shows_its_form() {
    // ADR 0070: what it varies comes from its model's parameters.
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Tuning"))
        .unwrap();
    app.open_text(e.name, e.text);
    let file = app.session.as_ref().unwrap().model().file.clone();
    let keys = app.study_keys(&file, "tuning");
    let names: Vec<&str> = keys.iter().map(|(k, _, _)| k.as_str()).collect();
    assert!(
        names.contains(&"cruise.ki") && names.contains(&"car.m"),
        "{names:?}"
    );
    assert!(app.study_keys(&file, "best").is_empty());
    app.diagram.canvas.selected = Some(crate::canvas::Selection::Block("tuning".into()));
    // SIMSANDBOX_SHOT=FILE.png writes the Inspector as drawn, to look at.
    if let Ok(path) = std::env::var("SIMSANDBOX_SHOT") {
        let o = simsandbox_session::render::RenderOptions {
            width: 520.0,
            height: 900.0,
            ..Default::default()
        };
        let image = simsandbox_session::render::paint(&o, |ui| app.param_panel(ui));
        std::fs::write(path, simsandbox_session::render::png(&image).unwrap()).unwrap();
    }
    inspect(&mut app);
    assert_eq!(app.picked, [(None, "tuning".to_string())]);
}

#[test]
fn a_study_of_points_from_inputs_shows_no_ranges() {
    // ADR 0078: its values arrive through its inputs; drawing its form
    // writes no range into it.
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Searching"))
        .unwrap();
    app.open_text(e.name, e.text);
    app.diagram.canvas.selected = Some(crate::canvas::Selection::Block("trials".into()));
    let o = simsandbox_session::render::RenderOptions {
        width: 520.0,
        height: 900.0,
        ..Default::default()
    };
    let image = simsandbox_session::render::paint(&o, |ui| app.param_panel(ui));
    if let Ok(path) = std::env::var("SIMSANDBOX_SHOT_INPUTS") {
        std::fs::write(path, simsandbox_session::render::png(&image).unwrap()).unwrap();
    }
    let file = app.session.as_ref().unwrap().model().file.clone();
    let trials = file.blocks.iter().find(|b| b.name == "trials").unwrap();
    let sample = trials.sample.as_ref().unwrap();
    assert!(sample
        .params
        .iter()
        .all(|p| p.range.is_none() && p.points.is_none()));
}

#[test]
fn a_value_typed_on_a_node_applies_to_the_running_model() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.run_until(2.0)).unwrap().unwrap();
    let m = live.model();
    app.diagram_outcomes(
        &m,
        vec![Outcome::SetValue {
            key: "car.m".into(),
            value: 1500.0,
        }],
    );
    let live = app.session.as_ref().unwrap();
    live.sync();
    let m = live.model();
    let row = m.params.iter().find(|r| r.key == "car.m").unwrap();
    assert_eq!(row.value, 1500.0);
    // No restart.
    assert!(live.status().time > 1.9);
}

#[test]
fn blocks_added_on_the_diagram_are_set_in_the_inspector_before_they_run() {
    use crate::canvas::Selection;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    // A conversion with nothing wired in does not load: a draft.
    for type_name in ["units.convert", "eq.pendulum"] {
        let add = Edit::Add {
            type_name: type_name.into(),
            at: [0.0, 0.0],
        };
        app.edit_at(Scope::Model, add);
    }
    assert!(app.diagram.draft_problem().is_some());
    // Its units and the new pendulum's values go into the draft.
    app.edit_at(
        Scope::Model,
        Edit::SetSetting {
            block: "convert1".into(),
            key: "from".into(),
            text: "km/h".into(),
        },
    );
    app.edit_at(
        Scope::Model,
        Edit::SetParam {
            block: "pendulum1".into(),
            param: "L".into(),
            value: 2.0,
        },
    );
    let draft = app.diagram.draft_file().unwrap();
    let block = |name: &str| draft.blocks.iter().find(|b| b.name == name).unwrap();
    assert_eq!(block("convert1").settings["from"], "km/h");
    assert_eq!(
        block("pendulum1").params["L"],
        simsandbox_file::ParamValue::Number(2.0)
    );
    // Both selected: the inspector shows each, from the draft.
    app.diagram.canvas.selected = Some(Selection::Many(vec![
        "convert1".into(),
        "pendulum1".into(),
        "leader".into(),
    ]));
    inspect(&mut app);
    assert_eq!(app.picked.len(), 3);
}

#[test]
fn open_nodes_are_saved_and_inner_values_are_every_uses_default() {
    use simsandbox_session::edit_use::How;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    let m = app.session.as_ref().unwrap().model();
    let open = vec!["car1".to_string(), "leader".to_string()];
    app.diagram_outcomes(&m, vec![Outcome::Opened(open.clone())]);
    let text = app.saved_text().unwrap();
    let (file, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
    assert_eq!(file.controls.open, open);
    // Opened again, the diagram opens the same nodes.
    let mut again = SimApp::new(None, None, None, None);
    again.open_text("again", &text);
    assert_eq!(again.diagram.canvas.open, open.into_iter().collect());
    // Inside the model the cars use, a value changes every use.
    let e = Edit::SetParam {
        block: "car".into(),
        param: "c".into(),
        value: 80.0,
    };
    app.diagram.inside = vec!["car1".into()];
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    let text = app.saved_text().unwrap();
    let current = move || Ok(text.clone());
    let outcomes = app
        .diagram
        .answer(vec![e], How::Original, &app.registry, &current);
    app.diagram_outcomes(&m, outcomes);
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    live.sync();
    let m = live.model();
    for car in ["car1", "car2", "car3"] {
        let key = format!("{car}/car.c");
        let row = m.params.iter().find(|r| r.key == key).unwrap();
        assert_eq!(row.value, 80.0, "{key}");
    }
}

#[test]
fn values_take_formulas_and_the_model_declares_parameters() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    let row = |app: &SimApp, key: &str| {
        let m = app.session.as_ref().unwrap().model();
        m.params.iter().find(|r| r.key == key).unwrap().clone()
    };
    // Type into a row as the inspector does, then carry out file edits.
    let type_in = |app: &mut SimApp, key: &str, typed: Typed| {
        let r = row(app, key);
        let (mut changes, mut file) = (Vec::new(), Vec::new());
        app.error = None;
        apply_typed(&r, typed, &mut changes, &mut file, &mut app.error);
        app.apply_changes(changes);
        for (scope, e) in file {
            app.edit_at(scope, e);
        }
        app.session.as_ref().unwrap().sync();
    };
    let declare = Edit::DeclareParam {
        name: "tau".into(),
        value: 8.0,
        unit: "s".into(),
    };
    app.edit_at(Scope::Model, declare);
    assert_eq!(row(&app, "tau").value, 8.0);
    // A block's value reads it.
    type_in(&mut app, "leader.T", Typed::Formula("tau / 2".into()));
    let t = row(&app, "leader.T");
    assert_eq!((t.value, t.formula.as_deref()), (4.0, Some("tau / 2")));
    // A formula that does not work is refused; the model stays.
    type_in(&mut app, "leader.T", Typed::Formula("kp".into()));
    assert!(app.error.as_deref().unwrap().starts_with("Not applied"));
    type_in(&mut app, "leader.T", Typed::Formula("nope".into()));
    assert!(app.error.as_deref().unwrap().contains("nope"));
    assert!(
        app.diagram.draft_problem().is_none(),
        "{:?}",
        app.diagram.draft_problem()
    );
    assert_eq!(row(&app, "leader.T").formula.as_deref(), Some("tau / 2"));
    // A use's own value takes a formula too; the model's parameters and
    // values inside uses stay numbers.
    type_in(&mut app, "car3.mass", Typed::Formula("2*1000".into()));
    assert_eq!(row(&app, "car3.mass").value, 2000.0);
    type_in(&mut app, "kp", Typed::Formula("2".into()));
    assert!(app.error.as_deref().unwrap().contains("is a number"));
    type_in(&mut app, "car1/car.c", Typed::Formula("kp".into()));
    assert!(app.error.as_deref().unwrap().contains("open the subsystem"));
    // A parameter a formula reads cannot go.
    app.edit_at(Scope::Model, Edit::RemoveParam { name: "tau".into() });
    assert!(app.error.take().is_some());
    // A number replaces the formula; then the parameter can go.
    type_in(&mut app, "leader.T", Typed::Number(6.0));
    let t = row(&app, "leader.T");
    assert_eq!((t.value, t.formula), (6.0, None));
    app.edit_at(Scope::Model, Edit::RemoveParam { name: "tau".into() });
    let m = app.session.as_ref().unwrap().model();
    assert!(m.params.iter().all(|r| r.key != "tau"));
}

#[test]
fn a_clicked_curve_selects_its_block_until_the_diagram_picks_another() {
    use crate::canvas::Selection;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    inspect(&mut app);
    // A curve inside a subsystem use selects the use.
    app.inspect_signal("car2/car.v");
    assert_eq!(app.picked, [(None, "car2".to_string())]);
    assert_eq!(
        app.diagram.canvas.selected,
        Some(Selection::Block("car2".into()))
    );
    inspect(&mut app);
    assert_eq!(app.picked, [(None, "car2".to_string())]);
    // Selecting on the diagram takes over.
    app.diagram.canvas.selected = Some(Selection::Block("leader".into()));
    inspect(&mut app);
    assert_eq!(app.picked, [(None, "leader".to_string())]);
    // A filter's signal has none.
    app.inspect_signal("energy");
    assert_eq!(app.picked, [(None, "leader".to_string())]);
}

#[test]
fn a_block_picked_in_a_model_expanded_in_place_shows_its_values() {
    use crate::canvas::Selection;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    inspect(&mut app);
    // car1 expanded: its car is named by path on the canvas.
    app.diagram.canvas.selected = Some(Selection::Block("car1/car".into()));
    inspect(&mut app);
    assert_eq!(app.picked, [(Some("car1".to_string()), "car".to_string())]);
    let m = app.session.as_ref().unwrap().model();
    let t = crate::inspector::selected(&m.params, &m.file, Some("car1"), "car")
        .expect("the car's values");
    assert!(t.rows.iter().any(|(k, _)| k == "c"), "{:?}", t.rows);
    assert!(app.error.is_none(), "{:?}", app.error);
}

#[test]
fn the_inspector_draws_selection_search_and_computed_values() {
    use crate::canvas::Selection;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    inspect(&mut app);
    app.diagram.canvas.selected = Some(Selection::Block("car2".into()));
    inspect(&mut app);
    app.show_computed = true;
    app.inspect_query = "mass".into();
    inspect(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
}

#[test]
fn data_edits_apply_live_and_canvas_edits_undo() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.run_until(5.0)).unwrap().unwrap();
    let shown = |app: &SimApp| app.views.entries()[1].bind["y"].signals();

    // A data edit: the second view also shows the speed; the run goes
    // on.
    canvas_edits(
        &mut app,
        vec![Edit::Bind {
            from: "car.v".into(),
            to: "view:1.y".into(),
            many: true,
        }],
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(shown(&app), ["cruise.F", "car.v"]);
    assert!(app.session.as_ref().unwrap().status().time > 4.9);
    // A filter added and moved on the canvas, kept with the model.
    canvas_edits(&mut app, vec![Edit::AddFilter { at: [500.0, 300.0] }]);
    canvas_edits(
        &mut app,
        vec![Edit::Move {
            block: "filter:signal1".into(),
            at: [520.0, 310.0],
        }],
    );
    let live = app.session.as_ref().unwrap();
    live.sync();
    let filters = live.model().filters.clone();
    assert_eq!(filters[0].name, "signal1");
    let saved = app.saved_text().unwrap();
    assert!(
        saved.contains("\"canvas\": [\n            520.0") || saved.contains("520.0"),
        "{saved}"
    );
    assert!(app.session.as_ref().unwrap().status().time > 4.9);

    // Undo steps back through them without a restart.
    canvas_step(&mut app, true); // the move
    canvas_step(&mut app, true); // the filter
    let live = app.session.as_ref().unwrap();
    live.sync();
    assert!(live.model().filters.is_empty());
    canvas_step(&mut app, true); // the binding
    assert_eq!(shown(&app), ["cruise.F"]);
    assert!(app.session.as_ref().unwrap().status().time > 4.9);
    canvas_step(&mut app, false); // redo the binding
    assert_eq!(shown(&app), ["cruise.F", "car.v"]);

    // A model edit that does not load waits as a draft; undoing it
    // goes back to the running model.
    canvas_edits(
        &mut app,
        vec![Edit::Delete {
            block: "cruise".into(),
        }],
    );
    // Removing cruise leaves car.F open: a draft, the run goes on.
    assert!(app.diagram.draft_problem().is_some());
    canvas_step(&mut app, true);
    assert!(
        app.diagram.draft_problem().is_none(),
        "{:?}",
        app.diagram.draft_problem()
    );
    let live = app.session.as_ref().unwrap();
    assert!(live.model().file.blocks.iter().any(|b| b.name == "cruise"));
    assert!(app.error.is_none(), "{:?}", app.error);
}

#[test]
fn a_promoted_filter_becomes_a_block_feeding_the_model() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    canvas_edits(&mut app, vec![Edit::AddFilter { at: [500.0, 300.0] }]);
    let live = app.session.as_ref().unwrap();
    let entries = vec![simsandbox_file::FilterEntry::expression(
        "push",
        "cruise.F",
        Vec::new(),
    )];
    live.call(move |s, _| s.set_filters(entries))
        .unwrap()
        .unwrap();
    canvas_edits(
        &mut app,
        vec![
            Edit::Disconnect { to: "car.F".into() },
            Edit::PromoteConnect {
                filter: "push".into(),
                to: "car.F".into(),
            },
        ],
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(
        app.diagram.draft_problem().is_none(),
        "{:?}",
        app.diagram.draft_problem()
    );
    let m = app.session.as_ref().unwrap().model();
    assert!(m.file.blocks.iter().any(|b| b.name == "push"));
    assert!(m
        .file
        .connections
        .iter()
        .any(|c| c.from == "push.push" && c.to == "car.F"));
    assert!(m.filters.is_empty());
}

#[test]
fn a_dragged_run_is_saved_beside_the_model_and_replayed() {
    let dir = std::env::temp_dir().join(format!("simsandbox-runs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Drag the bob"))
        .unwrap();
    app.open_text(e.name, e.text);
    let model = dir.join("drag.ssbx.json");
    app.save_to(&model);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    live.call(|s, _| s.restart()).unwrap().unwrap();
    live.call(|s, _| s.run_until(0.5)).unwrap().unwrap();
    // The view's pointer, as the dock sends it; a repeat is not recorded.
    for [x, y, down] in [[1.0, 0.0, 1.0], [1.0, 0.0, 1.0]] {
        for (k, v) in [("x", x), ("y", y), ("down", down)] {
            app.send_input(format!("hand.{k}"), v);
        }
        app.session.as_ref().unwrap().sync();
    }
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.run_until(1.0)).unwrap().unwrap();
    app.send_input("hand.down".into(), 0.0);
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.run_until(1.5)).unwrap().unwrap();
    let theta = |app: &SimApp| {
        app.session
            .as_ref()
            .unwrap()
            .call(|s, _| {
                let h = s.signals();
                let k = h.names().iter().position(|n| n == "bob.theta").unwrap();
                h.points(k)
                    .iter()
                    .map(|p| p.y.to_bits())
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    let reference = theta(&app);
    assert_eq!(live.call(|s, _| s.recorded().len()).unwrap(), 3);

    app.save_run("pull", "dragged right");
    let runs = runs::runs_beside(&model);
    assert_eq!(runs.len(), 1);
    let (path, run) = &runs[0];
    assert!(path.ends_with("drag.pull.run.json"));
    assert_eq!((run.name.as_str(), run.track.len()), ("pull", 3));
    assert!(runs::fits_disk(run, &model, &app.registry));
    // Opening it replays the drag bit for bit.
    app.open_run(path);
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    assert!(live.call(|s, _| s.replaying()).unwrap());
    live.call(|s, _| s.run_until(1.5)).unwrap().unwrap();
    assert_eq!(theta(&app), reference);
    // As the model's schedule, every run applies it.
    let run = run.clone();
    app.use_as_schedule(&run, "pull");
    let m = app.session.as_ref().unwrap().model();
    assert_eq!(m.file.run.inputs.len(), 3);
    assert!(runs::utc_now().ends_with('Z') && runs::utc_now().len() == 20);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probes_added_on_the_canvas_record_and_save_their_files() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    let dir = std::env::temp_dir().join(format!("simsandbox-app-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    app.path = Some(dir.join("cruise.ssbx.json"));
    let live = app.session.as_ref().unwrap();
    // Paused: running live, a slow machine passes t = 2 before the
    // probe is wired.
    live.set_running(false);
    live.call(|s, _| s.run_until(1.0)).unwrap().unwrap();
    canvas_edits(&mut app, vec![Edit::AddProbe { at: [600.0, 0.0] }]);
    canvas_edits(
        &mut app,
        vec![Edit::Bind {
            from: "car.v".into(),
            to: "probe:probe1.record".into(),
            many: true,
        }],
    );
    let mut probe = app.session.as_ref().unwrap().model().probes[0].clone();
    assert_eq!(probe.record, ["car.v"]);
    probe.to = Some("speed.csv".into());
    canvas_edits(&mut app, vec![Edit::SetProbe { probe }]);
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    // Not restarted: the probe records from t = 1 on.
    assert!(live.status().time > 0.9);
    live.call(|s, _| s.run_until(2.0)).unwrap().unwrap();
    app.save_probe_data();
    assert!(app.error.is_none(), "{:?}", app.error);
    let csv = std::fs::read_to_string(dir.join("speed.csv")).unwrap();
    // From when it was wired, at every step since.
    let t: Vec<f64> = csv
        .lines()
        .skip(1)
        .map(|l| l.split(',').next().unwrap().parse().unwrap())
        .collect();
    assert!(t.len() > 2 && t[0] > 0.9, "{csv}");
    assert!(
        t.windows(2).all(|w| (w[1] - w[0] - 0.05).abs() < 1e-9),
        "{csv}"
    );
    // Saved with the model, and undone without a restart.
    assert!(app.saved_text().unwrap().contains("\"to\": \"speed.csv\""));
    canvas_step(&mut app, true);
    canvas_step(&mut app, true);
    canvas_step(&mut app, true);
    let live = app.session.as_ref().unwrap();
    live.sync();
    assert!(live.model().probes.is_empty());
    assert!(live.status().time > 1.9);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn canvas_edits_apply_when_the_model_loads_and_wait_as_a_draft_otherwise() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.run_until(5.0)).unwrap().unwrap();
    let id = live.call(|s, _| s.ids["cruise"]).unwrap();
    live.call(move |s, _| s.set_param(id, "kp", 900.0))
        .unwrap()
        .unwrap();

    // Moving only saves the layout: the run goes on.
    canvas_edits(
        &mut app,
        vec![Edit::Move {
            block: "car".into(),
            at: [10.0, 20.0],
        }],
    );
    let live = app.session.as_ref().unwrap();
    live.sync();
    assert!(live.status().time > 4.9);
    assert_eq!(live.model().file.blocks[0].canvas, Some([10.0, 20.0]));

    // Removing a wire leaves an input open: kept as a draft, the run
    // goes on.
    canvas_edits(&mut app, vec![Edit::Disconnect { to: "car.F".into() }]);
    let problem = app.diagram.draft_problem().unwrap().to_string();
    assert!(problem.contains("car.F"), "{problem}");
    assert!(app.session.as_ref().unwrap().status().time > 4.9);
    // Wiring it again applies, restarting at t = 0
    // with the parameter values and layout kept.
    canvas_edits(
        &mut app,
        vec![Edit::Connect {
            from: "cruise.F".into(),
            to: "car.F".into(),
        }],
    );
    assert!(
        app.diagram.draft_problem().is_none(),
        "{:?}",
        app.diagram.draft_problem()
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    assert_eq!(live.status().time, 0.0);
    assert!(m
        .file
        .connections
        .iter()
        .any(|c| c.from == "cruise.F" && c.to == "car.F"));
    assert_eq!(
        m.params
            .iter()
            .find(|r| r.key == "cruise.kp")
            .unwrap()
            .value,
        900.0
    );
    assert_eq!(m.file.blocks[0].canvas, Some([10.0, 20.0]));
    assert!(
        m.file.blocks.iter().all(|b| b.canvas.is_some()),
        "the layout is fixed"
    );
}

#[test]
fn start_from_here_restarts_at_the_current_state() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("grazing"))
        .unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    live.call(|s, _| s.run_until(3.0)).unwrap().unwrap();
    let notes = live.call(|s, _| s.start_from_now()).unwrap().unwrap();
    assert!(notes.len() == 3, "{notes:?}");
    let m = live.model();
    assert!(m.file.blocks.iter().all(|b| b.start.is_some()));
    assert_eq!(live.status().time, 0.0);
}

#[test]
fn models_are_made_opened_and_edited_from_the_canvas() {
    use simsandbox_session::diagram::Edit;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
    app.open_text(e.name, e.text);
    // Make a model of both blocks: the file now defines and uses it.
    canvas_edits(
        &mut app,
        vec![Edit::GroupModel {
            blocks: vec!["car".into(), "cruise".into()],
        }],
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    let m = app.session.as_ref().unwrap().model();
    assert!(m.file.models.contains_key("model1"));
    assert!(m.params.iter().any(|r| r.key == "group1/cruise.kp"));
    // Inside it (used once: no question), an edit changes the model.
    app.diagram.inside = vec!["group1".into()];
    let live = app.session.as_ref().unwrap();
    let text = app.saved_text().unwrap();
    let current = move || Ok(text.clone());
    let rename = Edit::Rename {
        from: "car".into(),
        to: "body".into(),
    };
    let outcomes = app
        .diagram
        .begin_edit(vec![rename], &app.registry, &current);
    app.diagram_outcomes(&live.model(), outcomes);
    assert!(app.error.is_none(), "{:?}", app.error);
    let m = app.session.as_ref().unwrap().model();
    assert!(m.params.iter().any(|r| r.key == "group1/body.m"));
    // And the equation editor works on a block inside it.
    app.open_block(&m, "group1::body");
    let ed = app.editor.as_mut().unwrap();
    ed.text = ed.text.replace("(F - c*v)/m", "(F - drag2*c*v)/m");
    app.apply_equations();
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.editor.is_none(), "applied");
    let m = app.session.as_ref().unwrap().model();
    assert!(m.params.iter().any(|r| r.key == "group1/body.drag2"));
}

#[test]
fn conversions_change_their_units_from_the_panel() {
    let mut app = SimApp::new(None, None, None, None);
    let k = EXAMPLES
        .iter()
        .position(|e| e.name.starts_with("Units"))
        .unwrap();
    app.open_example(k);
    assert!(app.error.is_none(), "{:?}", app.error);
    let unit = |app: &SimApp, port: &str| {
        let m = app.session.as_ref().unwrap().model();
        m.file
            .blocks
            .iter()
            .find(|b| b.name == "b_cm")
            .unwrap()
            .settings[port]
            .clone()
    };
    let set = |app: &mut SimApp, text: &str| {
        let e = Edit::SetSetting {
            block: "b_cm".into(),
            key: "to".into(),
            text: text.into(),
        };
        app.edit_at(Scope::Model, e);
    };
    set(&mut app, "mm");
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(unit(&app, "to"), "mm");
    // A different kind of quantity does not load: the edit waits as a
    // draft, and the running model stays.
    set(&mut app, "kg");
    let problem = app.diagram.draft_problem().unwrap();
    assert!(problem.contains("measure different things"), "{problem}");
    assert_eq!(unit(&app, "to"), "mm");
}

#[test]
fn a_file_reader_added_on_the_canvas_reads_from_the_models_folder() {
    let dir = std::env::temp_dir().join(format!("simsandbox-app-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("push.csv"), "t,F [N]\n0,1\n1,3\n").unwrap();
    let model = dir.join("m.ssbx.json");
    std::fs::write(
        &model,
        r#"{"format": 1, "blocks": [{"name": "one", "type": "eq.equations",
                "equations": ["y = 1"]}]}"#,
    )
    .unwrap();
    let mut app = SimApp::new(None, None, None, None);
    app.open_path(&model.to_string_lossy());
    assert!(app.error.is_none(), "{:?}", app.error);
    // Added without a file it waits as a draft; naming one loads it.
    app.edit_at(
        Scope::Model,
        Edit::Add {
            type_name: simsandbox_eq::file_input::FILE.into(),
            at: [0.0, 0.0],
        },
    );
    let problem = app.diagram.draft_problem().unwrap();
    assert!(problem.contains("settings.file"), "{problem}");
    let name = app.diagram.draft_file().unwrap().blocks[1].name.clone();
    app.edit_at(
        Scope::Model,
        Edit::SetSetting {
            block: name.clone(),
            key: "file".into(),
            text: "push.csv".into(),
        },
    );
    assert!(
        app.diagram.draft_problem().is_none(),
        "{:?}",
        app.diagram.draft_problem()
    );
    let live = app.session.as_ref().unwrap();
    let key = format!("{name}.F");
    let f = live
        .call(move |s, _| {
            s.set_view_reads(vec![key.clone()]);
            s.restart().unwrap();
            s.run_until(1.0).unwrap();
            s.flush(true);
            let h = s.signals();
            h.values(h.resolve(&key).unwrap())
        })
        .unwrap();
    assert_eq!((f.first(), f.last()), (Some(&1.0), Some(&3.0)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_model_using_model_files_opens_and_runs() {
    let dir = std::env::temp_dir().join(format!("simsandbox-app-uses-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write =
        |name: &str, v: serde_json::Value| std::fs::write(dir.join(name), v.to_string()).unwrap();
    write(
        "count.ssbx.json",
        serde_json::json!({"format": 1, "run": {"dt": 0.1},
        "blocks": [{"name": "count", "type": "eq.equations", "method": {"name": "euler"},
                    "equations": ["state x", "dx/dt = 1"], "params": {"x0": 0}}],
        "outputs": [{"name": "x", "from": "count.x"}]}),
    );
    write(
        "hold.ssbx.json",
        serde_json::json!({"format": 1, "run": {"dt": 0.3},
        "blocks": [{"name": "u", "type": "input.value"},
                   {"name": "sum", "type": "eq.equations", "method": {"name": "euler"},
                    "equations": ["input u", "state z", "dz/dt = u"], "params": {"z0": 0}}],
        "connections": [{"from": "u.y", "to": "sum.u"}]}),
    );
    let model = dir.join("both.ssbx.json");
    write(
        "both.ssbx.json",
        serde_json::json!({"format": 1,
        "blocks": [{"name": "fast", "file": "count.ssbx.json"}, {"name": "slow", "file": "hold.ssbx.json"}],
        "connections": [{"from": "probe:p.fast/x", "to": "slow/u"}],
        "controls": {"probes": [{"name": "p", "record": ["fast/x"]}]}}),
    );
    let mut app = SimApp::new(None, None, None, None);
    app.open_path(&model.to_string_lossy());
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    let (t, u) = live
        .call(|s, _| {
            s.set_view_reads(vec!["slow/u.y".into(), "fast/x".into()]);
            s.restart().unwrap();
            s.run_until(0.6).unwrap();
            s.flush(true);
            let h = s.signals();
            // A declared output is recorded under its name.
            let x = h.values(h.resolve("fast/x").unwrap());
            assert!((x.last().unwrap() - 0.6).abs() < 1e-9, "{x:?}");
            (s.time(), h.values(h.resolve("slow/u.y").unwrap()))
        })
        .unwrap();
    assert!((t - 0.6).abs() < 1e-9, "{t}");
    // slow read fast's count at its own instants: 0 at the start,
    // about 0.3 from 0.3 s.
    assert_eq!(u[0], 0.0);
    assert!((u.last().unwrap() - 0.6).abs() < 1e-9, "{u:?}");
    // Opening a use reads its file, from the folder of the file using it.
    let (top, _) =
        simsandbox_file::ModelFile::from_json(&std::fs::read_to_string(&model).unwrap()).unwrap();
    let inside = crate::diagram::used_file(&top, Some(&dir), &["slow".into()]).unwrap();
    assert_eq!(inside.blocks[1].name, "sum");
    assert!(crate::diagram::used_file(&top, Some(&dir), &["nowhere".into()]).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_used_model_is_edited_from_the_model_using_it() {
    use simsandbox_session::edit_use::How;
    let dir = std::env::temp_dir().join(format!("simsandbox-app-edit-use-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write =
        |name: &str, v: serde_json::Value| std::fs::write(dir.join(name), v.to_string()).unwrap();
    write(
        "hold.ssbx.json",
        serde_json::json!({"format": 1, "run": {"dt": 0.1},
        "blocks": [{"name": "sum", "type": "eq.equations", "method": {"name": "euler"},
                    "equations": ["state z", "dz/dt = 1"], "params": {"z0": 0}}]}),
    );
    let model = dir.join("top.ssbx.json");
    write(
        "top.ssbx.json",
        serde_json::json!({"format": 1, "blocks": [{"name": "slow", "file": "hold.ssbx.json"}]}),
    );
    let hold = std::fs::read_to_string(dir.join("hold.ssbx.json")).unwrap();
    let start = |app: &SimApp| {
        app.session
            .as_ref()
            .unwrap()
            .call(|s, _| {
                s.set_view_reads(vec!["slow/sum.z".into()]);
                s.restart().unwrap();
                s.run_until(0.2).unwrap();
                s.flush(true);
                let h = s.signals();
                h.values(h.resolve("slow/sum.z").unwrap())[0]
            })
            .unwrap()
    };
    let z0 = |v: f64| {
        vec![simsandbox_session::diagram::Edit::SetParam {
            block: "sum".into(),
            param: "z0".into(),
            value: v,
        }]
    };
    let answer = |app: &mut SimApp, how: Option<How>, edits| {
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        let text = app.saved_text().unwrap();
        let current = move || Ok(text.clone());
        let outcomes = match how {
            Some(how) => app.diagram.answer(edits, how, &app.registry, &current),
            None => {
                let t = app.diagram.choices[&app.diagram.inside].clone();
                app.diagram
                    .edit_inside(None, &t, edits, &app.registry, &current)
            }
        };
        app.diagram_outcomes(&m, outcomes);
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    for how in [
        How::NewCopy(dir.join("hold-2.ssbx.json")),
        How::Original,
        How::LocalCopy,
    ] {
        let mut app = SimApp::new(None, None, None, None);
        app.open_path(&model.to_string_lossy());
        assert_eq!(start(&app), 0.0);
        app.diagram.inside = vec!["slow".into()];
        answer(&mut app, Some(how.clone()), z0(5.0));
        assert_eq!(start(&app), 5.0, "{how:?}");
        // The next edit goes to the same place without asking.
        answer(&mut app, None, z0(6.0));
        assert_eq!(start(&app), 6.0, "{how:?}");
        let saved = app.saved_text().unwrap();
        match &how {
            How::NewCopy(_) => assert!(saved.contains("hold-2.ssbx.json"), "{saved}"),
            How::Original => assert!(saved.contains("\"hold.ssbx.json\"")),
            How::LocalCopy => {
                assert!(saved.contains("\"model\"") && !saved.contains("hold.ssbx.json"))
            }
        }
        assert_eq!(app.diagram.edited.is_empty(), how == How::LocalCopy);
        assert!(app.has_unsaved());
        // Undo takes the edits back, used files included.
        canvas_step(&mut app, true);
        canvas_step(&mut app, true);
        assert_eq!(start(&app), 0.0, "{how:?} undone");
        assert!(app.diagram.edited.is_empty());
        canvas_step(&mut app, false);
        assert_eq!(start(&app), 5.0, "{how:?} redone");
        // Nothing on disk changes until saved; then every file is.
        assert_eq!(
            std::fs::read_to_string(dir.join("hold.ssbx.json")).unwrap(),
            hold
        );
        app.save_to(&model);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.diagram.edited.is_empty() && !app.has_unsaved());
        let reopened = {
            let mut again = SimApp::new(None, None, None, None);
            again.open_path(&model.to_string_lossy());
            start(&again)
        };
        assert_eq!(reopened, 5.0, "{how:?} reopened");
        // Back to the start for the next answer.
        write("hold.ssbx.json", serde_json::from_str(&hold).unwrap());
        write(
            "top.ssbx.json",
            serde_json::json!({"format": 1, "blocks": [{"name": "slow", "file": "hold.ssbx.json"}]}),
        );
        let _ = std::fs::remove_file(dir.join("hold-2.ssbx.json"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_model_defined_in_the_file_is_edited_for_every_use_or_one() {
    use simsandbox_session::edit_use::How;
    let counter = serde_json::json!({"blocks": [{"name": "c", "type": "eq.equations",
        "method": {"name": "euler"}, "equations": ["state x", "dx/dt = 1"], "params": {"x0": 0}}]});
    let text = serde_json::json!({"format": 1, "run": {"dt": 0.1}, "models": {"count": counter},
        "blocks": [{"name": "a", "model": "count"}, {"name": "b", "model": "count"}]})
    .to_string();
    let starts = |app: &SimApp| {
        app.session
            .as_ref()
            .unwrap()
            .call(|s, _| {
                s.set_view_reads(vec!["a/c.x".into(), "b/c.x".into()]);
                s.restart().unwrap();
                s.run_until(0.1).unwrap();
                s.flush(true);
                let h = s.signals();
                let first = |n: &str| h.values(h.resolve(n).unwrap())[0];
                (first("a/c.x"), first("b/c.x"))
            })
            .unwrap()
    };
    let x0 = |v: f64| {
        vec![simsandbox_session::diagram::Edit::SetParam {
            block: "c".into(),
            param: "x0".into(),
            value: v,
        }]
    };
    let edit = |app: &mut SimApp, how: Option<How>, edits| {
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        let text = app.saved_text().unwrap();
        let current = move || Ok(text.clone());
        let outcomes = match how {
            Some(how) => app.diagram.answer(edits, how, &app.registry, &current),
            None => app.diagram.begin_edit(edits, &app.registry, &current),
        };
        app.diagram_outcomes(&m, outcomes);
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    // Used twice: the question is asked; the definition changes both.
    let mut app = SimApp::new(None, None, None, None);
    app.open_text("defs", &text);
    assert_eq!(starts(&app), (0.0, 0.0));
    app.diagram.inside = vec!["a".into()];
    edit(&mut app, None, x0(2.0));
    assert!(app.diagram.asking.is_some(), "asked");
    assert_eq!(starts(&app), (0.0, 0.0));
    app.diagram.asking = None;
    edit(&mut app, Some(How::Original), x0(2.0));
    assert_eq!(starts(&app), (2.0, 2.0));
    assert!(app.diagram.edited.is_empty());
    // A local copy for one use: a new definition, the other unchanged.
    let mut app = SimApp::new(None, None, None, None);
    app.open_text("defs", &text);
    app.diagram.inside = vec!["b".into()];
    edit(&mut app, Some(How::LocalCopy), x0(3.0));
    assert_eq!(starts(&app), (0.0, 3.0));
    let saved = app.saved_text().unwrap();
    assert!(
        saved.contains("\"model\": \"b\"") && saved.contains("\"count\""),
        "{saved}"
    );
    // That copy is used once: the next edit goes to it without asking.
    app.diagram.choices.clear();
    edit(&mut app, None, x0(4.0));
    assert!(app.diagram.asking.is_none());
    assert_eq!(starts(&app), (0.0, 4.0));
}

#[test]
fn a_used_model_moves_into_the_file_and_back_out() {
    let dir = std::env::temp_dir().join(format!("simsandbox-app-move-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("count.ssbx.json"),
        serde_json::json!({"format": 1, "run": {"dt": 0.1}, "blocks": [{"name": "c",
            "type": "eq.equations", "method": {"name": "euler"}, "equations": ["state x", "dx/dt = 1"],
            "params": {"x0": 2}}]})
        .to_string(),
    )
    .unwrap();
    let model = dir.join("top.ssbx.json");
    std::fs::write(
        &model,
        serde_json::json!({"format": 1, "blocks": [{"name": "a", "file": "count.ssbx.json"}]})
            .to_string(),
    )
    .unwrap();
    let mut app = SimApp::new(None, None, None, None);
    app.open_path(&model.to_string_lossy());
    let mv = |app: &mut SimApp, to: Option<std::path::PathBuf>| {
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        let text = app.saved_text().unwrap();
        let current = move || Ok(text.clone());
        let outcomes = app.diagram.move_use("a", to, &app.registry, &current);
        app.diagram_outcomes(&m, outcomes);
        assert!(app.error.is_none(), "{:?}", app.error);
    };
    let x0 = |app: &SimApp| {
        app.session
            .as_ref()
            .unwrap()
            .call(|s, _| {
                s.set_view_reads(vec!["a/c.x".into()]);
                s.restart().unwrap();
                s.run_until(0.1).unwrap();
                s.flush(true);
                let h = s.signals();
                h.values(h.resolve("a/c.x").unwrap())[0]
            })
            .unwrap()
    };
    mv(&mut app, None);
    let saved = app.saved_text().unwrap();
    assert!(
        saved.contains("\"model\": \"count\"") && saved.contains("\"models\""),
        "{saved}"
    );
    assert_eq!(x0(&app), 2.0);
    mv(&mut app, Some(dir.join("count-2.ssbx.json")));
    assert!(app.saved_text().unwrap().contains("count-2.ssbx.json"));
    assert!(app
        .diagram
        .edited
        .contains_key(&dir.join("count-2.ssbx.json")));
    assert_eq!(x0(&app), 2.0);
    // Undo brings the definition back and forgets the new file.
    canvas_step(&mut app, true);
    assert!(app.diagram.edited.is_empty());
    assert!(app.saved_text().unwrap().contains("\"model\": \"count\""));
    canvas_step(&mut app, false);
    app.save_to(&model);
    assert!(dir.join("count-2.ssbx.json").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn edits_inside_an_expanded_use_change_the_model_it_uses() {
    use simsandbox_session::diagram::{Diagram, Edit};
    use simsandbox_session::edit_use::How;
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    // Expanded, and saved so.
    let m = app.session.as_ref().unwrap().model();
    app.diagram.expanded = vec!["car1".into()];
    app.diagram_outcomes(&m, vec![Outcome::Expanded(vec!["car1".into()])]);
    let saved = app.saved_text().unwrap();
    assert!(saved.contains("\"expand\": [\n      \"car1\""), "{saved}");
    let draw = |app: &mut SimApp| {
        let m = app.session.as_ref().unwrap().model();
        let mut d = Diagram::new(&m.file, &app.registry);
        d.expand(&m.file, &app.registry, &app.diagram.expanded.clone());
        app.diagram.set_drawn(d.clone());
        d
    };
    let d = draw(&mut app);
    // A value set on the car inside the outline: the follower is used
    // three times, so the question is asked for car1.
    canvas_edits(
        &mut app,
        vec![Edit::SetParam {
            block: "car1/car".into(),
            param: "c".into(),
            value: 80.0,
        }],
    );
    let asked = app.diagram.asking.as_ref().expect("asked");
    assert_eq!(asked.path, ["car1"]);
    // Answered: every use changes.
    let live = app.session.as_ref().unwrap();
    let m = live.model();
    let text = app.saved_text().unwrap();
    let current = move || Ok(text.clone());
    app.diagram.asking = None;
    app.diagram.inside = vec!["car1".into()];
    let edit = Edit::SetParam {
        block: "car".into(),
        param: "c".into(),
        value: 80.0,
    };
    let outcomes = app
        .diagram
        .answer(vec![edit], How::Original, &app.registry, &current);
    app.diagram.inside.clear();
    app.diagram_outcomes(&m, outcomes);
    assert!(app.error.is_none(), "{:?}", app.error);
    let live = app.session.as_ref().unwrap();
    live.sync();
    let m = live.model();
    assert!(m
        .params
        .iter()
        .filter(|r| r.key.ends_with("/car.c"))
        .all(|r| r.value == 80.0));
    // A node pushed aside keeps its own place when moved.
    let _ = draw(&mut app);
    let car2 = d.nodes[d.node("car2").unwrap()].clone();
    let pushed = d.shifted["car2"];
    canvas_edits(
        &mut app,
        vec![Edit::Move {
            block: "car2".into(),
            at: [car2.at[0] + 10.0, car2.at[1]],
        }],
    );
    let saved = app.saved_text().unwrap();
    let (f, _) = simsandbox_file::ModelFile::from_json(&saved).unwrap();
    let at = f
        .blocks
        .iter()
        .find(|b| b.name == "car2")
        .unwrap()
        .canvas
        .unwrap();
    assert_eq!(at[0], car2.at[0] + 10.0 - pushed[0]);
    // Reopened, it opens expanded.
    let mut again = SimApp::new(None, None, None, None);
    again.open_text("again", &saved);
    assert_eq!(again.diagram.expanded, ["car1"]);
}

#[test]
fn model_parameters_set_every_block_using_them() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Platoon"))
        .unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.call(|s, _| s.set_param_key("kp", 800.0))
        .unwrap()
        .unwrap();
    let m = live.model();
    for car in ["car1", "car2", "car3"] {
        let row = m
            .params
            .iter()
            .find(|r| r.key == format!("{car}/control.kp"))
            .unwrap();
        assert_eq!((row.value, row.formula.as_deref()), (800.0, Some("kp")));
    }
}

#[test]
fn the_dock_fitting_itself_to_the_window_is_not_an_edit() {
    let mut app = SimApp::new(None, None, None, None);
    let k = EXAMPLES
        .iter()
        .position(|e| e.name.contains("Cruise"))
        .unwrap();
    app.open_example(k);
    let frame = |app: &mut SimApp, ctx: &egui::Context, height: f32| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, height),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.docked(ui));
        out.textures_delta.clear();
    };
    // A short window: the splits are fitted to the panes' contents.
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&mut app, &ctx, 260.0);
    }
    assert!(app.dock_settled.is_some(), "the dock did not fit itself");
    assert!(!app.has_unsaved(), "fitting the dock is not an edit");
    // A real change still counts: a view in a tab of its own.
    let t = app.tile_ids()[0];
    app.dock.own_tab(t);
    assert!(app.has_unsaved());
}

#[test]
fn without_a_file_the_app_opens_on_its_landing_page() {
    let mut app = SimApp::new(None, None, None, None);
    assert!(app.session.is_none());
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.docked(ui));
    out.textures_delta.clear();
    assert!(!example_description(0).is_empty());
    assert!(!app.has_unsaved());
}

#[test]
fn an_example_that_reads_files_opens_and_saves_with_them() {
    // ADR 0069 step 3: the bridge reads its nodes and wires from CSV
    // files built into the app with it.
    let mut app = SimApp::new(None, None, None, None);
    let k = EXAMPLES.iter().position(|e| !e.files.is_empty()).unwrap();
    app.open_example(k);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.path.is_none(), "an example has no path until saved");
    // The window's own builds (the diagram's nodes, the Inspector) read
    // the files too.
    let m = app.session.as_ref().unwrap().model();
    let d = simsandbox_session::diagram::Diagram::new(&m.file, &app.registry);
    let n = &d.nodes[d.node("bridge").unwrap()];
    assert!(n.error.is_none(), "{:?}", n.error);
    assert!(
        n.values.iter().any(|v| v.name == "supply"),
        "its parameters"
    );
    for p in ["bridge.supply", "bridge.C"] {
        assert!(
            m.params.iter().any(|r| r.key.ends_with(p)),
            "{p} in the Inspector"
        );
    }
    let live = app.session.as_ref().unwrap();
    assert!(live
        .call(|s, _| s.run_until(1.0).map_err(|e| e.to_string()))
        .unwrap()
        .is_ok());
    // Save as puts its files beside it, and it opens from there.
    let dir = std::env::temp_dir().join(format!("simsandbox-app-net-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bridge.ssbx.json");
    app.save_to(&path);
    assert!(app.error.is_none(), "{:?}", app.error);
    for (name, text) in EXAMPLES[k].files {
        assert_eq!(&std::fs::read_to_string(dir.join(name)).unwrap(), text);
    }
    assert!(app.example_folder.is_none());
    let mut again = SimApp::new(None, None, None, None);
    again.open_path(&path.to_string_lossy());
    assert!(again.error.is_none(), "{:?}", again.error);
    // Another example after it reads no folder of the first's.
    app.open_example(0);
    assert!(app.error.is_none() && app.example_folder.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn replacing_a_changed_model_asks_first() {
    let mut app = SimApp::new(None, None, None, None);
    assert!(!app.has_unsaved(), "a model just opened");
    // Opening something else needs no question yet.
    app.open_example(1);
    assert_eq!(app.modal, None);
    let live = app.session.as_ref().unwrap();
    let id = live.call(|s, _| s.ids["a"]).unwrap();
    live.call(move |s, _| s.set_param(id, "kappa", 0.7))
        .unwrap()
        .unwrap();
    assert!(app.has_unsaved());
    // Now it asks, and nothing is replaced until answered.
    let title = app.session.as_ref().unwrap().model().title.clone();
    app.open_example(0);
    assert_eq!(app.modal, Some(Modal::Unsaved(Discard::Example(0))));
    assert_eq!(app.session.as_ref().unwrap().model().title, title);
    // "Don't save": it goes ahead.
    app.modal = None;
    app.proceed(&egui::Context::default(), Discard::Example(0));
    assert_ne!(app.session.as_ref().unwrap().model().title, title);
    assert!(!app.has_unsaved());
    assert_ne!(app.modal, Some(Modal::Discarding));

    // Saving clears the question; canvas edits make changes too.
    let dir = std::env::temp_dir().join(format!("simsandbox-unsaved-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cruise.ssbx.json");
    let k = EXAMPLES
        .iter()
        .position(|e| e.name.contains("Cruise"))
        .unwrap();
    app.open_example(k);
    canvas_edits(
        &mut app,
        vec![
            simsandbox_session::diagram::Edit::Disconnect { to: "car.F".into() },
            simsandbox_session::diagram::Edit::Connect {
                from: "cruise.F".into(),
                to: "car.F".into(),
            },
        ],
    );
    assert!(app.has_unsaved(), "rewired");
    app.save_to(&path);
    assert!(!app.has_unsaved());
    // The speed is saved with the model, so changing it counts.
    app.speed = 2.0;
    assert!(app.has_unsaved());
    // A new model from the equation editor asks too.
    app.editor = editor(None, NEW_MODEL);
    app.apply_equations();
    assert_eq!(app.modal, Some(Modal::Unsaved(Discard::NewModel)));
    assert!(app.editor.is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// One frame of `f` with `input`, as the window gives it.
fn frame(input: egui::RawInput, mut f: impl FnMut(&egui::Context)) {
    let ctx = egui::Context::default();
    ctx.run_ui(input, |ui| f(&ui.ctx().clone()))
        .textures_delta
        .clear();
}

/// A file dropped on the window, as the integration hands it over.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// While a question or form covers the window, a dropped file and the
/// file shortcuts do nothing; without one they act.
#[test]
fn a_modal_holds_dropped_files_and_shortcuts() {
    let dir = std::env::temp_dir().join(format!("simsandbox-modal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lorenz.ssbx.json");
    std::fs::write(&file, include_str!("../../../../examples/lorenz.ssbx.json")).unwrap();
    let drop = || egui::RawInput {
        dropped_files: vec![std::sync::Arc::new(Dropped(file.clone()))],
        ..Default::default()
    };
    let mut app = SimApp::new(None, None, None, None);
    app.modal = Some(Modal::SaveRun {
        name: "push".into(),
        note: String::new(),
    });
    frame(drop(), |ctx| app.handle_dropped_files(ctx));
    assert_eq!(app.path, None);
    assert!(matches!(app.modal, Some(Modal::SaveRun { .. })));
    // Ctrl+S taken, nothing saved: the form stays.
    let ctrl_s = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    ctx.run_ui(ctrl_s, |ui| {
        app.shortcuts(ui);
        assert!(!ui.input(|i| i.events.iter().any(|e| matches!(
            e,
            egui::Event::Key {
                key: egui::Key::S,
                ..
            }
        ))));
    })
    .textures_delta
    .clear();
    assert!(matches!(app.modal, Some(Modal::SaveRun { .. })));
    // Without the form, the dropped file opens.
    app.modal = None;
    frame(drop(), |ctx| app.handle_dropped_files(ctx));
    assert_eq!(app.path.as_deref(), Some(file.as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A close agreed to with unsaved changes is let through for a few
/// frames; a window still open after them asks again.
#[test]
fn a_close_that_never_comes_asks_again() {
    let mut app = SimApp::new(None, None, None, None);
    app.open_example(0);
    app.speed = 2.0;
    assert!(app.has_unsaved());
    let ctx = egui::Context::default();
    app.proceed(&ctx, Discard::Close);
    assert!(matches!(app.modal, Some(Modal::Closing(_))));
    // While the close is awaited, replacing the model does not ask.
    assert!(!app.ask_first(Discard::NewModel));
    for _ in 0..=CLOSING_FRAMES {
        frame(egui::RawInput::default(), |ctx| app.guard_close(ctx));
    }
    assert_eq!(app.modal, None);
    assert!(app.ask_first(Discard::NewModel));
    assert_eq!(app.modal, Some(Modal::Unsaved(Discard::NewModel)));
}
