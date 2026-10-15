//! `simsandbox` — headless runner. Writes CSV to stdout.
//!
//! ```text
//! simsandbox lorenz [--t-end 20] [--dt 0.01] [--method euler|rk4|dopri5] [--rho 28]
//! simsandbox --version
//! ```

use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::systems::Lorenz;
use simsandbox_eq::{Integrator, OdeBlock};
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: simsandbox lorenz [--t-end SECONDS] [--dt SECONDS] [--method euler|rk4|dopri5] [--rho VALUE]";

struct Options {
    t_end: f64,
    dt: f64,
    method: Integrator,
    rho: Option<f64>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        t_end: 20.0,
        dt: 0.01,
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
            "--t-end" => o.t_end = num()?,
            "--dt" => o.dt = num()?,
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
    if o.dt <= 0.0 || o.t_end <= 0.0 {
        return Err("--dt and --t-end must be positive".into());
    }
    Ok(o)
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
    let steps = (o.t_end / o.dt).round() as usize;
    for _ in 0..steps {
        sim.step(o.dt).map_err(|e| e.to_string())?;
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
