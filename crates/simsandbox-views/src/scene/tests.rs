use super::eval::{self, Placed};
use super::geometry::{Instance, Item};
use super::spec::{palette, Scene};
use super::*;
use crate::history::{FieldShape, History, Recorded};
use crate::view::{Available, Bindings, Settings};
use serde_json::json;
use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

fn settings(v: serde_json::Value) -> Settings {
    v.as_object().unwrap().clone().into_iter().collect()
}

/// A history of `cart.x` (m), `pole.theta` (rad), `car.v` (m/s) and two
/// tables: `rb.x`, `rb.y`, `rb.angle` over `rb`'s `bodies`, and `w.x` over
/// `w`'s `wheels`.
fn history() -> History {
    let fields = ["rb.x", "rb.y", "rb.angle", "w.x"]
        .map(|n| (n.to_string(), FieldShape::Rows { dim: 1 }))
        .to_vec();
    let mut h = History::with_fields(
        "t",
        vec!["cart.x".into(), "pole.theta".into(), "car.v".into()],
        fields,
    );
    h.set_unit("cart.x", "m");
    h.set_unit("car.v", "m/s");
    for n in ["rb.x", "rb.y"] {
        h.set_unit(n, "m");
    }
    for n in ["rb.x", "rb.y", "rb.angle"] {
        h.set_axes(n, vec!["bodies".into()]);
    }
    h.set_axes("w.x", vec!["wheels".into()]);
    h
}

/// A rows frame: version 1, the ids, then one value a row.
fn rows(ids: &[f64], values: &[f64]) -> Vec<f64> {
    let mut v = vec![1.0, ids.len() as f64];
    v.extend_from_slice(ids);
    v.extend_from_slice(values);
    v
}

fn push(h: &mut History, t: f64, cart: f64, theta: f64) {
    // The bodies' rows in one order, their angles in another: by id.
    h.push_sample(
        t,
        &[cart, theta, 1.0],
        &[
            &rows(&[7.0, 3.0], &[1.0, 2.0]),
            &rows(&[7.0, 3.0], &[10.0, 20.0]),
            &rows(&[3.0, 7.0], &[FRAC_PI_2, 0.0]),
            &rows(&[1.0], &[5.0]),
        ],
    );
}

struct Offer {
    scalars: Vec<String>,
    rows: Vec<String>,
}

fn offer() -> Offer {
    Offer {
        scalars: vec!["cart.x".into(), "pole.theta".into(), "car.v".into()],
        rows: vec![
            "rb.x".into(),
            "rb.y".into(),
            "rb.angle".into(),
            "w.x".into(),
        ],
    }
}

fn available<'a>(o: &'a Offer, h: &'a History) -> Available<'a> {
    Available {
        scalars: &o.scalars,
        rows: &o.rows,
        meta: Some(h),
        ..Default::default()
    }
}

fn check(v: serde_json::Value) -> Result<Scene, String> {
    let h = history();
    let o = offer();
    Scene::check(&settings(v), &available(&o, &h))
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12
}

fn centre(p: &Placed) -> [f64; 2] {
    match p.shape {
        Instance::Point { at, .. } => at,
        Instance::Box { centre, .. } | Instance::Disc { centre, .. } => centre,
        Instance::Segment { from, .. } => from,
    }
}

