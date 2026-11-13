//! Editing a used model from the model using it (ADR 0066 step 4c,
//! ADR 0068): the answers to the edit question, and where edits then go.
//!
//! Model files edited and not yet saved are kept by path (`edited`) and
//! read in place of the disk on this thread
//! ([`simsandbox_file::compose::set_edited`]), so the run, `check` and
//! the diagram see them before Save writes them. A local copy is a model
//! defined in the file open (`models`), used by that use only.

use simsandbox_file::compose::{as_definition, read_definition, set_edited, used_model};
use simsandbox_file::ModelFile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Files edited and not yet saved: full path → text.
pub type Edited = BTreeMap<PathBuf, String>;

/// The answer to the edit question.
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// Edit what the use reads: the used file, or the definition it names
    /// (every use of either sees the change).
    Original,
    /// Write its contents to this new file (full path), point this use at
    /// it, and edit the copy.
    NewCopy(PathBuf),
    /// Copy it into a definition of the model open, used here only.
    LocalCopy,
}

/// Where edits inside a used model go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The file holding the model edited; `None` for the model open.
    pub file: Option<PathBuf>,
    /// The definition edited, by its keys through nested `models` from
    /// that file (`["vehicle", "plant"]`); empty for the file itself.
    pub def: Vec<String>,
}

/// Models along a path of uses, each with the folder its files are
/// found from.
type Chain = Vec<(ModelFile, Option<PathBuf>)>;

/// The models along `path` (names of uses from `top`), each with the
/// folder its own files are found from, and the target an edit of the
/// original would have.
fn walk(
    top: &ModelFile,
    folder: Option<&Path>,
    path: &[String],
) -> Result<(Chain, Target), String> {
    let mut at = (top.clone(), folder.map(Path::to_path_buf));
    let mut chain = Vec::new();
    let mut target = Target {
        file: None,
        def: Vec::new(),
    };
    // Where definitions are found, innermost last: keys from the file and
    // the definitions there.
    let mut scopes: Vec<(Vec<String>, Vec<String>)> =
        vec![(Vec::new(), top.models.keys().cloned().collect())];
    for name in path {
        let entry =
            at.0.blocks
                .iter()
                .find(|b| &b.name == name && b.names_model())
                .ok_or_else(|| format!("no model used as '{name}'"))?;
        let (model, file, inner) = used_model(entry, at.1.as_deref())?;
        match (file, &entry.model) {
            (Some(f), _) => {
                target = Target {
                    file: Some(f),
                    def: Vec::new(),
                };
                scopes = vec![(Vec::new(), model.models.keys().cloned().collect())];
            }
            (None, Some(m)) => {
                let (keys, _) = scopes
                    .iter()
                    .rev()
                    .find(|(_, names)| names.contains(m))
                    .ok_or_else(|| format!("no model named '{m}' is defined here"))?;
                let mut keys = keys.clone();
                keys.push(m.clone());
                target.def.clone_from(&keys);
                scopes.push((keys, model.models.keys().cloned().collect()));
            }
            (None, None) => return Err(format!("'{name}' uses no model")),
        }
        at = (model, inner);
        chain.push(at.clone());
    }
    Ok((chain, target))
}

/// The model the use at `path` reads (edited files included).
pub fn model_at(
    top: &ModelFile,
    folder: Option<&Path>,
    path: &[String],
) -> Result<ModelFile, String> {
    let (chain, _) = walk(top, folder, path)?;
    chain
        .last()
        .map(|(m, _)| m.clone())
        .ok_or_else(|| "no model used".into())
}

/// How many uses in `top` (at any depth of its definitions) name the
/// definition `target` edits, when it is one of the model open.
pub fn uses_of(top: &ModelFile, target: &Target) -> usize {
    if target.file.is_some() || target.def.is_empty() {
        return 0;
    }
    let name = target.def.last().expect("not empty");
    fn count(m: &ModelFile, name: &str) -> usize {
        m.blocks
            .iter()
            .map(|b| {
                usize::from(b.model.as_deref() == Some(name))
                    + b.resolved.as_deref().map_or(0, |d| count(d, name))
            })
            .sum()
    }
    count(top, name)
}

