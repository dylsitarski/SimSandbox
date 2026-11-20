//! Cases for the native solver beyond the shared ones: the 50- and 200-link
//! chains' stability and energy (with the other engines), ABA against CRBA,
//! the double pendulum against its Lagrange equations, conservation, joint
//! damping, and a controller fused with the plant in one ODE.

use crate::common::{cartpole as cp, time, G};
use crate::native_2d::{
    self as nat, double, method_name, Algo, Mechanism, Method, PointForce, Scratch, Stepper,
};
use crate::rapier_2d::Model;
use crate::{joint_variants, native_variants, quick, sci};
use simsandbox_eq::implicit::Workspace;
use simsandbox_eq::Integrator;
use std::hint::black_box;

const RK4: Method = Method::Eq(Integrator::Rk4 { substeps: 1 });
const DOPRI: Method = Method::Eq(Integrator::Dopri5 {
    rtol: 1e-8,
    atol: 1e-10,
});
const TRBDF2: Method = Method::Eq(Integrator::TrBdf2 { substeps: 1 });
const METHODS: [Method; 4] = [Method::SemiImplicitEuler, RK4, DOPRI, TRBDF2];

/// A small deterministic generator (xorshift64*), for random states.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

// ------------------------------------------------- chains: stability, energy

/// Energy of the spike's chain from 2D rows (x, y, angle, vx, vy, ω per
/// link), potential zero at the pivot's height.
fn chain_energy_rows(r: &[f64]) -> f64 {
    let (m, i) = (
        nat::CHAIN_M,
        nat::CHAIN_M * nat::CHAIN_LEN * nat::CHAIN_LEN / 12.0,
    );
    r.chunks(6)
        .map(|c| 0.5 * m * (c[3] * c[3] + c[4] * c[4]) + 0.5 * i * c[5] * c[5] + m * G * c[1])
        .sum()
}

pub fn chain_stability() {
    println!("## Chains: stability and energy over 10 s\n");
    println!(
        "The speed case's chain (links of 0.1 m and 0.1 kg, uniform rods, from horizontal at rest, no damping) for 10 s at 1 kHz. \
         Energy every 10 steps from the bodies' rows (MuJoCo's from its joint coordinates through the native forward kinematics), \
         against E_ref, the potential energy the chain loses by hanging straight down (m g L n²/2). \
         'Failed' is the first time an engine warned (MuJoCo resets an unstable model), a step failed or a value stopped being a number. \
         Rapier 3D is left out (its 2D rows agree with 2D elsewhere); Rapier multibody is left out at 200 links (about a minute a simulated second), and typed TR-BDF2 (85 ms a step at 50 links).\n"
    );
    println!("| Links | Engine | max abs(E − E0)/E_ref | (E_end − E0)/E_ref | Failed at | Largest speed at the end | µs per step (with the energy) |");
    println!("|---|---|---|---|---|---|---|");
    let dt = 0.001;
    let secs = if quick() { 2.0 } else { 10.0 };
    let steps = (secs / dt) as usize;
    for links in [50usize, 200] {
        let mech = nat::chain(links);
        let mut w = Scratch::default();
        let eref = nat::CHAIN_M * G * nat::CHAIN_LEN * (links * links) as f64 / 2.0;
        let makes = if links == 50 {
            joint_variants()
        } else {
            native_variants()
        };
        for make in makes {
            let mut sim = make(Model::Chain { links }, dt);
            let label = sim.label();
            if label.starts_with("Rapier 3D") || label == "Typed equations, trbdf2" {
                continue;
            }
            let energy = |r: &[f64], w: &mut Scratch| -> f64 {
                if label.starts_with("MuJoCo") {
                    mech.energy(&r[..links], &r[links..], w)
                } else {
                    chain_energy_rows(r)
                }
            };
            let e0 = energy(&sim.rows(), &mut w);
            let w0 = sim.warnings();
            let mut failed = None;
            let mut maxe = 0.0f64;
            let mut last = e0;
            let (_, wall) = time(|| {
                for k in 0..steps {
                    sim.step(&[]);
                    if failed.is_none() && sim.warnings() > w0 {
                        failed = Some((k + 1) as f64 * dt);
                    }
                    if k % 10 == 9 {
                        let e = energy(&sim.rows(), &mut w);
                        if e.is_finite() {
                            maxe = maxe.max((e - e0).abs());
                        } else {
                            maxe = f64::INFINITY;
                        }
                        last = e;
                    }
                }
            });
            let r = sim.rows();
            let speed = if label.starts_with("MuJoCo") {
                r[links..].iter().fold(0.0f64, |m, v| m.max(v.abs()))
            } else {
                r.chunks(6)
                    .flat_map(|c| c[3..6].to_vec())
                    .fold(0.0f64, |m, v| m.max(v.abs()))
            };
            println!(
                "| {links} | {label} | {:.2e} | {} | {} | {:.3} | {:.1} |",
                maxe / eref,
                sci((last - e0) / eref),
                failed.map_or("no".to_string(), |t| format!("**{t:.3} s**")),
                speed,
                wall / steps as f64 * 1e6
            );
        }
    }
    println!();
}

