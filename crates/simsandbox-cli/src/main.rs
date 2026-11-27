//! `simsandbox` — the headless interface (ADR 0039): run, check, describe
//! and render models without the window. Everything an agent needs is
//! machine-readable: `describe`, `schema` and `check` print JSON, `run`
//! prints CSV or JSON, `render` writes PNG.
//!
//! `simsandbox --help` lists the commands, `simsandbox COMMAND --help`
//! gives one's options ([`USAGE`], [`COMMANDS`]). How a command fails sets
//! the exit status, and `--errors json` writes failures, warnings and notes
//! as one JSON object per line (ADR 0099, [`Failure`]).

use serde_json::{json, Map, Value};
use simsandbox_core::block::MethodLabels;
use simsandbox_session::examples::{registry, view_registry};
use simsandbox_session::render::{png, render_diagram, render_run, RenderOptions};
use simsandbox_session::run::{run_keeping, run_probes, start_from, Keep};
use simsandbox_session::runs::{self, is_run_file};
use simsandbox_session::session::InputEvent;
use simsandbox_session::{describe, probe};
use std::collections::BTreeMap;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

const USAGE: &str = "usage: simsandbox [--errors text|json] COMMAND [ARGS]

Commands:
  run FILE          run a model, or replay a run file, and print its outputs (CSV or JSON)
  check FILE        check a model or run file and print a JSON report
  describe [TYPE]   every block, view and filter type and the units as JSON, or one type
  explain FILE      the model's card: its parts, equations, methods, parameters and views
  schema [run]      the model file's, or a run file's, JSON Schema
  render FILE       draw the model's views, or its block diagram, to a PNG
  start-from FILE   the model file that starts from the state reached at a time
  upgrade FILE      the file in the current form of the format
  help [COMMAND]    this text, or a command's options (also: COMMAND --help)

Options (anywhere on the line):
  --errors text|json  how failures, warnings and notes go to standard error: as text
                      (default) or one JSON object per line
  -h, --help          this text
  -V, --version       the version

Exit status: 0 success; 1 the model cannot run, or the run failed; 2 a usage error
(an unknown command or flag, a bad or missing argument); 3 a file could not be read
or written.";

