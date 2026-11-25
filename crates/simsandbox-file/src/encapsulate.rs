//! Encapsulated models (ADR 0110): what changes a used model, or feeds a
//! result from it, goes through its interface. Older files reached inside
//! a use of a model defined in the file in three ways, and each converts
//! as the file is read, with results unchanged:
//!
//! - a value the use set for itself by path (`"car.c"`, `"plant/k"`)
//!   becomes a declared parameter of the model, with the value the model
//!   had as its default, read by the inner block (or inner use) by
//!   formula, and set by the use by name;
//! - a wire from an inner output that was not offered (`car1/car.v`)
//!   becomes an output the model offers, which the wire reads
//!   (`car1/v`);
//! - a probe link recording such an output does the same.
//!
//! Each step goes one level down; a path several levels deep is carried
//! down level by level. Uses of separate model files are not rewritten
//! (a file is not changed because another file uses it): they are
//! reported as reaching inside, with the fix. Observers (views, probes
//! that only record, filters) may still read inside by path.

use crate::FileError;
use serde_json::{Map, Value};
use simsandbox_core::registry::Registry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What converting a file gave.
pub struct Encapsulated {
    /// The file in the encapsulated form; the text as given when nothing
    /// changed.
    pub text: String,
    /// One sentence per conversion (ADR 0044).
    pub conversions: Vec<String>,
    /// What still reaches inside and could not be converted here: into
    /// separate model files, or values set by formulas inside.
    pub warnings: Vec<String>,
}

/// Convert `text` to the encapsulated form (ADR 0110). The registry gives
/// a block parameter's default when the model does not write it; `folder`
/// is where blocks read the files they name. Text that is not a model
/// file comes back as it is, for the loader to report.
pub fn encapsulate(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
) -> Result<Encapsulated, FileError> {
    let unchanged = |warnings| Encapsulated {
        text: text.to_string(),
        conversions: Vec::new(),
        warnings,
    };
    let Ok(mut root) = serde_json::from_str::<Value>(text) else {
        return Ok(unchanged(Vec::new()));
    };
    if !root.is_object() {
        return Ok(unchanged(Vec::new()));
    }
    let mut c = Converter {
        registry,
        folder,
        conversions: Vec::new(),
        skipped: BTreeSet::new(),
        warnings: Vec::new(),
        made: BTreeMap::new(),
    };
    // One change at a time, each a level down, until none is left.
    for _ in 0..10_000 {
        let Some(found) = c.next(&root) else { break };
        c.apply(&mut root, found);
    }
    c.report_files(&root);
    if c.conversions.is_empty() {
        return Ok(unchanged(c.warnings));
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| crate::invalid("", e))?;
    Ok(Encapsulated {
        text,
        conversions: c.conversions,
        warnings: c.warnings,
    })
}

/// A path of keys from the file's root to a level: `[]` is the file,
/// `["models", "follower"]` a definition in it.
type LevelPath = Vec<String>;

/// Something reaching inside a use of a model defined in the file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Reach {
    /// `blocks[block].params[key]`, a use's value set by path.
    Value {
        level: LevelPath,
        block: usize,
        key: String,
        model: LevelPath,
    },
    /// `connections[k].from` = `use/rest`.
    Wire {
        level: LevelPath,
        k: usize,
        model: LevelPath,
    },
    /// `outputs[k].from` = `use/rest`.
    Output {
        level: LevelPath,
        k: usize,
        model: LevelPath,
    },
    /// `connections[k].from` = `probe:P.use/rest`.
    Link {
        level: LevelPath,
        k: usize,
        model: LevelPath,
    },
}

struct Converter<'a> {
    registry: &'a Registry,
    folder: Option<&'a Path>,
    conversions: Vec<String>,
    /// What could not be converted, so it is not found again.
    skipped: BTreeSet<Reach>,
    warnings: Vec<String>,
    /// The declared parameter made for a model and a path, so every use
    /// setting that path sets the same one.
    made: BTreeMap<(LevelPath, String), String>,
}

/// Where a level is written, for messages: `models.follower.` or empty.
fn written(level: &[String]) -> String {
    level.iter().map(|k| format!("{k}.")).collect()
}