#[test]
fn the_adr_example_checks_and_defaults_fill_what_is_unbound() {
    let s = check(json!({"unit": "m", "extent": {"x": [-3, 3], "y": [-0.5, 2]},
        "frames": {
            "cart": {"x": "cart.x", "y": 0},
            "pole": {"in": "cart", "angle": "pole.theta"}
        },
        "layers": [
            {"draw": "segments", "bind": {"x1": -3, "y1": 0, "x2": 3, "y2": 0}},
            {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}, "grab": "hand"},
            {"draw": "boxes", "in": "pole", "bind": {"x": 0, "y": 0.5, "width": 0.04, "height": 1.0}},
            {"draw": "discs", "in": "pole", "bind": {"x": 0, "y": 1.0, "radius": 0.05, "colour": "orange"}, "grab": "hand"}
        ]}))
    .unwrap();
    assert_eq!(s.layers.len(), 4);
    assert_eq!(s.layers[1].grab.as_deref(), Some("hand"));
    assert_eq!(s.reads(), ["cart.x", "pole.theta"]);
    // A segment's width and colour, a box's angle: their defaults.
    use spec::Source;
    assert_eq!(s.layers[0].attributes[4], Some(Source::Constant(2.0)));
    assert_eq!(
        s.layers[0].attributes[5],
        Some(Source::Colour(palette("sky").unwrap()))
    );
    assert_eq!(s.layers[1].attributes[2], Some(Source::Constant(0.0)));
    // A disc's mark is drawn only when its angle is bound.
    assert_eq!(s.layers[3].attributes[3], None);
    assert_eq!(
        s.layers[3].attributes[4],
        Some(Source::Colour(palette("orange").unwrap()))
    );
    // Nothing given: an empty scene in metres.
    let empty = check(json!({})).unwrap();
    assert_eq!((empty.unit.as_str(), empty.layers.len()), ("m", 0));
}

#[test]
fn check_reports_each_problem_at_its_path() {
    let layer =
        |l: serde_json::Value| json!({"layers": [{"draw": "points", "bind": {"x": 0, "y": 0}}, l]});
    for (scene, error) in [
        (json!({"layer": []}), "'plot.scene' takes no setting 'layer'"),
        (layer(json!({"draw": "circles"})), "layers[1].draw: no primitive 'circles'"),
        (layer(json!({"draw": "discs", "bind": {"x": 0, "y": 0, "radius": 1, "r": 2}})), "layers[1].bind.r: 'discs' has no attribute 'r'"),
        (layer(json!({"draw": "discs", "bind": {"x": 0, "y": 0}})), "layers[1].bind.radius: required"),
        (layer(json!({"draw": "points", "bind": {"x": "cart.y", "y": 0}})), "layers[1].bind.x: no signal 'cart.y'"),
        (layer(json!({"draw": "points", "bind": {"x": 0, "y": 0, "colour": "mauve"}})), "layers[1].bind.colour: no signal or palette colour 'mauve'"),
        (layer(json!({"draw": "points", "bind": {"x": "car.v", "y": 0}})), "layers[1].bind.x: 'car.v' is in m/s; the scene's lengths are in m"),
        (layer(json!({"draw": "boxes", "bind": {"x": 0, "y": 0, "width": 1, "height": 1, "angle": "cart.x"}})), "layers[1].bind.angle: 'cart.x' is in m; an angle takes radians"),
        (layer(json!({"draw": "points", "bind": {"x": 0, "y": 0, "size": "cart.x"}})), "layers[1].bind.size: 'cart.x' is in m; it is in screen points"),
        (layer(json!({"draw": "points", "bind": {"x": 0, "y": 0}, "in": "cart"})), "layers[1].in: no frame 'cart'"),
        (layer(json!({"draw": "points", "bind": {"x": "rb.x", "y": "w.x"}})), "layers[1].bind.y: over the rows of 'w.wheels', but layers[1].bind.x is over the rows of 'rb.bodies'"),
        (layer(json!({"draw": "points", "bind": {"x": 0, "y": 0}, "fill": 1})), "layers[1].fill: a layer takes draw, in, bind and grab"),
        (json!({"unit": "mm", "layers": [{"draw": "points", "bind": {"x": "cart.x", "y": 0}}]}), "layers[0].bind.x: 'cart.x' is in m; the scene's lengths are in mm"),
        (json!({"frames": {"a": {"in": "b"}, "b": {"in": "a"}}}), "frames.a.in: the frames a in b in a form a cycle"),
        (json!({"frames": {"a": {"in": "a"}}}), "frames.a.in: the frames a in a form a cycle"),
        (json!({"frames": {"a": {"in": "nowhere"}}}), "frames.a.in: no frame 'nowhere'"),
        (json!({"frames": {"a": {"x": "car.v"}}}), "frames.a.x: 'car.v' is in m/s"),
        (json!({"frames": {"a": {"z": 1}}}), "frames.a.z: a frame takes in, x, y and angle"),
        (json!({"frames": {"body": {"x": "rb.x"}, "wheel": {"in": "body"}}}), "frames.wheel.in: 'body' is over a table's rows, so it cannot hold another frame yet"),
        (json!({"frames": {"body": {"x": "rb.x"}}, "view_from": "body"}), "view_from: 'body' is over a table's rows"),
        (json!({"view_from": "cart"}), "view_from: no frame 'cart'"),
        (json!({"frames": {"wheel": {"x": "w.x"}}, "layers": [{"draw": "points", "in": "wheel", "bind": {"x": "rb.x", "y": 0}}]}), "layers[0].bind.x: over the rows of 'rb.bodies', but layers[0].in is over the rows of 'w.wheels'"),
        (json!({"extent": {"x": [1, 0], "y": [0, 1]}}), "extent.x: an increasing finite range"),
        (json!({"extent": {"x": [0, 1]}}), "extent.y: required"),
    ] {
        let e = check(scene.clone()).unwrap_err();
        assert!(e.starts_with(error), "{scene}:\n  got {e}\n  not {error}");
    }
}

