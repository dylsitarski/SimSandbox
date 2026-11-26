//! `simsandbox._native`: what the Python package calls (ADR 0039). Model
//! files go in as JSON text; catalogue and check reports come back as JSON
//! text, numbers as little-endian `f64` bytes that the Python layer views
//! as numpy arrays without copying them again. All the work is in
//! simsandbox-session, shared with the CLI and the app.

use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use simsandbox_session::describe as catalogue;
use simsandbox_session::examples::{registry, view_registry, EXAMPLES};
use simsandbox_session::render::{png, render_run, RenderOptions};
use simsandbox_session::run::{run_keeping as run_model, run_probes as run_model_probes};
use simsandbox_session::session::InputEvent;

create_exception!(
    _native,
    ModelError,
    PyValueError,
    "A model file that cannot be loaded, run or rendered."
);

fn json(v: &serde_json::Value) -> String {
    v.to_string()
}

fn bytes<'py>(py: Python<'py>, values: &[f64]) -> Bound<'py, PyBytes> {
    let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    PyBytes::new(py, &raw)
}

/// Every block, view and filter type as JSON, or one type.
#[pyfunction]
#[pyo3(signature = (name=None))]
fn describe(name: Option<&str>) -> PyResult<String> {
    let r = registry();
    let views = view_registry();
    match name {
        None => Ok(json(&catalogue::catalogue(&r, &views))),
        Some("units") => Ok(json(&catalogue::units())),
        Some(n) if r.info(n).is_some() => Ok(json(&catalogue::block_type(&r, n))),
        Some(n) => match views.get(n) {
            Some(t) => Ok(json(&catalogue::view_type(t))),
            None => {
                let known: Vec<&str> = r
                    .type_names()
                    .chain(views.types().iter().map(|t| t.name))
                    .collect();
                Err(ModelError::new_err(format!(
                    "unknown type '{n}'; known: {}",
                    known.join(", ")
                )))
            }
        },
    }
}

/// The model file's JSON Schema.
#[pyfunction]
fn schema() -> &'static str {
    simsandbox_session::SCHEMA
}

/// Check a model file: a JSON report with `ok`, `errors` (each with a
/// `path` or `line` and `column`), `warnings`, parameters and what it
/// records.
/// With `agent`, also what agents must write for the parts they made
/// (ADR 0107), each an error at its path.
#[pyfunction]
#[pyo3(signature = (text, agent = false))]
fn check(py: Python<'_>, text: &str, agent: bool) -> String {
    py.detach(|| json(&catalogue::check_with(text, &registry(), agent)))
}

/// The hash a run file records of its model (ADR 0059): SHA-256 of the
/// model with every parameter written out, leaving out `controls` and the
/// settings a run records itself.
#[pyfunction]
fn model_hash(text: &str) -> PyResult<String> {
    simsandbox_session::run::model_hash(text).map_err(ModelError::new_err)
}

/// This build of SimSandbox as run files record it: `simsandbox 0.0.1`.
#[pyfunction]
fn made_with() -> &'static str {
    simsandbox_session::run::MADE_WITH
}

/// The note for a run file that records `made_with` when this build is
/// not the one that made it (ADR 0082); `None` when it is.
#[pyfunction]
fn version_note(made_with: &str) -> Option<String> {
    simsandbox_session::runs::version_note(made_with)
}

/// Now as a run file records `created`: RFC 3339 in UTC, to the second.
#[pyfunction]
fn utc_now() -> String {
    simsandbox_session::runs::utc_now()
}

/// A run file's text read and written as every interface does (ADR 0059):
/// unknown fields and a newer format are refused, and the text returned
/// is the one form run files are saved in, whatever wrote `text`.
#[pyfunction]
fn run_file(text: &str) -> PyResult<String> {
    simsandbox_session::runs::RunFile::from_json(text)
        .map(|run| run.to_json())
        .map_err(|e| ModelError::new_err(e.to_string()))
}

