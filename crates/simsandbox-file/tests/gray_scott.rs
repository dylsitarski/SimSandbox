//! Gray–Scott reaction–diffusion on a 2D grid (ADR 0060): the qualitative
//! check of the example. From two seeds, spots grow and split (Pearson's
//! "mitosis" regime, F = 0.035, k = 0.065): their number rises, each keeps
//! the size the parameters set, and the concentrations stay in [0, 1].

use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_file::load;

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    r
}

/// Spots: connected groups of cells above `level` on an `n` by `n` grid
/// that wraps around (4 neighbours). Returns each spot's size in cells.
fn spots(v: &[f64], n: usize, level: f64) -> Vec<usize> {
    let mut seen = vec![false; v.len()];
    let mut sizes = Vec::new();
    for start in 0..v.len() {
        if seen[start] || v[start] <= level {
            continue;
        }
        let (mut stack, mut size) = (vec![start], 0);
        seen[start] = true;
        while let Some(k) = stack.pop() {
            size += 1;
            let (i, j) = (k % n, k / n);
            for (di, dj) in [(1, 0), (n - 1, 0), (0, 1), (0, n - 1)] {
                let m = (i + di) % n + ((j + dj) % n) * n;
                if !seen[m] && v[m] > level {
                    seen[m] = true;
                    stack.push(m);
                }
            }
        }
        sizes.push(size);
    }
    sizes
}

#[test]
fn spots_grow_and_split() {
    // The example on a quarter of its area (64 × 64 cells of the same
    // size), seeds moved into it, to t = 3000 s.
    let text = include_str!("../../../examples/gray-scott.ssbx.json")
        .replace("size, 128", "size, 64")
        .replace(r#""param size [m] = 1.25""#, r#""param size [m] = 0.625""#)
        .replace(r#""param ax [m] = 0.55""#, r#""param ax [m] = 0.25""#)
        .replace(r#""param ay [m] = 0.6""#, r#""param ay [m] = 0.3""#)
        .replace(r#""param bx [m] = 0.75""#, r#""param bx [m] = 0.42""#)
        .replace(r#""param by [m] = 0.7""#, r#""param by [m] = 0.36""#);
    let loaded = load(&text, &registry()).expect("loads");
    let id = loaded.ids["gs"];
    let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
    let count = |sim: &Simulator| spots(sim.output(id, "v").unwrap(), 64, 0.15).len();
    let at_start = count(&sim);
    for _ in 0..3000 {
        sim.step(1.0).unwrap();
    }
    let v = sim.output(id, "v").unwrap();
    assert!(v.iter().all(|v| (0.0..=1.0).contains(v)), "v left [0, 1]");
    let sizes = spots(v, 64, 0.15);
    assert!(
        sizes.len() >= 2 * at_start.max(1),
        "{at_start} seeds became {} spots: {sizes:?}",
        sizes.len()
    );
    // Each spot about the width the parameters set: tens of cells.
    assert!(sizes.iter().all(|&s| (10..200).contains(&s)), "{sizes:?}");
}
