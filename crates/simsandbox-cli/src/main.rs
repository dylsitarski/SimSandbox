//! `simsandbox` — the headless interface (ADR 0039): run, check, describe
//! and render models without the window. Everything an agent needs is
//! machine-readable: `describe`, `schema` and `check` print JSON, `run`
//! prints CSV or JSON, `render` writes PNG.
//!
//! ```text
//! simsandbox run FILE [--t-end SECONDS] [--dt SECONDS] [--format csv|json]
//!                                   (default: the file's `run` section)
//!                                   (a model with no states: one row per sample of its sweep)
//! simsandbox check FILE             (JSON; exit status 1 if the model cannot run)
//! simsandbox describe [TYPE]        (JSON: every block, view and filter type and the units, or one; TYPE "units" lists the units)
//! simsandbox schema                 (the model file's JSON Schema)
//! simsandbox render FILE --out IMAGE.png [--t-end SECONDS] [--view N]
//!                   [--width POINTS] [--height POINTS] [--scale S] [--light]
//!                   [--diagram [--subsystem NAME]]  (the block diagram instead of the views)
//! simsandbox start-from FILE --t SECONDS [--out NEW.json]
//!                                   (the model file that starts from the state at t)
//! simsandbox upgrade FILE [--out NEW.json]
//!                                   (the file in the current form; older forms converted)
//! simsandbox lorenz [--t-end 20] [--dt 0.01] [--method euler|rk4|dopri5] [--rho 28]
//! simsandbox --version
//! ```

use serde_json::Value;
use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::systems::Lorenz;
use simsandbox_eq::{Integrator, OdeBlock};
use simsandbox_session::examples::{registry, view_registry};
use simsandbox_session::render::{png, render, render_diagram, RenderOptions};
use simsandbox_session::run::{run, start_from, DEFAULT_T_END};
use simsandbox_session::{describe, Session};
use std::collections::BTreeMap;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "usage:
  simsandbox run FILE [--t-end SECONDS] [--dt SECONDS] [--format csv|json]
  simsandbox check FILE
  simsandbox describe [TYPE | units]
  simsandbox schema
  simsandbox render FILE --out IMAGE.png [--t-end SECONDS] [--view N] [--width POINTS] [--height POINTS] [--scale S] [--light] [--diagram [--subsystem NAME]]
  simsandbox start-from FILE --t SECONDS [--out NEW.json]
  simsandbox upgrade FILE [--out NEW.json]
  simsandbox lorenz [--t-end SECONDS] [--dt SECONDS] [--method euler|rk4|dopri5] [--rho VALUE]";

/// Flags and their values; `--light` and `--diagram` take none.
struct Flags(BTreeMap<String, String>);