/// A model's card (ADR 0107): Markdown, or with `json` the card as JSON
/// text. Raises `ModelError` if the model cannot load.
#[pyfunction]
#[pyo3(signature = (text, json = false, part = "", depth = 1))]
fn explain(text: &str, json: bool, part: &str, depth: usize) -> PyResult<String> {
    let r = registry();
    let folder = simsandbox_core::registry::current_folder();
    let s = simsandbox_session::Session::load_in("model", text, &r, folder.as_deref())
        .map_err(ModelError::new_err)?;
    let card = simsandbox_session::card::card_at(
        &s,
        &|t| r.info(t).map(|i| i.summary.clone()),
        part.trim_matches('/'),
        depth.max(1),
    )
    .map_err(ModelError::new_err)?;
    Ok(if json {
        card.to_string()
    } else {
        simsandbox_session::card::markdown(&card)
    })
}

/// The run file's JSON Schema.
#[pyfunction]
fn run_schema() -> &'static str {
    simsandbox_session::run::RUN_SCHEMA
}

/// The built-in examples as (name, model file, folder) triples: the
/// folder holds the files an example reads (ADR 0069 step 3), unpacked
/// once; `None` for the others.
#[pyfunction]
fn examples() -> Vec<(&'static str, &'static str, Option<String>)> {
    EXAMPLES
        .iter()
        .map(|e| {
            let folder = e.folder().map(|d| d.to_string_lossy().into_owned());
            (e.name, e.text, folder)
        })
        .collect()
}

/// What a run keeps (ADR 0076): the outputs named, every `every`-th step.
fn keep(signals: Option<Vec<String>>, every: usize) -> simsandbox_session::run::Keep {
    simsandbox_session::run::Keep { signals, every }
}

/// Run a model file. Returns a dict: `index` and `outputs` as lists of
/// (name, shape, bytes), `populations` as (name, dim, [bytes per row]),
/// `over_rows` (a table's rows, ADR 0079) as (name, dim, [bytes per row],
/// each frame `[version, n, ids…, values…]`),
/// and `labels`, `notes`, `warnings` as JSON text. `signals` and `every`
/// keep only those outputs and every so many steps (ADR 0076).
#[pyfunction]
#[pyo3(signature = (text, t_end=None, dt=None, track=None, signals=None, every=1))]
fn run<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Option<Vec<(f64, String, f64)>>,
    signals: Option<Vec<String>>,
    every: usize,
) -> PyResult<Bound<'py, PyDict>> {
    // Recorded input (ADR 0058), applied with the file's schedule.
    let track: Vec<InputEvent> = track
        .unwrap_or_default()
        .into_iter()
        .map(|(t, key, value)| InputEvent { t, key, value })
        .collect();
    let r = py
        .detach(|| run_model(text, &registry(), t_end, dt, track, &keep(signals, every)))
        .map_err(ModelError::new_err)?;
    results(py, &r)
}

/// A run's results as the dict [`run`] returns.
fn results<'py>(
    py: Python<'py>,
    r: &simsandbox_session::run::Results,
) -> PyResult<Bound<'py, PyDict>> {
    let rows = r.rows();
    let d = PyDict::new(py);
    let index = PyList::empty(py);
    for (name, values) in &r.index {
        index.append((name, vec![rows], bytes(py, values)))?;
    }
    let outputs = PyList::empty(py);
    for o in &r.outputs {
        let shape = catalogue::shape_with_axes(&o.shape, &o.axes);
        outputs.append((&o.name, json(&shape), bytes(py, &o.values)))?;
    }
    let populations = PyList::empty(py);
    let over_rows = PyList::empty(py);
    for p in &r.populations {
        let frames: Vec<_> = p.frames.iter().map(|f| bytes(py, f)).collect();
        match p.rows {
            true => over_rows.append((&p.name, p.dim, frames))?,
            false => populations.append((&p.name, p.dim, frames))?,
        }
    }
    d.set_item("rows", rows)?;
    d.set_item("index", index)?;
    d.set_item("outputs", outputs)?;
    d.set_item("populations", populations)?;
    d.set_item("over_rows", over_rows)?;
    d.set_item(
        "labels",
        json(&serde_json::json!(r.labels.map(catalogue::labels))),
    )?;
    d.set_item("notes", r.notes.clone())?;
    d.set_item("warnings", r.warnings.clone())?;
    Ok(d)
}

