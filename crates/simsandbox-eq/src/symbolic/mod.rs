//! Equations typed as text become an [`OdeSystem`] (ADR 0017).
//!
//! ```text
//! # Lorenz system
//! dx/dt = sigma*(y - x)
//! dy/dt = x*(rho - z) - y
//! dz/dt = x*y - beta*z
//! r = sqrt(x^2 + y^2 + z^2)
//! ```
//!
//! - `dx/dt = …` (or `x' = …`) makes `x` a state that changes continuously.
//! - `x[n+1] = …` makes `x` a state of a discrete map, updated once per step
//!   (ADR 0019); on the right, `x[n]` and `x` both mean the current value.
//!   A block is either continuous or a map. `n` is reserved in maps.
//! - A block with only `name = …` lines is a set of formulas with no state
//!   (ADR 0019), evaluated from its parameters (and `t`).
//! - `name = …` defines a quantity computed from the states, e.g. an
//!   energy; it may use quantities defined above it.
//! - `input u, F` makes `u` and `F` input ports (ADR 0040), set by whatever
//!   is connected to them, instead of parameters. A defined quantity that
//!   uses an input (directly or through quantities above it) depends on it
//!   at the same instant (direct feedthrough); states do not.
//! - Every other name is a parameter (default 1). Each state `x` also gets
//!   an initial-value parameter `x0` (default 1), marked as an initial
//!   condition. `t` is time; `pi` and the function names are reserved.
//! - Every state and defined quantity is an output port of the same name.
//! - Blank lines and lines starting with `#` are ignored.
//! - `state x [m] = 0` declares a state with its unit and initial value
//!   (the default of `x0`); its dynamics are still a `dx/dt` or `x[n+1]`
//!   line (ADR 0049). Declaring is optional.
//! - Units (ADR 0045) go in brackets where a name is introduced:
//!   `state x [m]`, `input F [N], v [m/s]`, `param k [N/m] = 4`, and at
//!   the end of an equation for its left side as written: `E = m*v^2/2
//!   [J]`, `dv/dt = F/m [m/s^2]` (so `v` is in m/s: the rate's unit times
//!   s, or SI when that has no name; ADR 0049). Names without a unit are
//!   dimensionless; `t` is in s. Every equation must balance: terms added share a dimension,
//!   `dx/dt` has `x`'s unit per second, and functions such as `sin` and
//!   `exp` take dimensionless arguments. Values are computed in SI, so
//!   `km` and `m` mix correctly inside a block; ports carry the declared
//!   unit, and connections need equal units (convert with `units.convert`).

mod axes;
mod declare;
mod evaluate;
mod explicit;
mod functions;
mod interface;
mod layout;
mod lower;
mod parse;
mod program;
mod rules;
mod shaped;
mod shapes;
mod starts;
mod statements;
mod tables;
#[cfg(test)]
mod tests;
mod units;

pub use explicit::ExplicitBlock;
pub use functions::{FunctionInfo, DD_AXIS, FUNCTIONS};
pub use starts::StartCsv;
pub use statements::DEFAULT_MAX_ROWS;

use simsandbox_core::block::BlockSpec;
use simsandbox_core::registry::MethodSettings;
use simsandbox_expr::program::Span;
use simsandbox_expr::{Ast, Program};
use std::cell::RefCell;

/// What a set of typed equations describes (ADR 0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `dx/dt = …`: integrated by an ODE method.
    Continuous,
    /// `x[n+1] = …`: iterated once per step (method `iterate`).
    Map,
    /// Only `name = …`: evaluated, no state (method `evaluate`).
    Formulas,
}

impl Kind {
    /// The method a block of this kind uses by default.
    pub fn default_method(self) -> MethodSettings {
        match self {
            Kind::Continuous => MethodSettings::new("rk4").with("substeps", 1.0),
            Kind::Map => MethodSettings::new(ITERATE),
            Kind::Formulas => MethodSettings::new(EVALUATE),
        }
    }
}

