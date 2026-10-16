//! Colour encodings shared by views (ADR 0015): a colour slot bound to `t`
//! fades the trail from old to new; bound to another signal it colours by
//! value; unbound it is one colour. Also the trail-length window.

use egui::Color32;

/// The colour of a line with no colour encoding.
pub const LINE: Color32 = Color32::from_rgb(90, 170, 255);
/// The marker on the newest point.
pub const HEAD: Color32 = Color32::from_rgb(255, 200, 80);

/// How a line is coloured along its length.
pub enum ColourKey {
    Constant,
    /// Opacity grows with these times: old is dim, new is bright.
    Time(Vec<f64>),
    /// Colour follows these values on a sequential scale.
    Value(Vec<f64>),
}

/// A run of samples `start..=end` drawn in one colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub colour: Color32,
}

/// Split samples `first..n` into at most `chunks` coloured segments that
/// share their end points, so the line stays continuous.
pub fn segments(first: usize, n: usize, key: &ColourKey, chunks: usize) -> Vec<Segment> {
    if n < first + 2 {
        return Vec::new();
    }
    let range = |v: &[f64]| {
        let (lo, hi) = v[first..n]
            .iter()
            .filter(|x| x.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                (a.min(x), b.max(x))
            });
        (lo, (hi - lo).max(1e-300))
    };
    let chunk = (n - first).div_ceil(chunks.max(1)).max(1);
    let mut out = Vec::new();
    let mut start = first;
    while start + 1 < n {
        let end = (start + chunk).min(n - 1);
        let mid = (start + end) / 2;
        let colour = match key {
            ColourKey::Constant => LINE,
            ColourKey::Time(t) => {
                let (lo, span) = range(t);
                let age = ((t[mid] - lo) / span) as f32;
                LINE.gamma_multiply(0.15 + 0.85 * age.clamp(0.0, 1.0))
            }
            ColourKey::Value(v) => {
                let (lo, span) = range(v);
                sequential(((v[mid] - lo) / span) as f32)
            }
        };
        out.push(Segment { start, end, colour });
        start = end;
    }
    out
}

/// A colour-blind-safe sequential scale (viridis stops), `x` in [0, 1].
pub fn sequential(x: f32) -> Color32 {
    const STOPS: [[u8; 3]; 5] = [
        [68, 1, 84],
        [59, 82, 139],
        [33, 145, 140],
        [94, 201, 98],
        [253, 231, 37],
    ];
    let x = if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let f = x * (STOPS.len() - 1) as f32;
    let i = (f.floor() as usize).min(STOPS.len() - 2);
    let w = f - i as f32;
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * w).round() as u8;
    let [a, b] = [STOPS[i], STOPS[i + 1]];
    Color32::from_rgb(mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]))
}

/// First sample to draw when only the last `length` seconds of trail are
/// shown; a length of 0 (or less) shows the whole run.
pub fn trail_start(times: &[f64], length: f64) -> usize {
    match times.last() {
        Some(&last) if length > 0.0 => times.partition_point(|&t| t < last - length),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_cover_the_samples_continuously() {
        let t: Vec<f64> = (0..100).map(|k| k as f64).collect();
        let segs = segments(10, 100, &ColourKey::Time(t.clone()), 8);
        assert_eq!(segs.first().unwrap().start, 10);
        assert_eq!(segs.last().unwrap().end, 99);
        for w in segs.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        // Fading: newer segments are more opaque.
        assert!(segs.first().unwrap().colour.a() < segs.last().unwrap().colour.a());
        assert_eq!(segments(0, 1, &ColourKey::Constant, 8), Vec::new());
        assert_eq!(segments(0, 5, &ColourKey::Constant, 8)[0].colour, LINE);
    }

    #[test]
    fn value_colours_span_the_scale() {
        let v: Vec<f64> = (0..50).map(|k| k as f64).collect();
        let segs = segments(0, 50, &ColourKey::Value(v), 50);
        assert_ne!(segs[0].colour, segs.last().unwrap().colour);
        assert_eq!(sequential(0.0), Color32::from_rgb(68, 1, 84));
        assert_eq!(sequential(1.0), Color32::from_rgb(253, 231, 37));
        assert_eq!(sequential(f32::NAN), sequential(0.0));
    }

    #[test]
    fn trail_length_selects_the_recent_samples() {
        let t: Vec<f64> = (0..=100).map(|k| k as f64 * 0.1).collect();
        assert_eq!(trail_start(&t, 0.0), 0);
        assert_eq!(trail_start(&t, 100.0), 0);
        let s = trail_start(&t, 2.0);
        assert!((t[s] - 8.0).abs() < 1e-9, "{}", t[s]);
        assert_eq!(trail_start(&[], 2.0), 0);
    }
}
