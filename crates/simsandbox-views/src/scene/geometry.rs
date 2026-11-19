//! Poses and primitives as parametric meshes (ADR 0093 point 4, ADR 0101
//! point 1): from one instance's attributes, its triangles and lines at a
//! given scale, its bounding box and an exact containment test.

use egui::Color32;

/// A point in the plane.
pub type P = [f64; 2];

/// Where a frame is in its parent (ADR 0093 point 4): its origin and its
/// angle (radians, counter-clockwise). A point `p` drawn in the frame lies
/// at `origin + R(angle)·p` in the parent: turning before moving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

impl Pose {
    /// The parent frame itself.
    pub const IDENTITY: Pose = Pose {
        x: 0.0,
        y: 0.0,
        angle: 0.0,
    };

    /// Where a point given in this frame lies in its parent.
    pub fn apply(&self, p: P) -> P {
        let (s, c) = self.angle.sin_cos();
        [self.x + c * p[0] - s * p[1], self.y + s * p[0] + c * p[1]]
    }

    /// The pose in this frame's parent of a frame whose pose in this frame
    /// is `inner`: how frames chain.
    pub fn of(&self, inner: Pose) -> Pose {
        let [x, y] = self.apply([inner.x, inner.y]);
        Pose {
            x,
            y,
            angle: self.angle + inner.angle,
        }
    }

    /// The parent's pose in this frame: what drawing in this frame's
    /// coordinates applies (`view_from`).
    pub fn inverse(&self) -> Pose {
        let (s, c) = self.angle.sin_cos();
        Pose {
            x: -(c * self.x + s * self.y),
            y: s * self.x - c * self.y,
            angle: -self.angle,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.angle.is_finite()
    }
}

/// One instance of a primitive, its attributes resolved, in some frame's
/// coordinates. Lengths are in the scene's unit; `size` and `width` in
/// screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Instance {
    Point {
        at: P,
        size: f64,
        colour: Color32,
    },
    Segment {
        from: P,
        to: P,
        width: f64,
        colour: Color32,
    },
    Box {
        centre: P,
        angle: f64,
        width: f64,
        height: f64,
        colour: Color32,
    },
    Disc {
        centre: P,
        radius: f64,
        /// The mark showing rotation, if bound.
        angle: Option<f64>,
        colour: Color32,
    },
}

/// What a scene draws, in the coordinates it is drawn in: filled
/// triangles in the scene's unit, lines and dots of a width in screen
/// points (ADR 0101 point 2), in drawing order.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Triangles, three points each.
    Triangles {
        points: Vec<P>,
        colour: Color32,
    },
    Line {
        points: Vec<P>,
        width: f32,
        colour: Color32,
    },
    Dot {
        at: P,
        radius: f32,
        colour: Color32,
    },
}

/// The most a drawn curve strays from the true one, in screen points.
const TOLERANCE: f64 = 0.2;

/// How many straight pieces draw a circle of `radius` screen points so
/// that none strays more than [`TOLERANCE`] from it: more as it covers
/// more pixels, between 8 and 1024.
pub fn arc_steps(radius: f64) -> usize {
    if radius.is_nan() || radius <= TOLERANCE {
        return 8;
    }
    let n = std::f64::consts::PI / (1.0 - TOLERANCE / radius).acos();
    (n.ceil() as usize).clamp(8, 1024)
}

/// Black or white, whichever stands out on `c`.
fn contrast(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 140.0 {
        Color32::from_black_alpha(200)
    } else {
        Color32::from_white_alpha(220)
    }
}

