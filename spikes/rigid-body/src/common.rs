//! What every engine shares: the models' numbers, the exact answers they are
//! checked against, the cart-pole controller and disturbances, and hashing.

use std::time::Instant;

pub const G: f64 = 9.81;

/// The pendulum: a compact bob of mass `M` with its own inertia `IC` about
/// its centre, on a massless arm of length `L` from the pivot (a physical
/// pendulum: `ω0² = M g L / (IC + M L²)`). The angle is measured from
/// straight down, counter-clockwise.
pub mod pendulum {
    pub const M: f64 = 1.0;
    pub const L: f64 = 1.0;
    pub const IC: f64 = 0.01;
    pub fn omega0() -> f64 {
        (M * super::G * L / (IC + M * L * L)).sqrt()
    }
    /// Energy from the angle and its rate (generalized coordinates).
    pub fn energy(theta: f64, omega: f64) -> f64 {
        0.5 * (IC + M * L * L) * omega * omega + M * super::G * L * (1.0 - theta.cos())
    }
    /// The exact period at amplitude `a`: `4 K(sin(a/2)) / ω0`.
    pub fn period_exact(a: f64) -> f64 {
        4.0 * super::ellip_k((a / 2.0).sin()) / omega0()
    }
    pub fn period_small_angle() -> f64 {
        2.0 * std::f64::consts::PI / omega0()
    }
}

/// The complete elliptic integral of the first kind, K(k) with modulus k,
/// by the arithmetic–geometric mean (exact to rounding).
pub fn ellip_k(k: f64) -> f64 {
    let (mut a, mut b) = (1.0f64, (1.0 - k * k).sqrt());
    for _ in 0..64 {
        let (an, bn) = (0.5 * (a + b), (a * b).sqrt());
        if (an - bn).abs() <= 1e-17 * an {
            a = an;
            break;
        }
        a = an;
        b = bn;
    }
    std::f64::consts::PI / (2.0 * a)
}

/// Upward zero crossings of the angle, interpolated linearly inside the step,
/// give the period: the mean over the crossings recorded.
#[derive(Default)]
pub struct PeriodMeter {
    prev: Option<(f64, f64)>,
    pub crossings: Vec<f64>,
}

impl PeriodMeter {
    pub fn push(&mut self, t: f64, theta: f64) {
        if let Some((tp, ap)) = self.prev {
            if ap < 0.0 && theta >= 0.0 {
                self.crossings.push(tp + (t - tp) * (-ap) / (theta - ap));
            }
        }
        self.prev = Some((t, theta));
    }
    pub fn period(&self) -> Option<f64> {
        let n = self.crossings.len();
        (n >= 2).then(|| (self.crossings[n - 1] - self.crossings[0]) / (n - 1) as f64)
    }
}

/// The cart-pole: a cart of mass `MC` on a slider along x, a uniform pole of
/// mass `MP` and length `2 LH` hinged at the cart's centre. `theta` is the
/// pole's angle from straight up, counter-clockwise, so the tip is at
/// `(x - 2 LH sin θ, 2 LH cos θ)`.
pub mod cartpole {
    pub const MC: f64 = 1.0;
    pub const MP: f64 = 0.1;
    pub const LH: f64 = 0.5; // pivot to the pole's centre
    pub const LEN: f64 = 2.0 * LH;
    pub const IC: f64 = MP * LEN * LEN / 12.0; // about the pole's centre
    pub const CART_W: f64 = 0.4;
    pub const CART_H: f64 = 0.2;
    pub const THETA0: f64 = 0.1;
    pub const FORCE_LIMIT: f64 = 30.0;

    /// State `[x, ẋ, θ, θ̇]`.
    pub type State = [f64; 4];

    pub fn tip(s: &State) -> ([f64; 2], [f64; 2]) {
        let (x, xd, th, thd) = (s[0], s[1], s[2], s[3]);
        (
            [x - LEN * th.sin(), LEN * th.cos()],
            [xd - LEN * th.cos() * thd, -LEN * th.sin() * thd],
        )
    }

