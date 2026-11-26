//! The model card (ADR 0107, ADR 0108's Overview): what a model is, its
//! parts and their roles, each part's equations, method and parameters
//! by role, how the parts connect, what the views show and how far to
//! trust it, all generated from the file so it never goes stale. As JSON
//! for agents and the app, and as Markdown for people.

use crate::session::{ParamRow, Session};
use serde_json::{json, Value};
use simsandbox_core::block::{Causality, ParamRole};
use std::collections::BTreeMap;

/// Each registered type's summary, for [`card`]: a map that can be sent
/// to the thread holding a session.
pub fn type_summaries(
    registry: &simsandbox_core::registry::Registry,
) -> std::collections::HashMap<String, String> {
    registry
        .type_names()
        .filter_map(|n| Some((n.to_string(), registry.info(n)?.summary.clone())))
        .collect()
}

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
const ROLES: [&str; 6] = [
    "interaction",
    "model",
    "system",
    "controller",
    "formulas",
    "study",
];

fn block_of(key: &str) -> &str {
    key.split_once('.').map_or(key, |(b, _)| b)
}

fn param_json(r: &ParamRow) -> Value {
    json!({
        "name": r.name,
        "value": r.value,
        "unit": r.unit,
        "role": r.role.name(),
        // The stand-in for an undescribed parameter counts as none (ADR 0109).
        "description": if r.description == simsandbox_eq::symbolic::IMPLICIT_PARAMETER { "" } else { r.description.as_str() },
        "driven_by": r.driven,
        "formula": r.formula,
    })
}

/// The card of a loaded model, its top level; a part without a summary
/// of its own has its type's (`summary_of`, from a registry's
/// descriptions: see [`type_summaries`]).
pub fn card(s: &Session, summary_of: &dyn Fn(&str) -> Option<String>) -> Value {
    card_at(s, summary_of, "", 1).unwrap_or_else(|e| json!({"title": s.title, "error": e}))
}

/// The model at `part`, a path of uses (`""` for the model itself,
/// `car1`, `vehicle/plant`), as written, with the folder its own uses are
/// found from.
fn level_of(
    s: &Session,
    part: &str,
) -> Result<(simsandbox_file::ModelFile, Option<std::path::PathBuf>), String> {
    let mut file = s.file.clone();
    let mut folder = s.file.folder.clone();
    if part.is_empty() {
        return Ok((file, folder));
    }
    let mut walked = String::new();
    for seg in part.split('/') {
        let entry = file
            .blocks
            .iter()
            .find(|b| b.name == seg && b.is_use())
            .ok_or_else(|| {
                let at = if walked.is_empty() {
                    "the model".to_string()
                } else {
                    format!("'{walked}'")
                };
                let uses: Vec<&str> = file
                    .blocks
                    .iter()
                    .filter(|b| b.is_use())
                    .map(|b| b.name.as_str())
                    .collect();
                format!(
                    "{at} uses no model named '{seg}' (it uses: {})",
                    if uses.is_empty() {
                        "none".to_string()
                    } else {
                        uses.join(", ")
                    }
                )
            })?;
        let (m, _, inner) = simsandbox_file::compose::used_model(entry, folder.as_deref())?;
        file = m;
        folder = inner;
        if !walked.is_empty() {
            walked.push('/');
        }
        walked.push_str(seg);
    }
    Ok((file, folder))
}

/// The written name of what a wire end names: `leader` of `leader.v`,
/// `car1` of `car1/lead`.
fn part_of(end: &str) -> &str {
    let end = end
        .strip_prefix("probe:")
        .map_or(end, |r| r.split_once('.').map_or(r, |(_, signal)| signal));
    end.split(['.', '/']).next().unwrap_or(end)
}

/// The rest of a wire end after its part: `v` of `leader.v`, `lead` of
/// `car1/lead`.
fn end_port(end: &str) -> &str {
    let end = end
        .strip_prefix("probe:")
        .map_or(end, |r| r.split_once('.').map_or(r, |(_, signal)| signal));
    end.find(['.', '/']).map_or("", |k| &end[k + 1..])
}

/// A signal path reaching inside a use here: the use and the rest
/// (`car1`, `car.v` of `car1/car.v`). A use's own port (`car1/v`) does
/// not.
fn reaching<'a>(signal: &'a str, uses: &[&str]) -> Option<(&'a str, &'a str)> {
    let (u, rest) = signal.split_once('/')?;
    (uses.contains(&u) && (rest.contains('.') || rest.contains('/'))).then_some((u, rest))
}

