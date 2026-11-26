//! A checked scene at the newest sample: its bindings read, each layer's
//! instances placed through their frames, and the one list of triangles
//! and lines drawn from them (ADR 0101 point 2).

use super::geometry::{Instance, Item, Pose, P};
use super::spec::{Scene, Source};
use crate::colour::sequential;
use crate::history::{Recorded, SignalRef, Stamp};
use egui::Color32;
use std::collections::HashMap;

/// An instance placed in the scene's own frame: its layer, its row's id
/// (`None` for a layer of one instance) and its shape. What grabbing will
/// test (ADR 0101 point 3).
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub layer: usize,
    pub id: Option<u64>,
    pub shape: Instance,
}

/// A binding's value at the newest sample.
enum Read {
    One(f64),
    Colour(Color32),
    /// By id, and the ids in the table's order.
    Rows(HashMap<u64, f64>, Vec<u64>),
}

impl Read {
    fn of(source: &Source, recorded: &Recorded<'_>) -> Read {
        match source {
            Source::Constant(x) => Read::One(*x),
            Source::Colour(c) => Read::Colour(*c),
            // A parameter is recorded under its name like a signal.
            Source::Scalar(name) | Source::Param(name) => Read::One(
                recorded
                    .resolve(name)
                    .and_then(|(h, s)| Some(h.value(s, h.len().checked_sub(1)?)))
                    .unwrap_or(f64::NAN),
            ),
            Source::Rows(name) => {
                let rows = recorded.resolve(name).and_then(|(h, s)| {
                    let SignalRef::Field(f) = s else { return None };
                    let (_, ids, values) = h.fields()[f].rows(h.len().checked_sub(1)?)?;
                    let dim = values.len().checked_div(ids.len()).unwrap_or(1).max(1);
                    let ids: Vec<u64> = ids.iter().map(|&i| i as u64).collect();
                    let by_id = ids
                        .iter()
                        .enumerate()
                        .map(|(r, &id)| (id, values[r * dim]))
                        .collect();
                    Some((by_id, ids))
                });
                let (by_id, ids) = rows.unwrap_or_default();
                Read::Rows(by_id, ids)
            }
        }
    }

    /// The value for the row `id` (NaN where it has none), or the one value.
    fn at(&self, id: Option<u64>) -> f64 {
        match (self, id) {
            (Read::One(x), _) => *x,
            (Read::Rows(by_id, _), Some(id)) => by_id.get(&id).copied().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }

    /// As a colour: a palette colour, or a number through the colour
    /// scale, 0 dark to 1 bright; `None` where it is not finite.
    fn colour(&self, id: Option<u64>) -> Option<Color32> {
        match self {
            Read::Colour(c) => Some(*c),
            _ => {
                let v = self.at(id);
                v.is_finite().then(|| sequential(v.clamp(0.0, 1.0) as f32))
            }
        }
    }

    fn ids(&self) -> Option<&[u64]> {
        match self {
            Read::Rows(_, ids) => Some(ids),
            _ => None,
        }
    }
}

/// The primitive drawn from one instance's attribute values (in the
/// order the primitive declares them, NaN for an unbound optional one).
fn instance(primitive: &str, v: &[f64], colour: Color32) -> Instance {
    match primitive {
        "points" => Instance::Point {
            at: [v[0], v[1]],
            size: v[2],
            colour,
        },
        "segments" => Instance::Segment {
            from: [v[0], v[1]],
            to: [v[2], v[3]],
            width: v[4],
            colour,
        },
        "boxes" => Instance::Box {
            centre: [v[0], v[1]],
            angle: v[2],
            width: v[3],
            height: v[4],
            colour,
        },
        "discs" => Instance::Disc {
            centre: [v[0], v[1]],
            radius: v[2],
            angle: v[3].is_finite().then_some(v[3]),
            colour,
        },
        other => unreachable!("no primitive '{other}'"),
    }
}

impl Scene {
    /// Where each frame is, for the row `id` of a frame over rows.
    fn pose(&self, reads: &[[Read; 3]], f: usize, id: Option<u64>) -> Pose {
        let [x, y, angle] = &reads[f];
        let local = Pose {
            x: x.at(id),
            y: y.at(id),
            angle: angle.at(id),
        };
        match self.frames[f].parent {
            // A parent over rows is placed for the same row.
            Some(p) => {
                let row = self.frames[p].table.as_ref().and(id);
                self.pose(reads, p, row).of(local)
            }
            None => local,
        }
    }

    /// The pose of frame `f` in the scene, at the newest sample: of one
    /// pose, or for the row `id`.
    pub fn frame_pose(&self, recorded: &Recorded<'_>, f: usize, id: Option<u64>) -> Pose {
        let reads = self.frame_reads(recorded);
        self.pose(&reads, f, id)
    }

    fn frame_reads(&self, recorded: &Recorded<'_>) -> Vec<[Read; 3]> {
        self.frames
            .iter()
            .map(|f| f.pose.each_ref().map(|s| Read::of(s, recorded)))
            .collect()
    }