/// `name[k]`: the name of one element of an array (a state's cell, a
/// parameter's). Written out by hand, not with `format!`: a grid of 512 ×
/// 512 cells names half a million states each time its block is built,
/// which was over half of what building it cost.
fn element_name(name: &str, k: usize) -> String {
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    let mut rest = k;
    loop {
        at -= 1;
        digits[at] = b'0' + (rest % 10) as u8;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    let mut s = String::with_capacity(name.len() + 2 + digits.len() - at);
    s.push_str(name);
    s.push('[');
    for &d in &digits[at..] {
        s.push(d as char);
    }
    s.push(']');
    s
}

/// Method name of map blocks.
pub const ITERATE: &str = "iterate";
/// Method name of formula blocks.
pub const EVALUATE: &str = "evaluate";

/// A system of typed equations.
pub struct EquationSet {
    kind: Kind,
    /// Whether any equation, edge value or initial value reads `t`.
    reads_time: bool,
    spec: BlockSpec,
    /// Where the values of `t`, the states, inputs and parameters go in
    /// the registers (each run of them is contiguous), and where the
    /// outputs after the states (defined quantities and the grid's axis)
    /// and the rates are read from.
    layout: Layout,
    /// Computes the defined quantities and the grid's axis (ADR 0060's
    /// instruction list).
    outputs: Program,
    /// Computes the defined quantities, then each state's right-hand side
    /// into the registers of its rate.
    rates: Program,
    /// Computes the initial values of array states from the parameters.
    init: Option<Program>,
    /// For each state: its number of values, and the parameter holding
    /// its initial value (scalar states) or `None` (arrays, set by `init`).
    initial: Vec<(usize, Option<usize>)>,
    /// Register buffers of the programs: outputs, rates, initial values.
    scratch: RefCell<[Vec<f64>; 4]>,
    /// What the outputs and rates programs need copied, the rest of their
    /// large arrays read from the states and written to the results where
    /// they are.
    outputs_copy: Copies,
    rates_copy: Copies,
    /// Its tables (ADR 0079), in the order of their declarations.
    tables: Vec<TableRows>,
    /// For each state over rows, in state order after the fixed ones: its
    /// table (place in `tables`) and column.
    row_states: Vec<(usize, usize)>,
    /// Its name and lines, to compile it again for other numbers of rows.
    source: (String, Vec<String>),
    /// Rules changing them between steps, in written order, and the
    /// program computing their conditions and new rows' values.
    rules: Vec<RuleRun>,
    rules_program: Option<Program>,
    /// Each input in declaration order: its values' number as compiled,
    /// and for one over a table's rows, that table (place in `tables`).
    inputs_in: Vec<(usize, Option<usize>)>,
    /// Each output over a table's rows, in port order (ADR 0079 stage 1b):
    /// its table (place in `tables`) and where its values are.
    rows_out: Vec<(usize, RowsSource)>,
}

/// Where an output over a table's rows takes its values from.
#[derive(Debug, Clone, Copy)]
enum RowsSource {
    /// A state over the rows: the table's column.
    Column(usize),
    /// A defined quantity: its registers in the outputs program.
    Registers(Span),
    /// A table of cells (ADR 0086): each row's `n` corners.
    Corners(usize),
}

/// A table of a typed-equations block (ADR 0079): its axis, name, most
/// and starting rows, the rows it was compiled for, and its columns (the
/// states over its rows, in state order).
#[derive(Debug, Clone)]
pub struct TableRows {
    pub axis: usize,
    pub name: String,
    pub max: usize,
    pub start: usize,
    pub rows: usize,
    pub columns: Vec<String>,
    /// A table some input is over follows that input's rows by id (ADR
    /// 0079 stage 1b): the first such input's place among the block's
    /// inputs over rows.
    pub follows: Option<usize>,
    /// An edge table (ADR 0079 stage 2): the table (place in the block's
    /// tables) whose rows its edges join. Its kernel table's columns are
    /// each edge's `from` and `to` ids.
    pub ends_of: Option<usize>,
    /// An edge table starting as a chain of its table's starting rows.
    pub chain: bool,
    /// Cells' or sides' shape (ADR 0085); edges are segments.
    pub shape: Option<crate::mesh::CellShape>,
    /// Sides (ADR 0085): the cells (place in the block's tables) they lie
    /// between. Their kernel table's columns are the corners, then the
    /// inner and outer cells' ids (outer -1 on the boundary).
    pub sides_of: Option<usize>,
    /// Cells starting from a grid or a file (ADR 0085): each cell's
    /// corners as places among its points' starting rows (known when the
    /// block is first compiled).
    pub start_cells: Option<Vec<Vec<usize>>>,
    /// Points made by such a start: their coordinates, one list per
    /// coordinate, and each coordinate's column among its states.
    pub start_points: Option<Vec<Vec<f64>>>,
    pub coords: Vec<usize>,
    /// Cells split or merged by a rule (ADR 0085 step 2): after their
    /// corners they hold `level`, `parent` and `child`, and their parents
    /// are kept in a table of the block's own, after the declared ones
    /// (its place among the block's tables).
    pub parents: Option<usize>,
    /// Cells starting from a file (simplices are labelled for bisection
    /// by their longest edge).
    pub from_file: bool,
    /// Rows starting from a CSV file (ADR 0069 step 3).
    pub start_file: Option<std::sync::Arc<StartCsv>>,
}

impl TableRows {
    /// Its corners per row: 0 unless cells, sides or edges.
    pub fn corners(&self) -> usize {
        self.shape.map_or(0, |s| s.corners())
    }

    /// The kernel table's columns before its states: corners, and for
    /// sides their two cells.
    pub fn offset(&self) -> usize {
        self.corners()
            + if self.sides_of.is_some() { 2 } else { 0 }
            + if self.parents.is_some() { 3 } else { 0 }
    }

    /// The place arrays given with the ids: one per corner, and for sides
    /// the inner and outer cells' places, whether on the boundary, and the
    /// cell across (the outer, or the inner on the boundary).
    fn places(&self) -> usize {
        self.corners() + if self.sides_of.is_some() { 4 } else { 0 }
    }
}

/// A rule as compiled: its table (by axis), whether it removes (else
/// adds), and where its condition and new rows' values are in the
/// registers (one value per column, in column order).
#[derive(Debug, Clone)]
pub struct RuleRun {
    pub table: usize,
    pub kind: RuleKind,
    /// `split` and `merge`: the edge table (by axis) whose edges they act
    /// on; the values are computed over its rows.
    pub edges: Option<usize>,
    pub cond: Span,
    pub values: Vec<Span>,
    /// `split` and `merge` of cells (ADR 0085 step 2): for each state over
    /// the points (false) and over the cells (true), by column, how its
    /// value passes on; a given value is one of `values`.
    pub transfers: Vec<(bool, usize, CellTransfer)>,
}

/// How a value passes to the rows a refinement makes (ADR 0085 point 5):
/// see [`crate::refine::Transfer`]; `Given` is a place in the rule's values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellTransfer {
    Copy,
    Share,
    Mean,
    Sum,
    Given(usize),
}

