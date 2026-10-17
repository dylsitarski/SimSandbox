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
//! - `dx/dt = …` (or `x' = …`) makes `x` a state.
//! - `name = …` defines a quantity computed from the states, e.g. an
//!   energy; it may use quantities defined above it.
//! - Every other name is a parameter (default 1). Each state `x` also gets
//!   an initial-value parameter `x0` (default 1), marked as an initial
//!   condition. `t` is time; `pi` and the function names are reserved.
//! - Every state and defined quantity is an output port of the same name.
//! - Blank lines and lines starting with `#` are ignored.
//! - All quantities are dimensionless for now.

use crate::ode::OdeSystem;
use simsandbox_core::block::{
    BlockSpec, Domain, Equations, ParamSpec, PortSpec, Shape, TimeSemantics,
};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::{is_reserved, parse, Ast, Expr};
use std::cell::RefCell;

/// A system of typed equations.
pub struct EquationSet {
    spec: BlockSpec,
    states: usize,
    params: usize,
    /// Defined quantities, evaluated in order.
    defined: Vec<Expr>,
    /// Right-hand side of each state's equation.
    rhs: Vec<Expr>,
    /// For each state, the parameter holding its initial value.
    initial: Vec<usize>,
    /// Values by slot: `t`, states, parameters, defined quantities.
    scratch: RefCell<Vec<f64>>,
}

struct Line {
    number: usize,
    /// Character offset of the right-hand side within the line.
    offset: usize,
    lhs: Lhs,
    ast: Ast,
}

enum Lhs {
    State(String),
    Defined(String),
}