/// The ADR's cart and pole, read at the newest sample: frames chain (the
/// pole turns in the moving cart), and `view_from` draws in a frame's
/// coordinates.
#[test]
fn frames_chain_and_view_from_draws_in_a_frame() {
    let mut h = history();
    let o = offer();
    let scene = json!({
    "frames": {
        "cart": {"x": "cart.x", "y": 0.5},
        "pole": {"in": "cart", "angle": "pole.theta"},
        "tip": {"in": "pole", "y": 1.0, "angle": -0.25}
    },
    "layers": [
        {"draw": "discs", "in": "pole", "bind": {"x": 0, "y": 1.0, "radius": 0.1, "angle": 0}},
        {"draw": "points", "in": "tip", "bind": {"x": 0.5, "y": 0}}
    ]});
    let s = Scene::check(&settings(scene.clone()), &available(&o, &h)).unwrap();
    // Nothing recorded: nothing placed.
    assert!(s.place(&Recorded::single(&h)).is_empty());
    push(&mut h, 0.0, 9.0, 0.0);
    push(&mut h, 0.1, 2.0, FRAC_PI_2);
    let placed = s.place(&Recorded::single(&h));
    // The pole's top, turned a quarter left, is 1 to the left of the cart.
    assert!(close(centre(&placed[0]), [1.0, 0.5]), "{placed:?}");
    let Instance::Disc { angle, .. } = placed[0].shape else {
        panic!()
    };
    assert!((angle.unwrap() - FRAC_PI_2).abs() < 1e-12);
    // (0.5, 0) in the tip frame: turned by π/2 − 0.25, from the pole's top.
    let a = FRAC_PI_2 - 0.25;
    let want = [1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()];
    assert!(close(centre(&placed[1]), want));
    // Seen from the pole, the pole stands still: its top is at (0, 1).
    let mut from = settings(scene);
    from.insert("view_from".into(), json!("pole"));
    let s = Scene::check(&from, &available(&o, &h)).unwrap();
    let view = s.view(&Recorded::single(&h)).unwrap();
    let drawn = eval::draw(&s.place(&Recorded::single(&h)), view, 100.0);
    let Item::Triangles { points, .. } = &drawn.items[0] else {
        panic!()
    };
    assert!(close(points[0], [0.0, 1.0]), "{:?}", points[0]);
    // The box holding it: the disc, and the tip's point turned back by 0.25.
    let [lo, hi] = drawn.bounds.unwrap();
    let tip = [0.5 * 0.25f64.cos(), 1.0 - 0.5 * 0.25f64.sin()];
    assert!(
        close(lo, [-0.1, tip[1]]) && close(hi, [tip[0], 1.1]),
        "{lo:?} {hi:?}"
    );
    // And its mark points along the pole's x.
    let Item::Line { points: mark, .. } = &drawn.items[1] else {
        panic!()
    };
    assert!(close(mark[1], [0.1, 1.0]), "{mark:?}");
}