/// Run a composition (ADR 0065) to `t_end` (the file's, if not given):
/// a dict of each model's results, as [`run`] gives one model's, under
/// `models` (in written order), with `tick` and `warnings`.
#[pyfunction]
#[pyo3(signature = (text, t_end=None, signals=None, every=1))]
fn run_composition<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    signals: Option<Vec<String>>,
    every: usize,
) -> PyResult<Bound<'py, PyDict>> {
    let c = py
        .detach(|| {
            simsandbox_session::compose::run_composition_keeping(
                text,
                &registry(),
                t_end,
                simsandbox_core::parallel::Parallel::Auto,
                &keep(signals, every),
            )
        })
        .map_err(ModelError::new_err)?;
    let models = PyList::empty(py);
    for (name, r) in &c.models {
        models.append((name, results(py, r)?))?;
    }
    let d = PyDict::new(py);
    d.set_item("models", models)?;
    d.set_item("tick", c.tick)?;
    d.set_item("warnings", c.warnings.clone())?;
    Ok(d)
}

/// Run a model file with its probes (ADR 0062), all of them or those in
/// `names`, writing the files of those that have one into `save` if
/// given. Returns a dict: `probes` as a list of (name, times bytes,
/// [(signal, bytes)], [(field, dims, bytes)], [(population, dim, [bytes
/// per sample])]), `files` written, and `labels`, `warnings`.
#[pyfunction]
#[pyo3(signature = (text, t_end=None, dt=None, track=None, names=None, save=None, memory=None))]
#[allow(clippy::too_many_arguments)]
fn run_probes<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Option<Vec<(f64, String, f64)>>,
    names: Option<Vec<String>>,
    save: Option<std::path::PathBuf>,
    memory: Option<f64>,
) -> PyResult<Bound<'py, PyDict>> {
    use simsandbox_views::history::FieldShape;
    let track: Vec<InputEvent> = track
        .unwrap_or_default()
        .into_iter()
        .map(|(t, key, value)| InputEvent { t, key, value })
        .collect();
    let names = names.unwrap_or_default();
    let (r, files) = py
        .detach(|| {
            let r = run_model_probes(
                text,
                &registry(),
                t_end,
                dt,
                track,
                &names,
                save.as_deref(),
                memory,
            )?;
            let files = match &save {
                Some(dir) => r
                    .write_files(dir)
                    .map_err(|e| format!("{}: {e}", dir.display()))?,
                None => Default::default(),
            };
            Ok::<_, String>((r, files))
        })
        .map_err(ModelError::new_err)?;
    let probes = PyList::empty(py);
    for (spec, h) in &r.probes {
        let h = h.read();
        let signals = PyList::empty(py);
        for (i, n) in h.names().iter().enumerate() {
            let values = h.values(simsandbox_views::history::SignalRef::Signal(i));
            signals.append((n, bytes(py, &values)))?;
        }
        let fields = PyList::empty(py);
        let populations = PyList::empty(py);
        let over_rows = PyList::empty(py);
        for f in h.fields().iter() {
            let frames = (0..h.len()).map(|k| f.frame(k));
            match f.shape {
                FieldShape::Grid { nx, ny } => fields.append((
                    &f.name,
                    vec![ny, nx],
                    bytes(py, &frames.flatten().copied().collect::<Vec<_>>()),
                ))?,
                FieldShape::Points { count, dim } => fields.append((
                    &f.name,
                    vec![count, dim],
                    bytes(py, &frames.flatten().copied().collect::<Vec<_>>()),
                ))?,
                FieldShape::Population { dim } => populations.append((
                    &f.name,
                    dim,
                    frames.map(|v| bytes(py, v)).collect::<Vec<_>>(),
                ))?,
                FieldShape::Rows { dim } => over_rows.append((
                    &f.name,
                    dim,
                    frames.map(|v| bytes(py, v)).collect::<Vec<_>>(),
                ))?,
                // The last axis fastest, as numpy stores it (ADR 0081).
                FieldShape::Tensor { rank, dims } => fields.append((
                    &f.name,
                    dims[..rank].iter().rev().copied().collect::<Vec<_>>(),
                    bytes(py, &frames.flatten().copied().collect::<Vec<_>>()),
                ))?,
            }
        }
        probes.append((
            &spec.name,
            bytes(py, h.times()),
            signals,
            fields,
            populations,
            over_rows,
        ))?;
    }
    let (written, notes) = files;
    let mut warnings = r.warnings.clone();
    warnings.extend(notes);
    let d = PyDict::new(py);
    d.set_item("probes", probes)?;
    d.set_item("files", written)?;
    d.set_item(
        "memory",
        json(&serde_json::json!({
            "budget": r.memory.budget,
            "held": r.memory.held,
            "parts": r.memory.parts,
        })),
    )?;
    d.set_item(
        "labels",
        json(&serde_json::json!(r.labels.map(catalogue::labels))),
    )?;
    d.set_item("warnings", warnings)?;
    Ok(d)
}