fn definition_error(line: usize, column: usize, message: impl std::fmt::Display) -> SimError {
    SimError::InvalidDefinition(format!("line {line}, column {column}: {message}"))
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Read the left-hand side: `dx/dt` or `x'` for a state, a name otherwise.
fn read_lhs(lhs: &str) -> Option<Lhs> {
    let compact: String = lhs.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(name) = compact.strip_suffix('\'') {
        return is_name(name).then(|| Lhs::State(name.into()));
    }
    if let Some(name) = compact
        .strip_prefix('d')
        .and_then(|r| r.strip_suffix("/dt"))
    {
        if is_name(name) {
            return Some(Lhs::State(name.into()));
        }
    }
    is_name(&compact).then_some(Lhs::Defined(compact))
}

impl EquationSet {
    /// Build from the lines of a definition, as a Block named `name`.
    pub fn parse(name: &str, lines: &[String]) -> Result<Self> {
        let mut parsed = Vec::new();
        for (k, text) in lines.iter().enumerate() {
            let number = k + 1;
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((lhs, rhs)) = text.split_once('=') else {
                return Err(definition_error(
                    number,
                    1,
                    "expected an equation such as 'dx/dt = -k*x' or 'E = x^2'",
                ));
            };
            let lhs_kind = read_lhs(lhs).ok_or_else(|| {
                definition_error(
                    number,
                    1,
                    format!("'{}' is not 'dx/dt', 'x'' or a name", lhs.trim()),
                )
            })?;
            let offset = lhs.chars().count() + 1;
            let ast =
                parse(rhs).map_err(|e| definition_error(number, offset + e.column, e.message))?;
            parsed.push(Line {
                number,
                offset,
                lhs: lhs_kind,
                ast,
            });
        }

        // Names of states and defined quantities, checked for clashes.
        let mut states: Vec<String> = Vec::new();
        let mut defined: Vec<String> = Vec::new();
        for line in &parsed {
            let n = match &line.lhs {
                Lhs::State(n) | Lhs::Defined(n) => n,
            };
            if n == "t" || is_reserved(n) {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is a reserved name"),
                ));
            }
            if states.contains(n) || defined.contains(n) {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is defined twice"),
                ));
            }
            match &line.lhs {
                Lhs::State(_) => states.push(n.clone()),
                Lhs::Defined(_) => defined.push(n.clone()),
            }
        }
        if states.is_empty() {
            return Err(SimError::InvalidDefinition(
                "no state equations: write at least one line like 'dx/dt = -k*x'".into(),
            ));
        }

        // Every other name is a parameter, in order of first use.
        let mut params: Vec<String> = Vec::new();
        for line in &parsed {
            for (n, _) in line.ast.names() {
                let known = n == "t" || states.contains(&n) || defined.contains(&n);
                if !known && !params.contains(&n) {
                    params.push(n);
                }
            }
        }
        let mut initial = Vec::new();
        for s in &states {
            let p = format!("{s}0");
            let i = match params.iter().position(|q| *q == p) {
                Some(i) => i,
                None => {
                    params.push(p);
                    params.len() - 1
                }
            };
            initial.push(i);
        }

        // Slots: t, states, parameters, defined quantities.
        let (ns, np) = (states.len(), params.len());
        let slot_of = |n: &str, defined_so_far: usize| -> Option<usize> {
            if n == "t" {
                return Some(0);
            }
            if let Some(i) = states.iter().position(|s| s == n) {
                return Some(1 + i);
            }
            if let Some(i) = params.iter().position(|p| p == n) {
                return Some(1 + ns + i);
            }
            defined[..defined_so_far]
                .iter()
                .position(|d| d == n)
                .map(|i| 1 + ns + np + i)
        };
        let mut rhs = vec![None; ns];
        let mut defined_exprs = Vec::new();
        for line in &parsed {
            let resolve = |visible: usize| {
                line.ast.resolve(&|n| slot_of(n, visible)).map_err(|e| {
                    let hint = if defined
                        .iter()
                        .any(|d| e.message == format!("unknown name '{d}'"))
                    {
                        " (it is defined further down; define it above this line)"
                    } else {
                        ""
                    };
                    definition_error(
                        line.number,
                        line.offset + e.column,
                        format!("{}{hint}", e.message),
                    )
                })
            };
            match &line.lhs {
                Lhs::State(n) => {
                    let i = states.iter().position(|s| s == n).expect("state listed");
                    rhs[i] = Some(resolve(defined.len())?);
                }
                Lhs::Defined(_) => {
                    let visible = defined_exprs.len();
                    defined_exprs.push(resolve(visible)?);
                }
            }
        }

        let one = Unit::DIMENSIONLESS;
        let mut param_specs: Vec<ParamSpec> = params
            .iter()
            .map(|p| ParamSpec {
                name: p.clone(),
                unit: one,
                default: 1.0,
                suggested_min: 0.0,
                suggested_max: 10.0,
                description: "Parameter of the equations".into(),
                initial_condition: false,
            })
            .collect();
        for (s, &i) in states.iter().zip(&initial) {
            let p = &mut param_specs[i];
            p.initial_condition = true;
            p.suggested_min = -10.0;
            p.description = format!("Initial value of {s}");
        }
        let ports = states
            .iter()
            .chain(&defined)
            .map(|n| PortSpec::output(n, one, Shape::Scalar))
            .collect();
        let slots = 1 + ns + np + defined.len();
        Ok(Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params: param_specs,
                states: states.clone(),
                time: TimeSemantics::Continuous,
            },
            states: ns,
            params: np,
            defined: defined_exprs,
            rhs: rhs
                .into_iter()
                .map(|r| r.expect("one equation per state"))
                .collect(),
            initial,
            scratch: RefCell::new(vec![0.0; slots]),
        })
    }

    /// Fill the value buffer for time `t`, states `x` and parameters `p`,
    /// including the defined quantities.
    fn fill(&self, t: f64, x: &[f64], p: &[f64], v: &mut [f64]) {
        let (ns, np) = (self.states, self.params);
        v[0] = t;
        v[1..=ns].copy_from_slice(&x[..ns]);
        v[1 + ns..1 + ns + np].copy_from_slice(&p[..np]);
        for (k, e) in self.defined.iter().enumerate() {
            v[1 + ns + np + k] = e.eval(v);
        }
    }
}

