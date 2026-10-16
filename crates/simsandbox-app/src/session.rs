//! One open model: the running simulator, its snapshots for rewind, the
//! recorded signals the views draw from, and its parameters.

use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::{ModelFile, ViewEntry};
use simsandbox_views::history::thin;
use simsandbox_views::History;
use std::collections::BTreeMap;

/// Take a snapshot for rewind every this many steps.
const SNAPSHOT_EVERY: usize = 10;
/// When the snapshots grow past this, every other one is dropped: the whole
/// run stays reachable, at coarser resolution.
const MAX_SNAPSHOTS: usize = 10_000;

/// Where a recorded signal comes from: an element of an output port.
struct Source {
    block: BlockId,
    port: String,
    index: usize,
}

/// One parameter as the panel shows it.
#[derive(Clone)]
pub struct ParamRow {
    /// `"block.param"`, the key used in `controls.params`.
    pub key: String,
    pub block: BlockId,
    pub name: String,
    pub unit: String,
    pub description: String,
    pub suggested: [f64; 2],
    pub initial_condition: bool,
    pub value: f64,
}

pub struct Session {
    /// Display name, e.g. the file name.
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded; the source for saving.
    pub file: ModelFile,
    pub ids: BTreeMap<String, BlockId>,
    pub warnings: Vec<String>,
    pub dt: f64,
    /// Recorded signals, one per plotted output element (ADR 0015).
    pub signals: History,
    sources: Vec<Source>,
    /// Where the app pauses (the file's `run.t_end`); cleared once reached so
    /// Run continues past it (ADR 0013).
    stop_at: Option<f64>,
    sim: Simulator,
    snapshots: Vec<Snapshot>,
    steps_since_snapshot: usize,
}

