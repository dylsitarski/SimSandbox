//! Signals recorded over time.
//!
//! Every signal is recorded at every sample, and the history is thinned,
//! truncated and cleared as a whole, so signals always pair by index.

use egui_plot::PlotPoint;

/// The name that binds a slot to simulated time.
pub const TIME: &str = "t";

/// When a run grows past this many samples, every other sample is dropped:
/// the whole run stays visible, at coarser resolution.
const MAX_SAMPLES: usize = 100_000;

/// A signal a slot can bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalRef {
    /// Simulated time, `t`.
    Time,
    /// A recorded signal, by index.
    Signal(usize),
}

pub struct History {
    names: Vec<String>,
    times: Vec<f64>,
    /// Per signal, `(t, value)` points, ready to plot against time.
    points: Vec<Vec<PlotPoint>>,
}

impl History {
    /// An empty history of the named signals, e.g. `lorenz.xyz[0]`, `a.x`.
    pub fn new(names: Vec<String>) -> Self {
        let points = names.iter().map(|_| Vec::new()).collect();
        Self {
            names,
            times: Vec::new(),
            points,
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Record one sample; `values` has one entry per signal.
    pub fn push(&mut self, t: f64, values: &[f64]) {
        debug_assert_eq!(values.len(), self.names.len());
        self.times.push(t);
        for (p, &v) in self.points.iter_mut().zip(values) {
            p.push(PlotPoint::new(t, v));
        }
        if self.times.len() > MAX_SAMPLES {
            thin(&mut self.times);
            for p in &mut self.points {
                thin(p);
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
    }

    pub fn clear(&mut self) {
        self.times.clear();
        for p in &mut self.points {
            p.clear();
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
        if name == TIME {
            return Some(SignalRef::Time);
        }
        self.names
            .iter()
            .position(|n| n == name)
            .map(SignalRef::Signal)
    }

    pub fn name(&self, s: SignalRef) -> &str {
        match s {
            SignalRef::Time => TIME,
            SignalRef::Signal(i) => &self.names[i],
        }
    }

    /// Value of a signal at sample `k`.
    pub fn value(&self, s: SignalRef, k: usize) -> f64 {
        match s {
            SignalRef::Time => self.times[k],
            SignalRef::Signal(i) => self.points[i][k].y,
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
}
