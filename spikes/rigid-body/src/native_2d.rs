//! A native 2D articulated-body solver: a tree of rigid bodies in the plane
//! on revolute and prismatic joints, its dynamics in joint coordinates
//! (Featherstone's articulated-body algorithm, ABA, and as a cross-check
//! the composite rigid-body algorithm, CRBA, with recursive Newton–Euler,
//! RNEA), integrated as an ODE in the state `[q, q̇]` and nothing else.
//!
//! Spatial algebra in 2D: a motion vector is `(ω, vx, vy)`, a force vector
//! `(n, fx, fy)`, both in a body's frame at its origin (the body's joint).
//! Gravity enters as an upward acceleration of the ground, so no body
//! needs a gravity force.
//!
//! Integration: a semi-implicit Euler written here, and SimSandbox's own
//! methods (RK4, Dormand–Prince 5(4), TR-BDF2) from `simsandbox-eq`,
//! called through `Integrator::advance_in` exactly as the equation domain
//! calls them.

// Spatial algebra reads best indexed by row and column.
#![allow(clippy::needless_range_loop)]

use crate::common::{self, cartpole as cp, pendulum as pd, SnapKind, Snapshot};
use crate::rapier_2d::Model;
use simsandbox_eq::implicit::Workspace;
use simsandbox_eq::Integrator;
use std::cell::RefCell;

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];

/// One joint, one degree of freedom.
#[derive(Clone, Copy, Debug)]
pub enum Joint {
    /// An angle about the joint's point.
    Revolute,
    /// A displacement along `axis` (unit), fixed in the joint frame.
    Prismatic { axis: [f64; 2] },
}

/// A body and the joint that attaches it to its parent (or the ground).
#[derive(Clone, Debug)]
pub struct Body {
    /// Index of the parent body, lower than this body's; `None` is the ground.
    pub parent: Option<usize>,
    pub joint: Joint,
    /// The joint's point in the parent's frame (the ground's is the world).
    pub at: [f64; 2],
    /// The joint frame's fixed rotation in the parent's frame (rad).
    pub rot: f64,
    pub mass: f64,
    /// Centre of mass in the body's frame (whose origin is the joint).
    pub com: [f64; 2],
    /// Moment of inertia about the centre of mass.
    pub inertia: f64,
    /// Joint damping: a generalized force `−damping · q̇`.
    pub damping: f64,
}

impl Body {
    pub fn revolute(
        parent: Option<usize>,
        at: [f64; 2],
        mass: f64,
        com: [f64; 2],
        inertia: f64,
    ) -> Self {
        Body {
            parent,
            joint: Joint::Revolute,
            at,
            rot: 0.0,
            mass,
            com,
            inertia,
            damping: 0.0,
        }
    }
    fn subspace(&self) -> V3 {
        match self.joint {
            Joint::Revolute => [1.0, 0.0, 0.0],
            Joint::Prismatic { axis } => [0.0, axis[0], axis[1]],
        }
    }
    /// Spatial inertia at the body's origin.
    fn spatial_inertia(&self) -> M3 {
        let (m, [cx, cy]) = (self.mass, self.com);
        [
            [self.inertia + m * (cx * cx + cy * cy), -m * cy, m * cx],
            [-m * cy, m, 0.0],
            [m * cx, 0.0, m],
        ]
    }
    /// The body's frame in its parent's: rotation and origin.
    fn placement(&self, q: f64) -> (f64, [f64; 2]) {
        match self.joint {
            Joint::Revolute => (self.rot + q, self.at),
            Joint::Prismatic { axis } => {
                let (s, c) = self.rot.sin_cos();
                let d = [q * axis[0], q * axis[1]];
                (
                    self.rot,
                    [
                        self.at[0] + c * d[0] - s * d[1],
                        self.at[1] + s * d[0] + c * d[1],
                    ],
                )
            }
        }
    }
}

/// A force in world axes applied at a point fixed on a body (body frame),
/// held over a step: how a grab's spring toward the pointer is applied.
#[derive(Clone, Copy, Debug)]
pub struct PointForce {
    pub body: usize,
    pub point: [f64; 2],
    pub force: [f64; 2],
}

/// Motion transform from a parent frame to a child frame whose origin is
/// at `r` in the parent and which is rotated by θ (`c = cos θ`, `s = sin θ`).
#[derive(Clone, Copy, Default)]
struct Xf {
    c: f64,
    s: f64,
    rx: f64,
    ry: f64,
}

