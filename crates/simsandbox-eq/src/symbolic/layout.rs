//! Where every quantity's values are in the programs' registers.

use super::declare::Declared;
use super::shaped::Shaped;
use super::statements::EdgeKind;
use super::units::Units;
use super::Layout;
use simsandbox_expr::program::Span;
use simsandbox_expr::Edge;

/// The registers of a block's programs: what [`registers`] establishes.
/// Each quantity is a variable (a place in `spans`), laid out in this
/// order: t, states, inputs, each table's row ids and corners, parameters,
/// defined quantities, the axes (these two read as outputs), the grid's
/// ends and spacing, edge values, the rules' conditions and values, rates.
pub(super) struct Registers {
    /// Each variable's registers, and the factor from its unit to SI.
    pub(super) spans: Vec<Span>,
    pub(super) scales: Vec<f64>,
    /// How many registers there are.
    pub(super) size: usize,
    /// The variable of each state, input and parameter, in their order.
    pub(super) states: Vec<usize>,
    pub(super) inputs: Vec<usize>,
    pub(super) params: Vec<usize>,
    /// Each table's row ids, given with the inputs (ADR 0079), by axis.
    pub(super) ids: Vec<Option<usize>>,
    /// Each edge, cell or side table's places, given after the ids, by
    /// axis: one per corner, then for sides the inner and outer cells'
    /// places, the boundary flags and the cell across (ADR 0085).
    pub(super) ends: Vec<Option<Vec<usize>>>,
    /// Each defined quantity's variable, and each axis's (the position
    /// of every cell).
    pub(super) defined: Vec<usize>,
    pub(super) axes: Vec<usize>,
    /// For each axis: its positions, low end, high end and spacing.
    pub(super) grid: Vec<(usize, usize, usize, usize)>,
    /// Each edge's value or gradient (none for a periodic one).
    pub(super) edges: Vec<Option<usize>>,
    /// Each rule's condition and the values of the rows it adds, over
    /// the rows it removes or those it adds from (ADR 0079).
    pub(super) rules: Vec<(usize, Vec<usize>)>,
    /// Each state's rate.
    pub(super) rates: Vec<usize>,
    /// Each array parameter: its elements' registers, one after another
    /// (ADR 0069).
    array_params: Vec<usize>,
    pub(super) layout: Layout,
}

impl Registers {
    fn add(&mut self, len: usize, scale: f64) -> usize {
        self.spans.push(Span { at: self.size, len });
        self.scales.push(scale);
        self.size += len;
        self.spans.len() - 1
    }

    /// The variable `n` names where `defined_so_far` of the defined
    /// quantities are above the line.
    pub(super) fn slot_of(
        &self,
        d: &Declared,
        s: &Shaped,
        n: &str,
        defined_so_far: usize,
    ) -> Option<usize> {
        if n == "t" {
            return Some(0);
        }
        if let Some(i) = d.states.iter().position(|x| x == n) {
            return Some(self.states[i]);
        }
        if let Some(i) = d.inputs.iter().position(|x| x == n) {
            return Some(self.inputs[i]);
        }
        if let Some(i) = s.params.iter().position(|p| p == n) {
            return Some(self.params[i]);
        }
        if let Some(i) = d.array_params.iter().position(|a| a.name == n) {
            return Some(self.array_params[i]);
        }
        if let Some(i) = d.grid.iter().position(|a| a.name == n) {
            return Some(self.axes[i]);
        }
        d.defined[..defined_so_far]
            .iter()
            .position(|x| x == n)
            .map(|i| self.defined[i])
    }

    /// An array's edges along an axis, insulated where none is given.
    pub(super) fn edges_of(&self, d: &Declared, array: &str, axis: usize) -> [Edge; 2] {
        let mut out = [Edge::Gradient(None); 2];
        for (e, v) in d.edges.iter().zip(&self.edges) {
            if e.array == array && e.axis == axis {
                out[e.high as usize] = match (&e.kind, v) {
                    (EdgeKind::Periodic, _) => Edge::Periodic,
                    (EdgeKind::Value(_), Some(v)) => Edge::Value(*v),
                    (_, v) => Edge::Gradient(*v),
                };
            }
        }
        out
    }

    /// The variable holding an axis's spacing.
    pub(super) fn spacing_of(&self, axis: usize) -> usize {
        self.grid[axis].3
    }
}

