//! SI units with dimension checking.
//!
//! A [`Unit`] is a dimension (exponents of the seven SI base units) plus a
//! scale factor to SI. [`Unit::parse`] reads SI units with prefixes, common
//! non-SI units and US/imperial units at their exact definitions
//! ([`symbols`], ADR 0047); temperatures with an offset (°C, °F) are not
//! units, since a unit here is a scale. Connecting two ports requires the same unit, scale
//! included: converting is always explicit, with a conversion Block
//! (ADR 0045).

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

    /// Parse a unit expression such as `m/s`, `kg*m/s^2`, `kN`, `mm`,
    /// `ft/s`, `N/(m*s)`, `1`.
    ///
    /// Grammar: factors joined by `*`, `·` or `/`, read left to right; a
    /// factor is a symbol from [`symbols`] (with an SI prefix where the
    /// symbol takes one: `kN`, `µs`, `mL`), a positive number (`0.3048`), or
    /// a group in parentheses, each with an optional integer power `^n`.
    /// `a/b*c` is refused as ambiguous: write `a*c/b` or `a/(b*c)`.
    /// Whitespace is ignored; empty, `1` and `-` are dimensionless.
    pub fn parse(text: &str) -> Result<Unit> {
        let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if cleaned.is_empty() || cleaned == "-" {
            return Ok(Unit::DIMENSIONLESS);
        }
        let fail = |why: String| SimError::UnitParse(format!("'{text}' is not a unit: {why}"));
        if let Some(t) = TEMPERATURES.iter().find(|t| cleaned.contains(**t)) {
            return Err(fail(format!(
                "'{t}' has an offset from zero, so it is not a scale of K; use K (a difference \
                 of 1 °C is 1 K) and convert readings at the edge of the model"
            )));
        }
        let chars: Vec<char> = cleaned.chars().collect();
        let mut at = 0;
        let unit = expression(&chars, &mut at).map_err(fail)?;
        if at < chars.len() {
            return Err(fail(format!("unexpected '{}'", chars[at])));
        }
        Ok(unit)
    }

    /// True when values in `self` can be converted to `other`.
    pub fn compatible(&self, other: &Unit) -> bool {
        self.dim == other.dim
    }

    /// The same unit: dimension and scale (to rounding).
    pub fn same_as(&self, other: &Unit) -> bool {
        self.dim == other.dim && (self.scale / other.scale - 1.0).abs() < 1e-12
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

/// Temperature scales with an offset, refused by name.
const TEMPERATURES: [&str; 6] = ["°C", "°F", "degC", "degF", "celsius", "fahrenheit"];

/// `factor (('*' | '·' | '/') factor)*`, left to right.
fn expression(c: &[char], at: &mut usize) -> std::result::Result<Unit, String> {
    let mut unit = factor(c, at)?;
    let mut divided = false;
    while let Some(&op) = c.get(*at) {
        if !matches!(op, '*' | '·' | '/') {
            break;
        }
        if divided && op != '/' {
            return Err("'a/b*c' is ambiguous: write 'a*c/b' or 'a/(b*c)'".into());
        }
        *at += 1;
        let next = factor(c, at)?;
        if op == '/' {
            divided = true;
            unit = unit / next;
        } else {
            unit = unit * next;
        }
    }
    Ok(unit)
}

/// A symbol, a number or a group, with an optional power.
fn factor(c: &[char], at: &mut usize) -> std::result::Result<Unit, String> {
    let start = *at;
    let base = match c.get(*at) {
        None => return Err("a unit is missing at the end".into()),
        Some('(') => {
            *at += 1;
            let inner = expression(c, at)?;
            if c.get(*at) != Some(&')') {
                return Err("a '(' is not closed".into());
            }
            *at += 1;
            inner
        }
        Some(d) if d.is_ascii_digit() || *d == '.' => {
            while c.get(*at).is_some_and(|d| {
                d.is_ascii_digit()
                    || *d == '.'
                    || matches!(d, 'e' | 'E')
                    || (matches!(d, '+' | '-') && matches!(c.get(*at - 1), Some('e' | 'E')))
            }) {
                *at += 1;
            }
            let text: String = c[start..*at].iter().collect();
            match text.parse::<f64>() {
                Ok(v) if v > 0.0 && v.is_finite() => Unit::new([0; 7], v),
                _ => return Err(format!("'{text}' is not a positive number")),
            }
        }
        Some(_) => {
            while c
                .get(*at)
                .is_some_and(|d| !matches!(d, '*' | '·' | '/' | '^' | '(' | ')'))
            {
                *at += 1;
            }
            let symbol: String = c[start..*at].iter().collect();
            if symbol.is_empty() {
                return Err(format!("a unit is missing before '{}'", c[*at]));
            }
            lookup(&symbol).ok_or_else(|| {
                format!(
                    "unknown symbol '{symbol}' (SI units with prefixes such as kN, MPa, µs or \
                     mL; min, h, d, L, t, bar, atm, eV, Wh, cal, deg, rev, rpm, %, ppm; in, ft, \
                     yd, mi, nmi, kn, mph, lb, oz, lbf, psi, gal, hp, BTU, slug; \
                     'simsandbox describe' lists them all)"
                )
            })?
        }
    };
    if c.get(*at) != Some(&'^') {
        return Ok(base);
    }
    *at += 1;
    let from = *at;
    if c.get(*at) == Some(&'-') {
        *at += 1;
    }
    while c.get(*at).is_some_and(char::is_ascii_digit) {
        *at += 1;
    }
    let text: String = c[from..*at].iter().collect();
    let n = text
        .parse::<i8>()
        .map_err(|_| format!("'^{text}': a power is a whole number"))?;
    Ok(base.powi(n))
}

/// A symbol, whole or as an SI prefix and a symbol that takes one.
fn lookup(symbol: &str) -> Option<Unit> {
    if let Some(s) = SYMBOLS.iter().find(|s| s.symbol == symbol) {
        return Some(s.unit);
    }
    PREFIXES.iter().find_map(|(p, factor)| {
        let rest = symbol.strip_prefix(p)?;
        let s = SYMBOLS.iter().find(|s| s.symbol == rest && s.prefixed)?;
        Some(Unit {
            dim: s.unit.dim,
            scale: factor * s.unit.scale,
        })
    })
}

/// SI prefixes; `da` before the one-letter ones.
const PREFIXES: [(&str, f64); 18] = [
    ("da", 1e1),
    ("Q", 1e30),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("µ", 1e-6),
    ("μ", 1e-6),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
];

/// A unit symbol the parser knows.
#[derive(Debug, Clone, Copy)]
pub struct Symbol {
    pub symbol: &'static str,
    pub name: &'static str,
    pub unit: Unit,
    /// Whether SI prefixes apply (`kN`, `mL`).
    pub prefixed: bool,
}

const fn sym(
    symbol: &'static str,
    name: &'static str,
    dim: [i8; 7],
    scale: f64,
    prefixed: bool,
) -> Symbol {
    Symbol {
        symbol,
        name,
        unit: Unit::new(dim, scale),
        prefixed,
    }
}

const POUND: f64 = 0.453_592_37;
const POUND_FORCE: f64 = POUND * 9.806_65;
const INCH: f64 = 0.0254;
const FOOT: f64 = 0.3048;

/// Every symbol, in the order units are shown by name (the first match
/// wins). Non-SI factors are the exact legal definitions (international
/// inch, foot and pound; US gallon; IT BTU; thermochemical calorie;
/// mechanical horsepower).
const SYMBOLS: [Symbol; 55] = [
    sym("m", "metre", [1, 0, 0, 0, 0, 0, 0], 1.0, true),
    sym("g", "gram", [0, 1, 0, 0, 0, 0, 0], 1e-3, true),
    sym("s", "second", [0, 0, 1, 0, 0, 0, 0], 1.0, true),
    sym("A", "ampere", [0, 0, 0, 1, 0, 0, 0], 1.0, true),
    sym("K", "kelvin", [0, 0, 0, 0, 1, 0, 0], 1.0, true),
    sym("mol", "mole", [0, 0, 0, 0, 0, 1, 0], 1.0, true),
    sym("cd", "candela", [0, 0, 0, 0, 0, 0, 1], 1.0, true),
    sym("Hz", "hertz", [0, 0, -1, 0, 0, 0, 0], 1.0, true),
    sym("N", "newton", [1, 1, -2, 0, 0, 0, 0], 1.0, true),
    sym("Pa", "pascal", [-1, 1, -2, 0, 0, 0, 0], 1.0, true),
    sym("J", "joule", [2, 1, -2, 0, 0, 0, 0], 1.0, true),
    sym("W", "watt", [2, 1, -3, 0, 0, 0, 0], 1.0, true),
    sym("C", "coulomb", [0, 0, 1, 1, 0, 0, 0], 1.0, true),
    sym("V", "volt", [2, 1, -3, -1, 0, 0, 0], 1.0, true),
    sym("F", "farad", [-2, -1, 4, 2, 0, 0, 0], 1.0, true),
    sym("Ohm", "ohm", [2, 1, -3, -2, 0, 0, 0], 1.0, true),
    sym("Ω", "ohm", [2, 1, -3, -2, 0, 0, 0], 1.0, true),
    sym("S", "siemens", [-2, -1, 3, 2, 0, 0, 0], 1.0, true),
    sym("Wb", "weber", [2, 1, -2, -1, 0, 0, 0], 1.0, true),
    sym("T", "tesla", [0, 1, -2, -1, 0, 0, 0], 1.0, true),
    sym("H", "henry", [2, 1, -2, -2, 0, 0, 0], 1.0, true),
    sym("rad", "radian", [0; 7], 1.0, true),
    sym("min", "minute", [0, 0, 1, 0, 0, 0, 0], 60.0, false),
    sym("h", "hour", [0, 0, 1, 0, 0, 0, 0], 3600.0, false),
    sym("d", "day", [0, 0, 1, 0, 0, 0, 0], 86400.0, false),
    sym("L", "litre", [3, 0, 0, 0, 0, 0, 0], 1e-3, true),
    sym("l", "litre", [3, 0, 0, 0, 0, 0, 0], 1e-3, true),
    sym("t", "tonne", [0, 1, 0, 0, 0, 0, 0], 1e3, true),
    sym("ha", "hectare", [2, 0, 0, 0, 0, 0, 0], 1e4, false),
    sym("bar", "bar", [-1, 1, -2, 0, 0, 0, 0], 1e5, true),
    sym(
        "atm",
        "standard atmosphere",
        [-1, 1, -2, 0, 0, 0, 0],
        101_325.0,
        false,
    ),
    sym(
        "eV",
        "electronvolt",
        [2, 1, -2, 0, 0, 0, 0],
        1.602_176_634e-19,
        true,
    ),
    sym("Wh", "watt-hour", [2, 1, -2, 0, 0, 0, 0], 3600.0, true),
    sym(
        "cal",
        "calorie (thermochemical)",
        [2, 1, -2, 0, 0, 0, 0],
        4.184,
        true,
    ),
    sym(
        "deg",
        "degree of angle",
        [0; 7],
        std::f64::consts::PI / 180.0,
        false,
    ),
    sym(
        "°",
        "degree of angle",
        [0; 7],
        std::f64::consts::PI / 180.0,
        false,
    ),
    sym(
        "rev",
        "revolution",
        [0; 7],
        2.0 * std::f64::consts::PI,
        false,
    ),
    sym(
        "rpm",
        "revolution per minute",
        [0, 0, -1, 0, 0, 0, 0],
        2.0 * std::f64::consts::PI / 60.0,
        false,
    ),
    sym("%", "percent", [0; 7], 1e-2, false),
    sym("ppm", "part per million", [0; 7], 1e-6, false),
    sym("in", "inch", [1, 0, 0, 0, 0, 0, 0], INCH, false),
    sym("ft", "foot", [1, 0, 0, 0, 0, 0, 0], FOOT, false),
    sym("yd", "yard", [1, 0, 0, 0, 0, 0, 0], 0.9144, false),
    sym("mi", "mile", [1, 0, 0, 0, 0, 0, 0], 1609.344, false),
    sym("nmi", "nautical mile", [1, 0, 0, 0, 0, 0, 0], 1852.0, false),
    sym("kn", "knot", [1, 0, -1, 0, 0, 0, 0], 1852.0 / 3600.0, false),
    sym(
        "mph",
        "mile per hour",
        [1, 0, -1, 0, 0, 0, 0],
        1609.344 / 3600.0,
        false,
    ),
    sym("lb", "pound (mass)", [0, 1, 0, 0, 0, 0, 0], POUND, false),
    sym("oz", "ounce", [0, 1, 0, 0, 0, 0, 0], POUND / 16.0, false),
    sym(
        "lbf",
        "pound-force",
        [1, 1, -2, 0, 0, 0, 0],
        POUND_FORCE,
        false,
    ),
    sym(
        "psi",
        "pound-force per square inch",
        [-1, 1, -2, 0, 0, 0, 0],
        POUND_FORCE / (INCH * INCH),
        false,
    ),
    sym(
        "gal",
        "US gallon",
        [3, 0, 0, 0, 0, 0, 0],
        3.785_411_784e-3,
        false,
    ),
    sym(
        "hp",
        "horsepower (mechanical)",
        [2, 1, -3, 0, 0, 0, 0],
        745.699_871_582_270_2,
        false,
    ),
    sym(
        "BTU",
        "British thermal unit (IT)",
        [2, 1, -2, 0, 0, 0, 0],
        1_055.055_852_62,
        false,
    ),
    sym(
        "slug",
        "slug",
        [0, 1, 0, 0, 0, 0, 0],
        POUND_FORCE / FOOT,
        false,
    ),
];

/// Every unit symbol the parser knows, for `describe` (ADR 0039).
pub fn symbols() -> impl Iterator<Item = &'static Symbol> {
    SYMBOLS.iter()
}

