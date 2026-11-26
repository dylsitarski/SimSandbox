//! The Inspector editing a scene (ADR 0093 stage 3), through the app's
//! own methods, as its fields and buttons call them.

use super::*;
use crate::scene_edit::SceneEdit;
use simsandbox_file::{ModelFile, SceneValue};

/// The open model as Save would write it.
fn saved(app: &SimApp) -> ModelFile {
    ModelFile::from_json(&app.saved_text().unwrap()).unwrap().0
}

/// The scene tile drawn alone from the live recording.
fn picture(app: &mut SimApp, tile: u64) -> simsandbox_session::raster::Image {
    let live = app.session.as_ref().unwrap();
    live.sync();
    let h = live.history();
    let recorded = simsandbox_views::Recorded::single(&h);
    let views = &mut app.views;
    let t = views.registry.get("plot.scene").unwrap();
    let tile = views.tiles.iter_mut().find(|t| t.id == tile).unwrap();
    let o = simsandbox_session::render::RenderOptions {
        width: 300.0,
        height: 300.0,
        ..Default::default()
    };
    simsandbox_session::render::paint(&o, |ui| tile.state.show(ui, t, recorded))
}

/// Draw the inspector once, as the dock does (it follows the canvas).
fn inspect(app: &mut SimApp, name: &str) -> simsandbox_session::raster::Image {
    let o = simsandbox_session::render::RenderOptions {
        width: 520.0,
        height: 1000.0,
        ..Default::default()
    };
    let image = simsandbox_session::render::paint(&o, |ui| app.param_panel(ui));
    // SIMSANDBOX_SHOT_SCENE=FOLDER writes it there as NAME.png, to look at.
    if let Ok(dir) = std::env::var("SIMSANDBOX_SHOT_SCENE") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        std::fs::write(path, simsandbox_session::render::png(&image).unwrap()).unwrap();
    }
    image
}

#[test]
fn the_inspector_edits_a_scenes_layers_live_and_saves_them() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.contains("Grab the bob"))
        .unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    live.call(|s, _| s.run_until(0.5)).unwrap().unwrap();
    // Nothing picked, no scene edited; the scene's node picked on the
    // canvas, its tile is.
    inspect(&mut app, "nothing");
    assert_eq!(app.inspected_scene(), None);
    app.select("view:0");
    inspect(&mut app, "scene");
    let tile = app.inspected_scene().expect("the scene's tile");
    assert_eq!(app.views.tiles[0].id, tile);
    let layers = |app: &SimApp| saved(app).controls.views[0].scene.layers.clone();
    assert_eq!(layers(&app).len(), 4);
    let before = picture(&mut app, tile);

    // A layer added: a dot at the origin, drawn on top, saved.
    app.edit_scene(tile, &SceneEdit::AddLayer("points".into()));
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(layers(&app)[4].draw, "points");
    let added = picture(&mut app, tile);
    assert_ne!(added, before);
    // Its x bound to a signal: the dot moves with the bob.
    app.edit_scene(
        tile,
        &SceneEdit::Bind(4, "x".into(), Some(SceneValue::Name("bob.x".into()))),
    );
    assert_eq!(layers(&app)[4].bind["x"], SceneValue::Name("bob.x".into()));
    let bound = picture(&mut app, tile);
    assert_ne!(bound, added);
    // Grabbing with the hand: saved, and on the canvas an edge from it.
    app.edit_scene(tile, &SceneEdit::Grab(4, Some("hand".into())));
    assert_eq!(layers(&app)[4].grab.as_deref(), Some("hand"));
    let file = saved(&app);
    let mut d = simsandbox_session::diagram::Diagram::new(&file, &app.registry);
    d.observe(&file, &app.views.registry);
    assert!(d
        .wires
        .iter()
        .any(|w| w.from_name == "hand.picked" && w.to_name == "view:0.layers[4].grab"));

    // A binding that does not check is a draft: the error beside its
    // field, the scene drawn and saved as it was.
    app.edit_scene(
        tile,
        &SceneEdit::Bind(4, "y".into(), Some(SceneValue::Name("bob.omega".into()))),
    );
    let error = app.scene_drafts[&tile].error.clone();
    assert!(
        error.starts_with("layers[4].bind.y: 'bob.omega' is in 1/s"),
        "{error}"
    );
    assert_eq!(layers(&app)[4].bind["y"], SceneValue::Number(0.0));
    assert_eq!(picture(&mut app, tile), bound);
    inspect(&mut app, "draft");
    // Removing the layer from the draft checks again: applied, the
    // draft gone, the scene as it first was.
    app.edit_scene(tile, &SceneEdit::RemoveLayer(4));
    assert!(app.scene_drafts.is_empty());
    assert_eq!(layers(&app).len(), 4);
    assert_eq!(saved(&app).controls.views[0], {
        let (f, _) = ModelFile::from_json(e.text).unwrap();
        f.controls.views[0].clone()
    });
    assert_eq!(picture(&mut app, tile), before);
    // A canvas edit of its inputs (a wire taken away) is undone and
    // redone as canvas edits are; the inspector follows the scene's tile.
    let width = |app: &SimApp| layers(app)[2].bind.contains_key("width");
    super::tests::canvas_edits(
        &mut app,
        vec![simsandbox_session::diagram::Edit::Disconnect {
            to: "view:0.layers[2].bind.width".into(),
        }],
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(!width(&app));
    inspect(&mut app, "after-canvas");
    let tile = app.inspected_scene().expect("still the scene's tile");
    super::tests::canvas_step(&mut app, true);
    assert!(width(&app));
    super::tests::canvas_step(&mut app, false);
    assert!(!width(&app));
    // Undo clears the canvas's selection, and with it the scene's
    // editor.
    inspect(&mut app, "redone");
    assert_eq!(app.inspected_scene(), None);
    assert_ne!(app.views.tiles[0].id, tile, "a new tile each time");
    // Picking a block on the canvas leaves the scene.
    app.select("bob");
    inspect(&mut app, "block");
    assert_eq!(app.inspected_scene(), None);
    assert!(app.error.is_none(), "{:?}", app.error);
}

