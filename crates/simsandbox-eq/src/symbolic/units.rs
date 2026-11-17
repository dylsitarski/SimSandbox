//! Units (ADR 0045): every quantity's unit, and each equation balanced.

use super::declare::Declared;
use super::functions::{linear, meshes, reduce, tables, Function};
use super::shaped::{Arrays, Shaped};
use super::shapes::{ast_name, number};
use super::statements::{definition_error, EdgeKind, Lhs};
use simsandbox_core::units::Unit;
use simsandbox_core::Result;
use simsandbox_expr::units::{
    dim_div, dim_pow, dim_text, dimension_with, Dim, DIM_TOO_HIGH, NO_DIM,
};
use simsandbox_expr::Ast;

/// A dimension, or a mistake with its column.
pub(super) type DimResult = std::result::Result<Dim, (usize, String)>;

/// Every quantity's unit: what [`units`] establishes. A block that
/// declares none is dimensionless throughout, `t` included, as typed
/// equations were before units; otherwise every equation balances.
pub(super) struct Units<'a> {
    /// As declared, or given by a rate's.
    declared: &'a std::collections::HashMap<String, Unit>,
    /// Found: a state's initial-value parameter has its state's unit, a
    /// defined quantity without one its right side's.
    found: std::collections::HashMap<String, Unit>,
    /// The unit of `t`.
    pub(super) time: Unit,
    /// Each axis's dimension.
    pub(super) axis_dims: Vec<Dim>,
}

impl Units<'_> {
    /// The unit `name` was given or found to have, if any.
    pub(super) fn get(&self, name: &str) -> Option<Unit> {
        self.declared
            .get(name)
            .or_else(|| self.found.get(name))
            .copied()
    }

    /// The unit of `name`: dimensionless if it has none.
    pub(super) fn of(&self, name: &str) -> Unit {
        if name == "t" {
            self.time
        } else {
            self.get(name).unwrap_or(Unit::DIMENSIONLESS)
        }
    }

    fn dim_of(&self, name: &str) -> Dim {
        self.of(name).dim.0
    }
}

/// The dimensions of array operations: a stencil divides by its axis's
/// unit.
pub(super) struct ArrayDims<'a> {
    pub(super) arrays: &'a Arrays<'a>,
    pub(super) axis_dims: &'a [Dim],
}

