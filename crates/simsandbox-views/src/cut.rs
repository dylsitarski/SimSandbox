//! Cuts through 2D grids (ADR 0031): the arithmetic of the cut filter.
//!
//! A cut filter has one mode: along x at fixed y values, along y at fixed x
//! values, or free lines between two points. Each cut becomes one line of
//! values; all lines of a filter share one coordinate (the free direction,
//! or the fraction along a free line).

use serde::{Deserialize, Serialize};

/// How values between cell centres are found: an approximation, so it is a
/// method choice (ADR 0020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpolation {
    Nearest,
    #[default]
    Bilinear,
}

/// Which way a filter's cuts run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Across the grid at fixed y values.
    #[default]
    AlongX,
    /// Up the grid at fixed x values.
    AlongY,
    /// Straight lines between two points.
    Free,
}

/// A cut filter's settings, as saved in the model file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutSettings {
    #[serde(default)]
    pub mode: Mode,
    /// Along x or y: the fixed values of the other coordinate, one per cut.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub at: Vec<f64>,
    /// Free: the cuts' end points.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Along x or y: sample the whole field (default) or only `range`; free:
    /// extend each line to the edges of the grid (default) or stop at its
    /// end points.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub whole: bool,
    /// Along x or y with `whole` off: the stretch of the free coordinate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub interpolation: Interpolation,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Where a grid's cells lie: their centres along each axis, in the
/// coordinates' units (or cell numbers, centres at i + 0.5, as on a heat
/// map), and the coordinates' names.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub nx: usize,
    pub ny: usize,
    pub centres: [Vec<f64>; 2],
    pub names: [Option<String>; 2],
}

impl Geometry {
    /// Cell numbers along both axes.
    pub fn cells(nx: usize, ny: usize) -> Self {
        Self {
            nx,
            ny,
            centres: [
                (0..nx).map(|i| i as f64 + 0.5).collect(),
                (0..ny).map(|j| j as f64 + 0.5).collect(),
            ],
            names: [None, None],
        }
    }

    /// Place axis `axis` by a coordinate grid's frame (row by row, `nx`
    /// wide), if its values along that axis are finite and strictly
    /// monotonic.
    pub fn with_coordinate(mut self, axis: usize, name: &str, frame: &[f64]) -> Self {
        let cells = if axis == 0 { self.nx } else { self.ny };
        let values: Vec<f64> = (0..cells)
            .map(|k| {
                let at = if axis == 0 { k } else { k * self.nx };
                frame.get(at).copied().unwrap_or(f64::NAN)
            })
            .collect();
        if monotonic(&values) {
            self.centres[axis] = values;
            self.names[axis] = Some(name.into());
        }
        self
    }

    /// The axis's name for labels: its coordinate, or `column`/`row`.
    pub fn label(&self, axis: usize) -> String {
        self.names[axis]
            .clone()
            .unwrap_or_else(|| if axis == 0 { "column" } else { "row" }.into())
    }

    /// `[low, high]` of the cell centres along an axis.
    fn extent(&self, axis: usize) -> [f64; 2] {
        let c = &self.centres[axis];
        let (a, b) = (c[0], c[c.len() - 1]);
        [a.min(b), a.max(b)]
    }
}

fn monotonic(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
        && (v.windows(2).all(|w| w[1] > w[0]) || v.windows(2).all(|w| w[1] < w[0]))
}

/// The fractional cell index of `p` among monotonic `centres`, or `None`
/// outside them.
pub fn index_of(centres: &[f64], p: f64) -> Option<f64> {
    let n = centres.len();
    if n < 2 || !p.is_finite() {
        return (n == 1 && centres[0] == p).then_some(0.0);
    }
    let sign = if centres[1] > centres[0] { 1.0 } else { -1.0 };
    let q = sign * p;
    if q < sign * centres[0] || q > sign * centres[n - 1] {
        return None;
    }
    let k = centres.partition_point(|&c| sign * c <= q).clamp(1, n - 1) - 1;
    Some(k as f64 + (p - centres[k]) / (centres[k + 1] - centres[k]))
}

