//! Polylines thinned to the pixels they are drawn on
//! (`docs/design/performance.md`): a plot of 100 000 samples on a plot a
//! thousand pixels wide draws about a hundred samples in every pixel
//! column, and the screen shows only how far they reach.

use egui_plot::PlotPoint;

/// Columns a pixel is split into. Lines are drawn smoothed: a column's
/// few kept points cover a pixel a little less evenly than its hundred
/// did, and four columns to a pixel keep the difference below what shows
/// (`docs/design/performance.md`).
pub const COLUMNS_PER_PIXEL: f64 = 4.0;

/// Thins a run of finite points, given in order, to the pixel columns they
/// fall in. Of consecutive points in one column it keeps, in their order,
/// the first and the last (so the lines between columns are as before),
/// the lowest and the highest (so peaks stay), and the leftmost and the
/// rightmost (so the run spans as far as before): at most six points per
/// column. Over a monotone horizontal axis (time) that is the first, last,
/// lowest and highest of each column.
pub struct Columns {
    /// Column width in the horizontal coordinate; keeps every point when
    /// not positive and finite.
    width: f64,
    group: Option<Group>,
    /// Points pushed so far.
    count: usize,
    out: Vec<PlotPoint>,
}

/// The points of one visit to a column, by number.
#[derive(Clone, Copy)]
struct Group {
    column: i64,
    first: (usize, [f64; 2]),
    last: (usize, [f64; 2]),
    low: (usize, [f64; 2]),
    high: (usize, [f64; 2]),
    left: (usize, [f64; 2]),
    right: (usize, [f64; 2]),
}

impl Group {
    fn new(column: i64, p: (usize, [f64; 2])) -> Self {
        Self {
            column,
            first: p,
            last: p,
            low: p,
            high: p,
            left: p,
            right: p,
        }
    }

    fn add(&mut self, p: (usize, [f64; 2])) {
        self.last = p;
        // Ties keep the earlier point.
        if p.1[1] < self.low.1[1] {
            self.low = p;
        }
        if p.1[1] > self.high.1[1] {
            self.high = p;
        }
        if p.1[0] < self.left.1[0] {
            self.left = p;
        }
        if p.1[0] > self.right.1[0] {
            self.right = p;
        }
    }
}

impl Columns {
    /// Thin to columns `width` wide.
    pub fn new(width: f64) -> Self {
        Self {
            width,
            group: None,
            count: 0,
            out: Vec::new(),
        }
    }

    pub fn push(&mut self, p: [f64; 2]) {
        let k = self.count;
        self.count += 1;
        if !(self.width > 0.0 && self.width.is_finite()) {
            self.out.push(PlotPoint::new(p[0], p[1]));
            return;
        }
        let column = floor(p[0] / self.width);
        match &mut self.group {
            Some(g) if g.column == column => g.add((k, p)),
            _ => {
                self.flush();
                self.group = Some(Group::new(column, (k, p)));
            }
        }
    }

    fn flush(&mut self) {
        let Some(g) = self.group.take() else {
            return;
        };
        let mut kept = [g.first, g.low, g.high, g.left, g.right, g.last];
        kept.sort_by_key(|(k, _)| *k);
        let mut previous = None;
        for (k, p) in kept {
            if previous != Some(k) {
                self.out.push(PlotPoint::new(p[0], p[1]));
                previous = Some(k);
            }
        }
    }

    /// The points kept.
    pub fn finish(mut self) -> Vec<PlotPoint> {
        self.flush();
        self.out
    }
}

/// `q` rounded down, saturating, without a call to the C library's
/// `floor` (most of the time thinning took).
fn floor(q: f64) -> i64 {
    let c = q as i64;
    if (c as f64) > q {
        c - 1
    } else {
        c
    }
}

