//! `simsandbox._native`: what the Python package calls (ADR 0039). Model
//! files go in as JSON text; catalogue and check reports come back as JSON
//! text, numbers as little-endian `f64` bytes that the Python layer views
//! as numpy arrays without copying them again. All the work is in
//! simsandbox-session, shared with the CLI and the app.

use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use simsandbox_session::examples::{registry, view_registry, EXAMPLES};
use simsandbox_session::render::{png, render as render_views, RenderOptions};
use simsandbox_session::run::{run_probes as run_model_probes, run_with as run_model};
use simsandbox_session::session::InputEvent;
use simsandbox_session::{describe as catalogue, Session};

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
#[pyfunction]
fn check(py: Python<'_>, text: &str) -> String {
    py.detach(|| json(&catalogue::check(text, &registry())))
}

/// The hash a run file records of its model (ADR 0059): SHA-256 of the
/// model with every parameter written out, leaving out `controls` and the
/// settings a run records itself.
#[pyfunction]
fn model_hash(text: &str) -> PyResult<String> {
    simsandbox_session::run::model_hash(text).map_err(ModelError::new_err)
}

/// The run file's JSON Schema.
#[pyfunction]
fn run_schema() -> &'static str {
    simsandbox_session::run::RUN_SCHEMA
}

/// The built-in examples as (name, model file) pairs.
#[pyfunction]
fn examples() -> Vec<(&'static str, &'static str)> {
    EXAMPLES.iter().map(|e| (e.name, e.text)).collect()
}

/// Run a model file. Returns a dict: `index` and `outputs` as lists of
/// (name, shape, bytes), `populations` as (name, dim, [bytes per row]),
/// and `labels`, `notes`, `warnings` as JSON text.
#[pyfunction]
#[pyo3(signature = (text, t_end=None, dt=None, track=None))]
fn run<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Option<Vec<(f64, String, f64)>>,
) -> PyResult<Bound<'py, PyDict>> {
    // Recorded input (ADR 0058), applied with the file's schedule.
    let track: Vec<InputEvent> = track
        .unwrap_or_default()
        .into_iter()
        .map(|(t, key, value)| InputEvent { t, key, value })
        .collect();
    let r = py
        .detach(|| run_model(text, &registry(), t_end, dt, track))
        .map_err(ModelError::new_err)?;
    let rows = r.rows();
    let d = PyDict::new(py);
    let index = PyList::empty(py);
    for (name, values) in &r.index {
        index.append((name, vec![rows], bytes(py, values)))?;
    }
    let outputs = PyList::empty(py);
    for o in &r.outputs {
        let shape = catalogue::shape(&o.shape);
        outputs.append((&o.name, json(&shape), bytes(py, &o.values)))?;
    }
    let populations = PyList::empty(py);
    for p in &r.populations {
        let frames: Vec<_> = p.frames.iter().map(|f| bytes(py, f)).collect();
        populations.append((&p.name, p.dim, frames))?;
    }
    d.set_item("rows", rows)?;
    d.set_item("index", index)?;
    d.set_item("outputs", outputs)?;
    d.set_item("populations", populations)?;
    d.set_item(
        "labels",
        json(&serde_json::json!(r.labels.map(catalogue::labels))),
    )?;
    d.set_item("notes", r.notes.clone())?;
    d.set_item("warnings", r.warnings.clone())?;
    Ok(d)
}