impl Xf {
    fn new(theta: f64, r: [f64; 2]) -> Self {
        let (s, c) = theta.sin_cos();
        Xf {
            c,
            s,
            rx: r[0],
            ry: r[1],
        }
    }
    /// A motion vector from the parent's frame to the child's.
    #[inline]
    fn motion(&self, v: V3) -> V3 {
        let lx = v[1] - v[0] * self.ry;
        let ly = v[2] + v[0] * self.rx;
        [v[0], self.c * lx + self.s * ly, -self.s * lx + self.c * ly]
    }
    /// A force vector from the child's frame to the parent's (Xᵀ f).
    #[inline]
    fn force_to_parent(&self, f: V3) -> V3 {
        let fx = self.c * f[1] - self.s * f[2];
        let fy = self.s * f[1] + self.c * f[2];
        [f[0] + self.rx * fy - self.ry * fx, fx, fy]
    }
    fn mat(&self) -> M3 {
        let (c, s, rx, ry) = (self.c, self.s, self.rx, self.ry);
        [
            [1.0, 0.0, 0.0],
            [s * rx - c * ry, c, s],
            [s * ry + c * rx, -s, c],
        ]
    }
    /// Xᵀ I X: a child's inertia in its parent's frame.
    fn inertia_to_parent(&self, i: &M3) -> M3 {
        let x = self.mat();
        let ix = mul(i, &x);
        let mut r = [[0.0; 3]; 3];
        for a in 0..3 {
            for b in 0..3 {
                r[a][b] = x[0][a] * ix[0][b] + x[1][a] * ix[1][b] + x[2][a] * ix[2][b];
            }
        }
        r
    }
}

#[inline]
fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}
#[inline]
fn mv(a: &M3, v: V3) -> V3 {
    [
        a[0][0] * v[0] + a[0][1] * v[1] + a[0][2] * v[2],
        a[1][0] * v[0] + a[1][1] * v[1] + a[1][2] * v[2],
        a[2][0] * v[0] + a[2][1] * v[1] + a[2][2] * v[2],
    ]
}
#[inline]
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
/// Motion cross product v ×m u.
#[inline]
fn crm(v: V3, u: V3) -> V3 {
    [0.0, -v[0] * u[2] + v[2] * u[0], v[0] * u[1] - v[1] * u[0]]
}
/// Force cross product v ×* f.
#[inline]
fn crf(v: V3, f: V3) -> V3 {
    [-v[2] * f[1] + v[1] * f[2], -v[0] * f[2], v[0] * f[1]]
}

/// Working arrays, kept between calls.
#[derive(Default, Clone)]
pub struct Scratch {
    xf: Vec<Xf>,
    v: Vec<V3>,
    c: Vec<V3>,
    ia: Vec<M3>,
    pa: Vec<V3>,
    u_vec: Vec<V3>,
    d: Vec<f64>,
    u: Vec<f64>,
    a: Vec<V3>,
    theta_w: Vec<f64>,
    p_w: Vec<[f64; 2]>,
    m: Vec<f64>,
    rhs: Vec<f64>,
    bias: Vec<f64>,
    zeros: Vec<f64>,
}

impl Scratch {
    fn size(&mut self, n: usize) {
        if self.xf.len() != n {
            *self = Scratch {
                xf: vec![Xf::default(); n],
                v: vec![[0.0; 3]; n],
                c: vec![[0.0; 3]; n],
                ia: vec![[[0.0; 3]; 3]; n],
                pa: vec![[0.0; 3]; n],
                u_vec: vec![[0.0; 3]; n],
                d: vec![0.0; n],
                u: vec![0.0; n],
                a: vec![[0.0; 3]; n],
                theta_w: vec![0.0; n],
                p_w: vec![[0.0; 2]; n],
                m: vec![0.0; n * n],
                rhs: vec![0.0; n],
                bias: vec![0.0; n],
                zeros: vec![0.0; n],
            };
        }
    }
}

/// A body's state in the world, at its centre of mass: what a block outputs
/// as a row.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyState {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub vx: f64,
    pub vy: f64,
    pub omega: f64,
}

#[derive(Clone, Debug)]
pub struct Mechanism {
    pub bodies: Vec<Body>,
    pub gravity: [f64; 2],
    inertias: Vec<M3>,
}

