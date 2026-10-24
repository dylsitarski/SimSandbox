//! `agents.grazers`: animals on a square field that wander, eat grass,
//! breed and die (the grazing model, ADR 0033, ADR 0034).
//!
//! Each step, in this order:
//!
//! 1. **Move:** a random walk with diffusion D: each coordinate changes by
//!    a normal draw of variance 2 D dt; the field wraps at its edges.
//! 2. **Eat:** each animal wants c G dt of grass (kg), c the area it clears
//!    per second and G the density in its cell at the start of the step.
//!    Animals in one cell compete for its grass, G × cell area, in the
//!    block's update order ([`Update`]).
//! 3. **Energy:** each gains `efficiency` × what it ate and loses
//!    `metabolism` × dt.
//! 4. **Breed:** one with at least `birth_energy` splits its energy with a
//!    newborn in the same place.
//! 5. **Die:** one with no energy left is removed.
//!
//! What was eaten in each cell goes back to the grass as a grazing rate
//! (1/s): eaten / (G × area × dt), the fraction of the cell's grass eaten
//! per second, which the grass applies over the next step.

use crate::update::Update;
use simsandbox_core::block::{
    Block, BlockSpec, Domain, MethodSpec, ParamSpec, PortSpec, Shape, StepContext, StepIo,
    TimeSemantics,
};
use simsandbox_core::random::Draws;
use simsandbox_core::table::{Table, TableSpec};
use simsandbox_core::units::Unit;
use simsandbox_core::Result;

/// Cells along each side of the field; the same as `eq.grass`.
pub const CELLS: usize = 40;

/// Columns of the animals' table.
const X: usize = 0;
const Y: usize = 1;
const ENERGY: usize = 2;

/// The step number used for draws made when the run starts.
const START: u64 = u64::MAX;

/// Draw purposes.
const MOVE_X: u64 = 0;
const MOVE_Y: u64 = 1;
const PLACE_X: u64 = 0;
const PLACE_Y: u64 = 1;
const PLACE_ENERGY: u64 = 2;

/// Parameters, in `BlockSpec::params` order.
struct Params {
    size: f64,
    diffusion: f64,
    clearing: f64,
    efficiency: f64,
    metabolism: f64,
    birth_energy: f64,
}

impl Params {
    fn of(p: &[f64]) -> Self {
        Self {
            size: p[0],
            diffusion: p[1],
            clearing: p[2],
            efficiency: p[3],
            metabolism: p[4],
            birth_energy: p[5],
        }
    }

    fn cell(&self) -> f64 {
        self.size / CELLS as f64
    }

    /// The cell holding a point.
    fn cell_of(&self, x: f64, y: f64) -> usize {
        let c = self.cell();
        let index = |v: f64| ((v / c).floor().max(0.0) as usize).min(CELLS - 1);
        index(y) * CELLS + index(x)
    }
}

fn param(name: &str, unit: Unit, default: f64, range: [f64; 2], description: &str) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        unit,
        default,
        suggested_min: range[0],
        suggested_max: range[1],
        description: description.into(),
        initial_condition: false,
    }
}

pub struct Grazers {
    spec: BlockSpec,
    method: MethodSpec,
    update: Update,
}

impl Grazers {
    pub fn new(name: &str, update: Update) -> Self {
        let grid = Shape::Grid {
            nx: CELLS,
            ny: CELLS,
        };
        let density = Unit::new([-2, 1, 0, 0, 0, 0, 0], 1.0);
        let per_second = Unit::new([0, 0, -1, 0, 0, 0, 0], 1.0);
        let area_rate = Unit::new([2, 0, -1, 0, 0, 0, 0], 1.0);
        let mass_rate = Unit::new([0, 1, -1, 0, 0, 0, 0], 1.0);
        let kg = Unit::KILOGRAM;
        let one = Unit::DIMENSIONLESS;
        let initial = |p: ParamSpec| ParamSpec {
            initial_condition: true,
            ..p
        };
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Agents,
                ports: vec![
                    PortSpec::input("grass", density, grid),
                    PortSpec::output("position", Unit::METER, Shape::Population { dim: 2 }),
                    PortSpec::output("energy", kg, Shape::Population { dim: 1 }),
                    PortSpec::output("count", one, Shape::Scalar),
                    PortSpec::output("mean_energy", kg, Shape::Scalar),
                    PortSpec::output("grazing", per_second, grid),
                ],
                params: vec![
                    param(
                        "size",
                        Unit::METER,
                        40.0,
                        [5.0, 200.0],
                        "Side of the square field (the same as the grass's)",
                    ),
                    param(
                        "D",
                        area_rate,
                        2.0,
                        [0.0, 10.0],
                        "How fast animals wander: the diffusion of their random walk",
                    ),
                    param(
                        "c",
                        area_rate,
                        0.5,
                        [0.0, 5.0],
                        "Area an animal clears of grass per second",
                    ),
                    param(
                        "efficiency",
                        one,
                        0.5,
                        [0.0, 1.0],
                        "Energy gained per kilogram of grass eaten",
                    ),
                    param(
                        "metabolism",
                        mass_rate,
                        0.1,
                        [0.0, 1.0],
                        "Energy spent per second",
                    ),
                    param(
                        "birth_energy",
                        kg,
                        2.0,
                        [0.5, 10.0],
                        "Energy at which an animal splits its energy with a newborn",
                    ),
                    initial(param(
                        "count0",
                        one,
                        30.0,
                        [0.0, 500.0],
                        "Animals at the start, placed at random",
                    )),
                    initial(param(
                        "energy0",
                        kg,
                        1.0,
                        [0.0, 5.0],
                        "Mean energy of an animal at the start",
                    )),
                    initial(param(
                        "energy_spread",
                        one,
                        0.5,
                        [0.0, 1.0],
                        "Starting energies spread evenly over energy0 × (1 ± this), so \
                         animals do not all breed at once",
                    )),
                ],
                states: Vec::new(),
                tables: vec![TableSpec::new("animals", &["x", "y", "energy"])],
                time: TimeSemantics::FixedStep { dt: 0.1 },
            },
            method: crate::rules_method(),
            update,
        }
    }

    pub fn update(&self) -> Update {
        self.update
    }

    /// Outputs from the table: positions, energies, count and mean
    /// energy; `grazing` is written by the step.
    fn write_outputs(t: &Table, population: &mut [Vec<f64>], outputs: &mut [f64]) {
        let n = t.len();
        let (x, y, e) = (t.column(X), t.column(Y), t.column(ENERGY));
        let position = &mut population[0];
        position.clear();
        for r in 0..n {
            position.extend([x[r], y[r]]);
        }
        population[1].clear();
        population[1].extend_from_slice(e);
        outputs[0] = n as f64;
        outputs[1] = if n == 0 {
            0.0
        } else {
            e.iter().sum::<f64>() / n as f64
        };
    }
}