// ------------------------------------------------- the native solver's checks

pub fn native() {
    println!("## Native solver: checks and costs\n");
    cross_check();
    timing();
    double_pendulum();
    conservation();
    damping();
    fused();
}

fn random_tree(n: usize, rng: &mut Rng) -> Mechanism {
    let bodies = (0..n)
        .map(|i| {
            let parent = if i == 0 || rng.next() < 0.1 {
                None
            } else {
                Some((rng.next() * i as f64) as usize)
            };
            let joint = if rng.next() < 0.7 {
                nat::Joint::Revolute
            } else {
                let a = rng.range(-3.2, 3.2);
                nat::Joint::Prismatic {
                    axis: [a.cos(), a.sin()],
                }
            };
            nat::Body {
                parent,
                joint,
                at: [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)],
                rot: rng.range(-3.2, 3.2),
                mass: rng.range(0.1, 2.0),
                com: [rng.range(-0.5, 0.5), rng.range(-0.5, 0.5)],
                inertia: rng.range(0.01, 0.5),
                damping: rng.range(0.0, 0.3),
            }
        })
        .collect();
    Mechanism::new(bodies, [rng.range(-2.0, 2.0), -G])
}

fn cross_check() {
    println!("### ABA against CRBA + RNEA, and against equations written by hand\n");
    println!(
        "Random trees (each body's parent drawn among the earlier ones or the ground, 70 % revolute and 30 % prismatic joints on random axes, \
         random masses, centres, inertias, joint damping, a sideways gravity component), random states, joint forces and two point forces; \
         100 draws per size. Difference = max abs(q̈_ABA − q̈_other) / max(1, max abs(q̈)).\n"
    );
    println!("| Mechanisms | Draws | Largest difference |");
    println!("|---|---|---|");
    let mut rng = Rng(0x9e3779b97f4a7c15);
    for n in [2usize, 10, 50] {
        let mut worst = 0.0f64;
        for _ in 0..100 {
            let mech = random_tree(n, &mut rng);
            let q: Vec<f64> = (0..n).map(|_| rng.range(-3.0, 3.0)).collect();
            let qd: Vec<f64> = (0..n).map(|_| rng.range(-2.0, 2.0)).collect();
            let tau: Vec<f64> = (0..n).map(|_| rng.range(-5.0, 5.0)).collect();
            let ext: Vec<PointForce> = (0..2)
                .map(|_| PointForce {
                    body: (rng.next() * n as f64) as usize,
                    point: [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)],
                    force: [rng.range(-10.0, 10.0), rng.range(-10.0, 10.0)],
                })
                .collect();
            let (mut a, mut b) = (vec![0.0; n], vec![0.0; n]);
            let mut w = Scratch::default();
            mech.aba(&q, &qd, &tau, &ext, &mut a, &mut w);
            mech.crba_rnea(&q, &qd, &tau, &ext, &mut b, &mut w);
            let scale = a.iter().fold(1.0f64, |m, v| m.max(v.abs()));
            let d = a
                .iter()
                .zip(&b)
                .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
            worst = worst.max(d / scale);
        }
        println!(
            "| random trees of {n} bodies, ABA vs CRBA | 100 | {:.1e} |",
            worst
        );
    }
    // The cart-pole against the spike's exact equations (with a tip force),
    // and the double pendulum against its Lagrange equations.
    let mut wc = 0.0f64;
    let mut wd = 0.0f64;
    let cpm = nat::cartpole();
    let dpm = double::mechanism();
    let mut w = Scratch::default();
    for _ in 0..1000 {
        let s = [
            rng.range(-1.0, 1.0),
            rng.range(-2.0, 2.0),
            rng.range(-3.0, 3.0),
            rng.range(-5.0, 5.0),
        ];
        let (f, p) = (
            rng.range(-30.0, 30.0),
            [rng.range(-5.0, 5.0), rng.range(-5.0, 5.0)],
        );
        let (ex, et) = cp::accel(&s, f, p);
        let mut a = [0.0; 2];
        let ext = [PointForce {
            body: 1,
            point: [0.0, cp::LEN],
            force: p,
        }];
        cpm.aba(
            &[s[0], s[2]],
            &[s[1], s[3]],
            &[f, 0.0],
            &ext,
            &mut a,
            &mut w,
        );
        wc = wc.max((a[0] - ex).abs().max((a[1] - et).abs()) / ex.abs().max(et.abs()).max(1.0));
        let d = [
            rng.range(-3.0, 3.0),
            rng.range(-3.0, 3.0),
            rng.range(-5.0, 5.0),
            rng.range(-5.0, 5.0),
        ];
        let l = double::lagrange(&d);
        // Native: q₁ = φ₁, q₂ = φ₂ − φ₁; q̈₂ = φ̈₂ − φ̈₁.
        dpm.aba(
            &[d[0], d[1] - d[0]],
            &[d[2], d[3] - d[2]],
            &[0.0, 0.0],
            &[],
            &mut a,
            &mut w,
        );
        let (n1, n2) = (a[0], a[0] + a[1]);
        wd = wd.max((n1 - l[0]).abs().max((n2 - l[1]).abs()) / l[0].abs().max(l[1].abs()).max(1.0));
    }
    println!("| cart-pole, ABA vs the spike's exact equations (cart force and tip force) | 1000 | {wc:.1e} |");
    println!("| double pendulum, ABA vs its Lagrange equations by hand | 1000 | {wd:.1e} |");
    println!();
}

