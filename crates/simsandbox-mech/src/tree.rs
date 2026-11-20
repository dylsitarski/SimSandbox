//! The dynamics of a tree of rigid bodies in the plane (ADR 0103): each
//! body hangs from its parent (or the ground) by a joint that turns, slides
//! or is welded, and the motion is in joint coordinates `q`, `q̇`.
//!
//! `q̈` comes from Featherstone's articulated-body algorithm ([`aba`]), O(n)
//! in the bodies. The composite rigid-body algorithm with recursive
//! Newton–Euler ([`crba_rnea`]) gives the same `q̈` by another route, O(n³),
//! and is kept as the first's test oracle.
//!
//! Spatial algebra in 2D: a motion vector is `(ω, vx, vy)` and a force
//! vector `(n, fx, fy)`, both in a body's frame at its origin, the joint.
//! Gravity enters as an upward acceleration of the ground, so no body needs
//! a gravity force.
//!
//! The numbers ([`Numbers`]) are separate from the structure ([`Tree`]): a
//! block reads them from its parameters at every evaluation, so a slider on
//! a mass acts at once (ADR 0105 point 3).

// Spatial algebra reads best indexed by row and column.
#![allow(clippy::needless_range_loop)]

type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];

/// How a body is joined to its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Turning about the joint's point: one coordinate, an angle.
    Revolute,
    /// Sliding along the joint frame's x axis: one coordinate, a length.
    Prismatic,
    /// Welded: no coordinate.
    Fixed,
}

/// One body's place in the tree.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    /// The parent body, listed before this one; `None` is the ground.
    pub parent: Option<usize>,
    pub kind: Kind,
    /// Its joint's coordinate's index, `None` for a fixed joint.
    pub coord: Option<usize>,
}

/// The tree's structure: what does not change while it runs.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    links: Vec<Link>,
    dof: usize,
}

/// One body's numbers and its joint's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Numbers {
    /// Mass (kg).
    pub mass: f64,
    /// Moment of inertia about the centre of mass (kg·m²).
    pub inertia: f64,
    /// The centre of mass in the body's frame (m).
    pub centre: [f64; 2],
    /// The joint's point in the parent's frame (m).
    pub at: [f64; 2],
    /// The joint frame's fixed rotation from the parent's (rad).
    pub turn: f64,
    /// Viscous friction in the joint: a generalized force `−damping·q̇`.
    pub damping: f64,
    /// A spring in the joint: a generalized force `−stiffness·(q − rest)`.
    pub stiffness: f64,
    pub rest: f64,
}

/// A force in world axes at a body's centre of mass, and a torque.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    pub body: usize,
    pub force: [f64; 2],
    pub torque: f64,
}

/// A body's frame in the world: its origin (the joint's point), its angle,
/// and the velocity of its origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub vx: f64,
    pub vy: f64,
    pub omega: f64,
}

