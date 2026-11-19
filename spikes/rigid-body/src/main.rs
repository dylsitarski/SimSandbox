//! The rigid-body spike of ADR 0057 item 3: the same cases on Rapier (2D and
//! 3D, f64, impulse and multibody joints) and on MuJoCo (through its C API),
//! printed as Markdown tables. See README.md.

mod common;
#[cfg(feature = "mujoco")]
mod mujoco;
mod rapier_2d;
mod rapier_3d;

use common::{cartpole as cp, disturb, hash_bits, pendulum as pd, time, Sim, SnapKind};
use rapier_2d::{Joints, Model};
use std::hint::black_box;

type Make = fn(Model, f64) -> Box<dyn Sim>;

fn quick() -> bool {
    std::env::var("SPIKE_QUICK").is_ok()
}

/// Every engine and setting that runs jointed models.
fn joint_variants() -> Vec<Make> {
    let mut v: Vec<Make> = vec![
        |m, dt| Box::new(rapier_2d::R2::new(m, Joints::Impulse, dt)),
        |m, dt| Box::new(rapier_2d::R2::new(m, Joints::Multibody, dt)),
        |m, dt| Box::new(rapier_3d::R3::new(m, Joints::Impulse, dt)),
        |m, dt| Box::new(rapier_3d::R3::new(m, Joints::Multibody, dt)),
    ];
    #[cfg(feature = "mujoco")]
    v.extend([
        (|m, dt| Box::new(mujoco::Mj::new(m, mujoco::Integrator::Euler, dt))) as Make,
        |m, dt| Box::new(mujoco::Mj::new(m, mujoco::Integrator::Rk4, dt)),
        |m, dt| Box::new(mujoco::Mj::new(m, mujoco::Integrator::ImplicitFast, dt)),
    ]);
    v
}

/// The engines and settings for models with contacts and no joints.
fn contact_variants() -> Vec<Make> {
    let mut v: Vec<Make> = vec![
        |m, dt| Box::new(rapier_2d::R2::new(m, Joints::Impulse, dt)),
        |m, dt| Box::new(rapier_3d::R3::new(m, Joints::Impulse, dt)),
    ];
    #[cfg(feature = "mujoco")]
    v.extend([
        (|m, dt| Box::new(mujoco::Mj::new(m, mujoco::Integrator::Euler, dt))) as Make,
        |m, dt| Box::new(mujoco::Mj::new(m, mujoco::Integrator::ImplicitFast, dt)),
    ]);
    v
}

fn contact_label(s: &dyn Sim) -> String {
    s.label().replace(", impulse joints", "")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = [
        "pendulum",
        "cartpole",
        "rewind",
        "determinism",
        "speed",
        "contacts",
    ];
    let cases: Vec<&str> = if args.is_empty() || args.iter().any(|a| a == "all") {
        all.to_vec()
    } else {
        args.iter().map(String::as_str).collect()
    };
    if cases == ["hash"] {
        hashes();
        return;
    }
    header();
    for c in cases {
        match c {
            "pendulum" => pendulum(),
            "cartpole" => cartpole(),
            "rewind" => rewind(),
            "determinism" => determinism(),
            "speed" => speed(),
            "contacts" => contacts(),
            "trace" => trace(),
            other => eprintln!("unknown case {other}; cases: {}", all.join(", ")),
        }
    }
}

fn header() {
    println!("# Rigid-body spike results\n");
    println!(
        "- Rapier: rapier2d-f64 and rapier3d-f64 0.36.1, features {}",
        rapier_features()
    );
    #[cfg(feature = "mujoco")]
    println!(
        "- MuJoCo: {} (C library, bindings by hand)",
        mujoco::version()
    );
    #[cfg(not(feature = "mujoco"))]
    println!("- MuJoCo: not built (feature `mujoco` off)");
    println!(
        "- Build: {} profile; quick mode {}\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        if quick() { "on" } else { "off" }
    );
}