/// The value at fractional cell `(u, v)` of an `nx` × `ny` frame.
pub fn sample(frame: &[f64], nx: usize, ny: usize, u: f64, v: f64, how: Interpolation) -> f64 {
    let at = |i: usize, j: usize| frame.get(j * nx + i).copied().unwrap_or(f64::NAN);
    match how {
        Interpolation::Nearest => at(
            (u.round() as usize).min(nx - 1),
            (v.round() as usize).min(ny - 1),
        ),
        Interpolation::Bilinear => {
            // One cell along an axis: nothing to interpolate there.
            let split = |x: f64, n: usize| {
                if n < 2 {
                    return (0, 0.0);
                }
                let i = (x.floor() as usize).min(n - 2);
                (i, x - i as f64)
            };
            let (i, a) = split(u, nx);
            let (j, b) = split(v, ny);
            let i1 = (i + 1).min(nx - 1);
            let j1 = (j + 1).min(ny - 1);
            let row = |j| at(i, j) * (1.0 - a) + at(i1, j) * a;
            row(j) * (1.0 - b) + row(j1) * b
        }
    }
}

/// Clip the infinite line through `a` and `b` to a box; `None` if it misses
/// it or `a == b`.
fn clip_to(a: [f64; 2], b: [f64; 2], lo: [f64; 2], hi: [f64; 2]) -> Option<[[f64; 2]; 2]> {
    let d = [b[0] - a[0], b[1] - a[1]];
    if d == [0.0, 0.0] {
        return None;
    }
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..2 {
        if d[k] == 0.0 {
            if a[k] < lo[k] || a[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (s, e) = ((lo[k] - a[k]) / d[k], (hi[k] - a[k]) / d[k]);
        t0 = t0.max(s.min(e));
        t1 = t1.min(s.max(e));
    }
    if t0 > t1 {
        return None;
    }
    Some([
        [a[0] + t0 * d[0], a[1] + t0 * d[1]],
        [a[0] + t1 * d[0], a[1] + t1 * d[1]],
    ])
}

/// Evenly spaced values from `a` to `b`, ends exact.
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|k| {
            let f = if n > 1 {
                k as f64 / (n - 1) as f64
            } else {
                0.0
            };
            a * (1.0 - f) + b * f
        })
        .collect()
}

impl CutSettings {
    /// Number of cuts.
    pub fn count(&self) -> usize {
        match self.mode {
            Mode::AlongX | Mode::AlongY => self.at.len(),
            Mode::Free => self.lines.len(),
        }
    }

    /// Points per cut for an `nx` × `ny` grid: one per column (along x),
    /// per row (along y), or the larger of the two (free).
    pub fn points(&self, nx: usize, ny: usize) -> usize {
        match self.mode {
            Mode::AlongX => nx,
            Mode::AlongY => ny,
            Mode::Free => nx.max(ny),
        }
    }

    /// Why these settings cannot be used, if they cannot.
    pub fn check(&self) -> Result<(), String> {
        if self.count() == 0 {
            return Err("a cut filter needs at least one cut".into());
        }
        let finite = self
            .at
            .iter()
            .chain(self.lines.iter().flatten().flatten())
            .chain(self.range.iter().flatten())
            .all(|v| v.is_finite());
        if !finite {
            return Err("cut values must be finite".into());
        }
        if let Some([a, b]) = self.range {
            if a >= b {
                return Err(format!("[{a}, {b}] is not an increasing range"));
            }
        }
        Ok(())
    }

    /// The coordinate every cut runs along: the free coordinate's values
    /// (the whole field's centres, or evenly over `range`), or the
    /// fraction 0 to 1 along a free line.
    pub fn coordinate(&self, g: &Geometry) -> Vec<f64> {
        let axis = match self.mode {
            Mode::AlongX => 0,
            Mode::AlongY => 1,
            Mode::Free => return linspace(0.0, 1.0, g.nx.max(g.ny)),
        };
        match (self.whole, self.range) {
            (false, Some([a, b])) => linspace(a, b, self.points(g.nx, g.ny)),
            _ => g.centres[axis].clone(),
        }
    }