/// Each command's help: its synopsis (up to the first blank line, also
/// shown with a usage error), then what it does and its options.
const COMMANDS: &[(&str, &str)] = &[
    (
        "run",
        "usage: simsandbox run FILE [--t-end SECONDS] [--dt SECONDS] [--format csv|json] [--inputs RUN]
                      [--signals NAMES] [--every N] [--model NAME]
                      [--probe NAME] [--save-probes DIR] [--memory MB]

Run a model file and print every output at every step: CSV, or JSON with --format json.
FILE may be a run file (NAME.run.json), which is replayed (ADR 0059). Without --t-end
and --dt, the file's `run` section. A model whose only runs are its studies gives one
row per run (ADR 0071). Notes, warnings and labels go to standard error.

  --t-end SECONDS    run to this time
  --dt SECONDS       the step; not for a model using models, each keeps its own clock
  --format csv|json  CSV (default; populations are left out) or JSON
  --inputs RUN       replay a run file's recorded events on this model file
  --signals NAMES    only these outputs, comma-separated: block.port, block, or in a model
                     using models a model (`.` the top level) or a path in it (ADR 0076)
  --every N          only every N-th step, and the last (ADR 0076)
  --model NAME       in a model using models, this model's outputs at its own times
  --probe NAME       what this probe records, instead of every output (ADR 0062)
  --save-probes DIR  write the probes' files (their `to`) into DIR
  --memory MB        with probes, the app's memory budget (default 1024; ADR 0062)",
    ),
    (
        "check",
        "usage: simsandbox check FILE [--agent]

Check a model file, or a run file with its model, and print a JSON report on standard
output: `ok`, `errors` (each with its `path`, or `line` and `column`), `warnings`,
parameters, labels and what views can take. Exit status 1 if the model cannot run.

  --agent   also fail where a part an agent made (`\"agent\": true`) does not explain
            itself (ADR 0107): a model's description, a block's notes.purpose, a reason
            for each value set on it, a description for each parameter and input of
            typed equations; each an error at its path. Agents run this before handing
            a model over.",
    ),
    (
        "describe",
        "usage: simsandbox describe [TYPE | units]

Print, as JSON, every block, view and filter type and the units; or one block or view
type (its ports, parameters, methods and labels); or with `units` every unit symbol.",
    ),
    (
        "explain",
        "usage: simsandbox explain FILE [--format markdown|json] [--part PATH] [--depth N]

Print the model's card (ADR 0107), one level at a time (ADR 0110): what it is, its parts
by role (interaction, model, system, controller, formulas, study), each block's summary,
equations or bodies, method with its labels and what it is solved with, its parameters
grouped by role and its inputs and outputs; each model it uses as one part with its
interface for that use (parameters, inputs, outputs) and what it holds; how the parts
connect, wires between two parts as one bundle; the key parameters (those the model
declares, then those it pins, else those its types suggest); what each view shows; what
reads inside a used model by path; and how far to trust it. Markdown (default) or JSON.

  --part PATH   the card of a model used inside, by its path of uses (car1, vehicle/plant)
  --depth N     include the cards of used models N - 1 levels down (default 1)

Exit status 1 if the model cannot load or has no such part.",
    ),
    (
        "schema",
        "usage: simsandbox schema [run]

Print the model file's JSON Schema, or with `run` the run file's.",
    ),
    (
        "render",
        "usage: simsandbox render FILE --out IMAGE.png [--t-end SECONDS] [--view N] [--inputs RUN]
                         [--views VIEWS.ssbv.json] [--width POINTS] [--height POINTS] [--scale S] [--light]
       simsandbox render FILE --out IMAGE.png --diagram [--inside PATH] [--expand PATHS]
                         [--no-data] [--values] [--components] [--arrange] [--width POINTS] [--height POINTS] [--scale S] [--light]

Run a model file to a time and draw its views to a PNG, placed as the file's layout
arranges them; or with --diagram draw its block diagram (also for a file that cannot run).

  --out IMAGE.png   the image to write (required)
  --t-end SECONDS   run to this time before drawing
  --view N          only the view at this index
  --inputs RUN      replay a run file's recorded events on this model file
  --views FILE      draw this view file's views (and templates) in place of the model's
                    (ADR 0113); a run file keeps its identity, as views change no result
  --width, --height the size in points; --scale pixels per point
  --light           the light theme
  --diagram         the block diagram instead of the views
  --inside PATH     with --diagram, the inside of the model used at PATH (car1, vehicle/plant)
  --expand PATHS    with --diagram, these uses (comma-separated) expanded in place
  --no-data         with --diagram, without the data layer (filters, views, probes)
  --components      with --diagram, components: each node its header, the wires between
                    two nodes one bundle named by its ports' group or count
  --arrange         with --diagram, the nodes placed by role (interaction left, the plant in
                    the middle, controllers below it, readers right); the file is unchanged
  --values          with --diagram, every node's values",
    ),
    (
        "start-from",
        "usage: simsandbox start-from FILE --t SECONDS [--out NEW.json]

Run a model file to time t and write the model file that starts from the state reached
there (ADR 0042): to --out, or standard output.",
    ),
    (
        "upgrade",
        "usage: simsandbox upgrade FILE [--out NEW.json]

Write the model file in the current form of the format (ADR 0044): to --out, or standard
output. What was converted or dropped goes to standard error.",
    ),
    (
        "help",
        "usage: simsandbox help [COMMAND]

The list of commands, or one command's options.",
    ),
];

/// A command's help, if it is one.
fn help_of(command: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(n, _)| *n == command)
        .map(|(_, h)| *h)
}

/// The synopsis a usage error shows: the command's, or the commands'.
fn synopsis_of(command: Option<&str>) -> &'static str {
    let text = command.and_then(help_of).unwrap_or(USAGE);
    text.split("\n\n").next().unwrap_or(text)
}

/// How a command failed, which sets the exit status (ADR 0099).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// The model cannot load, or `check` found errors: exit status 1.
    Model,
    /// The model loaded, but the run (or the drawing) failed: exit status 1.
    Run,
    /// An unknown command or flag, a bad or missing argument: exit status 2.
    Usage,
    /// A file could not be read or written: exit status 3.
    Io,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Model => "model",
            Kind::Run => "run",
            Kind::Usage => "usage",
            Kind::Io => "io",
        }
    }

    fn code(self) -> u8 {
        match self {
            Kind::Model | Kind::Run => 1,
            Kind::Usage => 2,
            Kind::Io => 3,
        }
    }
}

/// A failure: its kind, its message as text mode prints it, and where it
/// is (`file`, and from `check` its `errors`), for `--errors json`.
#[derive(Debug)]
struct Failure {
    kind: Kind,
    message: String,
    fields: Map<String, Value>,
}

impl Failure {
    fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            fields: Map::new(),
        }
    }

    fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, message)
    }

    fn io(message: impl Into<String>, file: &str) -> Self {
        Self::new(Kind::Io, message).with("file", file)
    }

    fn model(message: impl Into<String>, file: &str) -> Self {
        Self::new(Kind::Model, message).with("file", file)
    }

    fn run(message: impl Into<String>, file: &str) -> Self {
        Self::new(Kind::Run, message).with("file", file)
    }

    fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_string(), value.into());
        self
    }

    /// The object `--errors json` prints.
    fn to_json(&self) -> Value {
        let mut e = Map::new();
        e.insert("kind".into(), self.kind.name().into());
        e.insert("message".into(), self.message.clone().into());
        e.extend(self.fields.clone());
        json!({ "error": e })
    }
}