fn rapier_features() -> &'static str {
    if cfg!(feature = "enhanced-determinism") {
        "`enhanced-determinism`, `serde-serialize`"
    } else {
        "`serde-serialize` (no `enhanced-determinism`)"
    }
}

fn sci(x: f64) -> String {
    format!("{x:+.2e}")
}

// ---------------------------------------------------------------- 1. Pendulum

fn measure_period(sim: &mut dyn Sim, crossings: usize, max_t: f64) -> Option<f64> {
    let mut meter = common::PeriodMeter::default();
    let dt = sim.dt();
    let mut k = 0usize;
    meter.push(0.0, sim.coords()[0]);
    while meter.crossings.len() < crossings && (k as f64) * dt < max_t {
        sim.step(&[]);
        k += 1;
        meter.push(k as f64 * dt, sim.coords()[0]);
    }
    meter.period()
}

fn pendulum() {
    println!("## 1. Pendulum period\n");
    println!(
        "A compact bob: mass {} kg, inertia {} kg m² about its centre, {} m from the pivot (ω0² = m g L / (I + m L²)). \
         Period: mean over 10 periods between upward zero crossings, each interpolated linearly in its step. \
         Error = (measured − exact) / exact, exact = 4 K(sin(a/2)) / ω0.\n",
        pd::M,
        pd::IC,
        pd::L
    );
    let dts = [0.01, 0.005, 0.002, 0.001, 0.0005];
    for a in [0.1, 2.0] {
        let exact = pd::period_exact(a);
        let small = pd::period_small_angle();
        println!(
            "Amplitude {a} rad: exact period {exact:.9} s; the small-angle formula gives {small:.9} s ({} off the exact).\n",
            sci((small - exact) / exact)
        );
        print!("| Engine |");
        for dt in dts {
            print!(" dt = {dt} s |");
        }
        println!(" Order (last two) |");
        println!("|---|{}---|", "---|".repeat(dts.len()));
        for make in joint_variants() {
            let mut errs = vec![];
            let mut label = String::new();
            for dt in dts {
                let mut sim = make(Model::Pendulum { amplitude: a }, dt);
                label = sim.label();
                let t = measure_period(sim.as_mut(), 11, 15.0 * exact);
                errs.push(t.map(|t| (t - exact) / exact));
            }
            print!("| {label} |");
            for e in &errs {
                match e {
                    Some(e) => print!(" {} |", sci(*e)),
                    None => print!(" no period |"),
                }
            }
            let n = errs.len();
            match (errs[n - 2], errs[n - 1]) {
                (Some(a), Some(b)) if b != 0.0 => println!(" {:.2} |", (a / b).abs().log2()),
                _ => println!(" – |"),
            }
        }
        println!();
    }

    println!("Energy over 100 periods without damping, dt = 0.001 s (energy from the bodies' velocities and heights, so joint drift shows):\n");
    println!("| Engine | Amplitude (rad) | max abs(E − E0)/E0 | (E_end − E0)/E0 | max joint drift (m) | wall time (s) |");
    println!("|---|---|---|---|---|---|");
    let periods = if quick() { 10.0 } else { 100.0 };
    for a in [0.1, 2.0] {
        for make in joint_variants() {
            let dt = 0.001;
            let mut sim = make(Model::Pendulum { amplitude: a }, dt);
            let e0 = sim.energy();
            let steps = (periods * pd::period_exact(a) / dt).round() as usize;
            let (mut maxe, mut maxd) = (0.0f64, 0.0f64);
            let (_, wall) = time(|| {
                for _ in 0..steps {
                    sim.step(&[]);
                    maxe = maxe.max(((sim.energy() - e0) / e0).abs());
                    maxd = maxd.max(sim.joint_drift());
                }
            });
            println!(
                "| {} | {a} | {:.2e} | {} | {:.1e} | {wall:.2} |",
                sim.label(),
                maxe,
                sci((sim.energy() - e0) / e0),
                maxd
            );
        }
    }
    println!();
}

