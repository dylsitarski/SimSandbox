//! One open model: the running simulator, its recorded history for rewind,
//! the plotted series, and its parameters.

use egui_plot::PlotPoint;
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::ModelFile;
use std::collections::BTreeMap;

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
    pub series: Vec<Series>,
    /// Where the app pauses (the file's `run.t_end`); cleared once reached so
    /// Run continues past it (ADR 0013).
    stop_at: Option<f64>,
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
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end,
            file: loaded.file,
            ids: loaded.ids,
            warnings: loaded.warnings,
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
        if self.time() == 0.0 && self.history.len() == 1 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        self.sim.restart(0.0)?;
        self.history = vec![self.sim.snapshot()];
        self.steps_since_snapshot = 0;
        self.stop_at = self.file.run.t_end;
        for s in &mut self.series {
            s.points.clear();
        }
        self.record();
        Ok(())
    }

    /// The model file with the current parameters, step and default speed
    /// (ADR 0012, ADR 0013), ready to write.
    pub fn save_json(&mut self, speed: f64) -> Result<String, String> {
        self.file
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.run.dt = self.dt;
        self.file.controls.speed = Some(speed);
        Ok(self.file.to_json())
    }

    /// Series `a` against series `b`, for a phase portrait. All series are
    /// recorded, thinned and rewound together, so they pair by index.
    pub fn phase_points(&self, a: usize, b: usize) -> Vec<[f64; 2]> {
        let (sa, sb) = (&self.series[a].points, &self.series[b].points);
        sa.iter().zip(sb).map(|(p, q)| [p.y, q.y]).collect()
    }

    /// Three series as a 3D trajectory.
    pub fn trajectory(&self, axes: [usize; 3]) -> Vec<[f64; 3]> {
        let [a, b, c] = axes.map(|i| &self.series[i].points);
        a.iter()
            .zip(b)
            .zip(c)
            .map(|((p, q), r)| [p.y, q.y, r.y])
            .collect()
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
        self.stop_at = self.file.run.t_end.filter(|&end| end > t0);
        for s in &mut self.series {
            s.points.retain(|p| p.x <= t0);
        }
        self.steps_since_snapshot = 0;
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

        s.rewind_to(0.0);
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.series[0].points.len(), 1);
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
        assert_eq!(s.series[0].points[0].y, 5.0);

        for _ in 0..50 {
            s.step().unwrap();
        }
        s.set_param(id, "x0", -3.0).unwrap();
        assert!(s.time() > 0.0, "a running model is not restarted");
        s.restart().unwrap();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.series[0].points.len(), 1);
        assert_eq!(s.series[0].points[0].y, -3.0);
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
        let text = s.save_json(2.5).unwrap();

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
    fn phase_and_trajectory_points_pair_series_by_time() {
        let mut s = lorenz();
        for _ in 0..200 {
            s.step().unwrap();
        }
        s.rewind_to(1.0);
        let traj = s.trajectory([0, 1, 2]);
        assert_eq!(traj.len(), s.series[0].points.len());
        for (k, p) in traj.iter().enumerate() {
            let t = s.series[0].points[k].x;
            assert!(s.series.iter().all(|ser| ser.points[k].x == t));
            assert_eq!(p[2], s.series[2].points[k].y);
        }
        let phase = s.phase_points(0, 2);
        assert_eq!(phase[0], [1.0, 1.0]);
        assert_eq!(phase.len(), traj.len());
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
