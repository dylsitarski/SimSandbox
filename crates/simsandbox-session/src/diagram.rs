//! The model graph as the node canvas shows it (ADR 0042): blocks with
//! their ports, wires, positions (saved as `at`, or laid out left to right
//! by signal flow), and the edits the canvas makes, as changes to the
//! model file. Shared by the app and headless rendering.

use serde_json::Value;
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::{BlockConfig, Registry};
use simsandbox_file::{BlockEntry, ConnectionEntry, ModelFile};
use std::collections::{BTreeMap, BTreeSet};

/// Node size in canvas units.
pub const NODE_WIDTH: f64 = 170.0;
pub const HEADER: f64 = 40.0;
pub const ROW: f64 = 20.0;
/// Space between automatically placed nodes.
const GAP_X: f64 = 90.0;
const GAP_Y: f64 = 40.0;

/// One port on a node.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub unit: String,
    pub shape: Shape,
}

/// One block.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub type_name: String,
    pub method: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// Top-left corner in canvas units.
    pub at: [f64; 2],
    /// Whether `at` comes from the file (else it was laid out).
    pub placed: bool,
    /// Why the block cannot be built, if it cannot.
    pub error: Option<String>,
}

impl Node {
    pub fn height(&self) -> f64 {
        HEADER + ROW * self.inputs.len().max(self.outputs.len()).max(1) as f64
    }

    /// Where a port's wire attaches, in canvas units.
    pub fn port_at(&self, causality: Causality, index: usize) -> [f64; 2] {
        let y = self.at[1] + HEADER + ROW * (index as f64 + 0.5);
        match causality {
            Causality::Input => [self.at[0], y],
            Causality::Output => [self.at[0] + NODE_WIDTH, y],
        }
    }
}

/// A connection, by block and port index.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub from: (usize, usize),
    pub to: (usize, usize),
    /// `"block.port"` as in the file.
    pub from_name: String,
    pub to_name: String,
}

/// The whole graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagram {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    /// Connections that name a block or port that does not exist.
    pub dangling: Vec<String>,
}

impl Diagram {
    /// The graph of a model file, each block built alone so one that fails
    /// still shows (with its error and no ports).
    pub fn new(file: &ModelFile, registry: &Registry) -> Self {
        let mut nodes: Vec<Node> = file.blocks.iter().map(|b| node(b, registry)).collect();
        let index: BTreeMap<&str, usize> = file
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.as_str(), i))
            .collect();
        let find = |key: &str, causality: Causality, nodes: &[Node]| {
            let (b, p) = key.split_once('.')?;
            let &n = index.get(b)?;
            let ports = match causality {
                Causality::Input => &nodes[n].inputs,
                Causality::Output => &nodes[n].outputs,
            };
            Some((n, ports.iter().position(|q| q.name == p)?))
        };
        let mut wires = Vec::new();
        let mut dangling = Vec::new();
        for c in &file.connections {
            match (
                find(&c.from, Causality::Output, &nodes),
                find(&c.to, Causality::Input, &nodes),
            ) {
                (Some(from), Some(to)) => wires.push(Wire {
                    from,
                    to,
                    from_name: c.from.clone(),
                    to_name: c.to.clone(),
                }),
                _ => dangling.push(format!("{} → {}", c.from, c.to)),
            }
        }
        layout(&mut nodes, &wires);
        Self {
            nodes,
            wires,
            dangling,
        }
    }

    /// The canvas area the nodes cover: `[min_x, min_y, max_x, max_y]`.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut it = self.nodes.iter();
        let first = it.next()?;
        let mut b = [
            first.at[0],
            first.at[1],
            first.at[0] + NODE_WIDTH,
            first.at[1] + first.height(),
        ];
        for n in it {
            b[0] = b[0].min(n.at[0]);
            b[1] = b[1].min(n.at[1]);
            b[2] = b[2].max(n.at[0] + NODE_WIDTH);
            b[3] = b[3].max(n.at[1] + n.height());
        }
        Some(b)
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
}

fn ports(spec: &simsandbox_core::block::BlockSpec, c: Causality) -> Vec<Port> {
    spec.ports
        .iter()
        .filter(|p| p.causality == c)
        .map(|p| Port {
            name: p.name.clone(),
            unit: p.unit.to_string(),
            shape: p.shape,
        })
        .collect()
}