    /// The coordinate's name for labels.
    pub fn coordinate_label(&self, g: &Geometry) -> String {
        match self.mode {
            Mode::AlongX => g.label(0),
            Mode::AlongY => g.label(1),
            Mode::Free => "fraction along the cut".into(),
        }
    }

    /// A label per cut, e.g. `spring.c = 0.05`.
    pub fn labels(&self, g: &Geometry) -> Vec<String> {
        match self.mode {
            Mode::AlongX => self
                .at
                .iter()
                .map(|v| format!("{} = {}", g.label(1), short(*v)))
                .collect(),
            Mode::AlongY => self
                .at
                .iter()
                .map(|v| format!("{} = {}", g.label(0), short(*v)))
                .collect(),
            Mode::Free => self
                .lines
                .iter()
                .map(|[a, b]| {
                    format!(
                        "({}, {}) to ({}, {})",
                        short(a[0]),
                        short(a[1]),
                        short(b[0]),
                        short(b[1])
                    )
                })
                .collect(),
        }
    }

    /// The values along every cut through `frame` (`nx` × `ny`, row by
    /// row); NaN outside the grid.
    pub fn cut(&self, g: &Geometry, frame: &[f64]) -> Vec<Vec<f64>> {
        let (nx, ny) = (g.nx, g.ny);
        let [cx, cy] = &g.centres;
        let how = self.interpolation;
        let at = |p: [f64; 2]| match (index_of(cx, p[0]), index_of(cy, p[1])) {
            (Some(u), Some(v)) => sample(frame, nx, ny, u, v, how),
            _ => f64::NAN,
        };
        let along = self.coordinate(g);
        match self.mode {
            Mode::AlongX | Mode::AlongY => {
                let x_first = self.mode == Mode::AlongX;
                self.at
                    .iter()
                    .map(|&fixed| {
                        if self.whole || self.range.is_none() {
                            // Every column (or row) exactly; only the fixed
                            // direction is interpolated.
                            let fixed_index = if x_first {
                                index_of(cy, fixed)
                            } else {
                                index_of(cx, fixed)
                            };
                            let n = if x_first { nx } else { ny };
                            (0..n)
                                .map(|k| match fixed_index {
                                    Some(f) if x_first => sample(frame, nx, ny, k as f64, f, how),
                                    Some(f) => sample(frame, nx, ny, f, k as f64, how),
                                    None => f64::NAN,
                                })
                                .collect()
                        } else {
                            along
                                .iter()
                                .map(|&s| at(if x_first { [s, fixed] } else { [fixed, s] }))
                                .collect()
                        }
                    })
                    .collect()
            }
            Mode::Free => {
                let lo = [g.extent(0)[0], g.extent(1)[0]];
                let hi = [g.extent(0)[1], g.extent(1)[1]];
                self.lines
                    .iter()
                    .map(|&[a, b]| {
                        let ends = if self.whole {
                            clip_to(a, b, lo, hi)
                        } else {
                            Some([a, b])
                        };
                        match ends {
                            Some([p, q]) => along
                                .iter()
                                .map(|&f| at([p[0] + f * (q[0] - p[0]), p[1] + f * (q[1] - p[1])]))
                                .collect(),
                            None => vec![f64::NAN; along.len()],
                        }
                    })
                    .collect()
            }
        }
    }
}

