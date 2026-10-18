//! Sampling designs (ADR 0028): the parameter values a sweep evaluates a
//! model at. Every design returns points, one value per parameter, inside
//! each parameter's `[low, high]` range.
//!
//! - [`grid`]: every combination of evenly spaced values, first parameter
//!   fastest, range ends exact;
//! - [`random`]: independent uniform values from a seeded generator;
//! - [`latin_hypercube`]: one point in each of `count` equal slices of
//!   every parameter, slices paired at random;
//! - [`sobol`]: the first `count` points of the Sobol low-discrepancy
//!   sequence (Joe–Kuo direction numbers), up to [`SOBOL_MAX_PARAMS`]
//!   parameters.

/// The most parameters [`sobol`] supports.
pub const SOBOL_MAX_PARAMS: usize = 10;

/// A swept parameter's range and, for grids, number of values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub range: [f64; 2],
    pub points: usize,
}

/// `k` of `n` evenly spaced values over `[a, b]`; weighted so both ends
/// are exact.
pub fn lerp_index(a: f64, b: f64, k: usize, n: usize) -> f64 {
    if n < 2 {
        return a;
    }
    let f = k as f64 / (n - 1) as f64;
    a * (1.0 - f) + b * f
}

/// Every combination of the axes' values; the first axis varies fastest,
/// so point `j * n0 + i` has value `i` of axis 0 and `j` of axis 1.
pub fn grid(axes: &[Axis]) -> Vec<Vec<f64>> {
    let total: usize = axes.iter().map(|a| a.points).product();
    (0..total)
        .map(|mut k| {
            axes.iter()
                .map(|a| {
                    let i = k % a.points;
                    k /= a.points;
                    lerp_index(a.range[0], a.range[1], i, a.points)
                })
                .collect()
        })
        .collect()
}

/// A small, fast, seeded generator (SplitMix64); the same seed always gives
/// the same points.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in 0..n.
    pub fn below(&mut self, n: usize) -> usize {
        (self.unit() * n as f64) as usize % n.max(1)
    }
}

fn scale(unit: &[f64], ranges: &[[f64; 2]]) -> Vec<f64> {
    unit.iter()
        .zip(ranges)
        .map(|(u, [a, b])| a + u * (b - a))
        .collect()
}

/// `count` independent uniform points.
pub fn random(ranges: &[[f64; 2]], count: usize, seed: u64) -> Vec<Vec<f64>> {
    let mut rng = Rng::new(seed);
    (0..count)
        .map(|_| {
            let u: Vec<f64> = ranges.iter().map(|_| rng.unit()).collect();
            scale(&u, ranges)
        })
        .collect()
}

/// `count` points with exactly one in each of `count` equal slices of every
/// parameter, at a random place in its slice.
pub fn latin_hypercube(ranges: &[[f64; 2]], count: usize, seed: u64) -> Vec<Vec<f64>> {
    let mut rng = Rng::new(seed);
    // A random order of slices per parameter (Fisher–Yates).
    let orders: Vec<Vec<usize>> = ranges
        .iter()
        .map(|_| {
            let mut o: Vec<usize> = (0..count).collect();
            for i in (1..count).rev() {
                o.swap(i, rng.below(i + 1));
            }
            o
        })
        .collect();
    (0..count)
        .map(|k| {
            let u: Vec<f64> = orders
                .iter()
                .map(|o| (o[k] as f64 + rng.unit()) / count as f64)
                .collect();
            scale(&u, ranges)
        })
        .collect()
}

/// Joe–Kuo direction numbers for dimensions 2 to 10: degree `s`,
/// coefficients `a` and initial `m` values.
const SOBOL_TABLE: [(u32, u32, &[u32]); SOBOL_MAX_PARAMS - 1] = [
    (1, 0, &[1]),
    (2, 1, &[1, 3]),
    (3, 1, &[1, 3, 1]),
    (3, 2, &[1, 1, 1]),
    (4, 1, &[1, 1, 3, 3]),
    (4, 4, &[1, 3, 5, 13]),
    (5, 2, &[1, 1, 5, 5, 17]),
    (5, 4, &[1, 1, 5, 5, 5]),
    (5, 7, &[1, 1, 7, 11, 19]),
];

const SOBOL_BITS: usize = 32;