// ---------------------------------------------------------------- 2. Cart-pole

struct Run {
    coords: Vec<[f64; 4]>,
    inputs: Vec<[f64; 3]>,
    drift: f64,
}

/// Closed loop (LQR) with the recorded disturbances, or open loop.
fn run_cartpole(sim: &mut dyn Sim, steps: usize, control: bool, gains: &[f64; 4]) -> Run {
    let dt = sim.dt();
    let mut run = Run {
        coords: vec![],
        inputs: vec![],
        drift: 0.0,
    };
    for k in 0..steps {
        let c = sim.coords();
        let s = [c[0], c[1], c[2], c[3]];
        run.coords.push(s);
        let (f, p) = if control {
            (cp::control(gains, &s), disturb::tip_force(k, dt, &s))
        } else {
            (0.0, [0.0, 0.0])
        };
        let input = [f, p[0], p[1]];
        sim.step(&input);
        run.inputs.push(input);
        run.drift = run.drift.max(sim.joint_drift());
    }
    let c = sim.coords();
    run.coords.push([c[0], c[1], c[2], c[3]]);
    run
}

fn run_reference(dt: f64, steps: usize, control: bool, gains: &[f64; 4]) -> Vec<[f64; 4]> {
    let mut s = [0.0, 0.0, cp::THETA0, 0.0];
    let mut out = vec![s];
    for k in 0..steps {
        let (f, p) = if control {
            (cp::control(gains, &s), disturb::tip_force(k, dt, &s))
        } else {
            (0.0, [0.0, 0.0])
        };
        cp::reference_step(&mut s, dt, 20, f, p);
        out.push(s);
    }
    out
}

fn cartpole() {
    println!("## 2. Cart-pole\n");
    let dt = 0.001;
    let gains = cp::lqr();
    println!(
        "Cart {} kg on a slider along x; uniform pole {} kg, {} m, hinged at the cart's centre, starting at θ = {} rad from upright. \
         Physics at 1 kHz (dt = {dt} s). LQR gains from the linearized equations in continuous time (Q = diag(10, 1, 100, 1), R = 0.1): \
         K = [{:.4}, {:.4}, {:.4}, {:.4}], force limited to ±{} N. \
         Disturbances, as the scene would send them: an impulse of {} N s along +x on the pole's tip at t = {} s; \
         from t = {} s to {} s a pull on the tip toward a held pointer at ({}, {}) m (spring {} N/m, damper {} N s/m), \
         computed from the state each step. Reference: the exact equations by RK4 with 20 substeps, same controller and disturbances.\n",
        cp::MC,
        cp::MP,
        cp::LEN,
        cp::THETA0,
        gains[0],
        gains[1],
        gains[2],
        gains[3],
        cp::FORCE_LIMIT,
        disturb::IMPULSE,
        disturb::IMPULSE_T,
        disturb::GRAB_FROM,
        disturb::GRAB_TO,
        disturb::POINTER[0],
        disturb::POINTER[1],
        disturb::K,
        disturb::C
    );
    let steps = 6000;
    let reference = run_reference(dt, steps, true, &gains);
    let peak = |c: &[[f64; 4]]| {
        let (a, b) = (
            (disturb::GRAB_FROM / dt) as usize,
            ((disturb::GRAB_TO + 0.5) / dt) as usize,
        );
        c[a..b].iter().map(|s| s[2].abs()).fold(0.0, f64::max)
    };
    let fin = reference[steps];
    println!(
        "Reference: largest |θ| while pulled {:.4} rad; at 6 s x = {:.5} m, θ = {:.2e} rad.\n",
        peak(&reference),
        fin[0],
        fin[2]
    );
    println!("| Engine | max abs(θ − θ_ref) (rad) | max abs(x − x_ref) (m) | largest abs(θ) while pulled | x at 6 s | θ at 6 s | max joint drift (m) |");
    println!("|---|---|---|---|---|---|---|");
    for make in joint_variants() {
        let mut sim = make(Model::CartPole, dt);
        let run = run_cartpole(sim.as_mut(), steps, true, &gains);
        let (mut et, mut ex) = (0.0f64, 0.0f64);
        for (a, b) in run.coords.iter().zip(&reference) {
            et = et.max((a[2] - b[2]).abs());
            ex = ex.max((a[0] - b[0]).abs());
        }
        let f = run.coords[steps];
        println!(
            "| {} | {:.2e} | {:.2e} | {:.4} | {:.5} | {} | {:.1e} |",
            sim.label(),
            et,
            ex,
            peak(&run.coords),
            f[0],
            sci(f[2]),
            run.drift
        );
    }
    println!();
    println!("Open loop (no controller, no disturbance): the pole falls from {} rad; error against the reference after 0.5 s and 1 s.\n", cp::THETA0);
    let reference = run_reference(dt, 1000, false, &gains);
    println!(
        "Reference: θ(0.5 s) = {:.6} rad, θ(1 s) = {:.6} rad, x(1 s) = {:.6} m.\n",
        reference[500][2], reference[1000][2], reference[1000][0]
    );
    println!("| Engine | θ − θ_ref at 0.5 s | θ − θ_ref at 1 s | x − x_ref at 1 s |");
    println!("|---|---|---|---|");
    for make in joint_variants() {
        let mut sim = make(Model::CartPole, dt);
        let run = run_cartpole(sim.as_mut(), 1000, false, &gains);
        println!(
            "| {} | {} | {} | {} |",
            sim.label(),
            sci(run.coords[500][2] - reference[500][2]),
            sci(run.coords[1000][2] - reference[1000][2]),
            sci(run.coords[1000][0] - reference[1000][0])
        );
    }
    println!();
}

