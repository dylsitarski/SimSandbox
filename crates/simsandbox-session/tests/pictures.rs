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
