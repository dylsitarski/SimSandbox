//! The model card (ADR 0107, ADR 0108's Overview): what a model is, its
//! parts and their roles, each part's equations, method and parameters
//! by role, how the parts connect, what the views show and how far to
//! trust it, all generated from the file so it never goes stale. As JSON
//! for agents and the app, and as Markdown for people.

use crate::session::{ParamRow, Session};
use serde_json::{json, Value};
use simsandbox_core::block::{Causality, ParamRole};
use std::collections::BTreeMap;

/// A part's role in its model, inferred from the graph.
fn role(type_name: &str, stateful: bool, reads_system: bool, drives_system: bool) -> &'static str {
    if type_name.starts_with("input.") {
        "interaction"
    } else if type_name == simsandbox_file::STUDY {
        "study"
    } else if stateful {
        "system"
    } else if reads_system && drives_system {
        "controller"
    } else {
        "formulas"
    }
}

/// The order parts are told in.
const ROLES: [&str; 5] = ["interaction", "system", "controller", "formulas", "study"];

fn block_of(key: &str) -> &str {
    key.split_once('.').map_or(key, |(b, _)| b)
}

fn port_of(key: &str) -> &str {
    key.split_once('.').map_or("", |(_, p)| p)
}

fn param_json(r: &ParamRow) -> Value {
    json!({
        "name": r.name,
        "value": r.value,
        "unit": r.unit,
        "role": r.role.name(),
        "description": r.description,
        "driven_by": r.driven,
        "formula": r.formula,
    })
}

