//! The compiled-route comparison of ADR 0084 point 6, on Gray–Scott's
//! right-hand side: microseconds per evaluation, compile time, whether the
//! bits equal the interpreter's. See README.md.
//!
//! `SPIKE_SIZES=128,256` picks the grids, `SPIKE_EVALS` the evaluations
//! per round; run under `taskset -c 0` for one core.

mod routes;

#[cfg(feature = "cranelift")]
mod jit;
#[cfg(feature = "wasm")]
mod wasm;

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

/// Microseconds per call, the best of five rounds.
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

fn same(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Gray–Scott's constants as the fused code takes them.
#[derive(Clone, Copy)]
struct Consts {
    n: usize,
    inv: f64,
    du: f64,
    dv: f64,
    f: f64,
    k: f64,
    r: f64,
}

/// Route 5: one cell, the typed program's operations in its order.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
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

/// Route 5: native fused Rust, one loop over the cells, interior first.
fn fused(c: &Consts, x: &[f64], dx: &mut [f64]) {
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
            let (a, b) = cell(
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
            let (a, b) = cell(
                c, u0[i], u0[w], u0[e], us[i], un[i], v0[i], v0[w], v0[e], vs[i], vn[i],
            );
            du0[i] = a;
            dv0[i] = b;
        }
    }
}

/// A model whose rates use what Gray–Scott does not: value, gradient and
/// periodic edges, a first derivative, shifts, a comparison, a power, a
/// function and an axis.
fn mixed(cells: usize) -> Vec<String> {
    [
        format!("grid x [m] = 0 .. 1, {cells}"),
        format!("grid y [m] = 0 .. 1, {cells}"),
        "param D [m^2/s] = 0.01".into(),
        "param vel [m/s] = 0.2".into(),
        "param T0 [K] = 1".into(),
        "param left [K] = 1".into(),
        "param hot [K] = 1.2".into(),
        "param a [1/s] = 0.3".into(),
        "param g [K/(m*s)] = 0.1".into(),
        "param kx [1/m] = 6".into(),
        "param ky [1/m] = 4".into(),
        "state T[x, y] [K] = T0 + 0.5*T0*sin(kx*x)*cos(ky*y)".into(),
        "edge T: x- = left, x+ gradient = 0, y periodic".into(),
        "dT/dt = D*lap(T) - vel*ddx(T) + a*T0*(T > hot) + a*T0*(T/T0)^1.5 - a*T0*sin(T/T0) + a*(prev(T, y) - next(T, x)) + g*x".into(),
    ]
    .into()
}

/// Whether each compiled route gives the interpreter's bits on `mixed`.
fn check_mixed() {
    let n = 64;
    let set = EquationSet::parse("m", &mixed(n)).expect("valid");
    let p: Vec<f64> = set.spec().params.iter().map(|q| q.default).collect();
    let mut x = vec![0.0; n * n];
    set.initial_state(&p, &mut x);
    let mut want = vec![0.0; n * n];
    set.rhs(0.0, &x, &[], &p, &mut want);
    let mut out = String::from("every operation kind (64 × 64):");
    #[cfg(feature = "cranelift")]
    {
        let mut set = EquationSet::parse("m", &mixed(n)).expect("valid");
        let phases = set.compile_phases(&mut |v| Some(jit::compile(v)));
        let mut got = vec![0.0; n * n];
        set.rhs(0.0, &x, &[], &p, &mut got);
        out += &format!(" Cranelift {phases} phases [{}]", same(&got, &want));
        // Every parameter changed, as sliders would: the same code, no
        // compiling again, still the interpreter's bits.
        let moved: Vec<f64> = p.iter().map(|v| v * 1.37 + 0.01).collect();
        let interp = EquationSet::parse("m", &mixed(n)).expect("valid");
        let mut want2 = vec![0.0; n * n];
        interp.rhs(0.0, &x, &[], &moved, &mut want2);
        set.rhs(0.0, &x, &[], &moved, &mut got);
        out += &format!(" parameters moved [{}, results changed {}]", same(&got, &want2), !same(&want2, &want));
    }
    #[cfg(feature = "wasm")]
    {
        let engine = wasmtime::Engine::default();
        let mut set = EquationSet::parse("m", &mixed(n)).expect("valid");
        let phases = set.compile_phases(&mut |v| Some(wasm::compile(&engine, v)));
        let mut got = vec![0.0; n * n];
        set.rhs(0.0, &x, &[], &p, &mut got);
        out += &format!(" Wasmtime {phases} phases [{}]", same(&got, &want));
    }
    println!("{out}");
}