impl ArrayDims<'_> {
    /// The first argument's dimension (none without arguments).
    pub(super) fn first(args: &[(&Ast, Dim)]) -> Dim {
        args.first().map_or(NO_DIM, |a| a.1)
    }

    pub(super) fn axis_dim_of(&self, name: &str) -> Dim {
        self.arrays
            .grid
            .iter()
            .position(|a| a.name == name)
            .map_or(NO_DIM, |i| self.axis_dims[i])
    }

    /// A mesh's coordinates are in one unit.
    pub(super) fn one_unit(&self, name: &str, coords: &[(&Ast, Dim)], col: usize) -> DimResult {
        let first = coords.first().map_or(NO_DIM, |a| a.1);
        match coords.iter().find(|a| a.1 != first) {
            Some((a, d)) => Err((
                a.names().first().map_or(col, |n| n.1),
                format!(
                    "'{name}' takes coordinates in one unit, not {} and {}",
                    dim_text(first),
                    dim_text(*d)
                ),
            )),
            None => Ok(first),
        }
    }

    /// The dimension of `name` applied to `args` (each with its own).
    fn apply(&self, name: &str, args: &[(&Ast, Dim)], col: usize) -> DimResult {
        use Function::*;
        let d = Self::first(args);
        match Function::by_name(name) {
            // T's, F's: the first argument's; a table, its rows or a
            // corner's number after it have none to check.
            Some(Id | Rows | FromEnd | ToEnd | Corner | Outflow) => Ok(d),
            Some(Of) => tables::of_dim(args, col),
            Some(Measure) => meshes::measure_dim(self, args, col),
            Some(Normal) => meshes::normal_dim(self, args, col),
            // T's, u's: the first argument's; the cells, sides or
            // table after it have none to check.
            Some(Centre | Inner | Outer | Across) => Ok(d),
            // The first argument's: the sides table's name.
            Some(Boundary) => Ok(d),
            // A pure number.
            Some(Rcond) => Ok(NO_DIM),
            Some(Solve) => linear::solve_dim(args, col),
            // e's: the first argument's; an axis after it has none.
            Some(Sum | Mean | MinOf | MaxOf) => Ok(d),
            // A cell number (ADR 0070).
            Some(ArgMinOf | ArgMaxOf) => Ok(NO_DIM),
            Some(At) => reduce::at_dim(args, col),
            _ => self.old_apply(name, args, col),
        }
    }

    fn old_apply(&self, name: &str, args: &[(&Ast, Dim)], col: usize) -> DimResult {
        let grid = self.arrays.grid;
        let d = args.first().map_or(NO_DIM, |a| a.1);
        let too_high = || (col, DIM_TOO_HIGH.to_string());
        Ok(match name {
            "lap" => {
                // Along the array's own axes (ADR 0080).
                let own = match args[0].0 {
                    Ast::Name(n, _) => self.arrays.of(n),
                    _ => Vec::new(),
                };
                let Some(&first) = own.first() else {
                    return Ok(d);
                };
                if own
                    .iter()
                    .any(|&a| self.axis_dims[a] != self.axis_dims[first])
                {
                    let names: Vec<&str> = own.iter().map(|&a| grid[a].name.as_str()).collect();
                    return Err((
                        col,
                        format!(
                            "lap adds second derivatives along {}, so the axes need the same unit",
                            names.join(" and ")
                        ),
                    ));
                }
                dim_pow(self.axis_dims[first], 2)
                    .and_then(|h2| dim_div(d, h2))
                    .ok_or_else(too_high)?
            }
            "spacing" => match args[0].0 {
                Ast::Name(n, _) => self.axis_dim_of(n),
                _ => NO_DIM,
            },
            n if n.starts_with("dd") => {
                let axis = if n == "dd" {
                    args.get(1).map_or("", |a| ast_name(a.0))
                } else {
                    &n[2..]
                };
                let a = self.axis_dim_of(axis);
                dim_div(d, a).ok_or_else(too_high)?
            }
            // prev, next, on: the first argument's.
            _ => d,
        })
    }
}

/// The right side `ast` has the dimension `want` of `what`.
fn balance(
    units: &Units,
    dims: &ArrayDims,
    ast: &Ast,
    want: Dim,
    what: &str,
    line: usize,
    col: usize,
) -> Result<()> {
    // Zero is zero in every unit: `dx/dt = 0` holds x.
    if matches!(ast, Ast::Num(v) if *v == 0.0) {
        return Ok(());
    }
    let got = dimension_with(ast, &|m| units.dim_of(m), &|n, a, c| dims.apply(n, a, c))
        .map_err(|(c, m)| definition_error(line, col + c - 1, m))?;
    if got != want {
        return Err(definition_error(
            line,
            col,
            format!(
                "{what} is in {} but the right side is in {}",
                dim_text(want),
                dim_text(got)
            ),
        ));
    }
    Ok(())
}