fn node(b: &BlockEntry, registry: &Registry) -> Node {
    let config = BlockConfig {
        method: (&b.method).into(),
        definition: b.equations.clone(),
        update: b.update.clone(),
    };
    let (inputs, outputs, error) = match registry.build(&b.type_name, &b.name, &config) {
        Ok(block) => (
            ports(block.spec(), Causality::Input),
            ports(block.spec(), Causality::Output),
            None,
        ),
        Err(e) => (Vec::new(), Vec::new(), Some(e.to_string())),
    };
    Node {
        name: b.name.clone(),
        type_name: b.type_name.clone(),
        method: b.method.name.clone(),
        inputs,
        outputs,
        at: b.at.unwrap_or_default(),
        placed: b.at.is_some(),
        error,
    }
}

/// Place nodes without a saved position: in columns by the longest chain
/// of wires leading to them (wires that close a loop are ignored), each
/// column ordered by where its sources sit. If some nodes are placed,
/// the new ones go to the right of them.
fn layout(nodes: &mut [Node], wires: &[Wire]) {
    let n = nodes.len();
    if nodes.iter().all(|n| n.placed) {
        return;
    }
    // Edges between distinct nodes, without those closing a cycle (found
    // by depth-first search in file order).
    let mut succ: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for w in wires {
        if w.from.0 != w.to.0 {
            succ[w.from.0].insert(w.to.0);
        }
    }
    let mut state = vec![0u8; n]; // 0 new, 1 on the stack, 2 done
    let mut forward: Vec<Vec<usize>> = vec![Vec::new(); n];
    fn visit(v: usize, succ: &[BTreeSet<usize>], state: &mut [u8], forward: &mut [Vec<usize>]) {
        state[v] = 1;
        for &w in &succ[v] {
            match state[w] {
                0 => {
                    forward[v].push(w);
                    visit(w, succ, state, forward);
                }
                2 => forward[v].push(w),
                _ => {} // closes a loop
            }
        }
        state[v] = 2;
    }
    for v in 0..n {
        if state[v] == 0 {
            visit(v, &succ, &mut state, &mut forward);
        }
    }
    // Longest path layering over the acyclic edges.
    let mut column = vec![0usize; n];
    let mut changed = true;
    while changed {
        changed = false;
        for v in 0..n {
            for &w in &forward[v] {
                if column[w] < column[v] + 1 {
                    column[w] = column[v] + 1;
                    changed = true;
                }
            }
        }
    }
    let columns = column.iter().max().map_or(0, |c| c + 1);
    let mut rank = vec![0.0f64; n];
    let left = nodes
        .iter()
        .filter(|n| n.placed)
        .map(|n| n.at[0] + NODE_WIDTH + GAP_X)
        .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
        .unwrap_or(0.0);
    let top = nodes
        .iter()
        .filter(|n| n.placed)
        .map(|n| n.at[1])
        .fold(None, |m: Option<f64>, y| Some(m.map_or(y, |m| m.min(y))))
        .unwrap_or(0.0);
    for c in 0..columns {
        let mut members: Vec<usize> = (0..n).filter(|&v| column[v] == c).collect();
        // Order by the mean rank of sources in earlier columns, then file
        // order.
        let key = |v: usize| {
            let src: Vec<f64> = (0..n)
                .filter(|&u| forward[u].contains(&v))
                .map(|u| rank[u])
                .collect();
            if src.is_empty() {
                v as f64
            } else {
                src.iter().sum::<f64>() / src.len() as f64
            }
        };
        members.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
        let mut y = top;
        for (k, &v) in members.iter().enumerate() {
            rank[v] = k as f64;
            if !nodes[v].placed {
                nodes[v].at = [left + c as f64 * (NODE_WIDTH + GAP_X), y];
                y += nodes[v].height() + GAP_Y;
            }
        }
    }
}

/// A change the canvas makes to the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Only the layout: never restarts the run.
    Move {
        block: String,
        at: [f64; 2],
    },
    Add {
        type_name: String,
        at: [f64; 2],
    },
    Delete {
        block: String,
    },
    Rename {
        from: String,
        to: String,
    },
    /// `"block.port"` to `"block.port"`; replaces whatever fed the input.
    Connect {
        from: String,
        to: String,
    },
    /// Remove the wire into this input.
    Disconnect {
        to: String,
    },
}