/// The width of a pixel column in a plot's horizontal coordinate: of the
/// plot's bounds (as last drawn) or the data's span, whichever is
/// narrower, over the plot's width in pixels. Never wider than a pixel
/// while the plot fits the data; zoomed in, the bounds are narrower.
pub fn pixel_width(bounds: f64, data: f64, pixels: f64) -> f64 {
    let span = match (bounds > 0.0 && bounds.is_finite(), data > 0.0) {
        (true, true) => bounds.min(data),
        (true, false) => bounds,
        (false, _) => data,
    };
    if pixels >= 1.0 {
        span / pixels
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thinned(points: &[[f64; 2]], width: f64) -> Vec<[f64; 2]> {
        let mut c = Columns::new(width);
        for &p in points {
            c.push(p);
        }
        c.finish().iter().map(|p| [p.x, p.y]).collect()
    }

    /// Every pixel column's lowest and highest value, first and last point.
    fn extremes(points: &[[f64; 2]], width: f64) -> Vec<(f64, [f64; 4])> {
        let mut out: Vec<(f64, [f64; 4])> = Vec::new();
        for p in points {
            let c = (p[0] / width).floor();
            match out.last_mut() {
                Some((col, e)) if *col == c => {
                    e[0] = e[0].min(p[1]);
                    e[1] = e[1].max(p[1]);
                    e[3] = p[1];
                }
                _ => out.push((c, [p[1], p[1], p[1], p[1]])),
            }
        }
        out
    }

    #[test]
    fn every_columns_extremes_are_kept() {
        // A noisy signal over time: 100 000 samples on 1 000 columns,
        // with a single-sample spike.
        let mut seed = 12345u64;
        let mut noise = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let mut points: Vec<[f64; 2]> = (0..100_000)
            .map(|k| {
                let t = k as f64 * 1e-3;
                [t, (7.0 * t).sin() + 0.3 * noise()]
            })
            .collect();
        points[54_321][1] = 40.0;
        let width = 100.0 / 1000.0;
        let kept = thinned(&points, width);
        assert!(kept.len() <= 4 * 1001, "{}", kept.len());
        assert!(kept.len() > 1000);
        // The same extremes, first and last values in every column, in
        // order: the picture at this resolution is the same.
        assert_eq!(extremes(&kept, width), extremes(&points, width));
        assert!(kept.contains(&[54.321, 40.0]));
        assert_eq!(kept.first(), points.first());
        assert_eq!(kept.last(), points.last());
        // Kept points are some of the points, in their order.
        let mut at = 0;
        for p in &kept {
            at += points[at..].iter().position(|q| q == p).expect("in order");
        }
    }

    #[test]
    fn a_path_back_and_forth_keeps_each_visit_and_its_reach() {
        // A circle drawn many times over: each visit to a column keeps its
        // lowest, highest, leftmost and rightmost point.
        let points: Vec<[f64; 2]> = (0..40_000)
            .map(|k| {
                let a = k as f64 * 5e-4;
                [a.cos(), a.sin()]
            })
            .collect();
        let width = 2.0 / 200.0;
        let kept = thinned(&points, width);
        assert!(kept.len() < points.len() / 2, "{}", kept.len());
        let reach = |v: &[[f64; 2]], i: usize| {
            v.iter()
                .map(|p| p[i])
                .fold([f64::INFINITY, f64::NEG_INFINITY], |r, x| {
                    [r[0].min(x), r[1].max(x)]
                })
        };
        assert_eq!(reach(&kept, 0), reach(&points, 0));
        assert_eq!(reach(&kept, 1), reach(&points, 1));
        assert_eq!(extremes(&kept, width), extremes(&points, width));
    }

    #[test]
    fn without_a_width_every_point_is_kept() {
        let points = [[0.0, 1.0], [0.0, 2.0], [0.0, 3.0], [0.0, 1.5]];
        assert_eq!(thinned(&points, 0.0), points);
        assert_eq!(thinned(&points, f64::NAN), points);
        // Few points to a column: all kept.
        let few = [[0.0, 1.0], [0.1, 2.0], [0.2, 0.0], [1.5, 3.0]];
        assert_eq!(thinned(&few, 1.0), few);
    }

    #[test]
    fn columns_are_numbered_by_rounding_down() {
        for q in [
            -2.5,
            -2.0,
            -0.5,
            -0.0,
            0.0,
            0.5,
            1.0,
            1e15 + 0.5,
            -1e15 - 0.5,
        ] {
            assert_eq!(floor(q), q.floor() as i64, "{q}");
        }
    }

    #[test]
    fn a_pixel_is_never_wider_than_the_plot_shows() {
        assert_eq!(pixel_width(10.0, 5.0, 100.0), 0.05);
        assert_eq!(pixel_width(2.0, 5.0, 100.0), 0.02);
        assert_eq!(pixel_width(f64::NAN, 5.0, 100.0), 0.05);
        assert_eq!(pixel_width(2.0, 0.0, 100.0), 0.02);
        assert_eq!(pixel_width(2.0, 5.0, 0.0), 0.0);
    }
}
