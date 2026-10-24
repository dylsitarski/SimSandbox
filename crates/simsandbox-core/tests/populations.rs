//! Tables and variable-length ports (ADR 0034): populations owned by the
//! kernel, exchanged between Blocks, saved in snapshots and drawn from
//! counter-based randomness.

use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, Shape, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::graph::Model;
use simsandbox_core::random::Draws;
use simsandbox_core::sim::Simulator;
use simsandbox_core::table::{Table, TableSpec};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};

fn method() -> MethodSpec {
    MethodSpec {
        name: "rules".into(),
        labels: MethodLabels {
            speed: SpeedClass::RealTime,
            accuracy: AccuracyClass::Quantitative,
        },
    }
}

/// Walkers on a line: each step every walker moves by a normal draw; those
/// past 5 leave; one is born at 0. Outputs their positions (a population)
/// and their number.
struct Walkers {
    spec: BlockSpec,
    method: MethodSpec,
}

impl Walkers {
    fn new() -> Self {
        Self {
            spec: BlockSpec {
                name: "walkers".into(),
                domain: Domain::Agents,
                ports: vec![
                    PortSpec::output("x", Unit::METER, Shape::Population { dim: 1 }),
                    PortSpec::output("count", Unit::DIMENSIONLESS, Shape::Scalar),
                ],
                params: vec![],
                states: vec![],
                tables: vec![TableSpec::new("walkers", &["x"])],
                time: TimeSemantics::FixedStep { dt: 1.0 },
            },
            method: method(),
        }
    }

    fn outputs(io: StepIo<'_>) {
        let t = &io.tables[0];
        io.population_outputs[0].clear();
        io.population_outputs[0].extend_from_slice(t.column(0));
        io.outputs[0] = t.len() as f64;
    }
}

impl Block for Walkers {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }
    fn method(&self) -> &MethodSpec {
        &self.method
    }
    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }
    fn init_tables(&mut self, _params: &[f64], seed: u64, tables: &mut [Table]) -> Result<()> {
        for k in 0..10 {
            tables[0].push(&[Draws::new(seed, u64::MAX, k).unit(0)]);
        }
        Ok(())
    }
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let t = &mut io.tables[0];
        let ids = t.ids().to_vec();
        for (x, id) in t.column_mut(0).iter_mut().zip(ids) {
            *x += Draws::new(ctx.seed, ctx.step, id).normal(0);
        }
        let x = t.column(0).to_vec();
        t.retain(|r| x[r].abs() <= 5.0);
        t.push(&[0.0]);
        Self::outputs(io);
        Ok(())
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        Self::outputs(io);
        Ok(())
    }
}

/// Reads a population of positions and outputs how many and their sum.
struct Census {
    spec: BlockSpec,
    method: MethodSpec,
}

impl Census {
    fn new(unit: Unit, dim: usize) -> Self {
        Self {
            spec: BlockSpec {
                name: "census".into(),
                domain: Domain::Agents,
                ports: vec![
                    PortSpec::input("x", unit, Shape::Population { dim }),
                    PortSpec::output("n", Unit::DIMENSIONLESS, Shape::Scalar),
                    PortSpec::output("sum", unit, Shape::Scalar),
                ],
                params: vec![],
                states: vec![],
                tables: vec![],
                time: TimeSemantics::FixedStep { dt: 1.0 },
            },
            method: method(),
        }
    }
}

impl Block for Census {
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
        let x = &io.population_inputs[0];
        io.outputs[0] = x.len() as f64;
        io.outputs[1] = x.iter().sum();
        Ok(())
    }
}

fn model(seed: u64) -> (Model, [simsandbox_core::graph::BlockId; 2]) {
    let mut m = Model::new();
    m.set_seed(seed);
    let w = m.add(Box::new(Walkers::new()));
    // Units must match exactly (ADR 0045): centimetres are refused.
    let mut other = Model::new();
    let (w2, cm) = (
        other.add(Box::new(Walkers::new())),
        other.add(Box::new(Census::new(Unit::parse("cm").unwrap(), 1))),
    );
    assert!(matches!(
        other.connect((w2, "x"), (cm, "x")),
        Err(simsandbox_core::SimError::UnitMismatch { .. })
    ));
    let c = m.add(Box::new(Census::new(Unit::METER, 1)));
    m.connect((w, "x"), (c, "x")).unwrap();
    (m, [w, c])
}

