//! Building an [`EquationSet`] from the lines of a definition: the
//! phases, each taking the typed results of those before it.

use super::declare::declare;
use super::interface::{interface, Interface};
use super::layout::registers;
use super::program::{programs, rule_runs, steps, Programs};
use super::shaped::shape;
use super::statements::{definition_error, row_params, statements};
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
