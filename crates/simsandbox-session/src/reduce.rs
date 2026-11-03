//! `reduce` formulas of an experiment (ADR 0067 step 3): each a named
//! value made of one run, written as one reduction of an expression over
//! the run's signals.
//!
//! - `final(e)`, `min(e)`, `max(e)`: the last, least and greatest value.
//! - `mean(e)`: the mean over time (trapezoids), or of the samples when
//!   the run has one.
//! - `amplitude(e)`: half of max − min.
//! - `period(e)`: the mean time between upward crossings of `e`'s mean,
//!   NaN with fewer than two.
//! - `first_time(a < b)` (also `>`, `<=`, `>=`): the first time the
//!   comparison holds, NaN if it never does.
//!
//! Names are signals by full name (`car.v`, `plant/car.v`,
//! `lorenz.xyz[0]`) and `t`.

use simsandbox_expr::{parse_signals, Expr};

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
}

/// One `reduce` formula, parsed.
#[derive(Debug, Clone)]
pub struct Reducer {
    pub name: String,
    kind: Kind,
    /// The expression (the left side for `first_time`), and the right
    /// side with its comparison.
    sides: Vec<Expr>,
    cmp: Option<Cmp>,
    /// The names the expressions read, by slot.
    pub names: Vec<String>,
}

/// The comparison at the top level of `s`, if any: its place, length and
/// kind.
fn comparison(s: &str) -> Option<(usize, usize, Cmp)> {
    let mut depth = 0i32;
    let b = s.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'<' | b'>' if depth == 0 => {
                let eq = b.get(i + 1) == Some(&b'=');
                let cmp = match (b[i], eq) {
                    (b'<', false) => Cmp::Lt,
                    (b'<', true) => Cmp::Le,
                    (_, false) => Cmp::Gt,
                    (_, true) => Cmp::Ge,
                };
                return Some((i, 1 + usize::from(eq), cmp));
            }
            _ => {}
        }
    }
    None
}

impl Reducer {
    /// Parse `text` (one reduction call) as the formula `name`.
    pub fn parse(name: &str, text: &str) -> Result<Self, String> {
        let text = text.trim();
        let open = text.find('(').filter(|_| text.ends_with(')'));
        let Some(open) = open else {
            return Err(format!(
                "{name}: write one reduction, e.g. max(car.v) or first_time(car.v > 20)"
            ));
        };
        let kind = match text[..open].trim() {
            "final" => Kind::Final,
            "mean" => Kind::Mean,
            "min" => Kind::Min,
            "max" => Kind::Max,
            "amplitude" => Kind::Amplitude,
            "period" => Kind::Period,
            "first_time" => Kind::FirstTime,
            other => {
                return Err(format!(
                    "{name}: '{other}' is not a reduction (final, mean, min, max, amplitude, period, first_time)"
                ))
            }
        };
        let inner = &text[open + 1..text.len() - 1];
        let (parts, cmp) = match (kind, comparison(inner)) {
            (Kind::FirstTime, Some((at, len, cmp))) => {
                (vec![&inner[..at], &inner[at + len..]], Some(cmp))
            }
            (Kind::FirstTime, None) => {
                return Err(format!(
                    "{name}: first_time takes a comparison, e.g. first_time(car.v > 20)"
                ))
            }
            (_, Some(_)) => return Err(format!("{name}: only first_time takes a comparison")),
            (_, None) => (vec![inner], None),
        };
        let mut names: Vec<String> = Vec::new();
        let mut sides = Vec::new();
        for part in parts {
            let ast = parse_signals(part).map_err(|e| format!("{name}: {}", e.message))?;
            for (n, _) in ast.names() {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            let slot = |n: &str| names.iter().position(|x| x == n);
            sides.push(
                ast.resolve(&slot)
                    .map_err(|e| format!("{name}: {}", e.message))?,
            );
        }
        Ok(Self {
            name: name.to_string(),
            kind,
            sides,
            cmp,
            names,
        })
    }

    /// The value of one run: `t` its times, `series` each name's values
    /// at them (in the order of [`Reducer::names`]).
    pub fn eval(&self, t: &[f64], series: &[Vec<f64>]) -> f64 {
        let at = |k: usize| -> Vec<f64> { series.iter().map(|s| s[k]).collect() };
        let values = |e: &Expr| -> Vec<f64> { (0..t.len()).map(|k| e.eval(&at(k))).collect() };
        let v = values(&self.sides[0]);
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
        match self.kind {
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
            Kind::FirstTime => {
                let r = values(&self.sides[1]);
                let holds = |a: f64, b: f64| match self.cmp.expect("first_time has one") {
                    Cmp::Lt => a < b,
                    Cmp::Le => a <= b,
                    Cmp::Gt => a > b,
                    Cmp::Ge => a >= b,
                };
                (0..n)
                    .find(|&k| holds(v[k], r[k]))
                    .map_or(f64::NAN, |k| t[k])
            }
        }
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
        for bad in ["sum(a.x)", "max(a.x > 1)", "first_time(a.x)", "a.x"] {
            assert!(Reducer::parse("b", bad).is_err(), "{bad}");
        }
    }
}
