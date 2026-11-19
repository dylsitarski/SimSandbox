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