type Result<T> = std::result::Result<T, Failure>;

/// Whether standard error takes one JSON object per line (`--errors json`).
static JSON_ERRORS: AtomicBool = AtomicBool::new(false);

fn json_errors() -> bool {
    JSON_ERRORS.load(Ordering::Relaxed)
}

/// One line on standard error: `text` as text, `object` with `--errors json`.
fn say(text: impl FnOnce() -> String, object: impl FnOnce() -> Value) {
    if json_errors() {
        eprintln!("{}", object());
    } else {
        eprintln!("{}", text());
    }
}

/// A problem worked around; `file` is the file it is in, if named.
fn warning(file: Option<&str>, message: &str) {
    say(
        || match file {
            Some(f) => format!("warning: {f}: {message}"),
            None => format!("warning: {message}"),
        },
        || match file {
            Some(f) => json!({"warning": {"message": message, "file": f}}),
            None => json!({"warning": {"message": message}}),
        },
    );
}

/// Something worth knowing; `model` names the model in a model using
/// models.
fn note(model: Option<&str>, message: &str) {
    say(
        || match model {
            Some(m) => format!("note: {m}: {message}"),
            None => format!("note: {message}"),
        },
        || match model {
            Some(m) => json!({"note": {"message": message, "model": m}}),
            None => json!({"note": {"message": message}}),
        },
    );
}

/// The speed and accuracy labels of a run's methods.
fn labels(l: MethodLabels) {
    say(
        || format!("speed: {:?} | accuracy: {:?}", l.speed, l.accuracy),
        || json!({ "labels": describe::labels(l) }),
    );
}

/// A file written.
fn wrote(path: &std::path::Path) {
    say(
        || format!("wrote {}", path.display()),
        || json!({"wrote": {"file": path.display().to_string()}}),
    );
}

/// An older form of the format, converted on reading.
fn converted(message: &str) {
    say(
        || format!("converted: {message}"),
        || json!({"converted": {"message": message}}),
    );
}

/// Flags and their values; `--light` and `--diagram` take none.
struct Flags(BTreeMap<String, String>);

impl Flags {
    /// Parse `--flag value` pairs, accepting only `allowed`.
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self> {
        let mut map = BTreeMap::new();
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            if !allowed.contains(&flag.as_str()) {
                return Err(Failure::usage(if flag.starts_with('-') {
                    format!("unknown flag '{flag}'")
                } else {
                    format!("unexpected argument '{flag}'")
                }));
            }
            let value = if flag == "--light"
                || flag == "--agent"
                || flag == "--diagram"
                || flag == "--no-data"
                || flag == "--values"
                || flag == "--components"
                || flag == "--arrange"
            {
                String::new()
            } else {
                it.next()
                    .ok_or_else(|| Failure::usage(format!("missing value for {flag}")))?
                    .clone()
            };
            map.insert(flag.clone(), value);
        }
        Ok(Self(map))
    }

    fn get(&self, flag: &str) -> Option<&str> {
        self.0.get(flag).map(String::as_str)
    }

    fn has(&self, flag: &str) -> bool {
        self.0.contains_key(flag)
    }

    /// A positive number.
    fn positive(&self, flag: &str) -> Result<Option<f64>> {
        let Some(v) = self.get(flag) else {
            return Ok(None);
        };
        match v.parse::<f64>() {
            Ok(x) if x.is_finite() && x > 0.0 => Ok(Some(x)),
            _ => Err(Failure::usage(format!(
                "{flag} must be a positive number, not '{v}'"
            ))),
        }
    }
}

/// The command's file: its first argument.
fn file_arg<'a>(command: &str, args: &'a [String]) -> Result<(&'a String, &'a [String])> {
    match args.split_first() {
        Some((path, rest)) if !path.starts_with("--") => Ok((path, rest)),
        _ => Err(Failure::usage(format!("{command} needs a FILE"))),
    }
}

fn read(path: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Failure::io(format!("{path}: {e}"), path))
}

/// Fail with an input/output error unless `path` can be read, and, for a
/// run file, the model it names.
fn readable(path: &str) -> Result<()> {
    read(path)?;
    if is_run_file(path) {
        if let Err(e @ runs::ReadError::Model { .. }) = runs::read_run(std::path::Path::new(path)) {
            let runs::ReadError::Model { model, .. } = &e else {
                unreachable!()
            };
            return Err(Failure::io(e.to_string(), &model.display().to_string()));
        }
    }
    Ok(())
}

fn write_file(path: &str, bytes: impl AsRef<[u8]>) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| Failure::io(format!("{path}: {e}"), path))
}

