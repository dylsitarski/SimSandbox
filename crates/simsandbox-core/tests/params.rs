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
                    suggested_min: 0.0,
                    suggested_max: 10.0,
                    description: "Decay rate".into(),
                    initial_condition: false,
                }],
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
    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        // Starts at 1/k so restart shows whether current parameters are used.
        state[0] = 1.0 / params[0];
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
fn set_param_accepts_values_beyond_the_suggested_range() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    for v in [-0.1, 10.1, 1e6] {
        model.set_param(id, "k", v).unwrap();
        assert_eq!(model.param(id, "k").unwrap(), v);
    }
}

#[test]
fn set_param_rejects_unknown_names_and_non_finite_values() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    assert!(matches!(
        model.set_param(id, "q", 1.0),
        Err(SimError::UnknownParam { .. })
    ));
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            model.set_param(id, "k", bad),
            Err(SimError::InvalidParam { .. })
        ));
    }
    // Rejected values leave the parameter unchanged.
    assert_eq!(model.param(id, "k").unwrap(), 1.0);
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

#[test]
fn restart_reinitialises_from_current_parameters() {
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    let mut sim = Simulator::new(model, 0.0).unwrap();
    let start = sim.snapshot();
    sim.step(0.1).unwrap();
    sim.set_param(id, "k", 4.0).unwrap();
    sim.restart(0.0).unwrap();
    assert_eq!(sim.time(), 0.0);
    assert_eq!(sim.state(id), &[0.25]);
    // Earlier snapshots still restore the earlier run.
    sim.restore(&start);
    assert_eq!(sim.state(id), &[1.0]);
    assert_eq!(sim.param(id, "k").unwrap(), 1.0);
}

#[test]
fn a_snapshot_knows_its_size() {
    // What the session's memory budget for rewind counts (ADR 0060):
    // states, outputs and parameters, one number each here.
    let mut model = Model::new();
    let id = model.add(Box::new(Decay::new()));
    let sim = Simulator::new(model, 0.0).unwrap();
    let spec = sim.model().block(id).unwrap().spec();
    let outputs: usize = spec.outputs().map(|p| p.shape.len()).sum();
    let expected = spec.states.len() + outputs + spec.params.len();
    assert_eq!(sim.snapshot().values(), expected);
}

/// `y = 2·g` from a parameter, declaring what it reads or not (ADR 0075).
struct Gain {
    spec: BlockSpec,
    method: MethodSpec,
}

impl Gain {
    fn new(name: &str, declared: Option<&[&str]>) -> Self {
        use simsandbox_core::block::{PortSpec, Shape};
        let mut y = PortSpec::output("y", Unit::DIMENSIONLESS, Shape::Scalar);
        if let Some(d) = declared {
            y = y.depends_on_params(d);
        }
        let param = |n: &str| ParamSpec {
            name: n.into(),
            unit: Unit::DIMENSIONLESS,
            default: 1.0,
            suggested_min: 0.0,
            suggested_max: 10.0,
            description: String::new(),
            initial_condition: false,
        };
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![y],
                params: vec![param("g"), param("h")],
                states: vec![],
                tables: Vec::new(),
                time: TimeSemantics::Stateless,
            },
            method: Decay::new().method,
        }
    }
}

impl Block for Gain {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }
    fn method(&self) -> &MethodSpec {
        &self.method
    }
    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t + ctx.dt, io)
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs[0] = 2.0 * io.params[0];
        Ok(())
    }
}

#[test]
fn an_undeclared_output_reads_every_driven_parameter() {
    // Not declared: driving h from y is counted as a loop, to be safe.
    let mut model = Model::new();
    let b = model.add(Box::new(Gain::new("b", None)));
    model.drive((b, "y"), (b, "h")).unwrap();
    let sim = Simulator::new(model, 0.0).unwrap();
    assert_eq!(sim.loops(), vec![vec!["b.y".to_string()]]);
    // Declared as reading g only: no loop.
    let mut model = Model::new();
    let b = model.add(Box::new(Gain::new("b", Some(&["g"]))));
    model.drive((b, "y"), (b, "h")).unwrap();
    let sim = Simulator::new(model, 0.0).unwrap();
    assert!(sim.loops().is_empty());
    // A declaration naming no parameter of the block is refused.
    let mut model = Model::new();
    model.add(Box::new(Gain::new("b", Some(&["nope"]))));
    assert!(Simulator::new(model, 0.0).is_err());
}