/// Run a model file to `t_end` (the file's, if not given) and draw its
/// views as a PNG. `track` is recorded input replayed with it (ADR 0058),
/// as for [`run`].
#[pyfunction]
#[pyo3(signature = (text, t_end=None, view=None, width=1200.0, height=800.0, scale=1.0, light=false, track=None))]
#[allow(clippy::too_many_arguments)]
fn render<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    view: Option<usize>,
    width: f32,
    height: f32,
    scale: f32,
    light: bool,
    track: Option<Vec<(f64, String, f64)>>,
) -> PyResult<(Bound<'py, PyBytes>, Vec<String>)> {
    let track = simsandbox_session::runs::events(&track.unwrap_or_default());
    let (image, warnings) = py
        .detach(|| -> Result<_, String> {
            let o = RenderOptions {
                width,
                height,
                scale,
                view,
                light,
                inside: None,
                expand: Vec::new(),
                data: false,
                values: false,
            };
            let mut warnings = Vec::new();
            let image = render_run("model", text, &registry(), t_end, track, &o, &mut warnings)?;
            Ok((png(&image)?, warnings))
        })
        .map_err(ModelError::new_err)?;
    Ok((PyBytes::new(py, &image), warnings))
}

/// The block diagram of a model file as a PNG, as the node canvas shows
/// it (ADR 0042); drawn even if the model cannot run.
#[pyfunction]
#[pyo3(signature = (text, width=1200.0, height=800.0, scale=1.0, light=false, inside=None, data=true, values=false, expand=Vec::new()))]
#[allow(clippy::too_many_arguments)]
fn render_diagram<'py>(
    py: Python<'py>,
    text: &str,
    width: f32,
    height: f32,
    scale: f32,
    light: bool,
    inside: Option<String>,
    data: bool,
    values: bool,
    expand: Vec<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let image = py
        .detach(|| -> Result<_, String> {
            let (file, _) =
                simsandbox_session::ModelFile::from_json(text).map_err(|e| e.to_string())?;
            let o = RenderOptions {
                width,
                height,
                scale,
                view: None,
                light,
                inside,
                expand,
                data,
                values,
            };
            png(&simsandbox_session::render::render_diagram(
                &file,
                &registry(),
                &o,
            )?)
        })
        .map_err(ModelError::new_err)?;
    Ok(PyBytes::new(py, &image))
}

/// Run a model file to `t` and return the model file that starts from the
/// state there (ADR 0042), with a note per block.
#[pyfunction]
fn start_from(py: Python<'_>, text: &str, t: f64) -> PyResult<(String, Vec<String>)> {
    py.detach(|| simsandbox_session::run::start_from(text, &registry(), t))
        .map_err(ModelError::new_err)
}

/// Move blocks into a new model defined in the file, as "Make model" on
/// the canvas does (ADR 0068). Returns the changed model file and a note.
#[pyfunction]
fn group(text: &str, blocks: Vec<String>) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let note = apply_in(
        &mut file,
        &Scope::Model,
        &registry(),
        &Edit::GroupModel { blocks },
    )
    .map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// Expose a value inside a model as a parameter of that model (ADR 0110