impl Mechanism {
    pub fn new(bodies: Vec<Body>, gravity: [f64; 2]) -> Self {
        for (i, b) in bodies.iter().enumerate() {
            assert!(b.parent.is_none_or(|p| p < i), "parents come first");
        }
        let inertias = bodies.iter().map(Body::spatial_inertia).collect();
        Mechanism {
            bodies,
            gravity,
            inertias,
        }
    }
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    /// Pass 1 of both algorithms: each body's transform, world pose and
    /// spatial velocity, and the velocity-product acceleration `c`.
    fn kinematics(&self, q: &[f64], qd: &[f64], w: &mut Scratch) {
        for (i, b) in self.bodies.iter().enumerate() {
            let (th, r) = b.placement(q[i]);
            let xf = Xf::new(th, r);
            let (thp, pp, vp) = match b.parent {
                Some(p) => (w.theta_w[p], w.p_w[p], w.v[p]),
                None => (0.0, [0.0, 0.0], [0.0; 3]),
            };
            let (sp, cpp) = thp.sin_cos();
            w.theta_w[i] = thp + th;
            w.p_w[i] = [
                pp[0] + cpp * r[0] - sp * r[1],
                pp[1] + sp * r[0] + cpp * r[1],
            ];
            let vj = scale(b.subspace(), qd[i]);
            let v = add(xf.motion(vp), vj);
            w.v[i] = v;
            w.c[i] = crm(v, vj);
            w.xf[i] = xf;
        }
    }

    /// Point forces as spatial forces in the bodies' frames, subtracted from
    /// `f` (bias forces are what the joints must overcome).
    fn subtract_point_forces(&self, ext: &[PointForce], w: &Scratch, f: &mut [V3]) {
        for e in ext {
            let (s, c) = w.theta_w[e.body].sin_cos();
            let fx = c * e.force[0] + s * e.force[1];
            let fy = -s * e.force[0] + c * e.force[1];
            let n = e.point[0] * fy - e.point[1] * fx;
            let g = &mut f[e.body];
            g[0] -= n;
            g[1] -= fx;
            g[2] -= fy;
        }
    }

    /// The ground's acceleration in the world: up, against gravity.
    fn base_accel(&self) -> V3 {
        [0.0, -self.gravity[0], -self.gravity[1]]
    }

    /// Featherstone's articulated-body algorithm: `q̈ = f(q, q̇, τ, ext)` in
    /// O(n).
    pub fn aba(
        &self,
        q: &[f64],
        qd: &[f64],
        tau: &[f64],
        ext: &[PointForce],
        qdd: &mut [f64],
        w: &mut Scratch,
    ) {
        let n = self.len();
        w.size(n);
        self.kinematics(q, qd, w);
        for i in 0..n {
            w.ia[i] = self.inertias[i];
            let h = mv(&self.inertias[i], w.v[i]);
            w.pa[i] = crf(w.v[i], h);
        }
        {
            let Scratch { pa, .. } = &mut *w;
            let mut pa_tmp = std::mem::take(pa);
            self.subtract_point_forces(ext, w, &mut pa_tmp);
            w.pa = pa_tmp;
        }
        for i in (0..n).rev() {
            let b = &self.bodies[i];
            let s = b.subspace();
            let uv = mv(&w.ia[i], s);
            let d = dot(s, uv);
            let u = tau[i] - b.damping * qd[i] - dot(s, w.pa[i]);
            w.u_vec[i] = uv;
            w.d[i] = d;
            w.u[i] = u;
            if let Some(p) = b.parent {
                let mut ia = w.ia[i];
                for r in 0..3 {
                    for c in 0..3 {
                        ia[r][c] -= uv[r] * uv[c] / d;
                    }
                }
                let pa = add(add(w.pa[i], mv(&ia, w.c[i])), scale(uv, u / d));
                let ip = w.xf[i].inertia_to_parent(&ia);
                for r in 0..3 {
                    for c in 0..3 {
                        w.ia[p][r][c] += ip[r][c];
                    }
                }
                w.pa[p] = add(w.pa[p], w.xf[i].force_to_parent(pa));
            }
        }
        let a0 = self.base_accel();
        for i in 0..n {
            let b = &self.bodies[i];
            let ap = match b.parent {
                Some(p) => w.a[p],
                None => a0,
            };
            let a = add(w.xf[i].motion(ap), w.c[i]);
            let qa = (w.u[i] - dot(w.u_vec[i], a)) / w.d[i];
            qdd[i] = qa;
            w.a[i] = add(a, scale(b.subspace(), qa));
        }
    }

