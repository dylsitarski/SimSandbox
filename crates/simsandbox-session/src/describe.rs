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
        Shape::Rows { dim } => {
            json!({"kind": "rows", "dim": dim, "with": "ids and a structure version"})
        }
        Shape::Tensor { rank, dims } => {
            json!({"kind": "axes", "axes": dims[..rank].to_vec(), "order": "first axis fastest"})
        }
    }
}

/// A shape with its axes (ADR 0081): [`shape`], and `axes` as each axis's
/// name (when given) and length (`"rows"` for a table's), the first
/// varying fastest.
pub fn shape_with_axes(s: &Shape, names: &[String]) -> Value {
    let mut v = shape(s);
    let axes: Vec<Value> = s
        .axes()
        .iter()
        .enumerate()
        .map(|(k, a)| {
            let length = match a {
                simsandbox_core::block::AxisLen::Cells(n) => json!(n),
                simsandbox_core::block::AxisLen::Rows => json!("rows"),
            };
            match names.get(k) {
                Some(n) => json!({"name": n, "length": length}),
                None => json!({"length": length}),
            }
        })
        .collect();
    v["axes"] = Value::Array(axes);
    v
}

fn port(p: &PortSpec) -> Value {
    let mut v = json!({
        "name": p.name,
        "direction": if p.causality == Causality::Input { "input" } else { "output" },
        "unit": p.unit.to_string(),
        "shape": shape(&p.shape),
    });
    if !p.axis_names.is_empty() {
        v["axes"] = json!(p.axis_names);
    }
    if !p.feedthrough.is_empty() {
        v["depends_on"] = json!(p.feedthrough);
    }
    if !p.coordinate.is_empty() {
        let of: Vec<Value> = p
            .coordinate
            .iter()
            .map(|c| json!({"port": c.port, "axis": c.axis}))
            .collect();
        v["coordinate_of"] = json!(of);
    }
    // Its group and description (ADR 0109), when said.
    if let Some(g) = &p.group {
        v["group"] = json!(g);
    }
    if !p.description.is_empty() {
        v["description"] = json!(p.description);
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
        // What the block is (ADR 0109): a typed-equations block's leading
        // comment lines.
        "summary": s.summary,
        "ports": s.ports.iter().map(port).collect::<Vec<_>>(),
        "params": s.params.iter().map(|p| json!({
            "name": p.name,
            "unit": p.unit.to_string(),
            "default": p.default,
            "suggested": [p.suggested_min, p.suggested_max],
            "initial_condition": p.initial_condition(),
            "role": p.role.name(),
            "key": p.key,
            "description": p.description,
        })).collect::<Vec<_>>(),
        "states": s.states.len(),
        "tables": s.tables.iter().map(|t| json!({"name": t.name, "columns": t.columns})).collect::<Vec<_>>(),
    })
}

/// A file error as `check` reports it: where it is and what.
pub fn file_error(e: &FileError) -> Value {
    match e {
        FileError::Syntax {
            line,
            column,
            message,
        } => json!({"line": line, "column": column, "message": message}),
        FileError::Invalid { path, message } => json!({"path": path, "message": message}),
        FileError::UnsupportedFormat(_) => json!({"path": "format", "message": e.to_string()}),
    }
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
    if let Some(text) = &info.reference {
        v["reference"] = json!(text);
    }
    if name == simsandbox_eq::registry::EQUATIONS {
        v["functions"] = functions();
    }
    if !info.files.is_empty() {
        v["files"] = json!(info.files);
        v["note"] = json!(
            "Reads the files these settings name, from the model's folder: its outputs come from the file."
        );
    }
    if let Ok(b) = r.example(name) {
        v["interface"] = block(b.as_ref());
    }
    v
}

/// The functions typed equations know, so that an agent has the list
/// without reading the reference: those of numbers with their number of
/// arguments, and those of arrays with how each is called, what it gives
/// and its unit.
fn functions() -> Value {
    json!({
        "of_numbers": simsandbox_expr::Func::ALL
            .iter()
            .map(|(name, f)| json!({"name": name, "arity": f.arity()}))
            .collect::<Vec<_>>(),
        "of_arrays": simsandbox_eq::symbolic::FUNCTIONS
            .iter()
            .map(|f| json!({
                "name": f.name,
                "signature": f.signature,
                "gives": f.gives,
                "unit": f.unit,
            }))
            .collect::<Vec<_>>(),
    })
}

