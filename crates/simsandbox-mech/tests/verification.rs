//! The mechanism's verification (ADR 0103 point 7, the verification
//! policy): the articulated-body algorithm against an independent route and
//! against equations derived by hand, against closed-form solutions, its
//! conservation laws, and exact rewind.

use simsandbox_core::block::{Block, StepContext, StepIo};
use simsandbox_core::registry::BlockConfig;
use simsandbox_mech::block::{build, Mechanism};
use simsandbox_mech::rows;
use simsandbox_mech::tree::{self, At, Kind, Load, Numbers, Scratch, Tree};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// A small generator with a fixed sequence, so every run tests the same.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

fn mechanism(lines: &[&str], settings: &[(&str, &str)]) -> Mechanism {
    let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let rows = rows::read(&lines).unwrap();
    let settings: BTreeMap<String, String> = settings
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Mechanism::new("m", &rows, &settings).unwrap()
}

/// The largest difference relative to the larger of 1 and the values.
fn rel_diff(a: &[f64], b: &[f64]) -> f64 {
    let scale = a.iter().chain(b).fold(1.0f64, |m, v| m.max(v.abs()));
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
        / scale
}

/// RK4 on `f` from `x` for `steps` steps of `h`.
fn rk4(f: &dyn Fn(&[f64], &mut [f64]), x: &mut [f64], h: f64, steps: usize) {
    let n = x.len();
    let (mut k1, mut k2, mut k3, mut k4, mut y) = (
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
    );
    for _ in 0..steps {
        f(x, &mut k1);
        (0..n).for_each(|i| y[i] = x[i] + 0.5 * h * k1[i]);
        f(&y, &mut k2);
        (0..n).for_each(|i| y[i] = x[i] + 0.5 * h * k2[i]);
        f(&y, &mut k3);
        (0..n).for_each(|i| y[i] = x[i] + h * k3[i]);
        f(&y, &mut k4);
        (0..n).for_each(|i| x[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]));
    }
}

/// The block's right-hand side as a closure, with inputs `u`.
fn rates<'a>(m: &'a Mechanism, u: &'a [f64], p: &'a [f64]) -> impl Fn(&[f64], &mut [f64]) + 'a {
    use simsandbox_core::block::Equations;
    move |x: &[f64], dx: &mut [f64]| m.rhs(0.0, x, u, p, dx)
}

#[test]
fn aba_agrees_with_crba_and_rnea_on_random_trees() {
    let mut r = Lcg(7);
    for size in [1, 2, 5, 12, 40] {
        for _ in 0..50 {
            let parents: Vec<(Option<usize>, Kind)> = (0..size)
                .map(|i| {
                    let parent = (i > 0 && r.next() < 0.85).then(|| (r.next() * i as f64) as usize);
                    let kind = match r.next() {
                        v if v < 0.6 => Kind::Revolute,
                        v if v < 0.9 => Kind::Prismatic,
                        _ => Kind::Fixed,
                    };
                    (parent, kind)
                })
                .collect();
            // Every chain from the ground starts with a joint that moves,
            // and each body has mass and inertia, so M is positive definite.
            let parents: Vec<_> = parents
                .into_iter()
                .map(|(p, k)| {
                    (
                        p,
                        if p.is_none() && k == Kind::Fixed {
                            Kind::Revolute
                        } else {
                            k
                        },
                    )
                })
                .collect();
            let t = Tree::new(&parents).unwrap();
            let nums: Vec<Numbers> = (0..size)
                .map(|_| Numbers {
                    mass: r.range(0.1, 2.0),
                    inertia: r.range(0.01, 0.5),
                    centre: [r.range(-0.5, 0.5), r.range(-0.5, 0.5)],
                    at: [r.range(-1.0, 1.0), r.range(-1.0, 1.0)],
                    turn: r.range(-PI, PI),
                    damping: r.range(0.0, 0.3),
                    stiffness: r.range(0.0, 2.0),
                    rest: r.range(-0.5, 0.5),
                })
                .collect();
            let dof = t.dof();
            let q: Vec<f64> = (0..dof).map(|_| r.range(-2.0, 2.0)).collect();
            let qd: Vec<f64> = (0..dof).map(|_| r.range(-2.0, 2.0)).collect();
            let tau: Vec<f64> = (0..dof).map(|_| r.range(-3.0, 3.0)).collect();
            let loads: Vec<Load> = (0..2)
                .map(|_| Load {
                    body: (r.next() * size as f64) as usize,
                    force: [r.range(-5.0, 5.0), r.range(-5.0, 5.0)],
                    torque: r.range(-1.0, 1.0),
                })
                .collect();
            let at = At {
                nums: &nums,
                gravity: [r.range(-2.0, 2.0), -9.81],
                q: &q,
                qd: &qd,
                tau: &tau,
                loads: &loads,
            };
            let (mut a, mut b) = (vec![0.0; dof], vec![0.0; dof]);
            let mut w = Scratch::default();
            tree::aba(&t, at, &mut a, &mut w);
            tree::crba_rnea(&t, at, &mut b, &mut w);
            assert!(
                rel_diff(&a, &b) < 1e-12,
                "{size} bodies: {a:?} against {b:?}"
            );
        }
    }
}