/// The pendulum's scene, its node picked: the tile the Inspector edits.
fn pendulum(app: &mut SimApp) -> u64 {
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Pendulum (with its own view)"))
        .unwrap();
    app.open_text(e.name, e.text);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    live.call(|s, _| s.run_until(0.5)).unwrap().unwrap();
    app.select("view:1");
    inspect(app, "pendulum");
    app.inspected_scene().expect("the scene's tile")
}

#[test]
fn a_frame_renamed_in_the_inspector_is_renamed_where_it_is_named() {
    let mut app = SimApp::new(None, None, None, None);
    let tile = pendulum(&mut app);
    let before = picture(&mut app, tile);
    let scene = |app: &SimApp| saved(app).controls.views[1].scene.clone();
    app.edit_scene(tile, &SceneEdit::RenameFrame("rod".into(), "arm".into()));
    assert!(app.error.is_none(), "{:?}", app.error);
    let s = scene(&app);
    assert_eq!(s.frames.keys().collect::<Vec<_>>(), ["arm", "pivot"]);
    assert_eq!(s.frames["arm"].parent.as_deref(), Some("pivot"));
    assert!(s.layers[1..]
        .iter()
        .all(|l| l.frame.as_deref() == Some("arm")));
    // The same picture, under a new name.
    assert_eq!(picture(&mut app, tile), before);
    inspect(&mut app, "frame-renamed");
    // A parent renamed: the frame in it follows.
    app.edit_scene(
        tile,
        &SceneEdit::RenameFrame("pivot".into(), "hinge".into()),
    );
    assert_eq!(scene(&app).frames["arm"].parent.as_deref(), Some("hinge"));
    // Refused: a name taken, or not a name; the scene is as it was.
    let kept = scene(&app);
    for to in ["hinge", "two words"] {
        app.error = None;
        app.edit_scene(tile, &SceneEdit::RenameFrame("arm".into(), to.into()));
        assert!(app.error.is_some(), "{to}");
        assert_eq!(scene(&app), kept);
    }
    assert!(app.scene_drafts.is_empty());
}

