//! The model graph as the node canvas shows it (ADR 0042, ADR 0043): one
//! level at a time (the model, or a subsystem's inside), with its blocks,
//! subsystem instances and, inside a subsystem, nodes for its inputs and
//! outputs; wires; positions (saved as `at`, or laid out left to right by
//! signal flow); and the edits the canvas makes, as changes to the model
//! file. Shared by the app and headless rendering.

use serde_json::Value;
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::{BlockConfig, Registry};
use simsandbox_file::{
    BlockEntry, ConnectionEntry, InputPort, ModelFile, OutputPort, ParamDecl, ParamValue, Subsystem,
};
use std::collections::{BTreeMap, BTreeSet};

/// Node size in canvas units.
pub const NODE_WIDTH: f64 = 170.0;
pub const HEADER: f64 = 40.0;
pub const ROW: f64 = 20.0;
/// A parameter value's row on a node (ADR 0053).
pub const VALUE_ROW: f64 = 16.0;
/// Space between automatically placed nodes.
const GAP_X: f64 = 90.0;
const GAP_Y: f64 = 40.0;

/// Which level of the model a diagram shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Model,
    /// The inside of a subsystem, by name; editing it changes every use.
    Subsystem(String),
}

/// Inside a subsystem, its ports are nodes named `in:u` and `out:y`, each
/// with one port named like it (so wires read `in:u.u`, `out:y.y`).
pub fn input_node(port: &str) -> String {
    format!("in:{port}")
}

pub fn output_node(port: &str) -> String {
    format!("out:{port}")
}

/// One port on a node.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub unit: String,
    pub shape: Shape,
}

/// What a node stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Block,
    /// A use of a subsystem.
    Subsystem,
    /// A subsystem's input or output, seen from inside.
    Input,
    Output,
    /// A filter on the data layer (ADR 0050): reads recorded data.
    Filter,
    /// A view on the data layer: where data is shown.
    View,
}

/// One node.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    /// The block type, or the subsystem's name; `input` or `output`.
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
    /// On the data layer: what it reads that has no port on this level
    /// (`t`, parameters, signals inside subsystems), as `slot: signal`.
    pub unreached: Vec<String>,
    /// On the model level: its parameter values (a use's own parameters),
    /// other values first, starting values last; not those set by formulas
    /// (ADR 0053).
    pub values: Vec<NodeValue>,
    /// Its values are shown, below its ports (opened on the canvas).
    pub open: bool,
    /// The block's text settings, such as a conversion's units.
    pub settings: BTreeMap<String, String>,
}

/// A parameter value shown on a node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeValue {
    /// `"block.param"`, or a use's `"use.param"`, as parameter keys go.
    pub key: String,
    pub name: String,
    pub value: f64,
    /// The formula setting it, if one does: shown, not edited.
    pub formula: Option<String>,
    pub unit: String,
    /// A starting value (applies on Restart).
    pub initial: bool,
    /// Inside a subsystem, where the value is every use's default: the
    /// uses that set their own.
    pub overridden: Vec<String>,
}

impl Node {
    pub fn height(&self) -> f64 {
        let ports = HEADER + ROW * self.inputs.len().max(self.outputs.len()).max(1) as f64;
        match self.value_rows() {
            0 => ports,
            n => ports + 4.0 + VALUE_ROW * n as f64,
        }
    }

    /// Rows of values drawn: all of them while the node is open.
    pub fn value_rows(&self) -> usize {
        if self.open {
            self.values.len()
        } else {
            0
        }
    }

    /// Where value row `k` starts (its top), in canvas units.
    pub fn value_at(&self, k: usize) -> [f64; 2] {
        let ports = HEADER + ROW * self.inputs.len().max(self.outputs.len()).max(1) as f64;
        [self.at[0], self.at[1] + ports + 2.0 + VALUE_ROW * k as f64]
    }

    /// Where a port's wire attaches, in canvas units.
    pub fn port_at(&self, causality: Causality, index: usize) -> [f64; 2] {
        let y = self.at[1] + HEADER + ROW * (index as f64 + 0.5);
        match causality {
            Causality::Input => [self.at[0], y],
            Causality::Output => [self.at[0] + NODE_WIDTH, y],
        }
    }

    /// Whether it is one of the model's blocks or instances (not a port
    /// node), so it can be grouped.
    pub fn is_block(&self) -> bool {
        matches!(self.kind, NodeKind::Block | NodeKind::Subsystem)
    }

    /// Whether it is on the data layer (ADR 0050).
    pub fn is_observer(&self) -> bool {
        matches!(self.kind, NodeKind::Filter | NodeKind::View)
    }
}

/// A connection, by node and port index.
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub from: (usize, usize),
    pub to: (usize, usize),
    /// `"node.port"` as the canvas names it.
    pub from_name: String,
    pub to_name: String,
    /// On the data layer: the recorded signal it carries (empty for a
    /// model wire, which carries current values each step).
    pub signal: String,
}

impl Wire {
    /// Whether it reads recorded data (ADR 0050).
    pub fn is_data(&self) -> bool {
        !self.signal.is_empty()
    }

    /// How a selection names it: the input for a model wire (one per
    /// input), the input and signal for a data wire.
    pub fn key(&self) -> String {
        if self.is_data() {
            format!("{}<{}", self.to_name, self.signal)
        } else {
            self.to_name.clone()
        }
    }
}

/// One level of the graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagram {
    pub scope: Scope,
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    /// Connections that name a block or port that does not exist.
    pub dangling: Vec<String>,
    /// The file's subsystems, which can be added like block types.
    pub subsystems: Vec<String>,
    /// Inside a subsystem: the blocks using it (`"car1"`, nested
    /// `"farm/pasture"` as flat names).
    pub used_by: Vec<String>,
    /// View types that can be added on the data layer: name and label.
    pub view_types: Vec<(String, String)>,
    /// Data-layer slots that take several signals, as `view:K.slot`.
    pub many: BTreeSet<String>,
}

/// The blocks and wires of one level, and its ports inside a subsystem.
struct Level<'a> {
    blocks: &'a [BlockEntry],
    connections: &'a [ConnectionEntry],
    inputs: &'a [InputPort],
    outputs: &'a [OutputPort],
}

fn level<'a>(file: &'a ModelFile, scope: &Scope) -> Option<Level<'a>> {
    match scope {
        Scope::Model => Some(Level {
            blocks: &file.blocks,
            connections: &file.connections,
            inputs: &[],
            outputs: &[],
        }),
        Scope::Subsystem(name) => file.subsystems.get(name).map(|d| Level {
            blocks: &d.blocks,
            connections: &d.connections,
            inputs: &d.inputs,
            outputs: &d.outputs,
        }),
    }
}

impl Diagram {
    /// The model level of a file.
    pub fn new(file: &ModelFile, registry: &Registry) -> Self {
        Self::of(file, &Scope::Model, registry)
    }