fn slot_kind(k: SlotKind) -> &'static str {
    match k {
        SlotKind::Scalar => "scalar",
        SlotKind::Values => "scalar or line",
        SlotKind::Grid => "grid",
        SlotKind::Points => "points",
        SlotKind::Line => "line",
        SlotKind::Axis => "axis",
        SlotKind::Coordinate => "coordinate",
        SlotKind::Rows => "rows",
        SlotKind::Mesh => "mesh",
    }
}

/// One view type: its slots, style properties and presets.
pub fn view_type(t: &ViewType) -> Value {
    let mut v = json!({
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
    });
    // Beyond slots: a scene's frames, layers and primitives (ADR 0093).
    if let Some(describe) = t.settings {
        v["takes"] = describe();
    }
    v
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
        // A type's reference is long: the catalogue says where it is.
        "block_types": r
            .type_names()
            .map(|n| {
                let mut v = block_type(r, n);
                if v.get("reference").is_some() {
                    v["reference"] = json!(format!("simsandbox describe {n} gives the whole reference"));
                }
                v
            })
            .collect::<Vec<_>>(),
        "view_types": views.types().iter().map(view_type).collect::<Vec<_>>(),
        "filter_types": filter_types(),
        "units": units(),
        "file_schema": "simsandbox schema prints the model file's JSON Schema",
        // What agents owe the people who read their models (ADR 0107).
        "for_agents": "Mark what you make or change with \"agent\": true (the model, blocks, wires; Python's agent_mode does it), describe it (a model's description; a block's notes: purpose, assumptions, reasons for the values you set; `# …` after each typed-equations line, `as ROLE` on parameters), then run `simsandbox check FILE --agent` until it is ok and read `simsandbox explain FILE` as a person will.",
    })
}

/// A model file rewritten in the current form (ADR 0044): the text, the
/// older forms it converted and what it dropped as unusable.
pub fn upgrade(text: &str, r: &Registry) -> Result<(String, Vec<String>, Vec<String>), String> {
    // A model that uses models loads as one (ADR 0090), and keeps the file
    // as written, converted (ADR 0110).
    let loaded = if simsandbox_file::compose::is_composition(text) {
        simsandbox_file::compose::load_merged(text, r, None)
    } else {
        simsandbox_file::load(text, r)
    }
    .map_err(|e| e.to_string())?;
    Ok((loaded.file.to_json(), loaded.conversions, loaded.warnings))
}

/// Check a model file: load it as the app does (building the model,
/// evaluating its sweep) and report every problem found. `ok` is false
/// when it cannot run.
/// [`check`], and with `agent` also what agents must write for the parts
/// they made (ADR 0107 point 3, [`crate::card::agent_problems`]): each is
/// an error at its path, and the model is then not `ok`.
pub fn check_with(text: &str, r: &Registry, agent: bool) -> Value {
    let mut report = check(text, r);
    if agent && report["ok"] == true {
        let written = simsandbox_file::ModelFile::from_json(text).map(|(f, _)| f);
        if let (Ok(s), Ok(written)) = (Session::load("check", text, r), written) {
            let missing = crate::card::agent_problems(&s, &written);
            report["agent"] = json!({"missing": missing.len()});
            if !missing.is_empty() {
                report["ok"] = json!(false);
                report["errors"] = json!(missing);
            }
        }
    }
    report
}

