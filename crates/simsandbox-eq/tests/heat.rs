//! The heat equation ∂T/∂t = α ∂²T/∂x² in typed equations on a grid
//! (ADR 0060), against its exact solutions: the reference problem for
//! arrays, stencils and edges.
//!
//! - A sine or cosine mode keeps its shape and decays as
//!   exp(−α k² t). On cells of width h the stencil decays it as
//!   exp(−α (4/h²) sin²(k h/2) t) instead, so the space error is second
//!   order, and that discrete mode is exact for the discretised system,
//!   which lets the time error be measured on its own.
//! - A Gaussian on a rod with both ends held at 0 has an exact solution
//!   by images, used to compare the typed rod with `eq.heat_rod`.

use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::sim::Simulator;
use simsandbox_eq::fields::HeatRod;
use simsandbox_eq::{EquationSet, Integrator, OdeBlock};
use std::f64::consts::PI;

const ALPHA: f64 = 0.01;
const LENGTH: f64 = 1.0;

/// The typed rod's equations: `cells` cells starting from `start(x)`,
/// with `extra` lines (edges, parameters).
fn lines(cells: usize, start: &str, extra: &str) -> Vec<String> {
    let mut l = vec![
        format!("grid x [m] = 0 .. length, {cells}"),
        format!("param length [m] = {LENGTH}"),
        format!("param alpha [m^2/s] = {ALPHA}"),
        "param amp [K] = 1".to_string(),
        format!("state T[x] [K] = {start}"),
        "dT/dt = alpha*lap(T)".to_string(),
    ];
    l.extend(extra.lines().map(str::to_string));
    l
}

/// A typed rod (see [`lines`]) as a model of its own.
fn rod(cells: usize, start: &str, extra: &str, integrator: Integrator) -> (Simulator, BlockId) {
    let set = EquationSet::parse("rod", &lines(cells, start, extra)).expect("valid equations");
    let mut m = Model::new();
    let id = m.add(Box::new(OdeBlock::new(set, integrator)));
    (Simulator::new(m, 0.0).expect("builds"), id)
}

fn run(sim: &mut Simulator, t_end: f64, dt: f64) {
    let steps = (t_end / dt).round() as usize;
    for _ in 0..steps {
        sim.step(dt).unwrap();
    }
}

/// The largest difference from `exact(x)` over the cells.
fn max_error(sim: &Simulator, id: BlockId, exact: impl Fn(f64) -> f64) -> f64 {
    let t = sim.output(id, "T").unwrap();
    let x = sim.output(id, "x").unwrap();
    t.iter()
        .zip(x)
        .map(|(t, x)| (t - exact(*x)).abs())
        .fold(0.0, f64::max)
}

/// A mode `start` with wavenumber `k`, under each edge condition.
struct Mode {
    name: &'static str,
    start: &'static str,
    edges: &'static str,
    shape: fn(f64) -> f64,
    k: f64,
}

const MODES: [Mode; 3] = [
    Mode {
        name: "held at 0 (sine)",
        start: "amp*sin(pi*x/length)",
        edges: "edge T: x- = 0, x+ = 0",
        shape: |x| (PI * x / LENGTH).sin(),
        k: PI / LENGTH,
    },
    Mode {
        name: "insulated (cosine)",
        start: "amp*cos(pi*x/length)",
        edges: "",
        shape: |x| (PI * x / LENGTH).cos(),
        k: PI / LENGTH,
    },
    Mode {
        name: "periodic (sine of the whole length)",
        start: "amp*sin(2*pi*x/length)",
        edges: "edge T: x periodic",
        shape: |x| (2.0 * PI * x / LENGTH).sin(),
        k: 2.0 * PI / LENGTH,
    },
];