    /// Recursive Newton–Euler: the generalized forces for accelerations
    /// `qdd`, with gravity, the velocity terms and the point forces.
    pub fn rnea(
        &self,
        q: &[f64],
        qd: &[f64],
        qdd: &[f64],
        ext: &[PointForce],
        tau: &mut [f64],
        w: &mut Scratch,
    ) {
        let n = self.len();
        w.size(n);
        self.kinematics(q, qd, w);
        let a0 = self.base_accel();
        // `pa` holds each body's force here.
        for i in 0..n {
            let b = &self.bodies[i];
            let ap = match b.parent {
                Some(p) => w.a[p],
                None => a0,
            };
            let a = add(add(w.xf[i].motion(ap), w.c[i]), scale(b.subspace(), qdd[i]));
            w.a[i] = a;
            let ii = &self.inertias[i];
            w.pa[i] = add(mv(ii, a), crf(w.v[i], mv(ii, w.v[i])));
        }
        let mut f = std::mem::take(&mut w.pa);
        self.subtract_point_forces(ext, w, &mut f);
        for i in (0..n).rev() {
            let b = &self.bodies[i];
            tau[i] = dot(b.subspace(), f[i]);
            if let Some(p) = b.parent {
                let fp = w.xf[i].force_to_parent(f[i]);
                f[p] = add(f[p], fp);
            }
        }
        w.pa = f;
    }

    /// The composite rigid-body algorithm: the joint-space mass matrix
    /// (row-major, n × n) in `w.m`. Needs `kinematics` (the transforms).
    fn crba(&self, w: &mut Scratch) {
        let n = self.len();
        for i in 0..n {
            w.ia[i] = self.inertias[i];
        }
        for i in (0..n).rev() {
            if let Some(p) = self.bodies[i].parent {
                let ip = w.xf[i].inertia_to_parent(&w.ia[i]);
                for r in 0..3 {
                    for c in 0..3 {
                        w.ia[p][r][c] += ip[r][c];
                    }
                }
            }
        }
        w.m.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..n {
            let si = self.bodies[i].subspace();
            let mut f = mv(&w.ia[i], si);
            w.m[i * n + i] = dot(si, f);
            let mut j = i;
            while let Some(p) = self.bodies[j].parent {
                f = w.xf[j].force_to_parent(f);
                j = p;
                let mij = dot(self.bodies[j].subspace(), f);
                w.m[i * n + j] = mij;
                w.m[j * n + i] = mij;
            }
        }
    }

    /// `q̈` by CRBA and RNEA: `M q̈ = τ − b q̇ − C(q, q̇, ext)`, M by Cholesky.
    pub fn crba_rnea(
        &self,
        q: &[f64],
        qd: &[f64],
        tau: &[f64],
        ext: &[PointForce],
        qdd: &mut [f64],
        w: &mut Scratch,
    ) {
        let n = self.len();
        w.size(n);
        let mut bias = std::mem::take(&mut w.bias);
        let zeros = std::mem::take(&mut w.zeros);
        self.rnea(q, qd, &zeros, ext, &mut bias, w);
        self.crba(w);
        for i in 0..n {
            qdd[i] = tau[i] - self.bodies[i].damping * qd[i] - bias[i];
        }
        w.bias = bias;
        w.zeros = zeros;
        cholesky_solve(&mut w.m, n, qdd);
    }

    /// Every body's state at its centre of mass, in the world.
    pub fn body_states(&self, q: &[f64], qd: &[f64], w: &mut Scratch) -> Vec<BodyState> {
        let n = self.len();
        w.size(n);
        self.kinematics(q, qd, w);
        (0..n)
            .map(|i| {
                let b = &self.bodies[i];
                let (s, c) = w.theta_w[i].sin_cos();
                let [cx, cy] = b.com;
                let v = w.v[i];
                // Velocity of the centre in the body's axes, then the world's.
                let (bx, by) = (v[1] - v[0] * cy, v[2] + v[0] * cx);
                BodyState {
                    x: w.p_w[i][0] + c * cx - s * cy,
                    y: w.p_w[i][1] + s * cx + c * cy,
                    angle: w.theta_w[i],
                    vx: c * bx - s * by,
                    vy: s * bx + c * by,
                    omega: v[0],
                }
            })
            .collect()
    }