/// A number short enough for a label.
pub fn short(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// z = 2x + 3y on unevenly spaced coordinates.
    fn linear() -> (Geometry, Vec<f64>) {
        let (cx, cy) = (vec![0.0, 1.0, 3.0, 3.5], vec![10.0, 20.0, 25.0]);
        let frame = cy
            .iter()
            .flat_map(|y| cx.iter().map(move |x| 2.0 * x + 3.0 * y))
            .collect();
        let g = Geometry {
            nx: 4,
            ny: 3,
            centres: [cx, cy],
            names: [Some("p.a".into()), Some("p.b".into())],
        };
        (g, frame)
    }

    fn settings(mode: Mode) -> CutSettings {
        CutSettings {
            mode,
            at: Vec::new(),
            lines: Vec::new(),
            whole: true,
            range: None,
            interpolation: Interpolation::Bilinear,
        }
    }

    #[test]
    fn positions_map_to_fractional_cells() {
        assert_eq!(index_of(&[0.0, 1.0, 3.0], 2.0), Some(1.5));
        assert_eq!(index_of(&[3.0, 1.0, 0.0], 2.0), Some(0.5));
        assert_eq!(index_of(&[0.0, 1.0, 3.0], 3.5), None);
        assert_eq!(index_of(&[0.0, 1.0], f64::NAN), None);
    }

    #[test]
    fn bilinear_cuts_reproduce_a_linear_field_exactly() {
        let (g, frame) = linear();
        let z = |x: f64, y: f64| 2.0 * x + 3.0 * y;
        // Along x at fixed y, between rows: every column, exactly.
        let mut s = settings(Mode::AlongX);
        s.at = vec![12.5, 25.0];
        let lines = s.cut(&g, &frame);
        assert_eq!(s.coordinate(&g), g.centres[0]);
        for (line, y) in lines.iter().zip([12.5, 25.0]) {
            for (v, x) in line.iter().zip(&g.centres[0]) {
                assert!((v - z(*x, y)).abs() < 1e-12);
            }
        }
        // Along y over part of the range.
        let mut s = settings(Mode::AlongY);
        s.at = vec![2.0];
        s.whole = false;
        s.range = Some([12.0, 24.0]);
        let ys = s.coordinate(&g);
        assert_eq!((ys[0], ys[2]), (12.0, 24.0));
        for (v, y) in s.cut(&g, &frame)[0].iter().zip(&ys) {
            assert!((v - z(2.0, *y)).abs() < 1e-12);
        }
        // A free line, extended to the grid's edges.
        let mut s = settings(Mode::Free);
        s.lines = vec![[[1.0, 15.0], [2.0, 20.0]]];
        let line = &s.cut(&g, &frame)[0];
        let f = s.coordinate(&g);
        assert_eq!(f.len(), 4);
        // The line x = 1 + (y - 15)/5 meets y = 10 at x = 0 and x = 3.5 at
        // y = 27.5, beyond the top row (25): it ends at (3, 25).
        let (x0, y0, x1, y1) = (0.0, 10.0, 3.0, 25.0);
        for (v, f) in line.iter().zip(&f) {
            let (x, y) = (x0 + f * (x1 - x0), y0 + f * (y1 - y0));
            assert!((v - z(x, y)).abs() < 1e-12, "{v} vs {}", z(x, y));
        }
        // Not extended, it runs between its end points only.
        s.whole = false;
        let short = &s.cut(&g, &frame)[0];
        assert!((short[0] - z(1.0, 15.0)).abs() < 1e-12);
        assert!((short[3] - z(2.0, 20.0)).abs() < 1e-12);
    }

    #[test]
    fn nearest_takes_the_closest_cell_and_outside_is_blank() {
        let (g, frame) = linear();
        let mut s = settings(Mode::AlongX);
        s.interpolation = Interpolation::Nearest;
        s.at = vec![12.0, 40.0];
        let lines = s.cut(&g, &frame);
        assert_eq!(lines[0], frame[0..4].to_vec());
        assert!(lines[1].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn labels_name_each_cut_and_settings_are_checked() {
        let (g, _) = linear();
        let mut s = settings(Mode::AlongX);
        s.at = vec![0.05];
        assert_eq!(s.labels(&g), ["p.b = 0.05"]);
        assert_eq!(s.coordinate_label(&g), "p.a");
        assert!(s.check().is_ok());
        s.at.clear();
        assert!(s.check().is_err());
        let json = serde_json::json!({"mode": "along_y", "at": [1.0], "whole": false, "range": [0.0, 2.0]});
        let parsed: CutSettings = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), {
            let mut j = json;
            j["interpolation"] = "bilinear".into();
            j
        });
    }
}