/// What a rule does (ADR 0079).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// `remove pts where c`: rows go (and the edges they end).
    Remove,
    /// `add pts [from k] [where c]: …`: new rows.
    Add,
    /// `split segs where c: …` (stage 2): a new row in the middle of
    /// each edge where `c` holds, the edge replaced by two.
    Split,
    /// `merge segs where c: …` (stage 2): each edge's two rows become one
    /// (its `from` row, with the values given), the edge removed.
    Merge,
}

/// The parts of the states a program needs copied into its registers and
/// of its results it needs copied out (offsets from the first of each);
/// the others it reads and writes in place ([`Program::place_outside`]).
#[derive(Debug, Clone)]
struct Copies {
    /// Whether any array was placed; if not, everything is copied.
    placed: bool,
    states: Vec<std::ops::Range<usize>>,
    results: Vec<std::ops::Range<usize>>,
}

impl Copies {
    /// Nothing placed: everything copied.
    fn none() -> Self {
        Self {
            placed: false,
            states: Vec::new(),
            results: Vec::new(),
        }
    }

    /// Place what `program` can in place: it reads the `states` registers
    /// from 1, and its `len` results from `at`.
    fn place(program: &mut Program, states: usize, at: usize, len: usize) -> Self {
        let (read, written) = program.place_outside(1..1 + states, at..at + len);
        Self {
            placed: !read.is_empty() || !written.is_empty(),
            states: rest(states, &read, 1),
            results: rest(len, &written, at),
        }
    }
}

/// The ranges of `0..len` outside `spans` (registers from `from`).
fn rest(len: usize, spans: &[Span], from: usize) -> Vec<std::ops::Range<usize>> {
    let mut taken: Vec<(usize, usize)> = spans.iter().map(|s| (s.at - from, s.len)).collect();
    taken.sort_unstable();
    let mut out = Vec::new();
    let mut next = 0;
    for (at, n) in taken {
        if at > next {
            out.push(next..at);
        }
        next = at + n;
    }
    if next < len {
        out.push(next..len);
    }
    out
}

/// Register positions of an [`EquationSet`]: `t` at 0, then the state
/// values, input values and parameters, each contiguous.
#[derive(Debug, Clone, Copy)]
struct Layout {
    states: usize,
    /// The states the block keeps in its fixed state, first; those over a
    /// table's rows (ADR 0079) follow, kept in its tables.
    fixed: usize,
    inputs: usize,
    params: usize,
    /// The outputs after the states: from here, this many registers.
    outputs_at: usize,
    outputs_len: usize,
    /// The rates of the state values.
    rates_at: usize,
}

/// A grid axis (ADR 0060): `grid x [m] = a .. b, n`.
#[derive(Clone)]
struct Axis {
    name: String,
    line: usize,
    lo: Ast,
    hi: Ast,
    cells: usize,
    /// A table's rows (ADR 0079): its most rows. Its length is the
    /// current number of rows, and changes between steps.
    table: Option<usize>,
    /// An edge table (ADR 0079 stage 2), cells or sides (ADR 0085): the
    /// table of points whose rows its corners are, by place among the axes.
    ends_of: Option<usize>,
    /// Cells' or sides' shape (edges are segments).
    shape: Option<crate::mesh::CellShape>,
    /// Sides (ADR 0085): the cells they lie between, by place.
    sides_of: Option<usize>,
}
