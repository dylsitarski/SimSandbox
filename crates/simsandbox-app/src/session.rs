//! One open model: the running simulator, its recorded history for rewind,
//! and the plotted series.

use egui_plot::PlotPoint;
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::SimError;
use simsandbox_file::ModelFile;

/// Default communication step. Model files do not store one yet
/// (open question Q7).
pub const DEFAULT_DT: f64 = 0.01;

/// Take a snapshot for rewind every this many steps.
const SNAPSHOT_EVERY: usize = 10;
/// When history or a series grows past these, every other entry is dropped:
/// the whole run stays reachable, at coarser resolution.
const MAX_SNAPSHOTS: usize = 10_000;
const MAX_POINTS: usize = 100_000;

/// One plotted value: an element of an output port.
pub struct Series {
    pub name: String,
    block: BlockId,
    port: String,
    index: usize,
    pub points: Vec<PlotPoint>,
}

pub struct Session {
    /// Display name, e.g. the file name.
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded; the source for saving.
    pub file: ModelFile,
    pub warnings: Vec<String>,
    pub dt: f64,
    pub series: Vec<Series>,
    sim: Simulator,
    history: Vec<Snapshot>,
    steps_since_snapshot: usize,
}

impl Session {
    /// Load a model file. Errors are formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
        let mut series = Vec::new();
        let plots = &loaded.file.controls.plots;
        for entry in &loaded.file.blocks {
            let id = loaded.ids[&entry.name];
            let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
            for port in spec.outputs() {
                let key = format!("{}.{}", entry.name, port.name);
                // Plot what the file asks for, or every output if it says nothing.
                if !plots.is_empty() && !plots.contains(&key) {
                    continue;
                }
                for index in 0..port.shape.len() {
                    series.push(Series {
                        name: match port.shape {
                            Shape::Scalar => key.clone(),
                            Shape::Vector(_) => format!("{key}[{index}]"),
                        },
                        block: id,
                        port: port.name.clone(),
                        index,
                        points: Vec::new(),
                    });
                }
            }
        }
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            file: loaded.file,
            warnings: loaded.warnings,
            dt: DEFAULT_DT,
            series,
            history: vec![sim.snapshot()],
            sim,
            steps_since_snapshot: 0,
        };
        session.record();
        Ok(session)
    }

    pub fn time(&self) -> f64 {
        self.sim.time()
    }

    pub fn labels(&self) -> Option<MethodLabels> {
        self.sim.labels()
    }

    /// Advance one communication step and record it.
    pub fn step(&mut self) -> Result<(), SimError> {
        self.sim.step(self.dt)?;
        self.record();
        self.steps_since_snapshot += 1;
        if self.steps_since_snapshot >= SNAPSHOT_EVERY {
            self.steps_since_snapshot = 0;
            self.history.push(self.sim.snapshot());
            if self.history.len() > MAX_SNAPSHOTS {
                thin(&mut self.history);
            }
        }
        Ok(())
    }

    /// Earliest and latest time the run can be rewound to.
    pub fn history_range(&self) -> (f64, f64) {
        let first = self.history.first().map_or(0.0, |s| s.t);
        let last = self.history.last().map_or(first, |s| s.t);
        (first, last)
    }

    /// Rewind to the latest snapshot at or before `t`. Everything recorded
    /// after it is discarded, so running on starts a new branch.
    pub fn rewind_to(&mut self, t: f64) {
        let keep = self.history.iter().rposition(|s| s.t <= t).unwrap_or(0);
        self.history.truncate(keep + 1);
        let snap = &self.history[keep];
        self.sim.restore(snap);
        let t0 = snap.t;
        for s in &mut self.series {
            s.points.retain(|p| p.x <= t0);
        }
        self.steps_since_snapshot = 0;
    }

    /// Back to the start of the run.
    pub fn reset(&mut self) {
        self.rewind_to(f64::NEG_INFINITY);
    }

    fn record(&mut self) {
        let t = self.sim.time();
        for s in &mut self.series {
            if let Some(v) = self
                .sim
                .output(s.block, &s.port)
                .and_then(|v| v.get(s.index))
            {
                s.points.push(PlotPoint::new(t, *v));
                if s.points.len() > MAX_POINTS {
                    thin(&mut s.points);
                }
            }
        }
    }
}

/// Drop every other entry, keeping the first and the last.
fn thin<T>(v: &mut Vec<T>) {
    let last = v.len() - 1;
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
    use crate::examples::{registry, EXAMPLES};

    fn lorenz() -> Session {
        Session::load("lorenz", EXAMPLES[0].text, &registry()).unwrap()
    }

    #[test]
    fn examples_load_and_plot_what_their_controls_ask_for() {
        let s = lorenz();
        let names: Vec<_> = s.series.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["lorenz.xyz[0]", "lorenz.xyz[1]", "lorenz.xyz[2]"]);
        assert!(s.warnings.is_empty());
        for e in EXAMPLES {
            Session::load(e.name, e.text, &registry()).unwrap();
        }
    }

    #[test]
    fn rewind_then_rerun_reproduces_the_run() {
        let mut s = lorenz();
        for _ in 0..300 {
            s.step().unwrap();
        }
        let reference: Vec<_> = s.series[0].points.clone();

        s.rewind_to(1.234);
        // Latest snapshot at or before 1.234 (snapshots every 0.1 s).
        assert!((s.time() - 1.2).abs() < 1e-9, "{}", s.time());
        assert!(s.series[0].points.last().unwrap().x <= s.time());
        while s.series[0].points.len() < reference.len() {
            s.step().unwrap();
        }
        assert_eq!(s.series[0].points, reference);

        s.reset();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.series[0].points.len(), 1);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    #[test]
    fn long_runs_are_thinned_not_truncated() {
        let mut v: Vec<usize> = (0..11).collect();
        thin(&mut v);
        assert_eq!(v, [0, 2, 4, 6, 8, 10]);
        let mut v: Vec<usize> = (0..10).collect();
        thin(&mut v);
        assert_eq!(v, [0, 2, 4, 6, 8, 9]);
    }

    #[test]
    fn bad_files_report_errors() {
        let err = Session::load("x", "{", &registry()).err().unwrap();
        assert!(err.contains("line"), "{err}");
    }
}
