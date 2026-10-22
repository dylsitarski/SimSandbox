//! Filters (ADR 0030, ADR 0032): named transformations of recorded data
//! into new signals and fields, evaluated with every recorded sample so
//! their results rewind, thin and branch with the rest of the history.
//!
//! - `expression` filters are derived signals: an expression over recorded
//!   signals, other expressions, parameters and `t`. They are evaluated in
//!   dependency order; a cycle or an unknown name drops the filter.
//! - `cut` filters (ADR 0031) cut a 2D grid along x, along y or along free
//!   lines, making one line field per cut and their shared coordinate.

use simsandbox_expr::Expr;
use simsandbox_file::FilterEntry;
use simsandbox_views::cut::{CutSettings, Geometry, Mode};
use simsandbox_views::{Coordinate, FieldShape};

/// A compiled expression filter.
pub struct ExprFilter {
    pub name: String,
    pub expr: Expr,
    /// The recorded signals and expression filters it reads, for saving.
    pub inputs: Vec<String>,
}

/// A checked cut filter.
pub struct CutFilter {
    pub name: String,
    /// The 2D grid it cuts.
    pub input: String,
    pub nx: usize,
    pub ny: usize,
    pub settings: CutSettings,
}

impl CutFilter {
    /// Its line fields, `name[k]`.
    pub fn lines(&self) -> Vec<String> {
        (0..self.settings.count())
            .map(|k| format!("{}[{k}]", self.name))
            .collect()
    }

    /// Its coordinate field: `name.x`, `name.y` or `name.fraction`.
    pub fn coordinate(&self) -> String {
        let axis = match self.settings.mode {
            Mode::AlongX => "x",
            Mode::AlongY => "y",
            Mode::Free => "fraction",
        };
        format!("{}.{axis}", self.name)
    }

    /// Fields it adds to the history, in order: lines, then coordinate.
    pub fn fields(&self) -> Vec<(String, FieldShape)> {
        let shape = FieldShape::Grid {
            nx: self.settings.points(self.nx, self.ny),
            ny: 1,
        };
        self.lines()
            .into_iter()
            .chain([self.coordinate()])
            .map(|n| (n, shape))
            .collect()
    }

    /// Where the grid's cells lie, from its coordinates' current frames.
    pub fn geometry<'a>(
        &self,
        coordinates: &[Coordinate],
        frame: impl Fn(&str) -> Option<&'a [f64]>,
    ) -> Geometry {
        coordinates
            .iter()
            .filter(|c| c.of == self.input && c.axis < 2)
            .fold(Geometry::cells(self.nx, self.ny), |g, c| {
                match frame(&c.name) {
                    Some(f) => g.with_coordinate(c.axis, &c.name, f),
                    None => g,
                }
            })
    }

    /// The frames it adds for one sample: one per line, then the
    /// coordinate.
    pub fn frames(&self, g: &Geometry, input: &[f64]) -> Vec<Vec<f64>> {
        let mut out = self.settings.cut(g, input);
        out.push(self.settings.coordinate(g));
        out
    }

    /// Display labels: each line by its cut, the coordinate by what it is.
    pub fn labels(&self, g: &Geometry) -> Vec<(String, String)> {
        self.lines()
            .into_iter()
            .zip(self.settings.labels(g))
            .chain([(self.coordinate(), self.settings.coordinate_label(g))])
            .collect()
    }
}

/// The filters of a model, compiled.
#[derive(Default)]
pub struct Filters {
    /// In dependency order.
    pub exprs: Vec<ExprFilter>,
    pub cuts: Vec<CutFilter>,
}