impl Equations for EquationSet {
    fn rhs(&self, t: f64, x: &[f64], _u: &[f64], p: &[f64], dx: &mut [f64]) {
        let mut v = self.scratch.borrow_mut();
        self.fill(t, x, p, &mut v);
        for (d, e) in dx.iter_mut().zip(&self.rhs) {
            *d = e.eval(&v);
        }
    }

    fn outputs(&self, t: f64, x: &[f64], p: &[f64], y: &mut [f64]) {
        let mut v = self.scratch.borrow_mut();
        self.fill(t, x, p, &mut v);
        let ns = self.states;
        y[..ns].copy_from_slice(&x[..ns]);
        let first = 1 + ns + self.params;
        y[ns..].copy_from_slice(&v[first..first + self.defined.len()]);
    }
}

impl OdeSystem for EquationSet {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        for (x, &i) in x0.iter_mut().zip(&self.initial) {
            *x = p[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(lines: &[&str]) -> Result<EquationSet> {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        EquationSet::parse("m", &lines)
    }

    fn error(lines: &[&str]) -> String {
        match set(lines) {
            Err(SimError::InvalidDefinition(m)) => m,
            Err(e) => panic!("unexpected error {e}"),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn states_parameters_initial_values_and_outputs_are_found() {
        let s = set(&[
            "# a damped oscillator",
            "dx/dt = v",
            "",
            "v' = -k*x - c*v",
            "E = 0.5*v^2 + 0.5*k*x^2",
        ])
        .unwrap();
        let spec = s.spec();
        assert_eq!(spec.states, ["x", "v"]);
        let params: Vec<_> = spec.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(params, ["k", "c", "x0", "v0"]);
        assert!(spec.params[2].initial_condition && !spec.params[0].initial_condition);
        let outs: Vec<_> = spec.outputs().map(|p| p.name.as_str()).collect();
        assert_eq!(outs, ["x", "v", "E"]);

        // k = 2, c = 0, x = 1, v = 3.
        let p = [2.0, 0.0, 1.0, 3.0];
        let mut x0 = [0.0; 2];
        s.initial_state(&p, &mut x0);
        assert_eq!(x0, [1.0, 3.0]);
        let mut dx = [0.0; 2];
        s.rhs(0.0, &x0, &[], &p, &mut dx);
        assert_eq!(dx, [3.0, -2.0]);
        let mut y = [0.0; 3];
        s.outputs(0.0, &x0, &p, &mut y);
        assert_eq!(y, [1.0, 3.0, 0.5 * 9.0 + 0.5 * 2.0]);
    }

    #[test]
    fn time_and_defined_quantities_can_be_used() {
        let s = set(&["a = sin(t)", "b = 2*a", "dx/dt = b"]).unwrap();
        let mut dx = [0.0];
        s.rhs(1.0, &[0.0], &[], &[0.0], &mut dx);
        assert_eq!(dx[0], 2.0 * 1.0f64.sin());
    }

    #[test]
    fn mistakes_are_reported_with_line_and_column() {
        assert_eq!(
            error(&["dx/dt = -k*"]),
            "line 1, column 12: expected a number, name or '(', found the end"
        );
        assert!(error(&["dx/dt = 2k"]).starts_with("line 1, column 10: missing operator"));
        assert!(error(&["dx/dt = 1", "dx/dt = 2"]).contains("defined twice"));
        assert!(error(&["dt/dt = 1"]).contains("reserved"));
        assert!(error(&["dsin/dt = 1"]).contains("reserved"));
        assert!(error(&["a = b", "b = 1", "dx/dt = a"]).contains("defined further down"));
        assert!(error(&["x + 1"]).contains("expected an equation"));
        assert!(error(&["2x = 1"]).contains("is not"));
        assert!(error(&["E = 1"]).contains("no state equations"));
    }
}
