//! The Block contract.
//!
//! See `docs/decisions/0002-layered-block-contract.md`. In short:
//!
//! - Every Block implements the required [`Block`] trait, which is enough for
//!   the co-simulation master to step it as a black box.
//! - Optional capability traits ([`Equations`], [`Rollback`], [`Derivatives`],
//!   [`Realtime`]) are exposed through the `as_*` accessors. The kernel checks
//!   them to decide how to schedule a Block, e.g. fusing equation Blocks of the
//!   same domain into one system.
//! - The kernel owns Block state as a flat `[f64]` slice so snapshots, rewind
//!   and saving work the same for every native Block.
//! - Each Block runs one chosen *method*, labelled for speed and accuracy.

use crate::table::{Table, TableSpec};
use crate::units::Unit;
use crate::Result;
use std::time::Duration;

/// How a Block is fused with its neighbours. Blocks in the same domain can be
/// merged into one problem; different domains meet through the master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    Equations,
    RigidBody,
    Agents,
    Fields,
    Ml,
    External,
}

/// Speed class of a method (see the plan's "Method classification").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpeedClass {
    /// Keeps a 30–60 fps loop and accepts live input.
    RealTime,
    /// Seconds to minutes; change a parameter, then rerun.
    Responsive,
    /// Hours or more; runs as a job.
    Batch,
}

/// Accuracy class of a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccuracyClass {
    /// Right behaviour; numbers not to be trusted.
    Qualitative,
    /// Numerically controlled, not yet checked for this case.
    Quantitative,
    /// Error estimated for this case; benchmarks pass.
    Verified,
}

/// The speed and accuracy labels a method carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodLabels {
    pub speed: SpeedClass,
    pub accuracy: AccuracyClass,
}

impl MethodLabels {
    /// Labels of a composite model: as slow as its slowest method and capped
    /// by its least accurate method.
    pub fn combine(labels: impl IntoIterator<Item = MethodLabels>) -> Option<MethodLabels> {
        labels.into_iter().reduce(|a, b| MethodLabels {
            speed: a.speed.max(b.speed),
            accuracy: a.accuracy.min(b.accuracy),
        })
    }
}

/// The method a Block is currently running.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodSpec {
    pub name: String,
    pub labels: MethodLabels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Causality {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Scalar,
    Vector(usize),
    /// A field on a regular grid, `nx` by `ny` values, stored row by row
    /// (`ny` = 1 for a line of cells). Views show it as a whole (ADR 0015).
    Grid {
        nx: usize,
        ny: usize,
    },
    /// `count` points of `dim` coordinates each, stored point by point.
    Points {
        count: usize,
        dim: usize,
    },
    /// Points of `dim` values each whose number changes from step to step,
    /// such as the positions of a population (ADR 0034). Such ports take no
    /// room in the fixed input and output arrays; their values are in
    /// [`StepIo::population_inputs`] and [`StepIo::population_outputs`].
    Population {
        dim: usize,
    },
}

impl Shape {
    pub fn len(&self) -> usize {
        match self {
            Shape::Scalar => 1,
            Shape::Vector(n) => *n,
            Shape::Grid { nx, ny } => nx * ny,
            Shape::Points { count, dim } => count * dim,
            // Not in the fixed arrays.
            Shape::Population { .. } => 0,
        }
    }

    /// Fields and point sets are recorded and shown as a whole, not as one
    /// signal per element.
    pub fn is_field(&self) -> bool {
        matches!(
            self,
            Shape::Grid { .. } | Shape::Points { .. } | Shape::Population { .. }
        )
    }

    /// The number of values changes from step to step.
    pub fn is_variable(&self) -> bool {
        matches!(self, Shape::Population { .. })
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A typed, unit-aware port. Signal (causal) ports only in v0; physical
/// (acausal) ports arrive with the equation-domain compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct PortSpec {
    pub name: String,
    pub causality: Causality,
    pub unit: Unit,
    pub shape: Shape,
    /// Set when this output gives the positions of another output's cells
    /// along one grid axis (ADR 0024), e.g. a rod's `position` for its
    /// `temperature`.
    pub coordinate: Option<CoordinateOf>,
}

/// What an output is a coordinate of: another output of the same Block, of
/// the same shape, along grid axis `axis` (0 = x, 1 = y).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinateOf {
    pub port: String,
    pub axis: usize,
}

impl PortSpec {
    pub fn input(name: &str, unit: Unit, shape: Shape) -> Self {
        Self {
            name: name.into(),
            causality: Causality::Input,
            unit,
            shape,
            coordinate: None,
        }
    }

    /// Declare this output a coordinate of output `port` along grid axis
    /// `axis` (ADR 0024).
    pub fn coordinate_of(mut self, port: &str, axis: usize) -> Self {
        self.coordinate = Some(CoordinateOf {
            port: port.into(),
            axis,
        });
        self
    }

    pub fn output(name: &str, unit: Unit, shape: Shape) -> Self {
        Self {
            name: name.into(),
            causality: Causality::Output,
            unit,
            shape,
            coordinate: None,
        }
    }
}