    /// The exact equations (Lagrange, frictionless): accelerations for a cart
    /// force `f` and a world force `p` at the tip.
    pub fn accel(s: &State, f: f64, p: [f64; 2]) -> (f64, f64) {
        let (th, thd) = (s[2], s[3]);
        let (sn, cs) = th.sin_cos();
        let j = IC + MP * LH * LH;
        let m11 = MC + MP;
        let m12 = -MP * LH * cs;
        let m22 = j;
        let qx = f + p[0] - MP * LH * sn * thd * thd;
        let qt = MP * super::G * LH * sn + p[0] * (-LEN * cs) + p[1] * (-LEN * sn);
        let det = m11 * m22 - m12 * m12;
        ((m22 * qx - m12 * qt) / det, (m11 * qt - m12 * qx) / det)
    }

    /// The reference: classic RK4 on the exact equations, `sub` substeps per
    /// step, inputs held over the step as the engines hold them.
    pub fn reference_step(s: &mut State, dt: f64, sub: usize, f: f64, p: [f64; 2]) {
        let h = dt / sub as f64;
        let d = |s: &State| {
            let (a, b) = accel(s, f, p);
            [s[1], a, s[3], b]
        };
        for _ in 0..sub {
            let k1 = d(s);
            let s2 = add(s, &k1, h / 2.0);
            let k2 = d(&s2);
            let s3 = add(s, &k2, h / 2.0);
            let k3 = d(&s3);
            let s4 = add(s, &k3, h);
            let k4 = d(&s4);
            for i in 0..4 {
                s[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
            }
        }
    }
    fn add(s: &State, k: &State, h: f64) -> State {
        [
            s[0] + h * k[0],
            s[1] + h * k[1],
            s[2] + h * k[2],
            s[3] + h * k[3],
        ]
    }

    /// LQR gains for the upright pose (continuous time): the Riccati
    /// equation `−dP/dt = AᵀP + PA − P B R⁻¹ Bᵀ P + Q` integrated backward
    /// (explicit Euler, small step) until P stops changing; `K = R⁻¹ Bᵀ P`.
    /// At 1 kHz the discrete optimum differs from this by far less than the
    /// controller needs.
    pub fn lqr() -> [f64; 4] {
        let j = IC + MP * LH * LH;
        let m11 = MC + MP;
        let m12 = -MP * LH;
        let det = m11 * j - m12 * m12;
        // [ẍ, θ̈] = M⁻¹ [f, m g l θ]
        let (i11, i12, i22) = (j / det, -m12 / det, m11 / det);
        let gl = MP * super::G * LH;
        let mut a = [[0.0; 4]; 4];
        a[0][1] = 1.0;
        a[1][2] = i12 * gl;
        a[2][3] = 1.0;
        a[3][2] = i22 * gl;
        let b = [0.0, i11, 0.0, i12];
        let q = [10.0, 1.0, 100.0, 1.0];
        let r = 0.1;
        let mut p = [[0.0; 4]; 4];
        let h = 1e-4;
        for _ in 0..2_000_000 {
            let pb = mulv(&p, &b);
            let pa = mul(&p, &a);
            let mut change = 0.0f64;
            let mut pn = p;
            for i in 0..4 {
                for jj in 0..4 {
                    let mut d = pa[i][jj] + pa[jj][i] - pb[i] * pb[jj] / r;
                    if i == jj {
                        d += q[i];
                    }
                    pn[i][jj] += h * d;
                    change = change.max((h * d).abs());
                }
            }
            p = pn;
            if change < 1e-13 {
                break;
            }
        }
        let pb = mulv(&p, &b);
        [pb[0] / r, pb[1] / r, pb[2] / r, pb[3] / r]
    }
    pub fn control(k: &[f64; 4], s: &State) -> f64 {
        let u = -(k[0] * s[0] + k[1] * s[1] + k[2] * s[2] + k[3] * s[3]);
        u.clamp(-FORCE_LIMIT, FORCE_LIMIT)
    }

    type M4 = [[f64; 4]; 4];
    fn mul(a: &M4, b: &M4) -> M4 {
        let mut r = [[0.0; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                r[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        r
    }
    fn mulv(a: &M4, v: &[f64; 4]) -> [f64; 4] {
        let mut r = [0.0; 4];
        for i in 0..4 {
            r[i] = (0..4).map(|k| a[i][k] * v[k]).sum();
        }
        r
    }
}

/// The disturbances, as the gate's scene would send them: a recorded impulse
/// on the pole's tip, and a pull toward a held pointer (the scene's grab),
/// computed from the state like a model would compute it.
pub mod disturb {
    use super::cartpole::{tip, State};
    pub const IMPULSE_T: f64 = 1.0; // s
    pub const IMPULSE: f64 = 0.05; // N s, along +x at the tip
    pub const GRAB_FROM: f64 = 3.0;
    pub const GRAB_TO: f64 = 4.0;
    pub const POINTER: [f64; 2] = [0.4, 0.9]; // m, held still
    pub const K: f64 = 5.0; // N/m
    pub const C: f64 = 1.0; // N s/m

    /// The world force on the tip during the step starting at step `k`.
    pub fn tip_force(k: usize, dt: f64, s: &State) -> [f64; 2] {
        let t = k as f64 * dt;
        let ki = (IMPULSE_T / dt).round() as usize;
        let mut f = [0.0, 0.0];
        if k == ki {
            f[0] += IMPULSE / dt;
        }
        if (GRAB_FROM..GRAB_TO).contains(&t) {
            let (p, v) = tip(s);
            f[0] += K * (POINTER[0] - p[0]) - C * v[0];
            f[1] += K * (POINTER[1] - p[1]) - C * v[1];
        }
        f
    }
}

/// Hash of the bits of a sequence of numbers (FNV-1a over the IEEE bits):
/// equal hashes for equal runs, any difference in any bit changes it.
pub fn hash_bits(values: impl IntoIterator<Item = f64>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in values {
        for b in v.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

pub fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_secs_f64())
}

/// One engine's model, stepped at a fixed `dt` with the inputs given.
pub trait Sim {
    fn label(&self) -> String;
    fn dt(&self) -> f64;
    /// Inputs for this step: the cart-pole takes `[cart force, tip fx, tip fy]`
    /// (the tip force in the plane, applied at the tip); others take none.
    fn step(&mut self, input: &[f64]);
    /// The model's coordinates: the cart-pole `[x, ẋ, θ, θ̇]`, the pendulum
    /// `[θ, θ̇]`, others what they choose.
    fn coords(&self) -> Vec<f64>;
    /// Every body's pose and velocity, as rows (what a block would output).
    fn rows(&self) -> Vec<f64>;
    /// Energy of the pendulum (from the bodies, so a drifting joint shows).
    fn energy(&self) -> f64 {
        0.0
    }
    /// Warnings the engine has given so far in this process (MuJoCo's
    /// instability resets among them).
    fn warnings(&self) -> usize {
        0
    }
    /// The boxes' centres as `[horizontal x, height]` (contact cases).
    fn box_poses(&self) -> Vec<[f64; 2]> {
        vec![]
    }
    /// How far the joints have come apart (m): zero in generalized
    /// coordinates, a measure of drift in maximal ones.
    fn joint_drift(&self) -> f64 {
        0.0
    }
    /// A snapshot of the engine's full state.
    fn snapshot(&self, kind: SnapKind) -> Snapshot;
    fn restore(&mut self, snap: &Snapshot);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapKind {
    /// Kept in memory as the engine's own objects (Rapier: clones of the sets).
    Memory,
    /// Bytes (Rapier: bincode of the sets; MuJoCo: `mj_getState` as f64s),
    /// restored into the same engine objects.
    Bytes,
    /// Bytes restored into newly built engine objects (as after loading a
    /// saved run).
    BytesFresh,
    /// Only body poses and velocities (and for MuJoCo `mjSTATE_PHYSICS`
    /// without warm start): what a flat state vector would hold.
    Minimal,
}

pub struct Snapshot {
    pub kind: SnapKind,
    pub bytes: usize,
    pub data: Box<dyn std::any::Any>,
}