/// Writing to standard output failed.
fn stdout_failed(e: impl std::fmt::Display) -> Failure {
    Failure::new(Kind::Io, e.to_string()).with("file", "-")
}

/// The model text to run and the recorded input to replay with it
/// ([`runs::model_and_track`]), its warnings on standard error.
fn model_and_track(path: &str, inputs: Option<&str>) -> Result<(String, Vec<InputEvent>)> {
    readable(path)?;
    if let Some(inputs) = inputs {
        if is_run_file(path) {
            return Err(Failure::usage(
                "--inputs applies a run to a model file; this is a run file",
            ));
        }
        readable(inputs)?;
    }
    let r =
        runs::model_and_track(path, inputs, &registry()).map_err(|e| Failure::model(e, path))?;
    for w in &r.warnings {
        warning(None, w);
    }
    Ok((r.text, r.track))
}

fn print_json(v: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(v).map_err(stdout_failed)?;
    println!("{text}");
    Ok(())
}

/// Run a model file and print every output (see
/// [`simsandbox_session::run`]) as CSV (without populations) or JSON.
fn run_file(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("run", args)?;
    let flags = Flags::parse(
        rest,
        &[
            "--t-end",
            "--dt",
            "--format",
            "--inputs",
            "--probe",
            "--save-probes",
            "--model",
            "--signals",
            "--every",
            "--memory",
        ],
    )?;
    let mut keep = keep_of(&flags)?;
    let json = match flags.get("--format") {
        None | Some("csv") => false,
        Some("json") => true,
        Some(other) => {
            return Err(Failure::usage(format!(
                "unknown format '{other}': csv or json"
            )))
        }
    };
    let (t_end, dt) = (flags.positive("--t-end")?, flags.positive("--dt")?);
    readable(path)?;
    // A model using models (ADR 0066): each model at its own times; with
    // inputs or probes, as one model on several clocks, as the app runs it
    // (step 5).
    let composed = if is_run_file(path) {
        runs::read_run(std::path::Path::new(path))
            .is_ok_and(|r| simsandbox_file::compose::is_composition(&r.text))
    } else {
        simsandbox_file::compose::is_composition(&read(path)?)
    };
    if composed && flags.has("--dt") {
        return Err(Failure::usage(
            "--dt does not apply to a model using models: each keeps its own clock",
        ));
    }
    let merged = flags.has("--inputs") || flags.has("--probe") || flags.has("--save-probes");
    if composed && !merged && !is_run_file(path) {
        // One model printed: only it recorded, unless signals say what.
        if let (Some(m), None) = (flags.get("--model"), &keep.signals) {
            keep.signals = Some(vec![m.to_string()]);
        }
        return run_composition_file(path, &read(path)?, t_end, &flags, json, &keep);
    }
    if composed && flags.has("--model") {
        return Err(Failure::usage("--model picks one model's own times; with inputs, probes or a run file the models run as one: drop --model"));
    }
    if flags.has("--model") && !composed {
        return Err(Failure::usage(
            "--model picks a model on its own clock (ADR 0066); this model uses no model files",
        ));
    }
    if (flags.has("--probe") || flags.has("--save-probes"))
        && (flags.has("--signals") || flags.has("--every"))
    {
        return Err(Failure::usage(
            "--signals and --every choose what run prints; a probe records what it is set to: drop one",
        ));
    }
    let (text, track) = model_and_track(path, flags.get("--inputs"))?;
    if flags.has("--probe") || flags.has("--save-probes") {
        return run_file_probes(path, &text, track, (t_end, dt), &flags, json);
    }
    let r = run_keeping(&text, &registry(), t_end, dt, track, &keep)
        .map_err(|e| Failure::run(format!("{path}: {e}"), path))?;
    for w in &r.warnings {
        warning(Some(path), w);
    }
    if let Some(l) = r.labels {
        labels(l);
    }
    for n in &r.notes {
        note(None, n);
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let written = if json {
        r.write_json(&mut out).and_then(|()| writeln!(out))
    } else {
        for p in r.populations.iter().filter(|p| !p.rows) {
            note(
                None,
                &format!(
                    "{} is a population (variable length) and is not written \
                     (--format json includes it)",
                    p.name
                ),
            );
        }
        r.write_csv(&mut out)
    };
    written.and_then(|()| out.flush()).map_err(stdout_failed)
}

/// `run` of a model that uses model files (ADR 0066): every model on its
/// own clock. JSON gives every one; CSV one, named by `--model` (`.` for
/// the top level's own blocks) when there are several.
fn run_composition_file(
    path: &str,
    text: &str,
    t_end: Option<f64>,
    flags: &Flags,
    json: bool,
    keep: &Keep,
) -> Result<()> {
    let c = simsandbox_session::compose::run_composition_keeping(
        text,
        &registry(),
        t_end,
        simsandbox_core::parallel::Parallel::Auto,
        keep,
    )
    .map_err(|e| Failure::run(format!("{path}: {e}"), path))?;
    for w in &c.warnings {
        warning(Some(path), w);
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let written = if json && !flags.has("--model") {
        c.write_json(&mut out).and_then(|()| writeln!(out))
    } else {
        let names: Vec<&str> = c.models.iter().map(|(n, _)| n.as_str()).collect();
        let name = match (flags.get("--model"), names.as_slice()) {
            (Some(n), _) => n,
            (None, [one]) => one,
            (None, _) => {
                return Err(Failure::usage(format!(
                    "{path} has several models, each at its own times: pick one with --model ({}) or use --format json",
                    names.join(", ")
                )))
            }
        };
        let r = c.model(name).ok_or_else(|| {
            Failure::usage(format!("no model '{name}'; models: {}", names.join(", ")))
        })?;
        for n in &r.notes {
            note(Some(name), n);
        }
        if json {
            r.write_json(&mut out).and_then(|()| writeln!(out))
        } else {
            r.write_csv(&mut out)
        }
    };
    written.and_then(|()| out.flush()).map_err(stdout_failed)
}

/// What `run` keeps (ADR 0076): `--signals a.x,b` and `--every N`.
fn keep_of(flags: &Flags) -> Result<Keep> {
    let signals = flags.get("--signals").map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .collect::<Vec<_>>()
    });
    if signals.as_ref().is_some_and(Vec::is_empty) {
        return Err(Failure::usage("--signals needs at least one name"));
    }
    let every = match flags.get("--every") {
        None => 1,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n >= 1 => n,
            _ => {
                return Err(Failure::usage(format!(
                    "--every must be a whole number of steps, 1 or more, not '{v}'"
                )))
            }
        },
    };
    Ok(Keep { signals, every })
}