#[test]
fn rows_pair_by_id_and_a_frame_over_rows_places_each_row() {
    let mut h = history();
    let o = offer();
    push(&mut h, 0.0, 0.0, 0.0);
    // Each body's box in its own frame: x, y and angle paired by id
    // though the angles come in another order.
    let s = Scene::check(
        &settings(json!({
        "frames": {"body": {"x": "rb.x", "y": "rb.y", "angle": "rb.angle"}},
        "layers": [
            {"draw": "points", "in": "body", "bind": {"x": 1, "y": 0}},
            {"draw": "points", "bind": {"x": "rb.x", "y": "rb.y"}},
            {"draw": "points", "bind": {"x": "w.x", "y": 0}}
        ]})),
        &available(&o, &h),
    )
    .unwrap();
    assert_eq!(s.frames[0].table.as_deref(), Some("rb.bodies"));
    let placed = s.place(&Recorded::single(&h));
    let at = |layer: usize, id: u64| {
        centre(
            placed
                .iter()
                .find(|p| p.layer == layer && p.id == Some(id))
                .unwrap(),
        )
    };
    // Body 7 at (1, 10) not turned; body 3 at (2, 20) turned a quarter.
    assert!(close(at(0, 7), [2.0, 10.0]));
    assert!(close(at(0, 3), [2.0, 21.0]));
    assert!(close(at(1, 7), [1.0, 10.0]));
    assert!(close(at(1, 3), [2.0, 20.0]));
    // In the table's order; another table's rows on their own.
    let ids: Vec<_> = placed.iter().map(|p| (p.layer, p.id)).collect();
    assert_eq!(
        ids,
        [
            (0, Some(7)),
            (0, Some(3)),
            (1, Some(7)),
            (1, Some(3)),
            (2, Some(1))
        ]
    );
    // A row missing from one binding is left out, not drawn at NaN.
    h.push_sample(
        0.1,
        &[0.0, 0.0, 1.0],
        &[
            &rows(&[7.0, 3.0], &[1.0, 2.0]),
            &rows(&[7.0], &[10.0]),
            &rows(&[3.0, 7.0], &[FRAC_PI_2, 0.0]),
            &rows(&[], &[]),
        ],
    );
    let placed = s.place(&Recorded::single(&h));
    let ids: Vec<_> = placed.iter().map(|p| (p.layer, p.id)).collect();
    assert_eq!(ids, [(0, Some(7)), (1, Some(7))]);
}

#[test]
fn colours_come_from_the_palette_or_the_scale() {
    let mut h = history();
    let o = offer();
    push(&mut h, 0.0, 0.25, 7.0);
    let s = Scene::check(
        &settings(json!({"layers": [
            {"draw": "points", "bind": {"x": 0, "y": 0, "colour": "red"}},
            {"draw": "points", "bind": {"x": 0, "y": 0, "colour": 0}},
            {"draw": "points", "bind": {"x": 0, "y": 0, "colour": "pole.theta"}},
            {"draw": "points", "bind": {"x": 0, "y": 0}}
        ]})),
        &available(&o, &h),
    )
    .unwrap();
    let colours: Vec<_> = s
        .place(&Recorded::single(&h))
        .iter()
        .map(|p| match p.shape {
            Instance::Point { colour, .. } => colour,
            _ => unreachable!(),
        })
        .collect();
    use crate::colour::sequential;
    assert_eq!(
        colours,
        [
            palette("red").unwrap(),
            sequential(0.0),
            sequential(1.0),
            palette("sky").unwrap()
        ]
    );
}

#[test]
fn grabbing_takes_the_topmost_grabbed_instance_containing_the_pointer() {
    let h = history();
    let o = offer();
    let s = Scene::check(
        &settings(json!({"layers": [
            {"draw": "boxes", "bind": {"x": 0, "y": 0, "width": 2, "height": 2}, "grab": "hand"},
            {"draw": "discs", "bind": {"x": 1, "y": 0, "radius": 0.5}, "grab": "hand"},
            {"draw": "discs", "bind": {"x": 0, "y": 0, "radius": 0.2}}
        ]})),
        &available(&o, &h),
    )
    .unwrap();
    let placed = s.place(&Recorded::single(&h));
    let at = |p| eval::picked(&s, &placed, p, 100.0).map(|i| i.layer);
    assert_eq!(at([1.0, 0.0]), Some(1));
    assert_eq!(at([0.9, 0.9]), Some(0));
    // The ungrabbed disc on top is passed over.
    assert_eq!(at([0.0, 0.0]), Some(0));
    assert_eq!(at([1.6, 0.0]), None);
}