fn timing() {
    println!("### Cost: ABA against CRBA + RNEA\n");
    println!(
        "The speed case's chain at n links, one evaluation of q̈ from a fixed random state (mean over many), and a whole step at 1 kHz from the horizontal start \
         (RK4: four evaluations; semi-implicit Euler: one), mean over the first 1000 steps. One thread.\n"
    );
    println!("| Links | ABA: µs per q̈ | CRBA + RNEA: µs per q̈ | CRBA / ABA | ABA + RK4: µs per step | CRBA + RK4: µs per step | ABA + semi-implicit Euler: µs per step |");
    println!("|---|---|---|---|---|---|---|");
    let mut rng = Rng(42);
    for n in [2usize, 10, 50, 200] {
        let mech = nat::chain(n);
        let q: Vec<f64> = (0..n).map(|_| rng.range(-1.0, 1.0)).collect();
        let qd: Vec<f64> = (0..n).map(|_| rng.range(-1.0, 1.0)).collect();
        let tau = vec![0.0; n];
        let mut out = vec![0.0; n];
        let mut w = Scratch::default();
        let reps = (2_000_000 / (n * n).max(200)).max(200);
        let reps_aba = (4_000_000 / n).max(1000);
        let (_, ta) = time(|| {
            for _ in 0..reps_aba {
                mech.aba(black_box(&q), black_box(&qd), &tau, &[], &mut out, &mut w);
                black_box(&out);
            }
        });
        let (_, tc) = time(|| {
            for _ in 0..reps {
                mech.crba_rnea(black_box(&q), black_box(&qd), &tau, &[], &mut out, &mut w);
                black_box(&out);
            }
        });
        let ta = ta / reps_aba as f64 * 1e6;
        let tc = tc / reps as f64 * 1e6;
        let step = |algo: Algo, method: Method| {
            let mut s = Stepper::new(mech.clone(), algo, method, &[], &[]);
            let steps = 1000;
            let (_, t) = time(|| {
                for _ in 0..steps {
                    s.step(0.001);
                }
            });
            t / steps as f64 * 1e6
        };
        println!(
            "| {n} | {ta:.3} | {tc:.3} | {:.1} | {:.2} | {:.2} | {:.2} |",
            tc / ta,
            step(Algo::Aba, RK4),
            step(Algo::Crba, RK4),
            step(Algo::Aba, Method::SemiImplicitEuler)
        );
    }
    println!();
}

