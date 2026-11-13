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
    /// A value (or `dim` values) per row of a table whose rows change
    /// between steps (ADR 0079): its frames carry the rows' ids and the
    /// table's structure version with the values ([`crate::table::RowsFrame`]).
    /// Like a population it takes no room in the fixed arrays.
    Rows {
        dim: usize,
    },
    /// Values over three or more axes, `dims` cells along each (the first
    /// varying fastest), up to [`MAX_PORT_AXES`] (ADR 0081). Fewer axes
    /// are a [`Shape::Grid`]: [`Shape::of_axes`] gives the canonical one.
    Tensor {
        rank: usize,
        dims: [usize; MAX_PORT_AXES],
    },
}

/// The most axes a port has (ADR 0081).
pub const MAX_PORT_AXES: usize = 8;

/// The length of one axis of a shape (ADR 0081): a number of cells, or a
/// table's rows, whose number changes between steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisLen {
    Cells(usize),
    Rows,
}

impl Shape {
    /// The canonical shape over axes of these lengths, the first varying
    /// fastest (ADR 0081): none is a scalar, one or two a grid (a line
    /// when one), more a tensor.
    ///
    /// # Panics
    /// With more than [`MAX_PORT_AXES`] axes.
    pub fn of_axes(dims: &[usize]) -> Self {
        match *dims {
            [] => Shape::Scalar,
            [nx] => Shape::Grid { nx, ny: 1 },
            [nx, ny] => Shape::Grid { nx, ny },
            _ => {
                assert!(dims.len() <= MAX_PORT_AXES, "at most {MAX_PORT_AXES} axes");
                let mut d = [1; MAX_PORT_AXES];
                d[..dims.len()].copy_from_slice(dims);
                Shape::Tensor {
                    rank: dims.len(),
                    dims: d,
                }
            }
        }
    }

    /// Its axes, the first varying fastest (ADR 0081): a vector's
    /// components; a grid's x then y (one axis for a line); points' values
    /// then the points; a population's values then its rows.
    pub fn axes(&self) -> Vec<AxisLen> {
        use AxisLen::{Cells, Rows};
        match *self {
            Shape::Scalar => Vec::new(),
            Shape::Vector(n) => vec![Cells(n)],
            Shape::Grid { nx, ny: 1 } => vec![Cells(nx)],
            Shape::Grid { nx, ny } => vec![Cells(nx), Cells(ny)],
            Shape::Points { count, dim } => vec![Cells(dim), Cells(count)],
            Shape::Population { dim } => vec![Cells(dim), Rows],
            Shape::Rows { dim: 1 } => vec![Rows],
            Shape::Rows { dim } => vec![Cells(dim), Rows],
            Shape::Tensor { rank, dims } => dims[..rank].iter().map(|&n| Cells(n)).collect(),
        }
    }

    /// The axes' lengths in cells, for a shape of fixed size.
    pub fn dims(&self) -> Option<Vec<usize>> {
        self.axes()
            .into_iter()
            .map(|a| match a {
                AxisLen::Cells(n) => Some(n),
                AxisLen::Rows => None,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        match self {
            Shape::Scalar => 1,
            Shape::Vector(n) => *n,
            Shape::Grid { nx, ny } => nx * ny,
            Shape::Points { count, dim } => count * dim,
            Shape::Tensor { rank, dims } => dims[..*rank].iter().product(),
            // Not in the fixed arrays.
            Shape::Population { .. } | Shape::Rows { .. } => 0,
        }
    }

    /// Fields and point sets are recorded and shown as a whole, not as one
    /// signal per element.
    pub fn is_field(&self) -> bool {
        matches!(
            self,
            Shape::Grid { .. }
                | Shape::Points { .. }
                | Shape::Population { .. }
                | Shape::Rows { .. }
                | Shape::Tensor { .. }
        )
    }

    /// The number of values changes from step to step.
    pub fn is_variable(&self) -> bool {
        matches!(self, Shape::Population { .. } | Shape::Rows { .. })
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A shape for messages: `scalar`, or its axes' lengths (`3 × 4`, `rows`),
/// with what kind of shape it is.
pub fn describe_shape(shape: &Shape) -> String {
    let axes: Vec<String> = shape
        .axes()
        .iter()
        .map(|a| match a {
            AxisLen::Cells(n) => n.to_string(),
            AxisLen::Rows => "rows".into(),
        })
        .collect();
    let kind = match shape {
        Shape::Scalar => return "scalar".into(),
        Shape::Vector(_) => "vector",
        Shape::Grid { ny: 1, .. } => "line",
        Shape::Grid { .. } => "grid",
        Shape::Points { .. } => "points",
        Shape::Population { .. } => "population",
        Shape::Rows { .. } => "rows",
        Shape::Tensor { .. } => "axes",
    };
    format!("{kind} {}", axes.join(" × "))
}

/// A typed, unit-aware port. Signal (causal) ports only in v0; physical
/// (acausal) ports arrive with the equation-domain compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct PortSpec {
    pub name: String,
    pub causality: Causality,
    pub unit: Unit,
    pub shape: Shape,
    /// Its axes' names, in [`Shape::axes`] order (ADR 0081), as its Block
    /// calls them (`x`, `feature`, `run`); empty when the Block does not
    /// name them. Names belong to the Block: wires do not compare them.
    pub axis_names: Vec<String>,
    /// The outputs whose cells this output gives the positions of, along
    /// one grid axis (ADR 0024), e.g. a rod's `position` for its
    /// `temperature`; an axis of typed equations (ADR 0060) for every
    /// array on its grid.
    pub coordinate: Vec<CoordinateOf>,
    /// For an output: the inputs of its Block it depends on at the same
    /// instant (direct feedthrough, ADR 0040). Empty if it depends on state
    /// and parameters only.
    pub feedthrough: Vec<String>,
    /// For an output: the parameters it depends on at the same instant, so
    /// a parameter driven by a wire feeds through to it (ADR 0075).
    /// `None` when the Block does not say: then every parameter, unless the
    /// output copies the state (`state`).
    pub param_feedthrough: Option<Vec<String>>,
    /// For an output: its values are the Block's state from this offset,
    /// exactly, so rewind snapshots need not keep them (ADR 0063). The
    /// kernel checks it at every snapshot and keeps them if not.
    pub state: Option<usize>,
    /// For an output of a mesh's cells (ADR 0085, ADR 0086): how to draw
    /// it. Its rows are the cells, each its corners as ids of its points.
    pub mesh: Option<MeshOf>,
}

/// How a mesh's cells are drawn (ADR 0086): structure only, no geometry
/// beyond the corners' order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshOf {
    /// The cells' shape as written (`triangle`, `hexa`, `box 3`).
    pub shape: String,
    /// 2: each cell drawn filled; 3: the mesh's outer surface.
    pub dim: usize,
    /// The polygons drawn of each cell, as places among its corners in
    /// order around: the cell itself in 2D, each of its sides in 3D.
    pub faces: Vec<Vec<usize>>,
    /// The outputs of the same Block giving its points' coordinates, one
    /// per dimension, each over the points' rows.
    pub coordinates: Vec<String>,
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
            axis_names: Vec::new(),
            coordinate: Vec::new(),
            feedthrough: Vec::new(),
            param_feedthrough: None,
            state: None,
            mesh: None,
        }
    }