/// `run` with probes (ADR 0062): print one (`--probe`), write their files
/// (`--save-probes DIR`), or both.
fn run_file_probes(
    path: &str,
    text: &str,
    track: Vec<InputEvent>,
    (t_end, dt): (Option<f64>, Option<f64>),
    flags: &Flags,
    json: bool,
) -> Result<()> {
    let names: Vec<String> = flags.get("--probe").map(String::from).into_iter().collect();
    let dir = flags.get("--save-probes").map(std::path::Path::new);
    // The app's memory budget, in MB (ADR 0062 step 5).
    let memory = match flags.get("--memory") {
        None => None,
        Some(v) => Some(
            v.parse::<f64>()
                .map_err(|_| Failure::usage(format!("--memory takes megabytes, not '{v}'")))?,
        ),
    };
    let r = run_probes(text, &registry(), t_end, dt, track, &names, dir, memory)
        .map_err(|e| Failure::run(format!("{path}: {e}"), path))?;
    for w in &r.warnings {
        warning(Some(path), w);
    }
    if let Some(l) = r.labels {
        labels(l);
    }
    if let Some(dir) = flags.get("--save-probes") {
        let (written, notes) = r
            .write_files(std::path::Path::new(dir))
            .map_err(|e| Failure::io(format!("{dir}: {e}"), dir))?;
        for n in notes {
            note(None, &n);
        }
        if written.is_empty() {
            note(None, "no probe has a file ('to'); nothing written");
        }
        for w in written {
            wrote(&w);
        }
    }
    let Some((_, h)) = r.probes.first().filter(|_| flags.has("--probe")) else {
        return Ok(());
    };
    let h = h.read();
    let mut out = BufWriter::new(io::stdout().lock());
    let written = if json {
        serde_json::to_writer(&mut out, &probe::to_json(&h))
            .map_err(io::Error::other)
            .and_then(|()| writeln!(out))
    } else {
        for f in h.fields() {
            note(
                None,
                &format!(
                "{} is a field and is not written as CSV (--format json or --save-probes include it)",
                f.name
            ),
            );
        }
        probe::write_csv(&h, &mut out)
    };
    written.and_then(|()| out.flush()).map_err(stdout_failed)
}

/// The `check` report of a model or run file; with `agent`, also what
/// agents must write for what they made (ADR 0107).
fn check_report(path: &str, agent: bool) -> Result<Value> {
    Ok(if is_run_file(path) {
        runs::check_run(path, &registry())
    } else if path.ends_with(simsandbox_file::templates::VIEW_FILE_EXTENSION) {
        // A view file (ADR 0113): its templates and views read; what they
        // bind is checked where it is applied.
        match simsandbox_file::templates::read_view_file(&read(path)?) {
            Ok(v) => {
                let as_model = simsandbox_file::ModelFile {
                    templates: v.templates.clone(),
                    ..Default::default()
                };
                serde_json::json!({
                    "ok": true,
                    "errors": [],
                    "warnings": [],
                    "view_file": true,
                    "description": v.description,
                    "templates": describe::templates(&as_model).0,
                    "views": v.views.len(),
                })
            }
            Err(e) => serde_json::json!({"ok": false, "errors": [{"message": e}], "warnings": []}),
        }
    } else {
        describe::check_with(&read(path)?, &registry(), agent)
    })
}