impl Edit {
    /// Whether the edit changes the model (and so rebuilds and restarts).
    pub fn changes_model(&self) -> bool {
        !matches!(self, Edit::Move { .. })
    }
}

/// Apply an edit to a model file. Returns a note for the log. The file
/// may still fail to load (e.g. a wire between mismatched units); loading
/// reports that.
pub fn apply(file: &mut ModelFile, registry: &Registry, edit: &Edit) -> Result<String, String> {
    let has = |file: &ModelFile, name: &str| file.blocks.iter().any(|b| b.name == name);
    match edit {
        Edit::Move { block, at } => {
            let b = file
                .blocks
                .iter_mut()
                .find(|b| &b.name == block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            b.at = Some(at.map(tidy));
            Ok(String::new())
        }
        Edit::Add { type_name, at } => {
            let info = registry
                .info(type_name)
                .ok_or_else(|| format!("unknown block type '{type_name}'"))?;
            let base = type_name.rsplit('.').next().unwrap_or(type_name);
            let name = (1..)
                .map(|k| format!("{base}{k}"))
                .find(|n| !has(file, n))
                .expect("some name is free");
            let method = info
                .methods
                .first()
                .map(|(m, _)| m.clone())
                .ok_or_else(|| format!("'{type_name}' lists no method"))?;
            file.blocks.push(BlockEntry {
                name: name.clone(),
                type_name: type_name.clone(),
                method: simsandbox_file::MethodEntry {
                    name: method,
                    options: BTreeMap::new(),
                },
                update: None,
                equations: info.definition.clone().unwrap_or_default(),
                params: BTreeMap::new(),
                at: Some(at.map(tidy)),
                start: None,
            });
            Ok(format!("Added {name} ({type_name})"))
        }
        Edit::Delete { block } => {
            if !has(file, block) {
                return Err(format!("no block '{block}'"));
            }
            let mine = |key: &str| key.split_once('.').is_some_and(|(b, _)| b == block);
            file.blocks.retain(|b| &b.name != block);
            file.connections.retain(|c| !mine(&c.from) && !mine(&c.to));
            for g in &mut file.groups {
                g.blocks.retain(|b| b != block);
            }
            file.groups.retain(|g| g.blocks.len() > 1);
            file.controls.params.retain(|k, _| !mine(k));
            if let Some(s) = &mut file.sample {
                s.params.retain(|p| !mine(&p.param));
            }
            if file.sample.as_ref().is_some_and(|s| s.params.is_empty()) {
                file.sample = None;
            }
            // Views and filters that used it are dropped with a warning
            // when the model opens.
            Ok(format!("Deleted {block}"))
        }
        Edit::Rename { from, to } => {
            if !has(file, from) {
                return Err(format!("no block '{from}'"));
            }
            if to.is_empty()
                || to.contains('.')
                || !to.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "'{to}': names are letters, digits and '_' (no '.' or spaces)"
                ));
            }
            if has(file, to) {
                return Err(format!("there is already a block named '{to}'"));
            }
            rename(file, from, to)?;
            Ok(format!("Renamed {from} to {to}"))
        }
        Edit::Connect { from, to } => {
            for key in [from, to] {
                let (b, _) = key
                    .split_once('.')
                    .ok_or_else(|| format!("'{key}' is not 'block.port'"))?;
                if !has(file, b) {
                    return Err(format!("no block '{b}'"));
                }
            }
            file.connections.retain(|c| &c.to != to);
            file.connections.push(ConnectionEntry {
                from: from.clone(),
                to: to.clone(),
            });
            Ok(format!("Connected {from} → {to}"))
        }
        Edit::Disconnect { to } => {
            let before = file.connections.len();
            file.connections.retain(|c| &c.to != to);
            if file.connections.len() == before {
                return Err(format!("nothing is connected to '{to}'"));
            }
            Ok(format!("Disconnected {to}"))
        }
    }
}

