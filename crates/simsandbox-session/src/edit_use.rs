//! Editing a used model from the model using it (ADR 0066 step 4c): the
//! three answers to the edit question, and where edits then go.
//!
//! Model files edited and not yet saved are kept by path (`edited`) and
//! read in place of the disk on this thread
//! ([`simsandbox_file::compose::set_edited`]), so the run, `check` and
//! the diagram see them before Save writes them.

use simsandbox_file::compose::{set_edited, used_model};
use simsandbox_file::ModelFile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Files edited and not yet saved: full path → text.
pub type Edited = BTreeMap<PathBuf, String>;

/// The answer to the edit question.
#[derive(Clone, Debug, PartialEq)]
pub enum How {
    /// Edit the used file, which every model using it sees.
    Original,
    /// Write its contents to this new file (full path), point this use at
    /// it, and edit the copy.
    NewCopy(PathBuf),
    /// Write its contents into the model open, detached from any file.
    LocalCopy,
}

/// Where edits inside a used model go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The file holding the model edited; `None` for the model open.
    pub file: Option<PathBuf>,
    /// Local copies inside that file, by name, down to the model edited.
    pub inline: Vec<String>,
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
        inline: Vec::new(),
    };
    for name in path {
        let entry =
            at.0.blocks
                .iter()
                .find(|b| &b.name == name && b.is_use())
                .ok_or_else(|| format!("no model used as '{name}'"))?;
        let (model, file, inner) = used_model(entry, at.1.as_deref())?;
        match file {
            Some(f) => {
                target = Target {
                    file: Some(f),
                    inline: Vec::new(),
                }
            }
            None => target.inline.push(name.clone()),
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

/// `p` relative to `to` if it is inside it, else as it is.
fn relative(p: &Path, to: Option<&Path>) -> String {
    to.and_then(|t| p.strip_prefix(t).ok())
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Make the files `model` names (uses, replacements, data files), written
/// from `from`, found the same from `to`.
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
        // A local copy's files are found from the same folder as its own.
        if let Some(inline) = &b.model {
            if let Ok((mut inner, _)) = ModelFile::from_json(&inline.to_string()) {
                rebase(&mut inner, from, to);
                b.model = serde_json::from_str(&inner.to_json()).ok();
            }
        }
    }
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
    let first = top
        .blocks
        .iter_mut()
        .find(|b| b.name == path[0] && b.is_use())
        .ok_or_else(|| format!("no model used as '{}'", path[0]))?;
    match how {
        How::Original => Ok(original),
        How::NewCopy(to) => {
            if to.exists() || edited.contains_key(&to) {
                return Err(format!("{} exists: choose another name", to.display()));
            }
            let (model, from) = chain.last().cloned().expect("a path has a model");
            let mut copy = model;
            rebase(&mut copy, from.as_deref(), to.parent());
            copy.folder = to.parent().map(Path::to_path_buf);
            edited.insert(to.clone(), copy.to_json());
            let named = relative(&to, folder);
            if path.len() == 1 {
                // Its replacements are in the copy now.
                first.file = Some(named);
                first.model = None;
                first.files.clear();
            } else {
                first.files.insert(path[1..].join("/"), named);
            }
            set_edited(edited.clone());
            Ok(Target {
                file: Some(to),
                inline: Vec::new(),
            })
        }
        How::LocalCopy => {
            // Each model along the path, its files found from here, held
            // in the one above it.
            let mut models: Vec<ModelFile> = chain
                .into_iter()
                .map(|(mut m, from)| {
                    rebase(&mut m, from.as_deref(), folder);
                    m
                })
                .collect();
            while models.len() > 1 {
                let inner = models.pop().expect("more than one");
                let name = &path[models.len()];
                let holder = models.last_mut().expect("one left");
                let entry = holder
                    .blocks
                    .iter_mut()
                    .find(|b| &b.name == name)
                    .expect("walked");
                hold(entry, &inner);
            }
            hold(first, &models[0]);
            Ok(Target {
                file: None,
                inline: path.to_vec(),
            })
        }
    }
}

/// `entry` becomes a local copy of `model`.
fn hold(entry: &mut simsandbox_file::BlockEntry, model: &ModelFile) {
    entry.file = None;
    entry.files.clear();
    entry.model = serde_json::from_str(&model.to_json()).ok();
}

/// The file `target.file` names: as edited, else from disk; the model
/// open for `None`.
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
    for name in &target.inline {
        let inline = m
            .blocks
            .iter()
            .find(|b| &b.name == name)
            .and_then(|b| b.model.clone())
            .ok_or_else(|| format!("no local copy named '{name}'"))?;
        let (inner, _) = ModelFile::from_json(&inline.to_string()).map_err(|e| e.to_string())?;
        m = inner;
        m.folder.clone_from(&folder);
    }
    Ok(m)
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
    // The models from the container down, then each put into the one
    // above it.
    let mut stack = vec![held.clone()];
    for name in &target.inline {
        let next = read(
            stack.last().expect("one"),
            &Edited::new(),
            &Target {
                file: None,
                inline: vec![name.clone()],
            },
        )?;
        stack.push(next);
    }
    stack.pop();
    let mut inner = model.clone();
    for (name, mut holder) in target.inline.iter().rev().zip(stack.into_iter().rev()) {
        let entry = holder
            .blocks
            .iter_mut()
            .find(|b| &b.name == name)
            .expect("read above");
        hold(entry, &inner);
        inner = holder;
    }
    held = inner;
    match &target.file {
        None => {
            let folder = top.folder.clone();
            *top = held;
            top.folder = folder;
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
