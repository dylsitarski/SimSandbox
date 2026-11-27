//! Block types bring default pictures (ADR 0114 step 1): a block's
//! `Pictures` capability offers templates, named `block/name` and bound by
//! name to its ports and parameters, wherever views are drawn and listed.

use serde_json::{json, Value};
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodSpec, ParamSpec, Picture, Pictures, Shape,
    SpeedClass, StepContext, StepIo,
};
use simsandbox_core::registry::{BlockConfig, Registry, TypeInfo};
use simsandbox_core::units::Unit;
use simsandbox_core::Result;
use simsandbox_session::examples::{registry, view_registry};
use simsandbox_session::session::Session;
use simsandbox_session::views::Views;

/// A dot at `x` (its parameter `p`, in m), offering a picture of itself.
struct Dot {
    spec: BlockSpec,
    method: MethodSpec,
}

impl Block for Dot {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }
    fn method(&self) -> &MethodSpec {
        &self.method
    }
    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t + ctx.dt, io)
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs[0] = io.params[0];
        Ok(())
    }
    fn as_pictures(&self) -> Option<&dyn Pictures> {
        Some(self)
    }
}

impl Pictures for Dot {
    fn pictures(&self) -> Vec<Picture> {
        vec![Picture {
            name: "dot".into(),
            description: "the dot as a disc".into(),
            template: json!({
                "type": "plot.scene", "unit": "m",
                "inputs": {"x": {"unit": "m"}},
                "params": {"colour": {"value": "red"}},
                "layers": [{"draw": "discs", "bind": {"x": "$x", "y": 0, "radius": 0.1, "colour": "$colour"}}]
            })
            .to_string(),
        }]
    }
}

fn dot(name: &str, _config: &BlockConfig) -> Result<Box<dyn Block>> {
    let m = Unit::parse("m").unwrap();
    Ok(Box::new(Dot {
        spec: BlockSpec::new(name, Domain::Equations)
            .param(ParamSpec::new("p", m, 0.5, [0.0, 1.0]))
            .output("x", m, Shape::Scalar),
        method: MethodSpec::new("formula", SpeedClass::RealTime, AccuracyClass::Quantitative),
    }))
}

fn with_dot() -> Registry {
    let mut r = registry();
    r.register("test.dot", dot).unwrap();
    r.describe("test.dot", TypeInfo::new("A dot.")).unwrap();
    r
}

fn model(views: Value) -> String {
    json!({
        "format": 1, "run": {"dt": 0.1, "t_end": 0.2},
        "blocks": [{"name": "d", "type": "test.dot"}],
        "controls": {"views": views}
    })
    .to_string()
}

#[test]
fn a_blocks_picture_is_a_template_bound_to_its_ports() {
    let r = with_dot();
    let text = model(json!([{"type": "plot.scene", "unit": "m",
        "layers": [{"draw": "template", "name": "pic", "template": "d/dot"}]}]));
    let s = Session::load("m", &text, &r).unwrap();
    let t = s.templates();
    assert!(t.contains_key("d/dot"), "{:?}", t.keys());
    // Expanded, its input binds the block's port `d.x`.
    let shown = simsandbox_file::templates::expand(&s.file.controls.views[0], &t).unwrap();
    assert_eq!(
        shown.scene.layers[0].bind["x"],
        simsandbox_file::SceneValue::Name("d.x".into())
    );
    // Drawn and recorded as any view.
    let mut views = Views::new(view_registry());
    views.templates = t;
    let w = views.open(&s.available(&s.signals()), &s.file.controls.views);
    assert!(w.is_empty(), "{w:?}");
    // Listed by `check`, by the card with its block, and by `describe`.
    let report = simsandbox_session::describe::check(&text, &r);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["warnings"], json!([]));
    assert!(
        report["pictures"]["d/dot"]["inputs"]["x"].is_object(),
        "{report}"
    );
    let card = simsandbox_session::card::card(&s, &|_| None);
    assert_eq!(card["parts"][0]["pictures"], json!(["d/dot"]));
    assert!(simsandbox_session::card::markdown(&card).contains("Pictures it offers: `d/dot`"));
    let described = simsandbox_session::describe::block_type(&r, "test.dot");
    assert_eq!(described["pictures"][0]["name"], "dot");
}

#[test]
fn a_block_in_a_used_model_offers_its_picture_under_the_path() {
    let r = with_dot();
    let text = json!({
        "format": 1, "run": {"dt": 0.1, "t_end": 0.2},
        "models": {"inner": {"blocks": [{"name": "d", "type": "test.dot"}]}},
        "blocks": [{"name": "u", "model": "inner"}],
        "controls": {"views": [{"type": "plot.scene", "unit": "m",
            "layers": [{"draw": "template", "name": "pic", "template": "u/d/dot"}]}]}
    })
    .to_string();
    let s = Session::load("m", &text, &r).unwrap();
    let t = s.templates();
    let shown = simsandbox_file::templates::expand(&s.file.controls.views[0], &t).unwrap();
    assert_eq!(
        shown.scene.layers[0].bind["x"],
        simsandbox_file::SceneValue::Name("u/d.x".into())
    );
    let report = simsandbox_session::describe::check(&text, &r);
    assert_eq!(report["warnings"], json!([]), "{report}");
}

const CARTPOLE: &str = include_str!("../../../examples/cartpole.ssbx.json");

/// The cart-pole's text with its views replaced.
fn cartpole_with(views: Value) -> String {
    let mut m: Value = serde_json::from_str(CARTPOLE).unwrap();
    m["controls"]["views"] = views;
    if views_empty(&m) {
        m["controls"].as_object_mut().unwrap().remove("views");
        m["controls"].as_object_mut().unwrap().remove("layout");
    }
    m.to_string()
}

