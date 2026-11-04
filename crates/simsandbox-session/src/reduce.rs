//! `reduce` formulas of a study or experiment (ADR 0067 step 3, ADR 0070
//! step 3): each a named value of one run, written as an expression whose
//! signals appear only inside reductions of the run:
//!
//! - `final(e)`, `min(e)`, `max(e)`: the last, least and greatest value.
//! - `mean(e)`: the mean over time (trapezoids), or of the samples when
//!   the run has one.
//! - `amplitude(e)`: half of max − min.
//! - `period(e)`: the mean time between upward crossings of `e`'s mean,
//!   NaN with fewer than two.
//! - `first_time(e)`: the first time `e` holds (is not 0), NaN if it never
//!   does: `first_time(abs(cruise.e) < 0.1)`.
//!
//! Reductions combine like numbers: `max(car.v) - min(car.v)`,
//! `final(a.x)/final(b.x)`. `min(a, b)` and `max(a, b)` with two arguments
//! stay the functions of two numbers. Names are signals by full name
//! (`car.v`, `plant/car.v`, `lorenz.xyz[0]`) and `t`.

use simsandbox_expr::{parse_reductions, Ast, Expr};

/// A reduction, as written.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Final,
    Mean,
    Min,
    Max,
    Amplitude,
    Period,
    FirstTime,
}

impl Kind {
    fn by_name(name: &str) -> Option<Kind> {
        Some(match name {
            "final" => Kind::Final,
            "mean" => Kind::Mean,
            "min" => Kind::Min,
            "max" => Kind::Max,
            "amplitude" => Kind::Amplitude,
            "period" => Kind::Period,
            "first_time" => Kind::FirstTime,
            _ => return None,
        })
    }
}

/// One reduction of a formula: what it does, and the expression it
/// reduces over the run's names (slots of [`Reducer::names`]).
#[derive(Debug, Clone)]
struct Part {
    kind: Kind,
    expr: Expr,
}

/// One `reduce` formula, parsed.
#[derive(Debug, Clone)]
pub struct Reducer {
    pub name: String,
    parts: Vec<Part>,
    /// The formula, each reduction the variable of its part.
    outer: Expr,
    /// The names the reductions read, by slot.
    pub names: Vec<String>,
}

/// What a result's unit is (ADR 0070).
#[derive(Debug, Clone, PartialEq)]
pub enum UnitOf {
    /// Seconds: `period`, `first_time`.
    Time,
    /// That of the one signal reduced, as in `max(car.v)`.
    Signal(String),
    /// Anything else: several reductions, an expression inside one.
    Unknown,
}

/// The reductions in `ast`, each call once (name, arguments), checking
/// that names appear only inside them and that they do not nest.
fn reductions(ast: &Ast, inside: bool, found: &mut Vec<(String, Vec<Ast>)>) -> Result<(), String> {
    match ast {
        Ast::Num(_) => Ok(()),
        Ast::Name(..) if inside => Ok(()),
        Ast::Name(n, _) => Err(format!(
            "'{n}' is outside a reduction: write final({n}), max({n}), … for one value of the run"
        )),
        Ast::Neg(a) => reductions(a, inside, found),
        Ast::Bin(_, a, b) => {
            reductions(a, inside, found)?;
            reductions(b, inside, found)
        }
        Ast::Call(_, args) => args.iter().try_for_each(|a| reductions(a, inside, found)),
        Ast::Apply(n, args, _) if Kind::by_name(n).is_some() => {
            if inside {
                return Err(format!(
                    "'{n}' is inside another reduction: a reduction reduces signals over the run"
                ));
            }
            if args.len() != 1 {
                return Err(format!("'{n}' reduces one expression, e.g. {n}(car.v)"));
            }
            if !found.iter().any(|(m, a)| m == n && a == args) {
                found.push((n.clone(), args.clone()));
            }
            reductions(&args[0], true, found)
        }
        Ast::Apply(n, ..) => Err(format!(
            "'{n}' works on arrays of typed equations; a run reduces with final, mean, min, max, \
             amplitude, period or first_time"
        )),
    }
}

impl Reducer {
    /// The unit of this result.
    pub fn unit_of(&self) -> UnitOf {
        match (&self.outer, self.parts.as_slice()) {
            (Expr::Var(0), [p]) => match (p.kind, &p.expr) {
                (Kind::Period | Kind::FirstTime, _) => UnitOf::Time,
                (_, Expr::Var(k)) => UnitOf::Signal(self.names[*k].clone()),
                _ => UnitOf::Unknown,
            },
            _ => UnitOf::Unknown,
        }
    }