/// point 7): `model` is a model defined in the file, or `None` for the
/// file itself; `key` is `block.param` or a path into a used model.
/// Returns the changed model file and the parameter's name.
#[pyfunction]
#[pyo3(signature = (text, model, key, name=None))]
fn expose(
    text: &str,
    model: Option<&str>,
    key: &str,
    name: Option<&str>,
) -> PyResult<(String, String)> {
    let folder = simsandbox_core::registry::current_folder();
    simsandbox_file::encapsulate::expose(text, &registry(), folder.as_deref(), model, key, name)
        .map_err(ModelError::new_err)
}

/// Offer an output inside a model as an output of that model (ADR 0110
/// point 4): `signal` is `block.port` or a path into a used model.
/// Returns the changed model file and the output's name.
#[pyfunction]
#[pyo3(signature = (text, model, signal, name=None))]
fn offer_output(
    text: &str,
    model: Option<&str>,
    signal: &str,
    name: Option<&str>,
) -> PyResult<(String, String)> {
    simsandbox_file::encapsulate::offer_output(text, model, signal, name)
        .map_err(ModelError::new_err)
}

/// Copy blocks or uses beside themselves, with their values and the wires
/// among them, as "Duplicate" on the canvas does (ADR 0069). Returns the
/// changed model file and a note.
#[pyfunction]
fn duplicate(text: &str, blocks: Vec<String>) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let note = apply_in(
        &mut file,
        &Scope::Model,
        &registry(),
        &Edit::Duplicate { blocks },
    )
    .map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// A study of a used model beside it (ADR 0070), as "Study this model"
/// does on the canvas. Returns the changed model file and a note.
#[pyfunction]
fn study(text: &str, block: String) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let note = apply_in(
        &mut file,
        &Scope::Model,
        &registry(),
        &Edit::Study { block },
    )
    .map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// Write a strategy model (ADR 0078 point 7): `spec` is the strategy as
/// JSON (`{"kind": "minimize", …}`), `target` a study, a used model or
/// none (the whole model). Returns the changed model file and a note.
#[pyfunction]
#[pyo3(signature = (text, spec, target=None))]
fn strategy(text: &str, spec: &str, target: Option<String>) -> PyResult<(String, String)> {
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let spec: simsandbox_session::strategies::Strategy =
        serde_json::from_str(spec).map_err(|e| ModelError::new_err(e.to_string()))?;
    let note =
        simsandbox_session::strategies::write(&mut file, &registry(), target.as_deref(), &spec)
            .map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// Study the whole model (ADR 0071), as "Study this model" does on the
/// canvas background. Returns the changed model file and a note.
#[pyfunction]
fn study_model(text: &str) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let note = apply_in(&mut file, &Scope::Model, &registry(), &Edit::StudyModel)
        .map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// Turn an expression filter into a formula block (ADR 0032, ADR 0050),
/// connected to a model input if `to` is given. Returns the changed model
/// file and a note.
#[pyfunction]
#[pyo3(signature = (text, filter, to=None))]
fn promote(text: &str, filter: String, to: Option<String>) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply, Edit};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let edit = match to {
        Some(to) => Edit::PromoteConnect { filter, to },
        None => Edit::Promote { filter },
    };
    let note = apply(&mut file, &registry(), &edit).map_err(ModelError::new_err)?;
    Ok((file.to_json(), note))
}

/// Set the folder blocks on this thread read the files their settings
/// name from (ADR 0063): the model file's. Returns the folder it replaces,
/// to set back.
#[pyfunction]
#[pyo3(signature = (folder=None))]
fn set_folder(folder: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    simsandbox_core::registry::set_folder(folder)
}