fn at<'v>(root: &'v Value, path: &[String]) -> Option<&'v Value> {
    path.iter().try_fold(root, |v, k| v.get(k))
}

fn at_mut<'v>(root: &'v mut Value, path: &[String]) -> Option<&'v mut Value> {
    path.iter().try_fold(root, |v, k| v.get_mut(k))
}

fn array<'v>(level: &'v Value, key: &str) -> &'v [Value] {
    level
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn field<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

/// A use names a model and has no type (a study has both).
fn is_use(b: &Value) -> bool {
    b.get("type").is_none() && (b.get("model").is_some() || b.get("file").is_some())
}

/// Every level of the file, with the definitions its uses may name: its
/// own and its enclosing levels' (as `resolve_models` finds them).
fn levels(root: &Value) -> Vec<(LevelPath, BTreeMap<String, LevelPath>)> {
    let mut out = Vec::new();
    let mut todo = vec![(Vec::new(), BTreeMap::new())];
    while let Some((path, outer)) = todo.pop() {
        let Some(level) = at(root, &path) else {
            continue;
        };
        let mut scope: BTreeMap<String, LevelPath> = outer;
        if let Some(models) = level.get("models").and_then(Value::as_object) {
            for name in models.keys() {
                let mut p = path.clone();
                p.extend(["models".to_string(), name.clone()]);
                scope.insert(name.clone(), p);
            }
            for name in models.keys() {
                todo.push((scope[name].clone(), scope.clone()));
            }
        }
        out.push((path, scope));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `use/rest` reaching inside the use (`rest` is more than an output's
/// name): the use and the rest.
fn inside(from: &str) -> Option<(&str, &str)> {
    let (u, rest) = from.split_once('/')?;
    (rest.contains('.') || rest.contains('/')).then_some((u, rest))
}

/// A name not taken: `base`, else `base_2`, `base_3` …
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) && !simsandbox_expr::is_reserved(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|n| !taken(n) && !simsandbox_expr::is_reserved(n))
        .expect("a free name")
}

/// What a model gives a value at `key` (`block.param`, `use/key`, or a
/// declared parameter's name): its value, unit, description and range.
struct Given {
    value: f64,
    unit: String,
    description: String,
    range: Option<[f64; 2]>,
}

impl Converter<'_> {
    /// The first thing reaching inside a use of a model defined in the
    /// file, not skipped.
    fn next(&self, root: &Value) -> Option<Reach> {
        for (path, scope) in levels(root) {
            let level = at(root, &path)?;
            let blocks = array(level, "blocks");
            let model_of = |name: &str| -> Option<LevelPath> {
                let b = blocks
                    .iter()
                    .find(|b| field(b, "name") == Some(name) && is_use(b))?;
                scope.get(field(b, "model")?).cloned()
            };
            for (k, b) in blocks.iter().enumerate() {
                let Some(model) = is_use(b)
                    .then(|| field(b, "model"))
                    .flatten()
                    .and_then(|m| scope.get(m))
                else {
                    continue;
                };
                if let Some(params) = b.get("params").and_then(Value::as_object) {
                    for key in params.keys().filter(|k| k.contains('.') || k.contains('/')) {
                        let r = Reach::Value {
                            level: path.clone(),
                            block: k,
                            key: key.clone(),
                            model: model.clone(),
                        };
                        if !self.skipped.contains(&r) {
                            return Some(r);
                        }
                    }
                }
            }
            for (k, c) in array(level, "connections").iter().enumerate() {
                let Some(from) = field(c, "from") else {
                    continue;
                };
                let (signal, link) = match from.strip_prefix("probe:") {
                    Some(rest) => match rest.split_once('.') {
                        Some((_, s)) => (s, true),
                        None => continue,
                    },
                    None => (from, false),
                };
                let Some((u, _)) = inside(signal) else {
                    continue;
                };
                let Some(model) = model_of(u) else { continue };
                let r = if link {
                    Reach::Link {
                        level: path.clone(),
                        k,
                        model,
                    }
                } else {
                    Reach::Wire {
                        level: path.clone(),
                        k,
                        model,
                    }
                };
                if !self.skipped.contains(&r) {
                    return Some(r);
                }
            }
            for (k, o) in array(level, "outputs").iter().enumerate() {
                let Some((u, _)) = field(o, "from").and_then(inside) else {
                    continue;
                };
                let Some(model) = model_of(u) else { continue };
                let r = Reach::Output {
                    level: path.clone(),
                    k,
                    model,
                };
                if !self.skipped.contains(&r) {
                    return Some(r);
                }
            }
        }
        None
    }

    fn apply(&mut self, root: &mut Value, r: Reach) {
        let done = match &r {
            Reach::Value {
                level,
                block,
                key,
                model,
            } => self.value(root, level, *block, key, model),
            Reach::Wire { level, k, model } => {
                self.wire(root, level, "connections", *k, model, false)
            }
            Reach::Output { level, k, model } => {
                self.wire(root, level, "outputs", *k, model, false)
            }
            Reach::Link { level, k, model } => {
                self.wire(root, level, "connections", *k, model, true)
            }
        };
        if let Err(why) = done {
            self.warnings.push(why);
            self.skipped.insert(r);
        }
    }

    /// A use's value set by path becomes a declared parameter of its model.
    fn value(
        &mut self,
        root: &mut Value,
        level: &[String],
        block: usize,
        key: &str,
        model: &[String],
    ) -> Result<(), String> {
        let place = format!("{}blocks[{block}].params.{key}", written(level));
        let reaches = |why: &str| {
            format!("{place} reaches inside the model it uses, and stays: {why} (ADR 0110)")
        };
        let def = at(root, model).ok_or_else(|| reaches("its model is not found"))?;
        // The inner item and what of it is set: `car.c` or `plant/k`.
        let (inner, rest, by_path) = match key.split_once('/') {
            Some((u, rest)) => (u, rest, true),
            None => {
                let (b, p) = key.split_once('.').expect("a key with '.' or '/'");
                (b, p, false)
            }
        };
        let entry = array(def, "blocks")
            .iter()
            .find(|b| field(b, "name") == Some(inner))
            .ok_or_else(|| reaches(&format!("the model has no block '{inner}'")))?;
        if by_path != is_use(entry) {
            return Err(reaches(&format!(
                "'{inner}' is {} the model",
                if by_path {
                    "not a model used in"
                } else {
                    "a model used in"
                }
            )));
        }
        let made_key = (model.to_vec(), key.to_string());
        let name = match self.made.get(&made_key) {
            Some(n) => n.clone(),
            None => {
                let name = self
                    .declare(root, model, inner, rest, None)
                    .map_err(|e| reaches(&e))?;
                self.made.insert(made_key, name.clone());
                name
            }
        };
        let use_entry = at_mut(root, level)
            .and_then(|l| l.get_mut("blocks"))
            .and_then(Value::as_array_mut)
            .and_then(|bs| bs.get_mut(block))
            .expect("found by next");
        let use_name = field(use_entry, "name").unwrap_or_default().to_string();
        let params = use_entry
            .get_mut("params")
            .and_then(Value::as_object_mut)
            .expect("found by next");
        let value = params.remove(key).expect("found by next");
        params.insert(name.clone(), value);
        self.conversions.push(format!(
            "{place}: {use_name} set {key} inside its model {}, which now declares it as \
             the parameter '{name}' (ADR 0110)",
            model.last().map_or("", String::as_str)
        ));
        Ok(())
    }

    /// Declare `inner`'s `rest` (`block.param`'s param, or a path into an
    /// inner use) a parameter of the model at `model`, named `name` or
    /// after it, with the value the model gives it; the inner item reads it
    /// by formula. Returns its name.
    fn declare(
        &self,
        root: &mut Value,
        model: &[String],
        inner: &str,
        rest: &str,
        name: Option<&str>,
    ) -> Result<String, String> {
        let given = self.given(root, model, inner, rest)?;
        let def = at(root, model).ok_or("its model is not found")?;
        let taken = |n: &str| {
            def.get("params").and_then(|p| p.get(n)).is_some()
                || array(def, "blocks")
                    .iter()
                    .any(|b| field(b, "name") == Some(n))
        };
        let name = match name {
            Some(n) if taken(n) || simsandbox_expr::is_reserved(n) => {
                return Err(format!("the name '{n}' is taken"))
            }
            Some(n) if !crate::subsystems::valid_name(n) => {
                return Err(format!("'{n}' is not a name: letters, digits and '_'"))
            }
            Some(n) => n.to_string(),
            None => unique(rest.rsplit(['.', '/']).next().unwrap_or(rest), taken),
        };
        let mut decl = Map::new();
        decl.insert("value".into(), given.value.into());
        if !given.unit.is_empty() {
            decl.insert("unit".into(), given.unit.into());
        }
        if !given.description.is_empty() {
            decl.insert("description".into(), given.description.into());
        }
        if let Some(r) = given.range {
            decl.insert("range".into(), Value::from(r.to_vec()));
        }
        let obj = at_mut(root, model)
            .and_then(Value::as_object_mut)
            .ok_or("its model is not an object")?;
        obj.entry("params")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or("its model's params are not an object")?
            .insert(name.clone(), Value::Object(decl));
        obj.get_mut("blocks")
            .and_then(Value::as_array_mut)
            .and_then(|bs| bs.iter_mut().find(|b| field(b, "name") == Some(inner)))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("the model has no block '{inner}'"))?
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or("the inner block's params are not an object")?
            .insert(rest.to_string(), Value::String(name.clone()));
        Ok(name)
    }

    /// What the model at `model` gives `inner`'s `rest` before any use
    /// sets it.
    fn given(
        &self,
        root: &Value,
        model: &[String],
        inner: &str,
        rest: &str,
    ) -> Result<Given, String> {
        let def = at(root, model).ok_or("its model is not found")?;
        let entry = array(def, "blocks")
            .iter()
            .find(|b| field(b, "name") == Some(inner))
            .ok_or_else(|| format!("the model has no block '{inner}'"))?;
        if is_use(entry) {
            // A value the inner use sets itself, or its model's.
            match entry.get("params").and_then(|p| p.get(rest)) {
                Some(Value::Number(n)) => {
                    let mut g = self.declared_or_inner(root, model, entry, rest)?;
                    g.value = n.as_f64().unwrap_or(g.value);
                    Ok(g)
                }
                Some(Value::String(f)) => declared(def, f.trim())
                    .ok_or_else(|| format!("'{inner}' sets {rest} by the formula '{f}'")),
                Some(_) => Err(format!("'{inner}' sets {rest} to something not a number")),
                None => self.declared_or_inner(root, model, entry, rest),
            }
        } else {
            let b: crate::BlockEntry = serde_json::from_value(entry.clone())
                .map_err(|e| format!("'{inner}' cannot be read: {e}"))?;
            let built = self
                .registry
                .build(&b.type_name, &b.name, &b.config(self.folder))
                .map_err(|e| format!("'{inner}' cannot be built: {e}"))?;
            let spec = built
                .spec()
                .params
                .iter()
                .find(|p| p.name == rest)
                .ok_or_else(|| format!("'{inner}' has no parameter '{rest}'"))?
                .clone();
            let value = match b.params.get(rest) {
                Some(crate::ParamValue::Number(v)) => *v,
                // Set to a declared parameter by name (converted before):
                // that parameter's.
                Some(crate::ParamValue::Formula(f)) => {
                    return declared(def, f.trim())
                        .ok_or_else(|| format!("'{inner}' sets {rest} by the formula '{f}'"))
                }
                None => spec.default,
            };
            Ok(Given {
                value,
                unit: if spec.unit == simsandbox_core::units::Unit::DIMENSIONLESS {
                    String::new()
                } else {
                    spec.unit.to_string()
                },
                // The stand-in for an undescribed parameter of typed
                // equations counts as none (ADR 0109).
                description: if spec.description == IMPLICIT_PARAMETER {
                    String::new()
                } else {
                    spec.description.clone()
                },
                // A suggested range the value lies outside of suggests
                // nothing.
                range: (spec.suggested_min < spec.suggested_max
                    && (spec.suggested_min..=spec.suggested_max).contains(&value))
                .then_some([spec.suggested_min, spec.suggested_max]),
            })
        }
    }

    /// What the model an inner use names gives `rest`: a declared
    /// parameter, or (by path) a value inside it.
    fn declared_or_inner(
        &self,
        root: &Value,
        model: &[String],
        entry: &Value,
        rest: &str,
    ) -> Result<Given, String> {
        let Some(name) = field(entry, "model") else {
            // A use of a model file: what that file gives it.
            let file = field(entry, "file").ok_or("its model is not found")?;
            let found = match self.folder {
                Some(f) if !Path::new(file).is_absolute() => f.join(file),
                _ => Path::new(file).to_path_buf(),
            };
            let read =
                std::fs::read_to_string(&found).map_err(|e| format!("{}: {e}", found.display()))?;
            let file_root: Value =
                serde_json::from_str(&read).map_err(|e| format!("{}: {e}", found.display()))?;
            if !rest.contains('.') && !rest.contains('/') {
                return declared(&file_root, rest)
                    .ok_or_else(|| format!("{file} declares no parameter '{rest}'"));
            }
            let (inner, deeper) = match rest.split_once('/') {
                Some(p) => p,
                None => rest.split_once('.').expect("checked above"),
            };
            return self.given(&file_root, &[], inner, deeper);
        };
        // The definition named, from the scope of `model`.
        let scope = levels(root)
            .into_iter()
            .find(|(p, _)| p == model)
            .map(|(_, s)| s)
            .unwrap_or_default();
        let inner_model = scope
            .get(name)
            .ok_or_else(|| format!("no model named '{name}'"))?;
        let def = at(root, inner_model).ok_or("its model is not found")?;
        if !rest.contains('.') && !rest.contains('/') {
            return declared(def, rest)
                .ok_or_else(|| format!("'{name}' declares no parameter '{rest}'"));
        }
        let (inner, deeper) = match rest.split_once('/') {
            Some(p) => p,
            None => rest.split_once('.').expect("checked above"),
        };
        self.given(root, inner_model, inner, deeper)
    }

    /// A wire, output or link from inside a use reads an output its model
    /// offers, offered now if it was not.
    fn wire(
        &mut self,
        root: &mut Value,
        level: &[String],
        list: &str,
        k: usize,
        model: &[String],
        link: bool,
    ) -> Result<(), String> {
        let place = format!("{}{list}[{k}].from", written(level));
        let item = at(root, level)
            .and_then(|l| l.get(list))
            .and_then(|l| l.get(k))
            .ok_or("not found")?;
        let from = field(item, "from").unwrap_or_default().to_string();
        let (probe, signal) = match from.strip_prefix("probe:") {
            Some(rest) => {
                let (p, s) = rest.split_once('.').expect("found by next");
                (Some(p.to_string()), s.to_string())
            }
            None => (None, from.clone()),
        };
        let (u, rest) = inside(&signal).expect("found by next");
        let (u, rest) = (u.to_string(), rest.to_string());
        let rest = rest.as_str();
        let name = offer(root, model, rest, None).map_err(|e| format!("{place}: {e}"))?;
        let new_signal = format!("{u}/{name}");
        let level_v = at_mut(root, level).expect("found above");
        match &probe {
            None => {
                level_v[list][k]["from"] = Value::String(new_signal.clone());
            }
            Some(p) => {
                // The link and what else reads the probe's signal follow it.
                let (old, new) = (
                    format!("probe:{p}.{signal}"),
                    format!("probe:{p}.{new_signal}"),
                );
                replace_strings(level_v.get_mut("connections"), &old, &new);
                replace_strings(level_v.get_mut("controls"), &old, &new);
                if let Some(probes) = level_v
                    .get_mut("controls")
                    .and_then(|c| c.get_mut("probes"))
                    .and_then(Value::as_array_mut)
                {
                    for pr in probes.iter_mut().filter(|pr| field(pr, "name") == Some(p)) {
                        if let Some(rec) = pr.get_mut("record").and_then(Value::as_array_mut) {
                            for r in rec.iter_mut().filter(|r| r.as_str() == Some(&signal)) {
                                *r = Value::String(new_signal.clone());
                            }
                        }
                    }
                }
            }
        }
        let what = if link {
            "a link"
        } else if list == "outputs" {
            "an output"
        } else {
            "a wire"
        };
        self.conversions.push(format!(
            "{place}: {what} read {signal} inside {u}; its model {} offers it as '{name}', \
             read as {new_signal} (ADR 0110)",
            model.last().map_or("", String::as_str)
        ));
        Ok(())
    }

    /// What reaches inside uses of separate model files: reported, not
    /// rewritten.
    fn report_files(&mut self, root: &Value) {
        for (path, _) in levels(root) {
            let Some(level) = at(root, &path) else {
                continue;
            };
            let blocks = array(level, "blocks");
            let file_of = |name: &str| {
                blocks
                    .iter()
                    .find(|b| field(b, "name") == Some(name) && is_use(b))
                    .and_then(|b| field(b, "file"))
                    .map(str::to_string)
            };
            let w = written(&path);
            for (k, b) in blocks.iter().enumerate() {
                let Some(file) = is_use(b).then(|| field(b, "file")).flatten() else {
                    continue;
                };
                if let Some(params) = b.get("params").and_then(Value::as_object) {
                    for key in params.keys().filter(|k| k.contains('.') || k.contains('/')) {
                        self.warnings.push(format!(
                            "{w}blocks[{k}].params.{key} reaches inside the model file \
                             {file}: expose it there as a parameter and set that \
                             (ADR 0110)"
                        ));
                    }
                }
            }
            for (list, kind) in [("connections", "a wire or link"), ("outputs", "an output")] {
                for (k, c) in array(level, list).iter().enumerate() {
                    let Some(from) = field(c, "from") else {
                        continue;
                    };
                    let signal = from
                        .strip_prefix("probe:")
                        .and_then(|r| r.split_once('.'))
                        .map_or(from, |(_, s)| s);
                    let Some((u, _)) = inside(signal) else {
                        continue;
                    };
                    if let Some(file) = file_of(u) {
                        self.warnings.push(format!(
                            "{w}{list}[{k}].from: {kind} reads {signal} inside the model \
                             file {file}: offer it there as an output and read that \
                             (ADR 0110)"
                        ));
                    }
                }
            }
        }
    }
}

