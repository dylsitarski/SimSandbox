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

#[test]
fn a_grab_must_name_an_input_pointer_block() {
    let h = history();
    let o = offer();
    let hands = ["hand".to_string()];
    let a = Available {
        pointers: Some(&hands),
        ..available(&o, &h)
    };
    let layer = |grab: &str| {
        settings(json!({"layers": [
            {"draw": "discs", "bind": {"x": 0, "y": 0, "radius": 1}, "grab": grab}
        ]}))
    };
    assert!(Scene::check(&layer("hand"), &a).is_ok());
    assert_eq!(
        Scene::check(&layer("cart"), &a).unwrap_err(),
        "layers[0].grab: no input.pointer block 'cart' (grabbing sends the pointer to one)"
    );
}

/// A scene view driven by simulated pointer events, frame by frame, as
/// the app drives it.
struct Driven {
    ctx: egui::Context,
    view: SceneView,
    h: History,
}

impl Driven {
    fn new(scene: serde_json::Value) -> Self {
        let mut h = history();
        push(&mut h, 0.0, 0.5, FRAC_PI_2);
        let o = offer();
        let mut view = SceneView::default();
        view.set_settings(&settings(scene), &available(&o, &h))
            .unwrap();
        let mut d = Driven {
            ctx: egui::Context::default(),
            view,
            h,
        };
        // The plot settles its bounds over the first frames.
        for _ in 0..4 {
            d.frame(Vec::new());
        }
        d
    }

    /// One frame with these events; what grabbing sent in it.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, [f64; 4])> {
        let registry = crate::view::ViewRegistry::builtin();
        let t = registry.get(SCENE).unwrap();
        let bind = Bindings::new();
        let style = BTreeMap::new();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let (view, h) = (&mut self.view, &self.h);
        let mut out = self.ctx.run_ui(raw, |ui| {
            let input = crate::view::view_input(&bind, &style, t, Recorded::single(h));
            view.show(ui, &input);
        });
        out.textures_delta.clear();
        self.view.grabs()
    }

    /// Where a point given in the scene's own frame is on screen.
    fn screen(&self, p: [f64; 2]) -> egui::Pos2 {
        let [x, y] = self.view.kept.view.unwrap().apply(p);
        self.view
            .shown
            .unwrap()
            .position_from_point(&PlotPoint::new(x, y))
    }

    fn bounds(&self) -> PlotBounds {
        *self.view.shown.unwrap().bounds()
    }

    fn press(&mut self, p: egui::Pos2) -> Vec<(String, [f64; 4])> {
        self.frame(vec![egui::Event::PointerMoved(p)]);
        self.frame(vec![button(p, true)])
    }

    fn release(&mut self, p: egui::Pos2) -> Vec<(String, [f64; 4])> {
        self.frame(vec![button(p, false)])
    }
}

fn button(p: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

/// `[x, y, down, picked]` sent, within a fraction of a screen point.
fn near(sent: &(String, [f64; 4]), block: &str, want: [f64; 4]) -> bool {
    let [x, y, down, picked] = sent.1;
    sent.0 == block
        && (x - want[0]).abs() < 0.01
        && (y - want[1]).abs() < 0.01
        && down == want[2]
        && picked == want[3]
}

const EXTENT: [[f64; 2]; 2] = [[-2.0, 2.0], [-2.0, 2.0]];

fn extent() -> serde_json::Value {
    json!({"x": EXTENT[0], "y": EXTENT[1]})
}

#[test]
fn pressing_dragging_and_releasing_an_instance_sends_the_pointer_until_let_go() {
    let mut d = Driven::new(json!({"extent": extent(), "layers": [
        {"draw": "discs", "bind": {"x": 1, "y": 0, "radius": 0.3}, "grab": "hand"}
    ]}));
    let before = d.bounds();
    let sent = d.press(d.screen([1.1, 0.0]));
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(near(&sent[0], "hand", [1.1, 0.0, 1.0, 0.0]), "{sent:?}");
    // Dragged far off the disc, and off the plot: still held, still sent,
    // and the plot does not pan.
    for (k, p) in [[0.0, 0.5], [-1.5, -1.0], [-3.0, 0.0]]
        .into_iter()
        .enumerate()
    {
        let sent = d.frame(vec![egui::Event::PointerMoved(d.screen(p))]);
        assert_eq!(sent.len(), 1, "move {k}: {sent:?}");
        assert!(
            near(&sent[0], "hand", [p[0], p[1], 1.0, 0.0]),
            "move {k}: {sent:?}"
        );
    }
    // Held still, it is sent again every frame (the app records changes
    // only).
    let still = d.frame(Vec::new());
    assert!(near(&still[0], "hand", [-3.0, 0.0, 1.0, 0.0]), "{still:?}");
    // Scrolling does not zoom while held.
    d.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 200.0),
        modifiers: Default::default(),
        phase: egui::TouchPhase::Move,
    }]);
    assert_eq!(d.bounds(), before, "panned or zoomed while held");
    let at = d.screen([-3.0, 0.0]);
    let sent = d.release(at);
    assert_eq!(sent.len(), 1);
    assert!(near(&sent[0], "hand", [-3.0, 0.0, 0.0, -1.0]), "{sent:?}");
    // Let go, nothing more is sent.
    assert!(d
        .frame(vec![egui::Event::PointerMoved(d.screen([0.0, 0.0]))])
        .is_empty());
    assert!(d.frame(Vec::new()).is_empty());
}