#[test]
fn modes_decay_at_the_exact_rate_with_second_order_in_space() {
    // RK4 with a step small enough that the time error is far below the
    // space error, up to t = 5 s.
    let t_end = 5.0;
    let rk4 = Integrator::Rk4 { substeps: 4 };
    for mode in MODES {
        let exact = |x: f64| (mode.shape)(x) * (-ALPHA * mode.k * mode.k * t_end).exp();
        let errors: Vec<f64> = [10, 20, 40, 80]
            .into_iter()
            .map(|cells| {
                let (mut sim, id) = rod(cells, mode.start, mode.edges, rk4);
                run(&mut sim, t_end, 0.01);
                max_error(&sim, id, exact)
            })
            .collect();
        for w in errors.windows(2) {
            let ratio = w[0] / w[1];
            assert!(
                (3.8..4.2).contains(&ratio),
                "{}: errors {errors:?}, ratio {ratio}",
                mode.name
            );
        }
        // 80 cells: within 0.004% of the amplitude per half wave on the
        // rod, the error growing as k²: (k h)² / 24 of the decay, roughly.
        let waves = mode.k * LENGTH / PI;
        assert!(
            errors[3] < 4e-5 * waves * waves,
            "{}: {errors:?}",
            mode.name
        );
    }
}

#[test]
fn the_stencil_decays_a_mode_at_its_discrete_rate_exactly() {
    // dT/dt from the equations is −α (4/h²) sin²(k h/2) T, cell by cell,
    // under each edge condition (the ghost cells reflect the mode).
    for mode in MODES {
        let cells = 16;
        let h = LENGTH / cells as f64;
        let rate = -ALPHA * 4.0 / (h * h) * (mode.k * h / 2.0).sin().powi(2);
        let set = EquationSet::parse("rod", &lines(cells, mode.start, mode.edges)).unwrap();
        let p = [LENGTH, ALPHA, 1.0];
        let mut x0 = vec![0.0; cells];
        use simsandbox_core::block::Equations;
        use simsandbox_eq::OdeSystem;
        set.initial_state(&p, &mut x0);
        let mut dx = vec![0.0; cells];
        set.rhs(0.0, &x0, &[], &p, &mut dx);
        for (k, (d, x)) in dx.iter().zip(&x0).enumerate() {
            assert!(
                (d - rate * x).abs() < 1e-12,
                "{}: cell {k}: {d} vs {}",
                mode.name,
                rate * x
            );
        }
    }
}

#[test]
fn rk4_is_fourth_order_in_time_on_the_grid() {
    // Against the exact solution of the discretised system: only the
    // time error remains.
    let (cells, t_end) = (20, 2.0);
    let h = LENGTH / cells as f64;
    let mode = &MODES[0];
    let rate = -ALPHA * 4.0 / (h * h) * (mode.k * h / 2.0).sin().powi(2);
    let exact = |x: f64| (mode.shape)(x) * (rate * t_end).exp();
    let errors: Vec<f64> = [0.2, 0.1, 0.05]
        .into_iter()
        .map(|dt| {
            let (mut sim, id) = rod(
                cells,
                mode.start,
                mode.edges,
                Integrator::Rk4 { substeps: 1 },
            );
            run(&mut sim, t_end, dt);
            max_error(&sim, id, exact)
        })
        .collect();
    for w in errors.windows(2) {
        let ratio = w[0] / w[1];
        assert!(
            (14.0..18.0).contains(&ratio),
            "errors {errors:?}, ratio {ratio}"
        );
    }
}

/// The exact temperature on a rod of length `l` with both ends held at 0,
/// from a Gaussian `peak·exp(−((x − c)/w)²)` at t = 0, by images: the
/// spreading Gaussian, reflected oddly about each end.
fn gaussian_on_rod(x: f64, t: f64, peak: f64, c: f64, w: f64, l: f64) -> f64 {
    let s2 = w * w + 4.0 * ALPHA * t;
    let g = |centre: f64| peak * w / s2.sqrt() * (-(x - centre).powi(2) / s2).exp();
    (-3i32..=3)
        .map(|n| {
            let shift = 2.0 * n as f64 * l;
            g(c + shift) - g(-c + shift)
        })
        .sum()
}

