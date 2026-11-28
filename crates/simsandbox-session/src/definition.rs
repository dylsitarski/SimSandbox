//! The Definitions projection (ADR 0108 point 1): each block shown where it
//! is defined, with what it is now. Typed equations as an equation sheet
//! (each line, what it declares, its unit and current value); a
//! mechanism as its bodies and the picture it offers; a study as its
//! design; any other block as its parameters. The app's Definitions pane
//! and the headless `explain --definition` read the same JSON.

use crate::session::Session;
use serde_json::{json, Value};

/// What a line of typed equations declares or defines, and its name:
/// `param k`, `state x`, `input u`, `rate x` (`dx/dt = …`), `map x`
/// (`x[n+1] = …`), `defined y` (`y = …`), another statement by its word
/// (`table pts`), or `note` for a comment line.
pub fn declares(line: &str) -> Option<(&'static str, String)> {
    let code = line.split('#').next().unwrap_or("").trim();
    if code.is_empty() {
        return line
            .trim()
            .starts_with('#')
            .then(|| ("note", String::new()));
    }
    let ident = |s: &str| -> String {
        s.trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    };
    for (word, what) in [
        ("param", "param"),
        ("state", "state"),
        ("input", "input"),
        ("grid", "grid"),
        ("table", "table"),
        ("edges", "edges"),
        ("cells", "cells"),
        ("sides", "sides"),
        ("start", "start"),
    ] {
        if let Some(rest) = code.strip_prefix(word).and_then(|r| r.strip_prefix(' ')) {
            if !rest.trim_start().starts_with('=') {
                return Some((what, ident(rest)));
            }
        }
    }
    let (lhs, _) = code.split_once('=')?;
    let lhs = lhs.trim();
    if let Some(x) = lhs.strip_prefix('d').and_then(|r| r.strip_suffix("/dt")) {
        return Some(("rate", ident(x)));
    }
    if let Some(x) = lhs.strip_suffix('\'') {
        return Some(("rate", ident(x)));
    }
    if lhs.contains("[n+1]") {
        return Some(("map", ident(lhs)));
    }
    let name = ident(lhs);
    (!name.is_empty()).then_some(("defined", name))
}

/// The definition of block `block` of the model's own level, with its
/// values now.
pub fn definition(s: &Session, block: &str) -> Result<Value, String> {
    let entry = s
        .file
        .blocks
        .iter()
        .find(|b| b.name == block)
        .ok_or_else(|| format!("no block '{block}'"))?;
    let params = s.params();
    let outputs = s.outputs_of(block);
    let value_of = |name: &str| -> Option<(f64, String)> {
        let key = format!("{block}.{name}");
        params
            .iter()
            .find(|r| r.key == key)
            .map(|r| (r.value, r.unit.clone()))
            .or_else(|| {
                outputs
                    .iter()
                    .find(|(n, _, _)| n == name)
                    .map(|(_, v, u)| (*v, u.clone()))
            })
            // An input: what its wire carries now.
            .or_else(|| {
                let c = s.flat.connections.iter().find(|c| c.to == key)?;
                let (from, port) = c.from.rsplit_once('.')?;
                s.outputs_of(from)
                    .into_iter()
                    .find(|(n, _, _)| n == port)
                    .map(|(_, v, u)| (v, u))
            })
    };
    if entry.is_use() {
        return Ok(json!({
            "block": block,
            "kind": "model",
            "model": entry.model.clone().or_else(|| entry.file.clone()),
        }));
    }
    if entry.type_name == simsandbox_eq::registry::EQUATIONS {
        let lines: Vec<Value> = entry
            .equations
            .iter()
            .enumerate()
            .map(|(k, text)| {
                let mut line = json!({"line": k + 1, "text": text});
                if let Some((what, name)) = declares(text) {
                    line["what"] = json!(what);
                    if !name.is_empty() {
                        line["name"] = json!(name);
                        if let Some((v, unit)) = value_of(&name) {
                            line["value"] = json!(v);
                            line["unit"] = json!(unit);
                        }
                    }
                }
                if let Some((_, note)) = text.split_once('#') {
                    if !text.trim_start().starts_with('#') {
                        line["note"] = json!(note.trim());
                    }
                }
                line
            })
            .collect();
        return Ok(json!({
            "block": block,
            "type": entry.type_name,
            "kind": "equations",
            "lines": lines,
        }));
    }
    if !entry.bodies.is_empty() {
        let pictures: Vec<String> = s
            .templates()
            .keys()
            .filter(|k| {
                k.strip_prefix(block)
                    .is_some_and(|r| r.starts_with('/') && !r[1..].contains('/'))
            })
            .cloned()
            .collect();
        return Ok(json!({
            "block": block,
            "type": entry.type_name,
            "kind": "bodies",
            "bodies": entry.bodies,
            "pictures": pictures,
        }));
    }
    if entry.type_name == simsandbox_file::STUDY {
        return Ok(json!({
            "block": block,
            "type": entry.type_name,
            "kind": "study",
            "model": entry.model.clone().or_else(|| entry.file.clone()),
            "sample": entry.sample,
        }));
    }
    let values: Vec<Value> = params
        .iter()
        .filter(|r| r.owner == block)
        .map(|r| json!({"name": r.name, "value": r.value, "unit": r.unit, "role": r.role.name()}))
        .collect();
    Ok(json!({
        "block": block,
        "type": entry.type_name,
        "kind": "parameters",
        "params": values,
    }))
}

#[cfg(test)]
mod tests {
    use super::declares;

    #[test]
    fn a_lines_declaration_is_read_by_its_first_word_or_its_left_side() {
        assert_eq!(
            declares("param k [1/s] = 2 as tuning # gain"),
            Some(("param", "k".into()))
        );
        assert_eq!(declares("state x [m] = 0"), Some(("state", "x".into())));
        assert_eq!(declares("input u, F"), Some(("input", "u".into())));
        assert_eq!(declares("dx/dt = v"), Some(("rate", "x".into())));
        assert_eq!(declares("v' = -x"), Some(("rate", "v".into())));
        assert_eq!(
            declares("x[n+1] = r*x[n]*(1 - x[n])"),
            Some(("map", "x".into()))
        );
        assert_eq!(
            declares("F = -k*x # the spring"),
            Some(("defined", "F".into()))
        );
        assert_eq!(declares("# what it is"), Some(("note", String::new())));
        assert_eq!(declares("param = 2"), Some(("defined", "param".into())));
        assert_eq!(declares(""), None);
    }
}