impl Tree {
    /// A tree of `links` in order; a parent must come before its children.
    /// Coordinates are numbered in order over the joints that have one.
    pub fn new(parents: &[(Option<usize>, Kind)]) -> Result<Self, String> {
        let mut links = Vec::with_capacity(parents.len());
        let mut dof = 0;
        for (i, &(parent, kind)) in parents.iter().enumerate() {
            if parent.is_some_and(|p| p >= i) {
                return Err(format!("body {i}'s parent is not listed before it"));
            }
            let coord = (kind != Kind::Fixed).then(|| {
                dof += 1;
                dof - 1
            });
            links.push(Link {
                parent,
                kind,
                coord,
            });
        }
        Ok(Self { links, dof })
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// The number of bodies.
    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// The number of joint coordinates.
    pub fn dof(&self) -> usize {
        self.dof
    }
}

/// The joint's motion subspace in the body's frame.
fn subspace(kind: Kind) -> V3 {
    match kind {
        Kind::Revolute => [1.0, 0.0, 0.0],
        Kind::Prismatic => [0.0, 1.0, 0.0],
        Kind::Fixed => [0.0; 3],
    }
}

/// Spatial inertia at the body's origin.
fn spatial_inertia(n: &Numbers) -> M3 {
    let (m, [cx, cy]) = (n.mass, n.centre);
    [
        [n.inertia + m * (cx * cx + cy * cy), -m * cy, m * cx],
        [-m * cy, m, 0.0],
        [m * cx, 0.0, m],
    ]
}

/// The body's frame in its parent's, at coordinate `q`: rotation and
/// origin. A prismatic joint slides along the joint frame's x axis.
fn placement(kind: Kind, n: &Numbers, q: f64) -> (f64, [f64; 2]) {
    match kind {
        Kind::Revolute => (n.turn + q, n.at),
        Kind::Prismatic => {
            let (s, c) = n.turn.sin_cos();
            (n.turn, [n.at[0] + c * q, n.at[1] + s * q])
        }
        Kind::Fixed => (n.turn, n.at),
    }
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
    fn motion(&self, v: V3) -> V3 {
        let lx = v[1] - v[0] * self.ry;
        let ly = v[2] + v[0] * self.rx;
        [v[0], self.c * lx + self.s * ly, -self.s * lx + self.c * ly]
    }

    /// A force vector from the child's frame to the parent's (Xᵀ f).
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

fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}

fn mv(a: &M3, v: V3) -> V3 {
    [
        a[0][0] * v[0] + a[0][1] * v[1] + a[0][2] * v[2],
        a[1][0] * v[0] + a[1][1] * v[1] + a[1][2] * v[2],
        a[2][0] * v[0] + a[2][1] * v[1] + a[2][2] * v[2],
    ]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Motion cross product v ×m u.
fn crm(v: V3, u: V3) -> V3 {
    [0.0, -v[0] * u[2] + v[2] * u[0], v[0] * u[1] - v[1] * u[0]]
}

/// Force cross product v ×* f.
fn crf(v: V3, f: V3) -> V3 {
    [-v[2] * f[1] + v[1] * f[2], -v[0] * f[2], v[0] * f[1]]
}

/// Working arrays, kept between calls: sized on first use and when the
/// tree's size changes, never allocated otherwise.
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
    bias: Vec<f64>,
}

impl Scratch {
    fn size(&mut self, n: usize, dof: usize) {
        if self.xf.len() != n || self.bias.len() != dof {
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
                m: vec![0.0; dof * dof],
                bias: vec![0.0; dof],
            };
        }
    }
}

/// The coordinate and rate of body `i`'s joint (0 for a fixed one).
fn joint(link: &Link, q: &[f64], qd: &[f64]) -> (f64, f64) {
    link.coord.map_or((0.0, 0.0), |k| (q[k], qd[k]))
}

/// Pass 1 of every algorithm: each body's transform, world pose and spatial
/// velocity, and the velocity-product acceleration `c`.
fn kinematics(tree: &Tree, nums: &[Numbers], q: &[f64], qd: &[f64], w: &mut Scratch) {
    for (i, l) in tree.links.iter().enumerate() {
        let (qi, qdi) = joint(l, q, qd);
        let (th, r) = placement(l.kind, &nums[i], qi);
        let xf = Xf::new(th, r);
        let (thp, pp, vp) = match l.parent {
            Some(p) => (w.theta_w[p], w.p_w[p], w.v[p]),
            None => (0.0, [0.0, 0.0], [0.0; 3]),
        };
        let (sp, cp) = thp.sin_cos();
        w.theta_w[i] = thp + th;
        w.p_w[i] = [pp[0] + cp * r[0] - sp * r[1], pp[1] + sp * r[0] + cp * r[1]];
        let vj = scale(subspace(l.kind), qdi);
        let v = add(xf.motion(vp), vj);
        w.v[i] = v;
        w.c[i] = crm(v, vj);
        w.xf[i] = xf;
    }
}

/// Loads as spatial forces in the bodies' frames, subtracted from `f`
/// (bias forces are what the joints must overcome). Needs [`kinematics`].
fn subtract_loads(nums: &[Numbers], loads: &[Load], theta_w: &[f64], f: &mut [V3]) {
    for e in loads {
        let (s, c) = theta_w[e.body].sin_cos();
        let fx = c * e.force[0] + s * e.force[1];
        let fy = -s * e.force[0] + c * e.force[1];
        let [cx, cy] = nums[e.body].centre;
        let g = &mut f[e.body];
        g[0] -= cx * fy - cy * fx + e.torque;
        g[1] -= fx;
        g[2] -= fy;
    }
}

/// The ground's acceleration in the world: up, against gravity.
fn base_accel(gravity: [f64; 2]) -> V3 {
    [0.0, -gravity[0], -gravity[1]]
}

/// The joint's own generalized force: actuation less damping and spring.
fn joint_force(n: &Numbers, tau: f64, q: f64, qd: f64) -> f64 {
    tau - n.damping * qd - n.stiffness * (q - n.rest)
}