/// `p` relative to `to` if it is inside it, else as it is.
fn relative(p: &Path, to: Option<&Path>) -> String {
    to.and_then(|t| p.strip_prefix(t).ok())
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Make the files `model` names (uses, replacements, data files, those of
/// its definitions), written from `from`, found the same from `to`.
pub fn rebase(model: &mut ModelFile, from: Option<&Path>, to: Option<&Path>) {
    if from == to {
        return;
    }
    let moved = |name: &str| -> String {
        let p = PathBuf::from(name);
        match from {
            Some(f) if p.is_relative() => relative(&f.join(p), to),
            _ => name.to_string(),
        }
    };
    let blocks = model.blocks.iter_mut().chain(
        model
            .subsystems
            .values_mut()
            .flat_map(|s| s.blocks.iter_mut()),
    );
    for b in blocks {
        if let Some(f) = &b.file {
            b.file = Some(moved(f));
        }
        for f in b.files.values_mut() {
            *f = moved(f);
        }
        if b.type_name == simsandbox_eq::file_input::FILE {
            if let Some(f) = b.settings.get_mut("file") {
                *f = moved(f);
            }
        }
        if let Some(d) = &mut b.resolved {
            rebase(d, from, to);
        }
    }
    for (name, v) in model.models.iter_mut() {
        if let Ok(mut d) = read_definition(name, v) {
            rebase(&mut d, from, to);
            *v = as_definition(&d);
        }
    }
}

/// Make `model` hold every definition its uses name, so it reads the same
/// wherever it is put: those found outside it are copied in.
fn self_contained(model: &mut ModelFile) {
    for b in &model.blocks {
        if let (Some(name), Some(d)) = (&b.model, &b.resolved) {
            if !model.models.contains_key(name) {
                let mut d = (**d).clone();
                self_contained(&mut d);
                model.models.insert(name.clone(), as_definition(&d));
            }
        }
    }
}

/// A key for a new definition in `models`: `name`, or `name_2`, ….
fn free_key(models: &BTreeMap<String, serde_json::Value>, name: &str) -> String {
    let mut key = name.to_string();
    let mut k = 2;
    while models.contains_key(&key) {
        key = format!("{name}_{k}");
        k += 1;
    }
    key
}

/// Prepare editing the model used at `path` in `top` (saved in `folder`)
/// as `how` says: `top` and `edited` change as the answer needs. Returns
/// where its edits go.
pub fn prepare(
    top: &mut ModelFile,
    folder: Option<&Path>,
    edited: &mut Edited,
    path: &[String],
    how: How,
) -> Result<Target, String> {
    set_edited(edited.clone());
    let (chain, original) = walk(top, folder, path)?;
    match how {
        How::Original => Ok(original),
        How::NewCopy(to) => {
            if to.exists() || edited.contains_key(&to) {
                return Err(format!("{} exists: choose another name", to.display()));
            }
            let (model, from) = chain.last().cloned().expect("a path has a model");
            let mut copy = model;
            self_contained(&mut copy);
            rebase(&mut copy, from.as_deref(), to.parent());
            copy.folder = to.parent().map(Path::to_path_buf);
            edited.insert(to.clone(), copy.to_json());
            let named = relative(&to, folder);
            let first = top
                .blocks
                .iter_mut()
                .find(|b| b.name == path[0] && b.names_model())
                .expect("walked");
            if path.len() == 1 {
                // Its replacements are in the copy now.
                first.file = Some(named);
                first.model = None;
                first.files.clear();
            } else {
                first.files.insert(path[1..].join("/"), named);
            }
            top.resolve_models().map_err(|e| e.to_string())?;
            set_edited(edited.clone());
            Ok(Target {
                file: Some(to),
                def: Vec::new(),
            })
        }
        How::LocalCopy => {
            // Each model along the path, its files found from here, held
            // as a definition of the one above it, the first in `top`.
            let mut models: Vec<ModelFile> = chain
                .into_iter()
                .map(|(mut m, from)| {
                    self_contained(&mut m);
                    rebase(&mut m, from.as_deref(), folder);
                    m
                })
                .collect();
            let mut keys: Vec<String> = Vec::new();
            let mut held: Option<(String, ModelFile)> = None;
            while let Some(mut m) = models.pop() {
                let k = models.len();
                if let Some((key, inner)) = held.take() {
                    let entry = m
                        .blocks
                        .iter_mut()
                        .find(|b| b.name == path[k + 1])
                        .expect("walked");
                    use_definition(entry, &key);
                    m.models.insert(key.clone(), as_definition(&inner));
                    keys.insert(0, key);
                }
                // Free among the definitions of the model holding it.
                let key = free_key(
                    if k == 0 {
                        &top.models
                    } else {
                        &models[k - 1].models
                    },
                    &path[k],
                );
                held = Some((key, m));
            }
            let (key, m) = held.expect("a path has a model");
            let first = top
                .blocks
                .iter_mut()
                .find(|b| b.name == path[0] && b.names_model())
                .expect("walked");
            use_definition(first, &key);
            top.models.insert(key.clone(), as_definition(&m));
            keys.insert(0, key);
            top.resolve_models().map_err(|e| e.to_string())?;
            Ok(Target {
                file: None,
                def: keys,
            })
        }
    }
}

/// `entry` uses the definition `key` (its replacements are in it).
fn use_definition(entry: &mut simsandbox_file::BlockEntry, key: &str) {
    entry.file = None;
    entry.files.clear();
    entry.model = Some(key.to_string());
    entry.resolved = None;
}

/// The file `file` names: as edited, else from disk; the model open for
/// `None`.
fn container(
    top: &ModelFile,
    edited: &Edited,
    file: &Option<PathBuf>,
) -> Result<ModelFile, String> {
    match file {
        None => Ok(top.clone()),
        Some(p) => {
            let text = match edited.get(p) {
                Some(t) => t.clone(),
                None => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
            };
            let (mut m, _) =
                ModelFile::from_json(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            m.folder = p.parent().map(Path::to_path_buf);
            Ok(m)
        }
    }
}

/// The model edits to `target` apply to, as written (not with the
/// replacements of the uses above it).
pub fn read(top: &ModelFile, edited: &Edited, target: &Target) -> Result<ModelFile, String> {
    let mut m = container(top, edited, &target.file)?;
    let folder = m.folder.clone();
    for key in &target.def {
        let v = m
            .models
            .get(key)
            .ok_or_else(|| format!("no model named '{key}' is defined here"))?;
        m = read_definition(key, v)?;
        m.folder.clone_from(&folder);
    }
    Ok(m)
}

/// Put `model`, edited, as the definition `keys` names inside `holder`.
fn put(holder: &mut ModelFile, keys: &[String], model: &ModelFile) -> Result<(), String> {
    match keys {
        [] => {
            let folder = holder.folder.clone();
            *holder = model.clone();
            holder.folder = folder;
        }
        [key, rest @ ..] => {
            let v = holder
                .models
                .get(key)
                .ok_or_else(|| format!("no model named '{key}' is defined here"))?;
            let mut inner = read_definition(key, v)?;
            put(&mut inner, rest, model)?;
            holder.models.insert(key.clone(), as_definition(&inner));
        }
    }
    Ok(())
}

/// Put `model`, edited, back where `target` says: into `top`, or into
/// `edited` (read in place of the disk until saved).
pub fn write(
    top: &mut ModelFile,
    edited: &mut Edited,
    target: &Target,
    model: &ModelFile,
) -> Result<(), String> {
    let mut held = container(top, edited, &target.file)?;
    put(&mut held, &target.def, model)?;
    match &target.file {
        None => {
            let folder = top.folder.clone();
            *top = held;
            top.folder = folder;
            top.resolve_models().map_err(|e| e.to_string())?;
        }
        Some(p) => {
            edited.insert(p.clone(), held.to_json());
            set_edited(edited.clone());
        }
    }
    Ok(())
}

/// Write every edited file to disk; they are no longer kept apart.
pub fn save(edited: &mut Edited) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for (p, text) in edited.iter() {
        std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p.clone());
    }
    edited.clear();
    set_edited(Edited::new());
    Ok(written)
}

/// A name for a new copy of `file` next to `beside`: `car-2.ssbx.json`,
/// or the next number free.
pub fn copy_name(file: &Path, beside: Option<&Path>, edited: &Edited) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model.ssbx.json".into());
    let (stem, ext) = match name.split_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.clone(), String::new()),
    };
    let dir = beside.map(Path::to_path_buf).unwrap_or_default();
    (2..)
        .map(|k| dir.join(format!("{stem}-{k}{ext}")))
        .find(|p| !p.exists() && !edited.contains_key(p))
        .expect("a free name")
}