/// The output the model at `model` offers from `rest` (`block.port`, or
/// a path into an inner use): the one it offers already, else a new one
/// named `name` or after the port. Returns its name.
fn offer(
    root: &mut Value,
    model: &[String],
    rest: &str,
    name: Option<&str>,
) -> Result<String, String> {
    let def = at(root, model).ok_or("its model is not found")?;
    if let Some(o) = array(def, "outputs")
        .iter()
        .find(|o| field(o, "from") == Some(rest))
        .and_then(|o| field(o, "name"))
    {
        return Ok(o.to_string());
    }
    let first = rest.split(['.', '/']).next().unwrap_or(rest);
    if !array(def, "blocks")
        .iter()
        .any(|b| field(b, "name") == Some(first))
    {
        return Err(format!("the model has no block '{first}'"));
    }
    let taken = |n: &str| {
        array(def, "outputs")
            .iter()
            .any(|o| field(o, "name") == Some(n))
            || array(def, "inputs")
                .iter()
                .any(|o| field(o, "name") == Some(n))
            || array(def, "blocks")
                .iter()
                .any(|b| field(b, "name") == Some(n))
    };
    let name = match name {
        Some(n) if taken(n) => return Err(format!("the name '{n}' is taken")),
        Some(n) if !crate::subsystems::valid_name(n) => {
            return Err(format!("'{n}' is not a name: letters, digits and '_'"))
        }
        Some(n) => n.to_string(),
        None => unique(rest.rsplit(['.', '/']).next().unwrap_or(rest), taken),
    };
    let mut out = Map::new();
    out.insert("name".into(), name.clone().into());
    out.insert("from".into(), rest.into());
    at_mut(root, model)
        .and_then(Value::as_object_mut)
        .ok_or("its model is not an object")?
        .entry("outputs")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("its model's outputs are not a list")?
        .push(Value::Object(out));
    Ok(name)
}

