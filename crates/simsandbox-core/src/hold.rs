//! Sample and hold (ADR 0063): what a probe feeding its own model puts
//! into it. At each communication point its probe records at, it takes
//! its input and passes it on in the same instant (an output with direct
//! feedthrough, evaluated in dependency order, ADR 0040); in between it
//! holds the last value. What it holds, and where its schedule stands,
//! is its state, so rewind and replay are exact. It never joins a fused
//! group: what it feeds sees a value held over each step.

use crate::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, Shape, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use crate::schedule::{Every, Trigger};
use crate::units::Unit;
use crate::Result;

/// State: whether it samples now, where its schedule stands (next time,
/// next listed time), then the value held.
const DUE: usize = 0;
const NEXT_TIME: usize = 1;
const NEXT_INDEX: usize = 2;
const HELD: usize = 3;

/// One signal of a probe, held between the probe's samples.
pub struct SampleHold {
    spec: BlockSpec,
    method: MethodSpec,
    every: Every,
}

impl SampleHold {
    /// A hold named `name` of a signal of this unit and shape (input `u`,
    /// output `y`), sampling as `every` says.
    pub fn new(name: &str, unit: Unit, shape: Shape, every: Every) -> Self {
        let n = shape.len();
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![
                    PortSpec::input("u", unit, shape),
                    PortSpec::output("y", unit, shape).depends_on(&["u"]),
                ],
                params: Vec::new(),
                states: ["due", "next_time", "next_index"]
                    .into_iter()
                    .map(String::from)
                    .chain((0..n).map(|k| format!("held{k}")))
                    .collect(),
                tables: Vec::new(),
                time: TimeSemantics::DiscreteEvent,
            },
            method: MethodSpec {
                name: "hold".into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Quantitative,
                },
            },
            every,
        }
    }

    fn trigger(&self, state: &[f64]) -> Trigger {
        let mut t = Trigger::new(self.every.clone());
        t.set_position((state[NEXT_TIME], state[NEXT_INDEX] as usize));
        t
    }
}

impl Block for SampleHold {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, _params: &[f64], state: &mut [f64]) -> Result<()> {
        // A run starts at t = 0, step 0.
        let mut t = Trigger::new(self.every.clone());
        state[DUE] = if t.due(0.0, 0, 0.0) { 1.0 } else { 0.0 };
        let (next_time, next_index) = t.position();
        state[NEXT_TIME] = next_time;
        state[NEXT_INDEX] = next_index as f64;
        state[HELD..].fill(0.0);
        Ok(())
    }

    /// Whether it samples at the end of this step, decided as a probe
    /// recording then would.
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let t = ctx.t + ctx.dt;
        let eps = 1e-9 * ctx.dt.max(t.abs());
        let mut trigger = self.trigger(io.state);
        io.state[DUE] = if trigger.due(t, ctx.step + 1, eps) {
            1.0
        } else {
            0.0
        };
        let (next_time, next_index) = trigger.position();
        io.state[NEXT_TIME] = next_time;
        io.state[NEXT_INDEX] = next_index as f64;
        Ok(())
    }

    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        if io.state[DUE] == 1.0 {
            io.state[HELD..].copy_from_slice(io.inputs);
        }
        io.outputs.copy_from_slice(&io.state[HELD..]);
        Ok(())
    }
}
