//! The programs of a block: each expression lowered to instructions on
//! the registers (ADR 0060, ADR 0080), then the lists that compute the
//! outputs, the rates, the initial values and the rules.

use super::declare::Declared;
use super::layout::Registers;
use super::lower::lower;
use super::shaped::Shaped;
use super::shapes::{ast_name, fit, line_of, needed, number};
use super::statements::{definition_error, EdgeKind, Lhs};
use super::{Copies, RuleKind, RuleRun};
use simsandbox_core::Result;
use simsandbox_expr::units::{from_si, in_si};
use simsandbox_expr::{ArrayOp, Ast, Expr, Line as GridLine, Op, Program};
use std::cell::RefCell;

/// A value computed into a variable.
type Step = (usize, Expr);

/// Every expression of a definition, lowered: what [`steps`]
/// establishes. Each is in the unit of the variable it is stored in.
pub(super) struct Steps {
    /// The defined quantities, in written order.
    defined: Vec<Expr>,
    /// Each state's right-hand side, in state order.
    rates: Vec<Expr>,
    /// The grid: each axis's ends, spacing and cell centres.
    grid: Vec<Step>,
    edges: Vec<Step>,
    /// Array states' initial values.
    init: Vec<Step>,
    /// Rules' conditions and the values of new rows (ADR 0079), from
    /// the values after a step.
    rules: Vec<Step>,
}

/// What lowering an expression needs.
struct Lowering<'a> {
    d: &'a Declared,
    s: &'a Shaped<'a>,
    r: &'a Registers,
}

impl Lowering<'_> {
    /// The stencils, along an array's own axes (ADR 0080). Their
    /// arguments are checked by [`super::shapes::axes_of`] first.
    fn stencil(&self, name: &str, args: &[Ast]) -> Expr {
        let (d, r) = (self.d, self.r);
        let grid = &d.grid;
        let first_name = ast_name(&args[0]);
        let axis_named = |n: &str| {
            grid.iter()
                .position(|a| a.name == n)
                .expect("checked: an axis")
        };
        if name == "spacing" {
            return Expr::Var(r.spacing_of(axis_named(first_name)));
        }
        let own = self.s.arrays.of(first_name);
        let a = r
            .slot_of(d, self.s, first_name, d.defined.len())
            .expect("checked: an array");
        // A derivative of an array along one axis.
        let along = |axis: usize, second: bool| {
            let (line, edges) = (line_of(&own, axis, grid), r.edges_of(d, first_name, axis));
            let op = if second {
                ArrayOp::Second { line, edges }
            } else {
                ArrayOp::First { line, edges }
            };
            Expr::Array(op, vec![Expr::Var(a), Expr::Var(r.spacing_of(axis))])
        };
        if name == "lap" {
            // The sum of the second derivatives along each axis.
            return own[1..].iter().fold(along(own[0], true), |sum, &i| {
                Expr::Bin(Op::Add, Box::new(sum), Box::new(along(i, true)))
            });
        }
        if name == "prev" || name == "next" {
            let axis = match args.get(1) {
                Some(Ast::Name(n, _)) => axis_named(n),
                _ => own[0],
            };
            let op = ArrayOp::Shift {
                line: line_of(&own, axis, grid),
                edges: r.edges_of(d, first_name, axis),
                up: name == "next",
            };
            return Expr::Array(op, vec![Expr::Var(a), Expr::Var(r.spacing_of(axis))]);
        }
        let axis = if name == "dd" {
            ast_name(&args[1])
        } else {
            &name[2..]
        };
        along(axis_named(axis), false)
    }

    /// An expression with its axes, where `visible` of the defined
    /// quantities are above it; spread over `to` when given.
    fn resolve(
        &self,
        ast: &Ast,
        visible: usize,
        to: Option<&[usize]>,
        line: usize,
        offset: usize,
    ) -> Result<Expr> {
        let (d, s, r) = (self.d, self.s, self.r);
        let (e, from) = lower(
            ast,
            &d.grid,
            &s.arrays.shapes,
            &|n| r.slot_of(d, s, n, visible),
            &|name, args| self.stencil(name, args),
            &r.ids,
            &r.ends,
        )
        .map_err(|e| {
            let hint = if d
                .defined
                .iter()
                .any(|x| e.message == format!("unknown name '{x}'"))
            {
                " (it is defined further down; define it above this line)"
            } else {
                ""
            };
            definition_error(line, offset + e.column, format!("{}{hint}", e.message))
        })?;
        Ok(match to {
            Some(to) => fit(e, &from, to, None, &d.grid),
            None => e,
        })
    }

    /// In the unit of the variable it is stored in.
    fn stored(&self, e: Expr, var: usize) -> Expr {
        from_si(in_si(e, &self.r.scales), self.r.scales[var])
    }

    /// An expression that may be a plain number, which is then in the
    /// unit of its variable: `0 .. 4` in cm, `state x [m] = 0`.
    fn value(
        &self,
        ast: &Ast,
        to: Option<&[usize]>,
        line: usize,
        offset: usize,
        var: usize,
    ) -> Result<Expr> {
        Ok(match number(ast) {
            Some(v) => Expr::Num(v),
            None => self.stored(self.resolve(ast, 0, to, line, offset)?, var),
        })
    }

    /// The grid: its ends, spacing and cell centres.
    fn grid_steps(&self) -> Result<Vec<Step>> {
        let mut steps = Vec::new();
        for (a, &(x, lo, hi, h)) in self.d.grid.iter().zip(&self.r.grid) {
            for (end, var) in [(&a.lo, lo), (&a.hi, hi)] {
                steps.push((var, self.value(end, None, a.line, 0, var)?));
            }
            let width = Expr::Bin(Op::Sub, Box::new(Expr::Var(hi)), Box::new(Expr::Var(lo)));
            steps.push((
                h,
                Expr::Bin(
                    Op::Div,
                    Box::new(width),
                    Box::new(Expr::Num(a.cells as f64)),
                ),
            ));
            steps.push((
                x,
                // Over the axis alone (ADR 0080).
                Expr::Array(
                    ArrayOp::Axis {
                        line: GridLine {
                            stride: 1,
                            count: a.cells,
                        },
                        cells: a.cells,
                    },
                    vec![Expr::Var(lo), Expr::Var(h)],
                ),
            ));
        }
        Ok(steps)
    }
}