/// Where `model` is written: the file itself (`None`), or a model defined
/// in it.
fn model_path(root: &Value, model: Option<&str>) -> Result<LevelPath, String> {
    let Some(m) = model else {
        return Ok(Vec::new());
    };
    let path = vec!["models".to_string(), m.to_string()];
    at(root, &path)
        .map(|_| path)
        .ok_or_else(|| format!("no model named '{m}' is defined in this file"))
}

/// Expose `key` inside `model` (the file itself, or a model defined in it)
/// as a parameter of that model (ADR 0110 point 7): `block.param`, or a
/// path into a model it uses (`plant/k`, `plant/car.m`). It is named
/// `name`, or after the parameter, with the value the model gives it as
/// its default, and what is inside reads it by formula; set it on each
/// use by that name. Returns the file and the name. Already exposed (set
/// by a declared parameter's name), that name is given back.
pub fn expose(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
    model: Option<&str>,
    key: &str,
    name: Option<&str>,
) -> Result<(String, String), String> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let path = model_path(&root, model)?;
    let (inner, rest) = match key.split_once('/') {
        Some(p) => p,
        None => key
            .split_once('.')
            .ok_or_else(|| format!("'{key}' is not block.param or a path (use/param)"))?,
    };
    let def = at(&root, &path).ok_or("its model is not found")?;
    let entry = array(def, "blocks")
        .iter()
        .find(|b| field(b, "name") == Some(inner))
        .ok_or_else(|| format!("the model has no block '{inner}'"))?;
    if key.contains('/') != is_use(entry) {
        return Err(format!(
            "'{inner}' is {}: write {}",
            if is_use(entry) {
                "a used model"
            } else {
                "a block"
            },
            if is_use(entry) {
                "use/param"
            } else {
                "block.param"
            }
        ));
    }
    if let Some(Value::String(f)) = entry.get("params").and_then(|p| p.get(rest)) {
        if declared(def, f.trim()).is_some() {
            return Ok((text.to_string(), f.trim().to_string()));
        }
        return Err(format!("{key} is set by the formula '{f}'"));
    }
    let c = Converter {
        registry,
        folder,
        conversions: Vec::new(),
        skipped: BTreeSet::new(),
        warnings: Vec::new(),
        made: BTreeMap::new(),
    };
    let name = c.declare(&mut root, &path, inner, rest, name)?;
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    Ok((text, name))
}