/// The canvas as the diagram tab draws it, written as NAME.png where
/// SIMSANDBOX_SHOT_SCENE says.
fn canvas(app: &mut SimApp, name: &str) -> simsandbox_session::raster::Image {
    let o = simsandbox_session::render::RenderOptions {
        width: 1100.0,
        height: 600.0,
        ..Default::default()
    };
    let image = simsandbox_session::render::paint(&o, |ui| app.diagram_tab(ui));
    if let Ok(dir) = std::env::var("SIMSANDBOX_SHOT_SCENE") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        std::fs::write(path, simsandbox_session::render::png(&image).unwrap()).unwrap();
    }
    image
}

/// The scene's node as the canvas has it, marked by the app's problems.
fn scene_node(app: &SimApp) -> simsandbox_session::diagram::Node {
    let file = saved(app);
    let mut d = simsandbox_session::diagram::Diagram::new(&file, &app.registry);
    d.observe(&file, &app.views.registry);
    d.mark_views(&app.view_problems());
    d.nodes[d.node("view:1").unwrap()].clone()
}

#[test]
fn a_draft_scene_marks_its_node_with_its_error() {
    let mut app = SimApp::new(None, None, None, None);
    let tile = pendulum(&mut app);
    assert!(app.view_problems().is_empty());
    let n = scene_node(&app);
    assert_eq!(n.error, None);
    let clean = canvas(&mut app, "canvas-clean");
    // The rod's end bound to a speed: a draft, its error on the node and
    // on the input at its path.
    app.edit_scene(
        tile,
        &SceneEdit::Bind(1, "x2".into(), Some(SceneValue::Name("bob.omega".into()))),
    );
    let error = app.scene_drafts[&tile].error.clone();
    assert!(
        error.starts_with("layers[1].bind.x2: 'bob.omega' is in 1/s"),
        "{error}"
    );
    assert_eq!(app.view_problems(), [(1, error.clone())]);
    let n = scene_node(&app);
    assert_eq!(n.error.as_deref(), Some(error.as_str()));
    let at = simsandbox_session::observers::view_problem_input(&n).unwrap();
    assert_eq!(n.inputs[at].name, "layers[1].bind.x2");
    let marked = canvas(&mut app, "canvas-draft");
    assert_ne!(marked, clean);
    // Discarded, the node is clean again.
    app.scene_drafts.clear();
    assert_eq!(scene_node(&app).error, None);
    assert_eq!(canvas(&mut app, "canvas-discarded"), clean);
}

#[test]
fn a_saved_scene_that_does_not_check_is_kept_and_says_why() {
    let mut app = SimApp::new(None, None, None, None);
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Pendulum (with its own view)"))
        .unwrap();
    // A signal the scene binds, renamed in the file by hand.
    let text = e
        .text
        .replace("\"angle\": \"bob.angle\"", "\"angle\": \"bob.angel\"");
    assert_ne!(text, e.text);
    app.open_text("renamed", &text);
    let live = app.session.as_ref().unwrap();
    live.set_running(false);
    live.call(|s, _| s.run_until(0.5)).unwrap().unwrap();
    let error = "frames.rod.angle: no signal or parameter 'bob.angel'";
    // Its tile is kept and says why; the log says it too.
    assert_eq!(app.views.tiles.len(), 3);
    let tile = app.views.tiles[1].id;
    assert_eq!(app.views.tiles[1].state.problem(), Some(error));
    assert!(
        app.log.entries().iter().any(|l| l.text.contains(error)),
        "the log"
    );
    let held = picture(&mut app, tile);
    // Saved back as it was, not dropped.
    assert_eq!(saved(&app).controls.views, {
        let (f, _) = ModelFile::from_json(&text).unwrap();
        f.controls.views
    });
    // Its node is marked, at the frame's angle.
    assert_eq!(app.view_problems(), [(1, error.to_string())]);
    let n = scene_node(&app);
    assert_eq!(n.error.as_deref(), Some(error));
    let at = simsandbox_session::observers::view_problem_input(&n).unwrap();
    assert_eq!(n.inputs[at].name, "frames.rod.angle");
    canvas(&mut app, "canvas-saved");
    // The Inspector shows the error beside the field; an edit that makes
    // it check draws it.
    app.select("view:1");
    inspect(&mut app, "held");
    assert_eq!(app.inspected_scene(), Some(tile));
    app.edit_scene(
        tile,
        &SceneEdit::Pose(
            "rod".into(),
            "angle",
            Some(SceneValue::Name("bob.angle".into())),
        ),
    );
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.views.tiles[1].state.problem().is_none());
    assert!(app.view_problems().is_empty());
    assert_ne!(picture(&mut app, tile), held);
    assert_eq!(scene_node(&app).error, None);
}