/// The SI prefixes and their factors.
pub fn prefixes() -> &'static [(&'static str, f64)] {
    &PREFIXES
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

/// Compound units shown by name when no single symbol fits.
const NAMED: [&str; 12] = [
    "km/h", "km/s", "cm/s", "mm/s", "g/s", "kg/h", "km^2", "cm^2", "mm^2", "cm^3", "mm^3", "ft/s",
];

/// Names for units that are not SI (scale 1), in order of preference:
/// symbols, prefixed symbols, then compounds.
fn names() -> &'static [(String, Unit)] {
    static NAMES: std::sync::OnceLock<Vec<(String, Unit)>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        // One spelling of each: no 'l', 'Ω', 'μ' or 'u' (shown as L, Ohm, µ).
        let symbols = SYMBOLS
            .iter()
            .filter(|s| !matches!(s.symbol, "l" | "Ω" | "°"));
        let mut names: Vec<(String, Unit)> = symbols
            .clone()
            .map(|s| (s.symbol.to_string(), s.unit))
            .collect();
        for s in symbols.filter(|s| s.prefixed) {
            for (p, _) in PREFIXES.iter().filter(|(p, _)| !matches!(*p, "μ" | "u")) {
                let name = format!("{p}{}", s.symbol);
                if let Some(u) = lookup(&name) {
                    names.push((name, u));
                }
            }
        }
        for n in NAMED {
            if let Ok(u) = Unit::parse(n) {
                names.push((n.to_string(), u));
            }
        }
        names
    })
}