impl Instance {
    /// The same shape moved by `pose`: drawn in a frame, placed in its
    /// parent. Frames move and turn; sizes are unchanged.
    pub fn moved(&self, pose: Pose) -> Instance {
        match *self {
            Instance::Point { at, size, colour } => Instance::Point {
                at: pose.apply(at),
                size,
                colour,
            },
            Instance::Segment {
                from,
                to,
                width,
                colour,
            } => Instance::Segment {
                from: pose.apply(from),
                to: pose.apply(to),
                width,
                colour,
            },
            Instance::Box {
                centre,
                angle,
                width,
                height,
                colour,
            } => Instance::Box {
                centre: pose.apply(centre),
                angle: angle + pose.angle,
                width,
                height,
                colour,
            },
            Instance::Disc {
                centre,
                radius,
                angle,
                colour,
            } => Instance::Disc {
                centre: pose.apply(centre),
                radius,
                angle: angle.map(|a| a + pose.angle),
                colour,
            },
        }
    }

    /// A box's corners, counter-clockwise from the lower left in its axes.
    fn corners(centre: P, angle: f64, width: f64, height: f64) -> [P; 4] {
        let (w, h) = (0.5 * width.abs(), 0.5 * height.abs());
        let pose = Pose {
            x: centre[0],
            y: centre[1],
            angle,
        };
        [[-w, -h], [w, -h], [w, h], [-w, h]].map(|p| pose.apply(p))
    }