/// Positions are saved to a tenth of a unit.
fn tidy(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Rename a block everywhere the file names it: connections, groups,
/// sweeps, controls, views and filters (`old.` becomes `new.` wherever
/// a reference starts). Equations name only their own quantities.
fn rename(file: &mut ModelFile, from: &str, to: &str) -> Result<(), String> {
    let mut value: Value = serde_json::from_str(&file.to_json()).map_err(|e| e.to_string())?;
    let obj = value.as_object_mut().ok_or("a model file is an object")?;
    for (key, v) in obj.iter_mut() {
        match key.as_str() {
            "blocks" => {
                for b in v.as_array_mut().into_iter().flatten() {
                    if b["name"] == from {
                        b["name"] = Value::String(to.into());
                    }
                }
            }
            "connections" | "groups" | "sample" | "controls" => {
                rename_in(v, from, to);
            }
            _ => {}
        }
    }
    let (renamed, _) = ModelFile::from_json(&value.to_string()).map_err(|e| e.to_string())?;
    *file = renamed;
    Ok(())
}

fn rename_in(v: &mut Value, from: &str, to: &str) {
    match v {
        Value::String(s) => *s = rename_refs(s, from, to),
        Value::Array(items) => items.iter_mut().for_each(|i| rename_in(i, from, to)),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (k, mut item) in entries {
                rename_in(&mut item, from, to);
                map.insert(rename_refs(&k, from, to), item);
            }
        }
        _ => {}
    }
}