const ROD_L: f64 = 1.0;
const ROD_M: f64 = 1.0;

/// Two uniform rods hinged end to end, hanging from the origin.
fn double_pendulum() -> Mechanism {
    mechanism(
        &[
            r#"{"name": "upper", "parent": "ground", "joint": "shoulder", "kind": "revolute", "mass": 1.0, "centre": [0, -0.5], "inertia": 0.08333333333333333}"#,
            r#"{"name": "lower", "parent": "upper", "joint": "elbow", "kind": "revolute", "at": [0, -1], "mass": 1.0, "centre": [0, -0.5], "inertia": 0.08333333333333333}"#,
        ],
        &[],
    )
}

/// The double pendulum's accelerations from its Lagrange equations,
/// written by hand in absolute angles φ from straight down.
fn double_lagrange(s: &[f64; 4], g: f64) -> [f64; 2] {
    let [p1, p2, w1, w2] = *s;
    let (m, l) = (ROD_M, ROD_L);
    let d = p1 - p2;
    let m11 = (m / 3.0 + m) * l * l;
    let m12 = m * l * (l / 2.0) * d.cos();
    let m22 = m * l * l / 3.0;
    let r1 = -m * l * (l / 2.0) * d.sin() * w2 * w2 - (m / 2.0 + m) * g * l * p1.sin();
    let r2 = m * l * (l / 2.0) * d.sin() * w1 * w1 - m * g * (l / 2.0) * p2.sin();
    let det = m11 * m22 - m12 * m12;
    [(r1 * m22 - r2 * m12) / det, (m11 * r2 - m12 * r1) / det]
}

#[test]
fn the_double_pendulum_matches_its_lagrange_equations() {
    let m = double_pendulum();
    let p = m.defaults();
    let mut r = Lcg(3);
    for _ in 0..500 {
        // Relative joint angles; absolute φ₂ = q₁ + q₂.
        let x = [
            r.range(-3.0, 3.0),
            r.range(-3.0, 3.0),
            r.range(-4.0, 4.0),
            r.range(-4.0, 4.0),
        ];
        let mut qdd = [0.0; 2];
        m.accelerations(&x, &[], &p, &mut qdd);
        let [a1, a2] = double_lagrange(&[x[0], x[0] + x[1], x[2], x[2] + x[3]], 9.81);
        assert!(
            rel_diff(&qdd, &[a1, a2 - a1]) < 1e-13,
            "{qdd:?} against {:?}",
            [a1, a2 - a1]
        );
    }
}

#[test]
fn the_double_pendulum_converges_at_fourth_order() {
    let m = double_pendulum();
    let p = m.defaults();
    let s0 = [1.0, -0.5, 0.0, 0.0];
    // The reference: the hand-written equations by RK4 at 1e−5 s, in
    // absolute angles.
    let reference = {
        let mut s = [s0[0], s0[0] + s0[1], 0.0, 0.0];
        let f = |s: &[f64], ds: &mut [f64]| {
            let a = double_lagrange(&[s[0], s[1], s[2], s[3]], 9.81);
            ds.copy_from_slice(&[s[2], s[3], a[0], a[1]]);
        };
        rk4(&f, &mut s, 1e-5, 100_000);
        [s[0], s[1] - s[0]]
    };
    let f = rates(&m, &[], &p);
    let error = |h: f64| {
        let mut x = s0;
        rk4(&f, &mut x, h, (1.0 / h).round() as usize);
        rel_diff(&x[..2], &reference)
    };
    let (e1, e2) = (error(0.002), error(0.001));
    let order = (e1 / e2).log2();
    assert!(e2 < 1e-9, "{e2}");
    assert!((order - 4.0).abs() < 0.2, "order {order}");
}