/// What the dynamics are evaluated at: the numbers, gravity, the state, the
/// joint forces (one per coordinate) and the loads.
#[derive(Clone, Copy)]
pub struct At<'a> {
    pub nums: &'a [Numbers],
    pub gravity: [f64; 2],
    pub q: &'a [f64],
    pub qd: &'a [f64],
    pub tau: &'a [f64],
    pub loads: &'a [Load],
}

/// Featherstone's articulated-body algorithm: `q̈` in O(n). A joint whose
/// subtree can give it no inertia (`d = 0`) makes its `q̈` not a number.
pub fn aba(tree: &Tree, at: At<'_>, qdd: &mut [f64], w: &mut Scratch) {
    let n = tree.len();
    w.size(n, tree.dof);
    kinematics(tree, at.nums, at.q, at.qd, w);
    for i in 0..n {
        let ii = spatial_inertia(&at.nums[i]);
        w.ia[i] = ii;
        w.pa[i] = crf(w.v[i], mv(&ii, w.v[i]));
    }
    subtract_loads(at.nums, at.loads, &w.theta_w, &mut w.pa);
    for i in (0..n).rev() {
        let l = tree.links[i];
        let (ia, pa) = match l.coord {
            Some(k) => {
                let s = subspace(l.kind);
                let uv = mv(&w.ia[i], s);
                let d = dot(s, uv);
                let u = joint_force(&at.nums[i], at.tau[k], at.q[k], at.qd[k]) - dot(s, w.pa[i]);
                w.u_vec[i] = uv;
                w.d[i] = d;
                w.u[i] = u;
                let mut ia = w.ia[i];
                for r in 0..3 {
                    for c in 0..3 {
                        ia[r][c] -= uv[r] * uv[c] / d;
                    }
                }
                let pa = add(add(w.pa[i], mv(&ia, w.c[i])), scale(uv, u / d));
                (ia, pa)
            }
            // A weld passes the whole inertia on; it adds no velocity, so
            // its `c` is 0.
            None => (w.ia[i], w.pa[i]),
        };
        if let Some(p) = l.parent {
            let ip = w.xf[i].inertia_to_parent(&ia);
            for r in 0..3 {
                for c in 0..3 {
                    w.ia[p][r][c] += ip[r][c];
                }
            }
            w.pa[p] = add(w.pa[p], w.xf[i].force_to_parent(pa));
        }
    }
    let a0 = base_accel(at.gravity);
    for i in 0..n {
        let l = tree.links[i];
        let ap = l.parent.map_or(a0, |p| w.a[p]);
        let mut a = add(w.xf[i].motion(ap), w.c[i]);
        if let Some(k) = l.coord {
            let qa = (w.u[i] - dot(w.u_vec[i], a)) / w.d[i];
            qdd[k] = qa;
            a = add(a, scale(subspace(l.kind), qa));
        }
        w.a[i] = a;
    }
}

/// Recursive Newton–Euler: the generalized forces the joints must give for
/// accelerations `qdd`, with gravity, the velocity terms and the loads
/// (`at.tau` is not read).
pub fn rnea(tree: &Tree, at: At<'_>, qdd: &[f64], tau: &mut [f64], w: &mut Scratch) {
    let n = tree.len();
    w.size(n, tree.dof);
    kinematics(tree, at.nums, at.q, at.qd, w);
    let a0 = base_accel(at.gravity);
    for i in 0..n {
        let l = tree.links[i];
        let ap = l.parent.map_or(a0, |p| w.a[p]);
        let qa = l.coord.map_or(0.0, |k| qdd[k]);
        let a = add(add(w.xf[i].motion(ap), w.c[i]), scale(subspace(l.kind), qa));
        w.a[i] = a;
        let ii = spatial_inertia(&at.nums[i]);
        w.pa[i] = add(mv(&ii, a), crf(w.v[i], mv(&ii, w.v[i])));
    }
    subtract_loads(at.nums, at.loads, &w.theta_w, &mut w.pa);
    for i in (0..n).rev() {
        let l = tree.links[i];
        if let Some(k) = l.coord {
            tau[k] = dot(subspace(l.kind), w.pa[i]);
        }
        if let Some(p) = l.parent {
            let fp = w.xf[i].force_to_parent(w.pa[i]);
            w.pa[p] = add(w.pa[p], fp);
        }
    }
}

