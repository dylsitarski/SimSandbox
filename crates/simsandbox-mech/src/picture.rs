//! The mechanism's default picture (ADR 0114 point 5), `bodies`: built from
//! its rows, as a template (ADR 0113) its block offers. Each body's frame is
//! composed as the tree composes it (`tree::placement`): the joint's frame
//! at its point in the parent, turned by its turn, then the body's moved by
//! the joint's coordinate (turned for a revolute joint, slid along x for a
//! prismatic one). The template computes nothing: the coordinates are the
//! block's ports and the geometry its parameters, bound by name.

use crate::rows::{self, Row};
use crate::tree::Kind;
use serde_json::{json, Map, Value};

/// The picture's name.
pub const BODIES: &str = "bodies";

/// The template drawing these rows' bodies.
pub fn bodies(rows: &[Row]) -> Value {
    let mut inputs = Map::new();
    let mut frames = Map::new();
    let mut layers = Vec::new();
    let mut input = |name: String, unit: &str, description: String| {
        inputs.insert(name, json!({"unit": unit, "description": description}));
    };
    for r in rows {
        let (b, j) = (&r.name, &r.joint);
        let kind = Kind::from(r.kind);
        for (n, u, d) in [
            (
                format!("{j}_at_x"),
                "m",
                "the joint's point, x in the parent's frame",
            ),
            (
                format!("{j}_at_y"),
                "m",
                "the joint's point, y in the parent's frame",
            ),
            (
                format!("{j}_turn"),
                "rad",
                "the joint frame's turn from the parent's",
            ),
            (
                format!("{b}_centre_x"),
                "m",
                "the centre of mass, x in the body's frame",
            ),
            (
                format!("{b}_centre_y"),
                "m",
                "the centre of mass, y in the body's frame",
            ),
        ] {
            input(n, u, d.to_string());
        }
        let joint_frame = format!("{j}_joint");
        let mut jf = json!({"x": format!("${j}_at_x"), "y": format!("${j}_at_y"), "angle": format!("${j}_turn")});
        if r.parent != "ground" {
            jf["in"] = json!(r.parent);
        }
        frames.insert(joint_frame.clone(), jf);
        let body = match rows::words(kind) {
            Some((q, _, _)) => {
                let coordinate = format!("{j}_{q}");
                let (unit, slot) = if kind == Kind::Revolute {
                    ("rad", "angle")
                } else {
                    ("m", "x")
                };
                input(coordinate.clone(), unit, format!("joint {j}'s {q}"));
                json!({"in": joint_frame, slot: format!("${coordinate}")})
            }
            None => json!({"in": joint_frame}),
        };
        frames.insert(b.clone(), body);
        // From the joint to the centre of mass, the centre, the joint.
        layers.push(
            json!({"draw": "segments", "in": b, "grab": "$hand", "bind": {
            "x1": 0, "y1": 0, "x2": format!("${b}_centre_x"), "y2": format!("${b}_centre_y"),
            "width": "$width", "colour": "$colour"}}),
        );
        layers.push(json!({"draw": "discs", "in": b, "grab": "$hand", "bind": {
            "x": format!("${b}_centre_x"), "y": format!("${b}_centre_y"),
            "radius": "$size", "colour": "$colour"}}));
        layers.push(json!({"draw": "points", "in": b, "bind": {
            "x": 0, "y": 0, "size": 7.0, "colour": "grey"}}));
    }
    input(
        "hand".into(),
        "",
        "the pointer a press on a body grabs with: bind an input.pointer block to grab".into(),
    );
    inputs["hand"]["kind"] = json!("pointer");
    let offers: Vec<&String> = rows.iter().map(|r| &r.name).collect();
    json!({
        "type": "plot.scene",
        "unit": "m",
        "description": "Each body as a stick from its joint to its centre of mass, a disc at the centre and a dot at the joint, in its frame composed from the joints",
        "inputs": inputs,
        "params": {
            "colour": {"value": "orange", "description": "the bodies' colour"},
            "width": {"value": 3.0, "description": "the sticks' width, in screen points"},
            "size": {"value": 0.05, "unit": "m", "description": "the centres' radius"}
        },
        "frames": frames,
        "offers": offers,
        "layers": layers,
    })
}