    /// The smallest box holding it, `[low, high]`: screen-sized parts
    /// (points, line widths) count as their centre.
    pub fn bounds(&self) -> [P; 2] {
        let of = |ps: &[P]| {
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for p in ps {
                for a in 0..2 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            [lo, hi]
        };
        match *self {
            Instance::Point { at, .. } => [at, at],
            Instance::Segment { from, to, .. } => of(&[from, to]),
            Instance::Box {
                centre,
                angle,
                width,
                height,
                ..
            } => of(&Self::corners(centre, angle, width, height)),
            Instance::Disc { centre, radius, .. } => {
                let r = radius.abs();
                [
                    [centre[0] - r, centre[1] - r],
                    [centre[0] + r, centre[1] + r],
                ]
            }
        }
    }

    /// Whether `p` lies inside, edges included (ADR 0101 point 1): a disc
    /// by distance, a box in its own axes, a point or a segment within
    /// half its size or width of it. `scale` is screen points per unit,
    /// for the parts sized on screen.
    pub fn contains(&self, p: P, scale: f64) -> bool {
        let dist = |a: P, b: P| (a[0] - b[0]).hypot(a[1] - b[1]);
        match *self {
            Instance::Point { at, size, .. } => dist(p, at) * scale <= 0.5 * size.abs(),
            Instance::Segment {
                from, to, width, ..
            } => {
                let d = [to[0] - from[0], to[1] - from[1]];
                let len2 = d[0] * d[0] + d[1] * d[1];
                let u = if len2 > 0.0 {
                    (((p[0] - from[0]) * d[0] + (p[1] - from[1]) * d[1]) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest = [from[0] + u * d[0], from[1] + u * d[1]];
                dist(p, nearest) * scale <= 0.5 * width.abs()
            }
            Instance::Box {
                centre,
                angle,
                width,
                height,
                ..
            } => {
                let local = Pose {
                    x: centre[0],
                    y: centre[1],
                    angle,
                }
                .inverse()
                .apply(p);
                local[0].abs() <= 0.5 * width.abs() && local[1].abs() <= 0.5 * height.abs()
            }
            Instance::Disc { centre, radius, .. } => dist(p, centre) <= radius.abs(),
        }
    }

    /// Its triangles and lines at `scale` screen points per unit, added
    /// to `out` (ADR 0101 point 1): a disc in as many pieces as the
    /// pixels it covers need.
    pub fn mesh(&self, scale: f64, out: &mut Vec<Item>) {
        match *self {
            Instance::Point { at, size, colour } => out.push(Item::Dot {
                at,
                radius: (0.5 * size.abs()) as f32,
                colour,
            }),
            Instance::Segment {
                from,
                to,
                width,
                colour,
            } => out.push(Item::Line {
                points: vec![from, to],
                width: width.abs() as f32,
                colour,
            }),
            Instance::Box {
                centre,
                angle,
                width,
                height,
                colour,
            } => {
                let [a, b, c, d] = Self::corners(centre, angle, width, height);
                out.push(Item::Triangles {
                    points: vec![a, b, c, a, c, d],
                    colour,
                });
            }
            Instance::Disc {
                centre,
                radius,
                angle,
                colour,
            } => {
                let r = radius.abs();
                let n = arc_steps(r * scale.abs());
                let rim = |k: usize| {
                    let a = std::f64::consts::TAU * k as f64 / n as f64;
                    [centre[0] + r * a.cos(), centre[1] + r * a.sin()]
                };
                let mut points = Vec::with_capacity(3 * n);
                for k in 0..n {
                    points.extend([centre, rim(k), rim(k + 1)]);
                }
                out.push(Item::Triangles { points, colour });
                if let Some(a) = angle {
                    out.push(Item::Line {
                        points: vec![centre, [centre[0] + r * a.cos(), centre[1] + r * a.sin()]],
                        width: 1.5,
                        colour: contrast(colour),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    const C: Color32 = Color32::RED;

    fn close(a: P, b: P) -> bool {
        (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12
    }

    #[test]
    fn poses_turn_before_moving_chain_and_invert() {
        let p = Pose {
            x: 1.0,
            y: 2.0,
            angle: FRAC_PI_2,
        };
        // (1, 0) turned a quarter left is (0, 1), then moved by (1, 2).
        assert!(close(p.apply([1.0, 0.0]), [1.0, 3.0]));
        let inner = Pose {
            x: 0.5,
            y: 0.0,
            angle: 0.3,
        };
        let q = [0.2, -0.7];
        // Chained: the same point through both frames.
        assert!(close(p.of(inner).apply(q), p.apply(inner.apply(q))));
        assert!(close(p.inverse().apply(p.apply(q)), q));
        assert!(close(p.of(p.inverse()).apply(q), q));
        assert_eq!(Pose::IDENTITY.apply(q), q);
    }

    #[test]
    fn a_box_is_two_triangles_at_its_corners_and_holds_its_edges() {
        let b = Instance::Box {
            centre: [1.0, 1.0],
            angle: FRAC_PI_2,
            width: 2.0,
            height: 1.0,
            colour: C,
        };
        let mut out = Vec::new();
        b.mesh(100.0, &mut out);
        let Item::Triangles { points, colour } = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!((points.len(), *colour), (6, C));
        // Turned a quarter: 1 wide across, 2 high.
        let [lo, hi] = b.bounds();
        assert!(
            close(lo, [0.5, 0.0]) && close(hi, [1.5, 2.0]),
            "{lo:?} {hi:?}"
        );
        for p in points {
            assert!(
                close(*p, [0.5, 0.0])
                    || close(*p, [1.5, 0.0])
                    || close(*p, [1.5, 2.0])
                    || close(*p, [0.5, 2.0])
            );
        }
        // Containment in its own axes: well inside and outside when
        // turned, and exactly at the edges when not.
        assert!(b.contains([1.0, 1.0], 1.0));
        assert!(b.contains([1.4, 1.9], 1.0));
        assert!(!b.contains([1.6, 1.0], 1.0));
        assert!(!b.contains([1.9, 1.4], 1.0));
        let flat = Instance::Box {
            centre: [1.0, 1.0],
            angle: 0.0,
            width: 2.0,
            height: 1.0,
            colour: C,
        };
        let e = 1e-9;
        assert!(flat.contains([2.0, 1.5], 1.0));
        assert!(flat.contains([0.0, 0.5], 1.0));
        assert!(!flat.contains([2.0 + e, 1.0], 1.0));
        assert!(!flat.contains([1.0, 0.5 - e], 1.0));
        assert!(flat.contains([1.9, 1.0], 1.0));
        assert!(!flat.contains([1.0, 1.9], 1.0));
    }

    #[test]
    fn a_disc_is_split_by_the_pixels_it_covers_and_holds_its_rim() {
        let d = Instance::Disc {
            centre: [0.0, 1.0],
            radius: 0.5,
            angle: Some(PI),
            colour: C,
        };
        let pieces = |scale: f64| {
            let mut out = Vec::new();
            d.mesh(scale, &mut out);
            let Item::Triangles { points, .. } = &out[0] else {
                panic!()
            };
            // Every rim point on the circle; centres at the centre.
            for t in points.chunks(3) {
                assert!(close(t[0], [0.0, 1.0]));
                for p in &t[1..] {
                    assert!(((p[0]).hypot(p[1] - 1.0) - 0.5).abs() < 1e-12);
                }
            }
            // The mark: from the centre towards the angle.
            let Item::Line { points: mark, .. } = &out[1] else {
                panic!()
            };
            assert!(close(mark[1], [-0.5, 1.0]));
            points.len() / 3
        };
        let (small, large) = (pieces(1.0), pieces(1000.0));
        assert_eq!(small, 8);
        assert!(large > 4 * small, "{small} {large}");
        // No piece strays more than the tolerance from the circle.
        let n = arc_steps(500.0) as f64;
        assert!(500.0 * (1.0 - (PI / n).cos()) <= TOLERANCE);
        let e = 1e-9;
        assert!(d.contains([0.5, 1.0], 1.0));
        assert!(d.contains([0.0, 0.5 + e], 1.0));
        assert!(!d.contains([0.0, 0.5 - e], 1.0));
        assert!(!d.contains([0.36, 1.36], 1.0));
        let [lo, hi] = d.bounds();
        assert!(close(lo, [-0.5, 0.5]) && close(hi, [0.5, 1.5]));
        // Moved by a frame, the mark turns with it.
        let m = d.moved(Pose {
            x: 1.0,
            y: 0.0,
            angle: FRAC_PI_2,
        });
        let Instance::Disc { centre, angle, .. } = m else {
            panic!()
        };
        assert!(close(centre, [0.0, 0.0]));
        assert!((angle.unwrap() - 1.5 * PI).abs() < 1e-12);
    }

    #[test]
    fn points_and_segments_are_sized_on_screen() {
        let p = Instance::Point {
            at: [1.0, 1.0],
            size: 10.0,
            colour: C,
        };
        let mut out = Vec::new();
        p.mesh(123.0, &mut out);
        assert_eq!(
            out,
            [Item::Dot {
                at: [1.0, 1.0],
                radius: 5.0,
                colour: C
            }]
        );
        // 5 points from its centre: 5 units at a point a unit, 2.5 at two.
        assert!(p.contains([6.0, 1.0], 1.0));
        assert!(!p.contains([6.0 + 1e-9, 1.0], 1.0));
        assert!(!p.contains([6.0, 1.0], 2.0));
        assert!(p.contains([3.5, 1.0], 2.0));
        assert_eq!(p.bounds(), [[1.0, 1.0], [1.0, 1.0]]);

        let s = Instance::Segment {
            from: [0.0, 0.0],
            to: [2.0, 0.0],
            width: 4.0,
            colour: C,
        };
        out.clear();
        s.mesh(1.0, &mut out);
        assert_eq!(
            out,
            [Item::Line {
                points: vec![[0.0, 0.0], [2.0, 0.0]],
                width: 4.0,
                colour: C
            }]
        );
        // Within 2 points of the line, beyond its ends by distance.
        assert!(s.contains([1.0, 2.0], 1.0));
        assert!(!s.contains([1.0, 2.0 + 1e-9], 1.0));
        assert!(s.contains([4.0, 0.0], 1.0));
        assert!(!s.contains([3.5, 1.5], 1.0));
        assert!(!s.contains([1.0, 2.0], 2.0));
        assert_eq!(s.bounds(), [[0.0, 0.0], [2.0, 0.0]]);
        // A segment of no length is a dot's worth of line.
        let dot = Instance::Segment {
            from: [1.0, 1.0],
            to: [1.0, 1.0],
            width: 4.0,
            colour: C,
        };
        assert!(dot.contains([1.0, 3.0], 1.0));
        assert!(!dot.contains([1.0, 3.5], 1.0));
    }
}