    /// Kinetic plus potential energy (potential zero at the world's origin).
    pub fn energy(&self, q: &[f64], qd: &[f64], w: &mut Scratch) -> f64 {
        self.body_states(q, qd, w)
            .iter()
            .zip(&self.bodies)
            .map(|(s, b)| {
                0.5 * b.mass * (s.vx * s.vx + s.vy * s.vy) + 0.5 * b.inertia * s.omega * s.omega
                    - b.mass * (self.gravity[0] * s.x + self.gravity[1] * s.y)
            })
            .sum()
    }

    /// Linear momentum and angular momentum about the world's origin.
    pub fn momentum(&self, q: &[f64], qd: &[f64], w: &mut Scratch) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (s, b) in self.body_states(q, qd, w).iter().zip(&self.bodies) {
            out[0] += b.mass * s.vx;
            out[1] += b.mass * s.vy;
            out[2] += b.inertia * s.omega + b.mass * (s.x * s.vy - s.y * s.vx);
        }
        out
    }
}

/// Solve `A x = b` for symmetric positive definite `A` (row-major, n × n,
/// overwritten by its Cholesky factor); `b` becomes `x`.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= a[i * n + k] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for k in i + 1..n {
            s -= a[k * n + i] * b[k];
        }
        b[i] = s / a[i * n + i];
    }
}

// ------------------------------------------------------------- the models

/// The spike's pendulum: the bob hangs at (0, −L) in the body frame, so
/// the joint angle is the angle from straight down.
pub fn pendulum() -> Mechanism {
    Mechanism::new(
        vec![Body::revolute(
            None,
            [0.0, 0.0],
            pd::M,
            [0.0, -pd::L],
            pd::IC,
        )],
        [0.0, -common::G],
    )
}

/// The spike's cart-pole: a cart on a slider along x, the pole's centre at
/// (0, LH) in its frame, so the joint angle is the angle from upright.
pub fn cartpole() -> Mechanism {
    Mechanism::new(
        vec![
            Body {
                parent: None,
                joint: Joint::Prismatic { axis: [1.0, 0.0] },
                at: [0.0, 0.0],
                rot: 0.0,
                mass: cp::MC,
                com: [0.0, 0.0],
                inertia: cp::MC * (cp::CART_W.powi(2) + cp::CART_H.powi(2)) / 12.0,
                damping: 0.0,
            },
            Body::revolute(Some(0), [0.0, 0.0], cp::MP, [0.0, cp::LH], cp::IC),
        ],
        [0.0, -common::G],
    )
}

pub const CHAIN_LEN: f64 = 0.1;
pub const CHAIN_M: f64 = 0.1;

/// The spike's chain: links 0.1 m and 0.1 kg (uniform rods), the first
/// hinged at the origin, starting horizontal along +x.
pub fn chain(links: usize) -> Mechanism {
    let (len, m) = (CHAIN_LEN, CHAIN_M);
    let bodies = (0..links)
        .map(|i| {
            Body::revolute(
                i.checked_sub(1),
                [if i == 0 { 0.0 } else { len }, 0.0],
                m,
                [len / 2.0, 0.0],
                m * len * len / 12.0,
            )
        })
        .collect();
    Mechanism::new(bodies, [0.0, -common::G])
}

/// A chain with a floating base in the plane: two massless bodies on
/// prismatic joints along x and y, then the base link on a revolute joint,
/// then `links − 1` more links. No gravity.
pub fn floating_chain(links: usize) -> Mechanism {
    let (len, m) = (CHAIN_LEN, CHAIN_M);
    let mut bodies = vec![
        Body {
            parent: None,
            joint: Joint::Prismatic { axis: [1.0, 0.0] },
            at: [0.0, 0.0],
            rot: 0.0,
            mass: 0.0,
            com: [0.0, 0.0],
            inertia: 0.0,
            damping: 0.0,
        },
        Body {
            parent: Some(0),
            joint: Joint::Prismatic { axis: [0.0, 1.0] },
            at: [0.0, 0.0],
            rot: 0.0,
            mass: 0.0,
            com: [0.0, 0.0],
            inertia: 0.0,
            damping: 0.0,
        },
    ];
    for i in 0..links {
        bodies.push(Body::revolute(
            Some(i + 1),
            [if i == 0 { 0.0 } else { len }, 0.0],
            m,
            [len / 2.0, 0.0],
            m * len * len / 12.0,
        ));
    }
    Mechanism::new(bodies, [0.0, 0.0])
}