/// Direction numbers of one dimension (0-based), scaled to 32 bits.
fn directions(dim: usize) -> [u32; SOBOL_BITS] {
    let mut v = [0u32; SOBOL_BITS];
    if dim == 0 {
        for (k, d) in v.iter_mut().enumerate() {
            *d = 1 << (31 - k);
        }
        return v;
    }
    let (s, a, m) = SOBOL_TABLE[dim - 1];
    let s = s as usize;
    for k in 0..SOBOL_BITS {
        v[k] = if k < s {
            m[k] << (31 - k)
        } else {
            let mut x = v[k - s] ^ (v[k - s] >> s);
            for i in 1..s {
                if (a >> (s - 1 - i)) & 1 == 1 {
                    x ^= v[k - i];
                }
            }
            x
        };
    }
    v
}

/// The first `count` Sobol points (starting at the lower corner), or an
/// error for more than [`SOBOL_MAX_PARAMS`] parameters.
pub fn sobol(ranges: &[[f64; 2]], count: usize) -> Result<Vec<Vec<f64>>, String> {
    if ranges.len() > SOBOL_MAX_PARAMS {
        return Err(format!(
            "Sobol points are available for up to {SOBOL_MAX_PARAMS} parameters"
        ));
    }
    let dirs: Vec<[u32; SOBOL_BITS]> = (0..ranges.len()).map(directions).collect();
    let mut x = vec![0u32; ranges.len()];
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        if i > 0 {
            // Gray-code order: flip the direction of the lowest zero bit of i - 1.
            let c = (!(i - 1)).trailing_zeros() as usize;
            for (xd, d) in x.iter_mut().zip(&dirs) {
                *xd ^= d[c.min(SOBOL_BITS - 1)];
            }
        }
        let u: Vec<f64> = x.iter().map(|&b| b as f64 / 4_294_967_296.0).collect();
        out.push(scale(&u, ranges));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_vary_the_first_parameter_fastest_with_exact_ends() {
        let g = grid(&[
            Axis {
                range: [0.0, 1.0],
                points: 3,
            },
            Axis {
                range: [10.0, 20.0],
                points: 2,
            },
        ]);
        assert_eq!(
            g,
            [
                [0.0, 10.0],
                [0.5, 10.0],
                [1.0, 10.0],
                [0.0, 20.0],
                [0.5, 20.0],
                [1.0, 20.0]
            ]
        );
    }

    /// The first 2^k points of a stratified design put exactly one point in
    /// each of 2^k equal slices of every parameter.
    fn one_per_slice(points: &[Vec<f64>], dims: usize) {
        let n = points.len();
        for d in 0..dims {
            let mut seen = vec![0; n];
            for p in points {
                assert!((0.0..1.0).contains(&p[d]), "{p:?}");
                seen[(p[d] * n as f64) as usize] += 1;
            }
            assert!(seen.iter().all(|&c| c == 1), "dimension {d}: {seen:?}");
        }
    }

    #[test]
    fn sobol_and_latin_hypercube_points_are_stratified() {
        let unit = vec![[0.0, 1.0]; SOBOL_MAX_PARAMS];
        for k in [1, 4, 8] {
            one_per_slice(&sobol(&unit, 1 << k).unwrap(), SOBOL_MAX_PARAMS);
        }
        one_per_slice(&latin_hypercube(&unit[..3], 37, 7), 3);
        // The first two Sobol dimensions also form a (0, m, 2)-net: every
        // 1/2^a by 1/2^b box with a + b = m holds exactly one point.
        let m = 6;
        let pts = sobol(&unit[..2], 1 << m).unwrap();
        for a in 0..=m {
            let b = m - a;
            let mut seen = vec![0; 1 << m];
            for p in &pts {
                let i = (p[0] * (1 << a) as f64) as usize;
                let j = (p[1] * (1 << b) as f64) as usize;
                seen[(j << a) | i] += 1;
            }
            assert!(seen.iter().all(|&c| c == 1), "a = {a}");
        }
        assert!(sobol(&[[0.0, 1.0]; 11], 4).is_err());
    }

    #[test]
    fn random_points_are_seeded_and_in_range() {
        let r = [[2.0, 3.0], [-1.0, 1.0]];
        let a = random(&r, 100, 42);
        assert_eq!(a, random(&r, 100, 42));
        assert_ne!(a, random(&r, 100, 43));
        assert!(a
            .iter()
            .all(|p| (2.0..3.0).contains(&p[0]) && (-1.0..1.0).contains(&p[1])));
        let mean: f64 = a.iter().map(|p| p[0]).sum::<f64>() / 100.0;
        assert!((mean - 2.5).abs() < 0.1, "{mean}");
    }
}