impl fmt::Display for Unit {
    /// Named units by name (`km/h`); others as SI base units with positive
    /// powers first and negative ones after a slash (`m/s^2`,
    /// `kg/(m·s^2)`), which [`Unit::parse`] reads back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale != 1.0 {
            if let Some((name, _)) = names().iter().find(|(_, u)| u.same_as(self)) {
                return write!(f, "{name}");
            }
        }
        let power = |s: &str, e: i8| {
            if e == 1 {
                s.to_string()
            } else {
                format!("{s}^{e}")
            }
        };
        // Mass first, as in kg·m^2.
        let base = || {
            [1, 0, 2, 3, 4, 5, 6]
                .into_iter()
                .map(|k| (BASE_SYMBOLS[k], self.dim.0[k]))
        };
        let mut num: Vec<String> = base()
            .filter(|(_, e)| *e > 0)
            .map(|(s, e)| power(s, e))
            .collect();
        let den: Vec<String> = base()
            .filter(|(_, e)| *e < 0)
            .map(|(s, e)| power(s, -e))
            .collect();
        if self.scale != 1.0 {
            num.insert(0, self.scale.to_string());
        }
        let top = if num.is_empty() {
            "1".to_string()
        } else {
            num.join("·")
        };
        match den.len() {
            0 => write!(f, "{top}"),
            1 => write!(f, "{top}/{}", den[0]),
            _ => write!(f, "{top}/({})", den.join("·")),
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

    #[test]
    fn units_match_only_when_the_same_and_scaled_ones_show_their_names() {
        let kmh = Unit::parse("km/h").unwrap();
        let ms = Unit::parse("m/s").unwrap();
        assert!(kmh.compatible(&ms) && !kmh.same_as(&ms));
        assert!(Unit::parse("N*s/m")
            .unwrap()
            .same_as(&Unit::parse("kg/s").unwrap()));
        assert_eq!(kmh.to_string(), "km/h");
        // Shown units read back.
        let force = Unit::parse("N").unwrap();
        assert!(Unit::parse(&force.to_string()).unwrap().same_as(&force));
        assert_eq!(Unit::parse("g").unwrap().to_string(), "g");
        assert_eq!(ms.to_string(), "m/s");
        for (text, shown) in [
            ("m/s^2", "m/s^2"),
            ("kg/(m*s^2)", "kg/(m·s^2)"),
            ("1/s", "1/s"),
            ("kg*m^2", "kg·m^2"),
            ("N*s/m", "kg/s"),
        ] {
            let u = Unit::parse(text).unwrap();
            assert_eq!(u.to_string(), shown);
            assert!(Unit::parse(shown).unwrap().same_as(&u));
        }
        assert!((kmh.conversion_to(&ms).unwrap() - 1.0 / 3.6).abs() < 1e-15);
    }

    fn si(text: &str) -> f64 {
        Unit::parse(text).unwrap_or_else(|e| panic!("{e}")).scale
    }

    fn same(a: &str, b: &str) -> bool {
        Unit::parse(a).unwrap().same_as(&Unit::parse(b).unwrap())
    }

    #[test]
    fn prefixes_apply_to_si_symbols() {
        assert!(same("kN", "1000*N"));
        assert!(same("MPa", "N/mm^2"));
        assert!(same("µs", "1e-6*s") && same("us", "µs") && same("μs", "µs"));
        assert!(same("mL", "cm^3") && same("dam", "10*m") && same("hPa", "mbar"));
        assert!(same("kg", "1000*g") && same("Mg", "t"));
        assert!(same("kWh", "3.6*MJ") && same("keV", "1000*eV"));
        // Whole symbols win over prefix readings, and only some symbols
        // take prefixes.
        assert_eq!(si("min"), 60.0);
        assert_eq!(si("cd"), 1.0);
        assert!(same("Pa", "N/m^2") && same("ha", "hm^2"));
        assert!(Unit::parse("kft").is_err() && Unit::parse("kh").is_err());
    }

    #[test]
    fn non_si_units_have_their_exact_definitions() {
        assert_eq!(si("ft"), 0.3048);
        assert_eq!(si("in"), 0.0254);
        assert_eq!(si("mi"), 1609.344);
        assert_eq!(si("lb"), 0.453_592_37);
        assert!((si("lbf") - 4.448_221_615_260_5).abs() < 1e-12);
        assert!((si("psi") - 6_894.757_293_168_361).abs() < 1e-9);
        assert!((si("mph") - 0.447_04).abs() < 1e-15);
        assert!((si("kn") - 0.514_444_444_444_444_4).abs() < 1e-15);
        assert!((si("gal") / si("in^3") - 231.0).abs() < 1e-9);
        assert!((si("slug") - 14.593_902_937_206_364).abs() < 1e-9);
        assert!((si("deg") * 180.0 - std::f64::consts::PI).abs() < 1e-15);
        assert!((si("rpm") - 2.0 * std::f64::consts::PI / 60.0).abs() < 1e-15);
        assert_eq!(si("%"), 0.01);
        assert!(same("L", "dm^3") && same("bar", "100*kPa") && same("atm", "101.325*kPa"));
        assert!(Unit::parse("ft/s")
            .unwrap()
            .compatible(&Unit::parse("m/s").unwrap()));
    }

    #[test]
    fn grammar_reads_left_to_right_with_groups_and_numbers() {
        assert!(same("N/(m*s)", "N/m/s"));
        assert!(same("(m/s)^2", "m^2/s^2"));
        assert!(same("1/s^2", "Hz^2"));
        assert!(same("0.3048*m", "ft"));
        assert!(same("1e-5·kg·m", "g*cm"));
        let err = Unit::parse("N/m*s").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        for bad in ["m//s", "(m", "m^x", "-2*m", "furlong", "m^"] {
            assert!(Unit::parse(bad).is_err(), "{bad}");
        }
        let err = Unit::parse("furlong").unwrap_err().to_string();
        assert!(err.contains("unknown symbol 'furlong'"), "{err}");
    }

    #[test]
    fn temperatures_with_an_offset_are_refused() {
        for t in ["°C", "degC", "°F", "degF/s"] {
            let err = Unit::parse(t).unwrap_err().to_string();
            assert!(err.contains("offset") && err.contains("K"), "{err}");
        }
        assert!(Unit::parse("K/s").is_ok());
    }

    #[test]
    fn every_unit_is_shown_in_a_form_that_reads_back() {
        for (text, shown) in [
            ("kN", "kN"),
            ("ft", "ft"),
            ("t", "t"),
            ("mL", "mL"),
            ("km/h", "km/h"),
            ("deg", "deg"),
            ("µs", "µs"),
            ("kPa", "kPa"),
        ] {
            assert_eq!(Unit::parse(text).unwrap().to_string(), shown);
        }
        for text in [
            "g*cm",
            "lbf*ft",
            "kWh/km",
            "psi*in^2",
            "N/(m*s)",
            "slug/ft^3",
        ] {
            let u = Unit::parse(text).unwrap();
            let back = Unit::parse(&u.to_string()).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert!(back.same_as(&u), "{text} shown as {u}");
        }
        assert!(symbols().count() > 50 && prefixes().len() > 15);
    }

    #[test]
    fn whole_symbols_hide_only_the_prefix_readings_listed_here() {
        // A whole symbol wins over a prefix reading of the same letters;
        // these are all such cases, so adding a symbol cannot silently
        // change what an existing one means.
        let mut hidden = Vec::new();
        for s in symbols() {
            for (p, _) in prefixes() {
                if let Some(rest) = s.symbol.strip_prefix(p) {
                    if symbols().any(|t| t.symbol == rest && t.prefixed) {
                        hidden.push(s.symbol);
                    }
                }
            }
        }
        // ft: foot, not femtotonne.
        assert_eq!(hidden, ["ft"]);
    }
}