impl Filters {
    /// Compile `entries` for a model recording the scalar `outputs` and
    /// the `grids`, with these parameter keys. Filters that cannot be used
    /// are dropped and described in the second result.
    pub fn compile(
        entries: &[FilterEntry],
        outputs: &[String],
        params: &[String],
        grids: &[(String, FieldShape)],
    ) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut filters = Self::default();
        // Expressions: parse, then order so each comes after what it reads.
        let mut pending: Vec<(String, simsandbox_expr::Ast)> = Vec::new();
        for e in entries.iter().filter(|e| e.type_name == "expression") {
            match e.expr().map(simsandbox_expr::parse_signals) {
                Some(Ok(ast)) => pending.push((e.name.clone(), ast)),
                Some(Err(err)) => problems.push(format!("{}: {err}", e.name)),
                None => problems.push(format!("{}: needs an 'expr'", e.name)),
            }
        }
        let names: Vec<String> = pending.iter().map(|(n, _)| n.clone()).collect();
        let mut ordered: Vec<(String, simsandbox_expr::Ast)> = Vec::new();
        loop {
            let ready = pending.iter().position(|(_, ast)| {
                ast.names()
                    .iter()
                    .all(|(n, _)| !names.contains(n) || ordered.iter().any(|(m, _)| m == n))
            });
            match ready {
                Some(i) => ordered.push(pending.remove(i)),
                None => break,
            }
        }
        for (name, _) in pending {
            problems.push(format!("{name}: depends on itself through other filters"));
        }
        // Compile with every slot in place; drop what fails (and what reads
        // it) and compile again, so slots always match the kept filters.
        loop {
            let derived: Vec<String> = ordered.iter().map(|(n, _)| n.clone()).collect();
            let compiled: Vec<Result<ExprFilter, String>> = ordered
                .iter()
                .map(|(name, ast)| compile_expr(name, ast, outputs, &derived, params))
                .collect();
            let failed: Vec<String> = ordered
                .iter()
                .zip(&compiled)
                .filter_map(|((name, ast), r)| match r {
                    Err(e) => Some(format!("{name}: {e}")),
                    // Reads a filter that is no longer there.
                    Ok(_) => ast
                        .names()
                        .iter()
                        .find(|(n, _)| names.contains(n) && !derived.contains(n))
                        .map(|(n, _)| format!("{name}: '{n}' could not be used")),
                })
                .collect();
            if failed.is_empty() {
                filters.exprs = compiled.into_iter().map(Result::unwrap).collect();
                break;
            }
            ordered.retain(|(name, _)| !failed.iter().any(|f| f.starts_with(&format!("{name}: "))));
            problems.extend(failed);
        }
        // Cuts.
        for e in entries.iter().filter(|e| e.type_name == "cut") {
            match compile_cut(e, grids) {
                Ok(c) => filters.cuts.push(c),
                Err(err) => problems.push(format!("{}: {err}", e.name)),
            }
        }
        (filters, problems)
    }

    /// The entries as saved, in a stable order (expressions, then cuts),
    /// with their inputs written out.
    pub fn entries(&self, given: &[FilterEntry]) -> Vec<FilterEntry> {
        let find = |name: &str| given.iter().find(|e| e.name == name).cloned();
        let exprs = self.exprs.iter().filter_map(|f| {
            let mut e = find(&f.name)?;
            e.inputs = f.inputs.clone();
            Some(e)
        });
        let cuts = self.cuts.iter().filter_map(|c| {
            let mut e = find(&c.name)?;
            e.inputs = vec![c.input.clone()];
            Some(e)
        });
        // Keep the given order where it is consistent; it carries no meaning.
        let mut out: Vec<FilterEntry> = exprs.chain(cuts).collect();
        out.sort_by_key(|e| given.iter().position(|g| g.name == e.name));
        out
    }
}

/// Plain names only: letters, digits and _, not `t` or a reserved word.
pub fn plain_name(name: &str) -> Result<(), String> {
    let simple = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !simple || name == "t" || simsandbox_expr::is_reserved(name) {
        return Err(format!(
            "'{name}' must be a plain name (letters, digits, _) other than t, pi or a function"
        ));
    }
    Ok(())
}

/// Slots: `t`, the output signals, the expression filters in order, then
/// the parameters.
fn compile_expr(
    name: &str,
    ast: &simsandbox_expr::Ast,
    outputs: &[String],
    derived: &[String],
    params: &[String],
) -> Result<ExprFilter, String> {
    plain_name(name)?;
    let expr = ast
        .resolve(&|n| {
            if n == "t" {
                return Some(0);
            }
            if let Some(i) = outputs.iter().position(|o| o == n) {
                return Some(1 + i);
            }
            if let Some(i) = derived.iter().position(|d| d == n) {
                return Some(1 + outputs.len() + i);
            }
            params
                .iter()
                .position(|p| p == n)
                .map(|i| 1 + outputs.len() + derived.len() + i)
        })
        .map_err(|e| {
            format!("{e} (use plotted signals such as a.x, other filters, parameters such as a.k, or t)")
        })?;
    let inputs = ast
        .names()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| outputs.contains(n) || derived.contains(n))
        .collect();
    Ok(ExprFilter {
        name: name.into(),
        expr,
        inputs,
    })
}

