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
/// keep about this many field values in memory: 256 MB (never below
/// [`MIN_SAMPLES`] samples, so a very large grid can use more).
const FIELD_VALUES: usize = 32_000_000;
/// The fewest samples a history with fields keeps before thinning. Large
/// grids reach it soon: a 128 × 128 grid of three fields keeps about 650.
const MIN_SAMPLES: usize = 100;

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
    /// Points of `dim` values whose number changes from sample to sample,
    /// such as a population's positions (ADR 0034).
    Population { dim: usize },
}

impl FieldShape {
    /// Values per frame; 0 for a population, whose frames vary.
    pub fn len(&self) -> usize {
        match *self {
            FieldShape::Grid { nx, ny } => nx * ny,
            FieldShape::Points { count, dim } => count * dim,
            FieldShape::Population { .. } => 0,
        }
    }

    /// Whether a frame of `n` values has this shape.
    pub fn fits(&self, n: usize) -> bool {
        match *self {
            FieldShape::Population { dim } => dim > 0 && n.is_multiple_of(dim),
            _ => n == self.len(),
        }
    }

    /// Values per point, for point sets and populations.
    pub fn point_dim(&self) -> Option<usize> {
        match *self {
            FieldShape::Points { dim, .. } | FieldShape::Population { dim } => Some(dim),
            FieldShape::Grid { .. } => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A field or point set recorded whole at every sample, or (`whole`) one
/// frame made from the whole history, such as a sweep reshaped into a grid
/// (ADR 0028).
pub struct Field {
    pub name: String,
    pub shape: FieldShape,
    /// One frame for the whole history instead of one per sample.
    pub whole: bool,
    frames: Vec<Vec<f64>>,
}

impl Field {
    /// The values at sample `k` (any `k` for a whole-history field; empty
    /// before it is set).
    pub fn frame(&self, k: usize) -> &[f64] {
        if self.whole {
            return self.frames.first().map_or(&[], Vec::as_slice);
        }
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

/// A binding's source and signal (ADR 0063): `probe:speed/car.v` is probe
/// `speed`'s recording of `car.v`; `car.v` alone is the model's, at every
/// step.
pub fn split_source(name: &str) -> (Option<&str>, &str) {
    name.strip_prefix("probe:")
        .and_then(|r| r.split_once('/'))
        .map_or((None, name), |(p, s)| (Some(p), s))
}

/// The binding of probe `probe`'s recording of `signal`.
pub fn from_probe(probe: &str, signal: &str) -> String {
    format!("probe:{probe}/{signal}")
}

/// A binding as shown: `car.v`, or `car.v (speed)` from probe `speed`.
pub fn label(name: &str) -> String {
    match split_source(name) {
        (Some(p), s) => format!("{s} ({p})"),
        (None, s) => s.to_string(),
    }
}

/// What views draw from (ADR 0063): the model's recording at every step,
/// each probe's, by name, and those of filters with clocks of their own,
/// found by their outputs' names.
#[derive(Clone, Copy)]
pub struct Recorded<'a> {
    pub all: &'a History,
    pub probes: &'a [(&'a str, &'a History)],
    pub derived: &'a [&'a History],
}

impl<'a> From<&'a History> for Recorded<'a> {
    fn from(all: &'a History) -> Self {
        Self::single(all)
    }
}

impl<'a> Recorded<'a> {
    /// Only the model's recording.
    pub fn single(all: &'a History) -> Self {
        Self {
            all,
            probes: &[],
            derived: &[],
        }
    }

    /// The recording a binding reads and its signal there; `None` for a
    /// probe that is gone or does not record it.
    pub fn resolve(&self, name: &str) -> Option<(&'a History, SignalRef)> {
        match split_source(name) {
            (None, s) => self.all.resolve(s).map(|r| (self.all, r)).or_else(|| {
                self.derived.iter().find_map(|h| {
                    h.resolve(s)
                        .filter(|r| *r != SignalRef::Time)
                        .map(|r| (*h, r))
                })
            }),
            (Some(p), s) => {
                let (_, h) = self.probes.iter().find(|(n, _)| *n == p)?;
                h.resolve(s).map(|r| (*h, r))
            }
        }
    }
}

/// Signals from one or more recordings at the times all of them recorded
/// (ADR 0063): never interpolated or held. `t` binds to those times.
#[derive(Debug, Default, PartialEq)]
pub struct Paired {
    pub t: Vec<f64>,
    pub columns: Vec<Vec<f64>>,
}

/// Pair `signals` at their common times.
pub fn paired(signals: &[(&History, SignalRef)]) -> Paired {
    let mut sources: Vec<&History> = Vec::new();
    for (h, r) in signals {
        if *r != SignalRef::Time && !sources.iter().any(|s| std::ptr::eq(*s, *h)) {
            sources.push(h);
        }
    }
    if sources.is_empty() {
        sources.extend(signals.first().map(|(h, _)| *h));
    }
    // For each common time, its sample in every source.
    let rows: Vec<Vec<usize>> = match sources.as_slice() {
        [] => Vec::new(),
        [h] => (0..h.len()).map(|k| vec![k]).collect(),
        _ => {
            let base = (0..sources.len())
                .min_by_key(|&i| sources[i].len())
                .unwrap_or(0);
            sources[base]
                .times()
                .iter()
                .enumerate()
                .filter_map(|(k, &t)| {
                    sources
                        .iter()
                        .enumerate()
                        .map(|(i, h)| {
                            if i == base {
                                return Some(k);
                            }
                            let j = h.times().partition_point(|&x| x < t);
                            (h.times().get(j) == Some(&t)).then_some(j)
                        })
                        .collect()
                })
                .collect()
        }
    };
    let at = |h: &History| sources.iter().position(|s| std::ptr::eq(*s, h));
    let t: Vec<f64> = rows.iter().map(|r| sources[0].times()[r[0]]).collect();
    let columns = signals
        .iter()
        .map(|(h, s)| match (s, at(h)) {
            (SignalRef::Time, _) | (_, None) => t.clone(),
            (s, Some(i)) => rows.iter().map(|r| h.value(*s, r[i])).collect(),
        })
        .collect();
    Paired { t, columns }
}

pub struct History {
    /// What the index is: `t`, or a swept parameter such as `spring.w`.
    index: String,
    coordinates: Vec<Coordinate>,
    /// The samples are a cloud with no order (ADR 0028).
    unordered: bool,
    /// Display labels for signals and fields whose names are not readable
    /// on their own, e.g. a cut filter's `curves[0]` as `spring.c = 0.05`.
    labels: std::collections::BTreeMap<String, String>,
    names: Vec<String>,
    times: Vec<f64>,
    /// Per signal, `(t, value)` points, ready to plot against time.
    points: Vec<Vec<PlotPoint>>,
    fields: Vec<Field>,
    max_samples: usize,
    /// Keep only this many of the newest samples, forgetting older ones,
    /// instead of thinning (a probe's `keep`, ADR 0062).
    last: Option<usize>,
    /// Values held in population frames, which vary in size.
    population_values: usize,
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
            .map_or(MAX_SAMPLES, |n| n.clamp(MIN_SAMPLES, MAX_SAMPLES));
        Self {
            index: index.into(),
            coordinates: Vec::new(),
            labels: Default::default(),
            unordered: false,
            names,
            times: Vec::new(),
            points,
            fields: fields
                .into_iter()
                .map(|(name, shape)| Field {
                    name,
                    shape,
                    whole: false,
                    frames: Vec::new(),
                })
                .collect(),
            max_samples,
            last: None,
            population_values: 0,
        }
    }

    /// Add a field made from the whole history (e.g. a sweep reshaped into
    /// a grid); its frame is set with [`History::set_whole`].
    pub fn add_whole(&mut self, name: &str, shape: FieldShape) {
        self.fields.push(Field {
            name: name.into(),
            shape,
            whole: true,
            frames: Vec::new(),
        });
    }

    /// Set a whole-history field's frame.
    pub fn set_whole(&mut self, name: &str, frame: Vec<f64>) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.whole && f.name == name) {
            f.frames = vec![frame];
        }
    }

    /// Keep every sample instead of thinning long histories (sweeps, whose
    /// samples are all needed to reshape them).
    pub fn keep_all(&mut self) {
        self.max_samples = usize::MAX;
        self.last = None;
    }

    /// Thin when longer than `samples` (at least 2), instead of the limit
    /// the fields' size sets.
    pub fn thin_past(&mut self, samples: usize) {
        self.max_samples = samples.max(2);
        self.last = None;
    }

    /// Keep only the newest `samples` (at least 1), forgetting the oldest,
    /// instead of thinning.
    pub fn keep_last(&mut self, samples: usize) {
        self.last = Some(samples.max(1));
        self.max_samples = usize::MAX;
    }

    /// Numbers recorded per sample: the signals and the fixed-size fields.
    pub fn values_per_sample(&self) -> usize {
        self.names.len()
            + self
                .fields
                .iter()
                .filter(|f| !f.whole)
                .map(|f| f.shape.len())
                .sum::<usize>()
    }

    /// Mark the samples as an unordered cloud (random designs), which views
    /// draw as points rather than lines.
    pub fn set_unordered(&mut self, unordered: bool) {
        self.unordered = unordered;
    }

    pub fn unordered(&self) -> bool {
        self.unordered
    }

    /// Declare which fields are coordinates of which (ADR 0024).
    pub fn set_coordinates(&mut self, coordinates: Vec<Coordinate>) {
        self.coordinates = coordinates;
    }

    /// Show `name` as `label` in legends and on axes.
    pub fn set_label(&mut self, name: &str, label: &str) {
        self.labels.insert(name.into(), label.into());
    }

    /// The label of a signal or field: its display label, or its name.
    pub fn label(&self, s: SignalRef) -> &str {
        let name = self.name(s);
        self.labels.get(name).map_or(name, String::as_str)
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
        let per_sample = self.fields.iter_mut().filter(|f| !f.whole);
        self.times.push(t);
        for (p, &v) in self.points.iter_mut().zip(values) {
            p.push(PlotPoint::new(t, v));
        }
        for (f, frame) in per_sample.zip(frames) {
            // A misshapen frame is recorded as NaN (empty for a population);
            // an empty one as not recorded (ADR 0063: a field wired after
            // the run started has no past), which views draw as nothing.
            let population = matches!(f.shape, FieldShape::Population { .. });
            f.frames.push(if f.shape.fits(frame.len()) {
                frame.to_vec()
            } else if population || frame.is_empty() {
                Vec::new()
            } else {
                vec![f64::NAN; f.shape.len()]
            });
            if population {
                self.population_values += f.frames.last().map_or(0, Vec::len);
            }
        }
        if let Some(n) = self.last {
            // Forget the oldest in batches, so each sample costs little.
            if self.times.len() > n + n / 8 {
                let drop = self.times.len() - n;
                self.times.drain(..drop);
                for p in &mut self.points {
                    p.drain(..drop);
                }
                for f in self.fields.iter_mut().filter(|f| !f.whole) {
                    f.frames.drain(..drop);
                }
                self.count_population_values();
            }
            return;
        }
        // Populations count by the values they hold, like fixed fields.
        let crowded = self.population_values > FIELD_VALUES && self.times.len() > MIN_SAMPLES;
        if self.times.len() > self.max_samples || crowded {
            thin(&mut self.times);
            for p in &mut self.points {
                thin(p);
            }
            for f in self.fields.iter_mut().filter(|f| !f.whole) {
                thin(&mut f.frames);
            }
            self.count_population_values();
        }
    }

    fn count_population_values(&mut self) {
        self.population_values = self
            .fields
            .iter()
            .filter(|f| !f.whole && matches!(f.shape, FieldShape::Population { .. }))
            .flat_map(|f| f.frames.iter().map(Vec::len))
            .sum();
    }

    /// Drop every sample after `t` (rewind).
    pub fn truncate_after(&mut self, t: f64) {
        let keep = self.times.partition_point(|&x| x <= t);
        self.times.truncate(keep);
        for p in &mut self.points {
            p.truncate(keep);
        }
        for f in self.fields.iter_mut().filter(|f| !f.whole) {
            f.frames.truncate(keep);
        }
        self.count_population_values();
    }

    pub fn clear(&mut self) {
        self.times.clear();
        for p in &mut self.points {
            p.clear();
        }
        for f in &mut self.fields {
            f.frames.clear();
        }
        self.population_values = 0;
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
    fn population_frames_vary_in_size() {
        let shape = FieldShape::Population { dim: 2 };
        let mut h = History::with_fields("t", vec![], vec![("p".into(), shape)]);
        h.push_sample(0.0, &[], &[&[1.0, 2.0, 3.0, 4.0]]);
        h.push_sample(1.0, &[], &[&[5.0, 6.0]]);
        // Not whole points: recorded as no points.
        h.push_sample(2.0, &[], &[&[7.0]]);
        let f = &h.fields()[0];
        assert_eq!(f.frame(0).len(), 4);
        assert_eq!(f.frame(1), [5.0, 6.0]);
        assert!(f.frame(2).is_empty());
        assert_eq!(f.shape.point_dim(), Some(2));
        assert!(shape.fits(0) && !shape.fits(3));
        h.truncate_after(0.5);
        assert_eq!(h.population_values, 4);

        // Thinned by the values held, as fixed fields are, but never below
        // MIN_SAMPLES samples.
        let mut big = History::with_fields("t", vec![], vec![("p".into(), shape)]);
        let frame = vec![0.0; 40_000];
        for k in 0..2_000 {
            big.push_sample(k as f64, &[], &[&frame]);
        }
        assert!(
            big.population_values <= FIELD_VALUES,
            "{}",
            big.population_values
        );
        assert!(big.len() > FIELD_VALUES / frame.len() / 2, "{}", big.len());
        assert_eq!(big.population_values, big.len() * frame.len());
        assert_eq!(big.times()[0], 0.0);
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
        // A large field lowers the sample limit, never below MIN_SAMPLES.
        let big = History::with_fields(
            "t",
            vec![],
            vec![("f".into(), FieldShape::Grid { nx: 1000, ny: 1000 })],
        );
        assert_eq!(big.max_samples, MIN_SAMPLES);
    }
}
