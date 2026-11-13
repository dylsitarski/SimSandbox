//! Verification of `agents.grazers` (ADR 0034, verification policy): each
//! rule against a known answer, reproducibility, and the grazing model's
//! mass balance with the grass ODE.

use simsandbox_agents::grazers::CELLS;
use simsandbox_agents::{Grazers, Update};
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, ParamSpec, PortSpec, Shape,
    SpeedClass, StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::Result;
use simsandbox_eq::fields::Grass;
use simsandbox_eq::{Integrator, OdeBlock};

/// Grass that never changes: every cell at the parameter `G`.
struct ConstantGrass {
    spec: BlockSpec,
    method: MethodSpec,
}

impl ConstantGrass {
    fn new() -> Self {
        Self {
            spec: BlockSpec {
                name: "field".into(),
                domain: Domain::External,
                ports: vec![PortSpec::output(
                    "grass",
                    Unit::new([-2, 1, 0, 0, 0, 0, 0], 1.0),
                    Shape::Grid {
                        nx: CELLS,
                        ny: CELLS,
                    },
                )],
                params: vec![ParamSpec {
                    name: "G".into(),
                    unit: Unit::new([-2, 1, 0, 0, 0, 0, 0], 1.0),
                    default: 0.0,
                    suggested_min: 0.0,
                    suggested_max: 1.0,
                    description: "Grass density everywhere".into(),
                    initial_condition: false,
                }],
                states: vec![],
                tables: vec![],
                time: TimeSemantics::Stateless,
            },
            method: MethodSpec {
                name: "constant".into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Quantitative,
                },
            },
        }
    }
}

impl Block for ConstantGrass {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }
    fn method(&self) -> &MethodSpec {
        &self.method
    }
    fn init(&mut self, _: &[f64], _: &mut [f64]) -> Result<()> {
        Ok(())
    }
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t, io)
    }
    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs.fill(io.params[0]);
        Ok(())
    }
}

/// Grazers on constant grass `g`, with parameters set by name.
fn on_constant(g: f64, update: Update, params: &[(&str, f64)]) -> (Simulator, BlockId) {
    let mut m = Model::new();
    let field = m.add(Box::new(ConstantGrass::new()));
    let sheep = m.add(Box::new(Grazers::new("sheep", update)));
    m.set_param(field, "G", g).unwrap();
    for &(name, v) in params {
        m.set_param(sheep, name, v).unwrap();
    }
    m.connect((field, "grass"), (sheep, "grass")).unwrap();
    (Simulator::new(m, 0.0).unwrap(), sheep)
}

/// Rules switched off unless set: no wandering, eating, spending or births;
/// every animal starts with exactly `energy0`.
const STILL: [(&str, f64); 6] = [
    ("energy_spread", 0.0),
    ("D", 0.0),
    ("c", 0.0),
    ("metabolism", 0.0),
    ("birth_energy", 1e9),
    ("efficiency", 1.0),
];

