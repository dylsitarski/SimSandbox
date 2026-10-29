//! Rewind snapshots leave out outputs they can have again exactly from
//! the state (ADR 0063), and keep them when they cannot.

use simsandbox_core::block::Shape;
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::Result;

/// x' = -x by Euler, with outputs `x` and `y` declared copies of the
/// state. An honest one computes them after its step, as
/// `compute_outputs` does; a lagging one writes `x` and `2x` before
/// moving its state, so neither recomputing nor copying gives them back.
struct Decay {
    spec: BlockSpec,
    method: MethodSpec,
    honest: bool,
}

impl Decay {
    fn new(name: &str, honest: bool) -> Self {
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![
                    PortSpec::output("x", Unit::DIMENSIONLESS, Shape::Scalar).from_state(0),
                    PortSpec::output("y", Unit::DIMENSIONLESS, Shape::Scalar).from_state(0),
                ],
                params: vec![],
                states: vec!["x".into()],
                tables: Vec::new(),
                time: TimeSemantics::Continuous,
            },
            method: MethodSpec {
                name: "euler".into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Qualitative,
                },
            },
            honest,
        }
    }
}

impl Block for Decay {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }
    fn method(&self) -> &MethodSpec {
        &self.method
    }
    fn init(&mut self, _params: &[f64], state: &mut [f64]) -> Result<()> {
        state[0] = 1.0;
        Ok(())
    }
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        if self.honest {
            io.state[0] -= ctx.dt * io.state[0];
            return self.compute_outputs(ctx.t + ctx.dt, io);
        }
        io.outputs[0] = io.state[0];
        io.outputs[1] = 2.0 * io.state[0];
        io.state[0] -= ctx.dt * io.state[0];
        Ok(())
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs[0] = io.state[0];
        io.outputs[1] = io.state[0];
        Ok(())
    }
}

#[test]
fn outputs_had_again_from_the_state_are_left_out_and_restored() {
    let mut model = Model::new();
    let honest = model.add(Box::new(Decay::new("honest", true)));
    let wrong = model.add(Box::new(Decay::new("lagging", false)));
    let mut sim = Simulator::new(model, 0.0).unwrap();
    for _ in 0..3 {
        sim.step(0.1).unwrap();
    }
    let out = |sim: &Simulator| -> Vec<f64> {
        [honest, wrong]
            .iter()
            .flat_map(|&b| ["x", "y"].map(|p| sim.output(b, p).unwrap()[0]))
            .collect()
    };
    let before = out(&sim);
    let snap = sim.snapshot();
    // Two states, the honest block's outputs left out, the lagging one's
    // kept whole (no parameters).
    assert_eq!(snap.values(), 2 + 2);
    sim.step(0.1).unwrap();
    sim.restore(&snap);
    assert_eq!(out(&sim), before);
    assert_ne!(before[2], before[3]);
}