fn compile_cut(e: &FilterEntry, grids: &[(String, FieldShape)]) -> Result<CutFilter, String> {
    plain_name(&e.name)?;
    let [input] = e.inputs.as_slice() else {
        return Err("a cut takes exactly one input, a 2D grid".into());
    };
    let (nx, ny) = match grids.iter().find(|(n, _)| n == input).map(|(_, s)| *s) {
        Some(FieldShape::Grid { nx, ny }) if nx >= 2 && ny >= 2 => (nx, ny),
        Some(_) => return Err(format!("'{input}' is not a 2D grid")),
        None => return Err(format!("no grid '{input}'")),
    };
    let object = serde_json::Value::Object(e.settings.clone().into_iter().collect());
    let settings: CutSettings = serde_json::from_value(object).map_err(|err| err.to_string())?;
    settings.check()?;
    Ok(CutFilter {
        name: e.name.clone(),
        input: input.clone(),
        nx,
        ny,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(name: &str, text: &str) -> FilterEntry {
        FilterEntry::expression(name, text, Vec::new())
    }

    #[test]
    fn expressions_run_in_dependency_order_and_cycles_are_dropped() {
        let outputs = ["a.x".to_string()];
        let params = ["a.k".to_string()];
        let entries = [
            expr("energy", "half * a.k"),
            expr("half", "a.x^2 / 2"),
            expr("loop1", "loop2 + 1"),
            expr("loop2", "loop1 + 1"),
            expr("bad", "nope + 1"),
        ];
        let (f, problems) = Filters::compile(&entries, &outputs, &params, &[]);
        let order: Vec<&str> = f.exprs.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(order, ["half", "energy"]);
        assert_eq!(f.exprs[1].inputs, ["half"]);
        assert_eq!(f.exprs[0].inputs, ["a.x"]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        // Slots: t, a.x, half, energy, a.k.
        let vars = [0.0, 3.0, 4.5, 0.0, 2.0];
        assert_eq!(f.exprs[0].expr.eval(&vars), 4.5);
        assert_eq!(f.exprs[1].expr.eval(&vars), 9.0);
    }

    #[test]
    fn cuts_need_one_2d_grid_and_sound_settings() {
        let grids = [
            ("z".to_string(), FieldShape::Grid { nx: 3, ny: 4 }),
            ("line".to_string(), FieldShape::Grid { nx: 3, ny: 1 }),
        ];
        let cut = |input: &str, settings: serde_json::Value| FilterEntry {
            name: "c".into(),
            type_name: "cut".into(),
            inputs: vec![input.into()],
            settings: serde_json::from_value(settings).unwrap(),
        };
        let good = cut(
            "z",
            serde_json::json!({"mode": "along_x", "at": [0.5, 1.5]}),
        );
        let (f, problems) = Filters::compile(&[good], &[], &[], &grids);
        assert!(problems.is_empty(), "{problems:?}");
        let c = &f.cuts[0];
        assert_eq!(c.lines(), ["c[0]", "c[1]"]);
        assert_eq!(c.coordinate(), "c.x");
        assert_eq!(c.fields()[0].1, FieldShape::Grid { nx: 3, ny: 1 });
        for bad in [
            cut("line", serde_json::json!({"mode": "along_x", "at": [0.5]})),
            cut("nope", serde_json::json!({"mode": "along_x", "at": [0.5]})),
            cut("z", serde_json::json!({"mode": "along_x", "at": []})),
            cut("z", serde_json::json!({"mode": "sideways", "at": [1.0]})),
            cut(
                "z",
                serde_json::json!({"at": [1.0], "range": [2.0, 1.0], "whole": false}),
            ),
        ] {
            let (f, problems) = Filters::compile(&[bad], &[], &[], &grids);
            assert!(f.cuts.is_empty() && problems.len() == 1, "{problems:?}");
        }
    }
}