impl Block for Grazers {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }

    fn init_tables(&mut self, params: &[f64], seed: u64, tables: &mut [Table]) -> Result<()> {
        let p = Params::of(params);
        let count = params[6].max(0.0).round() as u64;
        let (energy, spread) = (params[7], params[8].clamp(0.0, 1.0));
        for k in 0..count {
            let d = Draws::new(seed, START, k);
            tables[0].push(&[
                d.unit(PLACE_X) * p.size,
                d.unit(PLACE_Y) * p.size,
                energy * (1.0 + spread * (2.0 * d.unit(PLACE_ENERGY) - 1.0)),
            ]);
        }
        Ok(())
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let p = Params::of(io.params);
        let dt = ctx.dt;
        let area = p.cell() * p.cell();
        let grass: Vec<f64> = io.inputs.iter().map(|g| g.max(0.0)).collect();
        let t = &mut io.tables[0];
        let ids = t.ids().to_vec();
        let n = ids.len();

        // 1. Move, wrapping at the edges.
        let spread = (2.0 * p.diffusion * dt).max(0.0).sqrt();
        let wrap = |v: f64| v.rem_euclid(p.size);
        for (column, purpose) in [(X, MOVE_X), (Y, MOVE_Y)] {
            for (v, &id) in t.column_mut(column).iter_mut().zip(&ids) {
                let step = spread * Draws::new(ctx.seed, ctx.step, id).normal(purpose);
                *v = wrap(*v + step);
            }
        }

        // 2. Eat, competing within cells in the update order.
        let cells: Vec<usize> = (0..n)
            .map(|r| p.cell_of(t.column(X)[r], t.column(Y)[r]))
            .collect();
        let want: Vec<f64> = cells
            .iter()
            .map(|&c| (p.clearing * grass[c] * dt).max(0.0))
            .collect();
        let mut available: Vec<f64> = grass.iter().map(|g| g * area).collect();
        let order = self.update.order(n, Draws::new(ctx.seed, ctx.step, START));
        let eaten = self.update.share(&order, &want, &cells, &mut available);

        // 3. Energy.
        for (e, ate) in t.column_mut(ENERGY).iter_mut().zip(&eaten) {
            *e += p.efficiency * ate - p.metabolism * dt;
        }

        // 4. Breed: split energy with a newborn in the same place.
        let mut born = Vec::new();
        for r in 0..n {
            let e = t.column(ENERGY)[r];
            if e >= p.birth_energy && e > 0.0 {
                t.column_mut(ENERGY)[r] = e / 2.0;
                born.push([t.column(X)[r], t.column(Y)[r], e / 2.0]);
            }
        }
        // 5. Die.
        let energy = t.column(ENERGY).to_vec();
        t.retain(|r| energy[r] > 0.0);
        for b in born {
            t.push(&b);
        }

        // What was eaten goes back to the grass as a grazing rate.
        let mut eaten_in = vec![0.0; CELLS * CELLS];
        for (&c, ate) in cells.iter().zip(&eaten) {
            eaten_in[c] += ate;
        }
        for (k, rate) in io.outputs[2..].iter_mut().enumerate() {
            let there = grass[k] * area * dt;
            *rate = if there > 0.0 {
                eaten_in[k] / there
            } else {
                0.0
            };
        }
        Self::write_outputs(&io.tables[0], io.population_outputs, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        Self::write_outputs(&io.tables[0], io.population_outputs, io.outputs);
        io.outputs[2..].fill(0.0);
        Ok(())
    }
}