/// A cart on a rail and a pole hinged on it, from the file form's example.
fn cartpole() -> Mechanism {
    mechanism(
        &[
            r#"{"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0}"#,
            r#"{"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute", "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083}"#,
        ],
        &[("actuated", "slider, hinge"), ("loaded", "pole")],
    )
}

#[test]
fn the_cartpole_matches_its_exact_equations() {
    let m = cartpole();
    let p = m.defaults();
    let (mc, mp, l, i, g) = (1.0, 0.1, 0.5, 0.0083, 9.81);
    let mut r = Lcg(11);
    for _ in 0..500 {
        let x = [
            r.range(-1.0, 1.0),
            r.range(-PI, PI),
            r.range(-2.0, 2.0),
            r.range(-5.0, 5.0),
        ];
        // Cart force, hinge torque, then the pole's load at its centre.
        let u = [
            r.range(-10.0, 10.0),
            r.range(-1.0, 1.0),
            r.range(-2.0, 2.0),
            r.range(-2.0, 2.0),
            r.range(-0.5, 0.5),
        ];
        let mut qdd = [0.0; 2];
        m.accelerations(&x, &u, &p, &mut qdd);
        // Lagrange's equations with θ counter-clockwise from upright: the
        // pole's centre at (x − l sin θ, l cos θ). The load's generalized
        // forces are its work along each coordinate.
        let (th, w) = (x[1], x[3]);
        let (s, c) = th.sin_cos();
        let (fx, fy, n) = (u[2], u[3], u[4]);
        let qx = u[0] + fx;
        let qt = u[1] + n + fx * (-l * c) + fy * (-l * s);
        let (a11, a12, a22) = (mc + mp, -mp * l * c, i + mp * l * l);
        let b1 = qx - mp * l * s * w * w;
        let b2 = qt + mp * g * l * s;
        let det = a11 * a22 - a12 * a12;
        let exact = [(b1 * a22 - a12 * b2) / det, (a11 * b2 - a12 * b1) / det];
        assert!(rel_diff(&qdd, &exact) < 1e-13, "{qdd:?} against {exact:?}");
    }
}

/// The complete elliptic integral of the first kind, by the
/// arithmetic–geometric mean.
fn elliptic_k(k: f64) -> f64 {
    let (mut a, mut b) = (1.0, (1.0 - k * k).sqrt());
    for _ in 0..30 {
        (a, b) = (0.5 * (a + b), (a * b).sqrt());
    }
    PI / (2.0 * a)
}

#[test]
fn the_pendulum_s_period_matches_the_elliptic_integral() {
    // A point mass on a massless rod of 1 m, from rest at `a`: it swings to
    // −a in half a period, T = 4 K(sin(a/2)) / ω₀.
    let m = mechanism(
        &[
            r#"{"name": "bob", "parent": "ground", "joint": "pivot", "kind": "revolute", "mass": 1.0, "centre": [0, -1]}"#,
        ],
        &[],
    );
    let p = m.defaults();
    let f = rates(&m, &[], &p);
    for a in [0.1f64, 2.0] {
        let exact = 4.0 * elliptic_k((a / 2.0).sin()) / 9.81f64.sqrt();
        let h = 1e-3;
        let mut x = [a, 0.0];
        let mut t = 0.0;
        // Step until the rate turns from negative back to 0, then find the
        // instant by Newton's method on the cubic Hermite through the step.
        loop {
            let before = x;
            rk4(&f, &mut x, h, 1);
            if t > 0.0 && before[1] < 0.0 && x[1] >= 0.0 {
                let (mut d0, mut d1) = ([0.0; 2], [0.0; 2]);
                f(&before, &mut d0);
                f(&x, &mut d1);
                let (y0, y1, m0, m1) = (before[1], x[1], d0[1] * h, d1[1] * h);
                let mut s: f64 = 0.5;
                for _ in 0..50 {
                    let (s2, s3) = (s * s, s * s * s);
                    let y = (2.0 * s3 - 3.0 * s2 + 1.0) * y0
                        + (s3 - 2.0 * s2 + s) * m0
                        + (-2.0 * s3 + 3.0 * s2) * y1
                        + (s3 - s2) * m1;
                    let dy = (6.0 * s2 - 6.0 * s) * y0
                        + (3.0 * s2 - 4.0 * s + 1.0) * m0
                        + (-6.0 * s2 + 6.0 * s) * y1
                        + (3.0 * s2 - 2.0 * s) * m1;
                    s -= y / dy;
                }
                let half = t + s * h;
                assert!(
                    ((2.0 * half - exact) / exact).abs() < 1e-10,
                    "a = {a}: {} against {exact}",
                    2.0 * half
                );
                break;
            }
            t += h;
        }
    }
}

