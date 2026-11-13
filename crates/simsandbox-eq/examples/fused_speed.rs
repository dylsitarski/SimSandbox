//! How fast typed equations evaluate a large grid against hand-written,
//! fused Rust of the same right-hand side (Gray–Scott), and what fused
//! multiply-add (FMA), which changes the last bits, would add. Run with
//! `cargo run --release -p simsandbox-eq --example fused_speed`, and with
//! `taskset -c 0` in front for one core.
//!
//! Build it a second time for this machine's instructions (AVX2, FMA) to
//! see what they add: `RUSTFLAGS="-C target-cpu=native"` with another
//! `CARGO_TARGET_DIR`. Rust never fuses `a*b + c` by itself, so only the
//! FMA kernel's bits change.
//!
//! Kernels, each one evaluation of du/dt and dv/dt on a periodic grid:
//! - **typed**: the typed-equations block, as models run it;
//! - **fused**: one loop over the cells doing the same operations in the
//!   same order (same bits);
//! - **rewritten**: one five-point stencil instead of one per axis
//!   (different bits);
//! - **fused FMA**: the same with `mul_add` where it fits (different bits;
//!   a slow library call unless built for a CPU with FMA).

// Each cell function takes a cell and its four neighbours in both fields.
#![allow(clippy::too_many_arguments)]

use simsandbox_core::block::Equations;
use simsandbox_eq::ode::OdeSystem;
use simsandbox_eq::symbolic::EquationSet;
use std::time::Instant;

fn lines(cells: usize) -> Vec<String> {
    [
        format!("grid x [m] = 0 .. size, {cells}"),
        format!("grid y [m] = 0 .. size, {cells}"),
        "param size [m] = 1.25".into(),
        "param Du [m^2/s] = 2e-5".into(),
        "param Dv [m^2/s] = 1e-5".into(),
        "param F [1/s] = 0.035".into(),
        "param k [1/s] = 0.065".into(),
        "param r [1/s] = 1".into(),
        "param w [m] = 0.04".into(),
        "param ax [m] = 0.55".into(),
        "param ay [m] = 0.6".into(),
        "param bx [m] = 0.75".into(),
        "param by [m] = 0.7".into(),
        "state u[x, y] = 1 - 0.5*(exp(-((x - ax)^2 + (y - ay)^2)/w^2) + exp(-((x - bx)^2 + (y - by)^2)/w^2))".into(),
        "state v[x, y] = 0.25*(exp(-((x - ax)^2 + (y - ay)^2)/w^2) + exp(-((x - bx)^2 + (y - by)^2)/w^2))".into(),
        "edge u: x periodic, y periodic".into(),
        "edge v: x periodic, y periodic".into(),
        "reaction = r*u*v^2".into(),
        "du/dt = Du*lap(u) - reaction + F*(1 - u)".into(),
        "dv/dt = Dv*lap(v) + reaction - (F + k)*v".into(),
    ]
    .into()
}

#[derive(Clone, Copy)]
struct Consts {
    n: usize,
    /// 1/h², as typed equations' stencils multiply by it.
    inv: f64,
    du: f64,
    dv: f64,
    f: f64,
    k: f64,
    r: f64,
}

/// One cell, in the typed program's order of operations.
#[inline(always)]
fn cell(
    c: &Consts,
    uc: f64,
    uw: f64,
    ue: f64,
    us: f64,
    un: f64,
    vc: f64,
    vw: f64,
    ve: f64,
    vs: f64,
    vn: f64,
) -> (f64, f64) {
    let lu = (uw - 2.0 * uc + ue) * c.inv + (us - 2.0 * uc + un) * c.inv;
    let lv = (vw - 2.0 * vc + ve) * c.inv + (vs - 2.0 * vc + vn) * c.inv;
    let reaction = c.r * uc * (vc * vc);
    (
        c.du * lu - reaction + c.f * (1.0 - uc),
        c.dv * lv + reaction - (c.f + c.k) * vc,
    )
}