// ---------------------------------------------------------------- 3. Rewind

fn record_inputs(make: Make, model: Model, dt: f64, n: usize) -> Vec<[f64; 3]> {
    match model {
        Model::CartPole => {
            let mut sim = make(model, dt);
            run_cartpole(sim.as_mut(), n, true, &cp::lqr()).inputs
        }
        _ => vec![[0.0; 3]; n],
    }
}

fn compare(a: &[f64], b: &[f64]) -> (usize, f64) {
    let diff = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.to_bits() != y.to_bits())
        .count();
    let max = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max);
    (diff, max)
}

fn kind_name(k: SnapKind) -> &'static str {
    match k {
        SnapKind::Memory => "in memory",
        SnapKind::Bytes => "bytes, same objects",
        SnapKind::BytesFresh => "bytes, new objects",
        SnapKind::Minimal => "poses and velocities only",
    }
}

fn rewind() {
    println!("## 3. Snapshot and restore for rewind\n");
    println!(
        "Run to step k with the recorded input, take a snapshot, run on to step n and keep every body's pose and velocity; \
         restore the snapshot, run k..n again with the same recorded input, and compare all those values bit for bit. \
         Kinds: *in memory* (Rapier: clones of every set; MuJoCo: `mj_copyData` into a second `mjData`); \
         *bytes* (Rapier: bincode of every set; MuJoCo: `mj_getState(mjSTATE_INTEGRATION)`), restored into the same engine \
         objects or into new ones (Rapier: a new `PhysicsPipeline`; MuJoCo: a new `mjData`); \
         *poses and velocities only* (Rapier: each body's pose and velocities set back; MuJoCo: `mjSTATE_FULLPHYSICS`, without warm start or inputs). \
         Times are means over repeated calls.\n"
    );
    println!("| Model | Engine | Snapshot | Bit-identical at n | values differing / max abs diff | Size (bytes) | Take (µs) | Restore (µs) |");
    println!("|---|---|---|---|---|---|---|---|");
    let dt = 0.001;
    let cases: [(Model, usize, usize, bool); 3] = [
        (Model::CartPole, 2000, 5000, true),
        (Model::BoxOnBox, 300, 1500, false),
        (Model::Pile { boxes: 100 }, 500, 1500, false),
    ];
    for (model, k, n, jointed) in cases {
        let makes = if jointed {
            joint_variants()
        } else {
            contact_variants()
        };
        let model_name = match model {
            Model::CartPole => "cart-pole".to_string(),
            Model::BoxOnBox => "box on box".to_string(),
            Model::Pile { boxes } => format!("{boxes} boxes"),
            _ => unreachable!(),
        };
        for make in makes {
            let inputs = record_inputs(make, model, dt, n);
            for kind in [
                SnapKind::Memory,
                SnapKind::Bytes,
                SnapKind::BytesFresh,
                SnapKind::Minimal,
            ] {
                let mut sim = make(model, dt);
                for inp in &inputs[..k] {
                    sim.step(inp);
                }
                let snap = sim.snapshot(kind);
                for inp in &inputs[k..n] {
                    sim.step(inp);
                }
                let first = sim.rows();
                sim.restore(&snap);
                for inp in &inputs[k..n] {
                    sim.step(inp);
                }
                let again = sim.rows();
                let (diff, max) = compare(&first, &again);
                // Timing, on the state at n.
                let reps = if matches!(model, Model::Pile { .. }) {
                    20
                } else {
                    500
                };
                let (s2, t_take) = time(|| {
                    let mut s = None;
                    for _ in 0..reps {
                        s = Some(black_box(sim.snapshot(kind)));
                    }
                    s.unwrap()
                });
                let (_, t_restore) = time(|| {
                    for _ in 0..reps {
                        sim.restore(black_box(&s2));
                    }
                });
                let size = if snap.bytes == 0 {
                    "whole mjData".to_string()
                } else {
                    s2.bytes.to_string()
                };
                let label = if jointed {
                    sim.label()
                } else {
                    contact_label(sim.as_ref())
                };
                println!(
                    "| {model_name} | {label} | {} | {} | {} | {size} | {:.1} | {:.1} |",
                    kind_name(kind),
                    if diff == 0 { "yes" } else { "**no**" },
                    if diff == 0 {
                        "0".to_string()
                    } else {
                        format!("{diff} of {} / {:.1e}", first.len(), max)
                    },
                    t_take / reps as f64 * 1e6,
                    t_restore / reps as f64 * 1e6
                );
            }
        }
    }
    println!();
}