fn double_pendulum() {
    println!("### Double pendulum against its Lagrange equations\n");
    let s0 = [1.0, 0.5, 0.0, 0.0];
    let t_end: f64 = 5.0;
    println!(
        "Two uniform rods of 1 kg and 1 m, hinged end to end, from φ₁ = {} rad, φ₂ = {} rad (absolute angles from down) at rest, no damping, for {t_end} s. \
         Reference: the Lagrange equations written out by hand, classic RK4 at h = 1e−5 s (written separately from SimSandbox's). \
         Error = max over the run of abs(φ − φ_ref) for both angles, at the native run's step times.\n",
        s0[0], s0[1]
    );
    let h = 1e-5;
    let steps = (t_end / h).round() as usize;
    // Recorded every 0.5 ms; and the reference at 2h to show its own error.
    let reference = double::reference(s0, h, steps, 50);
    let ref2 = double::reference(s0, 2.0 * h, steps / 2, 25);
    let self_err = reference
        .iter()
        .zip(&ref2)
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0, f64::max);
    println!(
        "Reference at {t_end} s: φ₁ = {:.9}, φ₂ = {:.9}; the reference at h and at 2h differ by at most {self_err:.1e} rad.\n",
        reference[reference.len() - 1][0],
        reference[reference.len() - 1][1]
    );
    let dts = [0.004, 0.002, 0.001, 0.0005];
    print!("| Method |");
    for dt in dts {
        print!(" dt = {dt} s |");
    }
    println!(" Order (last two) |");
    println!("|---|{}---|", "---|".repeat(dts.len()));
    for method in METHODS {
        let mut errs = vec![];
        for dt in dts {
            let mut s = Stepper::new(
                double::mechanism(),
                Algo::Aba,
                method,
                &[s0[0], s0[1] - s0[0]],
                &[],
            );
            let n = (t_end / dt).round() as usize;
            let every = (dt / 0.0005).round() as usize;
            let mut e = 0.0f64;
            for k in 0..n {
                s.step(dt);
                let r = reference[(k + 1) * every];
                let (p1, p2) = (s.x[0], s.x[0] + s.x[1]);
                e = e.max((p1 - r[0]).abs().max((p2 - r[1]).abs()));
            }
            errs.push(e);
        }
        print!("| Native 2D, ABA, {} |", method_name(method));
        for e in &errs {
            print!(" {e:.2e} |");
        }
        println!(" {:.2} |", (errs[2] / errs[3]).log2());
    }
    println!();
}

fn conservation() {
    println!("### Conservation\n");
    let secs = if quick() { 10.0 } else { 100.0 };
    println!(
        "No damping, no forces but gravity, dt = 1 ms. Energy every step. The double pendulum of the check above for {secs} s; \
         a chain of 10 links (the speed case's) from horizontal for {secs} s; ΔE relative to the energy above the hanging rest.\n"
    );
    println!("| Model | Method | max abs(E − E0)/(E0 − E_rest) | (E_end − E0)/(E0 − E_rest) |");
    println!("|---|---|---|---|");
    let dt = 0.001;
    let n = (secs / dt) as usize;
    let cases: [(&str, Mechanism, Vec<f64>, f64); 2] = [
        (
            "double pendulum",
            double::mechanism(),
            vec![1.0, -0.5],
            // At rest hanging: both centres below the pivot.
            -(double::M1 * G * double::C1 + double::M2 * G * (double::L1 + double::C2)),
        ),
        (
            "chain of 10 links",
            nat::chain(10),
            vec![],
            -nat::CHAIN_M * G * nat::CHAIN_LEN * 100.0 / 2.0,
        ),
    ];
    for (name, mech, q0, e_rest) in cases {
        for method in METHODS {
            let mut s = Stepper::new(mech.clone(), Algo::Aba, method, &q0, &[]);
            let e0 = s.energy();
            let scale = e0 - e_rest;
            let mut maxe = 0.0f64;
            for _ in 0..n {
                s.step(dt);
                maxe = maxe.max((s.energy() - e0).abs());
            }
            println!(
                "| {name} | {} | {:.2e} | {} |",
                method_name(method),
                maxe / scale,
                sci((s.energy() - e0) / scale)
            );
        }
    }
    println!();

    let secs = if quick() { 2.0 } else { 10.0 };
    println!(
        "A free-floating chain in zero gravity: a floating base (two massless bodies on prismatic joints along x and y, then the first link on a revolute joint) \
         and 5 links of the speed case's, from a bent pose with every joint moving, for {secs} s at dt = 1 ms. \
         Linear momentum P and angular momentum L about the origin from the bodies' rows; changes relative to |P0| and |L0|.\n"
    );
    println!("| Method | max abs(ΔPx)/abs(P0) | max abs(ΔPy)/abs(P0) | max abs(ΔL)/abs(L0) | max abs(ΔE)/E0 |");
    println!("|---|---|---|---|---|");
    let mech = nat::floating_chain(5);
    let q0 = [0.0, 0.0, 0.3, -0.5, 0.8, -0.2, 0.4];
    let qd0 = [0.5, -0.3, 1.0, 2.0, -1.5, 0.7, 1.2];
    let n = (secs / dt) as usize;
    for method in METHODS {
        let mut s = Stepper::new(mech.clone(), Algo::Aba, method, &q0, &qd0);
        let p0 = s.momentum();
        let e0 = s.energy();
        let pn = p0[0].hypot(p0[1]);
        let mut d = [0.0f64; 4];
        for _ in 0..n {
            s.step(dt);
            let p = s.momentum();
            d[0] = d[0].max((p[0] - p0[0]).abs());
            d[1] = d[1].max((p[1] - p0[1]).abs());
            d[2] = d[2].max((p[2] - p0[2]).abs());
            d[3] = d[3].max((s.energy() - e0).abs());
        }
        println!(
            "| Native 2D, ABA, {} | {:.1e} | {:.1e} | {:.1e} | {:.1e} |",
            method_name(method),
            d[0] / pn,
            d[1] / pn,
            d[2] / p0[2].abs(),
            d[3] / e0
        );
    }
    println!(
        "\nP0 = ({:.4}, {:.4}) kg m/s, L0 = {:.4} kg m²/s, E0 = {:.4} J.\n",
        Stepper::new(mech.clone(), Algo::Aba, RK4, &q0, &qd0).momentum()[0],
        Stepper::new(mech.clone(), Algo::Aba, RK4, &q0, &qd0).momentum()[1],
        Stepper::new(mech.clone(), Algo::Aba, RK4, &q0, &qd0).momentum()[2],
        Stepper::new(mech.clone(), Algo::Aba, RK4, &q0, &qd0).energy()
    );
}