/// The same written another way: one five-point stencil, the neighbours
/// added first, multiplied by 1/h² once instead of once per axis. Still
/// exact on any machine, but not the same bits as `cell`.
#[inline(always)]
fn cell_rewritten(
    c: &Consts,
    uc: f64,
    uw: f64,
    ue: f64,
    us: f64,
    un: f64,
    vc: f64,
    vw: f64,
    ve: f64,
    vs: f64,
    vn: f64,
) -> (f64, f64) {
    let inv = c.inv;
    let lu = ((uw + ue) + (us + un) - 4.0 * uc) * inv;
    let lv = ((vw + ve) + (vs + vn) - 4.0 * vc) * inv;
    let reaction = c.r * uc * (vc * vc);
    (
        c.du * lu - reaction + c.f * (1.0 - uc),
        c.dv * lv + reaction - (c.f + c.k) * vc,
    )
}

/// `cell`'s operations with fused multiply-adds where `a*b + c` appears.
#[inline(always)]
fn cell_fma_only(
    c: &Consts,
    uc: f64,
    uw: f64,
    ue: f64,
    us: f64,
    un: f64,
    vc: f64,
    vw: f64,
    ve: f64,
    vs: f64,
    vn: f64,
) -> (f64, f64) {
    let lu = (-2.0f64).mul_add(uc, uw) + ue;
    let lu = lu * c.inv + ((-2.0f64).mul_add(uc, us) + un) * c.inv;
    let lv = (-2.0f64).mul_add(vc, vw) + ve;
    let lv = lv * c.inv + ((-2.0f64).mul_add(vc, vs) + vn) * c.inv;
    let reaction = c.r * uc * (vc * vc);
    (
        c.du.mul_add(lu, -reaction) + c.f * (1.0 - uc),
        c.dv.mul_add(lv, reaction) - (c.f + c.k) * vc,
    )
}

/// The rewritten cell with fused multiply-adds where they fit.
#[inline(always)]
fn cell_fma(
    c: &Consts,
    uc: f64,
    uw: f64,
    ue: f64,
    us: f64,
    un: f64,
    vc: f64,
    vw: f64,
    ve: f64,
    vs: f64,
    vn: f64,
) -> (f64, f64) {
    let inv = c.inv;
    let lu = ((-4.0f64).mul_add(uc, uw + ue) + (us + un)) * inv;
    let lv = ((-4.0f64).mul_add(vc, vw + ve) + (vs + vn)) * inv;
    let reaction = (c.r * uc) * (vc * vc);
    (
        c.du.mul_add(lu, c.f.mul_add(1.0 - uc, -reaction)),
        c.dv.mul_add(lv, (-(c.f + c.k)).mul_add(vc, reaction)),
    )
}

/// Every cell of the grid through `f`, rows interior first, edges wrapped.
#[inline(always)]
fn sweep(
    c: &Consts,
    x: &[f64],
    dx: &mut [f64],
    f: impl Fn(&Consts, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64) -> (f64, f64),
) {
    let n = c.n;
    let cells = n * n;
    let (u, v) = x.split_at(cells);
    let (du, dv) = dx.split_at_mut(cells);
    for j in 0..n {
        let (s, nn) = ((j + n - 1) % n * n, (j + 1) % n * n);
        let row = j * n;
        let (u0, us, un) = (&u[row..row + n], &u[s..s + n], &u[nn..nn + n]);
        let (v0, vs, vn) = (&v[row..row + n], &v[s..s + n], &v[nn..nn + n]);
        let (du0, dv0) = (&mut du[row..row + n], &mut dv[row..row + n]);
        for i in 1..n - 1 {
            let (a, b) = f(
                c,
                u0[i],
                u0[i - 1],
                u0[i + 1],
                us[i],
                un[i],
                v0[i],
                v0[i - 1],
                v0[i + 1],
                vs[i],
                vn[i],
            );
            du0[i] = a;
            dv0[i] = b;
        }
        for i in [0, n - 1] {
            let (w, e) = ((i + n - 1) % n, (i + 1) % n);
            let (a, b) = f(
                c, u0[i], u0[w], u0[e], us[i], un[i], v0[i], v0[w], v0[e], vs[i], vn[i],
            );
            du0[i] = a;
            dv0[i] = b;
        }
    }
}

