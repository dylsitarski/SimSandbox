//! `simsandbox` — headless runner. Writes CSV to stdout.
//!
//! ```text
//! simsandbox run FILE [--t-end SECONDS] [--dt SECONDS]   (default: the file's `run` section)
//!                                   (a model with no states: one row per sample of its sweep)
//! simsandbox lorenz [--t-end 20] [--dt 0.01] [--method euler|rk4|dopri5] [--rho 28]
//! simsandbox --version
//! ```

use simsandbox_core::block::Shape;
use simsandbox_core::graph::Model;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::systems::Lorenz;
use simsandbox_eq::{Integrator, OdeBlock};
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "usage:
  simsandbox run FILE [--t-end SECONDS] [--dt SECONDS]
  simsandbox lorenz [--t-end SECONDS] [--dt SECONDS] [--method euler|rk4|dopri5] [--rho VALUE]";

/// Used when neither a flag nor the model file gives a value.
const DEFAULT_T_END: f64 = 20.0;

struct Options {
    /// Flags override the model file's `run` section (ADR 0013).
    t_end: Option<f64>,
    dt: Option<f64>,
    method: Integrator,
    rho: Option<f64>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        t_end: None,
        dt: None,
        method: Integrator::Rk4 { substeps: 1 },
        rho: None,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or(format!("missing value for {flag}"))?;
        let num = || {
            value
                .parse::<f64>()
                .map_err(|_| format!("invalid number '{value}' for {flag}"))
        };
        match flag.as_str() {
            "--t-end" => o.t_end = Some(num()?),
            "--dt" => o.dt = Some(num()?),
            "--rho" => o.rho = Some(num()?),
            "--method" => {
                o.method = match value.as_str() {
                    "euler" => Integrator::Euler { substeps: 1 },
                    "rk4" => Integrator::Rk4 { substeps: 1 },
                    "dopri5" => Integrator::Dopri5 {
                        rtol: 1e-8,
                        atol: 1e-10,
                    },
                    other => return Err(format!("unknown method '{other}'")),
                }
            }
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    if [o.dt, o.t_end]
        .iter()
        .flatten()
        .any(|v| !(v.is_finite() && *v > 0.0))
    {
        return Err("--dt and --t-end must be positive".into());
    }
    Ok(o)
}

/// Every block type this binary can load from a file.
fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).expect("equation types register once");
    r
}

/// Run a model file and write every output, in file block order, as CSV.
/// Vector outputs get one column per element: `block.port[i]`. A model
/// with no states is swept instead (ADR 0028): one row per sample, with
/// the sample number and the swept parameters first.
fn run_file(args: &[String]) -> Result<(), String> {
    let (path, rest) = args.split_first().ok_or(USAGE)?;
    let o = parse(rest)?;
    if rest.iter().any(|a| a == "--method" || a == "--rho") {
        return Err("--method and --rho apply to 'lorenz' only; set them in the file".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let loaded = simsandbox_file::load(&text, &registry()).map_err(|e| format!("{path}: {e}"))?;
    for w in &loaded.warnings {
        eprintln!("warning: {path}: {w}");
    }
    let dt = o.dt.unwrap_or(loaded.file.run.dt);
    let t_end = o.t_end.or(loaded.file.run.t_end).unwrap_or(DEFAULT_T_END);
    if loaded.plan.is_some() && (o.dt.is_some() || o.t_end.is_some()) {
        return Err("--dt and --t-end do not apply to a swept model".into());
    }

    let mut columns = Vec::new();
    let mut header = vec!["t".to_string()];
    if let Some(plan) = &loaded.plan {
        header = vec!["sample".to_string()];
        header.extend(plan.keys.iter().cloned());
    }
    for entry in &loaded.file.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        for port in spec.outputs() {
            let n = port.shape.len();
            for i in 0..n {
                header.push(if port.shape == Shape::Scalar {
                    format!("{}.{}", entry.name, port.name)
                } else {
                    format!("{}.{}[{i}]", entry.name, port.name)
                });
            }
            columns.push((id, port.name.clone()));
        }
    }

    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    if let Some(l) = sim.labels() {
        eprintln!("speed: {:?} | accuracy: {:?}", l.speed, l.accuracy);
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let write = |out: &mut BufWriter<_>, sim: &Simulator| -> io::Result<()> {
        for (id, port) in &columns {
            for v in sim.output(*id, port).unwrap_or(&[]) {
                write!(out, ",{v}")?;
            }
        }
        writeln!(out)
    };
    writeln!(out, "{}", header.join(",")).map_err(|e| e.to_string())?;
    if let Some(plan) = &loaded.plan {
        let targets: Vec<_> = plan
            .keys
            .iter()
            .map(|k| {
                let (block, param) = k.split_once('.').expect("plan keys are block.param");
                (loaded.ids[block], param)
            })
            .collect();
        for (k, point) in plan.points.iter().enumerate() {
            for (&(id, param), &v) in targets.iter().zip(point) {
                sim.set_param(id, param, v).map_err(|e| e.to_string())?;
            }
            sim.restart(0.0).map_err(|e| e.to_string())?;
            let values: String = point.iter().map(|v| format!(",{v}")).collect();
            write!(out, "{k}{values}").map_err(|e| e.to_string())?;
            write(&mut out, &sim).map_err(|e| e.to_string())?;
        }
        return out.flush().map_err(|e| e.to_string());
    }
    write!(out, "{}", sim.time()).map_err(|e| e.to_string())?;
    write(&mut out, &sim).map_err(|e| e.to_string())?;
    let steps = (t_end / dt).round() as usize;
    for _ in 0..steps {
        sim.step(dt).map_err(|e| e.to_string())?;
        write!(out, "{}", sim.time()).map_err(|e| e.to_string())?;
        write(&mut out, &sim).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

fn run_lorenz(o: &Options) -> Result<(), String> {
    let mut model = Model::new();
    let id = model.add(Box::new(OdeBlock::new(Lorenz::new("lorenz"), o.method)));
    if let Some(rho) = o.rho {
        model.set_param(id, "rho", rho).map_err(|e| e.to_string())?;
    }
    let mut sim = Simulator::new(model, 0.0).map_err(|e| e.to_string())?;
    if let Some(l) = sim.labels() {
        eprintln!(
            "method: {} | speed: {:?} | accuracy: {:?}",
            o.method.method_spec().name,
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
    let dt = o.dt.unwrap_or(simsandbox_file::DEFAULT_DT);
    let steps = (o.t_end.unwrap_or(DEFAULT_T_END) / dt).round() as usize;
    for _ in 0..steps {
        sim.step(dt).map_err(|e| e.to_string())?;
        write(&mut out, &sim).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("simsandbox {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("run") => run_file(&args[1..]),
        Some("lorenz") => parse(&args[1..]).and_then(|o| run_lorenz(&o)),
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
