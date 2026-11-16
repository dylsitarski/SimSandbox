//! Building an [`EquationSet`] from the lines of a definition: the
//! phases, each taking the typed results of those before it.

use super::declare::declare;
use super::interface::{interface, Interface};
use super::layout::registers;
use super::program::{programs, rule_runs, steps, Programs};
use super::shaped::shape;
use super::statements::{definition_error, dynamics, statements};
use super::tables::tables;
use super::units::units;
use super::EquationSet;
use simsandbox_core::{Result, SimError};

impl EquationSet {
    /// Build from the lines of a definition, as a Block named `name`.
    pub fn parse(name: &str, lines: &[String]) -> Result<Self> {
        Self::parse_with_rows(name, lines, &std::collections::BTreeMap::new())
    }

    /// As [`EquationSet::parse`], with each table (ADR 0079) at the given
    /// number of rows instead of its starting number: tables change only
    /// between steps, and the block compiles its equations again then.
    pub fn parse_with_rows(
        name: &str,
        lines: &[String],
        rows: &std::collections::BTreeMap<String, usize>,
    ) -> Result<Self> {
        let written = lines.len();
        let (lines, from) = row_params(lines);
        Self::parse_lines(name, &lines, rows).map_err(|e| {
            // A mistake on a line holding a parameter over rows is one of
            // its `param` line: the line written.
            let SimError::InvalidDefinition(m) = &e else {
                return e;
            };
            let at = m
                .strip_prefix("line ")
                .and_then(|r| r.split_once(", column "))
                .and_then(|(l, r)| Some((l.parse::<usize>().ok()?, r.split_once(": ")?.1)));
            match at {
                Some((l, what)) if l > written => definition_error(from[l - written - 1], 1, what),
                _ => e,
            }
        })
    }

    /// The lines as [`row_params`] gives them, compiled.
    fn parse_lines(
        name: &str,
        lines: &[String],
        rows: &std::collections::BTreeMap<String, usize>,
    ) -> Result<Self> {
        // Each line read once, then what the lines declare, each
        // quantity's axes and unit, and where its values are.
        let statements = statements(lines);
        let declared = declare(&statements, rows)?;
        let shaped = shape(&declared)?;
        let units = units(&declared, &shaped)?;
        let registers = registers(&declared, &shaped, &units);
        // The expressions as instructions, the tables, the interface;
        // then the programs, which cannot fail.
        let steps = steps(&declared, &shaped, &registers)?;
        let tables = tables(&declared, &shaped, &units)?;
        let Interface {
            spec,
            reads_time,
            initial,
            inputs_in,
            rows_out,
            row_states,
        } = interface(name, &declared, &shaped, &units, &registers, &tables);
        let Programs {
            outputs,
            rates,
            init,
            rules: rules_program,
            outputs_copy,
            rates_copy,
            scratch,
        } = programs(&declared, &registers, &steps);
        Ok(Self {
            kind: declared.kind,
            reads_time,
            spec,
            layout: registers.layout,
            outputs,
            rates,
            init,
            initial,
            scratch,
            outputs_copy,
            rates_copy,
            row_states,
            rules: rule_runs(&shaped, &registers),
            tables,
            rules_program,
            inputs_in,
            rows_out,
            source: (name.to_string(), lines.to_vec()),
        })
    }
}

/// Parameters over a table's rows (ADR 0069, ADR 0079 point 2), `param
/// g[pipes] [S] = 2`: a value per row that does not change, kept with
/// the row through rules and set by the table's starting file. Written as
/// a state over the rows that holds (`dg/dt = 0`, or `g[n+1] = g` in a
/// block of maps), so it is stored, rewound and passed on as states are.
/// Lines are kept in place, so errors keep their line numbers; the
/// holding lines come after them, each with the line of its `param`.
fn row_params(lines: &[String]) -> (Vec<String>, Vec<usize>) {
    let tables: Vec<&str> = lines
        .iter()
        .filter_map(|l| {
            let l = l.trim();
            ["table ", "edges ", "cells ", "sides "]
                .iter()
                .find_map(|w| l.strip_prefix(w))
                .and_then(|r| r.split_whitespace().next())
        })
        .collect();
    let (continuous, maps) = dynamics(lines);
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut held = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        let t = l.trim();
        let over_rows = t.strip_prefix("param ").and_then(|rest| {
            let head = rest.split_once('=').map_or(rest, |(h, _)| h);
            let (n, dims) = head.split_once('[')?;
            let (dims, _) = dims.split_once(']')?;
            dims.split(',')
                .any(|d| tables.contains(&d.trim()))
                .then(|| (n.trim().to_string(), rest))
        });
        match over_rows {
            Some((n, rest)) if continuous || maps => {
                let line = if rest.contains('=') {
                    format!("state {rest}")
                } else {
                    format!("state {rest} = 1")
                };
                out.push(line);
                held.push((n, k + 1));
            }
            _ => out.push(l.clone()),
        }
    }
    let mut from = Vec::with_capacity(held.len());
    for (n, line) in held {
        out.push(if continuous {
            format!("d{n}/dt = 0")
        } else {
            format!("{n}[n+1] = {n}")
        });
        from.push(line);
    }
    (out, from)
}