fn fused(c: &Consts, x: &[f64], dx: &mut [f64]) {
    sweep(c, x, dx, cell);
}

fn fused_fma(c: &Consts, x: &[f64], dx: &mut [f64]) {
    sweep(c, x, dx, cell_fma);
}

fn fused_fma_only(c: &Consts, x: &[f64], dx: &mut [f64]) {
    sweep(c, x, dx, cell_fma_only);
}

fn fused_rewritten(c: &Consts, x: &[f64], dx: &mut [f64]) {
    sweep(c, x, dx, cell_rewritten);
}

/// Microseconds per evaluation, the best of five.
fn time(evals: usize, mut f: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..5 {
        let start = Instant::now();
        for _ in 0..evals {
            f();
        }
        best = best.min(start.elapsed().as_secs_f64() / evals as f64 * 1e6);
    }
    best
}

fn main() {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("cores available: {cores}");
    let fma = cfg!(target_feature = "fma");
    println!("built with FMA instructions: {fma}");
    println!("µs per evaluation; in brackets, the largest difference from `fused` against the largest rate");
    println!(
        "{:>9} {:>9} {:>9} {:>20} {:>20} {:>20}",
        "grid", "typed", "fused", "rewritten", "FMA only", "rewritten + FMA"
    );
    for n in [128, 256, 512] {
        let set = EquationSet::parse("gs", &lines(n)).expect("valid");
        let p: Vec<f64> = set.spec().params.iter().map(|q| q.default).collect();
        let value = |name: &str| {
            p[set
                .spec()
                .params
                .iter()
                .position(|q| q.name == name)
                .unwrap()]
        };
        let states = 2 * n * n;
        let mut x = vec![0.0; states];
        set.initial_state(&p, &mut x);
        let h = value("size") / n as f64;
        let c = Consts {
            n,
            inv: 1.0 / (h * h),
            du: value("Du"),
            dv: value("Dv"),
            f: value("F"),
            k: value("k"),
            r: value("r"),
        };
        let evals = (40_000_000 / (n * n)).max(20);
        let mut typed: Vec<f64> = vec![0.0; states];
        let t_typed = time(evals, || set.rhs(0.0, &x, &[], &p, &mut typed));
        let mut plain: Vec<f64> = vec![0.0; states];
        let t_fused = time(evals, || fused(&c, &x, &mut plain));
        let same = typed
            .iter()
            .zip(&plain)
            .all(|(a, b)| a.to_bits() == b.to_bits());
        assert!(
            typed
                .iter()
                .zip(&plain)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "typed and fused differ"
        );
        let _ = same;
        let scale = plain.iter().fold(0.0f64, |m, b| m.max(b.abs()));
        let other = |f: fn(&Consts, &[f64], &mut [f64]), slow: bool| {
            let mut out = vec![0.0f64; states];
            let t = time(if slow { 1 + evals / 20 } else { evals }, || {
                f(&c, &x, &mut out)
            });
            let diff = out
                .iter()
                .zip(&plain)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f64, f64::max)
                / scale;
            format!("{t:.1} ({diff:.0e})")
        };
        let rewritten = other(fused_rewritten, false);
        let fma_only = other(fused_fma_only, !fma);
        let both = other(fused_fma, !fma);
        println!(
            "{:>9} {t_typed:>9.1} {t_fused:>9.1} {rewritten:>20} {fma_only:>20} {both:>20}",
            format!("{n}×{n}")
        );
    }
}