/// Offer the output `signal` inside `model` (the file itself, or a model
/// defined in it) as an output of that model (ADR 0110 point 4):
/// `block.port`, or a path into a model it uses (`plant/car.v`). It is
/// named `name`, or after the port; what uses the model reads it as
/// `use/name`. Returns the file and the name; offered already, that name.
pub fn offer_output(
    text: &str,
    model: Option<&str>,
    signal: &str,
    name: Option<&str>,
) -> Result<(String, String), String> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let path = model_path(&root, model)?;
    if !signal.contains('.') && !signal.contains('/') {
        return Err(format!("'{signal}' is not block.port or a path (use/out)"));
    }
    let name = offer(&mut root, &path, signal, name)?;
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    Ok((text, name))
}

/// What `simsandbox_eq` describes an undescribed parameter of typed
/// equations with (`IMPLICIT_PARAMETER` there).
const IMPLICIT_PARAMETER: &str = "Parameter of the equations";

/// The model's declared parameter `name`, if it has one.
fn declared(def: &Value, name: &str) -> Option<Given> {
    let d = def.get("params")?.get(name)?;
    Some(Given {
        value: d.get("value").and_then(Value::as_f64)?,
        unit: field(d, "unit").unwrap_or_default().to_string(),
        description: field(d, "description").unwrap_or_default().to_string(),
        range: d.get("range").and_then(|r| {
            let r = r.as_array()?;
            Some([r.first()?.as_f64()?, r.get(1)?.as_f64()?])
        }),
    })
}

/// Replace every string equal to `old` under `v` by `new`.
fn replace_strings(v: Option<&mut Value>, old: &str, new: &str) {
    let Some(v) = v else { return };
    match v {
        Value::String(s) if s == old => *s = new.to_string(),
        Value::Array(a) => a
            .iter_mut()
            .for_each(|x| replace_strings(Some(x), old, new)),
        Value::Object(o) => o
            .values_mut()
            .for_each(|x| replace_strings(Some(x), old, new)),
        _ => {}
    }
}