/// Rewrite uses of the definition `def` in `model` and in its definitions
/// (where the name is not defined again) as uses of `file`.
fn point_at_file(model: &mut ModelFile, def: &str, file: &str) {
    for b in &mut model.blocks {
        if b.model.as_deref() == Some(def) {
            b.model = None;
            b.resolved = None;
            b.file = Some(file.to_string());
        }
    }
    for (name, v) in model.models.iter_mut() {
        if let Ok(mut d) = read_definition(name, v) {
            if !d.models.contains_key(def) {
                point_at_file(&mut d, def, file);
                *v = as_definition(&d);
            }
        }
    }
}

/// "Move to its own file" (ADR 0068): the definition `def` of `top` (saved
/// in `folder`) written to the new file `to`, kept in `edited` until
/// saved, and every use of it pointed at that file.
pub fn definition_to_file(
    top: &mut ModelFile,
    folder: Option<&Path>,
    edited: &mut Edited,
    def: &str,
    to: &Path,
) -> Result<(), String> {
    if to.exists() || edited.contains_key(to) {
        return Err(format!("{} exists: choose another name", to.display()));
    }
    // As read, with the definitions it uses from around it.
    let resolved = top
        .blocks
        .iter()
        .find(|b| b.model.as_deref() == Some(def))
        .and_then(|b| b.resolved.as_deref().cloned());
    let mut model = match resolved {
        Some(m) => m,
        None => {
            let v = top
                .models
                .get(def)
                .ok_or_else(|| format!("no model named '{def}' is defined here"))?;
            read_definition(def, v)?
        }
    };
    self_contained(&mut model);
    rebase(&mut model, folder, to.parent());
    edited.insert(to.to_path_buf(), model.to_json());
    set_edited(edited.clone());
    top.models.remove(def);
    point_at_file(top, def, &relative(to, folder));
    top.resolve_models().map_err(|e| e.to_string())
}