fn damping() {
    println!("### Joint damping\n");
    let b = 0.05;
    let a0 = 1e-5;
    let mut mech = nat::pendulum();
    mech.bodies[0].damping = b;
    let j = crate::common::pendulum::IC
        + crate::common::pendulum::M * crate::common::pendulum::L.powi(2);
    let w0 = crate::common::pendulum::omega0();
    let al = b / (2.0 * j);
    let wd = (w0 * w0 - al * al).sqrt();
    let exact = |t: f64| a0 * (-al * t).exp() * ((wd * t).cos() + al / wd * (wd * t).sin());
    println!(
        "The spike's pendulum with joint damping b = {b} N m s/rad, from {a0} rad (where the linear equation J θ̈ + b θ̇ + m g L θ = 0 is exact to about 2e−11), \
         10 s at dt = 1 ms, against θ = a e^(−αt) (cos ω_d t + (α/ω_d) sin ω_d t), α = b/(2J). Error = max abs(θ − θ_exact)/a.\n"
    );
    println!("| Method | Error |");
    println!("|---|---|");
    for method in METHODS {
        let mut s = Stepper::new(mech.clone(), Algo::Aba, method, &[a0], &[]);
        let mut e = 0.0f64;
        for k in 0..10_000 {
            s.step(0.001);
            e = e.max((s.x[0] - exact((k + 1) as f64 * 0.001)).abs());
        }
        println!(
            "| Native 2D, ABA, {} | {:.1e} |",
            method_name(method),
            e / a0
        );
    }
    println!();
}