// ---------------------------------------------------------------- 4. Determinism

fn run_hash(make: Make, model: Model, steps: usize) -> (String, u64) {
    let dt = 0.001;
    let mut sim = make(model, dt);
    let gains = cp::lqr();
    let mut all = vec![];
    for k in 0..steps {
        let input = if let Model::CartPole = model {
            let c = sim.coords();
            let s = [c[0], c[1], c[2], c[3]];
            let p = disturb::tip_force(k, dt, &s);
            [cp::control(&gains, &s), p[0], p[1]]
        } else {
            [0.0; 3]
        };
        sim.step(&input);
        all.extend(sim.rows());
    }
    (sim.label(), hash_bits(all))
}

fn hash_cases() -> Vec<(String, u64)> {
    let mut out = vec![];
    for make in joint_variants() {
        let (l, h) = run_hash(make, Model::CartPole, 6000);
        out.push((format!("cart-pole 6 s, {l}"), h));
    }
    for make in contact_variants() {
        let (l, h) = run_hash(make, Model::Pile { boxes: 100 }, 2000);
        out.push((
            format!("100 boxes 2 s, {}", l.replace(", impulse joints", "")),
            h,
        ));
    }
    out
}

fn hashes() {
    for (l, h) in hash_cases() {
        println!("{h:016x} {l}");
    }
}