#[test]
fn the_cart_pole_opens_its_picture_drawn_from_a_template() {
    // ADR 0113 step 1: the scene's template layer is put in place to draw,
    // and saved back as written.
    let mut app = SimApp::new(None, None, None, None);
    let k = EXAMPLES
        .iter()
        .position(|e| e.name.starts_with("Cart-pole"))
        .unwrap();
    app.open_example(k);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.views.tiles.len(), 2);
    let scene = &app.views.tiles[0];
    assert!(
        scene.state.problem().is_none(),
        "{:?}",
        scene.state.problem()
    );
    assert!(scene.written.is_some());
    let shown = &app.views.shown_entries()[0];
    assert!(shown.scene.frames.contains_key("rig/pole"));
    assert!(app.view_reads.iter().any(|r| r == "cartpole.hinge_angle"));
    // Saved as written: the template layer, not what it draws.
    let file = saved(&app);
    assert_eq!(file.controls.views[0].scene.layers[1].draw, "template");
    assert!(file.templates.contains_key("cartpole"));
    let id = app.views.tiles[0].id;
    let image = picture(&mut app, id);
    // The pole is drawn (orange, its template's default).
    assert!(image
        .rgba
        .chunks(4)
        .any(|p| p[0] > 200 && p[1] > 100 && p[1] < 200 && p[2] < 100));
}

#[test]
fn a_components_picture_grabs_its_own_pointer_in_its_own_frame() {
    // ADR 0113: "Two balancing poles" draws the cart-pole's picture twice;
    // each grabs its own use's pointer (`left/hand`), in the frame it is
    // drawn in, so the controller sees the pointer as in its own scene.
    let mut app = SimApp::new(None, None, None, None);
    let k = EXAMPLES
        .iter()
        .position(|e| e.name.starts_with("Two balancing poles"))
        .unwrap();
    app.open_example(k);
    assert!(app.error.is_none(), "{:?}", app.error);
    let scene = &app.views.tiles[0];
    assert!(
        scene.state.problem().is_none(),
        "{:?}",
        scene.state.problem()
    );
    let shown = &app.views.shown_entries()[0];
    let grabbing: Vec<(&str, &serde_json::Value)> = shown
        .scene
        .layers
        .iter()
        .filter_map(|l| Some((l.grab.as_deref()?, &l.other["grab_in"])))
        .collect();
    assert_eq!(
        grabbing,
        [
            ("left/hand", &serde_json::json!("left_track")),
            ("right/hand", &serde_json::json!("right_track"))
        ]
    );
    // The pointer reaches the left cart-pole's own input block.
    app.send_pointer("left/hand", &[0.6, 1.0, 1.0, 0.0]);
    let live = app.session.as_ref().unwrap();
    live.sync();
    let m = live.model();
    let down = m
        .params
        .iter()
        .find(|r| r.key == "left/hand.down")
        .map(|r| r.value);
    assert_eq!(down, Some(1.0));
    let right = m
        .params
        .iter()
        .find(|r| r.key == "right/hand.down")
        .map(|r| r.value);
    assert_eq!(right, Some(0.0));
}