/// Why a wire from `source` to `target` cannot be added to the model file
/// `text`, or `None` if it can. `Model.connect` calls it so a wire to a
/// block or port that does not exist fails where it is written, not at
/// the run. It judges as the node canvas does (a wire it would draw
/// dangling is refused), building only the one or two blocks named, so
/// its cost does not grow with the model. Not judged, and so let through
/// to `check`: a file that does not parse, a probe's samples
/// (`probe:…`), and an end on a block or used model that cannot be built
/// alone (its own error is the one to read).
#[pyfunction]
fn connection_problem(text: &str, source: &str, target: &str) -> Option<String> {
    use simsandbox_session::diagram::{Diagram, NodeKind};
    if source.starts_with("probe:") {
        return None;
    }
    let mut data: serde_json::Value = serde_json::from_str(text).ok()?;
    let blocks: Vec<(String, bool)> = data
        .get("blocks")?
        .as_array()?
        .iter()
        .filter_map(|b| {
            let name = b.get("name")?.as_str()?.to_string();
            let is_use =
                b.get("type").is_none() && (b.get("file").is_some() || b.get("model").is_some());
            Some((name, is_use))
        })
        .collect();
    // The block (or used model) an end names, and the rest: `car.v` is
    // `car` and `v`; `plant/car.v` is `plant` and `car.v` if `plant` is a
    // use, as `plant/force` is.
    let split = |key: &str| -> Option<(String, String)> {
        if let Some((u, rest)) = key.split_once('/') {
            if blocks.iter().any(|(n, is_use)| *is_use && n == u) {
                return Some((u.to_string(), rest.to_string()));
            }
        }
        key.split_once('.')
            .map(|(b, p)| (b.to_string(), p.to_string()))
    };
    let names = || {
        blocks
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut ends = Vec::new();
    for (key, what) in [(source, "source"), (target, "target")] {
        let Some((block, rest)) = split(key) else {
            if let Some((u, _)) = key.split_once('/') {
                return Some(format!(
                    "the {what} '{key}' names no used model '{u}'; blocks: {}",
                    names()
                ));
            }
            return Some(format!(
                "the {what} '{key}' is not 'block.port' (or a used model's 'use/port')"
            ));
        };
        if !blocks.iter().any(|(n, _)| *n == block) {
            return Some(format!(
                "the {what} '{key}' names no block '{block}'; blocks: {}",
                names()
            ));
        }
        ends.push((block, rest));
    }
    // The file with only those blocks and this wire, drawn as the canvas
    // draws it.
    let keep = [ends[0].0.clone(), ends[1].0.clone()];
    if let Some(list) = data.get_mut("blocks").and_then(|b| b.as_array_mut()) {
        list.retain(|b| {
            b.get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| keep.iter().any(|k| k == n))
        });
    }
    let map = data.as_object_mut()?;
    map.insert(
        "connections".into(),
        serde_json::json!([{ "from": source, "to": target }]),
    );
    map.remove("controls");
    let (file, _) = simsandbox_session::ModelFile::from_json(&data.to_string()).ok()?;
    let d = Diagram::new(&file, &registry());
    if d.dangling.is_empty() {
        return None;
    }
    let node = |name: &str| d.nodes.iter().find(|n| n.name == name);
    let (from_node, to_node) = (node(&ends[0].0)?, node(&ends[1].0)?);
    if from_node.error.is_some() || to_node.error.is_some() {
        return None;
    }
    let list = |v: Vec<&str>| match v.is_empty() {
        true => "none".to_string(),
        false => v.join(", "),
    };
    let what = |n: &simsandbox_session::diagram::Node| match n.kind {
        NodeKind::Use => "the used model",
        _ => "the block",
    };
    let outputs: Vec<&str> = from_node.outputs.iter().map(|p| p.name.as_str()).collect();
    if !outputs.contains(&ends[0].1.as_str()) {
        return Some(format!(
            "the source '{source}': {} '{}' has no output '{}'; outputs: {}",
            what(from_node),
            ends[0].0,
            ends[0].1,
            list(outputs)
        ));
    }
    // A parameter the file sets by a formula is not shown as a value, but
    // may still be driven: the run decides.
    let in_file = |block: &str, param: &str| {
        data.get("blocks")
            .and_then(|b| b.as_array())
            .and_then(|b| {
                b.iter()
                    .find(|b| b.get("name").and_then(|n| n.as_str()) == Some(block))
            })
            .and_then(|b| b.get("params"))
            .is_some_and(|p| p.get(param).is_some())
    };
    if in_file(&ends[1].0, &ends[1].1) {
        return None;
    }
    let inputs: Vec<&str> = to_node.inputs.iter().map(|p| p.name.as_str()).collect();
    let params: Vec<&str> = to_node.values.iter().map(|v| v.name.as_str()).collect();
    if !inputs.contains(&ends[1].1.as_str()) && !params.contains(&ends[1].1.as_str()) {
        return Some(format!(
            "the target '{target}': {} '{}' has no input or parameter '{}'; inputs: {}; parameters: {}",
            what(to_node),
            ends[1].0,
            ends[1].1,
            list(inputs),
            list(params)
        ));
    }
    Some(format!("cannot wire '{source}' to '{target}'"))
}