fn determinism() {
    println!("## 4. Determinism\n");
    println!(
        "Every body's pose and velocity after every step, hashed bit for bit (FNV-1a over the IEEE bits). \
         Twice in this process, and in two more processes of the same binary.\n"
    );
    let a = hash_cases();
    let b = hash_cases();
    let exe = std::env::current_exe().expect("exe");
    let p1 = String::from_utf8(
        std::process::Command::new(&exe)
            .arg("hash")
            .output()
            .expect("run")
            .stdout,
    )
    .unwrap();
    let p2 = String::from_utf8(
        std::process::Command::new(&exe)
            .arg("hash")
            .output()
            .expect("run")
            .stdout,
    )
    .unwrap();
    let p1: Vec<&str> = p1.lines().collect();
    let p2: Vec<&str> = p2.lines().collect();
    println!("| Run | Hash | Same in this process | Same in two other processes |");
    println!("|---|---|---|---|");
    for (i, ((l, h), (_, h2))) in a.iter().zip(&b).enumerate() {
        let line = format!("{h:016x} {l}");
        let same_proc = p1.get(i) == Some(&line.as_str()) && p2.get(i) == Some(&line.as_str());
        println!(
            "| {l} | `{h:016x}` | {} | {} |",
            if h == h2 { "yes" } else { "**no**" },
            if same_proc { "yes" } else { "**no**" }
        );
    }
    println!();
}

// ---------------------------------------------------------------- 5. Speed

fn speed() {
    println!("## 5. Speed\n");
    println!(
        "One thread. Steps per second over the run (wall time, including the controller for the cart-pole); \
         × real time is the speed over real time when the physics runs at 1 kHz.\n"
    );
    println!("| Model | Engine | Steps | µs per step | Steps per second | × real time at 1 kHz | Engine warnings | Largest speed at the end (m/s or rad/s) |");
    println!("|---|---|---|---|---|---|---|---|");
    let dt = 0.001;
    let q = quick();
    let cases: Vec<(Model, usize, bool, &str)> = vec![
        (
            Model::CartPole,
            if q { 20_000 } else { 200_000 },
            true,
            "cart-pole",
        ),
        (
            Model::Chain { links: 50 },
            if q { 1_000 } else { 10_000 },
            true,
            "chain of 50 links",
        ),
        (
            Model::Pile { boxes: 100 },
            if q { 500 } else { 3_000 },
            false,
            "100 boxes into a pile",
        ),
    ];
    for (model, steps, jointed, name) in cases {
        let makes = if jointed {
            joint_variants()
        } else {
            contact_variants()
        };
        for make in makes {
            let mut sim = make(model, dt);
            let w0 = sim.warnings();
            let gains = cp::lqr();
            let (_, wall) = time(|| {
                for _ in 0..steps {
                    if let Model::CartPole = model {
                        let c = sim.coords();
                        let u = cp::control(&gains, &[c[0], c[1], c[2], c[3]]);
                        sim.step(&[u, 0.0, 0.0]);
                    } else {
                        sim.step(&[]);
                    }
                }
            });
            let per = wall / steps as f64;
            let label = if jointed {
                sim.label()
            } else {
                contact_label(sim.as_ref())
            };
            let warn = sim.warnings() - w0;
            println!(
                "| {name} | {label} | {steps} | {:.2} | {:.0} | {:.0} | {} | {:.3} |",
                per * 1e6,
                1.0 / per,
                1.0 / per / 1000.0,
                if warn == 0 {
                    "0".to_string()
                } else {
                    format!("**{warn}**")
                },
                largest_speed(sim.as_ref())
            );
        }
    }
    println!();
}

/// The largest velocity component of any body (rows: Rapier 2D x, y, angle,
/// vx, vy, ω; Rapier 3D pose (7) then velocities (6); MuJoCo qpos then qvel).
fn largest_speed(sim: &dyn Sim) -> f64 {
    let r = sim.rows();
    let label = sim.label();
    let vals: Vec<f64> = if label.starts_with("Rapier 2D") {
        r.chunks(6).flat_map(|c| c[3..6].to_vec()).collect()
    } else if label.starts_with("Rapier 3D") {
        r.chunks(13).flat_map(|c| c[7..13].to_vec()).collect()
    } else {
        // qpos and qvel: the speeds are the last nv values; nv ≤ nq.
        let n = r.len();
        let nv = match n {
            100 => 50,   // chain: 50 + 50
            1300 => 600, // 100 free bodies: 700 + 600
            _ => n / 2,
        };
        r[n - nv..].to_vec()
    };
    vals.iter().fold(0.0f64, |m, v| {
        if v.is_finite() {
            m.max(v.abs())
        } else {
            f64::INFINITY
        }
    })
}