    /// Parse `text` as the formula `name`.
    pub fn parse(name: &str, text: &str) -> Result<Self, String> {
        let ast = parse_reductions(text.trim()).map_err(|e| format!("{name}: {}", e.message))?;
        let mut found = Vec::new();
        reductions(&ast, false, &mut found).map_err(|e| format!("{name}: {e}"))?;
        if found.is_empty() {
            return Err(format!(
                "{name}: reduce the run, e.g. max(car.v) or first_time(car.v > 20)"
            ));
        }
        let mut names: Vec<String> = Vec::new();
        for (_, args) in &found {
            for (n, _) in args[0].names() {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
        let slot = |n: &str| names.iter().position(|x| x == n);
        let parts = found
            .iter()
            .map(|(n, args)| {
                Ok(Part {
                    kind: Kind::by_name(n).expect("found by name"),
                    expr: args[0]
                        .resolve(&slot)
                        .map_err(|e| format!("{name}: {}", e.message))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let outer = ast
            .resolve_with(&|_| Some(0), &|n, args, _, _| {
                let k = found
                    .iter()
                    .position(|(m, a)| m == n && a.as_slice() == args)
                    .expect("every reduction was found");
                Ok(Expr::Var(k))
            })
            .map_err(|e| format!("{name}: {}", e.message))?;
        Ok(Self {
            name: name.to_string(),
            parts,
            outer,
            names,
        })
    }

    /// The value of one run: `t` its times, `series` each name's values
    /// at them (in the order of [`Reducer::names`]).
    pub fn eval(&self, t: &[f64], series: &[Vec<f64>]) -> f64 {
        let values: Vec<f64> = self.parts.iter().map(|p| reduce(p, t, series)).collect();
        self.outer.eval(&values)
    }
}

/// One reduction over a run.
fn reduce(part: &Part, t: &[f64], series: &[Vec<f64>]) -> f64 {
    let at = |k: usize| -> Vec<f64> { series.iter().map(|s| s[k]).collect() };
    let v: Vec<f64> = (0..t.len()).map(|k| part.expr.eval(&at(k))).collect();
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = || -> f64 {
        if n == 1 || t[n - 1] == t[0] {
            return v.iter().sum::<f64>() / n as f64;
        }
        let area: f64 = (1..n)
            .map(|k| 0.5 * (v[k] + v[k - 1]) * (t[k] - t[k - 1]))
            .sum();
        area / (t[n - 1] - t[0])
    };
    match part.kind {
        Kind::Final => v[n - 1],
        Kind::Min => lo,
        Kind::Max => hi,
        Kind::Mean => mean(),
        Kind::Amplitude => 0.5 * (hi - lo),
        Kind::Period => {
            let m = mean();
            let ups: Vec<f64> = (1..n)
                .filter(|&k| v[k - 1] < m && v[k] >= m)
                .map(|k| t[k - 1] + (m - v[k - 1]) / (v[k] - v[k - 1]) * (t[k] - t[k - 1]))
                .collect();
            if ups.len() < 2 {
                f64::NAN
            } else {
                (ups[ups.len() - 1] - ups[0]) / (ups.len() - 1) as f64
            }
        }
        // Holds: not 0, and not NaN.
        Kind::FirstTime => (0..n)
            .find(|&k| v[k] != 0.0 && !v[k].is_nan())
            .map_or(f64::NAN, |k| t[k]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reductions_of_a_sine() {
        // sin(2πt) on 0..2 s: two periods.
        let t: Vec<f64> = (0..=2000).map(|k| k as f64 / 1000.0).collect();
        let x: Vec<f64> = t
            .iter()
            .map(|t| (2.0 * std::f64::consts::PI * t).sin())
            .collect();
        let r = |text: &str| {
            Reducer::parse("r", text)
                .unwrap()
                .eval(&t, std::slice::from_ref(&x))
        };
        assert!((r("max(a.x)") - 1.0).abs() < 1e-6 && (r("min(a.x)") + 1.0).abs() < 1e-6);
        assert!((r("amplitude(a.x)") - 1.0).abs() < 1e-6);
        assert!(r("mean(a.x)").abs() < 1e-9);
        assert!((r("period(a.x)") - 1.0).abs() < 1e-3);
        assert!(r("final(a.x)").abs() < 1e-9);
        assert!((r("first_time(a.x > 0.5)") - 0.084).abs() < 1e-3);
        assert!(r("first_time(a.x > 2)").is_nan());
        // Expressions inside, by path.
        let p = Reducer::parse("e", "max(abs(plant/car.v - 2*car.v/car.m))").unwrap();
        assert_eq!(p.names, ["plant/car.v", "car.v", "car.m"]);
        // Reductions combine as numbers (ADR 0070 step 3); min and max of
        // two stay functions.
        assert!((r("max(a.x) - min(a.x)") - 2.0).abs() < 1e-6);
        assert!((r("max(final(a.x) + 3, 1)") - 3.0).abs() < 1e-9);
        assert!((r("2*amplitude(a.x) + first_time(a.x >= 1)") - 2.25).abs() < 1e-3);
        // A comparison inside any reduction: whether it ever held.
        assert_eq!(r("max(a.x > 0.9)"), 1.0);
        assert_eq!(r("max(a.x > 1.5)"), 0.0);
        // first_time of any expression: when it is first not 0.
        assert_eq!(r("first_time(a.x)"), 0.001);
        let both = Reducer::parse("d", "max(p.v) - final(q.v)").unwrap();
        assert_eq!(both.names, ["p.v", "q.v"]);
        assert_eq!(both.unit_of(), UnitOf::Unknown);
        assert_eq!(
            Reducer::parse("u", "max(car.v)").unwrap().unit_of(),
            UnitOf::Signal("car.v".into())
        );
        assert_eq!(
            Reducer::parse("u", "first_time(car.v > 1)")
                .unwrap()
                .unit_of(),
            UnitOf::Time
        );
        for (bad, says) in [
            ("sum(a.x)", "works on arrays"),
            ("a.x", "outside a reduction"),
            ("max(a.x) + a.x", "outside a reduction"),
            ("max(final(a.x))", "inside another reduction"),
            ("2*3", "reduce the run"),
            ("first_time(a.x, 2)", "one expression"),
        ] {
            let e = Reducer::parse("b", bad).unwrap_err();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }
}
