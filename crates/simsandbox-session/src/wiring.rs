//! Connecting by intent (the Structure lens, ADR 0108; the interface
//! report, section 6): what can feed an input or drive a parameter, and
//! what can go from one block to another. The Inspector's "Drive from…",
//! a block dragged onto another and the headless `sources` all ask this,
//! and each makes the same edit (`Edit::Connect`). A connection is offered
//! when its units are the same and its shapes fit; `check` still has the
//! last word.

use crate::diagram::{Diagram, Node};
use simsandbox_core::block::Shape;
use simsandbox_core::units::Unit;

/// An output that can feed a target.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// As a wire names it: `block.port`, or a used model's `use/port`.
    pub signal: String,
    pub unit: String,
    /// It has the target's name (`F` for `F`), so it is offered first.
    pub same_name: bool,
}

/// A connection from one block to another: an output and the input or
/// parameter it would feed.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub from: String,
    pub to: String,
    /// The input or parameter is fed or set by something else now: the
    /// connection would replace that.
    pub replaces: bool,
    /// Into a parameter, which it then drives (ADR 0075).
    pub drives: bool,
}

/// What a wire names a port of `n` by.
fn signal(n: &Node, port: &str) -> String {
    if n.kind == crate::diagram::NodeKind::Use {
        format!("{}/{port}", n.name)
    } else {
        format!("{}.{port}", n.name)
    }
}

/// The same unit, written either way (`""` is dimensionless).
fn same_unit(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        if s.is_empty() {
            Ok(Unit::DIMENSIONLESS)
        } else {
            Unit::parse(s)
        }
    };
    match (parse(a), parse(b)) {
        (Ok(a), Ok(b)) => a.same_as(&b),
        _ => a == b,
    }
}

/// What can be fed on node `to`: its inputs, then its parameter values,
/// each with its unit, shape and whether something feeds it now.
fn targets(d: &Diagram, to: usize) -> Vec<(String, String, Shape, bool, bool)> {
    let n = &d.nodes[to];
    let fed = |name: &str| d.wires.iter().any(|w| w.to_name == name);
    n.inputs
        .iter()
        .map(|p| {
            let key = signal(n, &p.name);
            let now = fed(&key);
            (key, p.unit.clone(), p.shape, now, false)
        })
        .chain(
            n.values
                .iter()
                .filter(|v| v.formula.is_none() && !v.initial)
                .map(|v| {
                    let now = v.driven.is_some() || v.exposed.is_some();
                    (v.key.clone(), v.unit.clone(), Shape::Scalar, now, true)
                }),
        )
        .collect()
}

/// Every output of the model's blocks and uses that can feed `target`
/// (`block.port`, `use/port` or a parameter `block.param`), not from its
/// own block: those with its name first, then in the order of the nodes.
pub fn sources(d: &Diagram, target: &str) -> Result<Vec<Source>, String> {
    let (to, unit, shape) = d
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_block())
        .find_map(|(i, _)| {
            targets(d, i)
                .into_iter()
                .find(|t| t.0 == target)
                .map(|t| (i, t.1, t.2))
        })
        .ok_or_else(|| format!("'{target}' is no input or parameter of the model's blocks"))?;
    let name = target.rsplit(['.', '/']).next().unwrap_or(target);
    let mut out: Vec<Source> = d
        .nodes
        .iter()
        .enumerate()
        .filter(|&(i, n)| i != to && n.is_block())
        .flat_map(|(_, n)| {
            n.outputs
                .iter()
                .filter(|p| p.shape == shape && same_unit(&p.unit, &unit))
                .map(|p| Source {
                    signal: signal(n, &p.name),
                    unit: p.unit.clone(),
                    same_name: p.name == name,
                })
        })
        .collect();
    // Its name first, then units written alike (`rad` before a
    // dimensionless number, which is the same unit).
    out.sort_by_key(|s| (!s.same_name, s.unit != unit));
    Ok(out)
}

/// Every connection from block `from`'s outputs to block `to`'s inputs and
/// parameters that fits: by input, those with the same name first, then
/// units written alike; inputs nothing feeds before those it would take
/// over.
pub fn pairs(d: &Diagram, from: &str, to: &str) -> Vec<Pair> {
    let (Some(a), Some(b)) = (d.node(from), d.node(to)) else {
        return Vec::new();
    };
    if a == b || !d.nodes[a].is_block() || !d.nodes[b].is_block() {
        return Vec::new();
    }
    let src = &d.nodes[a];
    let mut out = Vec::new();
    for (key, unit, shape, fed, drives) in targets(d, b) {
        let name = key.rsplit(['.', '/']).next().unwrap_or(&key).to_string();
        let mut fits: Vec<&crate::diagram::Port> = src
            .outputs
            .iter()
            .filter(|p| p.shape == shape && same_unit(&p.unit, &unit))
            .collect();
        fits.sort_by_key(|p| (p.name != name, p.unit != unit));
        for p in fits {
            let from = signal(src, &p.name);
            // Already so.
            if d.wires
                .iter()
                .any(|w| w.from_name == from && w.to_name == key)
            {
                continue;
            }
            out.push(Pair {
                from,
                to: key.clone(),
                replaces: fed,
                drives,
            });
        }
    }
    out.sort_by_key(|p| p.replaces);
    out
}

/// Whether a wire may run from `from` (an output) into `to`.
pub fn fits(d: &Diagram, from: &str, to: &str) -> bool {
    sources(d, to).is_ok_and(|s| s.iter().any(|s| s.signal == from))
}
