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
//! - Units (ADR 0045) go in brackets where a name is introduced:
//!   `dx/dt = v [m]` (the state `x` is in m), `E = m*v^2/2 [J]`,
//!   `input F [N], v [m/s]`, and `param k [N/m] = 4` for a parameter with
//!   a unit and a default. Names without a unit are dimensionless; `t` is
//!   in s. Every equation must balance: terms added share a dimension,
//!   `dx/dt` has `x`'s unit per second, and functions such as `sin` and
//!   `exp` take dimensionless arguments. Values are computed in SI, so
//!   `km` and `m` mix correctly inside a block; ports carry the declared
//!   unit, and connections need equal units (convert with `units.convert`).

use crate::ode::OdeSystem;
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, Equations, MethodLabels, MethodSpec, ParamSpec,
    PortSpec, Shape, SpeedClass, StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::MethodSettings;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::units::{dim_text, dimension, from_si, in_si, NO_DIM};
use simsandbox_expr::{is_reserved, parse, Ast, Expr};
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

/// Method name of map blocks.
pub const ITERATE: &str = "iterate";
/// Method name of formula blocks.
pub const EVALUATE: &str = "evaluate";

/// A system of typed equations.
pub struct EquationSet {
    kind: Kind,
    spec: BlockSpec,
    states: usize,
    inputs: usize,
    params: usize,
    /// Defined quantities, evaluated in order.
    defined: Vec<Expr>,
    /// Right-hand side of each state's equation.
    rhs: Vec<Expr>,
    /// For each state, the parameter holding its initial value.
    initial: Vec<usize>,
    /// Values by slot: `t`, states, inputs, parameters, defined quantities.
    scratch: RefCell<Vec<f64>>,
}

struct Line {
    number: usize,
    /// Character offset of the right-hand side within the line.
    offset: usize,
    lhs: Lhs,
    ast: Ast,
}

/// Split a trailing unit annotation, ` [unit]`, off the right-hand side
/// of an equation. `[n]` (a map's current value) is not a unit. Returns
/// the text without it and the unit's text and column.
fn split_unit(rhs: &str) -> (String, Option<(String, usize)>) {
    let trimmed = rhs.trim_end();
    if let (Some(open), true) = (trimmed.rfind('['), trimmed.ends_with(']')) {
        let inside = &trimmed[open + 1..trimmed.len() - 1];
        let spaced = trimmed[..open].ends_with(char::is_whitespace);
        let index = matches!(inside.trim(), "n" | "n+1");
        if spaced && !index {
            let col = trimmed[..open].chars().count() + 2;
            return (
                trimmed[..open].to_string(),
                Some((inside.trim().to_string(), col)),
            );
        }
    }
    (rhs.to_string(), None)
}

/// A name with an optional unit: `F`, `F [N]`, `F[N]`.
fn name_and_unit(item: &str) -> (String, Option<String>) {
    match item.split_once('[') {
        Some((n, rest)) => (
            n.trim().to_string(),
            Some(rest.trim_end().trim_end_matches(']').trim().to_string()),
        ),
        None => (item.trim().to_string(), None),
    }
}

fn read_unit(text: &str, line: usize, column: usize) -> Result<Unit> {
    Unit::parse(text).map_err(|_| {
        definition_error(
            line,
            column,
            format!(
                "unknown unit '{text}' (units are products of m, cm, mm, km, kg, g, s, ms, min, h, \
                 A, K, mol, cd, Hz, N, Pa, J, W, V, Ohm, rad with ^powers, e.g. 'N*s/m', 'm/s^2')"
            ),
        )
    })
}