/// Run a model file with its probes (ADR 0062), all of them or those in
/// `names`, writing the files of those that have one into `save` if
/// given. Returns a dict: `probes` as a list of (name, times bytes,
/// [(signal, bytes)], [(field, dims, bytes)], [(population, dim, [bytes
/// per sample])]), `files` written, and `labels`, `warnings`.
#[pyfunction]
#[pyo3(signature = (text, t_end=None, dt=None, track=None, names=None, save=None))]
fn run_probes<'py>(
    py: Python<'py>,
    text: &str,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Option<Vec<(f64, String, f64)>>,
    names: Option<Vec<String>>,
    save: Option<std::path::PathBuf>,
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
            let r = run_model_probes(text, &registry(), t_end, dt, track, &names, save.as_deref())?;
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
        for f in h.fields().iter().filter(|f| !f.whole) {
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
            }
        }
        probes.append((
            &spec.name,
            bytes(py, h.times()),
            signals,
            fields,
            populations,
        ))?;
    }
    let (written, notes) = files;
    let mut warnings = r.warnings.clone();
    warnings.extend(notes);
    let d = PyDict::new(py);
    d.set_item("probes", probes)?;
    d.set_item("files", written)?;
    d.set_item(
        "labels",
        json(&serde_json::json!(r.labels.map(catalogue::labels))),
    )?;
    d.set_item("warnings", warnings)?;
    Ok(d)
}

/// Run a model file to `t_end` (the file's, if not given) and draw its
/// views as a PNG.
#[pyfunction]
#[pyo3(signature = (text, t_end=None, view=None, width=1200.0, height=800.0, scale=1.0, light=false))]
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
) -> PyResult<(Bound<'py, PyBytes>, Vec<String>)> {
    let (image, warnings) = py
        .detach(|| -> Result<_, String> {
            let mut s = Session::load("model", text, &registry())?;
            let t = t_end
                .or(s.file.run.t_end)
                .unwrap_or(simsandbox_session::run::DEFAULT_T_END);
            s.run_until(t).map_err(|e| e.to_string())?;
            let o = RenderOptions {
                width,
                height,
                scale,
                view,
                light,
                subsystem: None,
                data: false,
                values: false,
            };
            let (image, mut warnings) = render_views(&s, view_registry(), &o)?;
            warnings.splice(0..0, s.warnings.iter().cloned());
            Ok((png(&image)?, warnings))
        })
        .map_err(ModelError::new_err)?;
    Ok((PyBytes::new(py, &image), warnings))
}

/// The block diagram of a model file as a PNG, as the node canvas shows
/// it (ADR 0042); drawn even if the model cannot run.
#[pyfunction]
#[pyo3(signature = (text, width=1200.0, height=800.0, scale=1.0, light=false, subsystem=None, data=true, values=false))]
#[allow(clippy::too_many_arguments)]
fn render_diagram<'py>(
    py: Python<'py>,
    text: &str,
    width: f32,
    height: f32,
    scale: f32,
    light: bool,
    subsystem: Option<String>,
    data: bool,
    values: bool,
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
                subsystem,
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

/// Group blocks into a new subsystem, as "Make subsystem" on the canvas
/// does (ADR 0043). Returns the changed model file and a note.
#[pyfunction]
#[pyo3(signature = (text, blocks, subsystem=None))]
fn group(text: &str, blocks: Vec<String>, subsystem: Option<String>) -> PyResult<(String, String)> {
    use simsandbox_session::diagram::{apply_in, Edit, Scope};
    let (mut file, _) = simsandbox_session::ModelFile::from_json(text)
        .map_err(|e| ModelError::new_err(e.to_string()))?;
    let scope = subsystem.map_or(Scope::Model, Scope::Subsystem);
    let note = apply_in(&mut file, &scope, &registry(), &Edit::Group { blocks })
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
    m.add_function(wrap_pyfunction!(schema, m)?)?;
    m.add_function(wrap_pyfunction!(check, m)?)?;
    m.add_function(wrap_pyfunction!(examples, m)?)?;
    m.add_function(wrap_pyfunction!(run, m)?)?;
    m.add_function(wrap_pyfunction!(run_probes, m)?)?;
    m.add_function(wrap_pyfunction!(model_hash, m)?)?;
    m.add_function(wrap_pyfunction!(run_schema, m)?)?;
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(render_diagram, m)?)?;
    m.add_function(wrap_pyfunction!(start_from, m)?)?;
    m.add_function(wrap_pyfunction!(group, m)?)?;
    m.add_function(wrap_pyfunction!(upgrade, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