#[test]
fn the_typed_and_built_in_rods_both_follow_the_exact_solution() {
    // Both rods: 50 points, a 100 K Gaussian of width 0.1 m at the middle,
    // ends held at 0, RK4 with 10 substeps of a 0.05 s step, to t = 2 s.
    let (peak, c, w) = (100.0, 0.5, 0.1);
    let exact = |x: f64| gaussian_on_rod(x, 2.0, peak, c, w, LENGTH);
    let rk4 = Integrator::Rk4 { substeps: 10 };
    let (mut typed, id) = rod(
        50,
        "peak*exp(-((x - length/2)/width)^2)",
        "edge T: x- = 0, x+ = 0\nparam peak [K] = 100\nparam width [m] = 0.1",
        rk4,
    );
    run(&mut typed, 2.0, 0.05);
    let typed_error = max_error(&typed, id, exact);

    let mut m = Model::new();
    let b = m.add(Box::new(OdeBlock::new(HeatRod::new("rod"), rk4)));
    let mut built_in = Simulator::new(m, 0.0).unwrap();
    run(&mut built_in, 2.0, 0.05);
    let t = built_in.output(b, "temperature").unwrap();
    let x = built_in.output(b, "position").unwrap();
    let built_in_error = t
        .iter()
        .zip(x)
        .map(|(t, x)| (t - exact(*x)).abs())
        .fold(0.0, f64::max);
    // The peak is 33.3 K by then; both stay within 0.05 K (0.15%) of the
    // exact temperature everywhere (measured 2026-10-28: 0.033 K typed,
    // 0.031 K built in).
    assert!(typed_error < 0.05, "typed {typed_error}");
    assert!(built_in_error < 0.05, "built in {built_in_error}");
}

/// The discrete decay rate of the held sine mode on `cells` cells.
fn discrete_rate(cells: usize) -> f64 {
    let h = LENGTH / cells as f64;
    -ALPHA * 4.0 / (h * h) * (PI * h / (2.0 * LENGTH)).sin().powi(2)
}

#[test]
fn implicit_methods_have_their_order_in_time_on_the_grid() {
    // As for RK4: against the exact solution of the discretised system.
    let (cells, t_end) = (20, 2.0);
    let mode = &MODES[0];
    let exact = |x: f64| (mode.shape)(x) * (discrete_rate(cells) * t_end).exp();
    for (method, low, high) in [
        (Integrator::BackwardEuler { substeps: 1 }, 1.8, 2.2),
        (Integrator::TrBdf2 { substeps: 1 }, 3.6, 4.4),
    ] {
        let errors: Vec<f64> = [0.4, 0.2, 0.1]
            .into_iter()
            .map(|dt| {
                let (mut sim, id) = rod(cells, mode.start, mode.edges, method);
                run(&mut sim, t_end, dt);
                max_error(&sim, id, exact)
            })
            .collect();
        for w in errors.windows(2) {
            let ratio = w[0] / w[1];
            assert!(
                (low..high).contains(&ratio),
                "{method:?}: errors {errors:?}, ratio {ratio}"
            );
        }
    }
}

#[test]
fn implicit_methods_take_steps_far_beyond_the_explicit_limit() {
    // 50 cells: RK4 is stable only below about 0.7 h²/α = 0.028 s. A slow
    // mode plus a fast one (which decays at 237 /s), with steps of 1 s.
    let cells = 50;
    let start = "amp*(sin(pi*x/length) + 0.1*sin(49*pi*x/length))";
    let edges = "edge T: x- = 0, x+ = 0";
    let t_end = 5.0;
    let exact = |x: f64| (PI * x / LENGTH).sin() * (-ALPHA * PI * PI * t_end).exp();
    let error = |method| {
        let (mut sim, id) = rod(cells, start, edges, method);
        run(&mut sim, t_end, 1.0);
        max_error(&sim, id, exact)
    };
    // The fast mode is gone (L-stable); the slow one is followed to the
    // method's accuracy at this step.
    let be = error(Integrator::BackwardEuler { substeps: 1 });
    let tr = error(Integrator::TrBdf2 { substeps: 1 });
    assert!(be < 0.02, "backward Euler {be}");
    assert!(tr < 1e-4, "TR-BDF2 {tr}");
    let rk4 = error(Integrator::Rk4 { substeps: 1 });
    assert!(!rk4.is_finite() || rk4 > 1e6, "RK4 stayed bounded: {rk4}");
}

#[test]
fn typed_equations_give_the_jacobian_structure_exactly() {
    use simsandbox_eq::OdeSystem;
    let set = EquationSet::parse(
        "rod",
        &lines(5, "amp*x/length", "edge T: x periodic\nmean_T = mean(T)"),
    )
    .unwrap();
    let rows = set.sparsity().unwrap();
    assert_eq!(rows[0], [0, 1, 4]);
    assert_eq!(rows[2], [1, 2, 3]);
    assert_eq!(rows[4], [0, 3, 4]);
}