// ---------------------------------------------------------------- 6. Contacts

fn contacts() {
    println!("## 6. Contacts\n");
    println!(
        "Boxes of 0.2 m and 1 kg, friction 1, at 1 kHz. A box resting on the ground for 10 s (centre should stay at 0.1 m); \
         a box resting on another, 0.05 m off centre, for 10 s (centres at 0.1 m and 0.3 m); \
         a box dropped from 0.3 m above another, 0.05 m off centre and tilted 0.1 rad, for 3 s (if it settles flat on it, at 0.3 m; it may also tumble off). \
         Speeds are the largest change of the box centres per step over the last second, divided by dt. Deterministic: the run's hash twice.\n"
    );
    println!("| Case | Engine | Final height(s) (m) | Final x (m) | Largest speed in the last second (m/s) | Deterministic |");
    println!("|---|---|---|---|---|---|");
    let dt = 0.001;
    for (model, secs, name) in [
        (Model::BoxOnGround, 10.0, "box on the ground"),
        (Model::Stack, 10.0, "box on a box"),
        (Model::BoxOnBox, 3.0, "box dropped onto a box"),
    ] {
        for make in contact_variants() {
            let mut sim = make(model, dt);
            let steps = (secs / dt) as usize;
            let mut prev = sim.box_poses();
            let mut vmax = 0.0f64;
            let mut all = vec![];
            for k in 0..steps {
                sim.step(&[]);
                let now = sim.box_poses();
                if k + 1000 >= steps {
                    for (a, b) in now.iter().zip(&prev) {
                        vmax = vmax.max(((a[0] - b[0]).hypot(a[1] - b[1])) / dt);
                    }
                }
                all.extend(sim.rows());
                prev = now;
            }
            let h1 = hash_bits(all);
            let (_, h2) = {
                let mut s2 = make(model, dt);
                let mut all = vec![];
                for _ in 0..steps {
                    s2.step(&[]);
                    all.extend(s2.rows());
                }
                ((), hash_bits(all))
            };
            let p = sim.box_poses();
            let heights: Vec<String> = p.iter().map(|b| format!("{:.5}", b[1])).collect();
            let xs: Vec<String> = p.iter().map(|b| format!("{:.4}", b[0])).collect();
            println!(
                "| {name} | {} | {} | {} | {:.1e} | {} |",
                contact_label(sim.as_ref()),
                heights.join(", "),
                xs.join(", "),
                vmax,
                if h1 == h2 { "yes" } else { "**no**" }
            );
        }
    }
    println!();
}

/// The upper box's path in the box-onto-box case, every 50 ms (for looking).
fn trace() {
    let dt = 0.001;
    for make in contact_variants() {
        let mut sim = make(Model::BoxOnBox, dt);
        println!("{}", contact_label(sim.as_ref()));
        for k in 0..1000 {
            sim.step(&[]);
            if (k % 50 == 49 && k < 1000) || ((220..300).contains(&k) && k % 5 == 0) {
                let p = sim.box_poses();
                let r = sim.rows();
                let extra = if r.len() == 12 {
                    format!(
                        " angle {:+.4} vx {:+.3} vy {:+.3} w {:+.3}",
                        r[8], r[9], r[10], r[11]
                    )
                } else {
                    String::new()
                };
                println!(
                    "  t {:.2}  lower {:+.4} {:.4}  upper {:+.4} {:.4}{extra}",
                    (k + 1) as f64 * dt,
                    p[0][0],
                    p[0][1],
                    p[1][0],
                    p[1][1]
                );
            }
        }
    }
}
