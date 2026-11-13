//! The implicit methods' LU (ADR 0087) against `faer`'s with partial
//! pivoting, on the matrices the implicit methods factorise: two coupled
//! fields on a periodic m × m grid (Gray–Scott's `I − c·h·J`), and the
//! heat equation on a chain and a ring of n cells.
//! `cargo run --release -p simsandbox-eq --example lu_speed [M] [N]`

use faer::linalg::solvers::Solve;
use faer::sparse::linalg::solvers::{Lu, SymbolicLu};
use faer::sparse::{SparseColMat, Triplet};
use simsandbox_eq::sparse_lu::Symbolic;
use std::time::Instant;

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

/// Factorise and solve `a` by both, `reps` times: (symbolic, factorise,
/// one solve) in ms for each, and the largest residual.
fn compare(name: &str, a: &SparseColMat<usize, f64>, reps: usize) {
    let n = a.nrows();
    let b: Vec<f64> = (0..n).map(|i| (i % 17) as f64 - 8.0).collect();
    let residual = |x: &[f64]| -> f64 {
        let mut r = b.clone();
        for (j, xj) in x.iter().enumerate() {
            for (i, v) in a.row_idx_of_col(j).zip(a.val_of_col(j)) {
                r[i] -= v * xj;
            }
        }
        r.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    };

    let t = Instant::now();
    let sym = SymbolicLu::try_new(a.symbolic()).expect("structure");
    let faer_sym = ms(t);
    let t = Instant::now();
    let mut lu = None;
    for _ in 0..reps {
        lu = Some(Lu::try_new_with_symbolic(sym.clone(), a.as_ref()).expect("factorises"));
    }
    let faer_num = ms(t) / reps as f64;
    let lu = lu.expect("factorised");
    let mut x = faer::Mat::<f64>::from_fn(n, 1, |i, _| b[i]);
    let t = Instant::now();
    for _ in 0..reps {
        x = faer::Mat::<f64>::from_fn(n, 1, |i, _| b[i]);
        lu.solve_in_place(x.as_mut());
    }
    let faer_solve = ms(t) / reps as f64;
    let faer_x: Vec<f64> = (0..n).map(|i| x[(i, 0)]).collect();

    let (cp, ri) = (a.symbolic().col_ptr(), a.symbolic().row_idx());
    let t = Instant::now();
    let own = Symbolic::new(n, cp, ri);
    let own_sym = ms(t);
    let t = Instant::now();
    let mut f = None;
    for _ in 0..reps {
        f = Some(own.factorise(a.val()).expect("pivots"));
    }
    let own_num = ms(t) / reps as f64;
    let f = f.expect("factorised");
    let mut y = b.clone();
    let t = Instant::now();
    for _ in 0..reps {
        y.copy_from_slice(&b);
        own.solve(&f, &mut y);
    }
    let own_solve = ms(t) / reps as f64;

    println!("{name} ({n} unknowns):");
    println!(
        "  faer, partial pivoting  symbolic {faer_sym:8.3}  factorise {faer_num:8.3}  solve {faer_solve:7.3} ms  residual {:.1e}",
        residual(&faer_x)
    );
    println!(
        "  own, no interchanges    symbolic {own_sym:8.3}  factorise {own_num:8.3}  solve {own_solve:7.3} ms  residual {:.1e}  ({} values)",
        residual(&y),
        own.factor_size()
    );
}

/// Two coupled fields on a periodic m × m grid: diffusion and a local
/// reaction, as Gray–Scott's `I − c·h·J` at c·h = 0.29.
fn grid(m: usize) -> SparseColMat<usize, f64> {
    let cells = m * m;
    let ch = 0.29;
    let mut t = Vec::new();
    for y in 0..m {
        for x in 0..m {
            let k = x + m * y;
            let nb = [
                (x + 1) % m + m * y,
                (x + m - 1) % m + m * y,
                x + m * ((y + 1) % m),
                x + m * ((y + m - 1) % m),
            ];
            let u = 0.5 + 0.1 * ((k * 7) % 13) as f64 / 13.0;
            let v = 0.25 * ((k * 5) % 11) as f64 / 11.0;
            for (f, d, own, other) in [
                (0, 0.2, -v * v - 0.035, -2.0 * u * v),
                (cells, 0.1, 2.0 * u * v - 0.1, v * v),
            ] {
                t.push(Triplet::new(f + k, f + k, 1.0 - ch * (own - 4.0 * d)));
                for &j in &nb {
                    t.push(Triplet::new(f + k, f + j, -ch * d));
                }
                t.push(Triplet::new(
                    f + k,
                    (f + cells) % (2 * cells) + k,
                    -ch * other,
                ));
            }
        }
    }
    SparseColMat::try_new_from_triplets(2 * cells, 2 * cells, &t).expect("matrix")
}

/// The heat equation's `I − c·h·J` on n cells, fixed ends or periodic.
fn line(n: usize, periodic: bool) -> SparseColMat<usize, f64> {
    let mut t = Vec::new();
    for i in 0..n {
        t.push(Triplet::new(i, i, 2.6));
        if periodic || i + 1 < n {
            t.push(Triplet::new(i, (i + 1) % n, -0.8));
        }
        if periodic || i > 0 {
            t.push(Triplet::new(i, (i + n - 1) % n, -0.8));
        }
    }
    SparseColMat::try_new_from_triplets(n, n, &t).expect("matrix")
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let m = args.first().copied().unwrap_or(128);
    let n = args.get(1).copied().unwrap_or(800);
    compare(&format!("grid {m} × {m}, two fields"), &grid(m), 5);
    compare(&format!("chain of {n}"), &line(n, false), 2000);
    compare(&format!("ring of {n}"), &line(n, true), 2000);
}