/// Replace references `from.` by `to.` where `from` is a whole name: at
/// the start or after a character that cannot be part of a name.
pub fn rename_refs(s: &str, from: &str, to: &str) -> String {
    let pattern = format!("{from}.");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut prev: Option<char> = None;
    while let Some(i) = rest.find(&pattern) {
        let before = rest[..i].chars().last().or(prev);
        out.push_str(&rest[..i]);
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            out.push_str(&pattern);
        } else {
            out.push_str(to);
            out.push('.');
        }
        prev = Some('.');
        rest = &rest[i + pattern.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn file(part: &str) -> ModelFile {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        ModelFile::from_json(e.text).unwrap().0
    }

    #[test]
    fn unplaced_blocks_are_laid_out_left_to_right_by_signal_flow() {
        let r = registry();
        let d = Diagram::new(&file("Sheep"), &r);
        let grass = &d.nodes[d.node("grass").unwrap()];
        let sheep = &d.nodes[d.node("sheep").unwrap()];
        // A loop (grass ↔ sheep): one goes first, neither overlaps.
        assert_ne!(grass.at[0], sheep.at[0]);
        assert_eq!(d.wires.len(), 2);
        assert!(d.dangling.is_empty());
        // Ports and where wires attach.
        let w = &d.wires[0];
        let from = d.nodes[w.from.0].port_at(Causality::Output, w.from.1);
        assert_eq!(from[0], d.nodes[w.from.0].at[0] + NODE_WIDTH);

        // A chain a → b → c, plus a free block d: three columns.
        let mut f = ModelFile::new();
        for name in ["c", "a", "b", "d"] {
            apply(
                &mut f,
                &r,
                &Edit::Add {
                    type_name: "eq.equations".into(),
                    at: [0.0, 0.0],
                },
            )
            .unwrap();
            let last = f.blocks.last_mut().unwrap();
            last.name = name.into();
            last.at = None;
            last.equations = vec!["input u".into(), "dx/dt = u - x".into()];
        }
        for (from, to) in [("a.x", "b.u"), ("b.x", "c.u")] {
            apply(
                &mut f,
                &r,
                &Edit::Connect {
                    from: from.into(),
                    to: to.into(),
                },
            )
            .unwrap();
        }
        let d = Diagram::new(&f, &r);
        let x = |n: &str| d.nodes[d.node(n).unwrap()].at[0];
        assert!(x("a") < x("b") && x("b") < x("c"));
        assert_eq!(x("d"), x("a"));
        let y = |n: &str| d.nodes[d.node(n).unwrap()].at[1];
        assert_ne!(y("a"), y("d"));

        // Saved positions are kept; new blocks go to their right.
        f.blocks[0].at = Some([500.0, 100.0]);
        let d = Diagram::new(&f, &r);
        assert_eq!(d.nodes[0].at, [500.0, 100.0]);
        assert!(d.nodes[1..].iter().all(|n| n.at[0] > 500.0));
    }

    #[test]
    fn blocks_that_fail_to_build_still_show() {
        let r = registry();
        let mut f = file("Cruise");
        f.blocks[0].equations = vec!["dx/dt = ".into()];
        let d = Diagram::new(&f, &r);
        assert!(d.nodes[0].error.is_some());
        assert!(d.nodes[0].inputs.is_empty());
        // Its wires dangle.
        assert_eq!(d.wires.len(), 0);
        assert_eq!(d.dangling.len(), 2);
    }

    #[test]
    fn edits_change_the_file_and_the_result_loads() {
        let r = registry();
        let mut f = file("Cruise");
        let loads = |f: &ModelFile| crate::Session::load("t", &f.to_json(), &r).map(|_| ());

        apply(&mut f, &r, &Edit::Disconnect { to: "car.F".into() }).unwrap();
        assert!(
            loads(&f).unwrap_err().contains("car.F"),
            "an open input fails"
        );
        apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "cruise.F".into(),
                to: "car.F".into(),
            },
        )
        .unwrap();
        loads(&f).unwrap();
        // Connecting an input again replaces its source.
        apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "cruise.e".into(),
                to: "car.F".into(),
            },
        )
        .unwrap();
        assert_eq!(f.connections.iter().filter(|c| c.to == "car.F").count(), 1);

        apply(
            &mut f,
            &r,
            &Edit::Rename {
                from: "cruise".into(),
                to: "pid".into(),
            },
        )
        .unwrap();
        assert!(f.connections.iter().any(|c| c.from == "pid.e"));
        assert!(f.controls.params.contains_key("pid.kp"));
        let bound = serde_json::to_string(&f.controls.views).unwrap();
        assert!(
            bound.contains("pid.F") && !bound.contains("cruise."),
            "{bound}"
        );
        loads(&f).unwrap();
        assert!(apply(
            &mut f,
            &r,
            &Edit::Rename {
                from: "pid".into(),
                to: "car".into()
            }
        )
        .is_err());
        assert!(apply(
            &mut f,
            &r,
            &Edit::Rename {
                from: "pid".into(),
                to: "a b".into()
            }
        )
        .is_err());

        let note = apply(
            &mut f,
            &r,
            &Edit::Add {
                type_name: "eq.lorenz".into(),
                at: [12.34567, 0.0],
            },
        )
        .unwrap();
        assert!(note.contains("lorenz1"));
        assert_eq!(f.blocks.last().unwrap().at, Some([12.3, 0.0]));
        loads(&f).unwrap();

        apply(
            &mut f,
            &r,
            &Edit::Delete {
                block: "pid".into(),
            },
        )
        .unwrap();
        assert!(f.connections.iter().all(|c| !c.from.starts_with("pid.")));
        assert!(!f.controls.params.keys().any(|k| k.starts_with("pid.")));
        // car.F is now open, so the model reports it.
        assert!(loads(&f).is_err());
        assert!(apply(
            &mut f,
            &r,
            &Edit::Delete {
                block: "pid".into()
            }
        )
        .is_err());
        assert!(apply(
            &mut f,
            &r,
            &Edit::Add {
                type_name: "eq.nope".into(),
                at: [0.0, 0.0]
            }
        )
        .is_err());
    }

    #[test]
    fn references_are_renamed_only_where_whole_names_start() {
        assert_eq!(
            rename_refs("a.x + ba.x + a.y", "a", "b"),
            "b.x + ba.x + b.y"
        );
        assert_eq!(rename_refs("grid.a.x", "a", "b"), "grid.a.x");
        assert_eq!(rename_refs("2*a.x", "a", "q"), "2*q.x");
        assert_eq!(rename_refs("t", "a", "b"), "t");
    }

    #[test]
    fn every_example_has_a_diagram_without_dangling_wires() {
        let r = registry();
        for e in EXAMPLES {
            let (f, _) = ModelFile::from_json(e.text).unwrap();
            let d = Diagram::new(&f, &r);
            assert_eq!(d.nodes.len(), f.blocks.len(), "{}", e.name);
            assert_eq!(d.wires.len(), f.connections.len(), "{}", e.name);
            assert!(d.nodes.iter().all(|n| n.error.is_none()), "{}", e.name);
            // No two nodes overlap.
            for (i, a) in d.nodes.iter().enumerate() {
                for b in &d.nodes[i + 1..] {
                    let apart = a.at[0] + NODE_WIDTH <= b.at[0]
                        || b.at[0] + NODE_WIDTH <= a.at[0]
                        || a.at[1] + a.height() <= b.at[1]
                        || b.at[1] + b.height() <= a.at[1];
                    assert!(apart, "{}: {} and {} overlap", e.name, a.name, b.name);
                }
            }
        }
    }
}
