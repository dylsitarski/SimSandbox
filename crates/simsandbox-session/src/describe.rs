//! What SimSandbox offers and what a model file holds, as JSON, for people
//! and AI agents working without the window (ADR 0039): the catalogue of
//! block, view and filter types (`simsandbox describe`) and the check of a
//! model file (`simsandbox check`).

use crate::Session;
use serde_json::{json, Value};
use simsandbox_core::block::{Block, Causality, PortSpec, Shape};
use simsandbox_core::registry::{BlockConfig, MethodSettings, Registry};
use simsandbox_file::FileError;
use simsandbox_views::{SlotKind, ViewRegistry, ViewType};

/// A port's shape. Grids are stored row by row (x fastest), points and
/// populations point by point.
pub fn shape(s: &Shape) -> Value {
    match *s {
        Shape::Scalar => json!({"kind": "scalar"}),
        Shape::Vector(n) => json!({"kind": "vector", "len": n}),
        Shape::Grid { nx, ny } => {
            json!({"kind": "grid", "nx": nx, "ny": ny, "order": "row by row, x fastest"})
        }
        Shape::Points { count, dim } => json!({"kind": "points", "count": count, "dim": dim}),
        Shape::Population { dim } => json!({"kind": "population", "dim": dim}),
    }
}

fn port(p: &PortSpec) -> Value {
    let mut v = json!({
        "name": p.name,
        "direction": if p.causality == Causality::Input { "input" } else { "output" },
        "unit": p.unit.to_string(),
        "shape": shape(&p.shape),
    });
    if !p.feedthrough.is_empty() {
        v["depends_on"] = json!(p.feedthrough);
    }
    if let Some(c) = &p.coordinate {
        v["coordinate_of"] = json!({"port": c.port, "axis": c.axis});
    }
    v
}

/// A Block's interface: ports, parameters, states and tables.
pub fn block(b: &dyn Block) -> Value {
    let s = b.spec();
    json!({
        "domain": format!("{:?}", s.domain),
        "ports": s.ports.iter().map(port).collect::<Vec<_>>(),
        "params": s.params.iter().map(|p| json!({
            "name": p.name,
            "unit": p.unit.to_string(),
            "default": p.default,
            "suggested": [p.suggested_min, p.suggested_max],
            "initial_condition": p.initial_condition,
            "description": p.description,
        })).collect::<Vec<_>>(),
        "states": s.states.len(),
        "tables": s.tables.iter().map(|t| json!({"name": t.name, "columns": t.columns})).collect::<Vec<_>>(),
    })
}

/// One block type: what it is, its methods (with their labels), settings
/// and, from an example built with its defaults, its interface.
pub fn block_type(r: &Registry, name: &str) -> Value {
    let Some(info) = r.info(name) else {
        return json!({"type": name, "summary": "(not described)"});
    };
    let methods: Vec<Value> = info
        .methods
        .iter()
        .map(|(m, options)| {
            let config = BlockConfig {
                method: MethodSettings::new(m),
                definition: info.definition.clone().unwrap_or_default(),
                update: None,
            };
            let labels = r
                .build(name, "example", &config)
                .ok()
                .map(|b| b.method().labels);
            json!({
                "name": m,
                "options": options,
                "speed": labels.map(|l| format!("{:?}", l.speed)),
                "accuracy": labels.map(|l| format!("{:?}", l.accuracy)),
            })
        })
        .collect();
    let mut v = json!({
        "type": name,
        "summary": info.summary,
        "methods": methods,
        "settings": info.settings.iter().map(|(n, c)| json!({"name": n, "choices": c})).collect::<Vec<_>>(),
    });
    if let Some(d) = &info.definition {
        v["definition_example"] = json!(d);
        v["note"] =
            json!("Defined by its 'equations' lines: the interface below is the example's.");
    }
    if let Ok(b) = r.example(name) {
        v["interface"] = block(b.as_ref());
    }
    v
}

fn slot_kind(k: SlotKind) -> &'static str {
    match k {
        SlotKind::Scalar => "scalar",
        SlotKind::Grid => "grid",
        SlotKind::Points => "points",
        SlotKind::Line => "line",
        SlotKind::Axis => "axis",
        SlotKind::Coordinate => "coordinate",
    }
}

/// One view type: its slots, style properties and presets.
pub fn view_type(t: &ViewType) -> Value {
    json!({
        "type": t.name,
        "label": t.label,
        "description": t.description,
        "slots": t.slots.iter().map(|s| json!({
            "name": s.name,
            "kind": slot_kind(s.kind),
            "of": s.of,
            "many": s.many,
            "required": s.required,
            "description": s.description,
        })).collect::<Vec<_>>(),
        "style": t.style.iter().map(|p| json!({
            "name": p.name,
            "default": p.default,
            "suggested": p.suggested,
            "unit": p.unit,
            "description": p.description,
        })).collect::<Vec<_>>(),
        "presets": t.presets.iter().map(|p| json!({"label": p.label, "description": p.description})).collect::<Vec<_>>(),
        "applies_to": t.applies_to,
    })
}

