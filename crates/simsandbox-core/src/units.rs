//! SI units with dimension checking.
//!
//! A [`Unit`] is a dimension (exponents of the seven SI base units) plus a
//! scale factor to SI. Connecting two ports requires equal dimensions; the
//! kernel then applies the scale ratio automatically.

use crate::{Result, SimError};
use std::fmt;
use std::ops::{Div, Mul};

/// Exponents of the SI base units: m, kg, s, A, K, mol, cd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dimension(pub [i8; 7]);

const BASE_SYMBOLS: [&str; 7] = ["m", "kg", "s", "A", "K", "mol", "cd"];

impl Dimension {
    pub const NONE: Dimension = Dimension([0; 7]);

    fn combine(self, other: Dimension, sign: i8) -> Dimension {
        let mut out = self.0;
        for (o, b) in out.iter_mut().zip(other.0) {
            *o += sign * b;
        }
        Dimension(out)
    }

    fn pow(self, n: i8) -> Dimension {
        Dimension(self.0.map(|e| e * n))
    }
}

/// A unit: a dimension and the factor that converts a value in this unit to SI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub dim: Dimension,
    pub scale: f64,
}

impl Unit {
    pub const fn new(dim: [i8; 7], scale: f64) -> Unit {
        Unit {
            dim: Dimension(dim),
            scale,
        }
    }

    pub const DIMENSIONLESS: Unit = Unit::new([0, 0, 0, 0, 0, 0, 0], 1.0);
    pub const METER: Unit = Unit::new([1, 0, 0, 0, 0, 0, 0], 1.0);
    pub const KILOGRAM: Unit = Unit::new([0, 1, 0, 0, 0, 0, 0], 1.0);
    pub const SECOND: Unit = Unit::new([0, 0, 1, 0, 0, 0, 0], 1.0);
    pub const AMPERE: Unit = Unit::new([0, 0, 0, 1, 0, 0, 0], 1.0);
    pub const KELVIN: Unit = Unit::new([0, 0, 0, 0, 1, 0, 0], 1.0);
    pub const NEWTON: Unit = Unit::new([1, 1, -2, 0, 0, 0, 0], 1.0);
    pub const VOLT: Unit = Unit::new([2, 1, -3, -1, 0, 0, 0], 1.0);

    /// Parse a unit expression such as `m/s`, `kg*m/s^2`, `N`, `mm`, `1`.
    ///
    /// Grammar: factors separated by `*` or `/`, each factor a known symbol
    /// with an optional integer exponent `^n`. Everything after a `/` is in
    /// the denominator. Whitespace is ignored.
    pub fn parse(text: &str) -> Result<Unit> {
        let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if cleaned.is_empty() || cleaned == "1" || cleaned == "-" {
            return Ok(Unit::DIMENSIONLESS);
        }
        let mut unit = Unit::DIMENSIONLESS;
        let mut divide = false;
        let mut token = String::new();
        for c in cleaned.chars().chain(std::iter::once('*')) {
            if c == '*' || c == '/' {
                if token.is_empty() {
                    return Err(SimError::UnitParse(text.to_string()));
                }
                let factor = parse_factor(&token).ok_or(SimError::UnitParse(text.to_string()))?;
                unit = if divide { unit / factor } else { unit * factor };
                token.clear();
                if c == '/' {
                    divide = true;
                }
            } else {
                token.push(c);
            }
        }
        Ok(unit)
    }

    /// True when values in `self` can be converted to `other`.
    pub fn compatible(&self, other: &Unit) -> bool {
        self.dim == other.dim
    }

    /// Factor `k` such that `value_in_other = k * value_in_self`.
    pub fn conversion_to(&self, other: &Unit) -> Option<f64> {
        self.compatible(other).then(|| self.scale / other.scale)
    }

    fn powi(self, n: i8) -> Unit {
        Unit {
            dim: self.dim.pow(n),
            scale: self.scale.powi(n as i32),
        }
    }
}