impl Session {
    /// Load a model file. Errors are formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        let mut sources = Vec::new();
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
                    names.push(match port.shape {
                        Shape::Scalar => key.clone(),
                        Shape::Vector(_) => format!("{key}[{index}]"),
                    });
                    sources.push(Source {
                        block: id,
                        port: port.name.clone(),
                        index,
                    });
                }
            }
        }
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end,
            file: loaded.file,
            ids: loaded.ids,
            warnings: loaded.warnings,
            signals: History::new(names),
            sources,
            snapshots: vec![sim.snapshot()],
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
            self.snapshots.push(self.sim.snapshot());
            if self.snapshots.len() > MAX_SNAPSHOTS {
                thin(&mut self.snapshots);
            }
        }
        Ok(())
    }

    /// The run has reached the file's `t_end`. Returns true once, then the
    /// run may continue past it.
    pub fn take_end_reached(&mut self) -> bool {
        let reached = self
            .stop_at
            .is_some_and(|end| self.time() >= end - 1e-9 * self.dt.max(end.abs()));
        if reached {
            self.stop_at = None;
        }
        reached
    }

    /// Every parameter, in file block order.
    pub fn params(&self) -> Vec<ParamRow> {
        let model = self.sim.model();
        let mut rows = Vec::new();
        for entry in &self.file.blocks {
            let id = self.ids[&entry.name];
            let (Ok(block), Ok(values)) = (model.block(id), model.params(id)) else {
                continue;
            };
            for (p, &value) in block.spec().params.iter().zip(values) {
                rows.push(ParamRow {
                    key: format!("{}.{}", entry.name, p.name),
                    block: id,
                    name: p.name.clone(),
                    unit: if p.unit == Unit::DIMENSIONLESS {
                        String::new()
                    } else {
                        p.unit.to_string()
                    },
                    description: p.description.clone(),
                    suggested: [p.suggested_min, p.suggested_max],
                    initial_condition: p.initial_condition,
                    value,
                });
            }
        }
        rows
    }

    /// Change a parameter while running. Initial conditions apply on
    /// [`Session::restart`], or at once if the run has not started.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<(), String> {
        self.sim
            .set_param(block, name, value)
            .map_err(|e| e.to_string())?;
        if self.time() == 0.0 && self.snapshots.len() == 1 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        self.sim.restart(0.0)?;
        self.snapshots = vec![self.sim.snapshot()];
        self.steps_since_snapshot = 0;
        self.stop_at = self.file.run.t_end;
        self.signals.clear();
        self.record();
        Ok(())
    }

    /// The model file with the current parameters, step, default speed and
    /// views (ADR 0012, ADR 0013, ADR 0015), ready to write.
    pub fn save_json(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
    ) -> Result<String, String> {
        self.file
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.run.dt = self.dt;
        self.file.controls.speed = Some(speed);
        if let Some(v) = views {
            self.file.controls.views = v;
        }
        Ok(self.file.to_json())
    }

    /// Earliest and latest time the run can be rewound to.
    pub fn history_range(&self) -> (f64, f64) {
        let first = self.snapshots.first().map_or(0.0, |s| s.t);
        let last = self.snapshots.last().map_or(first, |s| s.t);
        (first, last)
    }

    /// Rewind to the latest snapshot at or before `t`. Everything recorded
    /// after it is discarded, so running on starts a new branch.
    pub fn rewind_to(&mut self, t: f64) {
        let keep = self.snapshots.iter().rposition(|s| s.t <= t).unwrap_or(0);
        self.snapshots.truncate(keep + 1);
        let snap = &self.snapshots[keep];
        self.sim.restore(snap);
        let t0 = snap.t;
        self.stop_at = self.file.run.t_end.filter(|&end| end > t0);
        self.signals.truncate_after(t0);
        self.steps_since_snapshot = 0;
    }

    fn record(&mut self) {
        let values: Vec<f64> = self
            .sources
            .iter()
            .map(|src| {
                self.sim
                    .output(src.block, &src.port)
                    .and_then(|v| v.get(src.index))
                    .copied()
                    .unwrap_or(f64::NAN)
            })
            .collect();
        self.signals.push(self.sim.time(), &values);
    }
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
        assert_eq!(
            s.signals.names(),
            ["lorenz.xyz[0]", "lorenz.xyz[1]", "lorenz.xyz[2]"]
        );
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
        let reference: Vec<_> = s.signals.points(0).to_vec();

        s.rewind_to(1.234);
        // Latest snapshot at or before 1.234 (snapshots every 0.1 s).
        assert!((s.time() - 1.2).abs() < 1e-9, "{}", s.time());
        assert!(s.signals.points(0).last().unwrap().x <= s.time());
        while s.signals.points(0).len() < reference.len() {
            s.step().unwrap();
        }
        assert_eq!(s.signals.points(0), reference.as_slice());

        s.rewind_to(0.0);
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals.points(0).len(), 1);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    fn param(s: &Session, key: &str) -> f64 {
        s.params().iter().find(|r| r.key == key).unwrap().value
    }

    #[test]
    fn initial_conditions_apply_on_restart_or_at_once_before_running() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        let row = s
            .params()
            .into_iter()
            .find(|r| r.key == "lorenz.x0")
            .unwrap();
        assert!(row.initial_condition);

        // Before the run starts, the plot's first point moves at once.
        s.set_param(id, "x0", 5.0).unwrap();
        assert_eq!(s.signals.points(0)[0].y, 5.0);

        for _ in 0..50 {
            s.step().unwrap();
        }
        s.set_param(id, "x0", -3.0).unwrap();
        assert!(s.time() > 0.0, "a running model is not restarted");
        s.restart().unwrap();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals.points(0).len(), 1);
        assert_eq!(s.signals.points(0)[0].y, -3.0);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    #[test]
    fn saving_records_params_step_controls_and_speed() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        s.set_param(id, "rho", 150.0).unwrap();
        s.dt = 0.005;
        s.file
            .controls
            .params
            .entry("lorenz.beta".into())
            .or_default()
            .slider = true;
        let text = s.save_json(2.5, None).unwrap();

        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(param(&again, "lorenz.rho"), 150.0);
        assert_eq!(again.dt, 0.005);
        assert_eq!(again.file.controls.speed, Some(2.5));
        assert!(again.file.controls.params["lorenz.beta"].slider);
        assert!(again.file.controls.params["lorenz.rho"].slider);
    }

    #[test]
    fn the_run_pauses_once_at_t_end() {
        let text = EXAMPLES[0]
            .text
            .replace(r#""dt": 0.01"#, r#""dt": 0.01, "t_end": 0.5"#);
        let mut s = Session::load("l", &text, &registry()).unwrap();
        let mut steps = 0;
        while !s.take_end_reached() {
            s.step().unwrap();
            steps += 1;
        }
        assert_eq!(steps, 50);
        s.step().unwrap();
        assert!(!s.take_end_reached(), "Run continues past t_end");
        // Rewinding before t_end re-arms it.
        s.rewind_to(0.2);
        assert!(!s.take_end_reached());
        while !s.take_end_reached() {
            s.step().unwrap();
        }
        assert!((s.time() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn bad_files_report_errors() {
        let err = Session::load("x", "{", &registry()).err().unwrap();
        assert!(err.contains("line"), "{err}");
    }
}