/// Positions and ids of the walkers after each of `steps` steps.
fn run(
    sim: &mut Simulator,
    w: simsandbox_core::graph::BlockId,
    steps: usize,
) -> Vec<(Vec<u64>, Vec<f64>)> {
    (0..steps)
        .map(|_| {
            sim.step(1.0).unwrap();
            let t = sim.table(w, "walkers").unwrap();
            (t.ids().to_vec(), t.column(0).to_vec())
        })
        .collect()
}

#[test]
fn populations_grow_shrink_and_reach_other_blocks() {
    let (m, [w, c]) = model(1);
    let mut sim = Simulator::new(m, 0.0).unwrap();
    assert_eq!(sim.output(w, "x").unwrap().len(), 10);
    // As for fixed ports, inputs at t0 are what was exchanged after every
    // Block computed its first outputs: nothing yet.
    assert_eq!(sim.output(c, "n").unwrap(), [0.0]);
    let mut sizes = Vec::new();
    for _ in 0..200 {
        let before = sim.output(w, "x").unwrap().to_vec();
        sim.step(1.0).unwrap();
        // The census sees this step's walkers (exchanged after the step).
        let x = sim.output(w, "x").unwrap();
        assert_eq!(sim.output(c, "n").unwrap()[0], before.len() as f64);
        let sum: f64 = before.iter().sum();
        assert!((sim.output(c, "sum").unwrap()[0] - sum).abs() < 1e-9 * (1.0 + sum.abs()));
        assert_eq!(sim.output(w, "count").unwrap()[0], x.len() as f64);
        sizes.push(x.len());
    }
    // Some leave, one is born each step: the population changes size.
    assert!(sizes.iter().min() < sizes.iter().max(), "{sizes:?}");
    let t = sim.table(w, "walkers").unwrap();
    // Ids are unique and increasing (oldest first), never reused.
    assert!(t.ids().windows(2).all(|p| p[0] < p[1]));
    assert_eq!(*t.ids().last().unwrap(), 10 + 200 - 1);
    assert_eq!(sim.step_count(), 200);
}

#[test]
fn rewind_replays_populations_bit_for_bit() {
    let (m, [w, _]) = model(7);
    let mut sim = Simulator::new(m, 0.0).unwrap();
    run(&mut sim, w, 30);
    let snap = sim.snapshot();
    let ahead = run(&mut sim, w, 50);
    sim.restore(&snap);
    assert_eq!(sim.step_count(), 30);
    assert_eq!(run(&mut sim, w, 50), ahead);

    // Restarting gives a new run: ids from 0 again, the same first draws.
    let (m, [w2, _]) = model(7);
    let mut fresh = Simulator::new(m, 0.0).unwrap();
    let first = run(&mut fresh, w2, 5);
    sim.restart(0.0).unwrap();
    assert_eq!(
        sim.table(w, "walkers").unwrap().ids(),
        (0..10).collect::<Vec<u64>>()
    );
    assert_eq!(run(&mut sim, w, 5), first);
}

#[test]
fn the_seed_decides_the_run() {
    let go = |seed| {
        let (m, [w, _]) = model(seed);
        let mut sim = Simulator::new(m, 0.0).unwrap();
        run(&mut sim, w, 20)
    };
    assert_eq!(go(3), go(3));
    assert_ne!(go(3), go(4));
}

#[test]
fn populations_connect_only_to_populations_of_the_same_dimension() {
    let mut m = Model::new();
    let w = m.add(Box::new(Walkers::new()));
    let c = m.add(Box::new(Census::new(Unit::METER, 2)));
    assert!(matches!(
        m.connect((w, "x"), (c, "x")),
        Err(SimError::ShapeMismatch { .. })
    ));
    assert!(matches!(
        m.connect((w, "count"), (c, "x")),
        Err(SimError::ShapeMismatch { .. })
    ));
}