    /// One level of a file, each block built alone so one that fails
    /// still shows (with its error and no ports).
    pub fn of(file: &ModelFile, scope: &Scope, registry: &Registry) -> Self {
        let Some(l) = level(file, scope) else {
            return Self {
                scope: scope.clone(),
                ..Self::default()
            };
        };
        let mut nodes: Vec<Node> = l
            .blocks
            .iter()
            .map(|b| match file.subsystems.get(&b.type_name) {
                Some(_) => instance_node(file, b, registry),
                None => node(b, registry),
            })
            .collect();
        for p in l.inputs {
            let port = interface_port(
                file,
                l.blocks,
                p.to.first().map(String::as_str),
                true,
                registry,
                0,
            );
            nodes.push(boundary(
                &input_node(&p.name),
                NodeKind::Input,
                &p.name,
                port,
                p.canvas,
            ));
        }
        for p in l.outputs {
            let port = interface_port(file, l.blocks, Some(&p.from), false, registry, 0);
            nodes.push(boundary(
                &output_node(&p.name),
                NodeKind::Output,
                &p.name,
                port,
                p.canvas,
            ));
        }
        let index: BTreeMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.name.clone(), i))
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
        let mut links: Vec<(String, String)> = l
            .connections
            .iter()
            .map(|c| (c.from.clone(), c.to.clone()))
            .collect();
        for p in l.inputs {
            for t in &p.to {
                links.push((format!("{}.{}", input_node(&p.name), p.name), t.clone()));
            }
        }
        for p in l.outputs {
            links.push((
                p.from.clone(),
                format!("{}.{}", output_node(&p.name), p.name),
            ));
        }
        let mut wires = Vec::new();
        let mut dangling = Vec::new();
        for (from, to) in links {
            match (
                find(&from, Causality::Output, &nodes),
                find(&to, Causality::Input, &nodes),
            ) {
                (Some(f), Some(t)) => wires.push(Wire {
                    from: f,
                    to: t,
                    from_name: from,
                    to_name: to,
                    signal: String::new(),
                }),
                _ => dangling.push(format!("{from} -> {to}")),
            }
        }
        if let Scope::Subsystem(name) = scope {
            // Inside a subsystem a value is the default of every use; say
            // which uses set their own (ADR 0055).
            let entries: Vec<&BlockEntry> = file
                .blocks
                .iter()
                .chain(file.subsystems.values().flat_map(|d| d.blocks.iter()))
                .filter(|b| b.type_name == *name)
                .collect();
            for n in &mut nodes {
                for v in &mut n.values {
                    v.overridden = entries
                        .iter()
                        .filter(|u| u.params.contains_key(&v.key))
                        .map(|u| u.name.clone())
                        .collect();
                }
            }
        }
        layout(&mut nodes, &wires);
        let used_by = match scope {
            Scope::Model => Vec::new(),
            Scope::Subsystem(name) => uses(file, name),
        };
        Self {
            scope: scope.clone(),
            nodes,
            wires,
            dangling,
            subsystems: file.subsystems.keys().cloned().collect(),
            used_by,
            ..Self::default()
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

/// Flat names of every use of a subsystem (`"car1"`, `"farm/pasture"`).
pub fn uses(file: &ModelFile, subsystem: &str) -> Vec<String> {
    fn walk(
        file: &ModelFile,
        blocks: &[BlockEntry],
        prefix: &str,
        target: &str,
        out: &mut Vec<String>,
        depth: usize,
    ) {
        if depth > 16 {
            return;
        }
        for b in blocks {
            if let Some(def) = file.subsystems.get(&b.type_name) {
                let flat = format!("{prefix}{}", b.name);
                if b.type_name == target {
                    out.push(flat.clone());
                }
                walk(
                    file,
                    &def.blocks,
                    &format!("{flat}/"),
                    target,
                    out,
                    depth + 1,
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(file, &file.blocks, "", subsystem, &mut out, 0);
    out
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

fn build(
    b: &BlockEntry,
    registry: &Registry,
) -> Result<Box<dyn simsandbox_core::block::Block>, String> {
    let config = BlockConfig {
        method: (&b.method).into(),
        definition: b.equations.clone(),
        update: b.update.clone(),
        settings: b.settings.clone(),
    };
    registry
        .build(&b.type_name, &b.name, &config)
        .map_err(|e| e.to_string())
}

fn node(b: &BlockEntry, registry: &Registry) -> Node {
    let mut method = b.method.name.clone();
    let mut values = Vec::new();
    let (inputs, outputs, error) = match build(b, registry) {
        Ok(block) => {
            if method.is_empty() {
                method = block.method().name.clone();
            }
            // Its parameters as the file gives them, else their defaults.
            for p in &block.spec().params {
                let (value, formula) = match b.params.get(&p.name) {
                    Some(ParamValue::Number(v)) => (*v, None),
                    // Set by a formula: not listed.
                    Some(ParamValue::Formula(_)) => continue,
                    None => (p.default, None),
                };
                values.push(NodeValue {
                    key: format!("{}.{}", b.name, p.name),
                    name: p.name.clone(),
                    value,
                    formula,
                    unit: unit_text(&p.unit),
                    initial: p.initial_condition,
                    overridden: Vec::new(),
                });
            }
            values.sort_by_key(|v| v.initial);
            (
                ports(block.spec(), Causality::Input),
                ports(block.spec(), Causality::Output),
                None,
            )
        }
        Err(e) => (Vec::new(), Vec::new(), Some(e)),
    };
    // A conversion says what it converts (ADR 0045).
    if let (Some(from), Some(to)) = (b.settings.get("from"), b.settings.get("to")) {
        method = format!("{from} to {to}");
    }
    Node {
        name: b.name.clone(),
        kind: NodeKind::Block,
        type_name: b.type_name.clone(),
        method,
        inputs,
        outputs,
        at: b.canvas.unwrap_or_default(),
        placed: b.canvas.is_some(),
        error,
        unreached: Vec::new(),
        values,
        open: false,
        settings: b.settings.clone(),
    }
}

/// A unit as nodes show it: none for dimensionless.
fn unit_text(u: &simsandbox_core::units::Unit) -> String {
    if *u == simsandbox_core::units::Unit::DIMENSIONLESS {
        String::new()
    } else {
        u.to_string()
    }
}

impl Diagram {
    /// Show these current values (as the inspector lists them, ADR 0053)
    /// on the model level's nodes in place of the file's.
    pub fn live_values(&mut self, rows: &[crate::session::ParamRow]) {
        if self.scope != Scope::Model {
            return;
        }
        for n in self.nodes.iter_mut().filter(|n| n.is_block()) {
            // Values formulas set are not listed.
            n.values.retain(|v| {
                rows.iter()
                    .find(|r| r.key == v.key)
                    .is_none_or(|r| r.formula.is_none())
            });
            for v in &mut n.values {
                if let Some(r) = rows.iter().find(|r| r.key == v.key) {
                    v.value = r.value;
                }
            }
        }
    }
}

/// A port of a subsystem, typed by the inner port it leads to or from.
fn interface_port(
    file: &ModelFile,
    blocks: &[BlockEntry],
    inner: Option<&str>,
    input: bool,
    registry: &Registry,
    depth: usize,
) -> Option<Port> {
    let (b, p) = inner?.split_once('.')?;
    let entry = blocks.iter().find(|e| e.name == b)?;
    if depth > 16 {
        return None;
    }
    match file.subsystems.get(&entry.type_name) {
        Some(def) => {
            if input {
                let port = def.inputs.iter().find(|q| q.name == p)?;
                interface_port(
                    file,
                    &def.blocks,
                    port.to.first().map(String::as_str),
                    true,
                    registry,
                    depth + 1,
                )
            } else {
                let port = def.outputs.iter().find(|q| q.name == p)?;
                interface_port(
                    file,
                    &def.blocks,
                    Some(&port.from),
                    false,
                    registry,
                    depth + 1,
                )
            }
        }
        None => {
            let block = build(entry, registry).ok()?;
            let want = if input {
                Causality::Input
            } else {
                Causality::Output
            };
            ports(block.spec(), want).into_iter().find(|q| q.name == p)
        }
    }
}

fn instance_node(file: &ModelFile, b: &BlockEntry, registry: &Registry) -> Node {
    let def = &file.subsystems[&b.type_name];
    let unknown = |name: &str| Port {
        name: name.into(),
        unit: String::new(),
        shape: Shape::Scalar,
    };
    let inputs = def
        .inputs
        .iter()
        .map(|p| {
            let typed = interface_port(
                file,
                &def.blocks,
                p.to.first().map(String::as_str),
                true,
                registry,
                0,
            );
            Port {
                name: p.name.clone(),
                ..typed.unwrap_or_else(|| unknown(&p.name))
            }
        })
        .collect();
    let outputs = def
        .outputs
        .iter()
        .map(|p| {
            let typed = interface_port(file, &def.blocks, Some(&p.from), false, registry, 0);
            Port {
                name: p.name.clone(),
                ..typed.unwrap_or_else(|| unknown(&p.name))
            }
        })
        .collect();
    // The use's own parameters: given in its entry, else the subsystem's
    // defaults.
    let values = def
        .params
        .iter()
        .filter_map(|(name, d)| {
            let (value, formula) = match b.params.get(name) {
                Some(ParamValue::Number(v)) => (*v, None),
                // Set by a formula: not listed.
                Some(ParamValue::Formula(_)) => return None,
                None => (d.value, None),
            };
            Some(NodeValue {
                key: format!("{}.{name}", b.name),
                name: name.clone(),
                value,
                formula,
                unit: d.unit.clone(),
                initial: false,
                overridden: Vec::new(),
            })
        })
        .collect();
    Node {
        name: b.name.clone(),
        kind: NodeKind::Subsystem,
        type_name: b.type_name.clone(),
        method: "subsystem".into(),
        inputs,
        outputs,
        at: b.canvas.unwrap_or_default(),
        placed: b.canvas.is_some(),
        error: None,
        unreached: Vec::new(),
        values,
        open: false,
        settings: BTreeMap::new(),
    }
}

fn boundary(
    name: &str,
    kind: NodeKind,
    port: &str,
    typed: Option<Port>,
    at: Option<[f64; 2]>,
) -> Node {
    let p = Port {
        name: port.into(),
        ..typed.unwrap_or(Port {
            name: String::new(),
            unit: String::new(),
            shape: Shape::Scalar,
        })
    };
    let (inputs, outputs, type_name) = match kind {
        NodeKind::Input => (Vec::new(), vec![p], "input"),
        _ => (vec![p], Vec::new(), "output"),
    };
    Node {
        name: name.into(),
        kind,
        type_name: type_name.into(),
        method: String::new(),
        inputs,
        outputs,
        at: at.unwrap_or_default(),
        placed: at.is_some(),
        error: None,
        unreached: Vec::new(),
        values: Vec::new(),
        open: false,
        settings: BTreeMap::new(),
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
    // A subsystem's outputs go in the last column.
    let last = column.iter().max().copied().unwrap_or(0);
    for (v, node) in nodes.iter().enumerate() {
        if node.kind == NodeKind::Output {
            column[v] = last.max(1);
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

/// A change the canvas makes to the model, at one level.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Only the layout: never restarts the run. Also moves port nodes.
    Move { block: String, at: [f64; 2] },
    /// A registered type or one of the file's subsystems.
    Add { type_name: String, at: [f64; 2] },
    /// A block, or a port node (removing the port).
    Delete { block: String },
    /// A block, or a port node (renaming the port).
    Rename { from: String, to: String },
    /// `"node.port"` to `"node.port"`; replaces whatever fed the input.
    Connect { from: String, to: String },
    /// Remove the wire into this input.
    Disconnect { to: String },
    /// Inside a subsystem: a new input or output port.
    AddPort { input: bool, at: [f64; 2] },
    /// Move these blocks into a new subsystem and use it in their place.
    Group { blocks: Vec<String> },
    /// Rename a subsystem (its uses follow).
    RenameSubsystem { from: String, to: String },
    /// Data layer (ADR 0050): show the signal an output carries in a
    /// view's slot (`to` is `view:K.slot`), added to the slot's signals if
    /// it takes several (`many`), else in place of its signal.
    Bind {
        from: String,
        to: String,
        many: bool,
    },
    /// Data layer: stop a filter or view input reading a signal.
    Unbind { to: String, signal: String },
    /// Data layer: a new expression filter.
    AddFilter { at: [f64; 2] },
    /// Data layer: a new view of a type (bound from its first preset when
    /// the app shows it).
    AddView { type_name: String, at: [f64; 2] },
    /// Turn an expression filter into a formula block, so it can feed the
    /// model (ADR 0032).
    Promote { filter: String },
    /// Promote a filter and connect its block to a model input.
    PromoteConnect { filter: String, to: String },
    /// A block's parameter value, written in the file (for a block not yet
    /// in the running model; running values change without an edit).
    SetParam {
        block: String,
        param: String,
        value: f64,
    },
    /// A block's parameter set by a formula of the declared parameters in
    /// scope (ADR 0043, ADR 0056).
    SetFormula {
        block: String,
        param: String,
        formula: String,
    },
    /// Declare a parameter of the model itself, for formulas to read
    /// (ADR 0056).
    DeclareParam {
        name: String,
        value: f64,
        unit: String,
    },
    /// Remove a declared parameter of the model.
    RemoveParam { name: String },
    /// A block's text setting, such as a conversion's `from` or `to` unit
    /// (ADR 0045).
    SetSetting {
        block: String,
        key: String,
        text: String,
    },
}

impl Edit {
    /// Whether the edit changes the model (and so rebuilds and restarts).
    pub fn changes_model(&self) -> bool {
        !matches!(self, Edit::Move { .. }) && !self.data_only()
    }

    /// Whether the edit changes only filters and views, which apply to the
    /// running model without a restart (ADR 0050).
    pub fn data_only(&self) -> bool {
        let observer = crate::observers::is_observer;
        let node = |key: &str| key.split_once('.').map_or(key, |(n, _)| n).to_string();
        match self {
            Edit::Bind { .. }
            | Edit::Unbind { .. }
            | Edit::AddFilter { .. }
            | Edit::AddView { .. } => true,
            Edit::Connect { to, .. } | Edit::Disconnect { to } => observer(&node(to)),
            Edit::Delete { block } => observer(block),
            Edit::Rename { from, .. } => observer(from),
            _ => false,
        }
    }
}

/// Apply an edit at the model level; see [`apply_in`].
pub fn apply(file: &mut ModelFile, registry: &Registry, edit: &Edit) -> Result<String, String> {
    apply_in(file, &Scope::Model, registry, edit)
}

/// Apply an edit at one level of a model file. Returns a note for the
/// log. The file may still fail to load (e.g. a wire between mismatched
/// units); loading reports that.
pub fn apply_in(
    file: &mut ModelFile,
    scope: &Scope,
    registry: &Registry,
    edit: &Edit,
) -> Result<String, String> {
    if let Some(result) = crate::observers::apply(file, scope, registry, edit) {
        return result;
    }
    if let Edit::RenameSubsystem { from, to } = edit {
        let note = rename_subsystem(file, from, to)?;
        let (old, new) = (format!("{from}::"), format!("{to}::"));
        for k in &mut file.controls.open {
            if let Some(rest) = k.strip_prefix(&old) {
                *k = format!("{new}{rest}");
            }
        }
        return Ok(note);
    }
    if let Edit::Group { blocks } = edit {
        return group(file, scope, blocks);
    }
    match edit {
        Edit::DeclareParam { name, value, unit } => {
            let name = name.trim();
            if !simsandbox_file::subsystems::valid_name(name) || simsandbox_expr::is_reserved(name)
            {
                return Err(format!(
                    "'{name}': a parameter's name is letters, digits and '_', not a function \
                     or 'pi'"
                ));
            }
            if file.params.contains_key(name) {
                return Err(format!("the model already has a parameter '{name}'"));
            }
            if !value.is_finite() {
                return Err(format!("{name}: the value must be a finite number"));
            }
            file.params.insert(
                name.to_string(),
                ParamDecl {
                    value: *value,
                    unit: unit.trim().to_string(),
                    description: String::new(),
                    range: None,
                },
            );
            return Ok(format!("Declared the parameter {name}"));
        }
        Edit::RemoveParam { name } => {
            file.params
                .remove(name)
                .ok_or_else(|| format!("the model has no parameter '{name}'"))?;
            file.controls.params.remove(name);
            return Ok(format!("Removed the parameter {name}"));
        }
        _ => {}
    }
    // Which nodes are open follows renames and deletions (ADR 0055).
    let open = |file: &mut ModelFile| match edit {
        Edit::Delete { block } => {
            let key = crate::canvas::open_key(scope, block);
            file.controls.open.retain(|k| *k != key);
        }
        Edit::Rename { from, to } => {
            let key = crate::canvas::open_key(scope, from);
            for k in &mut file.controls.open {
                if *k == key {
                    *k = crate::canvas::open_key(scope, to);
                }
            }
        }
        _ => {}
    };
    let result = apply_level(file, scope, registry, edit);
    if result.is_ok() {
        open(file);
    }
    result
}

/// [`apply_in`] for edits within one level.
fn apply_level(
    file: &mut ModelFile,
    scope: &Scope,
    registry: &Registry,
    edit: &Edit,
) -> Result<String, String> {
    match scope {
        Scope::Model => {
            let mut def = Subsystem {
                blocks: std::mem::take(&mut file.blocks),
                connections: std::mem::take(&mut file.connections),
                ..Subsystem::default()
            };
            let result = edit_level(&mut def, file, registry, edit, false);
            file.blocks = def.blocks;
            file.connections = def.connections;
            let note = result?;
            // References outside the level: sweeps, controls, groups.
            match edit {
                Edit::Delete { block } => forget_block(file, block),
                Edit::Rename { from, to } => rename_refs_in_file(file, from, to)?,
                _ => {}
            }
            Ok(note)
        }
        Scope::Subsystem(name) => {
            let mut def = file
                .subsystems
                .remove(name)
                .ok_or_else(|| format!("no subsystem '{name}'"))?;
            let result = edit_level(&mut def, file, registry, edit, true);
            file.subsystems.insert(name.clone(), def);
            let note = result?;
            match edit {
                // Wires to a removed or renamed port, wherever it is used.
                Edit::Delete { block } if block.starts_with("in:") || block.starts_with("out:") => {
                    let port = &block[block.find(':').unwrap() + 1..];
                    for_each_level(file, |blocks, conns| {
                        let users: Vec<String> = blocks
                            .iter()
                            .filter(|b| &b.type_name == name)
                            .map(|b| format!("{}.{port}", b.name))
                            .collect();
                        conns.retain(|c| !users.contains(&c.from) && !users.contains(&c.to));
                    });
                }
                Edit::Rename { from, to }
                    if from.starts_with("in:") || from.starts_with("out:") =>
                {
                    let old = &from[from.find(':').unwrap() + 1..];
                    for_each_level(file, |blocks, conns| {
                        for b in blocks.iter().filter(|b| &b.type_name == name) {
                            let (o, n) = (format!("{}.{old}", b.name), format!("{}.{to}", b.name));
                            for c in conns.iter_mut() {
                                if c.from == o {
                                    c.from = n.clone();
                                }
                                if c.to == o {
                                    c.to = n.clone();
                                }
                            }
                        }
                    });
                }
                Edit::Rename { from, to } => {
                    // Flat names of its blocks in controls: `inst/old.` →
                    // `inst/new.` (any block of that name inside an
                    // instance).
                    let mut c = serde_json::to_value(&file.controls).map_err(|e| e.to_string())?;
                    replace_in(&mut c, &format!("/{from}."), &format!("/{to}."));
                    file.controls = serde_json::from_value(c).map_err(|e| e.to_string())?;
                }
                _ => {}
            }
            Ok(note)
        }
    }
}

/// Run `f` on the blocks and wires of every level: the model and each
/// subsystem.
fn for_each_level(
    file: &mut ModelFile,
    mut f: impl FnMut(&[BlockEntry], &mut Vec<ConnectionEntry>),
) {
    f(&file.blocks, &mut file.connections);
    for def in file.subsystems.values_mut() {
        f(&def.blocks, &mut def.connections);
    }
}

fn replace_in(v: &mut Value, from: &str, to: &str) {
    match v {
        Value::String(s) => *s = s.replace(from, to),
        Value::Array(items) => items.iter_mut().for_each(|i| replace_in(i, from, to)),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (k, mut item) in entries {
                replace_in(&mut item, from, to);
                map.insert(k.replace(from, to), item);
            }
        }
        _ => {}
    }
}

/// Edit one level's blocks, wires and (inside a subsystem) ports.
fn edit_level(
    l: &mut Subsystem,
    file: &ModelFile,
    registry: &Registry,
    edit: &Edit,
    inside: bool,
) -> Result<String, String> {
    let has = |l: &Subsystem, name: &str| l.blocks.iter().any(|b| b.name == name);
    let port_of = |node: &str| -> Option<(bool, String)> {
        let (kind, rest) = node.split_once(':')?;
        Some((kind == "in", rest.to_string()))
    };
    match edit {
        Edit::SetParam {
            block,
            param,
            value,
        } => {
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            b.params.insert(param.clone(), ParamValue::Number(*value));
            Ok(format!(
                "{block}.{param} is now {}",
                crate::canvas::format_number(*value)
            ))
        }
        Edit::SetFormula {
            block,
            param,
            formula,
        } => {
            let formula = formula.trim().trim_start_matches('=').trim();
            if formula.is_empty() {
                return Err("an empty formula: type a number or '= expression'".into());
            }
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            b.params
                .insert(param.clone(), ParamValue::Formula(formula.to_string()));
            Ok(format!("{block}.{param} = {formula}"))
        }
        Edit::SetSetting { block, key, text } => {
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            b.settings.insert(key.clone(), text.trim().to_string());
            Ok(format!("{block}: {key} is now {}", text.trim()))
        }
        Edit::Move { block, at } => {
            let at = Some(at.map(tidy));
            match port_of(block) {
                Some((true, p)) => {
                    let q = l
                        .inputs
                        .iter_mut()
                        .find(|q| q.name == p)
                        .ok_or("no such input")?;
                    q.canvas = at;
                }
                Some((false, p)) => {
                    let q = l
                        .outputs
                        .iter_mut()
                        .find(|q| q.name == p)
                        .ok_or("no such output")?;
                    q.canvas = at;
                }
                None => {
                    let b = l
                        .blocks
                        .iter_mut()
                        .find(|b| &b.name == block)
                        .ok_or_else(|| format!("no block '{block}'"))?;
                    b.canvas = at;
                }
            }
            Ok(String::new())
        }
        Edit::Add { type_name, at } => {
            let entry = if file.subsystems.contains_key(type_name) {
                let base = type_name.to_lowercase();
                let name = free_name(&base, |n| has(l, n));
                BlockEntry {
                    canvas: Some(at.map(tidy)),
                    ..BlockEntry::instance(&name, type_name)
                }
            } else {
                let info = registry
                    .info(type_name)
                    .ok_or_else(|| format!("unknown block type '{type_name}'"))?;
                let base = type_name.rsplit('.').next().unwrap_or(type_name);
                let name = free_name(base, |n| has(l, n));
                let method = info
                    .methods
                    .first()
                    .map(|(m, _)| m.clone())
                    .ok_or_else(|| format!("'{type_name}' lists no method"))?;
                BlockEntry {
                    name,
                    type_name: type_name.clone(),
                    method: simsandbox_file::MethodEntry {
                        name: method,
                        options: BTreeMap::new(),
                    },
                    equations: info.definition.clone().unwrap_or_default(),
                    canvas: Some(at.map(tidy)),
                    ..Default::default()
                }
            };
            let note = format!("Added {} ({type_name})", entry.name);
            l.blocks.push(entry);
            Ok(note)
        }
        Edit::Delete { block } => {
            if let Some((input, p)) = port_of(block) {
                if input {
                    l.inputs.retain(|q| q.name != p);
                } else {
                    l.outputs.retain(|q| q.name != p);
                }
                return Ok(format!(
                    "Removed the {} {p}",
                    if input { "input" } else { "output" }
                ));
            }
            if !has(l, block) {
                return Err(format!("no block '{block}'"));
            }
            let mine = |key: &str| key.split_once('.').is_some_and(|(b, _)| b == block);
            l.blocks.retain(|b| &b.name != block);
            l.connections.retain(|c| !mine(&c.from) && !mine(&c.to));
            for q in &mut l.inputs {
                q.to.retain(|t| !mine(t));
            }
            // An output whose source is gone waits for a new one.
            for q in &mut l.outputs {
                if mine(&q.from) {
                    q.from.clear();
                }
            }
            Ok(format!("Deleted {block}"))
        }
        Edit::Rename { from, to } => {
            if !simsandbox_file::subsystems::valid_name(to) {
                return Err(format!(
                    "'{to}': names are letters, digits and '_' (no '.', '/' or spaces)"
                ));
            }
            if let Some((input, p)) = port_of(from) {
                let taken = if input {
                    l.inputs.iter().any(|q| &q.name == to)
                } else {
                    l.outputs.iter().any(|q| &q.name == to)
                };
                if taken {
                    return Err(format!("there is already a port named '{to}'"));
                }
                if input {
                    if let Some(q) = l.inputs.iter_mut().find(|q| q.name == p) {
                        q.name = to.clone();
                    }
                } else if let Some(q) = l.outputs.iter_mut().find(|q| q.name == p) {
                    q.name = to.clone();
                }
                return Ok(format!("Renamed the port {p} to {to}"));
            }
            if !has(l, from) {
                return Err(format!("no block '{from}'"));
            }
            if has(l, to) {
                return Err(format!("there is already a block named '{to}'"));
            }
            for b in &mut l.blocks {
                if &b.name == from {
                    b.name = to.clone();
                }
            }
            for c in &mut l.connections {
                c.from = rename_refs(&c.from, from, to);
                c.to = rename_refs(&c.to, from, to);
            }
            for q in &mut l.inputs {
                for t in &mut q.to {
                    *t = rename_refs(t, from, to);
                }
            }
            for q in &mut l.outputs {
                q.from = rename_refs(&q.from, from, to);
            }
            Ok(format!("Renamed {from} to {to}"))
        }
        Edit::Connect { from, to } => {
            let src = port_of(from.split_once('.').map_or(from.as_str(), |(n, _)| n));
            let dst = port_of(to.split_once('.').map_or(to.as_str(), |(n, _)| n));
            for (key, port) in [(from, &src), (to, &dst)] {
                if port.is_none() {
                    let (b, _) = key
                        .split_once('.')
                        .ok_or_else(|| format!("'{key}' is not 'block.port'"))?;
                    if !has(l, b) {
                        return Err(format!("no block '{b}'"));
                    }
                }
            }
            match (src, dst) {
                (Some(_), Some(_)) => {
                    Err("a subsystem's input cannot feed its output directly".into())
                }
                (Some((true, p)), None) => {
                    unfeed(l, to);
                    let q = l
                        .inputs
                        .iter_mut()
                        .find(|q| q.name == p)
                        .ok_or("no such input")?;
                    q.to.push(to.clone());
                    Ok(format!("Connected input {p} -> {to}"))
                }
                (None, Some((false, p))) => {
                    let q = l
                        .outputs
                        .iter_mut()
                        .find(|q| q.name == p)
                        .ok_or("no such output")?;
                    q.from = from.clone();
                    Ok(format!("Connected {from} -> output {p}"))
                }
                (None, None) => {
                    unfeed(l, to);
                    l.connections.push(ConnectionEntry {
                        from: from.clone(),
                        to: to.clone(),
                    });
                    Ok(format!("Connected {from} -> {to}"))
                }
                _ => Err("wires run from an output to an input".into()),
            }
        }
        Edit::Disconnect { to } => {
            if let Some((false, p)) = port_of(to.split_once('.').map_or(to.as_str(), |(n, _)| n)) {
                let q = l
                    .outputs
                    .iter_mut()
                    .find(|q| q.name == p)
                    .ok_or("no such output")?;
                q.from.clear();
                return Ok(format!("Disconnected output {p}"));
            }
            if !unfeed(l, to) {
                return Err(format!("nothing is connected to '{to}'"));
            }
            Ok(format!("Disconnected {to}"))
        }
        Edit::AddPort { input, at } => {
            if !inside {
                return Err("only a subsystem has inputs and outputs".into());
            }
            let base = if *input { "in" } else { "out" };
            let name = if *input {
                free_name(base, |n| l.inputs.iter().any(|q| q.name == n))
            } else {
                free_name(base, |n| l.outputs.iter().any(|q| q.name == n))
            };
            let at = Some(at.map(tidy));
            if *input {
                l.inputs.push(InputPort {
                    name: name.clone(),
                    to: Vec::new(),
                    canvas: at,
                });
            } else {
                l.outputs.push(OutputPort {
                    name: name.clone(),
                    from: String::new(),
                    canvas: at,
                });
            }
            Ok(format!(
                "Added the {} {name}",
                if *input { "input" } else { "output" }
            ))
        }
        Edit::Group { .. }
        | Edit::RenameSubsystem { .. }
        | Edit::DeclareParam { .. }
        | Edit::RemoveParam { .. } => unreachable!("handled before"),
        // Data-layer edits are handled by `observers::apply` first.
        Edit::Bind { .. }
        | Edit::Unbind { .. }
        | Edit::AddFilter { .. }
        | Edit::AddView { .. }
        | Edit::Promote { .. }
        | Edit::PromoteConnect { .. } => unreachable!("handled before"),
    }
}

/// Remove whatever feeds an input at this level. Returns whether anything
/// did.
fn unfeed(l: &mut Subsystem, to: &str) -> bool {
    let before = l.connections.len() + l.inputs.iter().map(|q| q.to.len()).sum::<usize>();
    l.connections.retain(|c| c.to != to);
    for q in &mut l.inputs {
        q.to.retain(|t| t != to);
    }
    let after = l.connections.len() + l.inputs.iter().map(|q| q.to.len()).sum::<usize>();
    after < before
}

pub(crate) fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|k| format!("{base}{k}"))
        .find(|n| !taken(n))
        .expect("some name is free")
}

/// Drop a deleted model-level block from sweeps, controls and groups.
fn forget_block(file: &mut ModelFile, block: &str) {
    let mine = |key: &str| {
        key.split_once('.').is_some_and(|(b, _)| b == block)
            || key.split_once('/').is_some_and(|(b, _)| b == block)
    };
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
    // Views and filters that used it are dropped with a warning when the
    // model opens.
}

/// Positions are saved to a tenth of a unit.
pub(crate) fn tidy(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Rename a model-level block in sweeps, controls (sliders, views,
/// filters) and groups: `old.` and `old/` become `new.` and `new/`
/// wherever a reference starts.
fn rename_refs_in_file(file: &mut ModelFile, from: &str, to: &str) -> Result<(), String> {
    let mut value: Value = serde_json::from_str(&file.to_json()).map_err(|e| e.to_string())?;
    let obj = value.as_object_mut().ok_or("a model file is an object")?;
    for (key, v) in obj.iter_mut() {
        if matches!(key.as_str(), "groups" | "sample" | "controls") {
            rename_in(v, from, to);
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

/// Replace references `from.` (and `from/`, a block inside it) by `to.`
/// (`to/`) where `from` is a whole name: at the start or after a
/// character that cannot be part of a name.
pub fn rename_refs(s: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut prev: Option<char> = None;
    loop {
        let next = [".", "/"]
            .iter()
            .filter_map(|sep| rest.find(&format!("{from}{sep}")).map(|i| (i, *sep)))
            .min();
        let Some((i, sep)) = next else {
            break;
        };
        let before = rest[..i].chars().last().or(prev);
        out.push_str(&rest[..i]);
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '/') {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        out.push_str(sep);
        prev = Some(sep.chars().next().unwrap());
        rest = &rest[i + from.len() + 1..];
    }
    out.push_str(rest);
    out
}

/// Rename a subsystem, and every use of it.
fn rename_subsystem(file: &mut ModelFile, from: &str, to: &str) -> Result<String, String> {
    if !simsandbox_file::subsystems::valid_name(to) {
        return Err(format!("'{to}': names are letters, digits and '_'"));
    }
    if file.subsystems.contains_key(to) {
        return Err(format!("there is already a subsystem named '{to}'"));
    }
    let def = file
        .subsystems
        .remove(from)
        .ok_or_else(|| format!("no subsystem '{from}'"))?;
    file.subsystems.insert(to.into(), def);
    let fix = |blocks: &mut Vec<BlockEntry>| {
        for b in blocks.iter_mut().filter(|b| b.type_name == from) {
            b.type_name = to.into();
        }
    };
    fix(&mut file.blocks);
    for d in file.subsystems.values_mut() {
        fix(&mut d.blocks);
    }
    Ok(format!("Renamed the subsystem {from} to {to}"))
}

/// Move `names` (blocks of one level) into a new subsystem and use it in
/// their place. Wires crossing the boundary become its ports; declared
/// parameters the moved formulas use are passed in as its parameters.
fn group(file: &mut ModelFile, scope: &Scope, names: &[String]) -> Result<String, String> {
    if names.is_empty() {
        return Err("select the blocks to group".into());
    }
    let def_name = free_name("group", |n| file.subsystems.contains_key(n));
    // The level's blocks and wires, and the parameters in scope there.
    let (mut l, scope_params): (Subsystem, BTreeMap<String, ParamDecl>) = match scope {
        Scope::Model => (
            Subsystem {
                blocks: std::mem::take(&mut file.blocks),
                connections: std::mem::take(&mut file.connections),
                ..Subsystem::default()
            },
            file.params.clone(),
        ),
        Scope::Subsystem(s) => {
            let d = file
                .subsystems
                .remove(s)
                .ok_or_else(|| format!("no subsystem '{s}'"))?;
            let p = d.params.clone();
            (d, p)
        }
    };
    let result = (|| {
        for n in names {
            if !l.blocks.iter().any(|b| &b.name == n) {
                return Err(format!("no block '{n}' here"));
            }
        }
        let inside = |key: &str| {
            key.split_once('.')
                .is_some_and(|(b, _)| names.iter().any(|n| n == b))
        };
        let instance = if l.blocks.iter().any(|b| b.name == def_name) {
            free_name(&def_name, |n| l.blocks.iter().any(|b| b.name == n))
        } else {
            def_name.clone()
        };
        let moved: Vec<BlockEntry> = l
            .blocks
            .iter()
            .filter(|b| names.contains(&b.name))
            .cloned()
            .collect();
        // Where the instance goes: the moved blocks' top-left.
        let at = moved
            .iter()
            .filter_map(|b| b.canvas)
            .fold(None, |m: Option<[f64; 2]>, p| {
                Some(m.map_or(p, |m| [m[0].min(p[0]), m[1].min(p[1])]))
            });
        let mut def = Subsystem {
            description: None,
            blocks: moved,
            ..Subsystem::default()
        };
        let mut outer = Vec::new();
        let unique = |taken: &dyn Fn(&str) -> bool, base: &str| {
            if !taken(base) {
                base.to_string()
            } else {
                free_name(base, taken)
            }
        };
        for c in std::mem::take(&mut l.connections) {
            match (inside(&c.from), inside(&c.to)) {
                (true, true) => def.connections.push(c),
                (false, false) => outer.push(c),
                (false, true) => {
                    // One input per outside source.
                    let existing = def.inputs.iter_mut().find(|q| {
                        outer.iter().any(|o: &ConnectionEntry| {
                            o.from == c.from && o.to == format!("{instance}.{}", q.name)
                        })
                    });
                    match existing {
                        Some(q) => q.to.push(c.to.clone()),
                        None => {
                            let base = c.to.split_once('.').map_or("in", |(_, p)| p).to_string();
                            let name =
                                unique(&|n: &str| def.inputs.iter().any(|q| q.name == n), &base);
                            def.inputs.push(InputPort {
                                name: name.clone(),
                                to: vec![c.to.clone()],
                                canvas: None,
                            });
                            outer.push(ConnectionEntry {
                                from: c.from.clone(),
                                to: format!("{instance}.{name}"),
                            });
                        }
                    }
                }
                (true, false) => {
                    // One output per inside source.
                    let name = match def.outputs.iter().find(|q| q.from == c.from) {
                        Some(q) => q.name.clone(),
                        None => {
                            let base = c.from.split_once('.').map_or("out", |(_, p)| p).to_string();
                            let name =
                                unique(&|n: &str| def.outputs.iter().any(|q| q.name == n), &base);
                            def.outputs.push(OutputPort {
                                name: name.clone(),
                                from: c.from.clone(),
                                canvas: None,
                            });
                            name
                        }
                    };
                    outer.push(ConnectionEntry {
                        from: format!("{instance}.{name}"),
                        to: c.to.clone(),
                    });
                }
            }
        }
        // Ports of the level itself that fed moved blocks now feed the
        // instance; outputs that showed a moved block show the instance.
        for q in &mut l.inputs {
            let (into, rest): (Vec<String>, Vec<String>) = q.to.drain(..).partition(|t| inside(t));
            q.to = rest;
            if !into.is_empty() {
                let base = into[0].split_once('.').map_or("in", |(_, p)| p).to_string();
                let name = unique(&|n: &str| def.inputs.iter().any(|p| p.name == n), &base);
                def.inputs.push(InputPort {
                    name: name.clone(),
                    to: into,
                    canvas: None,
                });
                q.to.push(format!("{instance}.{name}"));
            }
        }
        for q in &mut l.outputs {
            if inside(&q.from) {
                let base = q.from.split_once('.').map_or("out", |(_, p)| p).to_string();
                let name = unique(&|n: &str| def.outputs.iter().any(|p| p.name == n), &base);
                def.outputs.push(OutputPort {
                    name: name.clone(),
                    from: q.from.clone(),
                    canvas: None,
                });
                q.from = format!("{instance}.{name}");
            }
        }
        // Parameters the moved formulas use come in as the subsystem's own.
        let mut entry = BlockEntry {
            canvas: at,
            ..BlockEntry::instance(&instance, &def_name)
        };
        for b in &def.blocks {
            for v in b.params.values() {
                let ParamValue::Formula(text) = v else {
                    continue;
                };
                let Ok(ast) = simsandbox_expr::parse(text) else {
                    continue;
                };
                for (name, _) in ast.names() {
                    // With its unit, description and range (ADR 0045).
                    if let Some(decl) = scope_params.get(&name) {
                        def.params
                            .entry(name.clone())
                            .or_insert_with(|| decl.clone());
                        entry.params.insert(name.clone(), ParamValue::Formula(name));
                    }
                }
            }
        }
        l.blocks.retain(|b| !names.contains(&b.name));
        l.blocks.push(entry);
        l.connections = outer;
        Ok((instance, def))
    })();
    // Put the level back, changed or not.
    match scope {
        Scope::Model => {
            file.blocks = l.blocks;
            file.connections = l.connections;
        }
        Scope::Subsystem(s) => {
            file.subsystems.insert(s.clone(), l);
        }
    }
    let (instance, def) = result?;
    let count = def.blocks.len();
    file.subsystems.insert(def_name.clone(), def);
    if *scope == Scope::Model {
        // Flat names change: `car.v` is now `group1/car.v`.
        for n in names {
            let mut c = serde_json::to_value(&file.controls).map_err(|e| e.to_string())?;
            rename_in(&mut c, n, &format!("{instance}/{n}"));
            file.controls = serde_json::from_value(c).map_err(|e| e.to_string())?;
            if let Some(s) = &mut file.sample {
                for p in &mut s.params {
                    p.param = rename_refs(&p.param, n, &format!("{instance}/{n}"));
                }
            }
            for g in &mut file.groups {
                g.blocks.retain(|b| b != n);
            }
        }
        file.groups.retain(|g| g.blocks.len() > 1);
    }
    Ok(format!(
        "Grouped {count} blocks into the subsystem {def_name} (used as {instance})"
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use simsandbox_file::ParamValue;

    fn file(part: &str) -> ModelFile {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        ModelFile::from_json(e.text).unwrap().0
    }

    #[test]
    fn nodes_carry_their_values_on_the_model_level() {
        let r = registry();
        let f = file("Cruise");
        let d = Diagram::new(&f, &r);
        let car = &d.nodes[d.node("car").unwrap()];
        let names: Vec<&str> = car.values.iter().map(|v| v.name.as_str()).collect();
        // Starting values last.
        assert_eq!(names, ["m", "c", "v0"]);
        assert!(car.values[2].initial);
        assert_eq!(
            (car.values[0].value, car.values[0].unit.as_str()),
            (1000.0, "kg")
        );
        // A use shows its own parameters, not those formulas set.
        let f = file("Platoon");
        let mut d = Diagram::new(&f, &r);
        let car1 = &d.nodes[d.node("car1").unwrap()];
        let names: Vec<&str> = car1.values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["mass"]);
        // Closed, a node is as tall as its ports; open, its values add rows.
        let closed: Vec<f64> = d.nodes.iter().map(Node::height).collect();
        for n in &mut d.nodes {
            n.open = true;
        }
        assert!(d.height_check());
        assert!(d.nodes.iter().zip(closed).all(|(n, h)| n.height() >= h));
        // Inside a subsystem: every use's defaults, and which uses set
        // their own.
        let mut g = f.clone();
        g.blocks[2].params.insert("car.c".into(), 70.0.into());
        let inside = Diagram::of(&g, &Scope::Subsystem("follower".into()), &r);
        let car = &inside.nodes[inside.node("car").unwrap()];
        let c = car.values.iter().find(|v| v.name == "c").unwrap();
        assert_eq!(
            (c.value, c.overridden.clone()),
            (50.0, vec!["car2".to_string()])
        );
        // `m` is set by a formula: not listed.
        assert!(car.values.iter().all(|v| v.name != "m"));
        // The running values replace the file's.
        let s = crate::Session::load("p", &f.to_json(), &r).unwrap();
        let mut rows = s.params();
        rows.iter_mut()
            .find(|r| r.key == "car2.mass")
            .unwrap()
            .value = 1234.0;
        let mut d = Diagram::new(&f, &r);
        d.live_values(&rows);
        let car2 = &d.nodes[d.node("car2").unwrap()];
        assert_eq!(
            car2.values.iter().find(|v| v.name == "mass").unwrap().value,
            1234.0
        );
    }

    impl Diagram {
        /// Nodes with values are taller by their value rows.
        fn height_check(&self) -> bool {
            self.nodes.iter().all(|n| {
                let ports = HEADER + ROW * n.inputs.len().max(n.outputs.len()).max(1) as f64;
                (n.height() > ports) == !n.values.is_empty()
            })
        }
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
            last.canvas = None;
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
        f.blocks[0].canvas = Some([500.0, 100.0]);
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
        // A speed into a force is refused when loading (ADR 0045); put the
        // force back.
        assert!(loads(&f).unwrap_err().contains("unit mismatch"));
        apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "cruise.F".into(),
                to: "car.F".into(),
            },
        )
        .unwrap();

        apply(
            &mut f,
            &r,
            &Edit::Rename {
                from: "cruise".into(),
                to: "pid".into(),
            },
        )
        .unwrap();
        assert!(f.connections.iter().any(|c| c.from == "pid.F"));
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
        assert_eq!(f.blocks.last().unwrap().canvas, Some([12.3, 0.0]));
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
        assert_eq!(
            rename_refs("car1/car.v + car10/car.v", "car1", "lead"),
            "lead/car.v + car10/car.v"
        );
    }

    /// The final value of a recorded signal after `t` seconds.
    fn final_value(f: &ModelFile, signal: &str, t: f64) -> f64 {
        let r = crate::run::run(&f.to_json(), &registry(), Some(t), None).unwrap();
        let (block, port) = signal.split_once('.').unwrap();
        let _ = block;
        let o = r
            .outputs
            .iter()
            .find(|o| o.name == signal)
            .unwrap_or_else(|| panic!("no {signal} ({port})"));
        *o.values.last().unwrap()
    }

    #[test]
    fn subsystems_show_as_one_node_and_open_to_their_inside() {
        let r = registry();
        let f = file("Platoon");
        let d = Diagram::new(&f, &r);
        let car1 = &d.nodes[d.node("car1").unwrap()];
        assert_eq!(car1.kind, NodeKind::Subsystem);
        assert_eq!(car1.type_name, "follower");
        assert_eq!(car1.inputs[0].name, "lead");
        assert_eq!(car1.outputs[0].name, "v");
        // The port carries the unit of the inner port it shows.
        assert_eq!(car1.outputs[0].unit, "m/s");
        assert_eq!((d.wires.len(), d.dangling.len()), (3, 0));
        assert_eq!(d.subsystems, ["follower"]);

        let inside = Diagram::of(&f, &Scope::Subsystem("follower".into()), &r);
        let names: Vec<&str> = inside.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["car", "control", "in:lead", "out:v"]);
        assert_eq!((inside.wires.len(), inside.dangling.len()), (4, 0));
        assert_eq!(inside.used_by, ["car1", "car2", "car3"]);
        let out = &inside.nodes[inside.node("out:v").unwrap()];
        assert!(out.at[0] > inside.nodes[0].at[0], "outputs go right");
    }

    #[test]
    fn edits_inside_a_subsystem_change_every_use() {
        let r = registry();
        let mut f = file("Platoon");
        let inside = Scope::Subsystem("follower".into());
        let loads = |f: &ModelFile| crate::Session::load("t", &f.to_json(), &r).map(|_| ());
        // Rename an input: wires to it everywhere follow.
        apply_in(
            &mut f,
            &inside,
            &r,
            &Edit::Rename {
                from: "in:lead".into(),
                to: "ahead".into(),
            },
        )
        .unwrap();
        assert!(f.connections.iter().any(|c| c.to == "car2.ahead"));
        loads(&f).unwrap();
        // A new output, fed from the controller's force.
        apply_in(
            &mut f,
            &inside,
            &r,
            &Edit::AddPort {
                input: false,
                at: [400.0, 0.0],
            },
        )
        .unwrap();
        loads(&f).unwrap_err();
        apply_in(
            &mut f,
            &inside,
            &r,
            &Edit::Connect {
                from: "control.F".into(),
                to: "out:out1.out1".into(),
            },
        )
        .unwrap();
        loads(&f).unwrap();
        let d = Diagram::new(&f, &r);
        assert_eq!(d.nodes[d.node("car3").unwrap()].outputs.len(), 2);
        // Rename an inner block: its flat names in views follow.
        apply_in(
            &mut f,
            &inside,
            &r,
            &Edit::Rename {
                from: "car".into(),
                to: "body".into(),
            },
        )
        .unwrap();
        let views = serde_json::to_string(&f.controls.views).unwrap();
        assert!(views.contains("car2/body.v"), "{views}");
        loads(&f).unwrap();
        // Remove the input: the wires into its uses go too, and the inner
        // input it fed is open.
        apply_in(
            &mut f,
            &inside,
            &r,
            &Edit::Delete {
                block: "in:ahead".into(),
            },
        )
        .unwrap();
        assert!(!f.connections.iter().any(|c| c.to.ends_with(".ahead")));
        assert!(loads(&f).unwrap_err().contains("lead"));
        assert!(apply_in(
            &mut f,
            &Scope::Model,
            &r,
            &Edit::AddPort {
                input: true,
                at: [0.0, 0.0]
            }
        )
        .is_err());
    }

    #[test]
    fn open_nodes_follow_renames_and_deletions() {
        let r = registry();
        let mut f = file("Platoon");
        f.controls.open = vec!["car1".into(), "car2".into(), "follower::car".into()];
        let rename = Edit::Rename {
            from: "car1".into(),
            to: "first".into(),
        };
        apply(&mut f, &r, &rename).unwrap();
        apply(
            &mut f,
            &r,
            &Edit::Delete {
                block: "car2".into(),
            },
        )
        .unwrap();
        let inside = Scope::Subsystem("follower".into());
        let rename = Edit::Rename {
            from: "car".into(),
            to: "body".into(),
        };
        apply_in(&mut f, &inside, &r, &rename).unwrap();
        let rename = Edit::RenameSubsystem {
            from: "follower".into(),
            to: "car_pi".into(),
        };
        apply(&mut f, &r, &rename).unwrap();
        assert_eq!(f.controls.open, ["first", "car_pi::body"]);
    }

    #[test]
    fn grouping_blocks_keeps_the_model_and_its_numbers() {
        let r = registry();
        let before = file("Cruise");
        let v = final_value(&before, "car.v", 5.0);
        // The controller alone: wires crossing the boundary become ports.
        let mut f = before.clone();
        let note = apply(
            &mut f,
            &r,
            &Edit::Group {
                blocks: vec!["cruise".into()],
            },
        )
        .unwrap();
        assert!(note.contains("group1"), "{note}");
        let def = &f.subsystems["group1"];
        assert_eq!(def.inputs[0].name, "v");
        assert_eq!(def.outputs[0].name, "F");
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "car.v" && c.to == "group1.v"));
        assert!(f.controls.params.contains_key("group1/cruise.kp"));
        assert_eq!(final_value(&f, "car.v", 5.0), v);
        // Both blocks: no ports, the same numbers.
        let mut f = before.clone();
        apply(
            &mut f,
            &r,
            &Edit::Group {
                blocks: vec!["car".into(), "cruise".into()],
            },
        )
        .unwrap();
        assert!(f.connections.is_empty());
        assert_eq!(final_value(&f, "group1/car.v", 5.0), v);
        // Instances whose parameters are formulas of the model's: they
        // come in as the new subsystem's parameters.
        let mut p = file("Platoon");
        let v3 = final_value(&p, "car3/car.v", 5.0);
        apply(
            &mut p,
            &r,
            &Edit::Group {
                blocks: vec!["car1".into(), "car2".into()],
            },
        )
        .unwrap();
        let def = &p.subsystems["group1"];
        assert!(def.params.contains_key("kp") && def.params.contains_key("ki"));
        assert_eq!(
            p.blocks.last().unwrap().params["kp"],
            ParamValue::Formula("kp".into())
        );
        assert_eq!(final_value(&p, "car3/car.v", 5.0), v3);
        // And a subsystem can be renamed.
        apply(
            &mut p,
            &r,
            &Edit::RenameSubsystem {
                from: "follower".into(),
                to: "chaser".into(),
            },
        )
        .unwrap();
        assert!(p.blocks.iter().any(|b| b.type_name == "chaser"));
        assert_eq!(final_value(&p, "car3/car.v", 5.0), v3);
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