#[test]
fn a_damped_slider_matches_the_linear_solution() {
    // m ẍ + c ẋ + k x = 0 along a rail, gravity across it.
    let m = mechanism(
        &[
            r#"{"name": "mass", "parent": "ground", "joint": "rail", "kind": "prismatic", "mass": 2.0, "damping": 0.4, "stiffness": 8.0, "start": [0.1, 0]}"#,
        ],
        &[],
    );
    let p = m.defaults();
    let mut x = [0.0; 2];
    use simsandbox_eq::OdeSystem;
    m.initial_state(&p, &mut x);
    let (mass, c, k): (f64, f64, f64) = (2.0, 0.4, 8.0);
    let (w0, zeta) = ((k / mass).sqrt(), c / (2.0 * (k * mass).sqrt()));
    let wd = w0 * (1.0 - zeta * zeta).sqrt();
    let exact = |t: f64| {
        let e = (-zeta * w0 * t).exp();
        0.1 * e * ((wd * t).cos() + zeta * w0 / wd * (wd * t).sin())
    };
    let f = rates(&m, &[], &p);
    let mut worst = 0.0f64;
    for step in 1..=5000 {
        rk4(&f, &mut x, 1e-3, 1);
        worst = worst.max((x[0] - exact(step as f64 * 1e-3)).abs());
    }
    assert!(worst < 1e-12, "{worst}");
}

#[test]
fn energy_and_momentum_are_kept() {
    // A chain of five links on a floating base (two massless sliders) in
    // zero gravity, every joint moving: momentum is kept; and the double
    // pendulum with springs in its joints keeps its energy.
    let mut lines = vec![
        r#"{"name": "bx", "parent": "ground", "joint": "jx", "kind": "prismatic", "mass": 0, "start": [0, 0.3]}"#.to_string(),
        r#"{"name": "by", "parent": "bx", "joint": "jy", "kind": "prismatic", "turn": 1.5707963267948966, "mass": 0, "start": [0, -0.2]}"#.to_string(),
    ];
    for i in 0..5 {
        let parent = if i == 0 {
            "by".to_string()
        } else {
            format!("l{}", i - 1)
        };
        let at = if i == 0 { "[0, 0]" } else { "[0.1, 0]" };
        lines.push(format!(
            r#"{{"name": "l{i}", "parent": "{parent}", "joint": "j{i}", "kind": "revolute", "at": {at}, "mass": 0.1, "centre": [0.05, 0], "inertia": 1e-4, "start": [{}, {}]}}"#,
            0.3 * i as f64,
            1.0 - 0.4 * i as f64
        ));
    }
    let rows = rows::read(&lines).unwrap();
    let m = Mechanism::new("m", &rows, &BTreeMap::new()).unwrap();
    let mut p = m.defaults();
    p[0] = 0.0;
    let mut x = vec![0.0; 2 * m.tree().dof()];
    use simsandbox_eq::OdeSystem;
    m.initial_state(&p, &mut x);
    let momentum = |x: &[f64]| {
        let n = m.tree().dof();
        tree::momentum(
            m.tree(),
            &numbers(&m, &p),
            &x[..n],
            &x[n..],
            &mut Scratch::default(),
        )
    };
    let (m0, e0) = (momentum(&x), m.energy(&x, &p));
    let f = rates(&m, &[], &p);
    rk4(&f, &mut x, 1e-3, 2000);
    let (m1, e1) = (momentum(&x), m.energy(&x, &p));
    assert!(rel_diff(&m0, &m1) < 1e-9, "{m0:?} → {m1:?}");
    assert!(((e1 - e0) / e0).abs() < 1e-9, "{e0} → {e1}");

    let d = mechanism(
        &[
            r#"{"name": "upper", "parent": "ground", "joint": "shoulder", "kind": "revolute", "mass": 1.0, "centre": [0, -0.5], "inertia": 0.0833, "stiffness": 3, "rest": 0.2, "start": [1.0, 0]}"#,
            r#"{"name": "lower", "parent": "upper", "joint": "elbow", "kind": "revolute", "at": [0, -1], "mass": 1.0, "centre": [0, -0.5], "inertia": 0.0833, "stiffness": 1, "start": [-0.5, 0.5]}"#,
        ],
        &[],
    );
    let p = d.defaults();
    let mut x = [0.0; 4];
    d.initial_state(&p, &mut x);
    let e0 = d.energy(&x, &p);
    rk4(&rates(&d, &[], &p), &mut x, 1e-3, 10_000);
    let e1 = d.energy(&x, &p);
    assert!(((e1 - e0) / e0).abs() < 1e-9, "{e0} → {e1}");
}