fn with(extra: &[(&'static str, f64)]) -> Vec<(&'static str, f64)> {
    let mut v: Vec<_> = STILL
        .iter()
        .filter(|(n, _)| !extra.iter().any(|(e, _)| e == n))
        .copied()
        .collect();
    v.extend_from_slice(extra);
    v
}

fn count(sim: &Simulator, id: BlockId) -> usize {
    sim.table(id, "animals").unwrap().len()
}

#[test]
fn wandering_spreads_as_a_random_walk() {
    // On a huge field nothing wraps: the mean squared displacement after
    // t is 4 D t.
    let (d, dt, steps, n) = (1.0, 0.1, 100, 400.0);
    let params = with(&[("D", d), ("size", 1e7), ("count0", n)]);
    let (mut sim, id) = on_constant(0.0, Update::RandomOrder, &params);
    let start = sim.table(id, "animals").unwrap().clone();
    for _ in 0..steps {
        sim.step(dt).unwrap();
    }
    let end = sim.table(id, "animals").unwrap();
    assert_eq!(end.ids(), start.ids());
    let msd: f64 = (0..end.len())
        .map(|r| {
            let dx = end.column(0)[r] - start.column(0)[r];
            let dy = end.column(1)[r] - start.column(1)[r];
            dx * dx + dy * dy
        })
        .sum::<f64>()
        / n;
    let t = steps as f64 * dt;
    let expected = 4.0 * d * t;
    // dx² + dy² has standard deviation 2·(2 D t) = 40 here, so the mean of
    // 400 has a standard error of 2; allow four of them.
    assert!((msd - expected).abs() < 8.0, "{msd} vs {expected}");
}

#[test]
fn well_fed_animals_double_on_schedule_and_starving_ones_die_on_time() {
    // Plenty of grass (cells of 100 m hold far more than they eat): each
    // gains efficiency × c × G = 1 kg/s, from 1 kg to the birth energy 2
    // in exactly 8 steps of 0.125 s, then splits; so the population
    // doubles every 8 steps.
    let params = with(&[
        ("size", 4000.0),
        ("c", 1.0),
        ("birth_energy", 2.0),
        ("count0", 3.0),
        ("energy0", 1.0),
    ]);
    let (mut sim, id) = on_constant(1.0, Update::RandomOrder, &params);
    for k in 1..=40 {
        sim.step(0.125).unwrap();
        assert_eq!(count(&sim, id), 3 << (k / 8), "after {k} steps");
    }
    assert_eq!(sim.output(id, "count").unwrap(), [96.0]);
    assert_eq!(sim.output(id, "position").unwrap().len(), 2 * 96);

    // No grass: spending 0.25 kg/s from 1 kg, they last exactly 4 s.
    let params = with(&[("metabolism", 0.25), ("count0", 5.0), ("energy0", 1.0)]);
    let (mut sim, id) = on_constant(0.0, Update::RandomOrder, &params);
    for _ in 0..31 {
        sim.step(0.125).unwrap();
    }
    assert_eq!(count(&sim, id), 5);
    let e = sim.output(id, "mean_energy").unwrap()[0];
    assert!((e - 0.03125).abs() < 1e-15, "{e}");
    sim.step(0.125).unwrap();
    assert_eq!(count(&sim, id), 0);
    assert_eq!(sim.output(id, "mean_energy").unwrap(), [0.0]);
}

#[test]
fn the_same_seed_gives_the_same_run_and_rewind_replays_it() {
    // Hungry and crowded, so animals sharing a cell compete for it; no
    // births, so the run stays small.
    let params = [("count0", 800.0), ("c", 50.0), ("birth_energy", 1e9)];
    let run = |seed: u64, update: Update| {
        let mut m = Model::new();
        m.set_seed(seed);
        let field = m.add(Box::new(ConstantGrass::new()));
        let sheep = m.add(Box::new(Grazers::new("sheep", update)));
        m.set_param(field, "G", 0.6).unwrap();
        for &(n, v) in &params {
            m.set_param(sheep, n, v).unwrap();
        }
        m.connect((field, "grass"), (sheep, "grass")).unwrap();
        let mut sim = Simulator::new(m, 0.0).unwrap();
        for _ in 0..30 {
            sim.step(0.1).unwrap();
        }
        let snap = sim.snapshot();
        let after = |sim: &mut Simulator| {
            for _ in 0..20 {
                sim.step(0.1).unwrap();
            }
            sim.table(sheep, "animals").unwrap().clone()
        };
        let a = after(&mut sim);
        sim.restore(&snap);
        assert_eq!(after(&mut sim), a, "rewind replays");
        a
    };
    let a = run(1, Update::RandomOrder);
    assert_eq!(a.len(), 800);
    assert_eq!(a, run(1, Update::RandomOrder));
    assert_ne!(a, run(2, Update::RandomOrder));
    // The update order is part of the model: it changes the run.
    assert_ne!(a, run(1, Update::Synchronous));
}

/// Total grass lost and total energy gained when static animals graze a
/// grass field that does not regrow, over `t` seconds in steps of `dt`.
fn grazed(dt: f64, t: f64) -> (f64, f64) {
    let mut m = Model::new();
    let grass = m.add(Box::new(OdeBlock::new(
        Grass::new("grass"),
        Integrator::Rk4 { substeps: 1 },
    )));
    let sheep = m.add(Box::new(Grazers::new("sheep", Update::Synchronous)));
    m.set_param(grass, "r", 0.0).unwrap();
    m.set_param(grass, "g0", 1.0).unwrap();
    for (n, v) in with(&[("c", 0.2), ("count0", 200.0), ("energy0", 1.0)]) {
        m.set_param(sheep, n, v).unwrap();
    }
    m.connect((grass, "grass"), (sheep, "grass")).unwrap();
    m.connect((sheep, "grazing"), (grass, "grazing")).unwrap();
    let mut sim = Simulator::new(m, 0.0).unwrap();
    let total = |sim: &Simulator| sim.output(grass, "total").unwrap()[0];
    let energy = |sim: &Simulator| sim.output(sheep, "energy").unwrap().iter().sum::<f64>();
    let (g0, e0) = (total(&sim), energy(&sim));
    for _ in 0..(t / dt).round() as usize {
        sim.step(dt).unwrap();
    }
    (g0 - total(&sim), energy(&sim) - e0)
}

#[test]
fn grass_eaten_matches_energy_gained_to_first_order_in_the_step() {
    // The grass applies what was eaten over the next step, so the books
    // balance up to the one-step coupling lag: the mismatch at a fixed
    // time halves with the step.
    let mismatch = |dt: f64| {
        let (lost, gained) = grazed(dt, 4.0);
        assert!(lost > 0.0 && gained > 0.0);
        ((lost - gained) / gained).abs()
    };
    let (a, b, c) = (mismatch(0.2), mismatch(0.1), mismatch(0.05));
    assert!(a < 0.2, "{a}");
    assert!((1.6..2.4).contains(&(a / b)), "{a} {b}");
    assert!((1.6..2.4).contains(&(b / c)), "{b} {c}");
}

#[test]
fn the_grazing_model_settles_where_grass_just_feeds_the_flock() {
    // A sheep breaks even where efficiency × c × G = metabolism, so over a
    // long run the grass density averages about G* = 0.1 / (0.5 × 0.5) =
    // 0.4 kg/m² (the example's parameters; animals spread over the field
    // see roughly the mean density).
    let mut m = Model::new();
    let grass = m.add(Box::new(OdeBlock::new(
        Grass::new("grass"),
        Integrator::Rk4 { substeps: 1 },
    )));
    let sheep = m.add(Box::new(Grazers::new("sheep", Update::RandomOrder)));
    m.set_param(grass, "g0", 1.0).unwrap();
    m.connect((grass, "grass"), (sheep, "grass")).unwrap();
    m.connect((sheep, "grazing"), (grass, "grazing")).unwrap();
    let mut sim = Simulator::new(m, 0.0).unwrap();
    let area = 40.0 * 40.0;
    let (mut sum, mut n) = (0.0, 0);
    for k in 0..4000 {
        sim.step(0.1).unwrap();
        if k >= 2000 {
            sum += sim.output(grass, "total").unwrap()[0] / area;
            n += 1;
        }
    }
    let mean = sum / n as f64;
    assert!((mean - 0.4).abs() < 0.06, "mean grass density {mean}");
    assert!(sim.output(sheep, "count").unwrap()[0] > 100.0);
}