/// The double pendulum checked against its Lagrange equations: two uniform
/// rods of 1 kg and 1 m, hanging; q₁ the first's angle from down, q₂ the
/// second's angle relative to the first.
pub mod double {
    pub const M1: f64 = 1.0;
    pub const L1: f64 = 1.0;
    pub const C1: f64 = 0.5;
    pub const I1: f64 = M1 * L1 * L1 / 12.0;
    pub const M2: f64 = 1.0;
    pub const C2: f64 = 0.5;
    pub const I2: f64 = M2 * 1.0 / 12.0;

    pub fn mechanism() -> super::Mechanism {
        super::Mechanism::new(
            vec![
                super::Body::revolute(None, [0.0, 0.0], M1, [0.0, -C1], I1),
                super::Body::revolute(Some(0), [0.0, -L1], M2, [0.0, -C2], I2),
            ],
            [0.0, -super::common::G],
        )
    }

    /// The Lagrange equations written out by hand in absolute angles
    /// φ₁ = q₁, φ₂ = q₁ + q₂: state `[φ₁, φ₂, φ̇₁, φ̇₂]`, returns `[φ̈₁, φ̈₂]`.
    ///
    /// A φ̈₁ + B cos Δ φ̈₂ + B sin Δ φ̇₂² + (m₁c₁ + m₂l₁) g sin φ₁ = 0
    /// C φ̈₂ + B cos Δ φ̈₁ − B sin Δ φ̇₁² + m₂c₂ g sin φ₂ = 0
    /// with Δ = φ₁ − φ₂, A = I₁ + m₁c₁² + m₂l₁², B = m₂l₁c₂, C = I₂ + m₂c₂².
    pub fn lagrange(s: &[f64; 4]) -> [f64; 2] {
        let g = super::common::G;
        let (p1, p2, w1, w2) = (s[0], s[1], s[2], s[3]);
        let a = I1 + M1 * C1 * C1 + M2 * L1 * L1;
        let b = M2 * L1 * C2;
        let c = I2 + M2 * C2 * C2;
        let (sd, cd) = (p1 - p2).sin_cos();
        let r1 = -b * sd * w2 * w2 - (M1 * C1 + M2 * L1) * g * p1.sin();
        let r2 = b * sd * w1 * w1 - M2 * C2 * g * p2.sin();
        let (m11, m12, m22) = (a, b * cd, c);
        let det = m11 * m22 - m12 * m12;
        [(m22 * r1 - m12 * r2) / det, (m11 * r2 - m12 * r1) / det]
    }

    /// The reference: classic RK4 on the Lagrange equations at step `h`.
    pub fn reference(s0: [f64; 4], h: f64, steps: usize, every: usize) -> Vec<[f64; 4]> {
        let d = |s: &[f64; 4]| {
            let a = lagrange(s);
            [s[2], s[3], a[0], a[1]]
        };
        let ax = |s: &[f64; 4], k: &[f64; 4], c: f64| {
            [
                s[0] + c * k[0],
                s[1] + c * k[1],
                s[2] + c * k[2],
                s[3] + c * k[3],
            ]
        };
        let mut s = s0;
        let mut out = vec![s];
        for k in 0..steps {
            let k1 = d(&s);
            let k2 = d(&ax(&s, &k1, h / 2.0));
            let k3 = d(&ax(&s, &k2, h / 2.0));
            let k4 = d(&ax(&s, &k3, h));
            for i in 0..4 {
                s[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
            }
            if (k + 1) % every == 0 {
                out.push(s);
            }
        }
        out
    }
}

// --------------------------------------------------------- integration

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    /// q̇ ← q̇ + h q̈(q, q̇); q ← q + h q̇ (velocity first, as MuJoCo's
    /// Euler and Rapier do). Written here.
    SemiImplicitEuler,
    /// One of SimSandbox's integrators, through `Integrator::advance_in`.
    Eq(Integrator),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Algo {
    Aba,
    Crba,
}

/// The rates `[q̇, q̈]` of the state `[q, q̇]`.
pub fn rates(
    mech: &Mechanism,
    algo: Algo,
    x: &[f64],
    tau: &[f64],
    ext: &[PointForce],
    dx: &mut [f64],
    w: &mut Scratch,
) {
    let n = mech.len();
    let (q, qd) = x.split_at(n);
    dx[..n].copy_from_slice(qd);
    match algo {
        Algo::Aba => mech.aba(q, qd, tau, ext, &mut dx[n..], w),
        Algo::Crba => mech.crba_rnea(q, qd, tau, ext, &mut dx[n..], w),
    }
}

/// A mechanism with its state and how it is stepped: what a block would
/// hold. The state is `[q, q̇]`; nothing else carries from step to step
/// (the workspace keeps only working arrays and, for TR-BDF2, the
/// Jacobian's structure, which never changes the numbers).
pub struct Stepper {
    pub mech: Mechanism,
    pub algo: Algo,
    pub method: Method,
    pub x: Vec<f64>,
    pub tau: Vec<f64>,
    pub ext: Vec<PointForce>,
    scratch: RefCell<Scratch>,
    ws: Workspace,
    pub failures: usize,
}

impl Stepper {
    pub fn new(mech: Mechanism, algo: Algo, method: Method, q0: &[f64], qd0: &[f64]) -> Self {
        let n = mech.len();
        let mut x = vec![0.0; 2 * n];
        x[..q0.len()].copy_from_slice(q0);
        x[n..n + qd0.len()].copy_from_slice(qd0);
        Stepper {
            mech,
            algo,
            method,
            x,
            tau: vec![0.0; n],
            ext: vec![],
            scratch: RefCell::new(Scratch::default()),
            ws: Workspace::default(),
            failures: 0,
        }
    }