/// Lower every expression of the definition, in the order of the
/// language: the equations, the grid, the edges, the initial values,
/// the rules.
pub(super) fn steps(d: &Declared, s: &Shaped, r: &Registers) -> Result<Steps> {
    let low = Lowering { d, s, r };
    let mut rates: Vec<Step> = Vec::new();
    let mut defined = Vec::new();
    for (line, &place) in d.equations.iter().zip(&d.places) {
        match &line.lhs {
            Lhs::State(n) | Lhs::Map(n) => {
                let own = s.arrays.of(n);
                let e = low.resolve(
                    &line.ast,
                    d.defined.len(),
                    Some(&own),
                    line.number,
                    line.offset,
                )?;
                rates.push((place, low.stored(e, r.states[place])));
            }
            Lhs::Defined(_) => {
                let e = low.resolve(&line.ast, place, None, line.number, line.offset)?;
                defined.push(low.stored(e, r.defined[place]));
            }
        }
    }
    // One per state, in state order.
    rates.sort_by_key(|x| x.0);
    let grid = low.grid_steps()?;
    let mut edges = Vec::new();
    for (e, v) in d.edges.iter().zip(&r.edges) {
        if let (EdgeKind::Value(ast) | EdgeKind::Gradient(ast), Some(v)) = (&e.kind, v) {
            edges.push((*v, low.value(ast, None, e.line, e.column - 1, *v)?));
        }
    }
    let mut init = grid.clone();
    for i in &s.inits {
        let var = r.states[i.state];
        let own = s.arrays.of(&i.name);
        init.push((
            var,
            low.value(&i.ast, Some(&own), i.line, i.column - 1, var)?,
        ));
    }
    let mut rules = Vec::new();
    for (rule, (cond_var, value_vars)) in s.rules.iter().zip(&r.rules) {
        let to = rule.over();
        let visible = d.defined.len();
        let line = rule.rule.line;
        let e = low.resolve(
            &rule.rule.cond,
            visible,
            Some(&to),
            line,
            rule.rule.cond_column - 1,
        )?;
        rules.push((*cond_var, e));
        for ((_, ast, line, c), &var) in rule.values.iter().zip(value_vars) {
            let e = low.resolve(ast, visible, Some(&to), *line, c - 1)?;
            rules.push((var, low.stored(e, var)));
        }
    }
    Ok(Steps {
        defined,
        rates: rates.into_iter().map(|x| x.1).collect(),
        grid,
        edges,
        init,
        rules,
    })
}

