//! The jointed models as SimSandbox's typed equations (`typed/*.ssbx.json`),
//! loaded by `simsandbox-file` and stepped by the kernel's `Simulator`, as
//! the CLI runs them: the fourth route beside Rapier, MuJoCo and the native
//! solver.

use crate::common::{self, pendulum as pd, SnapKind, Snapshot};
use crate::rapier_2d::Model;
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;

/// The model files, by the spike's model.
pub fn file_for(model: Model) -> String {
    match model {
        Model::Pendulum { .. } => "pendulum".into(),
        Model::CartPole => "cartpole".into(),
        Model::Chain { links } => format!("chain-{links}"),
        _ => panic!("no typed model for contacts"),
    }
}

pub fn path(name: &str) -> String {
    format!("{}/typed/{name}.ssbx.json", env!("CARGO_MANIFEST_DIR"))
}

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).expect("equation types");
    r
}

/// A typed-equations model loaded and ready to step.
pub struct Typed {
    pub sim: Simulator,
    pub id: BlockId,
    text: String,
    method: String,
}

impl Typed {
    /// Load `typed/NAME.ssbx.json` with its single block's method replaced
    /// by `method` (`rk4`, `dopri5`, `trbdf2`, …).
    pub fn load(name: &str, method: &str) -> Self {
        Self::load_path(&path(name), method)
    }
    pub fn load_path(file: &str, method: &str) -> Self {
        let text = std::fs::read_to_string(file).expect("model file");
        let rk4 = "\"method\": {\n        \"name\": \"rk4\"\n      }";
        assert!(
            text.contains(rk4),
            "the file's method is written as expected"
        );
        let text = text.replace(rk4, &format!("\"method\": {{\"name\": \"{method}\"}}"));
        let loaded = simsandbox_file::load_any(&text, &registry(), None).expect("model loads");
        let id = *loaded.ids.values().next().expect("one block");
        let sim = Simulator::new(loaded.model, 0.0).expect("simulator");
        Typed {
            sim,
            id,
            text,
            method: method.into(),
        }
    }
    pub fn out(&self, port: &str) -> &[f64] {
        self.sim.output(self.id, port).expect("port")
    }
    pub fn set(&mut self, param: &str, v: f64) {
        self.sim.set_param(self.id, param, v).expect("param");
    }
    pub fn step(&mut self, dt: f64) -> bool {
        self.sim.step(dt).is_ok()
    }
    /// How the block's code runs (`interpreted`, `compiled`, `native`).
    pub fn code(&self) -> String {
        self.sim
            .model()
            .block(self.id)
            .ok()
            .and_then(|b| b.as_generated_code().map(|c| c.code()))
            .unwrap_or_default()
    }
    fn fresh(&self) -> Simulator {
        let loaded = simsandbox_file::load_any(&self.text, &registry(), None).expect("model loads");
        Simulator::new(loaded.model, 0.0).expect("simulator")
    }
}

/// The typed models on the spike's cases.
pub struct T2 {
    pub t: Typed,
    model: Model,
    dt: f64,
    failures: usize,
}

impl T2 {
    pub fn new(model: Model, method: &str, dt: f64) -> Self {
        let mut t = Typed::load(&file_for(model), method);
        match model {
            Model::Pendulum { amplitude } => {
                t.set("theta0", amplitude);
                t.sim.restart(0.0).expect("restart");
            }
            Model::CartPole => {}
            Model::Chain { .. } => {}
            _ => unreachable!(),
        }
        T2 {
            t,
            model,
            dt,
            failures: 0,
        }
    }
}

impl common::Sim for T2 {
    fn label(&self) -> String {
        format!("Typed equations, {}", self.t.method)
    }
    fn dt(&self) -> f64 {
        self.dt
    }
    fn step(&mut self, input: &[f64]) {
        if let Model::CartPole = self.model {
            // Inputs held over the step: parameters set before it.
            self.t.set("F", input[0]);
            self.t.set("fx", input[1]);
            self.t.set("fy", input[2]);
        }
        if !self.t.step(self.dt) {
            self.failures += 1;
        }
    }
    fn coords(&self) -> Vec<f64> {
        match self.model {
            Model::Pendulum { .. } => vec![self.t.out("theta")[0], self.t.out("omega")[0]],
            Model::CartPole => vec![
                self.t.out("x")[0],
                self.t.out("v")[0],
                self.t.out("th")[0],
                self.t.out("w")[0],
            ],
            _ => self.rows(),
        }
    }
    fn rows(&self) -> Vec<f64> {
        let o = |p: &str| self.t.out(p).to_vec();
        match self.model {
            Model::Pendulum { .. } => {
                let (x, y, a, vx, vy, w) =
                    (o("x"), o("y"), o("theta"), o("vx"), o("vy"), o("omega"));
                vec![x[0], y[0], a[0], vx[0], vy[0], w[0]]
            }
            Model::CartPole => {
                let (x, v, th, w) = (o("x")[0], o("v")[0], o("th")[0], o("w")[0]);
                vec![
                    x,
                    0.0,
                    0.0,
                    v,
                    0.0,
                    0.0,
                    o("px")[0],
                    o("py")[0],
                    th,
                    o("pvx")[0],
                    o("pvy")[0],
                    w,
                ]
            }
            _ => {
                let (x, y, a, vx, vy, w) = (o("cx"), o("cy"), o("phi"), o("cvx"), o("cvy"), o("w"));
                (0..x.len())
                    .flat_map(|k| [x[k], y[k], a[k], vx[k], vy[k], w[k]])
                    .collect()
            }
        }
    }
    fn energy(&self) -> f64 {
        // As the others: potential zero at the bottom of the swing.
        let (th, w) = (self.t.out("theta")[0], self.t.out("omega")[0]);
        pd::energy(th, w)
    }
    fn warnings(&self) -> usize {
        self.failures
    }
    fn snapshot(&self, kind: SnapKind) -> Snapshot {
        // The kernel's own snapshot: every block's states, parameters (the
        // held inputs among them) and the outputs it cannot recompute.
        let states = self.t.sim.state(self.t.id).len();
        Snapshot {
            kind,
            bytes: states * 8,
            data: Box::new(self.t.sim.snapshot()),
        }
    }
    fn restore(&mut self, snap: &Snapshot) {
        let s: &simsandbox_core::sim::Snapshot = snap.data.downcast_ref().unwrap();
        if snap.kind == SnapKind::BytesFresh {
            self.t.sim = self.t.fresh();
        }
        self.t.sim.restore(s);
    }
}