    /// Name its axes, in [`Shape::axes`] order (ADR 0081).
    pub fn axes_named(mut self, names: &[&str]) -> Self {
        self.axis_names = names.iter().map(|n| n.to_string()).collect();
        self
    }

    /// Declare this output a mesh's cells (ADR 0086).
    pub fn mesh_of(mut self, mesh: MeshOf) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// Declare that this output depends on these inputs at the same
    /// instant (direct feedthrough, ADR 0040).
    pub fn depends_on(mut self, inputs: &[&str]) -> Self {
        self.feedthrough = inputs.iter().map(|i| i.to_string()).collect();
        self
    }

    /// Declare the parameters this output depends on at the same instant
    /// (ADR 0075); without it, every parameter but for state copies.
    pub fn depends_on_params(mut self, params: &[&str]) -> Self {
        self.param_feedthrough = Some(params.iter().map(|p| p.to_string()).collect());
        self
    }

    /// Whether this output depends on parameter `name` at the same instant:
    /// as declared, or, undeclared, unless it copies the state (ADR 0075).
    pub fn reads_param(&self, name: &str) -> bool {
        match &self.param_feedthrough {
            Some(list) => list.iter().any(|p| p == name),
            None => self.state.is_none(),
        }
    }

    /// Declare this output a coordinate of output `port` along grid axis
    /// `axis` (ADR 0024); call again for each output it is a coordinate of.
    pub fn coordinate_of(mut self, port: &str, axis: usize) -> Self {
        self.coordinate.push(CoordinateOf {
            port: port.into(),
            axis,
        });
        self
    }

    /// An output whose values are the Block's state from `offset`.
    pub fn from_state(mut self, offset: usize) -> Self {
        self.state = Some(offset);
        self
    }

    pub fn output(name: &str, unit: Unit, shape: Shape) -> Self {
        Self {
            name: name.into(),
            causality: Causality::Output,
            unit,
            shape,
            axis_names: Vec::new(),
            coordinate: Vec::new(),
            feedthrough: Vec::new(),
            param_feedthrough: None,
            state: None,
            mesh: None,
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

    /// Parameter values whose initial state (`init`) is exactly `state`,
    /// if the Block's initial-condition parameters can express it; `None`
    /// otherwise, and then a start state is saved instead (ADR 0042).
    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        let _ = (state, params);
        None
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

    /// The times its outputs change at, known before the run, such as the
    /// sample times of recorded data (ADR 0067 step 4): a model with no
    /// states is evaluated there. `None` for most blocks.
    fn instants(&self) -> Option<&[f64]> {
        None
    }

    /// Whether its outputs or next state depend on the time itself, beyond
    /// its state, inputs and parameters (ADR 0083): `Some(false)` for a
    /// block that never reads `t`, so a model of such blocks with only
    /// maps iterates instead of running in time. `None`: not said, as for
    /// most blocks.
    fn reads_time(&self) -> Option<bool> {
        None
    }

    /// How its equations run, for blocks whose code SimSandbox makes from
    /// them (typed equations: `interpreted`, `compiled`, `native`; ADR
    /// 0084, 0089). `None` for blocks written in Rust.
    fn code(&self) -> Option<String> {
        None
    }
}

/// Capability: the Block can expose its equations, so the equation domain can
/// fuse it with neighbours into one system solved by one integrator.
///
/// Outputs may depend on the inputs `u` only where their ports declare it
/// (`PortSpec::feedthrough`, ADR 0040); the kernel orders such outputs and
/// solves loops between them.
pub trait Equations {
    /// dx/dt = f(t, x, u, p), every element of `dx` written: integrators
    /// reuse their arrays between steps.
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]);
    /// y = g(t, x, u, p)
    fn outputs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], y: &mut [f64]);
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