fn parse_factor(token: &str) -> Option<Unit> {
    let (symbol, exp) = match token.split_once('^') {
        Some((s, e)) => (s, e.parse::<i8>().ok()?),
        None => (token, 1),
    };
    Some(named_unit(symbol)?.powi(exp))
}

fn named_unit(symbol: &str) -> Option<Unit> {
    let u = match symbol {
        "m" => Unit::METER,
        "cm" => Unit::new([1, 0, 0, 0, 0, 0, 0], 1e-2),
        "mm" => Unit::new([1, 0, 0, 0, 0, 0, 0], 1e-3),
        "km" => Unit::new([1, 0, 0, 0, 0, 0, 0], 1e3),
        "kg" => Unit::KILOGRAM,
        "g" => Unit::new([0, 1, 0, 0, 0, 0, 0], 1e-3),
        "s" => Unit::SECOND,
        "ms" => Unit::new([0, 0, 1, 0, 0, 0, 0], 1e-3),
        "min" => Unit::new([0, 0, 1, 0, 0, 0, 0], 60.0),
        "h" => Unit::new([0, 0, 1, 0, 0, 0, 0], 3600.0),
        "A" => Unit::AMPERE,
        "K" => Unit::KELVIN,
        "mol" => Unit::new([0, 0, 0, 0, 0, 1, 0], 1.0),
        "cd" => Unit::new([0, 0, 0, 0, 0, 0, 1], 1.0),
        "Hz" => Unit::new([0, 0, -1, 0, 0, 0, 0], 1.0),
        "N" => Unit::NEWTON,
        "Pa" => Unit::new([-1, 1, -2, 0, 0, 0, 0], 1.0),
        "J" => Unit::new([2, 1, -2, 0, 0, 0, 0], 1.0),
        "W" => Unit::new([2, 1, -3, 0, 0, 0, 0], 1.0),
        "V" => Unit::VOLT,
        "Ohm" => Unit::new([2, 1, -3, -2, 0, 0, 0], 1.0),
        "rad" | "1" => Unit::DIMENSIONLESS,
        _ => return None,
    };
    Some(u)
}

impl Mul for Unit {
    type Output = Unit;
    fn mul(self, rhs: Unit) -> Unit {
        Unit {
            dim: self.dim.combine(rhs.dim, 1),
            scale: self.scale * rhs.scale,
        }
    }
}

impl Div for Unit {
    type Output = Unit;
    fn div(self, rhs: Unit) -> Unit {
        Unit {
            dim: self.dim.combine(rhs.dim, -1),
            scale: self.scale / rhs.scale,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale != 1.0 {
            write!(f, "{}·", self.scale)?;
        }
        let parts: Vec<String> = BASE_SYMBOLS
            .iter()
            .zip(self.dim.0)
            .filter(|(_, e)| *e != 0)
            .map(|(s, e)| {
                if e == 1 {
                    s.to_string()
                } else {
                    format!("{s}^{e}")
                }
            })
            .collect();
        if parts.is_empty() {
            write!(f, "1")
        } else {
            write!(f, "{}", parts.join("·"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compound_units() {
        assert_eq!(Unit::parse("kg*m/s^2").unwrap(), Unit::NEWTON);
        assert_eq!(Unit::parse("N").unwrap(), Unit::NEWTON);
        assert!(Unit::parse("m/s")
            .unwrap()
            .compatible(&Unit::parse("km/h").unwrap()));
    }

    #[test]
    fn converts_scaled_units() {
        let kmh = Unit::parse("km/h").unwrap();
        let ms = Unit::parse("m/s").unwrap();
        let k = kmh.conversion_to(&ms).unwrap();
        assert!((k - 1.0 / 3.6).abs() < 1e-12);
    }

    #[test]
    fn rejects_incompatible_and_unknown() {
        assert!(Unit::parse("m")
            .unwrap()
            .conversion_to(&Unit::SECOND)
            .is_none());
        assert!(Unit::parse("furlong").is_err());
        assert!(Unit::parse("m//s").is_err());
    }
}
