//! Rewind snapshots leave out outputs that copy a state (ADR 0063), and
//! keep them when they do not.

use simsandbox_core::block::Shape;
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::Result;

/// x' = -x by Euler, with outputs `x` (declared a copy of the state) and
/// `y`, which is `x` too if `honest`, else `2x` (wrongly declared a copy).
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
        io.state[0] -= ctx.dt * io.state[0];
        self.compute_outputs(ctx.t + ctx.dt, io)
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs[0] = io.state[0];
        io.outputs[1] = if self.honest {
            io.state[0]
        } else {
            2.0 * io.state[0]
        };
        Ok(())
    }
}

#[test]
fn outputs_copying_a_state_are_left_out_and_restored() {
    let mut model = Model::new();
    let honest = model.add(Box::new(Decay::new("honest", true)));
    let wrong = model.add(Box::new(Decay::new("wrong", false)));
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
    // Two states, the honest block's outputs left out, the wrong one's
    // kept whole (no parameters).
    assert_eq!(snap.values(), 2 + 2);
    sim.step(0.1).unwrap();
    sim.restore(&snap);
    assert_eq!(out(&sim), before);
    assert_eq!(before[3], 2.0 * before[2]);
}