/// Check a model file and print the findings as JSON. Fails (exit status
/// 1) if the model cannot run.
fn check(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("check", args)?;
    let flags = Flags::parse(rest, &["--agent"])?;
    readable(path)?;
    let report = check_report(path, flags.has("--agent"))?;
    print_json(&report)?;
    if report["ok"] == true {
        Ok(())
    } else {
        Err(
            Failure::model(format!("{path} cannot run (see \"errors\")"), path)
                .with("errors", report["errors"].clone()),
        )
    }
}

/// Print a model's card (ADR 0107), as Markdown or JSON.
fn explain(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("explain", args)?;
    let flags = Flags::parse(rest, &["--format", "--part", "--depth"])?;
    let part = flags.get("--part").unwrap_or("").trim_matches('/');
    let depth = match flags.get("--depth") {
        None => 1,
        Some(d) => d.parse::<usize>().ok().filter(|&d| d >= 1).ok_or_else(|| {
            Failure::usage(format!("--depth is a whole number from 1, not '{d}'"))
        })?,
    };
    let json = match flags.get("--format") {
        None | Some("markdown") => false,
        Some("json") => true,
        Some(other) => {
            return Err(Failure::usage(format!(
                "--format is markdown or json, not '{other}'"
            )))
        }
    };
    let text = read(path)?;
    let title = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let title = title
        .strip_suffix(".ssbx.json")
        .unwrap_or(&title)
        .to_string();
    let session = simsandbox_session::Session::load_in(
        &title,
        &text,
        &registry(),
        model_folder(path).as_deref(),
    )
    .map_err(|e| Failure::model(format!("{path} cannot load: {e}"), path))?;
    let r = registry();
    let card = simsandbox_session::card::card_at(
        &session,
        &|t| r.info(t).map(|i| i.summary.clone()),
        part,
        depth,
    )
    .map_err(|e| Failure::model(format!("{path}: --part {part}: {e}"), path))?;
    if json {
        print_json(&card)
    } else {
        print!("{}", simsandbox_session::card::markdown(&card));
        Ok(())
    }
}

/// Print the catalogue, or one block or view type.
fn describe_types(args: &[String]) -> Result<()> {
    let r = registry();
    let views = view_registry();
    match args {
        [] => print_json(&describe::catalogue(&r, &views)),
        [name] => {
            if name == "units" {
                print_json(&describe::units())
            } else if r.info(name).is_some() {
                print_json(&describe::block_type(&r, name))
            } else if let Some(t) = views.get(name) {
                print_json(&describe::view_type(t))
            } else {
                let known: Vec<&str> = r
                    .type_names()
                    .chain(views.types().iter().map(|t| t.name))
                    .collect();
                Err(Failure::usage(format!(
                    "unknown type '{name}'; known: {}",
                    known.join(", ")
                )))
            }
        }
        _ => Err(Failure::usage("describe takes at most one TYPE")),
    }
}

fn schema(args: &[String]) -> Result<()> {
    match args {
        [] => print!("{}", simsandbox_file::SCHEMA),
        [kind] if kind == "run" => print!("{}", simsandbox_file::runs::RUN_SCHEMA),
        _ => return Err(Failure::usage("schema takes nothing, or `run`")),
    }
    Ok(())
}