/// What is wrong with a scene view (`plot.scene`, ADR 0093) for this
/// model, judged without building it: the scene's form as `check` reads
/// it (keys, primitives, attributes, frames, extent, `view_from`), each
/// bound signal named by one of the model's blocks or used models, a
/// filter, a probe or time, and each `grab` an `input.pointer` block.
/// Left to `check`: whether a block has that output, units, and tables
/// (`None` here then, as for a file that does not parse).
/// What is wrong with one block's entry (a model file's block object, as
/// JSON) when its type builds it alone, or `None`: for `Model.add_block`
/// to refuse a mechanism's rows at the call (ADR 0105 point 7). Types that
/// are not registered (a definition in the file) are left to `check`.
#[pyfunction]
fn block_problem(block: &str) -> Option<String> {
    let entry: simsandbox_file::BlockEntry = match serde_json::from_str(block) {
        Ok(e) => e,
        Err(e) => return Some(format!("not a block: {e}")),
    };
    let registry = simsandbox_session::examples::registry();
    if !registry.contains(&entry.type_name) {
        return None;
    }
    registry
        .build(&entry.type_name, &entry.name, &entry.config(None))
        .err()
        .map(|e| e.to_string())
}

#[pyfunction]
fn scene_problem(text: &str, scene: &str) -> Option<String> {
    use simsandbox_views::scene::{bound, Scene, SCENE};
    use simsandbox_views::Available;
    let data: serde_json::Value = serde_json::from_str(text).ok()?;
    let view: serde_json::Value = match serde_json::from_str(scene) {
        Ok(v) => v,
        Err(e) => return Some(format!("the scene is not JSON: {e}")),
    };
    let Some(view) = view.as_object() else {
        return Some("a scene is an object of its settings".into());
    };
    if view.get("type").and_then(|t| t.as_str()) != Some(SCENE) {
        return Some(format!("a scene's type is '{SCENE}'"));
    }
    // Its template layers put in place, as it draws (ADR 0113).
    let expanded;
    let view = match serde_json::from_value::<simsandbox_file::ViewEntry>(serde_json::Value::Object(
        view.clone(),
    )) {
        Ok(entry) if simsandbox_file::templates::uses_templates(&entry) => {
            // Its own and those the models it uses offer (ADR 0113).
            let folder = simsandbox_core::registry::current_folder();
            let templates = match simsandbox_file::ModelFile::from_json(text) {
                Ok((file, _)) => simsandbox_file::compose::templates_in(&file, folder.as_deref()),
                Err(_) => data
                    .get("templates")
                    .and_then(|t| serde_json::from_value(t.clone()).ok())
                    .unwrap_or_default(),
            };
            match simsandbox_file::templates::expand(&entry, &templates)
                .and_then(|e| serde_json::to_value(e).map_err(|e| e.to_string()))
            {
                Ok(serde_json::Value::Object(o)) => {
                    expanded = o;
                    &expanded
                }
                Ok(_) => return Some("a scene is an object of its settings".into()),
                Err(e) => return Some(e),
            }
        }
        _ => view,
    };
    // Its settings: all but what every view has.
    let settings: simsandbox_views::view::Settings = view
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "type" | "canvas"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let names = |list: Option<&serde_json::Value>, key: &str| -> Vec<(String, String)> {
        list.and_then(|l| l.as_array())
            .map(|l| {
                l.iter()
                    .filter_map(|e| {
                        let name = e.get("name")?.as_str()?.to_string();
                        let kind = e.get(key).and_then(|t| t.as_str()).unwrap_or("");
                        Some((name, kind.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let blocks = names(data.get("blocks"), "type");
    let controls = data.get("controls");
    let filters = names(controls.and_then(|c| c.get("filters")), "type");
    let probes = names(controls.and_then(|c| c.get("probes")), "type");
    let has = |list: &[(String, String)], n: &str| list.iter().any(|(name, _)| name == n);
    let named = |s: &str| -> bool {
        if s == "t" {
            return true;
        }
        if let Some(rest) = s.strip_prefix("probe:") {
            return rest.split_once('/').is_some_and(|(p, _)| has(&probes, p));
        }
        let first = s.split(['.', '/', '[']).next().unwrap_or(s);
        has(&blocks, first) || has(&filters, first) || (first == "grid" && s.len() > 5)
    };
    let mut signals = Vec::new();
    for (path, s) in bound(&settings) {
        if !named(&s) {
            let list: Vec<&str> = blocks.iter().map(|(n, _)| n.as_str()).collect();
            return Some(format!(
                "{path}: '{s}' names no block, filter or probe of the model; blocks: {}",
                list.join(", ")
            ));
        }
        signals.push(s);
    }
    let pointers: Vec<String> = blocks
        .iter()
        .filter(|(_, t)| t == "input.pointer")
        .map(|(n, _)| n.clone())
        .collect();
    let available = Available {
        scalars: &signals,
        pointers: Some(&pointers),
        ..Default::default()
    };
    Scene::check(&settings, &available).err()
}

/// A model file in the current form (ADR 0044): the text, the older forms
/// converted, and what was dropped as unusable.
#[pyfunction]
fn upgrade(text: &str) -> PyResult<(String, Vec<String>, Vec<String>)> {
    catalogue::upgrade(text, &registry()).map_err(ModelError::new_err)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ModelError", m.py().get_type::<ModelError>())?;
    m.add_function(wrap_pyfunction!(describe, m)?)?;
    m.add_function(wrap_pyfunction!(promote, m)?)?;
    m.add_function(wrap_pyfunction!(set_folder, m)?)?;
    m.add_function(wrap_pyfunction!(run_composition, m)?)?;
    m.add_function(wrap_pyfunction!(schema, m)?)?;
    m.add_function(wrap_pyfunction!(check, m)?)?;
    m.add_function(wrap_pyfunction!(examples, m)?)?;
    m.add_function(wrap_pyfunction!(run, m)?)?;
    m.add_function(wrap_pyfunction!(run_probes, m)?)?;
    m.add_function(wrap_pyfunction!(model_hash, m)?)?;
    m.add_function(wrap_pyfunction!(made_with, m)?)?;
    m.add_function(wrap_pyfunction!(run_schema, m)?)?;
    m.add_function(wrap_pyfunction!(version_note, m)?)?;
    m.add_function(wrap_pyfunction!(utc_now, m)?)?;
    m.add_function(wrap_pyfunction!(run_file, m)?)?;
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(render_diagram, m)?)?;
    m.add_function(wrap_pyfunction!(start_from, m)?)?;
    m.add_function(wrap_pyfunction!(group, m)?)?;
    m.add_function(wrap_pyfunction!(duplicate, m)?)?;
    m.add_function(wrap_pyfunction!(study, m)?)?;
    m.add_function(wrap_pyfunction!(study_model, m)?)?;
    m.add_function(wrap_pyfunction!(strategy, m)?)?;
    m.add_function(wrap_pyfunction!(upgrade, m)?)?;
    m.add_function(wrap_pyfunction!(connection_problem, m)?)?;
    m.add_function(wrap_pyfunction!(block_problem, m)?)?;
    m.add_function(wrap_pyfunction!(explain, m)?)?;
    m.add_function(wrap_pyfunction!(scene_problem, m)?)?;
    m.add_function(wrap_pyfunction!(expose, m)?)?;
    m.add_function(wrap_pyfunction!(offer_output, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