/// The rules as the block runs them: where each one's condition and
/// values are in the registers.
pub(super) fn rule_runs(s: &Shaped, r: &Registers) -> Vec<RuleRun> {
    s.rules
        .iter()
        .zip(&r.rules)
        .map(|(rule, (cond, values))| RuleRun {
            table: rule.table,
            kind: rule.rule.kind,
            edges: rule
                .source
                .filter(|_| matches!(rule.rule.kind, RuleKind::Split | RuleKind::Merge)),
            cond: r.spans[*cond],
            values: values.iter().map(|&v| r.spans[v]).collect(),
            transfers: rule.transfers.clone(),
        })
        .collect()
}

/// A block's programs: what [`programs`] establishes, ready to run.
pub(super) struct Programs {
    /// Computes the defined quantities and the grid's axes.
    pub(super) outputs: Program,
    /// Computes what the rates need, then each state's rate.
    pub(super) rates: Program,
    /// Computes array states' initial values, if any state is an array.
    pub(super) init: Option<Program>,
    /// Computes the rules' conditions and values, if there are rules.
    pub(super) rules: Option<Program>,
    pub(super) outputs_copy: Copies,
    pub(super) rates_copy: Copies,
    /// Register buffers: outputs, rates, initial values, rules.
    pub(super) scratch: RefCell<[Vec<f64>; 4]>,
}

/// Build the programs from the lowered expressions, their large arrays
/// placed and their phases compiled (ADR 0084).
pub(super) fn programs(d: &Declared, r: &Registers, steps: &Steps) -> Programs {
    let layout = &r.layout;
    let mut output_steps: Vec<(usize, &Expr)> = steps.grid.iter().map(|(v, e)| (*v, e)).collect();
    output_steps.extend(steps.edges.iter().map(|(v, e)| (*v, e)));
    output_steps.extend(
        steps
            .defined
            .iter()
            .enumerate()
            .map(|(k, e)| (r.defined[k], e)),
    );
    let mut rate_steps = output_steps.clone();
    rate_steps.extend(steps.rates.iter().enumerate().map(|(k, e)| (r.rates[k], e)));
    let rate_steps = needed(rate_steps, &r.rates);
    let mut outputs = Program::with_layout(&r.spans, r.size, &output_steps);
    let mut rates = Program::with_layout(&r.spans, r.size, &rate_steps);
    let mut rules = (!steps.rules.is_empty()).then(|| {
        let mut all = output_steps.clone();
        all.extend(steps.rules.iter().map(|(v, e)| (*v, e)));
        Program::with_layout(&r.spans, r.size, &all)
    });
    // Tables' states are gathered for each evaluation: no arrays in
    // place.
    let has_tables = d.grid.iter().any(|a| a.table.is_some());
    let (outputs_copy, rates_copy) = if has_tables {
        (Copies::none(), Copies::none())
    } else {
        (
            Copies::place(
                &mut outputs,
                layout.states,
                layout.outputs_at,
                layout.outputs_len,
            ),
            Copies::place(&mut rates, layout.states, layout.rates_at, layout.states),
        )
    };
    let mut init = d.state_decls.iter().any(|x| x.array).then(|| {
        let all: Vec<(usize, &Expr)> = steps.init.iter().map(|(v, e)| (*v, e)).collect();
        Program::with_layout(&r.spans, r.size, &all)
    });
    // Large arrays' phases compiled, once their arrays are placed
    // (ADR 0084).
    for p in [
        Some(&mut outputs),
        Some(&mut rates),
        init.as_mut(),
        rules.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        simsandbox_compiled::compile(p);
    }
    let scratch = RefCell::new([
        outputs.registers(),
        rates.registers(),
        init.as_ref().map_or_else(Vec::new, Program::registers),
        rules.as_ref().map_or_else(Vec::new, Program::registers),
    ]);
    Programs {
        outputs,
        rates,
        init,
        rules,
        outputs_copy,
        rates_copy,
        scratch,
    }
}