/// Run a model file to a time and draw its views to a PNG.
fn render_file(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("render", args)?;
    let flags = Flags::parse(
        rest,
        &[
            "--out",
            "--t-end",
            "--view",
            "--width",
            "--height",
            "--scale",
            "--light",
            "--diagram",
            "--subsystem",
            "--inside",
            "--expand",
            "--no-data",
            "--values",
            "--components",
            "--arrange",
            "--inputs",
            "--views",
        ],
    )?;
    let out = flags
        .get("--out")
        .ok_or_else(|| Failure::usage("render needs --out IMAGE.png"))?;
    let defaults = RenderOptions::default();
    let size = |flag: &str, default: f32| -> Result<f32> {
        Ok(flags.positive(flag)?.map_or(default, |v| v as f32))
    };
    if (flags.has("--inside") || flags.has("--subsystem") || flags.has("--expand"))
        && !flags.has("--diagram")
    {
        return Err(Failure::usage(
            "--inside draws a used model's diagram: add --diagram",
        ));
    }
    if flags.has("--components") && !flags.has("--diagram") {
        return Err(Failure::usage(
            "--components draws the diagram's components: add --diagram",
        ));
    }
    if flags.has("--arrange") && !flags.has("--diagram") {
        return Err(Failure::usage(
            "--arrange places the diagram's nodes by role: add --diagram",
        ));
    }
    if flags.has("--values") && !flags.has("--diagram") {
        return Err(Failure::usage(
            "--values shows values on the diagram: add --diagram",
        ));
    }
    if flags.has("--diagram") && (flags.has("--t-end") || flags.has("--view")) {
        return Err(Failure::usage(
            "--t-end and --view do not apply to --diagram",
        ));
    }
    let (text, track) = model_and_track(path, flags.get("--inputs"))?;
    // Another set of views on the same model or run (ADR 0113 point 8).
    let text = match flags.get("--views") {
        None => text,
        Some(views) => {
            let read = simsandbox_file::templates::read_view_file(&read(views)?)
                .map_err(|e| Failure::model(format!("{views}: {e}"), views))?;
            simsandbox_file::templates::apply_views(&text, &read, true)
                .map_err(|e| Failure::model(format!("{views}: {e}"), views))?
        }
    };
    if flags.has("--diagram") {
        // The diagram is drawn even if the model cannot run.
        let (file, _) = simsandbox_file::ModelFile::from_json(&text)
            .map_err(|e| Failure::model(format!("{path}: {e}"), path))?;
        let o = RenderOptions {
            width: size("--width", defaults.width)?,
            height: size("--height", defaults.height)?,
            scale: size("--scale", defaults.scale)?,
            view: None,
            light: flags.has("--light"),
            // `--subsystem` is the older name (ADR 0068).
            inside: flags
                .get("--inside")
                .or_else(|| flags.get("--subsystem"))
                .map(str::to_string),
            // Uses expanded in place, comma-separated paths (ADR 0066).
            expand: flags
                .get("--expand")
                .map(|v| v.split(',').map(|p| p.trim().to_string()).collect())
                .unwrap_or_default(),
            data: !flags.has("--no-data"),
            values: flags.has("--values"),
            components: flags.has("--components"),
            arrange: flags.has("--arrange"),
        };
        let image = render_diagram(&file, &registry(), &o).map_err(|e| Failure::run(e, path))?;
        return write_file(out, png(&image).map_err(|e| Failure::run(e, path))?);
    }
    let t_end = flags.positive("--t-end")?;
    let o = RenderOptions {
        width: size("--width", defaults.width)?,
        height: size("--height", defaults.height)?,
        scale: size("--scale", defaults.scale)?,
        view: flags
            .get("--view")
            .map(|v| {
                v.parse()
                    .map_err(|_| Failure::usage(format!("--view takes a number, not '{v}'")))
            })
            .transpose()?,
        light: flags.has("--light"),
        inside: None,
        expand: Vec::new(),
        data: false,
        values: false,
        components: false,
        arrange: false,
    };
    // Warnings reach standard error even if the run or the drawing fails.
    let mut warnings = Vec::new();
    let image = render_run(path, &text, &registry(), t_end, track, &o, &mut warnings);
    for w in warnings {
        warning(Some(path), &w);
    }
    let image = image.map_err(|e| Failure::run(e, path))?;
    write_file(out, png(&image).map_err(|e| Failure::run(e, path))?)
}

/// Run a model file to a time and write the file that starts from the
/// state reached there (ADR 0042): to `--out`, or standard output.
fn start_from_file(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("start-from", args)?;
    let flags = Flags::parse(rest, &["--t", "--out"])?;
    let t: f64 = flags
        .get("--t")
        .ok_or_else(|| Failure::usage("start-from needs --t SECONDS"))?
        .parse()
        .map_err(|_| Failure::usage("--t takes a number of seconds"))?;
    let (text, notes) = start_from(&read(path)?, &registry(), t)
        .map_err(|e| Failure::run(format!("{path}: {e}"), path))?;
    for n in notes {
        note(None, &n);
    }
    match flags.get("--out") {
        Some(out) => write_file(out, text + "\n"),
        None => {
            println!("{text}");
            Ok(())
        }
    }
}

