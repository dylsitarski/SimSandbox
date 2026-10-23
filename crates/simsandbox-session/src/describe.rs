//! What SimSandbox offers and what a model file holds, as JSON, for people
//! and AI agents working without the window (ADR 0039): the catalogue of
//! block, view and filter types (`simsandbox describe`) and the check of a
//! model file (`simsandbox check`).

use crate::Session;
use serde_json::{json, Value};
use simsandbox_core::block::{Block, Causality, MethodLabels, PortSpec, Shape};
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

/// Speed and accuracy labels (ADR 0004).
pub fn labels(l: MethodLabels) -> Value {
    json!({"speed": format!("{:?}", l.speed), "accuracy": format!("{:?}", l.accuracy)})
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
                ..Default::default()
            };
            let labels = r
                .build(name, "example", &config)
                .ok()
                .map(|b| b.method().labels);
            json!({
                "name": m,
                "options": options,
                "labels": labels.map(self::labels),
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

/// The units model files accept (ADR 0045, ADR 0047): every symbol with
/// its name, SI dimension, factor to SI and whether SI prefixes apply.
pub fn units() -> Value {
    use simsandbox_core::units::{prefixes, symbols, Unit};
    json!({
        "grammar": "Symbols (with an SI prefix where 'prefixes' is true), positive numbers and \
                    groups in parentheses, joined by '*', '·' or '/' and read left to right, each \
                    with an optional whole power '^n': 'N*s/m', 'kg/(m*s^2)', '(m/s)^2', \
                    '0.3048*m'. 'a/b*c' is refused as ambiguous. Temperatures with an offset (°C, \
                    °F) are not units: use K.",
        "symbols": symbols().map(|s| json!({
            "symbol": s.symbol,
            "name": s.name,
            "si": Unit::new(s.unit.dim.0, 1.0).to_string(),
            "factor": s.unit.scale,
            "prefixes": s.prefixed,
        })).collect::<Vec<_>>(),
        "prefixes": prefixes().iter().map(|(p, f)| json!({"prefix": p, "factor": f})).collect::<Vec<_>>(),
    })
}

/// Everything a model can be built from.
pub fn catalogue(r: &Registry, views: &ViewRegistry) -> Value {
    json!({
        "block_types": r.type_names().map(|n| block_type(r, n)).collect::<Vec<_>>(),
        "view_types": views.types().iter().map(view_type).collect::<Vec<_>>(),
        "filter_types": filter_types(),
        "units": units(),
        "file_schema": "simsandbox schema prints the model file's JSON Schema",
    })
}

/// A model file rewritten in the current form (ADR 0044): the text, the
/// older forms it converted and what it dropped as unusable.
pub fn upgrade(text: &str, r: &Registry) -> Result<(String, Vec<String>, Vec<String>), String> {
    let loaded = simsandbox_file::load(text, r).map_err(|e| e.to_string())?;
    Ok((loaded.file.to_json(), loaded.conversions, loaded.warnings))
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
                "conversions": s.conversions,
                "labels": labels.map(self::labels),
                "swept": s.sweep().map(|sw| sw.keys().to_vec()),
                "fused_groups": s.groups.iter().map(|g| json!({"blocks": g.blocks, "method": g.method.to_string()})).collect::<Vec<_>>(),
                "recorded": {
                    "signals": h.names(),
                    "fields": h.fields().iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                },
                "params": s.params().iter().map(|p| json!({
                    "key": p.key,
                    "value": p.value,
                    "formula": p.formula,
                    "unit": p.unit,
                    "suggested": p.suggested,
                    "initial_condition": p.initial_condition,
                    "description": p.description,
                })).collect::<Vec<_>>(),
                "subsystems": s.file.subsystems.iter().map(|(name, d)| json!({
                    "name": name,
                    "description": d.description,
                    "params": d.params.keys().collect::<Vec<_>>(),
                    "inputs": d.inputs.iter().map(|p| &p.name).collect::<Vec<_>>(),
                    "outputs": d.outputs.iter().map(|p| &p.name).collect::<Vec<_>>(),
                    "used_by": crate::diagram::uses(&s.file, name),
                })).collect::<Vec<_>>(),
                "blocks": s.flat.blocks.iter().map(|b| &b.name).collect::<Vec<_>>(),
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
                    assert!(m["labels"]["speed"].is_string(), "{name}: {m}");
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
        let rho = ok["params"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["key"] == "lorenz.rho")
            .unwrap();
        assert_eq!(rho["value"], 28.0);
        let bad = check(
            r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.nothing", "method": {"name": "rk4"}}]}"#,
            &r,
        );
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["errors"][0]["path"], "blocks[0].type");
        let syntax = check("{\n  \"format\": 1,,\n}", &r);
        assert_eq!(syntax["errors"][0]["line"], 2);
    }

    #[test]
    fn units_are_listed_with_their_factors() {
        let u = units();
        let symbols = u["symbols"].as_array().unwrap();
        let ft = symbols.iter().find(|s| s["symbol"] == "ft").unwrap();
        assert_eq!(
            (ft["si"].as_str(), ft["factor"].as_f64()),
            (Some("m"), Some(0.3048))
        );
        let n = symbols.iter().find(|s| s["symbol"] == "N").unwrap();
        assert_eq!(n["prefixes"], true);
        assert!(u["prefixes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["prefix"] == "k"));
    }
}