#[test]
fn reads_are_the_signals_the_settings_bind() {
    let r = reads(&settings(json!({
    "frames": {"a": {"x": "cart.x", "angle": 1}},
    "layers": [
        {"draw": "points", "bind": {"x": "rb.x", "y": "probe:p/rb.y", "colour": "red"}},
        {"draw": "points", "bind": {"x": 0, "y": 0, "colour": "car.v"}}
    ]})));
    assert_eq!(r, ["car.v", "cart.x", "probe:p/rb.y", "rb.x"]);
}

/// Draw the view for a frame in a headless egui context, as the app would.
fn frame(
    ctx: &egui::Context,
    view: &mut SceneView,
    h: &History,
    size: [f32; 2],
) -> Vec<egui::epaint::ClippedShape> {
    let registry = crate::view::ViewRegistry::builtin();
    let t = registry.get(SCENE).unwrap();
    let bind = Bindings::new();
    let style = BTreeMap::new();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        let input = crate::view::view_input(&bind, &style, t, Recorded::single(h));
        view.show(ui, &input);
    });
    out.textures_delta.clear();
    out.shapes
}

#[test]
fn a_still_scene_is_not_rebuilt_and_a_new_sample_or_size_rebuilds_it() {
    let mut h = history();
    let o = offer();
    push(&mut h, 0.0, 0.0, 0.0);
    let mut view = SceneView::default();
    view.set_settings(
        &settings(json!({"extent": {"x": [-2, 2], "y": [-2, 2]},
            "frames": {"cart": {"x": "cart.x"}},
            "layers": [{"draw": "discs", "in": "cart", "bind": {"x": 0, "y": 0, "radius": 0.5}}]})),
        &available(&o, &h),
    )
    .unwrap();
    let ctx = egui::Context::default();
    // The plot settles its bounds over the first frames.
    for _ in 0..4 {
        frame(&ctx, &mut view, &h, [800.0, 600.0]);
    }
    let settled = view.builds();
    for _ in 0..5 {
        frame(&ctx, &mut view, &h, [800.0, 600.0]);
    }
    assert_eq!(view.builds(), settled, "a still scene was rebuilt");
    push(&mut h, 0.1, 0.5, 0.0);
    frame(&ctx, &mut view, &h, [800.0, 600.0]);
    assert_eq!(view.builds(), settled + 1);
    frame(&ctx, &mut view, &h, [800.0, 600.0]);
    assert_eq!(view.builds(), settled + 1);
    frame(&ctx, &mut view, &h, [500.0, 600.0]);
    frame(&ctx, &mut view, &h, [500.0, 600.0]);
    assert!(view.builds() > settled + 1, "a new size was not rebuilt");
    // New settings start over.
    view.set_settings(&settings(json!({})), &available(&o, &h))
        .unwrap();
    assert_eq!(view.builds(), 0);
}

#[test]
fn the_scene_type_is_registered_without_slots_and_described() {
    let r = crate::view::ViewRegistry::builtin();
    let t = r.get(SCENE).unwrap();
    assert!(t.slots.is_empty() && t.presets.is_empty());
    let d = (t.settings.unwrap())();
    let names: Vec<&str> = d["primitives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["points", "segments", "boxes", "discs"]);
    let radius = &d["primitives"][3]["attributes"][2];
    assert_eq!(radius["name"], "radius");
    assert_eq!(radius["kind"], "length");
    assert_eq!(radius["required"], true);
    let size = &d["primitives"][0]["attributes"][2];
    assert_eq!(
        (size["unit"].as_str(), size["default"].as_f64()),
        (Some("pt"), Some(6.0))
    );
}