/// Every equation, axis end, initial value and edge value balances.
fn check_balance(d: &Declared, s: &Shaped, units: &Units, dims: &ArrayDims) -> Result<()> {
    for line in &d.equations {
        let (what, want) = match &line.lhs {
            Lhs::State(n) => {
                let x = units.dim_of(n);
                (
                    format!("d{n}/dt"),
                    dim_div(x, Unit::SECOND.dim.0).ok_or_else(|| {
                        definition_error(line.number, 1, DIM_TOO_HIGH.to_string())
                    })?,
                )
            }
            Lhs::Map(n) => (format!("{n}[n+1]"), units.dim_of(n)),
            Lhs::Defined(n) => (n.clone(), units.dim_of(n)),
        };
        balance(
            units,
            dims,
            &line.ast,
            want,
            &what,
            line.number,
            line.offset + 1,
        )?;
    }
    for (a, dim) in d.grid.iter().zip(&units.axis_dims) {
        for end in [&a.lo, &a.hi].into_iter().filter(|e| number(e).is_none()) {
            let what = format!("the axis {}", a.name);
            balance(units, dims, end, *dim, &what, a.line, 1)?;
        }
    }
    for i in s.inits.iter().filter(|i| number(&i.ast).is_none()) {
        let what = format!("{}'s initial value", i.name);
        let want = units.dim_of(&i.name);
        balance(units, dims, &i.ast, want, &what, i.line, i.column)?;
    }
    for e in &d.edges {
        // A plain number is in the array's unit (per axis unit for
        // a gradient).
        if let EdgeKind::Value(a) | EdgeKind::Gradient(a) = &e.kind {
            if number(a).is_some() {
                continue;
            }
        }
        let x = units.dim_of(&e.array);
        match &e.kind {
            EdgeKind::Value(ast) => {
                let what = format!("{}'s edge value", e.array);
                balance(units, dims, ast, x, &what, e.line, e.column)?
            }
            EdgeKind::Gradient(ast) => {
                let want = dim_div(x, units.axis_dims[e.axis])
                    .ok_or_else(|| definition_error(e.line, e.column, DIM_TOO_HIGH.to_string()))?;
                let what = format!("{}'s edge gradient", e.array);
                balance(units, dims, ast, want, &what, e.line, e.column)?
            }
            EdgeKind::Periodic => {}
        }
    }
    // A rule's condition holds or not: a comparison (1 or 0) or a number
    // without a unit. Each value is in its column's unit, a plain number
    // taken in it.
    for rule in &s.rules {
        let r = rule.rule;
        let got = dimension_with(&r.cond, &|m| units.dim_of(m), &|n, a, c| {
            dims.apply(n, a, c)
        })
        .map_err(|(c, m)| definition_error(r.line, r.cond_column + c - 1, m))?;
        if got != NO_DIM {
            return Err(definition_error(
                r.line,
                r.cond_column,
                format!(
                    "a rule's condition holds or not: a comparison, or a number without a \
                     unit, not a value in {}",
                    dim_text(got)
                ),
            ));
        }
        for (col, ast, line, c) in rule.values.iter().filter(|v| number(&v.1).is_none()) {
            let what = format!("{col}'s value");
            balance(units, dims, ast, units.dim_of(col), &what, *line, *c)?;
        }
    }
    Ok(())
}

/// Find every quantity's unit and check that the definition balances.
pub(super) fn units<'a>(d: &'a Declared, s: &Shaped) -> Result<Units<'a>> {
    let checked = !d.units.is_empty();
    let mut units = Units {
        declared: &d.units,
        found: Default::default(),
        time: if checked {
            Unit::SECOND
        } else {
            Unit::DIMENSIONLESS
        },
        axis_dims: Vec::new(),
    };
    // A state's initial-value parameter is in the state's unit.
    for (state, i) in d.states.iter().zip(&s.initial) {
        let Some(i) = *i else { continue };
        let x = units.get(state).unwrap_or(Unit::DIMENSIONLESS);
        let p = &s.params[i];
        match units.get(p) {
            Some(u) if u.dim != x.dim => {
                let number = d.params.iter().find(|q| &q.0 == p).map_or(1, |q| q.2);
                return Err(definition_error(
                    number,
                    1,
                    format!("{p} is the initial value of {state}, so it is in {x}, not {u}"),
                ));
            }
            Some(_) => {}
            None => {
                units.found.insert(p.clone(), x);
            }
        }
    }
    units.axis_dims = d
        .grid
        .iter()
        .map(|a| units.get(&a.name).map_or(NO_DIM, |u| u.dim.0))
        .collect();
    let axis_dims = units.axis_dims.clone();
    let dims = ArrayDims {
        arrays: &s.arrays,
        axis_dims: &axis_dims,
    };
    // Defined quantities without a unit take their right side's.
    for line in &d.equations {
        let Lhs::Defined(n) = &line.lhs else {
            continue;
        };
        if !checked || units.get(n).is_some() {
            continue;
        }
        let of = |m: &str| {
            if m == "t" {
                units.time.dim.0
            } else {
                units.get(m).map_or(NO_DIM, |u| u.dim.0)
            }
        };
        if let Ok(x) = dimension_with(&line.ast, &of, &|n, a, c| dims.apply(n, a, c)) {
            units.found.insert(n.clone(), Unit::new(x, 1.0));
        }
    }
    if checked {
        check_balance(d, s, &units, &dims)?;
    }
    Ok(units)
}