/// The floating chain's numbers at `p`, read back by the parameters'
/// names (ADR 0105 point 3).
fn numbers(m: &Mechanism, p: &[f64]) -> Vec<Numbers> {
    use simsandbox_eq::OdeSystem;
    let spec = m.spec();
    let get = |name: &str| {
        spec.params
            .iter()
            .position(|q| q.name == name)
            .map(|k| p[k])
    };
    let mut out = Vec::new();
    for (b, j) in [
        ("bx", "jx"),
        ("by", "jy"),
        ("l0", "j0"),
        ("l1", "j1"),
        ("l2", "j2"),
        ("l3", "j3"),
        ("l4", "j4"),
    ] {
        out.push(Numbers {
            mass: get(&format!("{b}_mass")).unwrap(),
            inertia: get(&format!("{b}_inertia")).unwrap(),
            centre: [
                get(&format!("{b}_centre_x")).unwrap(),
                get(&format!("{b}_centre_y")).unwrap(),
            ],
            at: [
                get(&format!("{j}_at_x")).unwrap(),
                get(&format!("{j}_at_y")).unwrap(),
            ],
            turn: get(&format!("{j}_turn")).unwrap(),
            ..Numbers::default()
        });
    }
    out
}

#[test]
fn poses_follow_the_joints() {
    let m = cartpole();
    let p = m.defaults();
    let x = [0.3, 0.4, 1.5, -2.0];
    let mut poses = Vec::new();
    m.poses(&x, &p, &mut poses);
    let (cart, pole) = (poses[0], poses[1]);
    assert_eq!((cart.x, cart.y, cart.angle, cart.vx), (0.3, 0.0, 0.0, 1.5));
    // The pole's frame is at the cart's point, turned by the hinge.
    assert_eq!(
        (pole.x, pole.y, pole.angle, pole.vx, pole.omega),
        (0.3, 0.0, 0.4, 1.5, -2.0)
    );
}

/// The block from a model file's rows, as the registry builds it.
fn block(lines: &[&str], settings: &[(&str, &str)], method: &str) -> Box<dyn Block> {
    let c = BlockConfig {
        method: simsandbox_core::registry::MethodSettings::new(method),
        definition: lines.iter().map(|l| l.to_string()).collect(),
        settings: settings
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..BlockConfig::default()
    };
    build("cp", &c).unwrap()
}

/// Steps the block `steps` times from `state`, its inputs `u`.
fn run(b: &mut dyn Block, state: &mut [f64], t0: usize, steps: usize, u: &[f64]) -> Vec<Vec<f64>> {
    let p: Vec<f64> = b.spec().params.iter().map(|q| q.default).collect();
    let mut y = vec![0.0; b.spec().output_len()];
    let mut rows = vec![Vec::new(); 6];
    let dt = 1e-3;
    for k in t0..t0 + steps {
        b.do_step(
            StepContext {
                t: k as f64 * dt,
                dt,
                step: k as u64,
                seed: 0,
            },
            StepIo {
                params: &p,
                inputs: u,
                state,
                outputs: &mut y,
                tables: &mut [],
                population_inputs: &[],
                population_outputs: &mut rows,
            },
        )
        .unwrap();
    }
    rows
}