fn fused() {
    println!("### A controller fused with the plant in one ODE\n");
    println!(
        "The cart-pole with the spike's LQR controller evaluated inside the right-hand side (at every RK4 stage: plant and controller one ODE, as a typed-equation \
         controller fused with the native block would be), against the same continuous closed loop on the exact equations by RK4 at h = 1e−5 s. \
         Beside it the controller held over each step (sampled, as with any hosted engine), against the same continuous reference: its gap is the sampling, first order in dt. \
         From θ = 0.1 rad, 6 s, no disturbances. max abs(θ − θ_ref).\n"
    );
    let gains = cp::lqr();
    let t_end: f64 = 6.0;
    // Reference: continuous closed loop on the exact equations.
    let h = 1e-5;
    let rhs = |s: &cp::State| {
        let u = cp::control(&gains, s);
        let (a, b) = cp::accel(s, u, [0.0, 0.0]);
        [s[1], a, s[3], b]
    };
    let mut s: cp::State = [0.0, 0.0, cp::THETA0, 0.0];
    let mut reference = vec![s];
    let steps = (t_end / h).round() as usize;
    for k in 0..steps {
        let k1 = rhs(&s);
        let ax = |s: &cp::State, k: &[f64; 4], c: f64| {
            [
                s[0] + c * k[0],
                s[1] + c * k[1],
                s[2] + c * k[2],
                s[3] + c * k[3],
            ]
        };
        let k2 = rhs(&ax(&s, &k1, h / 2.0));
        let k3 = rhs(&ax(&s, &k2, h / 2.0));
        let k4 = rhs(&ax(&s, &k3, h));
        for i in 0..4 {
            s[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
        }
        if (k + 1) % 100 == 0 {
            reference.push(s); // every 1 ms
        }
    }
    let dts = [0.01, 0.005, 0.002, 0.001];
    print!("| Controller |");
    for dt in dts {
        print!(" dt = {dt} s |");
    }
    println!(" Order (last two) |");
    println!("|---|{}---|", "---|".repeat(dts.len()));
    let mech = nat::cartpole();
    for fused in [true, false] {
        let mut errs = vec![];
        for dt in dts {
            let n = (t_end / dt).round() as usize;
            let every = (dt / 0.001).round() as usize;
            let mut x = vec![0.0, cp::THETA0, 0.0, 0.0];
            let mut ws = Workspace::default();
            let w = std::cell::RefCell::new(Scratch::default());
            let mut e = 0.0f64;
            for k in 0..n {
                let held = cp::control(&gains, &[x[0], x[2], x[1], x[3]]);
                let f = |_t: f64, x: &[f64], dx: &mut [f64]| {
                    let u = if fused {
                        cp::control(&gains, &[x[0], x[2], x[1], x[3]])
                    } else {
                        held
                    };
                    nat::rates(&mech, Algo::Aba, x, &[u, 0.0], &[], dx, &mut w.borrow_mut());
                };
                Integrator::Rk4 { substeps: 1 }
                    .advance_in(&f, 0.0, dt, &mut x, &mut ws, None)
                    .expect("rk4");
                let r = reference[(k + 1) * every];
                e = e.max((x[1] - r[2]).abs());
            }
            errs.push(e);
        }
        print!(
            "| {} |",
            if fused {
                "fused (in the RHS)"
            } else {
                "held over the step"
            }
        );
        for e in &errs {
            print!(" {e:.2e} |");
        }
        println!(" {:.2} |", (errs[2] / errs[3]).log2());
    }
    println!();
}

// ------------------------------------------------- the typed-equations route

/// The cart-pole's continuous closed loop (controller inside the
/// right-hand side) on the exact equations, RK4 at 1e−5 s, every 1 ms.
fn continuous_cartpole(t_end: f64) -> Vec<cp::State> {
    let gains = cp::lqr();
    let h = 1e-5;
    let rhs = |s: &cp::State| {
        let u = cp::control(&gains, s);
        let (a, b) = cp::accel(s, u, [0.0, 0.0]);
        [s[1], a, s[3], b]
    };
    let ax = |s: &cp::State, k: &[f64; 4], c: f64| {
        [
            s[0] + c * k[0],
            s[1] + c * k[1],
            s[2] + c * k[2],
            s[3] + c * k[3],
        ]
    };
    let mut s: cp::State = [0.0, 0.0, cp::THETA0, 0.0];
    let mut out = vec![s];
    for k in 0..(t_end / h).round() as usize {
        let k1 = rhs(&s);
        let k2 = rhs(&ax(&s, &k1, h / 2.0));
        let k3 = rhs(&ax(&s, &k2, h / 2.0));
        let k4 = rhs(&ax(&s, &k3, h));
        for i in 0..4 {
            s[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
        }
        if (k + 1) % 100 == 0 {
            out.push(s);
        }
    }
    out
}

pub fn typed_cases() {
    use crate::typed::Typed;
    println!("## Typed equations: checks and costs\n");
    println!(
        "The models in `typed/` loaded by `simsandbox-file` and stepped by the kernel's `Simulator` (as `simsandbox run` does), \
         the block's method set to each of SimSandbox's. Native code: `{}`; interpreter forced: {}.\n",
        std::env::var("SIMSANDBOX_NATIVE").unwrap_or_else(|_| "default (cache)".into()),
        std::env::var("SIMSANDBOX_INTERPRET").is_ok()
    );
    let methods = ["rk4", "dopri5", "trbdf2"];

    // Double pendulum against the Lagrange reference.
    println!("### Double pendulum (typed) against its Lagrange equations\n");
    let s0 = [1.0, 0.5, 0.0, 0.0];
    let t_end: f64 = 5.0;
    let reference = double::reference(s0, 1e-5, (t_end / 1e-5).round() as usize, 50);
    let dts: [f64; 4] = [0.004, 0.002, 0.001, 0.0005];
    print!("| Method |");
    for dt in dts {
        print!(" dt = {dt} s |");
    }
    println!(" Order (last two) |");
    println!("|---|{}---|", "---|".repeat(dts.len()));
    for m in methods {
        let mut errs = vec![];
        for dt in dts {
            let mut t = Typed::load("double-pendulum", m);
            let every = (dt / 0.0005).round() as usize;
            let mut e = 0.0f64;
            for k in 0..(t_end / dt).round() as usize {
                t.step(dt);
                let r = reference[(k + 1) * every];
                e = e.max(
                    (t.out("p1")[0] - r[0])
                        .abs()
                        .max((t.out("p2")[0] - r[1]).abs()),
                );
            }
            errs.push(e);
        }
        print!("| Typed equations, {m} |");
        for e in &errs {
            print!(" {e:.2e} |");
        }
        println!(" {:.2} |", (errs[2] / errs[3]).log2());
    }
    println!();

    // The chain through solve(M, b) against the native solver.
    println!("### Chains through `solve(M, b)` against the native solver\n");
    println!(
        "The typed chain (absolute angles, M and b in closed form over axes i, j) against the native ABA (relative angles, summed to absolute), both RK4 at 1 ms \
         from horizontal at rest for 2 s; and the typed chain's energy over 10 s against E_ref (as in the chains' table).\n"
    );
    println!("| Links | max abs(φ_typed − φ_native) over 2 s | Typed RK4: max abs(E − E0)/E_ref over 10 s | Typed Dopri5: same | Failed (RK4) |");
    println!("|---|---|---|---|---|");
    for n in [2usize, 10, 50] {
        let dt = 0.001;
        let mut t = Typed::load(&format!("chain-{n}"), "rk4");
        let mut s = Stepper::new(nat::chain(n), Algo::Aba, RK4, &[], &[]);
        let mut d = 0.0f64;
        for _ in 0..2000 {
            t.step(dt);
            s.step(dt);
            let mut acc = 0.0;
            for (k, p) in t.out("phi").iter().enumerate() {
                acc += s.x[k];
                d = d.max((p - acc).abs());
            }
        }
        let eref = nat::CHAIN_M * G * nat::CHAIN_LEN * (n * n) as f64 / 2.0;
        let drift = |m: &str| {
            let mut t = Typed::load(&format!("chain-{n}"), m);
            let e0 = t.out("E")[0];
            let mut e = 0.0f64;
            let mut failed = None;
            for k in 0..10_000 {
                if !t.step(dt) || !t.out("E")[0].is_finite() {
                    failed.get_or_insert((k + 1) as f64 * dt);
                    break;
                }
                e = e.max((t.out("E")[0] - e0).abs());
            }
            (e / eref, failed)
        };
        let (er, fr) = drift("rk4");
        let (ed, fd) = drift("dopri5");
        let f = |x: Option<f64>| x.map_or("no".to_string(), |t| format!("**{t:.3} s**"));
        println!(
            "| {n} | {d:.1e} | {er:.2e} | {ed:.2e}{} | {} |",
            if fd.is_some() {
                format!(" (failed {})", f(fd))
            } else {
                String::new()
            },
            f(fr)
        );
    }
    println!();

    // The controller fused in the typed block.
    println!("### Cart-pole with its controller in the typed block\n");
    println!(
        "`cartpole-lqr.ssbx.json`: plant, LQR and grab in one block, so one ODE. From θ = 0.1 rad, 6 s, no disturbances, against the continuous closed loop on the exact equations (RK4 at 1e−5 s). \
         Then the same file with the recorded impulse (fi = 0.05 N s / dt for one step) and the grab (grab = 1 from 3 s to 4 s), set as parameters between steps, against the spike's reference (controller held over each step): \
         their gap is the sampling of the controller, not an error of either.\n"
    );
    let reference = continuous_cartpole(6.0);
    let gains = cp::lqr();
    let dts: [f64; 4] = [0.01, 0.005, 0.002, 0.001];
    print!("| Method |");
    for dt in dts {
        print!(" dt = {dt} s |");
    }
    println!(" Order (last two) |");
    println!("|---|{}---|", "---|".repeat(dts.len()));
    let load = |m: &str| {
        let mut t = Typed::load("cartpole-lqr", m);
        // The gains to the last bit, as the reference has them.
        for (k, g) in ["k1", "k2", "k3", "k4"].iter().zip(gains) {
            t.set(k, g);
        }
        t
    };
    for m in methods {
        let mut errs = vec![];
        for dt in dts {
            let mut t = load(m);
            let every = (dt / 0.001).round() as usize;
            let mut e = 0.0f64;
            for k in 0..(6.0 / dt).round() as usize {
                t.step(dt);
                e = e.max((t.out("th")[0] - reference[(k + 1) * every][2]).abs());
            }
            errs.push(e);
        }
        print!("| Typed equations, {m} |");
        for e in &errs {
            print!(" {e:.2e} |");
        }
        println!(" {:.2} |", (errs[2] / errs[3]).log2());
    }
    {
        let dt = 0.001;
        let held = {
            let mut s = [0.0, 0.0, cp::THETA0, 0.0];
            let mut out = vec![s];
            for k in 0..6000 {
                let (f, p) = (
                    cp::control(&gains, &s),
                    crate::common::disturb::tip_force(k, dt, &s),
                );
                cp::reference_step(&mut s, dt, 20, f, p);
                out.push(s);
            }
            out
        };
        let mut t = load("rk4");
        let mut e = 0.0f64;
        let mut peak = 0.0f64;
        for k in 0..6000usize {
            let tt = k as f64 * dt;
            t.set("fi", if k == 1000 { 0.05 / dt } else { 0.0 });
            t.set("grab", if (3.0..4.0).contains(&tt) { 1.0 } else { 0.0 });
            t.step(dt);
            e = e.max((t.out("th")[0] - held[k + 1][2]).abs());
            if (3000..4500).contains(&k) {
                peak = peak.max(t.out("th")[0].abs());
            }
        }
        println!(
            "\nWith the impulse and the grab, RK4 at 1 ms: largest abs(θ) while pulled {peak:.4} rad (the held reference: 0.2126); max abs(θ − θ_held ref) {e:.1e} rad.\n"
        );
    }

    generated_aba();
    typed_cost();
}

/// The equation-generator route: ABA unrolled into scalar typed equations
/// by `typed/generate_aba.py`.
pub fn generated_aba() {
    use crate::typed::Typed;
    println!("### ABA generated as typed equations\n");
    println!(
        "`typed/aba-chain-N.ssbx.json`, written by `typed/generate_aba.py`: the native solver's ABA unrolled into scalar formulas per body (no arrays, no new functions). \
         Against the native ABA, both RK4 at 1 ms from horizontal for 2 s: max abs(q_gen − q_native) over every joint and step. Then µs per kernel step, the model loaded beforehand, from the start.\n"
    );
    println!("| Links | Lines | max abs(q_gen − q_native) | RK4: µs per step | Dopri5: µs per step | Native ABA + RK4: µs per step | code |");
    println!("|---|---|---|---|---|---|---|");
    for (n, steps) in [(2usize, 20_000usize), (10, 5_000), (50, 2_000), (200, 500)] {
        let name = format!("aba-chain-{n}");
        if !std::path::Path::new(&crate::typed::path(&name)).exists() {
            println!("| {n} | – | run `python3 typed/generate_aba.py` first | | | | |");
            continue;
        }
        let lines = std::fs::read_to_string(crate::typed::path(&name))
            .unwrap()
            .matches("\",\n")
            .count();
        let dt = 0.001;
        let mut t = Typed::load(&name, "rk4");
        let mut s = Stepper::new(nat::chain(n), Algo::Aba, RK4, &[], &[]);
        let mut d = 0.0f64;
        for _ in 0..2000 {
            t.step(dt);
            s.step(dt);
            for k in 0..n {
                d = d.max((t.out(&format!("q{k}_"))[0] - s.x[k]).abs());
            }
        }
        let per = |m: &str| {
            let mut t = Typed::load(&name, m);
            let (_, wall) = time(|| {
                for _ in 0..steps {
                    t.step(dt);
                }
            });
            (wall / steps as f64 * 1e6, t.code())
        };
        let (r, code) = per("rk4");
        let (dp, _) = per("dopri5");
        let mut s = Stepper::new(nat::chain(n), Algo::Aba, RK4, &[], &[]);
        let (_, wall) = time(|| {
            for _ in 0..steps {
                s.step(dt);
            }
        });
        println!(
            "| {n} | {lines} | {d:.1e} | {r:.2} | {dp:.2} | {:.2} | {code} |",
            wall / steps as f64 * 1e6
        );
    }
    println!();
}

pub fn typed_cost() {
    use crate::typed::Typed;
    let methods = ["rk4", "dopri5", "trbdf2"];
    println!("### Cost of the typed models\n");
    println!(
        "µs per kernel step (`Simulator::step`, the model loaded and built beforehand, outputs computed as every step computes them), mean over the steps given, from the start. \
         `code` is how the block runs after the timing (`check` reports the same).\n"
    );
    println!("| Model | Method | Steps | µs per step | code |");
    println!("|---|---|---|---|---|");
    let wait_native = std::env::var("SIMSANDBOX_NATIVE").is_ok_and(|v| v == "build");
    for (file, steps) in [
        ("pendulum", 20_000usize),
        ("cartpole", 20_000),
        ("double-pendulum", 20_000),
        ("chain-2", 20_000),
        ("chain-10", 5_000),
        ("chain-50", 1_000),
        ("chain-200", 200),
    ] {
        // For experiments: SPIKE_TYPED_ONLY=chain-200 times one model.
        if std::env::var("SPIKE_TYPED_ONLY").is_ok_and(|o| !o.split(',').any(|f| f == file)) {
            continue;
        }
        for m in methods {
            if m == "trbdf2" && file == "chain-200" {
                continue;
            }
            let mut t = Typed::load(file, m);
            if wait_native {
                // A native library builds in the background; wait for it.
                let start = std::time::Instant::now();
                while !t.code().starts_with("native") && start.elapsed().as_secs() < 300 {
                    t.step(0.001);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                t = Typed::load(file, m);
                let _ = t.step(0.0);
            }
            let (_, wall) = time(|| {
                for _ in 0..steps {
                    t.step(0.001);
                }
            });
            println!(
                "| {file} | {m} | {steps} | {:.2} | {} |",
                wall / steps as f64 * 1e6,
                t.code()
            );
        }
    }
    println!();
}