/// Write a model file in the current form (ADR 0044): to `--out`, or
/// standard output. What was converted or dropped goes to standard error.
fn upgrade_file(args: &[String]) -> Result<()> {
    let (path, rest) = file_arg("upgrade", args)?;
    let flags = Flags::parse(rest, &["--out"])?;
    let (text, conversions, warnings) = describe::upgrade(&read(path)?, &registry())
        .map_err(|e| Failure::model(format!("{path}: {e}"), path))?;
    if conversions.is_empty() {
        note(None, &format!("{path} is already in the current form"));
    }
    for c in conversions {
        converted(&c);
    }
    for w in warnings {
        warning(Some(path), &w);
    }
    match flags.get("--out") {
        Some(out) => write_file(out, text),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

/// The folder of the model a command's file names (a run file's model):
/// where blocks read the files their settings name (ADR 0063).
fn model_folder(path: &str) -> Option<std::path::PathBuf> {
    let model = if is_run_file(path) {
        let run = runs::RunFile::from_json(&std::fs::read_to_string(path).ok()?).ok()?;
        runs::model_beside(std::path::Path::new(path), &run)
    } else {
        std::path::PathBuf::from(path)
    };
    let folder = model.parent()?;
    Some(if folder.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        folder.to_path_buf()
    })
}

/// Take `--errors text|json` (or `--errors=...`) out of the arguments,
/// wherever it is: whether failures are JSON.
fn errors_option(args: &mut Vec<String>) -> Result<bool> {
    let mut json = false;
    while let Some(k) = args
        .iter()
        .position(|a| a == "--errors" || a.starts_with("--errors="))
    {
        let value = match args[k].strip_prefix("--errors=") {
            Some(v) => {
                let v = v.to_string();
                args.remove(k);
                v
            }
            None if k + 1 < args.len() => args.drain(k..k + 2).nth(1).unwrap_or_default(),
            None => {
                args.remove(k);
                String::new()
            }
        };
        json = match value.as_str() {
            "json" => true,
            "text" => false,
            other => {
                return Err(Failure::usage(format!(
                    "--errors takes text or json, not '{other}'"
                )))
            }
        };
    }
    Ok(json)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let result = match errors_option(&mut args) {
        Ok(json) => {
            JSON_ERRORS.store(json, Ordering::Relaxed);
            let rest = args.get(1..).unwrap_or(&[]);
            let folder = rest.first().and_then(|p| model_folder(p));
            simsandbox_core::registry::in_folder(folder.as_deref(), || {
                command(&args, rest).map_err(|f| with_check_errors(f, &args))
            })
        }
        Err(f) => Err(f),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) if args.is_empty() && !json_errors() => {
            eprintln!("{USAGE}");
            ExitCode::from(f.kind.code())
        }
        Err(f) => {
            report(&f, args.first().map(String::as_str));
            ExitCode::from(f.kind.code())
        }
    }
}

/// With `--errors json`, a failed run or drawing carries the model's
/// `check` errors when it has any, and is then a model failure.
fn with_check_errors(mut f: Failure, args: &[String]) -> Failure {
    let command = args.first().map(String::as_str);
    if !json_errors()
        || f.kind != Kind::Run
        || !matches!(command, Some("run" | "render" | "start-from"))
    {
        return f;
    }
    let Some(path) = f
        .fields
        .get("file")
        .and_then(Value::as_str)
        .map(String::from)
    else {
        return f;
    };
    if let Ok(report) = check_report(&path, false) {
        if report["ok"] != true {
            f.kind = Kind::Model;
            f.fields.insert("errors".into(), report["errors"].clone());
        }
    }
    f
}

/// Write a failure to standard error, as text or JSON.
fn report(f: &Failure, command: Option<&str>) {
    let command = command.filter(|c| help_of(c).is_some());
    if json_errors() {
        let mut v = f.to_json();
        if f.kind == Kind::Usage {
            v["error"]["usage"] = synopsis_of(command).into();
        }
        eprintln!("{v}");
    } else if f.kind == Kind::Usage {
        eprintln!("error: {}\n{}", f.message, synopsis_of(command));
        match command {
            Some(c) => eprintln!("(simsandbox {c} --help for its options)"),
            None => eprintln!("(simsandbox --help for the commands)"),
        }
    } else {
        eprintln!("error: {}", f.message);
    }
}

fn command(args: &[String], rest: &[String]) -> Result<()> {
    let Some(first) = args.first().map(String::as_str) else {
        // No command: the usage, on standard error, as a usage error.
        return Err(Failure::usage("no command"));
    };
    if let Some(help) = help_of(first) {
        if first != "help" && rest.iter().any(|a| a == "--help" || a == "-h") {
            println!("{help}");
            return Ok(());
        }
    }
    match first {
        "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        "help" => match rest {
            [] => {
                println!("{USAGE}");
                Ok(())
            }
            [c] => match help_of(c) {
                Some(h) => {
                    println!("{h}");
                    Ok(())
                }
                None => Err(Failure::usage(format!("no command '{c}'"))),
            },
            _ => Err(Failure::usage("help takes at most one COMMAND")),
        },
        "--version" | "-V" => {
            println!("simsandbox {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "run" => run_file(rest),
        "check" => check(rest),
        "describe" => describe_types(rest),
        "explain" => explain(rest),
        "schema" => schema(rest),
        "render" => render_file(rest),
        "start-from" => start_from_file(rest),
        "upgrade" => upgrade_file(rest),
        other => Err(Failure::usage(format!("unknown command '{other}'"))),
    }
}