    /// Every layer's instances at the newest sample, in the scene's own
    /// frame, first layer first. A layer over rows has one instance a row
    /// of its table, its bindings (and its frame's) paired by id; an
    /// instance with a value missing or not finite is left out.
    pub fn place(&self, recorded: &Recorded<'_>) -> Vec<Placed> {
        let frames = self.frame_reads(recorded);
        let mut out = Vec::new();
        for (k, layer) in self.layers.iter().enumerate() {
            let reads: Vec<Option<Read>> = layer
                .attributes
                .iter()
                .map(|s| s.as_ref().map(|s| Read::of(s, recorded)))
                .collect();
            // Its rows: the first of its own bindings over rows, else its
            // frame's.
            let ids: Option<Vec<u64>> = reads
                .iter()
                .flatten()
                .find_map(Read::ids)
                .or_else(|| {
                    // Its frame's, or the rows of a frame its frame is in.
                    let mut at = layer.frame;
                    while let Some(f) = at {
                        if let Some(ids) = frames[f].iter().find_map(Read::ids) {
                            return Some(ids);
                        }
                        at = self.frames[f].parent;
                    }
                    None
                })
                .map(<[u64]>::to_vec);
            let rows: Vec<Option<u64>> = match ids {
                Some(ids) => ids.into_iter().map(Some).collect(),
                None => vec![None],
            };
            let colour_at = layer.primitive.attribute("colour");
            for id in rows {
                let mut values = Vec::with_capacity(reads.len());
                let mut colour = None;
                let mut ok = true;
                for (a, r) in reads.iter().enumerate() {
                    if Some(a) == colour_at {
                        colour = r.as_ref().and_then(|r| r.colour(id));
                        ok &= colour.is_some();
                        values.push(f64::NAN);
                        continue;
                    }
                    match r {
                        Some(r) => {
                            let v = r.at(id);
                            ok &= v.is_finite();
                            values.push(v);
                        }
                        // Optional and unbound: nothing drawn for it.
                        None => values.push(f64::NAN),
                    }
                }
                let (true, Some(colour)) = (ok, colour) else {
                    continue;
                };
                let pose = match layer.frame {
                    Some(f) => self.pose(&frames, f, id),
                    None => Pose::IDENTITY,
                };
                if !pose.is_finite() {
                    continue;
                }
                out.push(Placed {
                    layer: k,
                    id,
                    shape: instance(layer.primitive.name, &values, colour).moved(pose),
                });
            }
        }
        out
    }

    /// What the picture is drawn in: the scene's own frame, or the
    /// `view_from` frame's (ADR 0093 point 4), as the pose that takes the
    /// scene's coordinates there; `None` if that frame's pose is unknown.
    pub fn view(&self, recorded: &Recorded<'_>) -> Option<Pose> {
        match self.view_from {
            None => Some(Pose::IDENTITY),
            Some(f) => {
                let pose = self.frame_pose(recorded, f, None);
                pose.is_finite().then(|| pose.inverse())
            }
        }
    }

    /// The recordings it reads and how often each has changed: the same
    /// stamps mean the same picture.
    pub fn stamps(&self, recorded: &Recorded<'_>) -> Vec<Stamp> {
        self.reads()
            .iter()
            .filter_map(|n| recorded.resolve(n))
            .map(|(h, _)| h.stamp())
            .collect()
    }
}

/// The instance grabbing takes at `p` (ADR 0101 point 3): of the layers
/// with a `grab`, the topmost instance that contains `p`, tested exactly
/// in the scene's own frame (never the triangles). `scale` is screen
/// points per unit, for points and segments sized on screen.
pub fn picked<'p>(scene: &Scene, placed: &'p [Placed], p: P, scale: f64) -> Option<&'p Placed> {
    placed
        .iter()
        .rev()
        .find(|i| scene.layers[i.layer].grab.is_some() && i.shape.contains(p, scale))
}

/// The topmost instance of any layer that contains `p`: what a press
/// selects (ADR 0108, one selection).
pub fn touched(placed: &[Placed], p: P, scale: f64) -> Option<&Placed> {
    placed.iter().rev().find(|i| i.shape.contains(p, scale))
}

/// What a scene draws, in the coordinates it is drawn in, and the box
/// holding it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Drawn {
    pub items: Vec<Item>,
    pub bounds: Option<[P; 2]>,
}

/// The one list of triangles and lines (ADR 0101 point 2): every placed
/// instance seen through `view`, at `scale` screen points per unit.
pub fn draw(placed: &[Placed], view: Pose, scale: f64) -> Drawn {
    let mut out = Drawn::default();
    for p in placed {
        let shape = p.shape.moved(view);
        let [lo, hi] = shape.bounds();
        out.bounds = Some(match out.bounds {
            None => [lo, hi],
            Some([a, b]) => [
                [a[0].min(lo[0]), a[1].min(lo[1])],
                [b[0].max(hi[0]), b[1].max(hi[1])],
            ],
        });
        shape.mesh(scale, &mut out.items);
    }
    out
}