fn views_empty(m: &Value) -> bool {
    m["controls"]["views"].as_array().is_some_and(Vec::is_empty)
}

#[test]
fn the_mechanisms_bodies_are_drawn_where_its_rows_put_them() {
    // ADR 0114 step 2: `cartpole/bodies` composes each body's frame from
    // the joints, bound by name to the block's coordinates and geometry;
    // a point at (1, 0) in each frame it offers lands where the same point
    // of the mechanism's own pose rows does.
    use simsandbox_views::scene::{geometry::Instance, Scene};
    use simsandbox_views::Recorded;
    let r = registry();
    let text = cartpole_with(json!([{"type": "plot.scene", "unit": "m",
    "frames": {"rows": {"x": "cartpole.x", "y": "cartpole.y", "angle": "cartpole.angle"}},
    "layers": [
        {"draw": "template", "name": "b", "template": "cartpole/bodies"},
        {"draw": "points", "in": "b/cart", "bind": {"x": 1, "y": 0}},
        {"draw": "points", "in": "b/pole", "bind": {"x": 1, "y": 0}},
        {"draw": "points", "in": "rows", "bind": {"x": 1, "y": 0}}
    ]}]));
    let mut s = Session::load("cartpole", &text, &r).unwrap();
    let report = simsandbox_session::describe::check(&text, &r);
    assert_eq!(report["warnings"], json!([]), "{report}");
    assert!(report["pictures"]["cartpole/bodies"]["offers"] == json!(["cart", "pole"]));
    let shown =
        simsandbox_file::templates::expand(&s.file.controls.views[0], &s.templates()).unwrap();
    let settings = shown.scene.to_map();
    // A grab of the pole: the picture follows the pole as it swings.
    for (t, down) in [(0.2, 1.0), (0.6, 0.0)] {
        s.set_input("hand.x", 0.5).unwrap();
        s.set_input("hand.y", 0.8).unwrap();
        s.set_input("hand.picked", 0.0).unwrap();
        s.set_input("hand.down", down).unwrap();
        s.run_until(t).unwrap();
    }
    let mut worst: f64 = 0.0;
    for k in 1..=10 {
        s.run_until(0.6 + 0.1 * f64::from(k)).unwrap();
        let h = s.signals();
        let scene = Scene::check(&settings, &s.available(&h)).unwrap();
        let placed = scene.place(&Recorded::single(&h));
        let point = |frame: &str, id: Option<u64>| -> [f64; 2] {
            // The scene's own point, after the picture's layers.
            let p = placed
                .iter()
                .rev()
                .find(|p| {
                    p.id == id
                        && scene.layers[p.layer]
                            .frame
                            .map(|f| scene.frames[f].name.as_str())
                            == Some(frame)
                        && matches!(p.shape, Instance::Point { .. })
                        && scene.layers[p.layer].attributes.len() == 4
                })
                .unwrap_or_else(|| panic!("{frame} {id:?}"));
            match p.shape {
                Instance::Point { at, .. } => at,
                _ => unreachable!(),
            }
        };
        for (frame, id) in [("b/cart", 0), ("b/pole", 1)] {
            let (a, b) = (point(frame, None), point("rows", Some(id)));
            worst = worst.max((a[0] - b[0]).abs().max((a[1] - b[1]).abs()));
        }
    }
    assert!(worst < 1e-9, "the picture and the rows differ by {worst}");
    let described = simsandbox_session::describe::block_type(&r, "mech.planar");
    assert_eq!(described["pictures"][0]["name"], "bodies");
}

#[test]
fn a_model_writing_no_views_opens_with_its_blocks_pictures() {
    // ADR 0114 point 4: one scene with every picture its blocks offer,
    // besides the plot chosen by fit; saved only if saved with it.
    let r = registry();
    let text = cartpole_with(json!([]));
    let mut s = Session::load("cartpole", &text, &r).unwrap();
    assert!(s.file.controls.views.is_empty());
    // What it binds is recorded without a window (render, run, Python):
    // the pole's disc is drawn at its centre, above the cart.
    s.run_until(0.1).unwrap();
    let h = s.signals();
    let pictures = simsandbox_session::views::default_pictures(&s.templates());
    let shown = simsandbox_file::templates::expand(&pictures[0], &s.templates()).unwrap();
    let scene =
        simsandbox_views::scene::Scene::check(&shown.scene.to_map(), &s.available(&h)).unwrap();
    let discs: Vec<[f64; 2]> = scene
        .place(&simsandbox_views::Recorded::single(&h))
        .iter()
        .filter_map(|p| match p.shape {
            simsandbox_views::scene::geometry::Instance::Disc { centre, .. } => Some(centre),
            _ => None,
        })
        .collect();
    assert!(discs.iter().any(|c| c[1] > 0.1), "{discs:?}");
    let mut views = Views::new(view_registry());
    views.templates = s.templates();
    let w = views.open(&s.available(&s.signals()), &s.file.controls.views);
    assert!(w.is_empty(), "{w:?}");
    let entries = views.entries();
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[0].type_name, "plot.scene");
    assert_eq!(
        entries[0].scene.layers[0].other["template"],
        "cartpole/bodies"
    );
    assert!(
        views.tiles[0].state.problem().is_none(),
        "{:?}",
        views.tiles[0].state.problem()
    );
    assert_eq!(entries[1].type_name, "plot.xy");
}