/// "Put in this file" (ADR 0068): the model file the use `name` reads
/// becomes a definition of `top`, named after the file, and every use of
/// that file at this level uses it. Returns the definition's name.
pub fn file_to_definition(
    top: &mut ModelFile,
    folder: Option<&Path>,
    name: &str,
) -> Result<String, String> {
    let entry = top
        .blocks
        .iter()
        .find(|b| b.name == name && b.file.is_some())
        .ok_or_else(|| format!("'{name}' does not use a model file"))?;
    let file = entry.file.clone().expect("found");
    // The file as written, without this use's replacements.
    let bare = simsandbox_file::BlockEntry {
        name: name.to_string(),
        file: Some(file.clone()),
        ..Default::default()
    };
    let (mut model, at, inner) = used_model(&bare, folder)?;
    self_contained(&mut model);
    rebase(&mut model, inner.as_deref(), folder);
    let stem = at
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem: String = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() || stem.starts_with(|c: char| c.is_ascii_digit()) {
        format!("model_{stem}")
    } else {
        stem
    };
    let key = free_key(&top.models, &stem);
    top.models.insert(key.clone(), as_definition(&model));
    for b in &mut top.blocks {
        if b.file.as_deref() == Some(file.as_str()) {
            b.file = None;
            b.model = Some(key.clone());
        }
    }
    top.resolve_models().map_err(|e| e.to_string())?;
    Ok(key)
}