#[test]
fn implicit_runs_rewind_and_replay_bit_for_bit() {
    let (mut sim, id) = rod(
        40,
        "amp*sin(pi*x/length)",
        "edge T: x- = 0, x+ = 0",
        Integrator::TrBdf2 { substeps: 2 },
    );
    run(&mut sim, 1.0, 0.1);
    let snap = sim.snapshot();
    run(&mut sim, 1.0, 0.1);
    let first: Vec<u64> = sim
        .output(id, "T")
        .unwrap()
        .iter()
        .map(|v| v.to_bits())
        .collect();
    sim.restore(&snap);
    run(&mut sim, 1.0, 0.1);
    let again: Vec<u64> = sim
        .output(id, "T")
        .unwrap()
        .iter()
        .map(|v| v.to_bits())
        .collect();
    assert_eq!(first, again);
    // A fresh run of the whole gives the same bits too.
    let (mut fresh, f) = rod(
        40,
        "amp*sin(pi*x/length)",
        "edge T: x- = 0, x+ = 0",
        Integrator::TrBdf2 { substeps: 2 },
    );
    run(&mut fresh, 2.0, 0.1);
    let fresh: Vec<u64> = fresh
        .output(f, "T")
        .unwrap()
        .iter()
        .map(|v| v.to_bits())
        .collect();
    assert_eq!(first, fresh);
}

#[test]
fn the_built_in_rod_runs_implicitly_by_probing_its_structure() {
    // eq.heat_rod does not tell its structure; it is probed once.
    let mut m = Model::new();
    let b = m.add(Box::new(OdeBlock::new(
        HeatRod::new("rod"),
        Integrator::TrBdf2 { substeps: 1 },
    )));
    let mut sim = Simulator::new(m, 0.0).unwrap();
    run(&mut sim, 2.0, 0.5);
    let exact = |x: f64| gaussian_on_rod(x, 2.0, 100.0, 0.5, 0.1, LENGTH);
    let t = sim.output(b, "temperature").unwrap();
    let x = sim.output(b, "position").unwrap();
    let error = t
        .iter()
        .zip(x)
        .map(|(t, x)| (t - exact(*x)).abs())
        .fold(0.0, f64::max);
    // Steps of 0.5 s, 18 times RK4's limit; the peak is 33.3 K.
    assert!(error < 0.2, "{error}");
}

#[test]
fn a_2d_mode_decays_at_the_exact_rate_with_second_order_in_space() {
    // sin(πx)·sin(πy) on the unit square held at 0, decaying as
    // exp(−2απ² t).
    let t_end = 1.0;
    let exact =
        |x: f64, y: f64| (PI * x).sin() * (PI * y).sin() * (-2.0 * ALPHA * PI * PI * t_end).exp();
    let errors: Vec<f64> = [8, 16, 32]
        .into_iter()
        .map(|cells| {
            let lines: Vec<String> = [
                format!("grid x [m] = 0 .. 1, {cells}"),
                format!("grid y [m] = 0 .. 1, {cells}"),
                format!("param alpha [m^2/s] = {ALPHA}"),
                "param amp [K] = 1".into(),
                "param l [m] = 1".into(),
                "state T[x, y] [K] = amp*sin(pi*x/l)*sin(pi*y/l)".into(),
                "edge T: x- = 0, x+ = 0, y- = 0, y+ = 0".into(),
                "dT/dt = alpha*lap(T)".into(),
            ]
            .into();
            let set = EquationSet::parse("plate", &lines).unwrap();
            let mut m = Model::new();
            let id = m.add(Box::new(OdeBlock::new(
                set,
                Integrator::Rk4 { substeps: 1 },
            )));
            let mut sim = Simulator::new(m, 0.0).unwrap();
            run(&mut sim, t_end, 0.01);
            let t = sim.output(id, "T").unwrap();
            let (x, y) = (sim.output(id, "x").unwrap(), sim.output(id, "y").unwrap());
            t.iter()
                .zip(x.iter().zip(y))
                .map(|(t, (x, y))| (t - exact(*x, *y)).abs())
                .fold(0.0, f64::max)
        })
        .collect();
    for w in errors.windows(2) {
        let ratio = w[0] / w[1];
        assert!(
            (3.8..4.2).contains(&ratio),
            "errors {errors:?}, ratio {ratio}"
        );
    }
}