impl Flags {
    /// Parse `--flag value` pairs, accepting only `allowed`.
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            if !allowed.contains(&flag.as_str()) {
                return Err(format!("unknown flag '{flag}'\n{USAGE}"));
            }
            let value = if flag == "--light" || flag == "--diagram" {
                String::new()
            } else {
                it.next()
                    .ok_or(format!("missing value for {flag}"))?
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
    fn positive(&self, flag: &str) -> Result<Option<f64>, String> {
        let Some(v) = self.get(flag) else {
            return Ok(None);
        };
        match v.parse::<f64>() {
            Ok(x) if x.is_finite() && x > 0.0 => Ok(Some(x)),
            _ => Err(format!("{flag} must be a positive number, not '{v}'")),
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn print_json(v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

/// Run a model file and print every output (see
/// [`simsandbox_session::run`]) as CSV (without populations) or JSON.
fn run_file(args: &[String]) -> Result<(), String> {
    let (path, rest) = args.split_first().ok_or(USAGE)?;
    let flags = Flags::parse(rest, &["--t-end", "--dt", "--format"])?;
    let json = match flags.get("--format") {
        None | Some("csv") => false,
        Some("json") => true,
        Some(other) => return Err(format!("unknown format '{other}': csv or json")),
    };
    let (t_end, dt) = (flags.positive("--t-end")?, flags.positive("--dt")?);
    let r = run(&read(path)?, &registry(), t_end, dt).map_err(|e| format!("{path}: {e}"))?;
    for w in &r.warnings {
        eprintln!("warning: {path}: {w}");
    }
    if let Some(l) = r.labels {
        eprintln!("speed: {:?} | accuracy: {:?}", l.speed, l.accuracy);
    }
    for n in &r.notes {
        eprintln!("note: {n}");
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let written = if json {
        serde_json::to_writer(&mut out, &r.to_json())
            .map_err(io::Error::other)
            .and_then(|()| writeln!(out))
    } else {
        for p in &r.populations {
            eprintln!(
                "note: {} is a population (variable length) and is not written \
                 (--format json includes it)",
                p.name
            );
        }
        r.write_csv(&mut out)
    };
    written
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}

/// Check a model file and print the findings as JSON. Fails (exit status
/// 1) if the model cannot run.
fn check(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err(USAGE.into());
    };
    let report = describe::check(&read(path)?, &registry());
    print_json(&report)?;
    if report["ok"] == true {
        Ok(())
    } else {
        Err(format!("{path} cannot run (see \"errors\")"))
    }
}

/// Print the catalogue, or one block or view type.
fn describe_types(args: &[String]) -> Result<(), String> {
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
                Err(format!(
                    "unknown type '{name}'; known: {}",
                    known.join(", ")
                ))
            }
        }
        _ => Err(USAGE.into()),
    }
}

/// Run a model file to a time and draw its views to a PNG.
fn render_file(args: &[String]) -> Result<(), String> {
    let (path, rest) = args.split_first().ok_or(USAGE)?;
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
        ],
    )?;
    let out = flags.get("--out").ok_or("render needs --out IMAGE.png")?;
    let text = read(path)?;
    let defaults = RenderOptions::default();
    let size = |flag: &str, default: f32| -> Result<f32, String> {
        Ok(flags.positive(flag)?.map_or(default, |v| v as f32))
    };
    if flags.has("--subsystem") && !flags.has("--diagram") {
        return Err("--subsystem draws a subsystem's diagram: add --diagram".into());
    }
    if flags.has("--diagram") {
        if flags.has("--t-end") || flags.has("--view") {
            return Err("--t-end and --view do not apply to --diagram".into());
        }
        // The diagram is drawn even if the model cannot run.
        let (file, _) =
            simsandbox_file::ModelFile::from_json(&text).map_err(|e| format!("{path}: {e}"))?;
        let o = RenderOptions {
            width: size("--width", defaults.width)?,
            height: size("--height", defaults.height)?,
            scale: size("--scale", defaults.scale)?,
            view: None,
            light: flags.has("--light"),
            subsystem: flags.get("--subsystem").map(str::to_string),
        };
        let image = render_diagram(&file, &registry(), &o)?;
        return std::fs::write(out, png(&image)?).map_err(|e| format!("{out}: {e}"));
    }
    let mut s = Session::load(path, &text, &registry())?;
    for w in &s.warnings {
        eprintln!("warning: {path}: {w}");
    }
    let t_end = flags
        .positive("--t-end")?
        .or(s.file.run.t_end)
        .unwrap_or(DEFAULT_T_END);
    s.run_until(t_end).map_err(|e| e.to_string())?;
    let o = RenderOptions {
        width: flags
            .positive("--width")?
            .map_or(defaults.width, |v| v as f32),
        height: flags
            .positive("--height")?
            .map_or(defaults.height, |v| v as f32),
        scale: flags
            .positive("--scale")?
            .map_or(defaults.scale, |v| v as f32),
        view: flags
            .get("--view")
            .map(|v| {
                v.parse()
                    .map_err(|_| format!("--view takes a number, not '{v}'"))
            })
            .transpose()?,
        light: flags.has("--light"),
        subsystem: None,
    };
    let (image, warnings) = render(&s, view_registry(), &o)?;
    for w in warnings {
        eprintln!("warning: {path}: {w}");
    }
    std::fs::write(out, png(&image)?).map_err(|e| format!("{out}: {e}"))
}