    pub fn step(&mut self, dt: f64) {
        let n = self.mech.len();
        let Stepper {
            mech,
            algo,
            method,
            x,
            tau,
            ext,
            scratch,
            ws,
            failures,
        } = self;
        match method {
            Method::SemiImplicitEuler => {
                let mut w = scratch.borrow_mut();
                let mut qdd = std::mem::take(&mut w.rhs);
                qdd.resize(n, 0.0);
                let (q, qd) = x.split_at_mut(n);
                match algo {
                    Algo::Aba => mech.aba(q, qd, tau, ext, &mut qdd, &mut w),
                    Algo::Crba => mech.crba_rnea(q, qd, tau, ext, &mut qdd, &mut w),
                }
                for i in 0..n {
                    qd[i] += dt * qdd[i];
                    q[i] += dt * qd[i];
                }
                w.rhs = qdd;
            }
            Method::Eq(int) => {
                let f = |_t: f64, x: &[f64], dx: &mut [f64]| {
                    rates(mech, *algo, x, tau, ext, dx, &mut scratch.borrow_mut());
                };
                // The Jacobian's structure, given rather than probed: q̇'s
                // rates read q̇, q̈'s read the whole state. Probing finds it
                // at the state of the first step, where an entry that is
                // zero there (∂q̈/∂q̇ at rest) is missed; a run restored into
                // a new workspace would then probe at another state and get
                // other bits.
                let structure = || {
                    Some(
                        (0..2 * n)
                            .map(|i| {
                                if i < n {
                                    vec![n + i]
                                } else {
                                    (0..2 * n).collect()
                                }
                            })
                            .collect(),
                    )
                };
                if int
                    .advance_in(&f, 0.0, dt, x, ws, Some(&structure))
                    .is_err()
                {
                    *failures += 1;
                }
            }
        }
        if self.x.iter().any(|v| !v.is_finite()) {
            self.failures += 1;
        }
    }