/// The card of a loaded model; a part without a summary of its own has
/// its type's (`registry`'s).
pub fn card(s: &Session, registry: &simsandbox_core::registry::Registry) -> Value {
    let model = s.sim_model();
    let rows = s.params();
    let connections = &s.flat.connections;
    let stateful = |name: &str| {
        s.ids
            .get(name)
            .and_then(|&id| model.block(id).ok())
            .is_some_and(|b| !b.spec().states.is_empty() || !b.spec().tables.is_empty())
    };
    let mut parts = Vec::new();
    for entry in &s.flat.blocks {
        let Some(block) = s.ids.get(&entry.name).and_then(|&id| model.block(id).ok()) else {
            continue;
        };
        let spec = block.spec();
        let method = block.method();
        let incoming: Vec<_> = connections
            .iter()
            .filter(|c| block_of(&c.to) == entry.name)
            .collect();
        let outgoing: Vec<_> = connections
            .iter()
            .filter(|c| block_of(&c.from) == entry.name)
            .collect();
        let reads_system = incoming.iter().any(|c| stateful(block_of(&c.from)));
        let drives_system = outgoing.iter().any(|c| stateful(block_of(&c.to)));
        let fused = s
            .groups
            .iter()
            .find(|g| g.blocks.contains(&entry.name))
            .map(|g| json!({"with": g.blocks.iter().filter(|b| **b != entry.name).collect::<Vec<_>>(), "method": g.method}));
        let mine: Vec<&ParamRow> = rows.iter().filter(|r| r.owner == entry.name).collect();
        let mut by_role = serde_json::Map::new();
        for role in ParamRole::ALL {
            let list: Vec<Value> = mine
                .iter()
                .filter(|r| r.role == role)
                .map(|r| param_json(r))
                .collect();
            if !list.is_empty() {
                by_role.insert(role.name().into(), Value::Array(list));
            }
        }
        let ports = |c: Causality| -> Vec<Value> {
            spec.ports
                .iter()
                .filter(|p| p.causality == c)
                .map(|p| {
                    let key = format!("{}.{}", entry.name, p.name);
                    let wired: Vec<&str> = match c {
                        Causality::Input => connections
                            .iter()
                            .filter(|w| w.to == key)
                            .map(|w| w.from.as_str())
                            .collect(),
                        _ => connections
                            .iter()
                            .filter(|w| w.from == key)
                            .map(|w| w.to.as_str())
                            .collect(),
                    };
                    json!({
                        "name": p.name,
                        "unit": if p.unit == simsandbox_core::units::Unit::DIMENSIONLESS { String::new() } else { p.unit.to_string() },
                        "group": p.group,
                        "description": p.description,
                        if c == Causality::Input { "from" } else { "to" }: wired,
                    })
                })
                .collect()
        };
        let mut part = json!({
            "name": entry.name,
            "type": entry.type_name,
            "role": role(&entry.type_name, stateful(&entry.name), reads_system, drives_system),
            "summary": if spec.summary.is_empty() {
                registry.info(&entry.type_name).map(|i| i.summary.clone()).unwrap_or_default()
            } else {
                spec.summary.clone()
            },
            "method": {
                "name": method.name,
                "speed": format!("{:?}", method.labels.speed),
                "accuracy": format!("{:?}", method.labels.accuracy),
            },
            "fused": fused,
            "params": Value::Object(by_role),
            "inputs": ports(Causality::Input),
            "outputs": ports(Causality::Output),
        });
        // What defines it, as written.
        if !entry.equations.is_empty() {
            let code: Vec<&String> = entry
                .equations
                .iter()
                .skip_while(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
                .collect();
            part["equations"] = json!(code);
        }
        if !entry.bodies.is_empty() {
            part["bodies"] = json!(entry.bodies);
        }
        parts.push(part);
    }
    parts.sort_by_key(|p| {
        ROLES
            .iter()
            .position(|r| p["role"] == *r)
            .unwrap_or(ROLES.len())
    });

    // Wires between the same two parts, in one direction, as one bundle.
    let mut bundles: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for c in connections {
        bundles
            .entry((block_of(&c.from).to_string(), block_of(&c.to).to_string()))
            .or_default()
            .push(format!("{} → {}", port_of(&c.from), port_of(&c.to)));
    }
    let bundles: Vec<Value> = bundles
        .into_iter()
        .map(|((from, to), wires)| json!({"from": from, "to": to, "wires": wires}))
        .collect();

    // Key parameters: what the model pins, else what its parts' types suggest.
    let pinned: Vec<&ParamRow> = rows
        .iter()
        .filter(|r| {
            s.file
                .controls
                .params
                .get(&r.key)
                .is_some_and(|c| c.pinned())
        })
        .collect();
    let (keys, keys_from) = if pinned.is_empty() {
        (
            rows.iter().filter(|r| r.suggested_key).collect::<Vec<_>>(),
            "suggested by the block types",
        )
    } else {
        (pinned, "pinned by the model")
    };
    let keys: Vec<Value> = keys
        .iter()
        .map(|r| {
            let mut v = param_json(r);
            v["key"] = json!(r.key);
            v
        })
        .collect();

    let views: Vec<Value> = s
        .file
        .controls
        .views
        .iter()
        .map(|v| {
            let mut shows: Vec<String> = v.bind.values().flat_map(|b| b.signals()).collect();
            for f in v.scene.frames.values() {
                for x in [&f.x, &f.y, &f.angle].into_iter().flatten() {
                    if let simsandbox_file::SceneValue::Name(n) = x {
                        shows.push(n.clone());
                    }
                }
            }
            for l in &v.scene.layers {
                for x in l.bind.values() {
                    if let simsandbox_file::SceneValue::Name(n) = x {
                        shows.push(n.clone());
                    }
                }
            }
            // Signals only: a scene's colours are names too.
            shows.retain(|n| n == "t" || n.contains('.'));
            shows.sort();
            shows.dedup();
            let mut out = json!({"type": v.type_name, "shows": shows});
            if v.type_name == simsandbox_views::scene::SCENE {
                let grabs: Vec<&String> = v
                    .scene
                    .layers
                    .iter()
                    .filter_map(|l| l.grab.as_ref())
                    .collect();
                out["grabbed_with"] = json!(grabs);
            }
            out
        })
        .collect();

    let labels = s.labels();
    json!({
        "title": s.title,
        "description": s.file.description,
        "trust": {
            "speed": labels.map(|l| format!("{:?}", l.speed)),
            "accuracy": labels.map(|l| format!("{:?}", l.accuracy)),
            // Verified is earned per case (ADR 0061); no case earns it yet.
            "verified": false,
        },
        "run": {"dt": s.dt, "t_end": s.file.run.t_end},
        "key_parameters": {"from": keys_from, "params": keys},
        "parts": parts,
        "connections": bundles,
        "views": views,
    })
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

fn value_text(p: &Value) -> String {
    let unit = p["unit"].as_str().unwrap_or("");
    let v = match (&p["driven_by"], &p["formula"]) {
        (Value::String(d), _) => format!("driven by `{d}`"),
        (_, Value::String(f)) => format!("= {f}"),
        _ => format!("{}", p["value"]),
    };
    if unit.is_empty() || v.starts_with("driven") {
        v
    } else {
        format!("{v} {unit}")
    }
}

/// The card as Markdown, for people.
pub fn markdown(card: &Value) -> String {
    let mut out = String::new();
    let mut line = |s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    line(&format!("# {}", card["title"].as_str().unwrap_or("Model")));
    line("");
    if let Some(d) = card["description"].as_str() {
        line(d);
        line("");
    }
    let t = &card["trust"];
    line(&format!(
        "**Trust:** {} speed, {} accuracy (the least of its methods); no case verified yet.",
        t["speed"].as_str().unwrap_or("unknown"),
        t["accuracy"].as_str().unwrap_or("unknown"),
    ));
    line("");
    let keys = &card["key_parameters"];
    if let Some(list) = keys["params"].as_array().filter(|l| !l.is_empty()) {
        line(&format!(
            "## Key parameters ({})",
            keys["from"].as_str().unwrap_or("")
        ));
        line("");
        line("| Parameter | Value | Role | What it is |");
        line("| --- | --- | --- | --- |");
        for p in list {
            line(&format!(
                "| `{}` | {} | {} | {} |",
                p["key"].as_str().unwrap_or(""),
                md_escape(&value_text(p)),
                p["role"].as_str().unwrap_or(""),
                md_escape(p["description"].as_str().unwrap_or("")),
            ));
        }
        line("");
    }
    line("## Parts");
    line("");
    for part in card["parts"].as_array().into_iter().flatten() {
        line(&format!(
            "### {} — {} (`{}`)",
            part["name"].as_str().unwrap_or(""),
            part["role"].as_str().unwrap_or(""),
            part["type"].as_str().unwrap_or(""),
        ));
        line("");
        if let Some(s) = part["summary"].as_str().filter(|s| !s.is_empty()) {
            line(&s.replace('\n', " "));
            line("");
        }
        let m = &part["method"];
        let fused = part["fused"]["with"]
            .as_array()
            .filter(|w| !w.is_empty())
            .map(|w| {
                let names: Vec<&str> = w.iter().filter_map(Value::as_str).collect();
                format!(
                    "; solved with {} as one system ({})",
                    names.join(", "),
                    part["fused"]["method"].as_str().unwrap_or("")
                )
            })
            .unwrap_or_default();
        line(&format!(
            "Method: {} ({} speed, {} accuracy){fused}.",
            m["name"].as_str().unwrap_or(""),
            m["speed"].as_str().unwrap_or(""),
            m["accuracy"].as_str().unwrap_or(""),
        ));
        line("");
        if let Some(eqs) = part["equations"].as_array() {
            line("```text");
            for e in eqs {
                line(e.as_str().unwrap_or(""));
            }
            line("```");
            line("");
        }
        if let Some(bodies) = part["bodies"].as_array() {
            line("| Body | Joint | Kind | On | Mass |");
            line("| --- | --- | --- | --- | --- |");
            for b in bodies {
                line(&format!(
                    "| {} | {} | {} | {} | {} kg |",
                    b["name"].as_str().unwrap_or(""),
                    b["joint"].as_str().unwrap_or(""),
                    b["kind"].as_str().unwrap_or(""),
                    b["parent"].as_str().unwrap_or(""),
                    b["mass"],
                ));
            }
            line("");
        }
        if let Some(by_role) = part["params"].as_object().filter(|m| !m.is_empty()) {
            // What drives nothing and sits at 0 is counted, not listed.
            let idle = |p: &Value| {
                p["role"] == "drive" && p["driven_by"].is_null() && p["value"].as_f64() == Some(0.0)
            };
            let resting = by_role
                .get("drive")
                .and_then(Value::as_array)
                .map_or(0, |l| l.iter().filter(|p| idle(p)).count());
            line("| Parameter | Value | Role | What it is |");
            line("| --- | --- | --- | --- |");
            for role in ParamRole::ALL {
                for p in by_role
                    .get(role.name())
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|p| !idle(p))
                {
                    line(&format!(
                        "| `{}` | {} | {} | {} |",
                        p["name"].as_str().unwrap_or(""),
                        md_escape(&value_text(p)),
                        role.name(),
                        md_escape(p["description"].as_str().unwrap_or("")),
                    ));
                }
            }
            line("");
            if resting > 0 {
                line(&format!(
                    "And {resting} drive parameters at 0 that nothing drives."
                ));
                line("");
            }
        }
        for (what, key, dir) in [("Inputs", "inputs", "from"), ("Outputs", "outputs", "to")] {
            let list: Vec<String> = part[key]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let unit = p["unit"]
                        .as_str()
                        .filter(|u| !u.is_empty())
                        .map(|u| format!(" [{u}]"))
                        .unwrap_or_default();
                    let desc = p["description"]
                        .as_str()
                        .filter(|d| !d.is_empty())
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default();
                    let wired: Vec<&str> = p[dir]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    let wired = if wired.is_empty() {
                        String::new()
                    } else if dir == "from" {
                        format!(" ← `{}`", wired.join("`, `"))
                    } else {
                        format!(" → `{}`", wired.join("`, `"))
                    };
                    format!("`{}`{unit}{desc}{wired}", p["name"].as_str().unwrap_or(""))
                })
                .collect();
            if !list.is_empty() {
                line(&format!("{what}: {}.", list.join("; ")));
                line("");
            }
        }
    }
    if let Some(bundles) = card["connections"].as_array().filter(|b| !b.is_empty()) {
        line("## How the parts connect");
        line("");
        for b in bundles {
            let wires: Vec<&str> = b["wires"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            line(&format!(
                "- {} → {}: {}",
                b["from"].as_str().unwrap_or(""),
                b["to"].as_str().unwrap_or(""),
                wires.join(", "),
            ));
        }
        line("");
    }
    if let Some(views) = card["views"].as_array().filter(|v| !v.is_empty()) {
        line("## Views");
        line("");
        for v in views {
            let shows: Vec<&str> = v["shows"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let grab = v["grabbed_with"]
                .as_array()
                .filter(|g| !g.is_empty())
                .map(|g| {
                    let names: Vec<&str> = g.iter().filter_map(Value::as_str).collect();
                    format!("; grabbed with `{}`", names.join("`, `"))
                })
                .unwrap_or_default();
            line(&format!(
                "- `{}` showing {}{grab}",
                v["type"].as_str().unwrap_or(""),
                if shows.is_empty() {
                    "nothing bound".to_string()
                } else {
                    format!("`{}`", shows.join("`, `"))
                },
            ));
        }
        line("");
    }
    out
}