#[test]
fn pressing_off_every_grabbing_instance_sends_nothing_and_pans() {
    let mut d = Driven::new(json!({"extent": extent(), "layers": [
        {"draw": "discs", "bind": {"x": 1, "y": 0, "radius": 0.3}, "grab": "hand"},
        // Not grabbing: pressing on it pans.
        {"draw": "discs", "bind": {"x": -1, "y": 0, "radius": 0.3}}
    ]}));
    let before = d.bounds();
    let p = d.screen([-1.0, 0.0]);
    assert!(d.press(p).is_empty());
    for k in 1..=10 {
        let sent = d.frame(vec![egui::Event::PointerMoved(
            p + egui::vec2(8.0 * k as f32, 0.0),
        )]);
        assert!(sent.is_empty());
    }
    assert!(d.release(p + egui::vec2(80.0, 0.0)).is_empty());
    assert_ne!(
        d.bounds(),
        before,
        "a drag off the instances pans as plots do"
    );
}

#[test]
fn the_topmost_grabbing_layer_wins_and_rows_send_their_ids() {
    // `rb`'s bodies: id 7 at (1, 10), id 3 at (2, 20); a box under them
    // grabbing with another block.
    let mut d = Driven::new(json!({"layers": [
        {"draw": "boxes", "bind": {"x": 1.5, "y": 15, "width": 4, "height": 14}, "grab": "under"},
        {"draw": "discs", "bind": {"x": "rb.x", "y": "rb.y", "radius": 0.5}, "grab": "hand"},
        // On top but not grabbing: passed over.
        {"draw": "discs", "bind": {"x": 2, "y": 20, "radius": 0.2}}
    ]}));
    for (p, block, picked) in [
        ([2.0, 20.0], "hand", 3.0),
        ([1.0, 10.2], "hand", 7.0),
        ([1.5, 15.0], "under", 0.0),
    ] {
        let at = d.screen(p);
        let sent = d.press(at);
        assert!(
            near(&sent[0], block, [p[0], p[1], 1.0, picked]),
            "{p:?}: {sent:?}"
        );
        let sent = d.release(at);
        assert!(
            near(&sent[0], block, [p[0], p[1], 0.0, -1.0]),
            "{p:?}: {sent:?}"
        );
    }
}

#[test]
fn grabbing_sends_the_scene_frame_under_view_from_and_in_a_turned_frame() {
    // `cart.x` is 0.5 and `pole.theta` π/2: the pole's frame stands at
    // (0.5, 0) turned a quarter, so its (1, 0) is the scene's (0.5, 1).
    // The picture is seen from the cart.
    let mut d = Driven::new(json!({"extent": extent(),
    "frames": {
        "cart": {"x": "cart.x"},
        "pole": {"in": "cart", "angle": "pole.theta"}
    },
    "view_from": "cart",
    "layers": [
        {"draw": "discs", "in": "pole", "bind": {"x": 1, "y": 0, "radius": 0.2}, "grab": "hand"},
        {"draw": "boxes", "bind": {"x": -1, "y": -1, "width": 0.4, "height": 0.4}, "grab": "hand"}
    ]}));
    // Drawn from the cart, the disc is at (0, 1) on the plot's axes ...
    let plot = d.view.shown.unwrap();
    let on_plot = plot.value_from_position(d.screen([0.5, 1.0]));
    assert!(close([on_plot.x, on_plot.y], [0.0, 1.0]));
    // ... and pressing on it sends the scene's (0.5, 1), not the plot's.
    let sent = d.press(d.screen([0.5, 1.0]));
    assert!(near(&sent[0], "hand", [0.5, 1.0, 1.0, 0.0]), "{sent:?}");
    let sent = d.frame(vec![egui::Event::PointerMoved(d.screen([0.6, 1.1]))]);
    assert!(near(&sent[0], "hand", [0.6, 1.1, 1.0, 0.0]), "{sent:?}");
    d.release(d.screen([0.6, 1.1]));
    // A box in the scene's own frame, seen from the cart.
    let sent = d.press(d.screen([-1.1, -0.9]));
    assert!(near(&sent[0], "hand", [-1.1, -0.9, 1.0, 0.0]), "{sent:?}");
}

#[test]
fn new_settings_let_go_of_what_is_held() {
    let mut d = Driven::new(json!({"extent": extent(), "layers": [
        {"draw": "discs", "bind": {"x": 0, "y": 0, "radius": 0.5}, "grab": "hand"}
    ]}));
    let sent = d.press(d.screen([0.0, 0.0]));
    assert_eq!(sent[0].1[2], 1.0);
    let (h, o) = (history(), offer());
    d.view
        .set_settings(&settings(json!({})), &available(&o, &h))
        .unwrap();
    let sent = d.view.grabs();
    assert!(near(&sent[0], "hand", [0.0, 0.0, 0.0, -1.0]), "{sent:?}");
}