/// Run a model file to a time and write the file that starts from the
/// state reached there (ADR 0042): to `--out`, or standard output.
fn start_from_file(args: &[String]) -> Result<(), String> {
    let (path, rest) = args.split_first().ok_or(USAGE)?;
    let flags = Flags::parse(rest, &["--t", "--out"])?;
    let t: f64 = flags
        .get("--t")
        .ok_or("start-from needs --t SECONDS")?
        .parse()
        .map_err(|_| "--t takes a number of seconds".to_string())?;
    let (text, notes) =
        start_from(&read(path)?, &registry(), t).map_err(|e| format!("{path}: {e}"))?;
    for n in notes {
        eprintln!("note: {n}");
    }
    match flags.get("--out") {
        Some(out) => std::fs::write(out, text + "\n").map_err(|e| format!("{out}: {e}")),
        None => {
            println!("{text}");
            Ok(())
        }
    }
}

/// Write a model file in the current form (ADR 0044): to `--out`, or
/// standard output. What was converted or dropped goes to standard error.
fn upgrade_file(args: &[String]) -> Result<(), String> {
    let (path, rest) = args.split_first().ok_or(USAGE)?;
    let flags = Flags::parse(rest, &["--out"])?;
    let (text, conversions, warnings) =
        describe::upgrade(&read(path)?, &registry()).map_err(|e| format!("{path}: {e}"))?;
    if conversions.is_empty() {
        eprintln!("note: {path} is already in the current form");
    }
    for c in conversions {
        eprintln!("converted: {c}");
    }
    for w in warnings {
        eprintln!("warning: {path}: {w}");
    }
    match flags.get("--out") {
        Some(out) => std::fs::write(out, text).map_err(|e| format!("{out}: {e}")),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

fn run_lorenz(args: &[String]) -> Result<(), String> {
    let flags = Flags::parse(args, &["--t-end", "--dt", "--method", "--rho"])?;
    let method = match flags.get("--method").unwrap_or("rk4") {
        "euler" => Integrator::Euler { substeps: 1 },
        "rk4" => Integrator::Rk4 { substeps: 1 },
        "dopri5" => Integrator::Dopri5 {
            rtol: 1e-8,
            atol: 1e-10,
        },
        other => return Err(format!("unknown method '{other}'")),
    };
    let mut model = Model::new();
    let id = model.add(Box::new(OdeBlock::new(Lorenz::new("lorenz"), method)));
    if let Some(rho) = flags.get("--rho") {
        let rho: f64 = rho
            .parse()
            .map_err(|_| format!("invalid number '{rho}' for --rho"))?;
        model.set_param(id, "rho", rho).map_err(|e| e.to_string())?;
    }
    let mut sim = Simulator::new(model, 0.0).map_err(|e| e.to_string())?;
    if let Some(l) = sim.labels() {
        eprintln!(
            "method: {} | speed: {:?} | accuracy: {:?}",
            method.method_spec().name,
            l.speed,
            l.accuracy
        );
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let write = |out: &mut BufWriter<_>, sim: &Simulator| -> io::Result<()> {
        let p = sim.output(id, "xyz").unwrap_or(&[]);
        writeln!(out, "{},{},{},{}", sim.time(), p[0], p[1], p[2])
    };
    writeln!(out, "t,x,y,z").map_err(|e| e.to_string())?;
    write(&mut out, &sim).map_err(|e| e.to_string())?;
    let dt = flags
        .positive("--dt")?
        .unwrap_or(simsandbox_file::DEFAULT_DT);
    let t_end = flags.positive("--t-end")?.unwrap_or(DEFAULT_T_END);
    for _ in 0..(t_end / dt).round() as usize {
        sim.step(dt).map_err(|e| e.to_string())?;
        write(&mut out, &sim).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("simsandbox {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("run") => run_file(rest),
        Some("check") => check(rest),
        Some("describe") => describe_types(rest),
        Some("schema") => {
            print!("{}", simsandbox_file::SCHEMA);
            Ok(())
        }
        Some("render") => render_file(rest),
        Some("start-from") => start_from_file(rest),
        Some("upgrade") => upgrade_file(rest),
        Some("lorenz") => run_lorenz(rest),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
