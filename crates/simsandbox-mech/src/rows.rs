//! A mechanism's `bodies` rows as the model file writes them (ADR 0105
//! point 2): read, checked, and turned into a [`Tree`] and the names of
//! what each row adds.

use crate::tree::{Kind, Tree};
use serde::Deserialize;

/// The parent that is not a body.
pub const GROUND: &str = "ground";

/// One row of `bodies`: a body and the joint to its parent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub name: String,
    pub parent: String,
    pub joint: String,
    pub kind: KindName,
    #[serde(default)]
    pub at: Option<[f64; 2]>,
    #[serde(default)]
    pub turn: Option<f64>,
    pub mass: f64,
    #[serde(default)]
    pub centre: Option<[f64; 2]>,
    #[serde(default)]
    pub inertia: Option<f64>,
    #[serde(default)]
    pub damping: Option<f64>,
    #[serde(default)]
    pub stiffness: Option<f64>,
    #[serde(default)]
    pub rest: Option<f64>,
    #[serde(default)]
    pub start: Option<[f64; 2]>,
}

/// A joint kind as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KindName {
    Revolute,
    Prismatic,
    Fixed,
}

impl From<KindName> for Kind {
    fn from(k: KindName) -> Self {
        match k {
            KindName::Revolute => Kind::Revolute,
            KindName::Prismatic => Kind::Prismatic,
            KindName::Fixed => Kind::Fixed,
        }
    }
}

/// What a joint kind calls its coordinate and rate, and its actuation
/// (ADR 0105 point 4).
pub fn words(kind: Kind) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        Kind::Revolute => Some(("angle", "rate", "torque")),
        Kind::Prismatic => Some(("position", "speed", "force")),
        Kind::Fixed => None,
    }
}

/// Whether `s` is a name: a letter or `_`, then letters, digits or `_`.
fn is_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The rows, read from the block's definition lines (one JSON object
/// each, as the model file's `bodies` field gives them) and checked; an
/// error names the row and column (`bodies[1].parent`).
pub fn read(lines: &[String]) -> Result<Vec<Row>, String> {
    if lines.is_empty() {
        return Err("a mechanism lists its bodies ('bodies'), one row per body".into());
    }
    let rows = lines
        .iter()
        .enumerate()
        .map(|(k, l)| {
            serde_json::from_str::<Row>(l).map_err(|e| {
                if l.trim_start().starts_with('{') {
                    format!("bodies[{k}]: {e}")
                } else {
                    format!(
                        "bodies[{k}]: a mechanism is defined by its bodies, not by equations ({e})"
                    )
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    check(&rows)?;
    Ok(rows)
}

/// The checks of ADR 0105 point 6 that need no numbers evaluated.
fn check(rows: &[Row]) -> Result<(), String> {
    let mut names: Vec<&str> = Vec::new();
    for (k, r) in rows.iter().enumerate() {
        let at = |col: &str| format!("bodies[{k}].{col}");
        for (col, n) in [("name", &r.name), ("joint", &r.joint)] {
            if !is_name(n) {
                return Err(format!(
                    "{}: '{n}' is not a name (letters, digits, _)",
                    at(col)
                ));
            }
            if n == GROUND {
                return Err(format!(
                    "{}: '{GROUND}' is the parent of the first bodies, not a name",
                    at(col)
                ));
            }
            if names.contains(&n.as_str()) {
                return Err(format!(
                    "{}: '{n}' is used twice; bodies and joints need names of their own",
                    at(col)
                ));
            }
            names.push(n);
        }
        if r.parent != GROUND && !rows[..k].iter().any(|b| b.name == r.parent) {
            return Err(format!(
                "{}: '{}' is not '{GROUND}' or a body listed before this one",
                at("parent"),
                r.parent
            ));
        }
        let numbers = [
            ("mass", Some(r.mass)),
            ("inertia", r.inertia),
            ("damping", r.damping),
            ("stiffness", r.stiffness),
            ("turn", r.turn),
            ("rest", r.rest),
        ];
        let pairs = [("at", r.at), ("centre", r.centre), ("start", r.start)];
        for (col, v) in numbers {
            if v.is_some_and(|v| !v.is_finite()) {
                return Err(format!("{}: not a finite number", at(col)));
            }
        }
        for (col, v) in pairs {
            if v.is_some_and(|v| !v.iter().all(|x| x.is_finite())) {
                return Err(format!("{}: not two finite numbers", at(col)));
            }
        }
        for (col, v) in [
            ("mass", Some(r.mass)),
            ("inertia", r.inertia),
            ("damping", r.damping),
            ("stiffness", r.stiffness),
        ] {
            if v.is_some_and(|v| v < 0.0) {
                return Err(format!("{}: must be 0 or more", at(col)));
            }
        }
        if r.kind == KindName::Fixed {
            for (col, set) in [
                ("damping", r.damping.is_some()),
                ("stiffness", r.stiffness.is_some()),
                ("rest", r.rest.is_some()),
                ("start", r.start.is_some()),
            ] {
                if set {
                    return Err(format!("{}: a fixed joint does not move", at(col)));
                }
            }
        }
    }
    Ok(())
}

/// The tree the rows make.
pub fn tree(rows: &[Row]) -> Tree {
    let parents: Vec<_> = rows
        .iter()
        .map(|r| {
            let parent = rows.iter().position(|b| b.name == r.parent);
            (parent, Kind::from(r.kind))
        })
        .collect();
    Tree::new(&parents).expect("checked: parents come first")
}

/// The names a setting lists (`"slider, hinge"`), each checked against
/// `known`; an error names the setting.
pub fn listed(
    setting: &str,
    text: Option<&String>,
    known: &[&str],
    what: &str,
) -> Result<Vec<String>, String> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for n in text.split([',', ' ']).filter(|n| !n.is_empty()) {
        if !known.contains(&n) {
            return Err(format!(
                "settings.{setting}: '{n}' is not {what} of this mechanism"
            ));
        }
        if out.iter().any(|o| o == n) {
            return Err(format!("settings.{setting}: '{n}' is listed twice"));
        }
        out.push(n.to_string());
    }
    Ok(out)
}
