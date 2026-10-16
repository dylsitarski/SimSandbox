//! The co-simulation master and the kernel-owned state store.
//!
//! v0 master: fixed communication step, Jacobi-style exchange (every Block
//! sees the outputs from the end of the previous step). Adaptive steps,
//! iteration and rollback build on this; see the roadmap.

use crate::block::{Causality, MethodLabels, StepContext, StepIo};
use crate::graph::{port_offset, BlockId, Model};
use crate::{Result, SimError};

/// A saved point in a run, used for rewind and branching: time, every
/// Block's state, outputs and parameter values.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub t: f64,
    states: Vec<Vec<f64>>,
    outputs: Vec<Vec<f64>>,
    params: Vec<Vec<f64>>,
}

/// One resolved route from an output slice to an input slice.
struct Route {
    from_block: usize,
    from_offset: usize,
    to_block: usize,
    to_offset: usize,
    len: usize,
    factor: f64,
}

/// Runs a [`Model`].
pub struct Simulator {
    model: Model,
    routes: Vec<Route>,
    states: Vec<Vec<f64>>,
    inputs: Vec<Vec<f64>>,
    outputs: Vec<Vec<f64>>,
    t: f64,
}

impl Simulator {
    /// Validate the model (every input connected), initialise all Blocks and
    /// compute outputs at `t0`.
    pub fn new(model: Model, t0: f64) -> Result<Self> {
        let mut routes = Vec::new();
        for (bi, block) in model.blocks.iter().enumerate() {
            let spec = block.spec();
            for p in spec.inputs() {
                let conn = model
                    .connections
                    .iter()
                    .find(|c| c.to.block.0 == bi && c.to.port == p.name)
                    .ok_or_else(|| {
                        SimError::UnconnectedInput(format!("{}.{}", spec.name, p.name))
                    })?;
                let from_spec = model.blocks[conn.from.block.0].spec();
                let (from_offset, len) =
                    port_offset(&from_spec.ports, Causality::Output, &conn.from.port)
                        .ok_or_else(|| SimError::BadConnection(conn.from.port.clone()))?;
                let (to_offset, _) = port_offset(&spec.ports, Causality::Input, &p.name)
                    .ok_or_else(|| SimError::BadConnection(p.name.clone()))?;
                routes.push(Route {
                    from_block: conn.from.block.0,
                    from_offset,
                    to_block: bi,
                    to_offset,
                    len,
                    factor: conn.factor,
                });
            }
        }

        let mut states = Vec::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for block in &model.blocks {
            let spec = block.spec();
            states.push(vec![0.0; spec.states.len()]);
            inputs.push(vec![0.0; spec.input_len()]);
            outputs.push(vec![0.0; spec.output_len()]);
        }

        let mut sim = Self {
            model,
            routes,
            states,
            inputs,
            outputs,
            t: t0,
        };
        sim.restart(t0)?;
        Ok(sim)
    }

    /// Start the run again at `t0` from the current parameter values: every
    /// Block re-initialises its state, so changed initial conditions take
    /// effect. Snapshots taken earlier stay valid.
    pub fn restart(&mut self, t0: f64) -> Result<()> {
        for (bi, block) in self.model.blocks.iter_mut().enumerate() {
            let params = &self.model.params[bi];
            block.init(params, &mut self.states[bi])?;
            self.inputs[bi].fill(0.0);
            block.compute_outputs(
                t0,
                StepIo {
                    params,
                    inputs: &self.inputs[bi],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                },
            )?;
        }
        self.t = t0;
        self.exchange();
        Ok(())
    }

    /// Copy outputs to connected inputs, applying unit conversion.
    fn exchange(&mut self) {
        for r in &self.routes {
            for k in 0..r.len {
                self.inputs[r.to_block][r.to_offset + k] =
                    r.factor * self.outputs[r.from_block][r.from_offset + k];
            }
        }
    }

    /// Advance every Block by one communication step `dt`.
    pub fn step(&mut self, dt: f64) -> Result<()> {
        let ctx = StepContext { t: self.t, dt };
        for (bi, block) in self.model.blocks.iter_mut().enumerate() {
            block.do_step(
                ctx,
                StepIo {
                    params: &self.model.params[bi],
                    inputs: &self.inputs[bi],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                },
            )?;
        }
        self.t += dt;
        self.exchange();
        Ok(())
    }

    pub fn time(&self) -> f64 {
        self.t
    }

    /// Kernel-owned state of one Block.
    pub fn state(&self, id: BlockId) -> &[f64] {
        &self.states[id.0]
    }

    /// Current value of a named output port.
    pub fn output(&self, id: BlockId, port: &str) -> Option<&[f64]> {
        let spec = self.model.blocks.get(id.0)?.spec();
        let (off, len) = port_offset(&spec.ports, Causality::Output, port)?;
        Some(&self.outputs[id.0][off..off + len])
    }

    /// The model being run: Block specs, parameter values and connections.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Change a parameter while running (live sliders). Checked like
    /// [`Model::set_param`].
    pub fn set_param(&mut self, id: BlockId, name: &str, value: f64) -> Result<()> {
        self.model.set_param(id, name, value)
    }

    /// Current value of a parameter.
    pub fn param(&self, id: BlockId, name: &str) -> Result<f64> {
        self.model.param(id, name)
    }

    /// Speed and accuracy labels of the whole model.
    pub fn labels(&self) -> Option<MethodLabels> {
        MethodLabels::combine(self.model.blocks.iter().map(|b| b.method().labels))
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            t: self.t,
            states: self.states.clone(),
            outputs: self.outputs.clone(),
            params: self.model.params.clone(),
        }
    }

    /// Rewind to a snapshot taken from this simulator, including the
    /// parameter values in effect when it was taken.
    pub fn restore(&mut self, snap: &Snapshot) {
        self.t = snap.t;
        self.states.clone_from(&snap.states);
        self.outputs.clone_from(&snap.outputs);
        self.model.params.clone_from(&snap.params);
        self.exchange();
    }
}