/// The card of the model at `part` (ADR 0110 point 5): its own blocks,
/// and each model it uses as one part with its interface, `depth` levels
/// deep (1: this level only; each use's own card nested as `card` for
/// more).
pub fn card_at(
    s: &Session,
    summary_of: &dyn Fn(&str) -> Option<String>,
    part: &str,
    depth: usize,
) -> Result<Value, String> {
    let (level, folder) = level_of(s, part)?;
    let prefix = if part.is_empty() {
        String::new()
    } else {
        format!("{part}/")
    };
    let local = |n: &str| n.strip_prefix(prefix.as_str()).unwrap_or(n).to_string();
    let model = s.sim_model();
    let rows = s.params();
    let connections = &s.flat.connections;
    let stateful = |name: &str| {
        s.ids
            .get(name)
            .and_then(|&id| model.block(id).ok())
            .is_some_and(|b| !b.spec().states.is_empty() || !b.spec().tables.is_empty())
    };
    let uses: Vec<&str> = level
        .blocks
        .iter()
        .filter(|b| b.is_use())
        .map(|b| b.name.as_str())
        .collect();
    let mut parts = Vec::new();
    for entry in &s.flat.blocks {
        // This level's own blocks: inside it, not inside a use of it.
        let Some(name) = entry.name.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if name.contains('/') || uses.contains(&name) {
            continue;
        }
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
            .map(|g| json!({"with": g.blocks.iter().filter(|b| **b != entry.name).map(|b| local(b)).collect::<Vec<_>>(), "method": g.method}));
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
                    // As written at this level (`car1/lead`, not the
                    // inner port it feeds).
                    let key = format!("{name}.{}", p.name);
                    // The model's own ports, which carry it across its edge.
                    let mut wired: Vec<String> = match c {
                        Causality::Input => level
                            .connections
                            .iter()
                            .filter(|w| w.to == key)
                            .map(|w| w.from.clone())
                            .chain(
                                level
                                    .inputs
                                    .iter()
                                    .filter(|i| i.to.contains(&key))
                                    .map(|i| format!("the model's input {}", i.name)),
                            )
                            .collect(),
                        _ => level
                            .connections
                            .iter()
                            .filter(|w| w.from == key)
                            .map(|w| w.to.clone())
                            .collect(),
                    };
                    if c == Causality::Output {
                        wired.extend(
                            level
                                .outputs
                                .iter()
                                .filter(|o| o.from == key)
                                .map(|o| format!("the model's output {}", o.name)),
                        );
                    }
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
            "name": name,
            // Its full name, for keys (`car1/car`).
            "path": entry.name,
            "kind": "block",
            "type": entry.type_name,
            "role": role(&entry.type_name, stateful(&entry.name), reads_system, drives_system),
            "summary": if spec.summary.is_empty() {
                summary_of(&entry.type_name).unwrap_or_default()
            } else {
                spec.summary.clone()
            },
            // Whether the summary is its type's, not its own.
            "summary_of_type": spec.summary.is_empty(),
            "method": {
                "name": method.name,
                "speed": format!("{:?}", method.labels.speed),
                "accuracy": format!("{:?}", method.labels.accuracy),
            },
            "fused": fused,
            "params": Value::Object(by_role),
            // What it says about itself, and whether an agent made it
            // (ADR 0107).
            "notes": entry.notes,
            "agent": entry.agent,
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
    // Each model used here, as one part: its interface (ADR 0110).
    for entry in level.blocks.iter().filter(|b| b.is_use()) {
        parts.push(use_part(
            s,
            summary_of,
            &level,
            entry,
            &prefix,
            folder.as_deref(),
            depth,
        )?);
    }
    parts.sort_by_key(|p| {
        ROLES
            .iter()
            .position(|r| p["role"] == *r)
            .unwrap_or(ROLES.len())
    });

    // Wires between the same two parts, in one direction, as one bundle,
    // as written at this level.
    let mut bundles: BTreeMap<(String, String), (Vec<String>, bool)> = BTreeMap::new();
    for c in &level.connections {
        let b = bundles
            .entry((part_of(&c.from).to_string(), part_of(&c.to).to_string()))
            .or_default();
        b.0.push(format!("{} → {}", end_port(&c.from), end_port(&c.to)));
        b.1 |= c.agent;
    }
    let bundles: Vec<Value> = bundles
        .into_iter()
        .map(|((from, to), (wires, agent))| json!({"from": from, "to": to, "wires": wires, "agent": agent}))
        .collect();

    // Key parameters: what the model declares (its interface), then what
    // it pins; with neither, what its blocks' types suggest.
    let declared_key = |n: &str| {
        if part.is_empty() {
            n.to_string()
        } else {
            format!("{part}.{n}")
        }
    };
    let here = |r: &&ParamRow| {
        r.owner
            .strip_prefix(prefix.as_str())
            .is_some_and(|o| !o.contains('/'))
    };
    let mut keys: Vec<&ParamRow> = level
        .params
        .keys()
        .filter_map(|n| rows.iter().find(|r| r.key == declared_key(n)))
        .collect();
    let declared = keys.len();
    for r in rows.iter().filter(here) {
        let pinned = s
            .file
            .controls
            .params
            .get(&r.key)
            .is_some_and(|c| c.pinned());
        if pinned && !keys.iter().any(|k| k.key == r.key) {
            keys.push(r);
        }
    }
    let keys_from = match (declared, keys.len() - declared) {
        (0, 0) => {
            keys = rows
                .iter()
                .filter(here)
                .filter(|r| r.suggested_key)
                .collect();
            "suggested by the block types"
        }
        (0, _) => "pinned by the model",
        (_, 0) => "declared by the model",
        _ => "declared and pinned by the model",
    };
    let keys: Vec<Value> = keys
        .iter()
        .map(|r| {
            let mut v = param_json(r);
            v["key"] = json!(r.key);
            v
        })
        .collect();

    // Views and probes are the whole model's, at its top level.
    let mut reaching_inside = Vec::new();
    let templates = simsandbox_file::compose::templates_in(&s.file, s.file.folder.as_deref());
    let views: Vec<Value> = if !part.is_empty() {
        Vec::new()
    } else {
        s.file
            .controls
            .views
            .iter()
            .enumerate()
            .map(|(k, v)| {
                // As it draws, its templates put in place (ADR 0113).
                let shown =
                    simsandbox_file::templates::expand(v, &templates).unwrap_or_else(|_| v.clone());
                let v = &shown;
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
                shows.retain(|n| n == "t" || n.contains('.') || n.contains('/'));
                shows.sort();
                shows.dedup();
                for sig in &shows {
                    if let Some((u, rest)) = reaching(sig, &uses) {
                        reaching_inside.push(reach_json(
                            &format!("controls.views[{k}]"),
                            sig,
                            u,
                            rest,
                            &level,
                        ));
                    }
                }
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
            .collect()
    };
    if part.is_empty() {
        for (k, p) in s.file.controls.probes.iter().enumerate() {
            for sig in &p.record {
                if let Some((u, rest)) = reaching(sig, &uses) {
                    reaching_inside.push(reach_json(
                        &format!("controls.probes[{k}]"),
                        sig,
                        u,
                        rest,
                        &level,
                    ));
                }
            }
        }
    }
    // Experiments may set values inside a used model by path (ADR 0112):
    // the run's schedule, pinned controls and studies' points, marked.
    if part.is_empty() {
        let mut sets = |at: String, key: &str, studied: bool| {
            let Some((u, rest)) = key.split_once('/').filter(|(u, r)| {
                (studied || uses.contains(u)) && (r.contains('.') || r.contains('/'))
            }) else {
                return;
            };
            let entry = level.blocks.iter().find(|b| b.name == u && b.is_use());
            reaching_inside.push(json!({
                "where": at,
                "signal": key,
                "use": u,
                "inside": rest,
                "sets": true,
                "model": entry.and_then(|e| e.model.clone()),
                "file": entry.and_then(|e| e.file.clone()),
                "can_expose": entry.is_some_and(|e| e.model.is_some()),
            }));
        };
        for (k, c) in s.file.run.inputs.iter().enumerate() {
            sets(format!("run.inputs[{k}].set"), &c.set, false);
        }
        for key in s.file.controls.params.keys() {
            sets(format!("controls.params.{key}"), key, false);
        }
        for (k, b) in level.blocks.iter().enumerate() {
            for (j, p) in b.sample.iter().flat_map(|x| x.params.iter()).enumerate() {
                // A path into a model the studied model uses.
                sets(format!("blocks[{k}].sample.params[{j}]"), &p.param, true);
            }
        }
    }
    // A use's start states, by inner block: a state restored, shown as
    // reaching inside (ADR 0110 point 3).
    for (k, b) in level.blocks.iter().enumerate().filter(|(_, b)| b.is_use()) {
        for inner in b.starts.keys() {
            reaching_inside.push(json!({
                "where": format!("blocks[{k}].starts.{inner}"),
                "signal": format!("{}/{inner}", b.name),
                "use": b.name,
                "inside": inner,
                "start": true,
                "can_offer": false,
            }));
        }
    }

    let labels = if part.is_empty() {
        s.labels()
    } else {
        labels_under(s, &prefix)
    };
    let title = if part.is_empty() {
        s.title.clone()
    } else {
        format!("{} > {}", s.title, part.replace('/', " > "))
    };
    Ok(json!({
        "title": title,
        // The path of uses this card is of; empty for the model itself.
        "part": part,
        "description": level.description,
        "notes": level.notes,
        "agent": level.agent,
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
        "reaching_inside": reaching_inside,
        // The pictures it offers to models using it (ADR 0113).
        "templates": crate::describe::templates(&level).0,
    }))
}

/// One reference reaching inside a use, and whether its model can offer
/// it here (a model defined in this file).
fn reach_json(
    at: &str,
    signal: &str,
    u: &str,
    rest: &str,
    level: &simsandbox_file::ModelFile,
) -> Value {
    let entry = level.blocks.iter().find(|b| b.name == u);
    // Offered already: the name to read it by.
    let offered = entry
        .and_then(|e| e.resolved.as_ref())
        .and_then(|m| m.outputs.iter().find(|o| o.from == rest))
        .map(|o| o.name.clone());
    json!({
        "offered_as": offered,
        "where": at,
        "signal": signal,
        "use": u,
        "inside": rest,
        "model": entry.and_then(|e| e.model.clone()),
        "file": entry.and_then(|e| e.file.clone()),
        "can_offer": entry.is_some_and(|e| e.model.is_some()),
    })
}

/// The least labels of the blocks under `prefix`.
fn labels_under(s: &Session, prefix: &str) -> Option<simsandbox_core::block::MethodLabels> {
    let model = s.sim_model();
    simsandbox_core::block::MethodLabels::combine(
        s.ids
            .iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .filter_map(|(_, &id)| model.block(id).ok())
            .map(|b| b.method().labels),
    )
}

/// A model used here, as one part: what it is and its interface for
/// this use, `depth` - 1 levels of its own card inside.
fn use_part(
    s: &Session,
    summary_of: &dyn Fn(&str) -> Option<String>,
    level: &simsandbox_file::ModelFile,
    entry: &simsandbox_file::BlockEntry,
    prefix: &str,
    folder: Option<&std::path::Path>,
    depth: usize,
) -> Result<Value, String> {
    let path = format!("{prefix}{}", entry.name);
    let (used, _, _) = simsandbox_file::compose::used_model(entry, folder)?;
    let rows = s.params();
    let params: Vec<Value> = used
        .params
        .iter()
        .map(|(n, d)| {
            let row = rows.iter().find(|r| r.key == format!("{path}.{n}"));
            json!({
                "name": n,
                "value": row.map_or(d.value, |r| r.value),
                "unit": d.unit,
                "role": "property",
                "description": d.description,
                "driven_by": row.and_then(|r| r.driven.clone()),
                "formula": row.and_then(|r| r.formula.clone()),
            })
        })
        .collect();
    let wired_to = |port: &str| -> Vec<&str> {
        let end = format!("{}/{port}", entry.name);
        level
            .connections
            .iter()
            .filter(|c| c.to == end)
            .map(|c| c.from.as_str())
            .collect()
    };
    let wired_from = |port: &str| -> Vec<&str> {
        let end = format!("{}/{port}", entry.name);
        level
            .connections
            .iter()
            .filter(|c| {
                c.from == end
                    || c.from
                        .strip_prefix("probe:")
                        .and_then(|r| r.split_once('.'))
                        .is_some_and(|(_, sig)| sig == end)
            })
            .map(|c| c.to.as_str())
            .collect()
    };
    let mut inputs: Vec<Value> = used
        .inputs
        .iter()
        .map(|i| json!({"name": i.name, "unit": i.unit, "description": "", "from": wired_to(&i.name)}))
        .collect();
    for b in used
        .blocks
        .iter()
        .filter(|b| b.type_name == simsandbox_eq::input::VALUE)
    {
        inputs.push(json!({
            "name": b.name,
            "unit": b.settings.get("unit").cloned().unwrap_or_default(),
            "description": "an input block: its own value until something drives it",
            "from": wired_to(&b.name),
        }));
    }
    // A pointer the model is grabbed with (ADR 0074, ADR 0113): live input.
    for b in used
        .blocks
        .iter()
        .filter(|b| b.type_name == simsandbox_eq::input::POINTER)
    {
        inputs.push(json!({
            "name": b.name,
            "unit": "",
            "description": "a pointer: a scene that draws its picture grabs with it",
            "from": [],
        }));
    }
    let outputs: Vec<Value> = used
        .outputs
        .iter()
        .map(|o| json!({"name": o.name, "unit": "", "description": format!("from {}", o.from), "to": wired_from(&o.name)}))
        .collect();
    let clock = match &entry.clock {
        Some(simsandbox_file::UseClock::Shared) => json!("shared"),
        Some(simsandbox_file::UseClock::Step(dt)) => json!({"dt": dt}),
        Some(simsandbox_file::UseClock::Times(t)) => json!({"times": t}),
        None => Value::Null,
    };
    let labels = labels_under(s, &format!("{path}/"));
    let summary = entry
        .notes
        .purpose
        .clone()
        .or_else(|| used.notes.purpose.clone())
        .or_else(|| used.description.clone())
        .unwrap_or_default();
    let mut out = json!({
        "name": entry.name,
        "path": path,
        "kind": "use",
        "role": "model",
        "type": entry.model.clone().or_else(|| entry.file.clone()).unwrap_or_default(),
        "model": entry.model,
        "file": entry.file,
        "summary": summary,
        "summary_of_type": false,
        "notes": entry.notes,
        "agent": entry.agent || used.agent,
        "clock": clock,
        "trust": {
            "speed": labels.map(|l| format!("{:?}", l.speed)),
            "accuracy": labels.map(|l| format!("{:?}", l.accuracy)),
        },
        // Its interface for this use (ADR 0074, ADR 0110).
        "params": {"property": params},
        "inputs": inputs,
        "outputs": outputs,
        // What it holds, opened one level down.
        "inside": used.blocks.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        // The pictures its model offers, drawn as `USE/NAME` (ADR 0113).
        "pictures": used.templates.keys().map(|n| format!("{path}/{n}")).collect::<Vec<_>>(),
    });
    if depth > 1 {
        out["card"] = card_at(s, summary_of, &path, depth - 1)?;
    }
    Ok(out)
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

/// Assumptions and reasons as Markdown lines.
fn notes_md(notes: &Value, line: &mut impl FnMut(&str)) {
    let assumptions: Vec<&str> = notes["assumptions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !assumptions.is_empty() {
        line("Assumptions:");
        for a in assumptions {
            line(&format!("- {a}"));
        }
        line("");
    }
    if let Some(reasons) = notes["reasons"].as_object().filter(|r| !r.is_empty()) {
        line("Why these values:");
        for (k, v) in reasons {
            line(&format!("- `{k}`: {}", v.as_str().unwrap_or("")));
        }
        line("");
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
    if card["agent"] == true {
        line("_Made or last changed by an AI agent; not yet reviewed by a person._");
        line("");
    }
    notes_md(&card["notes"], &mut line);
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
        if part["kind"] == "use" {
            use_md(part, &mut line);
            continue;
        }
        line(&format!(
            "### {} — {} (`{}`)",
            part["name"].as_str().unwrap_or(""),
            part["role"].as_str().unwrap_or(""),
            part["type"].as_str().unwrap_or(""),
        ));
        line("");
        if part["agent"] == true {
            line("_Made or last changed by an AI agent; not yet reviewed by a person._");
            line("");
        }
        if let Some(p) = part["notes"]["purpose"].as_str() {
            line(&format!("Purpose: {p}"));
            line("");
        } else if let Some(s) = part["summary"].as_str().filter(|s| !s.is_empty()) {
            line(&s.replace('\n', " "));
            line("");
        }
        let notes = json!({"assumptions": part["notes"]["assumptions"], "reasons": part["notes"]["reasons"]});
        notes_md(&notes, &mut line);
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
                    // Names in code; the model's own ports in words.
                    let wired: Vec<String> = wired
                        .iter()
                        .map(|w| {
                            if w.starts_with("the model's ") {
                                (*w).to_string()
                            } else {
                                format!("`{w}`")
                            }
                        })
                        .collect();
                    let wired = if wired.is_empty() {
                        String::new()
                    } else if dir == "from" {
                        format!(" ← {}", wired.join(", "))
                    } else {
                        format!(" → {}", wired.join(", "))
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
                "- {} → {}: {}{}",
                b["from"].as_str().unwrap_or(""),
                b["to"].as_str().unwrap_or(""),
                wires.join(", "),
                if b["agent"] == true {
                    " _(wired by an agent)_"
                } else {
                    ""
                },
            ));
        }
        line("");
    }
    if let Some(list) = card["reaching_inside"].as_array().filter(|l| !l.is_empty()) {
        line("## Reaching inside");
        line("");
        line("What reads inside a used model by path, not through its interface (ADR 0110): it breaks if that model changes inside.");
        line("");
        for r in list {
            let what = if r["sets"] == true {
                format!(
                    "`{}` sets `{}` inside `{}` (an experiment may; ADR 0112)",
                    r["where"].as_str().unwrap_or(""),
                    r["inside"].as_str().unwrap_or(""),
                    r["use"].as_str().unwrap_or(""),
                )
            } else if r["start"] == true {
                format!(
                    "`{}` starts `{}` from a state of its own",
                    r["use"].as_str().unwrap_or(""),
                    r["inside"].as_str().unwrap_or("")
                )
            } else {
                format!(
                    "`{}` reads `{}` inside `{}`{}",
                    r["where"].as_str().unwrap_or(""),
                    r["inside"].as_str().unwrap_or(""),
                    r["use"].as_str().unwrap_or(""),
                    if let Some(o) = r["offered_as"].as_str() {
                        format!(
                            ": its model offers it as `{o}`, read `{}/{o}`",
                            r["use"].as_str().unwrap_or("")
                        )
                    } else if r["can_offer"] == true {
                        format!(
                            ": its model `{}` can offer it as an output",
                            r["model"].as_str().unwrap_or("")
                        )
                    } else {
                        String::new()
                    }
                )
            };
            line(&format!("- {what}"));
        }
        line("");
    }
    if let Some(templates) = card["templates"].as_object().filter(|t| !t.is_empty()) {
        line("## Templates");
        line("");
        line("Pictures it offers to models that use it, each drawn by binding its inputs (ADR 0113).");
        line("");
        for (name, t) in templates {
            let inputs: Vec<String> = t["inputs"]
                .as_object()
                .into_iter()
                .flatten()
                .map(
                    |(n, i)| match i["unit"].as_str().filter(|u| !u.is_empty()) {
                        Some(u) => format!("`{n}` [{u}]"),
                        None => format!("`{n}`"),
                    },
                )
                .collect();
            let params: Vec<&String> = t["params"]
                .as_object()
                .into_iter()
                .flat_map(|p| p.keys())
                .collect();
            line(&format!(
                "- `{name}` ({}): inputs {}{}{}",
                t["type"].as_str().unwrap_or(""),
                if inputs.is_empty() {
                    "none".to_string()
                } else {
                    inputs.join(", ")
                },
                if params.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; parameters `{}`",
                        params
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join("`, `")
                    )
                },
                t["description"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .map(|d| format!(". {d}"))
                    .unwrap_or_default(),
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

/// A used model as one part, in Markdown: what it is, its interface for
/// this use, and its own card if the card goes deeper.
fn use_md(part: &Value, line: &mut impl FnMut(&str)) {
    let name = part["name"].as_str().unwrap_or("");
    let what = match (part["model"].as_str(), part["file"].as_str()) {
        (Some(m), _) => format!("model `{m}`"),
        (_, Some(f)) => format!("model file `{f}`"),
        _ => "model".to_string(),
    };
    line(&format!("### {name} — {what}"));
    line("");
    if part["agent"] == true {
        line("_Made or last changed by an AI agent; not yet reviewed by a person._");
        line("");
    }
    if let Some(s) = part["summary"].as_str().filter(|s| !s.is_empty()) {
        // Its first sentence: the rest is on its own card.
        let s = s.replace('\n', " ");
        line(s.split_inclusive(". ").next().unwrap_or(&s).trim());
        line("");
    }
    notes_md(&part["notes"], line);
    let t = &part["trust"];
    let clock = match &part["clock"] {
        Value::String(c) if c == "shared" => "; on this model's clock".to_string(),
        Value::Object(o) if o.contains_key("dt") => {
            format!("; on its own clock, every {} s", o["dt"])
        }
        Value::Object(_) => "; on its own clock, at listed times".to_string(),
        _ => String::new(),
    };
    line(&format!(
        "Trust: {} speed, {} accuracy (the least of its methods){clock}.",
        t["speed"].as_str().unwrap_or("unknown"),
        t["accuracy"].as_str().unwrap_or("unknown"),
    ));
    line("");
    let params: Vec<&Value> = part["params"]["property"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if !params.is_empty() {
        line("| Parameter | Value for this use | What it is |");
        line("| --- | --- | --- |");
        for p in params {
            line(&format!(
                "| `{}` | {} | {} |",
                p["name"].as_str().unwrap_or(""),
                md_escape(&value_text(p)),
                md_escape(p["description"].as_str().unwrap_or("")),
            ));
        }
        line("");
    }
    for (what, key, dir, arrow) in [
        ("Inputs", "inputs", "from", "←"),
        ("Outputs", "outputs", "to", "→"),
    ] {
        let list: Vec<String> = part[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let wired: Vec<&str> = p[dir]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let wired = if wired.is_empty() {
                    String::new()
                } else {
                    format!(" {arrow} `{}`", wired.join("`, `"))
                };
                format!("`{}`{wired}", p["name"].as_str().unwrap_or(""))
            })
            .collect();
        if !list.is_empty() {
            line(&format!("{what}: {}.", list.join("; ")));
            line("");
        }
    }
    let pictures: Vec<&str> = part["pictures"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !pictures.is_empty() {
        line(&format!(
            "Pictures it offers: `{}` (a scene draws one as a template layer).",
            pictures.join("`, `")
        ));
        line("");
    }
    let inside: Vec<&str> = part["inside"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    match part["card"].as_object() {
        Some(_) => {
            // Its own card from its key parameters on (what it is was
            // said above), its headings two levels down.
            let text = markdown(&part["card"]);
            for l in text.lines().skip_while(|l| !l.starts_with("## ")) {
                if l.starts_with('#') {
                    line(&format!("##{l}"));
                } else {
                    line(l);
                }
            }
        }
        None if !inside.is_empty() => {
            line(&format!(
                "Inside: {} (its own card: `--part {}`).",
                inside.join(", "),
                part["path"].as_str().unwrap_or(name)
            ));
            line("");
        }
        None => {}
    }
}

/// What `check --agent` (ADR 0107 point 3) finds missing in the parts an
/// agent made (`"agent": true`): a marked model's description; a marked
/// block's purpose (a typed-equations block's leading comment lines will
/// do), a reason for each value set on it, and, for typed equations, a
/// description for each parameter and input it declares. Each with the
/// path to write it at.
pub fn agent_problems(s: &Session, file: &simsandbox_file::ModelFile) -> Vec<Value> {
    let mut out = Vec::new();
    if file.agent
        && file
            .description
            .as_deref()
            .is_none_or(|d| d.trim().is_empty())
    {
        out.push(json!({
            "path": "description",
            "message": "the model was made by an agent: say what it is for in its description",
        }));
    }
    let model = s.sim_model();
    for (k, b) in file.blocks.iter().enumerate().filter(|(_, b)| b.agent) {
        let at = format!("blocks[{k}]");
        let spec = s
            .ids
            .get(&b.name)
            .and_then(|&id| model.block(id).ok())
            .map(|block| block.spec().clone());
        let summary = spec.as_ref().is_some_and(|s| !s.summary.trim().is_empty());
        if b.notes
            .purpose
            .as_deref()
            .is_none_or(|p| p.trim().is_empty())
            && !summary
        {
            out.push(json!({
                "path": format!("{at}.notes.purpose"),
                "message": format!(
                    "'{}' was made by an agent: say what it is for (notes.purpose, or comment lines before a typed-equations block's first statement)",
                    b.name
                ),
            }));
        }
        for name in b
            .params
            .keys()
            .filter(|n| !b.notes.reasons.contains_key(*n))
        {
            out.push(json!({
                "path": format!("{at}.notes.reasons.{name}"),
                "message": format!("'{}.{name}' was set by an agent: say why this value", b.name),
            }));
        }
        if b.type_name == simsandbox_eq::registry::EQUATIONS {
            if let Some(spec) = &spec {
                let params = spec
                    .params
                    .iter()
                    .filter(|p| {
                        !p.initial_condition()
                            && (p.description.trim().is_empty()
                                || p.description == simsandbox_eq::symbolic::IMPLICIT_PARAMETER)
                    })
                    .map(|p| ("parameter", &p.name));
                let inputs = spec
                    .inputs()
                    .filter(|p| p.description.trim().is_empty())
                    .map(|p| ("input", &p.name));
                for (what, name) in params.chain(inputs) {
                    out.push(json!({
                        "path": format!("{at}.equations"),
                        "message": format!(
                            "{what} '{name}' of '{}' has no description: write `# …` after the line declaring it (declare it with `param` or `input` if no line does)",
                            b.name
                        ),
                    }));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn platoon() -> String {
        EXAMPLES
            .iter()
            .find(|e| e.name.starts_with("Platoon"))
            .unwrap()
            .text
            .to_string()
    }

    fn card_of(text: &str, part: &str, depth: usize) -> Result<Value, String> {
        let r = registry();
        let s = Session::load("platoon", text, &r).unwrap();
        card_at(&s, &|t| r.info(t).map(|i| i.summary.clone()), part, depth)
    }

    #[test]
    fn a_used_model_is_one_part_with_its_interface() {
        // ADR 0110 point 5: the platoon is told once, its followers as uses.
        let card = card_of(&platoon(), "", 1).unwrap();
        let names: Vec<&str> = card["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["car1", "car2", "car3", "leader"]);
        let car3 = &card["parts"][2];
        assert_eq!(car3["kind"], "use");
        assert_eq!(car3["params"]["property"][2]["value"], 2500.0);
        assert_eq!(car3["inputs"][0]["from"], json!(["car2/v"]));
        assert!(car3.get("card").is_none());
        // Its interface is its key parameters' source, before its pins.
        assert_eq!(
            card["key_parameters"]["from"],
            "declared and pinned by the model"
        );
        let leader = &card["parts"][3];
        assert_eq!(leader["outputs"][0]["to"], json!(["car1/lead"]));
        // One level further, and nested.
        let inner = card_of(&platoon(), "car3", 1).unwrap();
        assert_eq!(inner["title"], "platoon > car3");
        assert_eq!(inner["parts"][1]["path"], "car3/control");
        let inputs = &inner["parts"][1]["inputs"][0];
        assert_eq!(inputs["from"], json!(["the model's input lead"]));
        let deep = card_of(&platoon(), "", 2).unwrap();
        assert_eq!(deep["parts"][0]["card"]["part"], "car1");
        let missing = card_of(&platoon(), "car4", 1).unwrap_err();
        assert!(missing.contains("car1, car2, car3"), "{missing}");
        // Its Markdown.
        let md = markdown(&deep);
        assert!(md.contains("### car1 — model `follower`"), "{md}");
        assert!(md.contains("#### Parts"), "{md}");
    }

    #[test]
    fn a_components_pictures_are_listed_with_it() {
        // ADR 0113 step 2: the cart-pole offers its template; the model
        // using it twice lists it for each use.
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.starts_with("Two balancing poles"))
            .unwrap();
        let r = registry();
        let s = Session::load_in("two", e.text, &r, e.folder().as_deref()).unwrap();
        let card = card(&s, &|_| None);
        let pictures: Vec<&Value> = card["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| &p["pictures"])
            .collect();
        assert_eq!(
            pictures,
            [
                &json!(["left/body_axes", "left/cartpole"]),
                &json!(["right/body_axes", "right/cartpole"])
            ]
        );
        let shows = &card["views"][0]["shows"];
        assert_eq!(
            shows,
            &json!(["left/theta", "left/x", "right/theta", "right/x"])
        );
        let own = card_of(
            EXAMPLES
                .iter()
                .find(|e| e.name.starts_with("Cart-pole"))
                .unwrap()
                .text,
            "",
            1,
        )
        .unwrap();
        assert!(own["templates"]["cartpole"]["inputs"]["theta"]["unit"] == "rad");
    }

    #[test]
    fn experiments_setting_values_inside_are_marked() {
        // ADR 0112: a schedule and a pinned control may set inside a use by
        // path; the card says so. A use's own parameter does not.
        let mut v: Value = serde_json::from_str(&platoon()).unwrap();
        v["run"]["inputs"] = json!([
            {"t": 5.0, "set": "car2/car.c", "to": 80.0},
            {"t": 6.0, "set": "car2.mass", "to": 1600.0}
        ]);
        v["controls"]["params"]["car1/car.c"] = json!({"slider": true});
        let card = card_of(&v.to_string(), "", 1).unwrap();
        let sets: Vec<&Value> = card["reaching_inside"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["sets"] == true)
            .collect();
        let at: Vec<&str> = sets.iter().map(|r| r["where"].as_str().unwrap()).collect();
        assert_eq!(
            at,
            ["run.inputs[0].set", "controls.params.car1/car.c"],
            "{sets:?}"
        );
        assert_eq!(sets[0]["model"], "follower");
        assert!(markdown(&card).contains("`run.inputs[0].set` sets `car.c` inside `car2`"));
    }

    #[test]
    fn what_reads_inside_is_listed_and_offered_as_an_output() {
        // ADR 0110 point 4: a view reading inside a use, then offered.
        let mut v: Value = serde_json::from_str(&platoon()).unwrap();
        let ys = &mut v["controls"]["views"][0]["bind"]["y"];
        ys[1] = json!("car1/car.v");
        ys[2] = json!("car2/control.e");
        let text = v.to_string();
        let card = card_of(&text, "", 1).unwrap();
        let reach = card["reaching_inside"].as_array().unwrap();
        assert_eq!(reach.len(), 2, "{reach:?}");
        // car.v is offered already as v; control.e is not.
        assert_eq!(reach[0]["offered_as"], "v");
        assert_eq!(reach[1]["signal"], "car2/control.e");
        assert!(reach[1]["offered_as"].is_null() && reach[1]["can_offer"] == true);
        let (mut file, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
        let r = registry();
        let scope = crate::diagram::Scope::Model;
        let note = crate::diagram::apply_in(
            &mut file,
            &scope,
            &r,
            &crate::diagram::Edit::OfferAsOutput {
                signal: "car2/control.e".into(),
            },
        )
        .unwrap();
        assert!(note.contains("'e'"), "{note}");
        let def = file.blocks[1].resolved.as_ref().unwrap();
        assert!(def
            .outputs
            .iter()
            .any(|o| o.name == "e" && o.from == "control.e"));
        let after = file.to_json();
        assert!(after.contains("\"car2/e\"") && !after.contains("car2/control.e"));
        // It loads and reads the same signal by its new name.
        let card = card_of(&after, "", 1).unwrap();
        assert_eq!(card["reaching_inside"].as_array().unwrap().len(), 1);
        assert!(crate::diagram::apply_in(
            &mut file,
            &scope,
            &r,
            &crate::diagram::Edit::OfferAsOutput {
                signal: "leader.v".into()
            },
        )
        .is_err());
    }
}