pub fn check(text: &str, r: &Registry) -> Value {
    if simsandbox_file::compose::is_composition(text) {
        return crate::compose::check(text, r);
    }
    // File errors carry where they are; every block's and wire's.
    let errors = simsandbox_file::load_errors(text, r);
    if !errors.is_empty() {
        let errors: Vec<Value> = errors.iter().map(file_error).collect();
        return json!({"ok": false, "errors": errors, "warnings": []});
    }
    match Session::load("check", text, r) {
        Err(e) => json!({"ok": false, "errors": [{"message": e}], "warnings": []}),
        Ok(s) => {
            let labels = s.labels();
            // Views that cannot be shown, as the app and `render` find
            // them: a scene's layers at their paths (ADR 0093).
            let mut warnings = s.warnings.clone();
            {
                let h = s.signals();
                let mut views = crate::views::Views::new(crate::examples::view_registry());
                warnings.extend(views.open(&s.available(&h), &s.file.controls.views));
            }
            json!({
                "ok": true,
                "errors": [],
                "warnings": warnings,
                "conversions": s.conversions,
                "labels": labels.map(self::labels),
                // What it offers to models that use it (ADR 0066 step 4b).
                "declared_outputs": s.file.outputs.iter().map(|o| json!({"name": o.name, "from": o.from})).collect::<Vec<_>>(),
                "fused_groups": s.groups.iter().map(|g| json!({"blocks": g.blocks, "method": g.method.to_string()})).collect::<Vec<_>>(),
                // What views and probes can take (ADR 0063); what the history
                // records follows the views.
                "recorded": {
                    "signals": s.bindable_scalars(),
                    "fields": recorded_fields(&s),
                },
                "params": s.params().iter().map(|p| json!({
                    "key": p.key,
                    "value": p.value,
                    "formula": p.formula,
                    "unit": p.unit,
                    "suggested": p.suggested,
                    "initial_condition": p.initial_condition,
                    "role": p.role.name(),
                    "suggested_key": p.suggested_key,
                    "description": p.description,
                    "driven_by": p.driven,
                })).collect::<Vec<_>>(),
                // Parameters driven by wires (ADR 0075).
                "driven": crate::session::drives(s.sim_model()).into_iter().map(|(to, from)| json!({"param": to, "from": from})).collect::<Vec<_>>(),
                "definitions": crate::compose::definitions(&s.file),
                "blocks": s.flat.blocks.iter().map(|b| &b.name).collect::<Vec<_>>(),
                // How each block made from equations runs (ADR 0084, 0089).
                "code": code(s.sim_model(), &s.ids, |n| n.to_string()),
                "filters": s.filters().iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                "views": s.file.controls.views.iter().map(|v| v.type_name.clone()).collect::<Vec<_>>(),
            })
        }
    }
}

/// How each block whose code SimSandbox makes from equations runs, by
/// name as `full` gives it: `interpreted`, `compiled`, `native`, or
/// `compiled` and why not native ([`simsandbox_core::block::GeneratedCode`]).
pub(crate) fn code(
    model: &simsandbox_core::graph::Model,
    ids: &std::collections::BTreeMap<String, simsandbox_core::graph::BlockId>,
    full: impl Fn(&str) -> String,
) -> serde_json::Map<String, Value> {
    ids.iter()
        .filter_map(|(name, &id)| {
            let code = model.block(id).ok()?.as_generated_code()?.code();
            Some((full(name), Value::String(code)))
        })
        .collect()
}

/// The fields views and probes can take: grids, point sets and rows.
fn recorded_fields(s: &Session) -> Vec<String> {
    let b = s.bindable();
    [b[0], b[1], b[4]].concat()
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
            // A type reading a file has its ports from the file.
            if v["files"].is_array() {
                continue;
            }
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
            .any(|p| p["name"] == "x" && p["coordinate_of"][0]["axis"] == 0));
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
    fn typed_equations_list_every_function() {
        let r = registry();
        let typed = block_type(&r, simsandbox_eq::registry::EQUATIONS);
        let listed = |kind: &str| -> Vec<String> {
            typed["functions"][kind]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["name"].as_str().unwrap().to_string())
                .collect()
        };
        let numbers = listed("of_numbers");
        for (name, _) in simsandbox_expr::Func::ALL {
            assert!(numbers.iter().any(|n| n == name), "{name}");
        }
        assert_eq!(numbers.len(), simsandbox_expr::Func::ALL.len());
        let arrays = listed("of_arrays");
        for name in simsandbox_expr::ARRAY_OPS {
            assert!(arrays.iter().any(|n| n == name), "{name}");
        }
        assert!(arrays.iter().any(|n| n == simsandbox_eq::symbolic::DD_AXIS));
        assert_eq!(arrays.len(), simsandbox_expr::ARRAY_OPS.len() + 1);
        for f in typed["functions"]["of_arrays"].as_array().unwrap() {
            for key in ["signature", "gives", "unit"] {
                assert!(f[key].as_str().is_some_and(|s| !s.is_empty()), "{f}");
            }
        }
        assert_eq!(typed["functions"]["of_numbers"][0]["arity"], 1);
        // Only typed equations have a language of their own.
        assert!(block_type(&r, "eq.grass").get("functions").is_none());
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
        // How blocks made from equations run (ADR 0084, 0089): a large grid
        // compiled (native where a library was built before), a small one
        // interpreted, built-in blocks not listed.
        let text = |name: &str| {
            EXAMPLES
                .iter()
                .find(|e| e.name.starts_with(name))
                .unwrap()
                .text
        };
        let gs = check(text("Gray–Scott"), &r);
        let code = gs["code"]["gs"].as_str().unwrap();
        assert!(code == "compiled" || code == "native", "{code}");
        let rod = check(text("Typed heat equation"), &r);
        assert_eq!(rod["code"]["rod"], "interpreted");
        assert_eq!(ok["code"], json!({}), "Lorenz is built in");
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