/// `q̈` by the composite rigid-body algorithm and recursive Newton–Euler:
/// `M q̈ = τ − b q̇ − k (q − rest) − C(q, q̇, loads)`, M by Cholesky. O(n³):
/// [`aba`]'s test oracle.
pub fn crba_rnea(tree: &Tree, at: At<'_>, qdd: &mut [f64], w: &mut Scratch) {
    let n = tree.len();
    let dof = tree.dof;
    w.size(n, dof);
    let mut bias = std::mem::take(&mut w.bias);
    let zeros = vec![0.0; dof];
    rnea(tree, at, &zeros, &mut bias, w);
    // The composite inertias, with the transforms `rnea` left.
    for i in 0..n {
        w.ia[i] = spatial_inertia(&at.nums[i]);
    }
    for i in (0..n).rev() {
        if let Some(p) = tree.links[i].parent {
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
        let li = tree.links[i];
        let Some(ci) = li.coord else { continue };
        let mut f = mv(&w.ia[i], subspace(li.kind));
        w.m[ci * dof + ci] = dot(subspace(li.kind), f);
        let mut j = i;
        while let Some(p) = tree.links[j].parent {
            f = w.xf[j].force_to_parent(f);
            j = p;
            let lj = tree.links[j];
            if let Some(cj) = lj.coord {
                let mij = dot(subspace(lj.kind), f);
                w.m[ci * dof + cj] = mij;
                w.m[cj * dof + ci] = mij;
            }
        }
    }
    for (i, l) in tree.links.iter().enumerate() {
        if let Some(k) = l.coord {
            qdd[k] = joint_force(&at.nums[i], at.tau[k], at.q[k], at.qd[k]) - bias[k];
        }
    }
    w.bias = bias;
    cholesky_solve(&mut w.m, dof, qdd);
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

/// Every body's frame in the world and its origin's velocity, into `out`.
pub fn poses(
    tree: &Tree,
    nums: &[Numbers],
    q: &[f64],
    qd: &[f64],
    out: &mut [Pose],
    w: &mut Scratch,
) {
    w.size(tree.len(), tree.dof);
    kinematics(tree, nums, q, qd, w);
    for (i, p) in out.iter_mut().enumerate() {
        let (s, c) = w.theta_w[i].sin_cos();
        let v = w.v[i];
        *p = Pose {
            x: w.p_w[i][0],
            y: w.p_w[i][1],
            angle: w.theta_w[i],
            vx: c * v[1] - s * v[2],
            vy: s * v[1] + c * v[2],
            omega: v[0],
        };
    }
}

/// Each body's centre of mass in the world and its velocity: `(x, y, vx,
/// vy, ω)`. Needs [`kinematics`].
fn centres<'a>(nums: &'a [Numbers], w: &'a Scratch) -> impl Iterator<Item = [f64; 5]> + 'a {
    nums.iter().enumerate().map(|(i, b)| {
        let (s, c) = w.theta_w[i].sin_cos();
        let [cx, cy] = b.centre;
        let v = w.v[i];
        // The centre's velocity in the body's axes, then the world's.
        let (bx, by) = (v[1] - v[0] * cy, v[2] + v[0] * cx);
        [
            w.p_w[i][0] + c * cx - s * cy,
            w.p_w[i][1] + s * cx + c * cy,
            c * bx - s * by,
            s * bx + c * by,
            v[0],
        ]
    })
}

/// Kinetic energy, plus the potential of gravity (0 at the world's origin)
/// and of the joints' springs.
pub fn energy(
    tree: &Tree,
    nums: &[Numbers],
    gravity: [f64; 2],
    q: &[f64],
    qd: &[f64],
    w: &mut Scratch,
) -> f64 {
    w.size(tree.len(), tree.dof);
    kinematics(tree, nums, q, qd, w);
    let bodies: f64 = centres(nums, w)
        .zip(nums)
        .map(|([x, y, vx, vy, om], b)| {
            0.5 * b.mass * (vx * vx + vy * vy) + 0.5 * b.inertia * om * om
                - b.mass * (gravity[0] * x + gravity[1] * y)
        })
        .sum();
    let springs: f64 = tree
        .links
        .iter()
        .zip(nums)
        .filter_map(|(l, n)| l.coord.map(|k| 0.5 * n.stiffness * (q[k] - n.rest).powi(2)))
        .sum();
    bodies + springs
}

/// Linear momentum and angular momentum about the world's origin.
pub fn momentum(tree: &Tree, nums: &[Numbers], q: &[f64], qd: &[f64], w: &mut Scratch) -> [f64; 3] {
    w.size(tree.len(), tree.dof);
    kinematics(tree, nums, q, qd, w);
    let mut out = [0.0; 3];
    for ([x, y, vx, vy, om], b) in centres(nums, w).zip(nums) {
        out[0] += b.mass * vx;
        out[1] += b.mass * vy;
        out[2] += b.inertia * om + b.mass * (x * vy - y * vx);
    }
    out
}