/// Lay every quantity out in the registers.
pub(super) fn registers(d: &Declared, s: &Shaped, units: &Units) -> Registers {
    let grid = &d.grid;
    let len_of = |n: &str| s.arrays.len_of(n);
    let scale_of = |n: &str| units.of(n).scale;
    let mut r = Registers {
        spans: Vec::new(),
        scales: Vec::new(),
        size: 0,
        states: Vec::new(),
        inputs: Vec::new(),
        params: Vec::new(),
        ids: Vec::new(),
        ends: Vec::new(),
        defined: vec![0; d.defined.len()],
        axes: vec![0; grid.len()],
        grid: Vec::new(),
        edges: Vec::new(),
        rules: Vec::new(),
        rates: Vec::new(),
        array_params: Vec::new(),
        layout: Layout {
            states: 0,
            fixed: 0,
            inputs: 0,
            params: 0,
            outputs_at: 0,
            outputs_len: 0,
            rates_at: 0,
        },
    };
    r.add(1, units.time.scale);
    r.states = d
        .states
        .iter()
        .map(|n| r.add(len_of(n), scale_of(n)))
        .collect();
    r.inputs = d
        .inputs
        .iter()
        .map(|n| r.add(len_of(n), scale_of(n)))
        .collect();
    r.ids = grid
        .iter()
        .map(|a| a.table.map(|_| r.add(a.cells, 1.0)))
        .collect();
    r.ends = grid
        .iter()
        .map(|a| {
            let n = a.shape?.corners() + if a.sides_of.is_some() { 4 } else { 0 };
            Some((0..n).map(|_| r.add(a.cells, 1.0)).collect())
        })
        .collect();
    r.params = s.params.iter().map(|n| r.add(1, scale_of(n))).collect();
    let axis_scales: Vec<f64> = grid.iter().map(|a| scale_of(&a.name)).collect();
    // The outputs: defined quantities, then each axis's position of
    // every cell; those over a table's rows after them, not outputs.
    let mut out_vars = Vec::new();
    for rows in [false, true] {
        for (k, n) in d.defined.iter().enumerate() {
            if s.arrays.row_table(n).is_some() == rows {
                r.defined[k] = r.add(len_of(n), scale_of(n));
                if !rows {
                    out_vars.push(r.defined[k]);
                }
            }
        }
        for (i, a) in grid.iter().enumerate() {
            if a.table.is_some() == rows {
                r.axes[i] = r.add(a.cells, axis_scales[i]);
                if !rows {
                    out_vars.push(r.axes[i]);
                }
            }
        }
    }
    let axis_vars = r.axes.clone();
    r.grid = axis_vars
        .iter()
        .zip(&axis_scales)
        .map(|(&x, &scale)| (x, r.add(1, scale), r.add(1, scale), r.add(1, scale)))
        .collect();
    r.edges = d
        .edges
        .iter()
        .map(|e| match e.kind {
            EdgeKind::Periodic => None,
            EdgeKind::Value(_) => Some(r.add(1, scale_of(&e.array))),
            EdgeKind::Gradient(_) => Some(r.add(1, scale_of(&e.array) / axis_scales[e.axis])),
        })
        .collect();
    r.rules = s
        .rules
        .iter()
        .map(|rule| {
            let n = match (&rule.rule.add, rule.source) {
                (None, _) => grid[rule.table].cells,
                (Some(_), Some(src)) => grid[src].cells,
                (Some(_), None) => 1,
            };
            let cond = r.add(n, 1.0);
            let vals = rule
                .values
                .iter()
                .map(|(c, ..)| r.add(n, scale_of(c)))
                .collect();
            (cond, vals)
        })
        .collect();
    r.rates = d
        .states
        .iter()
        .map(|n| r.add(len_of(n), scale_of(n)))
        .collect();
    // An array parameter reads its elements' registers.
    for a in &d.array_params {
        r.spans.push(Span {
            at: r.spans[r.params[a.first]].at,
            len: a.cells,
        });
        r.scales.push(scale_of(&a.name));
        r.array_params.push(r.spans.len() - 1);
    }
    r.layout = Layout {
        states: r.states.iter().map(|&v| r.spans[v].len).sum(),
        fixed: d
            .states
            .iter()
            .zip(&r.states)
            .filter(|(n, _)| s.arrays.row_table(n).is_none())
            .map(|(_, &v)| r.spans[v].len)
            .sum(),
        inputs: r
            .inputs
            .iter()
            .chain(r.ids.iter().flatten())
            .copied()
            .chain(r.ends.iter().flatten().flatten().copied())
            .map(|v| r.spans[v].len)
            .sum(),
        params: s.params.len(),
        outputs_at: out_vars.first().map_or(0, |&v| r.spans[v].at),
        outputs_len: out_vars.iter().map(|&v| r.spans[v].len).sum::<usize>(),
        rates_at: r.rates.first().map_or(r.size, |&v| r.spans[v].at),
    };
    r
}