fn main() {
    check_mixed();
    let sizes: Vec<usize> = std::env::var("SPIKE_SIZES")
        .ok()
        .map(|v| v.split(',').map(|n| n.parse().unwrap()).collect())
        .unwrap_or(vec![128, 256, 512]);
    println!("µs per evaluation of Gray–Scott's rates (best of 5); compile ms; [same bits as the interpreter]");
    #[cfg(feature = "wasm")]
    let engine = wasmtime::Engine::default();
    for n in sizes {
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
        let mut x = vec![0.0; 2 * n * n];
        set.initial_state(&p, &mut x);
        let evals = std::env::var("SPIKE_EVALS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or((40_000_000 / (n * n)).max(20));
        let mut want = vec![0.0; 2 * n * n];
        let t = time(evals, || set.rhs(0.0, &x, &[], &p, &mut want));
        println!("{n}×{n}");
        println!("  1 interpreter          {t:>8.1}");

        #[cfg(feature = "cranelift")]
        {
            let mut set = EquationSet::parse("gs", &lines(n)).expect("valid");
            let start = Instant::now();
            let phases = set.compile_phases(&mut |v| Some(jit::compile(v)));
            let ms = start.elapsed().as_secs_f64() * 1e3;
            let mut got = vec![0.0; 2 * n * n];
            let t = time(evals, || set.rhs(0.0, &x, &[], &p, &mut got));
            println!(
                "  2 Cranelift            {t:>8.1}   compile {ms:.1} ms for {phases} phases  [{}]",
                same(&got, &want)
            );
        }

        #[cfg(feature = "wasm")]
        {
            use std::sync::atomic::Ordering;
            let mut set = EquationSet::parse("gs", &lines(n)).expect("valid");
            let start = Instant::now();
            let phases = set.compile_phases(&mut |v| Some(wasm::compile(&engine, v)));
            let ms = start.elapsed().as_secs_f64() * 1e3;
            let mut got = vec![0.0; 2 * n * n];
            wasm::INSIDE.store(0, Ordering::Relaxed);
            let t = time(evals, || set.rhs(0.0, &x, &[], &p, &mut got));
            let inside = wasm::INSIDE.load(Ordering::Relaxed) as f64 / 1e3 / (5 * evals) as f64;
            println!(
                "  3 Wasmtime, generated  {t:>8.1}   inside WebAssembly {inside:.1} (mean)  compile {ms:.1} ms for {phases} phases  [{}]",
                same(&got, &want)
            );

            // Route 4: the hand-written block, its grid in its own memory.
            let start = Instant::now();
            let wasm_bytes = wat::parse_str(include_str!("gray_scott.wat")).unwrap();
            let module = wasmtime::Module::new(&engine, &wasm_bytes).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1e3;
            let mut store = wasmtime::Store::new(&engine, ());
            let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
            let memory = instance.get_memory(&mut store, "memory").unwrap();
            type Rates = (i32, i32, i32, i32, i32, f64, f64, f64, f64, f64, f64);
            let rates = instance
                .get_typed_func::<Rates, ()>(&mut store, "rates")
                .unwrap();
            let bytes = 4 * n * n * 8;
            memory
                .grow(&mut store, (bytes as u64).div_ceil(65536))
                .unwrap();
            let base = 65536;
            let at = |k: usize| (base + k * n * n * 8) as i32;
            let h = value("size") / n as f64;
            let args: Rates = (
                n as i32,
                at(0),
                at(1),
                at(2),
                at(3),
                1.0 / (h * h),
                value("Du"),
                value("Dv"),
                value("r"),
                value("F"),
                value("F") + value("k"),
            );
            let put = |mem: &mut [u8], from: usize, xs: &[f64]| {
                for (c, v) in mem[from..from + 8 * xs.len()]
                    .as_chunks_mut::<8>()
                    .0
                    .iter_mut()
                    .zip(xs)
                {
                    *c = v.to_le_bytes();
                }
            };
            let take = |mem: &[u8], from: usize, xs: &mut [f64]| {
                for (v, c) in xs.iter_mut().zip(mem[from..].as_chunks::<8>().0.iter()) {
                    *v = f64::from_le_bytes(*c);
                }
            };
            put(memory.data_mut(&mut store), base, &x);
            let inside = time(evals, || rates.call(&mut store, args).unwrap());
            let mut got = vec![0.0; 2 * n * n];
            take(memory.data(&store), base + 2 * n * n * 8, &mut got);
            let same_bits = same(&got, &want);
            // With the state copied in and the rates out at each call, as
            // when the host owns them.
            let t = time(evals, || {
                put(memory.data_mut(&mut store), base, &x);
                rates.call(&mut store, args).unwrap();
                take(memory.data(&store), base + 2 * n * n * 8, &mut got);
            });
            println!(
                "  4 Wasmtime, by hand    {t:>8.1}   inside WebAssembly {inside:.1}  compile {ms:.1} ms  [{same_bits}]"
            );
        }

        let c = Consts {
            n,
            inv: {
                let h = value("size") / n as f64;
                1.0 / (h * h)
            },
            du: value("Du"),
            dv: value("Dv"),
            f: value("F"),
            k: value("k"),
            r: value("r"),
        };
        let mut got = vec![0.0; 2 * n * n];
        let t = time(evals, || fused(&c, &x, &mut got));
        println!(
            "  5 fused Rust           {t:>8.1}   [{}]",
            same(&got, &want)
        );
    }
}
