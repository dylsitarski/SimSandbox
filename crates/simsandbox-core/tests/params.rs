//! Parameter access and validation on `Model` and `Simulator`.

use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, ParamSpec, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};

/// x' = -k x, advanced with one explicit Euler step per communication step.
struct Decay {
    spec: BlockSpec,
    method: MethodSpec,
}

impl Decay {
    fn new() -> Self {
        Self {
            spec: BlockSpec {
                name: "decay".into(),
                domain: Domain::Equations,
                ports: vec![],
                params: vec![ParamSpec {
                    name: "k".into(),
                    unit: Unit::DIMENSIONLESS,
                    default: 1.0,
                    min: 0.0,
                    max: 10.0,
                    description: "Decay rate".into(),
                }],
                states: vec!["x".into()],
                time: TimeSemantics::Continuous,
            },
            method: MethodSpec {
                name: "euler".into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Qualitative,
                },
            },
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
        io.state[0] -= ctx.dt * io.params[0] * io.state[0];
        Ok(())
    }
    fn compute_outputs(&self, _t: f64, _io: StepIo<'_>) -> Result<()> {
        Ok(())
    }
}

#[test]
fn params_can_be_read_and_listed() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    assert_eq!(model.block_ids().collect::<Vec<_>>(), vec![id]);
    assert_eq!(model.param(id, "k").unwrap(), 1.0);
    model.set_param(id, "k", 2.5).unwrap();
    assert_eq!(model.params(id).unwrap(), &[2.5]);

    let mut sim = Simulator::new(model, 0.0).unwrap();
    sim.set_param(id, "k", 3.0).unwrap();
    assert_eq!(sim.param(id, "k").unwrap(), 3.0);
    assert_eq!(sim.model().block(id).unwrap().spec().params[0].name, "k");
}

#[test]
fn set_param_rejects_unknown_names_and_bad_values() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    assert!(matches!(
        model.set_param(id, "q", 1.0),
        Err(SimError::UnknownParam { .. })
    ));
    for bad in [-0.1, 10.1, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            model.set_param(id, "k", bad),
            Err(SimError::ParamOutOfRange { .. })
        ));
    }
    // Range ends are allowed; rejected values leave the parameter unchanged.
    model.set_param(id, "k", 10.0).unwrap();
    assert!(model.set_param(id, "k", 11.0).is_err());
    assert_eq!(model.param(id, "k").unwrap(), 10.0);
}

#[test]
fn restore_rewinds_parameters() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    let mut sim = Simulator::new(model, 0.0).unwrap();
    sim.step(0.1).unwrap();
    let snap = sim.snapshot();
    sim.step(0.1).unwrap();
    let expected = sim.state(id).to_vec();

    sim.restore(&snap);
    sim.set_param(id, "k", 5.0).unwrap();
    sim.step(0.1).unwrap();
    assert_ne!(sim.state(id), expected.as_slice());

    sim.restore(&snap);
    assert_eq!(sim.param(id, "k").unwrap(), 1.0);
    sim.step(0.1).unwrap();
    assert_eq!(sim.state(id), expected.as_slice());
}