/// The filter types (ADR 0030–0032).
pub fn filter_types() -> Value {
    json!([
        {
            "type": "expression",
            "description": "A signal computed from others: 'expr' uses recorded signals, other filters, parameters and t, with the operators and functions of typed equations.",
            "settings": {"expr": "the expression"},
        },
        {
            "type": "cut",
            "description": "Lines cut through a 2D grid (its one input): outputs name[k] and the coordinate name.x, name.y or name.fraction.",
            "settings": {
                "mode": "along_x (default) | along_y | free",
                "at": "along_x/along_y: the fixed values of the other coordinate, one per cut",
                "lines": "free: [[x0, y0], [x1, y1]] per cut",
                "whole": "true (default): sample the whole field; false: only 'range' (free: stop at the end points)",
                "range": "[from, to] when 'whole' is false",
                "interpolation": "bilinear (default) | nearest",
            },
        },
    ])
}

/// Everything a model can be built from.
pub fn catalogue(r: &Registry, views: &ViewRegistry) -> Value {
    json!({
        "block_types": r.type_names().map(|n| block_type(r, n)).collect::<Vec<_>>(),
        "view_types": views.types().iter().map(view_type).collect::<Vec<_>>(),
        "filter_types": filter_types(),
        "file_schema": "simsandbox schema prints the model file's JSON Schema",
    })
}

/// Check a model file: load it as the app does (building the model,
/// evaluating its sweep) and report every problem found. `ok` is false
/// when it cannot run.
pub fn check(text: &str, r: &Registry) -> Value {
    // File errors carry where they are.
    if let Err(e) = simsandbox_file::load(text, r) {
        let error = match &e {
            FileError::Syntax {
                line,
                column,
                message,
            } => json!({"line": line, "column": column, "message": message}),
            FileError::Invalid { path, message } => json!({"path": path, "message": message}),
            FileError::UnsupportedFormat(_) => json!({"path": "format", "message": e.to_string()}),
        };
        return json!({"ok": false, "errors": [error], "warnings": []});
    }
    match Session::load("check", text, r) {
        Err(e) => json!({"ok": false, "errors": [{"message": e}], "warnings": []}),
        Ok(s) => {
            let h = s.signals();
            let labels = s.labels();
            json!({
                "ok": true,
                "errors": [],
                "warnings": s.warnings,
                "labels": labels.map(|l| json!({"speed": format!("{:?}", l.speed), "accuracy": format!("{:?}", l.accuracy)})),
                "swept": s.sweep().map(|sw| sw.keys().to_vec()),
                "fused_groups": s.groups.iter().map(|g| json!({"blocks": g.blocks, "method": g.method.to_string()})).collect::<Vec<_>>(),
                "recorded": {
                    "signals": h.names(),
                    "fields": h.fields().iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                },
                "filters": s.filters().iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                "views": s.file.controls.views.iter().map(|v| v.type_name.clone()).collect::<Vec<_>>(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, view_registry, EXAMPLES};

    #[test]
    fn every_block_type_is_described_with_a_working_example() {
        let r = registry();
        for name in r.type_names() {
            let v = block_type(&r, name);
            assert_ne!(v["summary"], "(not described)", "{name}");
            assert!(v["interface"].is_object(), "{name}: no example");
            for m in v["methods"].as_array().unwrap() {
                // Every listed method builds (formulas need the example to
                // be continuous, so typed equations list theirs anyway).
                if name != simsandbox_eq::registry::EQUATIONS {
                    assert!(m["speed"].is_string(), "{name}: {m}");
                }
            }
        }
        let grass = block_type(&r, "eq.grass");
        let ports = grass["interface"]["ports"].as_array().unwrap();
        assert!(ports
            .iter()
            .any(|p| p["name"] == "x" && p["coordinate_of"]["axis"] == 0));
        let typed = block_type(&r, "eq.equations");
        let e = typed["interface"]["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "E")
            .cloned()
            .unwrap();
        assert!(e.get("depends_on").is_none());
        let sheep = block_type(&r, "agents.grazers");
        assert_eq!(sheep["settings"][0]["name"], "update");
        let all = catalogue(&r, &view_registry());
        assert!(all["view_types"].as_array().unwrap().len() >= 8);
    }

    #[test]
    fn checks_report_where_problems_are_and_what_a_model_records() {
        let r = registry();
        let ok = check(EXAMPLES[0].text, &r);
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["recorded"]["signals"][0], "lorenz.xyz[0]");
        let bad = check(
            r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.nothing", "method": {"name": "rk4"}}]}"#,
            &r,
        );
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["errors"][0]["path"], "blocks[0].type");
        let syntax = check("{\n  \"format\": 1,,\n}", &r);
        assert_eq!(syntax["errors"][0]["line"], 2);
    }
}
