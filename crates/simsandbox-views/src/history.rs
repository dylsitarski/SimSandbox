//! Signals recorded over time.
//!
//! Every signal is recorded at every sample, and the history is thinned,
//! truncated and cleared as a whole, so signals always pair by index.

use egui_plot::PlotPoint;

/// The name that binds a slot to the index: simulated time, or the swept
/// parameter when a model is swept (ADR 0021).
pub const TIME: &str = "t";

/// When a run grows past this many samples, every other sample is dropped:
/// the whole run stays visible, at coarser resolution.
const MAX_SAMPLES: usize = 100_000;
/// Field frames are large, so with fields the sample limit is lowered to
/// keep about this many field values in memory (never below 1000 samples).
const FIELD_VALUES: usize = 4_000_000;

/// A signal a slot can bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalRef {
    /// Simulated time, `t`.
    Time,
    /// A recorded signal, by index.
    Signal(usize),
    /// A recorded field or point set, by index.
    Field(usize),
}

/// The shape of a recorded field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldShape {
    /// `nx` by `ny` values, row by row.
    Grid { nx: usize, ny: usize },
    /// `count` points of `dim` coordinates, point by point.
    Points { count: usize, dim: usize },
}

impl FieldShape {
    pub fn len(&self) -> usize {
        match *self {
            FieldShape::Grid { nx, ny } => nx * ny,
            FieldShape::Points { count, dim } => count * dim,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A field or point set recorded whole at every sample.
pub struct Field {
    pub name: String,
    pub shape: FieldShape,
    frames: Vec<Vec<f64>>,
}

impl Field {
    /// The values at sample `k`.
    pub fn frame(&self, k: usize) -> &[f64] {
        &self.frames[k]
    }
}

/// A field that gives the positions of another field's cells along one
/// grid axis (ADR 0024): `rod.position` for `rod.temperature` along axis 0,
/// or a swept parameter for every result of a two-parameter sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordinate {
    pub name: String,
    pub of: String,
    pub axis: usize,
}

pub struct History {
    /// What the index is: `t`, or a swept parameter such as `spring.w`.
    index: String,
    coordinates: Vec<Coordinate>,
    names: Vec<String>,
    times: Vec<f64>,
    /// Per signal, `(t, value)` points, ready to plot against time.
    points: Vec<Vec<PlotPoint>>,
    fields: Vec<Field>,
    max_samples: usize,
}

impl History {
    /// An empty history of the named signals, e.g. `lorenz.xyz[0]`, `a.x`,
    /// indexed by time.
    pub fn new(names: Vec<String>) -> Self {
        Self::with_index(TIME, names)
    }

    /// An empty history indexed by something other than time, e.g. a swept
    /// parameter. `t` still binds to the index.
    pub fn with_index(index: &str, names: Vec<String>) -> Self {
        Self::with_fields(index, names, Vec::new())
    }

    /// An empty history of scalar signals and whole fields.
    pub fn with_fields(index: &str, names: Vec<String>, fields: Vec<(String, FieldShape)>) -> Self {
        let points = names.iter().map(|_| Vec::new()).collect();
        let field_values: usize = fields.iter().map(|(_, s)| s.len()).sum();
        let max_samples = FIELD_VALUES
            .checked_div(field_values)
            .map_or(MAX_SAMPLES, |n| n.clamp(1000, MAX_SAMPLES));
        Self {
            index: index.into(),
            coordinates: Vec::new(),
            names,
            times: Vec::new(),
            points,
            fields: fields
                .into_iter()
                .map(|(name, shape)| Field {
                    name,
                    shape,
                    frames: Vec::new(),
                })
                .collect(),
            max_samples,
        }
    }

    /// Declare which fields are coordinates of which (ADR 0024).
    pub fn set_coordinates(&mut self, coordinates: Vec<Coordinate>) {
        self.coordinates = coordinates;
    }

    pub fn coordinates(&self) -> &[Coordinate] {
        &self.coordinates
    }

    /// The grid axis along which field `coord` places the cells of field
    /// `of`, if it is one of its coordinates.
    pub fn coordinate_axis(&self, coord: &str, of: &str) -> Option<usize> {
        self.coordinates
            .iter()
            .find(|c| c.name == coord && c.of == of)
            .map(|c| c.axis)
    }

    /// Whether the history is indexed by time (not a swept parameter).
    pub fn over_time(&self) -> bool {
        self.index == TIME
    }

    /// The recorded fields and point sets.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The name of the index: `t`, or the swept parameter.
    pub fn index(&self) -> &str {
        &self.index
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Record one sample of scalar signals; `values` has one entry per
    /// signal. Use [`History::push_sample`] when there are fields.
    pub fn push(&mut self, t: f64, values: &[f64]) {
        self.push_sample(t, values, &[]);
    }

    /// Record one sample: scalar values and one frame per field.
    pub fn push_sample(&mut self, t: f64, values: &[f64], frames: &[&[f64]]) {
        debug_assert_eq!(values.len(), self.names.len());
        debug_assert_eq!(frames.len(), self.fields.len());
        self.times.push(t);
        for (p, &v) in self.points.iter_mut().zip(values) {
            p.push(PlotPoint::new(t, v));
        }
        for (f, frame) in self.fields.iter_mut().zip(frames) {
            // A missing or misshapen frame is recorded as NaN, so every
            // frame has the field's size.
            let n = f.shape.len();
            f.frames.push(if frame.len() == n {
                frame.to_vec()
            } else {
                vec![f64::NAN; n]
            });
        }
        if self.times.len() > self.max_samples {
            thin(&mut self.times);
            for p in &mut self.points {
                thin(p);
            }
            for f in &mut self.fields {
                thin(&mut f.frames);
            }
        }
    }

    /// Drop every sample after `t` (rewind).
    pub fn truncate_after(&mut self, t: f64) {
        let keep = self.times.partition_point(|&x| x <= t);
        self.times.truncate(keep);
        for p in &mut self.points {
            p.truncate(keep);
        }
        for f in &mut self.fields {
            f.frames.truncate(keep);
        }
    }

    pub fn clear(&mut self) {
        self.times.clear();
        for p in &mut self.points {
            p.clear();
        }
        for f in &mut self.fields {
            f.frames.clear();
        }
    }

    pub fn times(&self) -> &[f64] {
        &self.times
    }

    /// `(t, value)` points of a recorded signal.
    pub fn points(&self, i: usize) -> &[PlotPoint] {
        &self.points[i]
    }

    /// Find a signal by name; `t` is time.
    pub fn resolve(&self, name: &str) -> Option<SignalRef> {
        if name == TIME || name == self.index {
            return Some(SignalRef::Time);
        }
        self.names
            .iter()
            .position(|n| n == name)
            .map(SignalRef::Signal)
            .or_else(|| {
                self.fields
                    .iter()
                    .position(|f| f.name == name)
                    .map(SignalRef::Field)
            })
    }

    pub fn name(&self, s: SignalRef) -> &str {
        match s {
            SignalRef::Time => &self.index,
            SignalRef::Signal(i) => &self.names[i],
            SignalRef::Field(i) => &self.fields[i].name,
        }
    }

    /// Value of a scalar signal at sample `k` (NaN for a field).
    pub fn value(&self, s: SignalRef, k: usize) -> f64 {
        match s {
            SignalRef::Time => self.times[k],
            SignalRef::Signal(i) => self.points[i][k].y,
            SignalRef::Field(_) => f64::NAN,
        }
    }

    /// All values of a signal, in sample order.
    pub fn values(&self, s: SignalRef) -> Vec<f64> {
        (0..self.len()).map(|k| self.value(s, k)).collect()
    }
}

/// Drop every other entry, keeping the first and the last.
pub fn thin<T>(v: &mut Vec<T>) {
    let Some(last) = v.len().checked_sub(1) else {
        return;
    };
    let mut i = 0;
    v.retain(|_| {
        let keep = i % 2 == 0 || i == last;
        i += 1;
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_pair_by_index_through_rewind() {
        let mut h = History::new(vec!["a".into(), "b".into()]);
        for k in 0..10 {
            let t = k as f64 * 0.1;
            h.push(t, &[t, -t]);
        }
        assert_eq!(h.resolve("t"), Some(SignalRef::Time));
        assert_eq!(h.resolve("b"), Some(SignalRef::Signal(1)));
        assert_eq!(h.resolve("c"), None);
        h.truncate_after(0.45);
        assert_eq!(h.len(), 5);
        assert_eq!(h.points(1).len(), 5);
        assert_eq!(h.value(SignalRef::Signal(1), 4), -0.4);
        assert_eq!(h.values(SignalRef::Time), h.times());
        h.clear();
        assert!(h.is_empty());

        let swept = History::with_index("s.w", vec!["s.a".into()]);
        assert_eq!(swept.resolve("t"), Some(SignalRef::Time));
        assert_eq!(swept.resolve("s.w"), Some(SignalRef::Time));
        assert_eq!(swept.name(SignalRef::Time), "s.w");
    }

    #[test]
    fn thinning_keeps_the_first_and_last() {
        let mut v: Vec<usize> = (0..11).collect();
        thin(&mut v);
        assert_eq!(v, [0, 2, 4, 6, 8, 10]);
        let mut v: Vec<usize> = (0..10).collect();
        thin(&mut v);
        assert_eq!(v, [0, 2, 4, 6, 8, 9]);
        let mut empty: Vec<usize> = Vec::new();
        thin(&mut empty);
    }

    #[test]
    fn fields_are_recorded_whole_and_rewound_with_the_signals() {
        let shape = FieldShape::Grid { nx: 3, ny: 1 };
        let mut h = History::with_fields("t", vec!["a".into()], vec![("rod.T".into(), shape)]);
        for k in 0..5 {
            let t = k as f64;
            h.push_sample(t, &[t], &[&[t, 2.0 * t, 3.0 * t]]);
        }
        assert_eq!(h.resolve("rod.T"), Some(SignalRef::Field(0)));
        assert_eq!(h.fields()[0].frame(4), [4.0, 8.0, 12.0]);
        h.truncate_after(2.5);
        assert_eq!(h.len(), 3);
        assert_eq!(h.fields()[0].frame(2), [2.0, 4.0, 6.0]);
        assert!(h.value(SignalRef::Field(0), 0).is_nan());
        // A large field lowers the sample limit, never below 1000.
        let big = History::with_fields(
            "t",
            vec![],
            vec![("f".into(), FieldShape::Grid { nx: 1000, ny: 1000 })],
        );
        assert_eq!(big.max_samples, 1000);
    }
}