#[test]
fn a_run_restored_into_a_fresh_block_gives_the_same_bits() {
    let lines = [
        r#"{"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0}"#,
        r#"{"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute", "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083, "damping": 0.01, "start": [0.1, 0]}"#,
    ];
    for method in ["rk4", "dopri5", "trbdf2", "backward_euler"] {
        let settings = [("actuated", "slider")];
        let mut b = block(&lines, &settings, method);
        let p: Vec<f64> = b.spec().params.iter().map(|q| q.default).collect();
        let mut x = vec![0.0; 4];
        b.init(&p, &mut x).unwrap();
        run(b.as_mut(), &mut x, 0, 500, &[0.5]);
        let snapshot = x.clone();
        let rows = run(b.as_mut(), &mut x, 500, 500, &[0.5]);
        let mut y = snapshot;
        let mut fresh = block(&lines, &settings, method);
        let again = run(fresh.as_mut(), &mut y, 500, 500, &[0.5]);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&x), bits(&y), "{method}");
        assert_eq!(rows, again, "{method}");
        // The rows: two bodies, ids 0 and 1, after the version and count.
        assert_eq!(&rows[0][..4], &[0.0, 2.0, 0.0, 1.0]);
        assert_eq!(rows[0][4], x[0], "the cart's x is the slider's position");
    }
}

#[test]
fn check_refuses_what_point_6_lists() {
    let ok = r#"{"name": "a", "parent": "ground", "joint": "ja", "kind": "revolute", "mass": 1, "centre": [0, -1]}"#;
    type Case<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)], &'a str);
    let cases: [Case; 13] = [
        (&[], &[], "lists its bodies"),
        (&["dx/dt = 1"], &[], "not by equations"),
        (
            &[r#"{"name": "a", "parent": "ground", "joint": "ja", "kind": "hinge", "mass": 1}"#],
            &[],
            "bodies[0]",
        ),
        (
            &[
                r#"{"name": "a", "parent": "ground", "joint": "ja", "kind": "revolute", "mass": 1, "colour": 2}"#,
            ],
            &[],
            "colour",
        ),
        (
            &[
                ok,
                r#"{"name": "b", "parent": "c", "joint": "jb", "kind": "revolute", "mass": 1}"#,
            ],
            &[],
            "bodies[1].parent",
        ),
        (
            &[
                ok,
                r#"{"name": "b", "parent": "a", "joint": "a", "kind": "revolute", "mass": 1}"#,
            ],
            &[],
            "bodies[1].joint: 'a' is used twice",
        ),
        (
            &[
                r#"{"name": "ground", "parent": "ground", "joint": "j", "kind": "revolute", "mass": 1}"#,
            ],
            &[],
            "bodies[0].name",
        ),
        (
            &[
                r#"{"name": "a b", "parent": "ground", "joint": "j", "kind": "revolute", "mass": 1}"#,
            ],
            &[],
            "not a name",
        ),
        (
            &[r#"{"name": "a", "parent": "ground", "joint": "j", "kind": "revolute", "mass": -1}"#],
            &[],
            "bodies[0].mass",
        ),
        (
            &[
                r#"{"name": "a", "parent": "ground", "joint": "j", "kind": "fixed", "mass": 1, "start": [1, 0]}"#,
            ],
            &[],
            "fixed joint",
        ),
        (
            &[r#"{"name": "a", "parent": "ground", "joint": "j", "kind": "revolute", "mass": 1}"#],
            &[],
            "joint 'j'",
        ),
        (&[ok], &[("actuated", "a")], "settings.actuated"),
        (&[ok], &[("colour", "red")], "settings.colour"),
    ];
    for (lines, settings, wanted) in cases {
        let c = BlockConfig {
            definition: lines.iter().map(|l| l.to_string()).collect(),
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..BlockConfig::default()
        };
        let e = build("m", &c)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            e.contains(wanted),
            "{lines:?} {settings:?}: '{e}' lacks '{wanted}'"
        );
    }
}