    pub fn q(&self) -> &[f64] {
        &self.x[..self.mech.len()]
    }
    pub fn qd(&self) -> &[f64] {
        &self.x[self.mech.len()..]
    }
    pub fn body_states(&self) -> Vec<BodyState> {
        self.mech
            .body_states(self.q(), self.qd(), &mut self.scratch.borrow_mut())
    }
    pub fn energy(&self) -> f64 {
        self.mech
            .energy(self.q(), self.qd(), &mut self.scratch.borrow_mut())
    }
    pub fn momentum(&self) -> [f64; 3] {
        self.mech
            .momentum(self.q(), self.qd(), &mut self.scratch.borrow_mut())
    }
    /// A fresh stepper on the same mechanism and method (new working
    /// arrays), for restoring into new objects.
    fn fresh(&self) -> Self {
        let mut s = Stepper::new(self.mech.clone(), self.algo, self.method, &[], &[]);
        s.x = self.x.clone();
        s
    }
}

pub fn method_name(m: Method) -> String {
    match m {
        Method::SemiImplicitEuler => "semi-implicit Euler".into(),
        Method::Eq(Integrator::Rk4 { substeps: 1 }) => "RK4".into(),
        Method::Eq(Integrator::Dopri5 { rtol, .. }) => format!("Dopri5 rtol {rtol:.0e}"),
        Method::Eq(Integrator::TrBdf2 { substeps: 1 }) => "TR-BDF2".into(),
        Method::Eq(i) => format!("{i:?}"),
    }
}

// ------------------------------------------------- the spike's Sim trait

/// The native solver on the spike's models.
pub struct N2 {
    pub s: Stepper,
    pub model: Model,
    dt: f64,
}

impl N2 {
    pub fn new(model: Model, method: Method, dt: f64) -> Self {
        Self::with_algo(model, Algo::Aba, method, dt)
    }
    pub fn with_algo(model: Model, algo: Algo, method: Method, dt: f64) -> Self {
        let s = match model {
            Model::Pendulum { amplitude } => {
                Stepper::new(pendulum(), algo, method, &[amplitude], &[])
            }
            Model::CartPole => Stepper::new(cartpole(), algo, method, &[0.0, cp::THETA0], &[]),
            Model::Chain { links } => Stepper::new(chain(links), algo, method, &[], &[]),
            _ => panic!("the native solver has no contacts"),
        };
        N2 { s, model, dt }
    }
}

impl common::Sim for N2 {
    fn label(&self) -> String {
        let a = match self.s.algo {
            Algo::Aba => "ABA",
            Algo::Crba => "CRBA",
        };
        format!("Native 2D, {a}, {}", method_name(self.s.method))
    }
    fn dt(&self) -> f64 {
        self.dt
    }
    fn step(&mut self, input: &[f64]) {
        if let Model::CartPole = self.model {
            self.s.tau[0] = input[0];
            self.s.ext.clear();
            if input[1] != 0.0 || input[2] != 0.0 {
                self.s.ext.push(PointForce {
                    body: 1,
                    point: [0.0, cp::LEN],
                    force: [input[1], input[2]],
                });
            }
        }
        self.s.step(self.dt);
    }
    fn coords(&self) -> Vec<f64> {
        let x = &self.s.x;
        match self.model {
            Model::Pendulum { .. } => vec![x[0], x[1]],
            Model::CartPole => vec![x[0], x[2], x[1], x[3]],
            _ => self.rows(),
        }
    }
    fn rows(&self) -> Vec<f64> {
        self.s
            .body_states()
            .iter()
            .flat_map(|b| [b.x, b.y, b.angle, b.vx, b.vy, b.omega])
            .collect()
    }
    fn energy(&self) -> f64 {
        // Potential zero at the bottom of the swing, as for the others.
        self.s.energy() + pd::M * common::G * pd::L
    }
    fn warnings(&self) -> usize {
        self.s.failures
    }
    fn snapshot(&self, kind: SnapKind) -> Snapshot {
        // The whole state is `[q, q̇]`: every kind of snapshot is that vector,
        // kept as numbers or as bytes.
        match kind {
            SnapKind::Memory | SnapKind::Minimal => Snapshot {
                kind,
                bytes: self.s.x.len() * 8,
                data: Box::new(self.s.x.clone()),
            },
            SnapKind::Bytes | SnapKind::BytesFresh => {
                let b: Vec<u8> = self.s.x.iter().flat_map(|v| v.to_le_bytes()).collect();
                Snapshot {
                    kind,
                    bytes: b.len(),
                    data: Box::new(b),
                }
            }
        }
    }
    fn restore(&mut self, snap: &Snapshot) {
        match snap.kind {
            SnapKind::Memory | SnapKind::Minimal => {
                let v: &Vec<f64> = snap.data.downcast_ref().unwrap();
                self.s.x.copy_from_slice(v);
            }
            SnapKind::Bytes | SnapKind::BytesFresh => {
                let b: &Vec<u8> = snap.data.downcast_ref().unwrap();
                if snap.kind == SnapKind::BytesFresh {
                    self.s = self.s.fresh();
                }
                for (x, c) in self.s.x.iter_mut().zip(b.chunks_exact(8)) {
                    *x = f64::from_le_bytes(c.try_into().unwrap());
                }
            }
        }
    }
}