/// A named, unit-aware parameter. Interfaces generate an entry field for each
/// one, plus an optional slider.
///
/// The suggested range is a hint for the slider's initial range, not a limit:
/// any finite value is accepted (ADR 0011).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    pub unit: Unit,
    pub default: f64,
    pub suggested_min: f64,
    pub suggested_max: f64,
    pub description: String,
    /// The parameter sets initial state, so a change takes effect when the
    /// run restarts (`Simulator::restart`), not while it runs.
    pub initial_condition: bool,
}

/// How a Block advances in time. Continuous time with events is the general
/// case (as in FMI 3.0); the others are special cases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeSemantics {
    Continuous,
    FixedStep { dt: f64 },
    DiscreteEvent,
    Stateless,
}

/// Everything the kernel needs to know about a Block before running it.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockSpec {
    pub name: String,
    pub domain: Domain,
    pub ports: Vec<PortSpec>,
    pub params: Vec<ParamSpec>,
    /// Names of the kernel-owned state variables, in slice order.
    pub states: Vec<String>,
    /// Kernel-owned populations of rows that grow and shrink (ADR 0034).
    pub tables: Vec<TableSpec>,
    pub time: TimeSemantics,
}

impl BlockSpec {
    pub fn inputs(&self) -> impl Iterator<Item = &PortSpec> {
        self.ports
            .iter()
            .filter(|p| p.causality == Causality::Input)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &PortSpec> {
        self.ports
            .iter()
            .filter(|p| p.causality == Causality::Output)
    }

    pub fn input_len(&self) -> usize {
        self.inputs().map(|p| p.shape.len()).sum()
    }

    pub fn output_len(&self) -> usize {
        self.outputs().map(|p| p.shape.len()).sum()
    }

    pub fn default_params(&self) -> Vec<f64> {
        self.params.iter().map(|p| p.default).collect()
    }
}

/// Time context for one step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepContext {
    pub t: f64,
    pub dt: f64,
    /// Steps taken since the run (re)started: 0 for the first.
    pub step: u64,
    /// The Block's random stream ([`crate::random`], ADR 0034).
    pub seed: u64,
}

/// Data for one step. Inputs and outputs are flattened in port order;
/// variable-length ports (`Shape::Population`) have one vector each, in port
/// order among such ports.
pub struct StepIo<'a> {
    pub params: &'a [f64],
    pub inputs: &'a [f64],
    pub state: &'a mut [f64],
    pub outputs: &'a mut [f64],
    /// The Block's tables, in `BlockSpec::tables` order.
    pub tables: &'a mut [Table],
    pub population_inputs: &'a [Vec<f64>],
    pub population_outputs: &'a mut [Vec<f64>],
}

/// The required contract. Enough for the master to step any Block as a black box.
pub trait Block: Send {
    fn spec(&self) -> &BlockSpec;

    /// The method this Block is currently running, with its labels.
    fn method(&self) -> &MethodSpec;

    /// Set initial state from parameters.
    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()>;

    /// Fill the Block's tables, empty when called, with their first rows
    /// (ADR 0034); `seed` is the Block's random stream. Blocks without
    /// tables need not implement it.
    fn init_tables(&mut self, params: &[f64], seed: u64, tables: &mut [Table]) -> Result<()> {
        let _ = (params, seed, tables);
        Ok(())
    }

    /// Advance from `ctx.t` to `ctx.t + ctx.dt`, then write outputs.
    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()>;

    /// Write outputs for the current state without advancing (used at t0).
    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()>;

    fn as_equations(&self) -> Option<&dyn Equations> {
        None
    }
    fn as_rollback(&mut self) -> Option<&mut dyn Rollback> {
        None
    }
    fn as_derivatives(&self) -> Option<&dyn Derivatives> {
        None
    }
    fn as_realtime(&self) -> Option<&dyn Realtime> {
        None
    }
}

/// Capability: the Block can expose its equations, so the equation domain can
/// fuse it with neighbours into one system solved by one integrator.
///
/// v0 restriction: outputs depend on state and parameters only (no direct
/// feedthrough from inputs), which rules out algebraic loops inside a group.
pub trait Equations {
    /// dx/dt = f(t, x, u, p)
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]);
    /// y = g(t, x, p)
    fn outputs(&self, t: f64, x: &[f64], p: &[f64], y: &mut [f64]);
}

/// Capability: cheap undo of a rejected step (for external Blocks whose state
/// the kernel does not own).
pub trait Rollback {
    fn rollback(&mut self) -> Result<()>;
}

/// Capability: Jacobians of the right-hand side, for implicit coupling,
/// sensitivity analysis and parameter fitting.
pub trait Derivatives {
    /// Row-major Jacobian d(dx/dt)/dx, length `n * n`.
    fn jacobian_x(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], jac: &mut [f64]);
}

/// Capability: a guaranteed worst-case step time, which admits the Block into
/// real-time models.
pub trait Realtime {
    fn worst_case_step(&self) -> Duration;
}