enum Lhs {
    State(String),
    Map(String),
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

/// Read the left-hand side: `dx/dt` or `x'` for a continuous state,
/// `x[n+1]` for a map state, a name otherwise.
fn read_lhs(lhs: &str) -> Option<Lhs> {
    let compact: String = lhs.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(name) = compact.strip_suffix("[n+1]") {
        return is_name(name).then(|| Lhs::Map(name.into()));
    }
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
        // First the left-hand sides, which decide the kind of block, and
        // the inputs.
        let mut sides = Vec::new();
        let mut inputs: Vec<String> = Vec::new();
        // Units by name, as declared (ADR 0045), and declared parameters
        // with their defaults.
        let mut units: std::collections::HashMap<String, Unit> = Default::default();
        let mut declared: Vec<(String, f64, usize)> = Vec::new();
        for (k, text) in lines.iter().enumerate() {
            let number = k + 1;
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("param ") {
                // `param k [N/m] = 4`, `param k [N/m]`, `param k = 4`.
                let (head, value) = match rest.split_once('=') {
                    Some((h, v)) => {
                        let v = v.trim().parse::<f64>().map_err(|_| {
                            definition_error(number, 1, format!("'{}' is not a number", v.trim()))
                        })?;
                        (h, v)
                    }
                    None => (rest, 1.0),
                };
                let (n, unit) = name_and_unit(head);
                if !is_name(&n) || n == "t" || is_reserved(&n) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' cannot be a parameter name"),
                    ));
                }
                if declared.iter().any(|d| d.0 == n) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' is declared twice"),
                    ));
                }
                if let Some(u) = unit {
                    units.insert(n.clone(), read_unit(&u, number, 1)?);
                }
                declared.push((n, value, number));
                continue;
            }
            if let Some(names) = trimmed.strip_prefix("input ") {
                for item in names.split(',').filter(|n| !n.trim().is_empty()) {
                    let (item, unit) = name_and_unit(item);
                    // Names may also be separated by spaces (no units then).
                    let parts: Vec<&str> = item.split(' ').filter(|n| !n.is_empty()).collect();
                    if let Some(u) = unit {
                        if parts.len() != 1 {
                            return Err(definition_error(
                                number,
                                1,
                                "separate inputs with commas when giving units: 'input F [N], v [m/s]'",
                            ));
                        }
                        units.insert(item.clone(), read_unit(&u, number, 1)?);
                    }
                    for n in parts {
                        if !is_name(n) || n == "t" || is_reserved(n) {
                            return Err(definition_error(
                                number,
                                1,
                                format!("'{n}' cannot be an input name"),
                            ));
                        }
                        if inputs.iter().any(|i| i == n) {
                            return Err(definition_error(
                                number,
                                1,
                                format!("'{n}' is an input twice"),
                            ));
                        }
                        inputs.push(n.to_string());
                    }
                }
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
                    format!("'{}' is not 'dx/dt', 'x'', 'x[n+1]' or a name", lhs.trim()),
                )
            })?;
            let offset = lhs.chars().count() + 1;
            let (rhs, unit) = split_unit(rhs);
            if let Some((u, col)) = unit {
                let n = match &lhs_kind {
                    Lhs::State(n) | Lhs::Map(n) | Lhs::Defined(n) => n.clone(),
                };
                units.insert(n, read_unit(&u, number, offset + col)?);
            }
            sides.push((number, offset, lhs_kind, rhs));
        }
        if sides.is_empty() {
            return Err(SimError::InvalidDefinition(
                "no equations: write lines like 'dx/dt = -k*x', 'x[n+1] = r*x*(1 - x)' or 'y = a*x^2'"
                    .into(),
            ));
        }
        let continuous = sides.iter().find(|s| matches!(s.2, Lhs::State(_)));
        let map = sides.iter().find(|s| matches!(s.2, Lhs::Map(_)));
        let kind = match (continuous, map) {
            (Some(_), Some(m)) => {
                return Err(definition_error(
                    m.0,
                    1,
                    "a block is either continuous (dx/dt) or a map (x[n+1]), not both",
                ))
            }
            (Some(_), None) => Kind::Continuous,
            (None, Some(_)) => Kind::Map,
            (None, None) => Kind::Formulas,
        };
        let mut parsed = Vec::new();
        for (number, offset, lhs, rhs) in sides {
            // In maps, x[n] is the current value: blank out "[n]" so columns stay put.
            let rhs = if kind == Kind::Map {
                rhs.replace("[n]", "   ")
            } else {
                rhs
            };
            let ast =
                parse(&rhs).map_err(|e| definition_error(number, offset + e.column, e.message))?;
            parsed.push(Line {
                number,
                offset,
                lhs,
                ast,
            });
        }

        // Names of states and defined quantities, checked for clashes.
        let mut states: Vec<String> = Vec::new();
        let mut defined: Vec<String> = Vec::new();
        for line in &parsed {
            let n = match &line.lhs {
                Lhs::State(n) | Lhs::Map(n) | Lhs::Defined(n) => n,
            };
            if n == "t" || is_reserved(n) {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is a reserved name"),
                ));
            }
            if states.contains(n) || defined.contains(n) || inputs.contains(n) {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is defined twice"),
                ));
            }
            match &line.lhs {
                Lhs::State(_) | Lhs::Map(_) => states.push(n.clone()),
                Lhs::Defined(_) => defined.push(n.clone()),
            }
        }
        // Declared parameters, then every other name in order of first use.
        let mut params: Vec<String> = Vec::new();
        for (n, _, number) in &declared {
            if states.contains(n) || defined.contains(n) || inputs.contains(n) {
                return Err(definition_error(
                    *number,
                    1,
                    format!("'{n}' is defined twice"),
                ));
            }
            params.push(n.clone());
        }
        for line in &parsed {
            for (n, _) in line.ast.names() {
                let known =
                    n == "t" || states.contains(&n) || defined.contains(&n) || inputs.contains(&n);
                if !known && !params.contains(&n) {
                    params.push(n);
                }
            }
        }
        if kind == Kind::Map {
            if let Some(line) = parsed
                .iter()
                .find(|l| l.ast.names().iter().any(|(n, _)| n == "n"))
            {
                return Err(definition_error(
                    line.number,
                    1,
                    "'n' is the step number in maps and cannot be used in equations yet",
                ));
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

        // Units (ADR 0045). A block that declares none is dimensionless
        // throughout, `t` included, as typed equations were before units.
        let checked = !units.is_empty();
        for (s, &i) in states.iter().zip(&initial) {
            let x = units.get(s).copied().unwrap_or(Unit::DIMENSIONLESS);
            let p = &params[i];
            match units.get(p) {
                Some(u) if u.dim != x.dim => {
                    let number = declared.iter().find(|d| &d.0 == p).map_or(1, |d| d.2);
                    return Err(definition_error(
                        number,
                        1,
                        format!("{p} is the initial value of {s}, so it is in {x}, not {u}"),
                    ));
                }
                Some(_) => {}
                None => {
                    units.insert(p.clone(), x);
                }
            }
        }
        let time = if checked {
            Unit::SECOND
        } else {
            Unit::DIMENSIONLESS
        };
        // Defined quantities without a unit take their right side's.
        for line in &parsed {
            let Lhs::Defined(n) = &line.lhs else {
                continue;
            };
            if !checked || units.contains_key(n) {
                continue;
            }
            let of = |m: &str| {
                if m == "t" {
                    time.dim.0
                } else {
                    units.get(m).map_or(NO_DIM, |u| u.dim.0)
                }
            };
            if let Ok(d) = dimension(&line.ast, &of) {
                units.insert(n.clone(), Unit::new(d, 1.0));
            }
        }
        let unit_of = |n: &str| -> Unit {
            if n == "t" {
                time
            } else {
                units.get(n).copied().unwrap_or(Unit::DIMENSIONLESS)
            }
        };
        // Every equation balances.
        if checked {
            for line in &parsed {
                let of = |m: &str| unit_of(m).dim.0;
                let got = dimension(&line.ast, &of)
                    .map_err(|(c, m)| definition_error(line.number, line.offset + c, m))?;
                let (what, want) = match &line.lhs {
                    Lhs::State(n) => {
                        let x = unit_of(n).dim.0;
                        (
                            format!("d{n}/dt"),
                            std::array::from_fn(|i| x[i] - Unit::SECOND.dim.0[i]),
                        )
                    }
                    Lhs::Map(n) => (format!("{n}[n+1]"), unit_of(n).dim.0),
                    Lhs::Defined(n) => (n.clone(), unit_of(n).dim.0),
                };
                if got != want {
                    return Err(definition_error(
                        line.number,
                        line.offset + 1,
                        format!(
                            "{what} is in {} but the right side is in {}",
                            dim_text(want),
                            dim_text(got)
                        ),
                    ));
                }
            }
        }

        // Slots: t, states, inputs, parameters, defined quantities.
        let (ns, ni, np) = (states.len(), inputs.len(), params.len());
        // Values are kept in each name's unit and computed in SI.
        let scales: Vec<f64> = std::iter::once(time.scale)
            .chain(states.iter().map(|n| unit_of(n).scale))
            .chain(inputs.iter().map(|n| unit_of(n).scale))
            .chain(params.iter().map(|n| unit_of(n).scale))
            .chain(defined.iter().map(|n| unit_of(n).scale))
            .collect();
        let slot_of = |n: &str, defined_so_far: usize| -> Option<usize> {
            if n == "t" {
                return Some(0);
            }
            if let Some(i) = states.iter().position(|s| s == n) {
                return Some(1 + i);
            }
            if let Some(i) = inputs.iter().position(|s| s == n) {
                return Some(1 + ns + i);
            }
            if let Some(i) = params.iter().position(|p| p == n) {
                return Some(1 + ns + ni + i);
            }
            defined[..defined_so_far]
                .iter()
                .position(|d| d == n)
                .map(|i| 1 + ns + ni + np + i)
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
                Lhs::State(n) | Lhs::Map(n) => {
                    let i = states.iter().position(|s| s == n).expect("state listed");
                    let e = in_si(resolve(defined.len())?, &scales);
                    rhs[i] = Some(from_si(e, unit_of(n).scale));
                }
                Lhs::Defined(n) => {
                    let visible = defined_exprs.len();
                    let e = in_si(resolve(visible)?, &scales);
                    defined_exprs.push(from_si(e, unit_of(n).scale));
                }
            }
        }

        let mut param_specs: Vec<ParamSpec> = params
            .iter()
            .map(|p| ParamSpec {
                name: p.clone(),
                unit: unit_of(p),
                default: declared.iter().find(|d| &d.0 == p).map_or(1.0, |d| d.1),
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
        // Which inputs each defined quantity uses, directly or through
        // quantities above it.
        let mut uses: Vec<Vec<String>> = Vec::new();
        for line in parsed.iter().filter(|l| matches!(l.lhs, Lhs::Defined(_))) {
            let mut u: Vec<String> = Vec::new();
            for (n, _) in line.ast.names() {
                let via: Vec<String> = match defined[..uses.len()].iter().position(|d| *d == n) {
                    Some(i) => uses[i].clone(),
                    None if inputs.contains(&n) => vec![n],
                    None => Vec::new(),
                };
                for i in via {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
            }
            // In input order, so the declaration reads naturally.
            u.sort_by_key(|i| inputs.iter().position(|x| x == i));
            uses.push(u);
        }
        let ports = inputs
            .iter()
            .map(|n| PortSpec::input(n, unit_of(n), Shape::Scalar))
            .chain(
                states
                    .iter()
                    .map(|n| PortSpec::output(n, unit_of(n), Shape::Scalar)),
            )
            .chain(defined.iter().zip(&uses).map(|(n, u)| {
                let u: Vec<&str> = u.iter().map(String::as_str).collect();
                PortSpec::output(n, unit_of(n), Shape::Scalar).depends_on(&u)
            }))
            .collect();
        let slots = 1 + ns + ni + np + defined.len();
        Ok(Self {
            kind,
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params: param_specs,
                states: states.clone(),
                tables: Vec::new(),
                time: match kind {
                    Kind::Continuous => TimeSemantics::Continuous,
                    Kind::Map => TimeSemantics::DiscreteEvent,
                    Kind::Formulas => TimeSemantics::Stateless,
                },
            },
            states: ns,
            inputs: ni,
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

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Fill the value buffer for time `t`, states `x`, inputs `u` and
    /// parameters `p`, including the defined quantities. Missing inputs read
    /// as 0.
    fn fill(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], v: &mut [f64]) {
        let (ns, ni, np) = (self.states, self.inputs, self.params);
        v[0] = t;
        v[1..=ns].copy_from_slice(&x[..ns]);
        for (k, slot) in v[1 + ns..1 + ns + ni].iter_mut().enumerate() {
            *slot = u.get(k).copied().unwrap_or(0.0);
        }
        v[1 + ns + ni..1 + ns + ni + np].copy_from_slice(&p[..np]);
        let first = 1 + ns + ni + np;
        for (k, e) in self.defined.iter().enumerate() {
            v[first + k] = e.eval(v);
        }
    }
}

impl Equations for EquationSet {
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        let mut v = self.scratch.borrow_mut();
        self.fill(t, x, u, p, &mut v);
        for (d, e) in dx.iter_mut().zip(&self.rhs) {
            *d = e.eval(&v);
        }
    }

    fn outputs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], y: &mut [f64]) {
        let mut v = self.scratch.borrow_mut();
        self.fill(t, x, u, p, &mut v);
        let ns = self.states;
        y[..ns].copy_from_slice(&x[..ns]);
        let first = 1 + ns + self.inputs + self.params;
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

/// A map or a set of formulas as a kernel Block (ADR 0019): maps are
/// iterated once per communication step, formulas are evaluated. Neither
/// approximates anything, so there is no method to choose (ADR 0020).
pub struct ExplicitBlock {
    set: EquationSet,
    method: MethodSpec,
    next: Vec<f64>,
}

impl ExplicitBlock {
    /// Wrap a map or formula set; `method` must be `iterate` for maps and
    /// `evaluate` for formulas.
    pub fn new(set: EquationSet, method: &MethodSettings) -> Result<Self> {
        let (expected, label) = match set.kind {
            Kind::Map => (ITERATE, "Iterate the map once per step"),
            Kind::Formulas => (EVALUATE, "Evaluate the formulas"),
            Kind::Continuous => {
                return Err(SimError::InvalidMethod(
                    "continuous equations need an ODE method such as rk4".into(),
                ))
            }
        };
        if method.name != expected || !method.options.is_empty() {
            let what = if set.kind == Kind::Map {
                "maps are iterated"
            } else {
                "formulas are evaluated"
            };
            return Err(SimError::InvalidMethod(format!(
                "{what}: use method '{expected}' with no options"
            )));
        }
        let states = set.spec.states.len();
        Ok(Self {
            set,
            method: MethodSpec {
                name: label.into(),
                // No discretisation error, but not benchmarked case by case.
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Quantitative,
                },
            },
            next: vec![0.0; states],
        })
    }
}

impl Block for ExplicitBlock {
    fn spec(&self) -> &BlockSpec {
        &self.set.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        self.set.initial_state(params, state);
        Ok(())
    }

    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        crate::ode::initial_params_by_name(&self.set.spec, state, params, |p, x| {
            self.set.initial_state(p, x)
        })
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        if self.set.kind == Kind::Map {
            // All states update together from the current values.
            self.set
                .rhs(ctx.t, io.state, io.inputs, io.params, &mut self.next);
            io.state.copy_from_slice(&self.next);
        }
        self.set
            .outputs(ctx.t + ctx.dt, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        self.set
            .outputs(t, io.state, io.inputs, io.params, io.outputs);
        Ok(())
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
        s.outputs(0.0, &x0, &[], &p, &mut y);
        assert_eq!(y, [1.0, 3.0, 0.5 * 9.0 + 0.5 * 2.0]);
    }

    #[test]
    fn maps_iterate_all_states_together() {
        // A rotation by 90 degrees: needs the old x to compute the new y.
        let s = set(&["x[n+1] = -y[n]", "y[n+1] = x"]).unwrap();
        assert_eq!(s.kind(), Kind::Map);
        let mut b = ExplicitBlock::new(s, &Kind::Map.default_method()).unwrap();
        let (p, mut x, mut y) = ([1.0, 0.0], [0.0; 2], [0.0; 2]);
        b.init(&p, &mut x).unwrap();
        let ctx = StepContext {
            t: 0.0,
            dt: 1.0,
            step: 0,
            seed: 0,
        };
        let mut step = |x: &mut [f64], y: &mut [f64]| {
            b.do_step(
                ctx,
                StepIo {
                    params: &p,
                    inputs: &[],
                    state: x,
                    outputs: y,
                    tables: &mut [],
                    population_inputs: &[],
                    population_outputs: &mut [],
                },
            )
            .unwrap();
        };
        step(&mut x, &mut y);
        assert_eq!(x, [0.0, 1.0]);
        step(&mut x, &mut y);
        assert_eq!(x, [-1.0, 0.0]);
        assert_eq!(y, x);
    }

    #[test]
    fn formulas_have_no_state_and_need_the_evaluate_method() {
        let s = set(&["y = a*x^2 + b"]).unwrap();
        assert_eq!(s.kind(), Kind::Formulas);
        assert!(s.spec().states.is_empty());
        let names: Vec<_> = s.spec().params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "x", "b"]);
        assert!(ExplicitBlock::new(set(&["y = 1"]).unwrap(), &Kind::Map.default_method()).is_err());
        assert!(
            ExplicitBlock::new(set(&["dx/dt = 1"]).unwrap(), &Kind::Map.default_method()).is_err()
        );
        let b = ExplicitBlock::new(s, &Kind::Formulas.default_method()).unwrap();
        let mut y = [0.0];
        b.compute_outputs(
            0.0,
            StepIo {
                params: &[2.0, 3.0, 1.0],
                inputs: &[],
                state: &mut [],
                outputs: &mut y,
                tables: &mut [],
                population_inputs: &[],
                population_outputs: &mut [],
            },
        )
        .unwrap();
        assert_eq!(y, [19.0]);
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
        assert!(error(&["# only a comment"]).contains("no equations"));
        assert!(error(&["dx/dt = 1", "y[n+1] = y"]).contains("not both"));
        assert!(error(&["x[n+1] = x + n"]).contains("step number"));
    }

    fn unit(text: &str) -> Unit {
        Unit::parse(text).unwrap()
    }

    #[test]
    fn units_are_declared_where_names_appear() {
        let s = set(&[
            "input F [N]",
            "param m [kg] = 1000",
            "param c [N*s/m] = 50",
            "dx/dt = v [m]",
            "dv/dt = (F - c*v)/m [m/s]",
            "E = m*v^2/2",
        ])
        .unwrap();
        let port = |n: &str| s.spec.ports.iter().find(|p| p.name == n).unwrap().unit;
        assert!(port("F").same_as(&Unit::NEWTON));
        assert!(port("x").same_as(&Unit::METER));
        assert!(port("v").same_as(&unit("m/s")));
        // A defined quantity without a unit takes its right side's: J.
        assert!(port("E").same_as(&unit("J")));
        let param = |n: &str| s.spec.params.iter().find(|p| p.name == n).unwrap();
        assert_eq!(
            (param("m").default, param("m").unit),
            (1000.0, Unit::KILOGRAM)
        );
        assert!(param("x0").unit.same_as(&Unit::METER) && param("x0").initial_condition);
    }

    #[test]
    fn equations_must_balance() {
        let e = error(&["dx/dt = v [m]", "dv/dt = -x [m/s]"]);
        assert!(
            e.contains("line 2") && e.contains("dv/dt is in m·s^-2 but the right side is in m"),
            "{e}"
        );
        let e = error(&["input F [N]", "dx/dt = F - x [m/s]"]);
        assert!(e.contains("cannot subtract"), "{e}");
        let e = error(&["param L [m] = 1", "y = sin(L)"]);
        assert!(e.contains("dimensionless"), "{e}");
        let e = error(&["param L [m] = 1", "y = sqrt(L)"]);
        assert!(e.contains("sqrt"), "{e}");
        assert!(error(&["dx/dt = 1 [furlong]"]).contains("unknown unit 'furlong'"));
        // Names without a unit are dimensionless, which can be what is meant.
        set(&["param L [m] = 2", "param k = 3", "y = k*L [m]"]).unwrap();
        set(&[
            "param A [m^2] = 4",
            "side = sqrt(A) [m]",
            "theta = atan2(side, side)",
        ])
        .unwrap();
    }

    #[test]
    fn values_are_computed_in_si_across_scales() {
        let s = set(&[
            "param d [km] = 2",
            "param t_go [min] = 1",
            "y = d [m]",
            "speed = d/t_go [m/s]",
        ])
        .unwrap();
        let mut y = vec![0.0; 2];
        s.outputs(0.0, &[], &[], &[2.0, 1.0], &mut y);
        assert_eq!(y[0], 2000.0);
        assert!((y[1] - 2000.0 / 60.0).abs() < 1e-12);
        // dx/dt in km/h from a speed in m/s: x in km grows 3.6 per hour of
        // 1 m/s.
        let s = set(&["input v [m/s]", "dx/dt = v [km]"]).unwrap();
        let mut dx = vec![0.0];
        s.rhs(0.0, &[0.0], &[1.0], &[0.0], &mut dx);
        assert!((dx[0] - 1e-3).abs() < 1e-15, "km per second: {}", dx[0]);
    }

    #[test]
    fn maps_and_models_without_units_are_as_before() {
        // `[n]` is the current value, not a unit.
        set(&["x[n+1] = r*x[n]"]).unwrap();
        set(&["x[n+1] = r*x[n] [m]", "param r = 2"]).unwrap();
        // No units anywhere: dimensionless, time included, unchecked.
        let s = set(&["dx/dt = -k*x + t"]).unwrap();
        assert!(s.spec.ports.iter().all(|p| p.unit == Unit::DIMENSIONLESS));
        assert!(error(&["input u v [m]", "y = u"]).contains("commas"));
    }
}
