//! The model graph as the node canvas shows it (ADR 0042, ADR 0043): one
//! level at a time (the model, or a subsystem's inside), with its blocks,
//! subsystem instances and, inside a subsystem, nodes for its inputs and
//! outputs; wires; positions (saved as `at`, or laid out left to right by
//! signal flow); and the edits the canvas makes, as changes to the model
//! file. Shared by the app and headless rendering.

use serde_json::Value;
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::Registry;
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

/// Which level of the model a diagram shows: the model's own. (The inside
/// of a used model is shown from its own file or definition, ADR 0068.)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Model,
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
    /// A filter on the data layer (ADR 0050): reads recorded data.
    Filter,
    /// A view on the data layer: where data is shown.
    View,
    /// A probe on the data layer (ADR 0062): what is recorded, when, and
    /// the file it writes.
    Probe,
    /// The recorder (ADR 0064): with it, the model can be rewound.
    Recorder,
    /// A use of a model file (ADR 0066): its input blocks as inputs, its
    /// outputs by `block.port`, titled with its clock.
    Use,
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
    /// A used model's ports not shown on its node (ADR 0066 step 4b),
    /// reachable by path from its list: `true` for an input.
    pub reachable: Vec<(bool, Port)>,
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
    /// Exposed as the model's parameter of this name (ADR 0074): shown
    /// with the model's value, set by each use or study.
    pub exposed: Option<String>,
    /// The signal driving it (ADR 0075), `node.port`: shown instead of a
    /// number; the value is the signal's.
    pub driven: Option<String>,
}

impl Node {
    pub fn height(&self) -> f64 {
        let ports = HEADER + ROW * self.inputs.len().max(self.outputs.len()).max(1) as f64;
        match self.value_rows() {
            0 => ports,
            n => ports + 4.0 + VALUE_ROW * n as f64,
        }
    }

    /// Rows of values drawn: all of them while the node is open, else
    /// the driven ones, which come first (their wires end there, ADR 0075).
    pub fn value_rows(&self) -> usize {
        if self.open {
            self.values.len()
        } else {
            self.values
                .iter()
                .take_while(|v| v.driven.is_some())
                .count()
        }
    }

    /// Where a wire driving value `k` attaches: the left of its row.
    pub fn value_port(&self, k: usize) -> [f64; 2] {
        let [x, y] = self.value_at(k);
        [x, y + VALUE_ROW / 2.0]
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
        matches!(self.kind, NodeKind::Block | NodeKind::Use)
    }

    /// Whether it is on the data layer (ADR 0050).
    pub fn is_observer(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Filter | NodeKind::View | NodeKind::Probe | NodeKind::Recorder
        )
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
    /// It drives a parameter (ADR 0075): `to.1` is the value's index in
    /// its node's `values`, not an input.
    pub param: bool,
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
    /// The models defined in the file (ADR 0068), which can be added like
    /// block types.
    pub subsystems: Vec<String>,
    /// The outputs the model offers to models using it: name and
    /// `block.port` (ADR 0066 step 4b).
    pub offered: Vec<(String, String)>,
    /// View types that can be added on the data layer: name and label.
    pub view_types: Vec<(String, String)>,
    /// Data-layer slots that take several signals, as `view:K.slot`.
    pub many: BTreeSet<String>,
    /// Uses expanded in place (ADR 0066 step 4d), outermost first.
    pub outlines: Vec<crate::expand::Outline>,
    /// Nodes pushed aside on screen by an expanded use, by name: what to
    /// take off their place before it is saved.
    pub shifted: BTreeMap<String, [f64; 2]>,
    /// The inside of a used model, open on the canvas: its blocks can offer
    /// outputs to the models using it (ADR 0073).
    pub inside: bool,
    /// What the model exposes (ADR 0074), by node name: `in:block.port`,
    /// `out:block.port` and `par:block.param`, each to the name its users
    /// see.
    pub exposed: BTreeMap<String, String>,
}

/// The blocks and wires of one level.
struct Level<'a> {
    blocks: &'a [BlockEntry],
    connections: &'a [ConnectionEntry],
}

fn level<'a>(file: &'a ModelFile, scope: &Scope) -> Option<Level<'a>> {
    match scope {
        Scope::Model => Some(Level {
            blocks: &file.blocks,
            connections: &file.connections,
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
            .map(|b| {
                if b.is_use() {
                    let wired = simsandbox_file::compose::wired(l.connections, &b.name);
                    use_node(b, registry, file.folder.as_deref(), 0, wired)
                } else {
                    node(b, registry, file.folder.as_deref(), &file.params)
                }
            })
            .collect();
        reveal_wired(&mut nodes, file);
        let index: BTreeMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.name.clone(), i))
            .collect();
        let find = |key: &str, causality: Causality, nodes: &[Node]| {
            // Inside a use of a model file: the rest of the path is its
            // port (`plant/car.v`, `controller/speed`).
            if let Some((u, rest)) = key.split_once('/') {
                if let Some(&n) = index.get(u).filter(|&&n| nodes[n].kind == NodeKind::Use) {
                    let ports = match causality {
                        Causality::Input => &nodes[n].inputs,
                        Causality::Output => &nodes[n].outputs,
                    };
                    return Some((n, ports.iter().position(|q| q.name == rest)?));
                }
            }
            let (b, p) = key.split_once('.')?;
            let &n = index.get(b)?;
            let ports = match causality {
                Causality::Input => &nodes[n].inputs,
                Causality::Output => &nodes[n].outputs,
            };
            Some((n, ports.iter().position(|q| q.name == p)?))
        };
        // Probes feeding blocks are drawn with the data layer (ADR 0063).
        let links: Vec<(String, String)> = l
            .connections
            .iter()
            .filter(|c| !c.from.starts_with("probe:"))
            .map(|c| (c.from.clone(), c.to.clone()))
            .collect();
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
                    param: false,
                }),
                // Into a parameter: it is driven (ADR 0075).
                (Some(f), None) => match value_of(&nodes, &index, &to) {
                    Some(t) => wires.push(Wire {
                        from: f,
                        to: t,
                        from_name: from,
                        to_name: to,
                        signal: String::new(),
                        param: true,
                    }),
                    None => dangling.push(format!("{from} -> {to}")),
                },
                _ => dangling.push(format!("{from} -> {to}")),
            }
        }
        drive_values(&mut nodes, &mut wires);
        layout(&mut nodes, &wires);
        Self {
            scope: scope.clone(),
            nodes,
            wires,
            dangling,
            subsystems: file.models.keys().cloned().collect(),
            offered: file
                .outputs
                .iter()
                .map(|o| (o.name.clone(), o.from.clone()))
                .collect(),
            exposed: exposed(file),
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

    /// Whether node `i`'s ports and parameters can be exposed (ADR 0074):
    /// a block inside a model, open or expanded in place.
    pub fn exposable(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        n.kind == NodeKind::Block
            && n.type_name != simsandbox_file::STUDY
            && (self.inside || n.name.contains('/'))
    }

    /// The name node `i`'s port or parameter is exposed as, if it is:
    /// `kind` is `in`, `out` or `par`.
    pub fn exposed_as(&self, i: usize, kind: &str, item: &str) -> Option<&str> {
        self.exposed
            .get(&format!("{kind}:{}.{item}", self.nodes[i].name))
            .map(String::as_str)
    }

    /// The edit that exposes node `i`'s port or parameter, or hides it if
    /// it is exposed (ADR 0074).
    pub fn toggle_exposed(&self, i: usize, kind: &str, item: &str) -> Edit {
        let at = format!("{}.{item}", self.nodes[i].name);
        let shown = self.exposed_as(i, kind, item).is_some();
        match (kind, shown) {
            ("in", false) => Edit::ExposeInput { input: at },
            ("in", true) => Edit::HideInput { input: at },
            ("out", false) => Edit::DeclareOutput {
                name: String::new(),
                from: at,
            },
            ("out", true) => Edit::HideOutput { from: at },
            (_, false) => Edit::ExposeParam {
                block: self.nodes[i].name.clone(),
                param: item.to_string(),
            },
            (_, true) => Edit::HideParam {
                block: self.nodes[i].name.clone(),
                param: item.to_string(),
            },
        }
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

fn build(
    b: &BlockEntry,
    registry: &Registry,
    folder: Option<&std::path::Path>,
) -> Result<Box<dyn simsandbox_core::block::Block>, String> {
    let config = b.config(folder);
    registry
        .build(&b.type_name, &b.name, &config)
        .map_err(|e| e.to_string())
}

/// Show on a used model's node the ports something reaches by path
/// (ADR 0066 step 4b): wires and links into and out of it, what probes
/// record, what views and filters read.
fn reveal_wired(nodes: &mut [Node], file: &ModelFile) {
    let mut ends: Vec<(bool, String)> = Vec::new();
    for c in &file.connections {
        let from = c
            .from
            .strip_prefix("probe:")
            .and_then(|r| r.split_once('.'))
            .map_or(c.from.as_str(), |(_, s)| s);
        ends.push((false, from.to_string()));
        ends.push((true, c.to.clone()));
    }
    for p in &file.controls.probes {
        ends.extend(p.record.iter().map(|r| (false, r.clone())));
    }
    for v in &file.controls.views {
        for b in v.bind.values() {
            ends.extend(b.signals().into_iter().map(|s| (false, s)));
        }
    }
    for f in &file.controls.filters {
        ends.extend(
            crate::observers::filter_reads(f)
                .into_iter()
                .map(|s| (false, s)),
        );
    }
    for (input, end) in ends {
        let Some((u, rest)) = end.split_once('/') else {
            continue;
        };
        let Some(n) = nodes
            .iter_mut()
            .find(|n| n.kind == NodeKind::Use && n.name == u)
        else {
            continue;
        };
        let shown = if input { &n.inputs } else { &n.outputs };
        if shown.iter().any(|p| p.name == rest)
            || (!input
                && n.settings
                    .iter()
                    .any(|(k, v)| k.starts_with("output.") && v == rest))
        {
            continue;
        }
        if let Some(k) = n
            .reachable
            .iter()
            .position(|(i, p)| *i == input && p.name == rest)
        {
            let (_, port) = n.reachable.remove(k);
            if input {
                n.inputs.push(port);
            } else {
                n.outputs.push(port);
            }
        }
    }
}

/// A use of a model file (ADR 0066): read from `folder` (or the thread's,
/// `in_folder`); its input blocks are its inputs, the outputs of its
/// blocks and of the uses inside it (`engine/x.y`) its outputs. Shown
/// with its clock; an error if the file cannot be read.
fn use_node(
    b: &BlockEntry,
    registry: &Registry,
    folder: Option<&std::path::Path>,
    depth: usize,
    wired: bool,
) -> Node {
    use simsandbox_file::UseClock;
    let file_name = match (&b.file, &b.model) {
        (Some(f), _) => f.clone(),
        (None, Some(m)) => format!("model {m}"),
        (None, None) => String::new(),
    };
    let mut n = Node {
        name: b.name.clone(),
        kind: NodeKind::Use,
        type_name: file_name.clone(),
        method: String::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        at: b.canvas.unwrap_or_default(),
        placed: b.canvas.is_some(),
        error: None,
        unreached: Vec::new(),
        values: Vec::new(),
        open: false,
        reachable: Vec::new(),
        settings: Default::default(),
    };
    let folder = folder
        .map(std::path::Path::to_path_buf)
        .or_else(simsandbox_core::registry::current_folder);
    // As the run reads it: edited files and replacements (ADR 0066 4c).
    let read = simsandbox_file::compose::used_model(b, folder.as_deref());
    let (mut model, inner_folder) = match read {
        Ok((m, _, inner)) if depth < 8 => (m, inner),
        Ok(_) => {
            n.error = Some("model files used too deeply (or using themselves)".into());
            return n;
        }
        Err(e) => {
            n.error = Some(e);
            return n;
        }
    };
    // Its exposed inputs are ports, as input blocks are (ADR 0074).
    simsandbox_file::compose::expose_inputs(&mut model);
    let mut has_states = false;
    // Every input and output it has, by path; the node shows its
    // interface and keeps the rest reachable (ADR 0066 step 4b).
    let mut all_inputs: Vec<Port> = Vec::new();
    let mut all_outputs: Vec<Port> = Vec::new();
    let mut own_inputs: Vec<Port> = Vec::new();
    for e in &model.blocks {
        if e.is_use() {
            let wired = simsandbox_file::compose::wired(&model.connections, &e.name);
            let inner = use_node(e, registry, inner_folder.as_deref(), depth + 1, wired);
            has_states |= !inner.method.contains("new data");
            let prefixed = |p: Port| Port {
                name: format!("{}/{}", e.name, p.name),
                ..p
            };
            all_inputs.extend(inner.inputs.into_iter().map(prefixed));
            all_outputs.extend(inner.outputs.into_iter().map(prefixed));
            for (input, p) in inner.reachable {
                if input {
                    all_inputs.push(prefixed(p));
                } else {
                    all_outputs.push(prefixed(p));
                }
            }
            continue;
        }
        if e.type_name == simsandbox_eq::input::VALUE {
            // An input block is an input; its own `y` is not an output.
            let unit = e.settings.get("unit").cloned().unwrap_or_default();
            own_inputs.push(Port {
                name: e.name.clone(),
                unit,
                shape: Shape::Scalar,
            });
            continue;
        }
        let ports: Vec<Port> = {
            match build(e, registry, inner_folder.as_deref()) {
                Ok(block) => {
                    let spec = block.spec();
                    has_states |= !spec.states.is_empty() || !spec.tables.is_empty();
                    ports(spec, Causality::Output)
                }
                Err(_) => Vec::new(),
            }
        };
        all_outputs.extend(ports.into_iter().map(|p| Port {
            name: format!("{}.{}", e.name, p.name),
            ..p
        }));
    }
    // Declared outputs, shaped like what they stand for.
    let declared: Vec<Port> = model
        .outputs
        .iter()
        .map(|o| {
            let source = all_outputs.iter().find(|p| p.name == o.from);
            Port {
                name: o.name.clone(),
                unit: source.map(|p| p.unit.clone()).unwrap_or_default(),
                shape: source.map_or(Shape::Scalar, |p| p.shape),
            }
        })
        .collect();
    // The values of this use: its model's declared parameters, as this
    // use sets them; those a formula sets are not listed (ADR 0068).
    for (name, decl) in &model.params {
        let value = match b.params.get(name) {
            Some(simsandbox_file::ParamValue::Formula(_)) => continue,
            Some(simsandbox_file::ParamValue::Number(x)) => *x,
            None => decl.value,
        };
        n.values.push(NodeValue {
            key: format!("{}.{name}", b.name),
            name: name.clone(),
            value,
            formula: None,
            unit: decl.unit.clone(),
            initial: false,
            overridden: Vec::new(),
            exposed: None,
            driven: None,
        });
    }
    // What each declared output shows, so a signal named by it is drawn
    // from that port (`car1/car.v` from `v`).
    for o in &model.outputs {
        n.settings
            .insert(format!("output.{}", o.name), o.from.clone());
    }
    n.inputs = own_inputs;
    // A signal a declared output shows is reached by that name only
    // (ADR 0073).
    let shown: Vec<&str> = model.outputs.iter().map(|o| o.from.as_str()).collect();
    n.reachable = all_inputs
        .into_iter()
        .map(|p| (true, p))
        .chain(
            all_outputs
                .into_iter()
                .filter(|p| !shown.contains(&p.name.as_str()))
                .map(|p| (false, p)),
        )
        .collect();
    n.outputs = declared;
    n.method = match &b.clock {
        Some(UseClock::Step(dt)) => format!("every {dt} s"),
        Some(UseClock::Shared) => "shared clock".into(),
        Some(UseClock::Times(t)) => format!("at {} listed times", t.len()),
        // ADR 0068: wired and with no step of its own, it shares.
        None if wired && !model.run.dt_written => "shared clock".into(),
        None if has_states => format!("every {} s", model.run.dt),
        None => "on new data".into(),
    };
    n
}

/// The value a wire into `to` drives (ADR 0075): `block.param`, or a
/// used model's `use/param`; node and value index.
fn value_of(nodes: &[Node], index: &BTreeMap<String, usize>, to: &str) -> Option<(usize, usize)> {
    let (n, param) = match to.split_once('/') {
        Some((u, p)) if !p.contains(['.', '/']) => (*index.get(u)?, p),
        _ => {
            let (b, p) = to.split_once('.')?;
            (*index.get(b)?, p)
        }
    };
    let k = nodes[n].values.iter().position(|v| v.name == param)?;
    Some((n, k))
}

/// Mark the values wires drive and list them first on their nodes, so a
/// closed node still shows where its wires end; wires follow the values.
pub(crate) fn drive_values(nodes: &mut [Node], wires: &mut [Wire]) {
    for w in wires.iter().filter(|w| w.param) {
        if let Some(v) = nodes[w.to.0].values.get_mut(w.to.1) {
            v.driven = Some(w.from_name.clone());
        }
    }
    for (i, n) in nodes.iter_mut().enumerate() {
        if n.values.iter().all(|v| v.driven.is_none()) {
            continue;
        }
        let before: Vec<String> = n.values.iter().map(|v| v.name.clone()).collect();
        n.values.sort_by_key(|v| v.driven.is_none());
        for w in wires.iter_mut().filter(|w| w.param && w.to.0 == i) {
            let name = &before[w.to.1];
            w.to.1 = n.values.iter().position(|v| &v.name == name).unwrap_or(0);
        }
    }
}

/// What `file` exposes to its users (ADR 0074), as [`Diagram::exposed`]
/// lists it.
fn exposed(file: &ModelFile) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for i in &file.inputs {
        for t in &i.to {
            out.insert(format!("in:{t}"), i.name.clone());
        }
    }
    for o in &file.outputs {
        out.insert(format!("out:{}", o.from), o.name.clone());
    }
    for b in &file.blocks {
        for (p, v) in &b.params {
            if let ParamValue::Formula(f) = v {
                if file.params.contains_key(f.trim()) {
                    out.insert(format!("par:{}.{p}", b.name), f.trim().to_string());
                }
            }
        }
    }
    out
}

fn node(
    b: &BlockEntry,
    registry: &Registry,
    folder: Option<&std::path::Path>,
    declared: &BTreeMap<String, simsandbox_file::ParamDecl>,
) -> Node {
    let mut method = b.method.name.clone();
    let mut values = Vec::new();
    let (inputs, outputs, error) = match build(b, registry, folder) {
        Ok(block) => {
            if method.is_empty() {
                method = block.method().name.clone();
            }
            // Its parameters as the file gives them, else their defaults.
            for p in &block.spec().params {
                let (value, exposed) = match b.params.get(&p.name) {
                    Some(ParamValue::Number(v)) => (*v, None),
                    // Exposed as the model's parameter: its value.
                    Some(ParamValue::Formula(f)) => match declared.get(f.trim()) {
                        Some(d) => (d.value, Some(f.trim().to_string())),
                        // Set by a formula: not listed.
                        None => continue,
                    },
                    None => (p.default, None),
                };
                values.push(NodeValue {
                    key: format!("{}.{}", b.name, p.name),
                    name: p.name.clone(),
                    value,
                    formula: None,
                    unit: unit_text(&p.unit),
                    initial: p.initial_condition,
                    overridden: Vec::new(),
                    exposed,
                    driven: None,
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
    // A study says what it studies and how many runs it makes (ADR 0070).
    if b.type_name == crate::study::STUDY {
        let what = b.model.as_deref().or(b.file.as_deref()).unwrap_or("?");
        method = match outputs.first() {
            Some(p) => format!("of {what} · {} runs", p.shape.len()),
            None => format!("of {what}"),
        };
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
        reachable: Vec::new(),
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
        // Wires into values follow them by name (ADR 0075).
        let names: Vec<(usize, String)> = self
            .wires
            .iter()
            .map(|w| match self.nodes[w.to.0].values.get(w.to.1) {
                Some(v) if w.param => (w.to.0, v.name.clone()),
                _ => (usize::MAX, String::new()),
            })
            .collect();
        for n in self.nodes.iter_mut().filter(|n| n.is_block()) {
            // Values formulas set are not listed, but for exposed and
            // driven ones.
            n.values.retain(|v| {
                v.exposed.is_some()
                    || v.driven.is_some()
                    || rows
                        .iter()
                        .find(|r| r.key == v.key)
                        .is_none_or(|r| r.formula.is_none())
            });
            for v in &mut n.values {
                let key = v.exposed.as_ref().unwrap_or(&v.key);
                if let Some(r) = rows.iter().find(|r| &r.key == key) {
                    v.value = r.value;
                }
            }
        }
        for (w, (node, name)) in self.wires.iter_mut().zip(names) {
            if node != usize::MAX {
                w.to.1 = self.nodes[node]
                    .values
                    .iter()
                    .position(|v| v.name == name)
                    .unwrap_or(0);
            }
        }
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

/// A change the canvas makes to the model, at one level.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Only the layout: never restarts the run. Also moves port nodes.
    Move { block: String, at: [f64; 2] },
    /// A registered type or one of the file's subsystems.
    Add { type_name: String, at: [f64; 2] },
    /// A use of a model file (ADR 0066), named after it, on its own clock;
    /// the path is kept relative to the model's folder when inside it.
    AddUse { file: String, at: [f64; 2] },
    /// A block, or a port node (removing the port).
    Delete { block: String },
    /// Copies of these blocks or uses, with their values and the wires
    /// among them, beside the originals (ADR 0069); edited apart after.
    Duplicate { blocks: Vec<String> },
    /// A study block beside a used model, of the same model (ADR 0070):
    /// a grid of 20 points over its first parameter, its inputs fed as
    /// the use's are.
    Study { block: String },
    /// Study the whole model open (ADR 0071): its blocks move into a new
    /// definition used in their place ("Make model"), with a study of it
    /// beside the use ("Study this model").
    StudyModel,
    /// A study's `sample` (ADR 0070), as JSON.
    SetSample { block: String, sample: String },
    /// When a study runs (ADR 0088): a mode (`rise`, `fall`, `change`,
    /// `while`), or none. Without one, its `activate` input and the wire
    /// into it go.
    SetActivate { block: String, mode: Option<String> },
    /// A strategy driving a study (ADR 0078 point 7, "Search…"): the
    /// strategy as JSON (`crate::strategies::Strategy`), written by
    /// `crate::strategies::write`.
    Search { study: String, strategy: String },
    /// A block, or a port node (renaming the port).
    Rename { from: String, to: String },
    /// `"node.port"` to `"node.port"`; replaces whatever fed the input.
    Connect { from: String, to: String },
    /// Remove the wire into this input.
    Disconnect { to: String },
    /// Move these blocks into a new model defined in the file and use it
    /// in their place (ADR 0068): crossing wires become its input blocks
    /// and declared outputs.
    GroupModel { blocks: Vec<String> },
    /// Offer an output to models using this one (ADR 0066 step 4b): a
    /// declared output `name` showing `from` (`block.port`).
    DeclareOutput { name: String, from: String },
    /// Stop offering `from` (`block.port`) to models using this one
    /// (ADR 0074): the declared outputs showing it go.
    HideOutput { from: String },
    /// Expose a block's parameter as the model's (ADR 0074): a declared
    /// parameter of its name with its value and unit, which the block's
    /// parameter is then set to.
    ExposeParam { block: String, param: String },
    /// Stop exposing it: the block keeps the value, the declaration goes
    /// when nothing else reads it.
    HideParam { block: String, param: String },
    /// Expose a block's input (`block.port`) as the model's input (ADR
    /// 0074): a signal its users wire.
    ExposeInput { input: String },
    /// Stop exposing the block input `block.port`.
    HideInput { input: String },
    /// Stop offering the declared output `name`.
    RemoveOutput { name: String },
    /// Rename a model defined in the file (ADR 0068): its uses follow.
    RenameModel { from: String, to: String },
    /// A typed-equation block's equations, one per line (ADR 0017): the
    /// parameters it no longer has are dropped, and its method follows
    /// its kind.
    SetEquations {
        block: String,
        equations: Vec<String>,
    },
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
    /// Data layer: whether a filter with inputs from probes runs whenever
    /// one of them records (`any`), or where their times pair (ADR 0063).
    SetFilterWhen { filter: String, any: bool },
    /// Data layer: add the recorder, so the model can be rewound (ADR 0064).
    AddRecorder { at: [f64; 2] },
    /// Data layer: the recorder's settings.
    SetRecorder {
        recorder: simsandbox_file::RecorderEntry,
    },
    /// Data layer: a new probe recording nothing yet (ADR 0062).
    AddProbe { at: [f64; 2] },
    /// Data layer: a probe's settings (what, when, how long, its file),
    /// replacing the probe of the same name.
    SetProbe { probe: simsandbox_file::ProbeEntry },
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
            | Edit::AddView { .. }
            | Edit::AddProbe { .. }
            | Edit::SetProbe { .. }
            | Edit::SetFilterWhen { .. }
            | Edit::AddRecorder { .. }
            | Edit::SetRecorder { .. } => true,
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
    let note = apply_in_level(file, scope, registry, edit)?;
    // A study names its model by `model`: found again (ADR 0070).
    if matches!(
        edit,
        Edit::Study { .. } | Edit::StudyModel | Edit::SetSetting { .. } | Edit::Duplicate { .. }
    ) {
        file.resolve_models().map_err(|e| e.to_string())?;
    }
    Ok(note)
}

fn apply_in_level(
    file: &mut ModelFile,
    scope: &Scope,
    registry: &Registry,
    edit: &Edit,
) -> Result<String, String> {
    if let Edit::AddUse { file: path, at } = edit {
        return add_use(file, scope, path, *at);
    }
    // Wires to and from used models (ADR 0066 step 4c).
    if *scope == Scope::Model {
        match edit {
            Edit::Connect { from, to }
                if use_end(file, from).is_some() || use_end(file, to).is_some() =>
            {
                return connect_uses(file, scope, registry, from, to);
            }
            Edit::Disconnect { to } if use_end(file, to).is_some() => {
                let to = use_end(file, to).expect("checked");
                return disconnect_use(file, &to);
            }
            _ => {}
        }
    }
    if let Some(result) = crate::observers::apply(file, scope, registry, edit) {
        return result;
    }
    match edit {
        Edit::DeclareOutput { name, from } => return declare_output(file, name, from),
        Edit::ExposeParam { block, param } => return expose_param(file, registry, block, param),
        Edit::HideParam { block, param } => return hide_param(file, block, param),
        Edit::ExposeInput { input } => return expose_input(file, registry, input),
        Edit::HideInput { input } => {
            let before: usize = file.inputs.iter().map(|i| i.to.len()).sum();
            for i in &mut file.inputs {
                i.to.retain(|t| t != input);
            }
            file.inputs.retain(|i| !i.to.is_empty());
            let after: usize = file.inputs.iter().map(|i| i.to.len()).sum();
            return if after < before {
                Ok(format!("{input} is no longer an input of the model"))
            } else {
                Err(format!("{input} is not an input of the model"))
            };
        }
        Edit::HideOutput { from } => {
            let before = file.outputs.len();
            file.outputs.retain(|o| &o.from != from);
            return if file.outputs.len() < before {
                Ok(format!("{from} is no longer an output of the model"))
            } else {
                Err(format!("{from} is not an output of the model"))
            };
        }
        Edit::RemoveOutput { name } => {
            let before = file.outputs.len();
            file.outputs.retain(|o| &o.name != name);
            return if file.outputs.len() < before {
                Ok(format!("No longer offers {name}"))
            } else {
                Err(format!("it offers no output '{name}'"))
            };
        }
        Edit::RenameModel { from, to } => return rename_model(file, from, to),
        _ => {}
    }
    if let Edit::GroupModel { blocks } = edit {
        return group_model(file, scope, blocks);
    }
    if let Edit::StudyModel = edit {
        return study_model(file, scope, registry);
    }
    if let Edit::Search { study, strategy } = edit {
        if *scope != Scope::Model {
            return Err(
                "a strategy drives a study of the model open, not inside a subsystem".into(),
            );
        }
        let spec: crate::strategies::Strategy =
            serde_json::from_str(strategy).map_err(|e| e.to_string())?;
        return crate::strategies::write(file, registry, Some(study), &spec);
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
            let result = edit_level(&mut def, file, registry, edit);
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
    }
}

/// What a study of `model` can vary (ADR 0070): its declared parameters,
/// then each block parameter (`car.m`), with a range around its value (the
/// declared or suggested range where there is one) and whether it is an
/// initial condition. Input blocks are the study's inputs, not listed.
pub fn study_parameters(
    model: &ModelFile,
    registry: &Registry,
    folder: Option<&std::path::Path>,
) -> Vec<(String, [f64; 2], bool)> {
    let around = |v: f64| {
        if v == 0.0 {
            [0.0, 1.0]
        } else {
            let [a, b] = [v / 2.0, v * 2.0];
            [a.min(b), a.max(b)]
        }
    };
    let mut out: Vec<(String, [f64; 2], bool)> = model
        .params
        .iter()
        .map(|(k, d)| (k.clone(), d.range.unwrap_or_else(|| around(d.value)), false))
        .collect();
    for b in &model.blocks {
        if b.is_use() || b.type_name == simsandbox_eq::input::VALUE {
            continue;
        }
        let Ok(block) = registry.build(&b.type_name, &b.name, &b.config(folder)) else {
            continue;
        };
        for p in &block.spec().params {
            let value = match b.params.get(&p.name) {
                Some(ParamValue::Number(v)) => *v,
                _ => p.default,
            };
            let range = if p.suggested_min < p.suggested_max && !b.params.contains_key(&p.name) {
                [p.suggested_min, p.suggested_max]
            } else {
                around(value)
            };
            out.push((format!("{}.{}", b.name, p.name), range, p.initial_condition));
        }
    }
    out
}

/// The parameter "Study this model" varies first: the first declared
/// parameter, else the first block parameter that is not an initial
/// condition.
fn study_parameter(
    model: &ModelFile,
    registry: &Registry,
    folder: Option<&std::path::Path>,
) -> Option<(String, [f64; 2])> {
    let all = study_parameters(model, registry, folder);
    let declared = model.params.len();
    all.iter()
        .enumerate()
        .find(|(k, (_, _, ic))| *k < declared || !ic)
        .map(|(_, (key, range, _))| (key.clone(), *range))
}

/// Edit one level's blocks and wires.
fn edit_level(
    l: &mut Subsystem,
    file: &ModelFile,
    registry: &Registry,
    edit: &Edit,
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
        Edit::SetSample { block, sample } => {
            let parsed: simsandbox_file::Sampling =
                serde_json::from_str(sample).map_err(|e| format!("{block}'s sample: {e}"))?;
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block && b.type_name == simsandbox_file::STUDY)
                .ok_or_else(|| format!("no study '{block}'"))?;
            b.sample = Some(parsed);
            Ok(format!("{block}: sample changed"))
        }
        Edit::SetActivate { block, mode } => {
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block && b.type_name == simsandbox_file::STUDY)
                .ok_or_else(|| format!("no study '{block}'"))?;
            b.activate.clone_from(mode);
            if mode.is_none() {
                let port = format!("{block}.activate");
                l.connections.retain(|c| c.to != port);
            }
            Ok(match mode {
                Some(m) => format!("{block} runs on its activate input ({m})"),
                None => format!("{block} runs when its inputs change"),
            })
        }
        Edit::SetSetting { block, key, text } => {
            let b = l
                .blocks
                .iter_mut()
                .find(|b| b.name == *block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            if text.trim().is_empty() {
                b.settings.remove(key);
                return Ok(format!("{block}: {key} cleared"));
            }
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
        Edit::SetEquations { block, equations } => {
            let entry = l
                .blocks
                .iter_mut()
                .find(|b| &b.name == block)
                .ok_or_else(|| format!("no block '{block}'"))?;
            // Files the equations start from are read from the thread's
            // folder: the app applies edits in the model's.
            let set =
                simsandbox_eq::EquationSet::parse(block, equations).map_err(|e| e.to_string())?;
            let keep: Vec<String> = simsandbox_eq::OdeSystem::spec(&set)
                .params
                .iter()
                .map(|p| p.name.clone())
                .collect();
            entry.equations.clone_from(equations);
            entry.params.retain(|name, _| keep.contains(name));
            let method = entry.method.name.as_str();
            let compatible = match set.kind() {
                simsandbox_eq::Kind::Continuous => !matches!(method, "iterate" | "evaluate"),
                simsandbox_eq::Kind::Map => method == "iterate",
                simsandbox_eq::Kind::Formulas => method == "evaluate",
            };
            if !compatible {
                entry.method = set.kind().default_method().into();
            }
            Ok(format!("Changed the equations of {block}"))
        }
        Edit::Study { block } => {
            let u = l
                .blocks
                .iter()
                .find(|b| &b.name == block)
                .filter(|b| b.is_use())
                .cloned()
                .ok_or_else(|| format!("'{block}' is not a used model"))?;
            let (model, _, inner) =
                simsandbox_file::compose::used_model(&u, file.folder.as_deref())?;
            let (key, range) = study_parameter(&model, registry, inner.as_deref())
                .ok_or_else(|| format!("{block}'s model has no parameter to vary"))?;
            let base = format!("{block}_study");
            let name = if has(l, &base) {
                free_name(&format!("{base}_"), |n| has(l, n))
            } else {
                base
            };

            let sample: simsandbox_file::Sampling = serde_json::from_value(serde_json::json!({
                "design": "grid", "params": [{"param": key, "range": range, "points": 20}]}))
            .map_err(|e| e.to_string())?;
            // Its inputs fed as the use's input blocks are.
            let fed: Vec<ConnectionEntry> = l
                .connections
                .iter()
                .filter_map(|c| {
                    let input = c.to.strip_prefix(&format!("{block}/"))?;
                    (!input.contains(['/', '.']) && format!("{input}.y") != key).then(|| {
                        ConnectionEntry {
                            from: c.from.clone(),
                            to: format!("{name}.{input}"),
                        }
                    })
                })
                .collect();
            l.blocks.push(BlockEntry {
                name: name.clone(),
                type_name: simsandbox_file::STUDY.into(),
                // The same model, named as the use names it.
                model: u.model.clone(),
                file: u.file.clone(),
                sample: Some(sample),
                // The use's values of its model's parameters (ADR 0074).
                params: u.params.clone(),
                canvas: u.canvas.map(|[x, y]| [x + 260.0, y]),
                ..Default::default()
            });
            l.connections.extend(fed);
            Ok(format!(
                "Added {name}, a study of {block}'s model over {key}"
            ))
        }
        Edit::Duplicate { blocks } => {
            let mut renamed: BTreeMap<String, String> = BTreeMap::new();
            for name in blocks {
                let entry = l
                    .blocks
                    .iter()
                    .find(|b| &b.name == name)
                    .cloned()
                    .ok_or_else(|| format!("no block '{name}'"))?;
                // `car1` → `car2`, or the next number free.
                let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
                let stem = if stem.is_empty() { name.as_str() } else { stem };
                let new = free_name(stem, |n| has(l, n) || renamed.values().any(|r| r == n));
                let mut copy = entry;
                copy.name.clone_from(&new);
                copy.canvas = copy.canvas.map(|[x, y]| [x + 40.0, y + 60.0]);
                l.blocks.push(copy);
                renamed.insert(name.clone(), new);
            }
            // Wires among the copied, copied too; a copy's input fed from
            // outside them is fed as its original's (an input has no
            // default, ADR 0074).
            let end = |e: &str| -> Option<String> {
                let i = e.find(['.', '/'])?;
                renamed.get(&e[..i]).map(|n| format!("{n}{}", &e[i..]))
            };
            let copies: Vec<ConnectionEntry> = l
                .connections
                .iter()
                .filter_map(|c| {
                    Some(ConnectionEntry {
                        to: end(&c.to)?,
                        from: end(&c.from).unwrap_or_else(|| c.from.clone()),
                    })
                })
                .collect();
            l.connections.extend(copies);
            // A copy of a use keeps its original's clock, wired or not.
            for (orig, new) in &renamed {
                let shared = l.blocks.iter().find(|b| &b.name == orig).is_some_and(|b| {
                    b.is_use()
                        && simsandbox_file::compose::runs_shared(
                            b,
                            file.folder.as_deref(),
                            simsandbox_file::compose::wired(&l.connections, orig),
                        )
                });
                let wired = simsandbox_file::compose::wired(&l.connections, new);
                if let Some(c) = l.blocks.iter_mut().find(|b| &b.name == new) {
                    if shared && !wired && c.clock.is_none() {
                        c.clock = Some(simsandbox_file::UseClock::Shared);
                    }
                }
            }
            let names: Vec<&str> = renamed.values().map(String::as_str).collect();
            Ok(format!("Duplicated as {}", names.join(", ")))
        }
        Edit::Add { type_name, at } => {
            // A model defined in the file is added as a use of it
            // (ADR 0068).
            let entry = if file.models.contains_key(type_name) {
                let base = type_name.to_lowercase();
                let name = free_name(&base, |n| has(l, n));
                BlockEntry {
                    name,
                    model: Some(type_name.clone()),
                    canvas: Some(at.map(tidy)),
                    ..Default::default()
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
        Edit::GroupModel { .. }
        | Edit::StudyModel
        | Edit::Search { .. }
        | Edit::DeclareOutput { .. }
        | Edit::RemoveOutput { .. }
        | Edit::HideOutput { .. }
        | Edit::ExposeParam { .. }
        | Edit::HideParam { .. }
        | Edit::ExposeInput { .. }
        | Edit::HideInput { .. }
        | Edit::RenameModel { .. }
        | Edit::DeclareParam { .. }
        | Edit::RemoveParam { .. } => unreachable!("handled before"),
        // Data-layer edits are handled by `observers::apply` first.
        Edit::Bind { .. }
        | Edit::Unbind { .. }
        | Edit::AddFilter { .. }
        | Edit::AddView { .. }
        | Edit::AddProbe { .. }
        | Edit::SetProbe { .. }
        | Edit::SetFilterWhen { .. }
        | Edit::AddRecorder { .. }
        | Edit::SetRecorder { .. }
        | Edit::Promote { .. }
        | Edit::PromoteConnect { .. }
        | Edit::AddUse { .. } => unreachable!("handled before"),
    }
}

/// Add a use of the model file at `path` (ADR 0066 step 4c).
fn add_use(
    file: &mut ModelFile,
    scope: &Scope,
    path: &str,
    at: [f64; 2],
) -> Result<String, String> {
    if *scope != Scope::Model {
        return Err("model files are used at a model's level, not inside a subsystem".into());
    }
    let full = std::path::PathBuf::from(path);
    let written = match &file.folder {
        Some(f) => full.strip_prefix(f).unwrap_or(&full).to_path_buf(),
        None => full.clone(),
    };
    let stem = full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem.split('.').next().unwrap_or_default();
    let mut base: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
        base = format!("model_{base}");
    }
    let mut name = base.clone();
    let mut k = 2;
    while file.blocks.iter().any(|b| b.name == name) {
        name = format!("{base}_{k}");
        k += 1;
    }
    file.blocks.push(simsandbox_file::BlockEntry {
        name: name.clone(),
        file: Some(written.to_string_lossy().into_owned()),
        canvas: Some(at),
        ..Default::default()
    });
    Ok(format!("Added {name}, using {}", written.display()))
}

/// A wire end on a used model's node (`plant.car.v`, `controller.speed`)
/// as the file writes it (`plant/car.v`, `controller/speed`); `None` for
/// any other end.
fn use_end(file: &ModelFile, key: &str) -> Option<String> {
    let (node, rest) = key.split_once('.')?;
    file.blocks
        .iter()
        .any(|b| b.name == node && b.is_use())
        .then(|| format!("{node}/{rest}"))
}

/// The clock an end at the model's level runs on: a use's own, or the
/// model's (`None`) for its blocks and uses on a shared clock.
fn clock_of<'a>(file: &'a ModelFile, end: &str) -> Option<&'a str> {
    let (first, _) = end.split_once('/')?;
    file.blocks
        .iter()
        .find(|b| b.name == first && b.is_use())
        // As if wired: a model with no step of its own shares its user's
        // clock once a wire reaches it (ADR 0068).
        .filter(|b| !simsandbox_file::compose::runs_shared(b, file.folder.as_deref(), true))
        .map(|b| b.name.as_str())
}

/// Connect two ends of which one or both are in used models: a wire on
/// one clock, or across clocks a probe recording the output, linked into
/// the input block (ADR 0066 step 4c).
fn connect_uses(
    file: &mut ModelFile,
    scope: &Scope,
    registry: &Registry,
    from: &str,
    to: &str,
) -> Result<String, String> {
    let src = use_end(file, from).unwrap_or_else(|| from.to_string());
    let dst = use_end(file, to).unwrap_or_else(|| to.to_string());
    if clock_of(file, &src) == clock_of(file, &dst) {
        // One clock: a wire, as between blocks.
        file.connections.retain(|c| c.to != dst);
        file.connections.push(ConnectionEntry {
            from: src.clone(),
            to: dst.clone(),
        });
        let _ = (scope, registry);
        return Ok(format!("Connected {src} -> {dst}"));
    }
    // Across clocks: into an input block only.
    let input = dst
        .split_once('/')
        .map(|(u, b)| (u, b.trim_end_matches(".u")));
    let into_input_block = match input {
        Some(_) => !dst.contains('.') || dst.ends_with(".u"),
        None => file.blocks.iter().any(|b| {
            Some(b.name.as_str()) == dst.split_once('.').map(|(n, _)| n)
                && b.type_name == simsandbox_eq::input::VALUE
        }),
    };
    if !into_input_block {
        return Err(format!(
            "{src} runs on another clock than {dst}: it reaches it through an input block \
             (an input.value) that takes its samples"
        ));
    }
    let dst = match dst.split_once('.') {
        // An input block of the model's own is fed by name.
        Some((b, _)) if !dst.contains('/') => b.to_string(),
        _ => dst.trim_end_matches(".u").to_string(),
    };
    // A probe already recording it, or a new one.
    let probe = match file
        .controls
        .probes
        .iter()
        .find(|p| p.record.iter().any(|r| r == &src))
    {
        Some(p) => p.name.clone(),
        None => {
            let base = format!("link_{}", src.replace(['/', '.'], "_"));
            let mut name = base.clone();
            let mut k = 2;
            while file.controls.probes.iter().any(|p| p.name == name) {
                name = format!("{base}_{k}");
                k += 1;
            }
            file.controls.probes.push(simsandbox_file::ProbeEntry {
                name: name.clone(),
                record: vec![src.clone()],
                every: None,
                keep: None,
                to: None,
                stream: false,
                canvas: None,
            });
            name
        }
    };
    file.connections.retain(|c| c.to != dst);
    file.connections.push(ConnectionEntry {
        from: format!("probe:{probe}.{src}"),
        to: dst.clone(),
    });
    Ok(format!(
        "Linked {src} -> {dst} through the probe {probe}: they run on different clocks, so {dst} \
         takes its samples, held between them"
    ))
}

/// Remove the wire or link into an input of a used model; a probe made
/// for a link and feeding nothing else goes too.
fn disconnect_use(file: &mut ModelFile, to: &str) -> Result<String, String> {
    let bare = to.trim_end_matches(".u");
    let before = file.connections.len();
    file.connections.retain(|c| c.to != to && c.to != bare);
    if file.connections.len() == before {
        return Err(format!("nothing is connected to '{to}'"));
    }
    let used: Vec<String> = file
        .connections
        .iter()
        .filter_map(|c| {
            c.from
                .strip_prefix("probe:")
                .and_then(|p| p.split_once('.'))
                .map(|(p, _)| p.to_string())
        })
        .collect();
    file.controls
        .probes
        .retain(|p| !p.name.starts_with("link_") || used.contains(&p.name));
    Ok(format!("Disconnected {bare}"))
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

/// `base` followed by the first number that makes a name not `taken`
/// (`car1`, `car2`, …).
pub fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|k| format!("{base}{k}"))
        .find(|n| !taken(n))
        .expect("some name is free")
}

/// Drop a deleted model-level block from controls and groups.
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

/// Move `names` (blocks of one level) into a new subsystem and use it in
/// their place. Wires crossing the boundary become its ports; declared
/// parameters the moved formulas use are passed in as its parameters.
/// Offer `from` (`block.port`) as the declared output `name`.
/// With no name, the port's own, made unique (ADR 0074).
fn declare_output(file: &mut ModelFile, name: &str, from: &str) -> Result<String, String> {
    let chosen;
    let name = match name.trim() {
        "" => {
            let port = from.rsplit(['.', '/']).next().unwrap_or(from);
            let taken = |n: &str| {
                file.outputs.iter().any(|o| o.name == n)
                    || file.inputs.iter().any(|i| i.name == n)
                    || file.blocks.iter().any(|b| b.name == n)
            };
            chosen = unique_name(port, taken);
            chosen.as_str()
        }
        n => n,
    };
    if !simsandbox_file::subsystems::valid_name(name) {
        return Err(format!(
            "'{name}': an output's name is letters, digits and '_'"
        ));
    }
    if file.outputs.iter().any(|o| o.name == name)
        || file
            .blocks
            .iter()
            .any(|b| b.name == name && b.type_name == "input.value")
    {
        return Err(format!(
            "'{name}' is already an output or an input block's name"
        ));
    }
    file.outputs.push(OutputPort {
        name: name.to_string(),
        from: from.to_string(),
        canvas: None,
    });
    Ok(format!("Offers {from} as the output {name}"))
}

/// A name based on `base` that `taken` does not hold: `base`, or
/// `base` followed by a number.
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if taken(base) {
        free_name(base, taken)
    } else {
        base.to_string()
    }
}

/// Expose `block.param` as a parameter of the model (ADR 0074).
fn expose_param(
    file: &mut ModelFile,
    registry: &Registry,
    block: &str,
    param: &str,
) -> Result<String, String> {
    let folder = file.folder.clone();
    let b = file
        .blocks
        .iter()
        .find(|b| b.name == block)
        .ok_or_else(|| format!("no block named '{block}'"))?;
    if let Some(ParamValue::Formula(f)) = b.params.get(param) {
        return Err(format!("{block}.{param} is set by '{f}' already"));
    }
    let built = build(b, registry, folder.as_deref())?;
    let spec = built
        .spec()
        .params
        .iter()
        .find(|p| p.name == param)
        .ok_or_else(|| format!("{block} has no parameter '{param}'"))?
        .clone();
    let value = match b.params.get(param) {
        Some(ParamValue::Number(v)) => *v,
        _ => spec.default,
    };
    let taken = |n: &str| {
        file.params.contains_key(n)
            || file.blocks.iter().any(|b| b.name == n)
            || simsandbox_expr::is_reserved(n)
    };
    let name = unique_name(param, taken);
    let unit = if spec.unit == simsandbox_core::units::Unit::DIMENSIONLESS {
        String::new()
    } else {
        spec.unit.to_string()
    };
    file.params.insert(
        name.clone(),
        simsandbox_file::ParamDecl {
            value,
            unit,
            description: spec.description.clone(),
            range: (spec.suggested_min < spec.suggested_max)
                .then_some([spec.suggested_min, spec.suggested_max]),
        },
    );
    let b = file
        .blocks
        .iter_mut()
        .find(|b| b.name == block)
        .expect("found above");
    b.params
        .insert(param.to_string(), ParamValue::Formula(name.clone()));
    Ok(format!(
        "{block}.{param} is the model's parameter {name}: each use or study sets its own"
    ))
}

/// Stop exposing `block.param`: it keeps the model's value.
fn hide_param(file: &mut ModelFile, block: &str, param: &str) -> Result<String, String> {
    let b = file
        .blocks
        .iter()
        .find(|b| b.name == block)
        .ok_or_else(|| format!("no block named '{block}'"))?;
    let Some(ParamValue::Formula(name)) = b.params.get(param).cloned() else {
        return Err(format!("{block}.{param} is not the model's parameter"));
    };
    let decl = file.params.get(&name).cloned().ok_or_else(|| {
        format!("{block}.{param} is set by '{name}', not a parameter of the model")
    })?;
    let b = file
        .blocks
        .iter_mut()
        .find(|b| b.name == block)
        .expect("found above");
    b.params
        .insert(param.to_string(), ParamValue::Number(decl.value));
    // The declaration goes when nothing else reads it.
    let read = file.blocks.iter().any(|b| {
        b.params
            .values()
            .any(|v| matches!(v, ParamValue::Formula(f) if f.contains(name.as_str())))
    });
    if !read {
        file.params.remove(&name);
    }
    Ok(format!("{block}.{param} is the block's own again"))
}

/// Expose the block input `input` (`block.port`) as the model's (ADR 0074).
fn expose_input(file: &mut ModelFile, registry: &Registry, input: &str) -> Result<String, String> {
    let (block, port) = input
        .split_once('.')
        .ok_or_else(|| format!("'{input}': a block's input, block.port"))?;
    if file.connections.iter().any(|c| c.to == input) {
        return Err(format!(
            "{input} is wired inside the model: an exposed input is fed by its users"
        ));
    }
    if file.inputs.iter().any(|i| i.to.iter().any(|t| t == input)) {
        return Err(format!("{input} is an input of the model already"));
    }
    let folder = file.folder.clone();
    let b = file
        .blocks
        .iter()
        .find(|b| b.name == block)
        .ok_or_else(|| format!("no block named '{block}'"))?;
    let built = build(b, registry, folder.as_deref())?;
    let p = built
        .spec()
        .inputs()
        .find(|p| p.name == port)
        .ok_or_else(|| format!("{block} has no input '{port}'"))?;
    let unit = if p.unit == simsandbox_core::units::Unit::DIMENSIONLESS {
        String::new()
    } else {
        p.unit.to_string()
    };
    let taken = |n: &str| {
        file.inputs.iter().any(|i| i.name == n)
            || file.outputs.iter().any(|o| o.name == n)
            || file.blocks.iter().any(|b| b.name == n)
    };
    let name = unique_name(port, taken);
    file.inputs.push(simsandbox_file::InputPort {
        name: name.clone(),
        to: vec![input.to_string()],
        unit,
        canvas: None,
    });
    Ok(format!(
        "{input} is the model's input {name}: its users wire it"
    ))
}

/// Rename the model `from` defined in `file` to `to`; its uses in the
/// file and in its definitions (where the name is not defined again)
/// follow.
fn rename_model(file: &mut ModelFile, from: &str, to: &str) -> Result<String, String> {
    let to = to.trim();
    if !simsandbox_file::subsystems::valid_name(to) {
        return Err(format!("'{to}': a model's name is letters, digits and '_'"));
    }
    if file.models.contains_key(to) {
        return Err(format!("a model named '{to}' is already defined"));
    }
    let def = file
        .models
        .remove(from)
        .ok_or_else(|| format!("no model named '{from}' is defined here"))?;
    file.models.insert(to.to_string(), def);
    fn follow(m: &mut ModelFile, from: &str, to: &str) {
        for b in &mut m.blocks {
            if b.model.as_deref() == Some(from) {
                b.model = Some(to.to_string());
            }
        }
        for (name, v) in m.models.iter_mut() {
            if let Ok(mut d) = simsandbox_file::compose::read_definition(name, v) {
                if !d.models.contains_key(from) {
                    follow(&mut d, from, to);
                    *v = simsandbox_file::compose::as_definition(&d);
                }
            }
        }
    }
    follow(file, from, to);
    file.resolve_models().map_err(|e| e.to_string())?;
    Ok(format!("Renamed the model {from} to {to}"))
}

/// "Make model" (ADR 0068): the selection grouped as a subsystem would
/// be, then written as a model defined in the file, used in its place on
/// the same clock.
/// "Study this model" on the whole model (ADR 0071): "Make model" of
/// every block, then "Study this model" on the use made.
fn study_model(file: &mut ModelFile, scope: &Scope, registry: &Registry) -> Result<String, String> {
    let names: Vec<String> = file.blocks.iter().map(|b| b.name.clone()).collect();
    if names.is_empty() {
        return Err("the model has no blocks to study".into());
    }
    let before = file.clone();
    let made = group_model(file, scope, &names)?;
    let used = file
        .blocks
        .iter()
        .find(|b| !names.contains(&b.name))
        .map(|b| b.name.clone());
    let studied = used
        .ok_or_else(|| "the model was not made".to_string())
        .and_then(|block| apply_in_level(file, scope, registry, &Edit::Study { block }));
    match studied {
        Ok(note) => Ok(format!("{made}; {note}")),
        Err(e) => {
            *file = before;
            Err(e)
        }
    }
}

fn group_model(file: &mut ModelFile, scope: &Scope, names: &[String]) -> Result<String, String> {
    if *scope != Scope::Model {
        return Err("a model is made at a model's level, not inside a subsystem".into());
    }
    let sub = free_name("group", |n| file.subsystems.contains_key(n));
    let before = file.clone();
    group(file, scope, names)?;
    simsandbox_file::compose::convert_some_subsystems(file, std::slice::from_ref(&sub));
    let key = free_name("model", |n| file.models.contains_key(n));
    let def = file.models.remove(&sub).ok_or("the group was not made")?;
    file.models.insert(key.clone(), def);
    let Some(k) = file
        .blocks
        .iter()
        .position(|b| b.model.as_deref() == Some(sub.as_str()))
    else {
        *file = before;
        return Err("the group was not made".into());
    };
    file.blocks[k].model = Some(key.clone());
    let instance = file.blocks[k].name.clone();
    // On its user's clock, as the blocks were, wired or not.
    if !simsandbox_file::compose::wired(&file.connections, &instance) {
        file.blocks[k].clock = Some(simsandbox_file::UseClock::Shared);
    }
    file.resolve_models().map_err(|e| e.to_string())?;
    Ok(format!(
        "Made the model {key} of {} blocks (used as {instance})",
        names.len()
    ))
}

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
    };
    let result = (|| {
        for n in names {
            if !l.blocks.iter().any(|b| &b.name == n) {
                return Err(format!("no block '{n}' here"));
            }
        }
        // `block.port`, or a path into a use (`car1/lead`, ADR 0068).
        let node_of = |key: &str| key.split(['.', '/']).next().unwrap_or(key).to_string();
        let port_of = |key: &str| {
            key.find(['.', '/'])
                .map_or(key.to_string(), |i| key[i + 1..].to_string())
        };
        let inside = |key: &str| names.iter().any(|n| *n == node_of(key));
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
                            let base = port_of(&c.to).replace(['/', '.'], "_");
                            let name =
                                unique(&|n: &str| def.inputs.iter().any(|q| q.name == n), &base);
                            def.inputs.push(InputPort {
                                unit: String::new(),
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
                            let base = port_of(&c.from).replace(['/', '.'], "_");
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
                    unit: String::new(),
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
    #[test]
    fn wires_into_values_drive_them_and_show_where_they_end() {
        let r = registry();
        let mut f = file("Forced spring");
        let d = Diagram::new(&f, &r);
        let shaken = d.node("shaken").unwrap();
        // The wire ends at the driven value, listed first and shown on the
        // closed node (ADR 0075).
        let w = d.wires.iter().find(|w| w.param).unwrap();
        assert_eq!(w.to.0, shaken);
        let v = &d.nodes[shaken].values[w.to.1];
        assert_eq!(
            (v.name.as_str(), v.driven.as_deref()),
            ("F", Some("push.F"))
        );
        assert_eq!(w.to.1, 0);
        assert_eq!(d.nodes[shaken].value_rows(), 1);
        assert!(d.dangling.is_empty(), "{:?}", d.dangling);
        // Wiring a value keeps its own, which applies again once the wire
        // is lifted (ADR 0075).
        f.blocks
            .iter_mut()
            .find(|b| b.name == "held")
            .unwrap()
            .params
            .insert("F".into(), ParamValue::Number(0.5));
        let note = apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "push.F".into(),
                to: "held.F".into(),
            },
        )
        .unwrap();
        assert!(note.contains("Connected"), "{note}");
        let held = f.blocks.iter().find(|b| b.name == "held").unwrap();
        assert_eq!(held.params["F"], ParamValue::Number(0.5));
        let d = Diagram::new(&f, &r);
        let h = d.node("held").unwrap();
        assert_eq!(d.nodes[h].values[0].driven.as_deref(), Some("push.F"));
        assert!(crate::describe::check(&f.to_json(), &r)["ok"] == true);
        // Lifted, it has its own value again, kept all along.
        apply(
            &mut f,
            &r,
            &Edit::Disconnect {
                to: "held.F".into(),
            },
        )
        .unwrap();
        let d = Diagram::new(&f, &r);
        let h = d.node("held").unwrap();
        assert!(d.nodes[h].values.iter().all(|v| v.driven.is_none()));
        // Expanded, the wire ends at the inner value set to F.
        let f = file("Forced spring");
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["shaken".into()]);
        let mass = d.node("shaken/mass").unwrap();
        let w = d.wires.iter().find(|w| w.param).unwrap();
        assert_eq!(w.to.0, mass);
        assert_eq!(d.nodes[mass].values[w.to.1].name, "F");
        assert_eq!(
            d.nodes[mass].values[w.to.1].driven.as_deref(),
            Some("push.F")
        );
    }

    #[test]
    fn ports_and_parameters_are_exposed_and_hidden_inside_a_model() {
        let r = registry();
        let platoon = file("Platoon");
        let mut f =
            simsandbox_file::compose::read_definition("follower", &platoon.models["follower"])
                .unwrap();
        let mut d = Diagram::new(&f, &r);
        d.inside = true;
        // What the follower exposes, by node (ADR 0074).
        assert_eq!(d.exposed["in:control.lead"], "lead");
        assert_eq!(d.exposed["out:car.v"], "v");
        assert_eq!(d.exposed["par:car.m"], "mass");
        let car = d.node("car").unwrap();
        let control = d.node("control").unwrap();
        assert!(d.exposable(car));
        // An exposed parameter is listed with the model's value.
        let m = d.nodes[car].values.iter().find(|v| v.name == "m").unwrap();
        assert_eq!((m.exposed.as_deref(), m.value), (Some("mass"), 1000.0));
        let toggle = |f: &mut ModelFile, node: usize, kind: &str, item: &str| {
            let mut d = Diagram::new(f, &r);
            d.inside = true;
            let edit = d.toggle_exposed(node, kind, item);
            apply(f, &r, &edit)
        };
        // A parameter: declared with its value and unit, then the block's
        // own again.
        toggle(&mut f, car, "par", "c").unwrap();
        assert_eq!(f.params["c"].value, 50.0);
        assert_eq!(f.params["c"].unit, "N*s/m");
        let d = Diagram::new(&f, &r);
        let c = d.nodes[car].values.iter().find(|v| v.name == "c").unwrap();
        assert_eq!(c.exposed.as_deref(), Some("c"));
        toggle(&mut f, car, "par", "c").unwrap();
        assert!(!f.params.contains_key("c"));
        let b = f.blocks.iter().find(|b| b.name == "car").unwrap();
        assert_eq!(b.params["c"], ParamValue::Number(50.0));
        // An output: offered under its port's name, then not.
        toggle(&mut f, control, "out", "F").unwrap();
        assert!(f
            .outputs
            .iter()
            .any(|o| o.name == "F" && o.from == "control.F"));
        toggle(&mut f, control, "out", "F").unwrap();
        assert!(!f.outputs.iter().any(|o| o.from == "control.F"));
        // An input: hidden, then exposed again; one wired inside is not.
        toggle(&mut f, control, "in", "lead").unwrap();
        assert!(f.inputs.is_empty());
        toggle(&mut f, control, "in", "lead").unwrap();
        assert_eq!(f.inputs[0].name, "lead");
        assert_eq!(f.inputs[0].unit, "m/s");
        let refused = toggle(&mut f, car, "in", "F").unwrap_err();
        assert!(refused.contains("wired inside"), "{refused}");
        // Outside, at the top level, nothing is exposable; expanded in
        // place, the follower's nodes are, with their exposure.
        let mut top = Diagram::new(&platoon, &r);
        assert!(!top.exposable(top.node("leader").unwrap()));
        top.expand(&platoon, &r, &["car1".into()]);
        let inner = top.node("car1/control").unwrap();
        assert!(top.exposable(inner));
        assert_eq!(top.exposed_as(inner, "in", "lead"), Some("lead"));
        assert_eq!(
            crate::expand::route(&top, &top.toggle_exposed(inner, "in", "lead"))
                .unwrap()
                .1,
            Edit::HideInput {
                input: "control.lead".into()
            }
        );
    }

    fn final_value(f: &ModelFile, signal: &str, t: f64) -> f64 {
        let text = f.to_json();
        if simsandbox_file::compose::is_composition(&text) {
            let r = crate::compose::run_composition(&text, &registry(), Some(t)).unwrap();
            return *r.values(signal).unwrap().last().unwrap();
        }
        let r = crate::run::run(&text, &registry(), Some(t), None).unwrap();
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
    fn a_used_models_port_list_holds_only_what_no_declared_output_shows() {
        // ADR 0073: the follower offers `v` (its car's `v`): the node shows
        // `v`, and `car.v` is not listed again.
        let d = Diagram::of(&file("Platoon"), &Scope::Model, &registry());
        let n = &d.nodes[d.node("car1").unwrap()];
        assert!(n.outputs.iter().any(|p| p.name == "v"), "{:?}", n.outputs);
        let listed: Vec<&str> = n.reachable.iter().map(|(_, p)| p.name.as_str()).collect();
        assert!(!listed.contains(&"car.v"), "{listed:?}");
        assert!(listed.contains(&"control.F"), "{listed:?}");
        // Only the inside of a used model offers outputs from its blocks.
        assert!(!d.inside);
    }

    #[test]
    fn the_whole_model_is_studied_through_a_definition() {
        // ADR 0071: "Study this model" on the canvas background.
        let r = registry();
        let mut f = file("Cruise control");
        let note = apply(&mut f, &r, &Edit::StudyModel).unwrap();
        assert!(note.contains("Made the model"), "{note}");
        let names: Vec<&str> = f.blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let study = f.blocks.iter().find(|b| b.type_name == "study").unwrap();
        let def = &f.models[study.model.as_deref().unwrap()];
        assert_eq!(def["blocks"].as_array().unwrap().len(), 2);
        // It runs: the model as before, beside its study's 20 runs.
        let rows = crate::run::run(&f.to_json(), &r, Some(0.2), None).unwrap();
        let studied = rows
            .outputs
            .iter()
            .find(|o| o.name.starts_with(&format!("{}.", study.name)))
            .unwrap();
        assert_eq!(studied.shape.len(), 20);
        // Nothing to study in an empty model.
        let mut empty = ModelFile::new();
        assert!(apply(&mut empty, &r, &Edit::StudyModel).is_err());
    }

    #[test]
    fn a_study_of_a_used_model_is_added_set_and_follows_renames() {
        // ADR 0070 step 5: "Study this model" on a follower of the Platoon.
        let r = registry();
        let mut f = file("Platoon");
        let note = apply(
            &mut f,
            &r,
            &Edit::Study {
                block: "car1".into(),
            },
        )
        .unwrap();
        assert!(note.contains("car1_study") && note.contains("ki"), "{note}");
        let s = f
            .blocks
            .iter()
            .find(|b| b.name == "car1_study")
            .unwrap()
            .clone();
        assert_eq!(s.model.as_deref(), Some("follower"));
        let sample = serde_json::to_value(s.sample.as_ref().unwrap()).unwrap();
        assert_eq!(sample["params"][0]["param"], "ki");
        assert_eq!(
            sample["params"][0]["range"],
            serde_json::json!([20.0, 80.0])
        );
        assert_eq!(sample["params"][0]["points"], 20);
        // Its input fed as car1's: from the leader.
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "leader.v" && c.to == "car1_study.lead"));
        // The node says what it studies; it runs, 20 runs.
        let d = Diagram::of(&f, &Scope::Model, &r);
        let n = d.nodes.iter().find(|n| n.name == "car1_study").unwrap();
        assert!(n.error.is_none(), "{:?}", n.error);
        assert_eq!(n.method, "of follower · 20 runs");
        let run = |f: &ModelFile| crate::run::run(&f.to_json(), &r, Some(0.2), None).unwrap();
        let ki = run(&f)
            .outputs
            .into_iter()
            .find(|o| o.name == "car1_study.ki")
            .unwrap();
        assert_eq!(ki.shape.len(), 20);
        assert_eq!(ki.values[..2], [20.0, 20.0 + 60.0 / 19.0]);
        // Its sample set from JSON; bad JSON refused with why.
        apply(
            &mut f,
            &r,
            &Edit::SetSample {
                block: "car1_study".into(),
                sample: r#"{"design": "list", "params": [{"param": "mass"}], "values": [[800], [1600], [3200]],
                           "reduce": {"v_max": "max(car.v)"}}"#
                    .into(),
            },
        )
        .unwrap();
        let v = run(&f)
            .outputs
            .into_iter()
            .find(|o| o.name == "car1_study.v_max")
            .unwrap();
        assert_eq!(v.shape.len(), 3);
        let e = apply(
            &mut f,
            &r,
            &Edit::SetSample {
                block: "car1_study".into(),
                sample: "{design: grid".into(),
            },
        )
        .unwrap_err();
        assert!(e.contains("car1_study's sample"), "{e}");
        // Activated (ADR 0088): an 'activate' input on its node, wired from
        // a switch (high above 0.5; a speed is refused, being in m/s); off
        // again, the input and the wire go.
        f.blocks.push(
            serde_json::from_value(serde_json::json!(
                {"name": "go", "type": "input.value", "params": {"y": 1.0}}))
            .unwrap(),
        );
        let set = |f: &mut ModelFile, mode: Option<&str>| {
            apply(
                f,
                &r,
                &Edit::SetActivate {
                    block: "car1_study".into(),
                    mode: mode.map(String::from),
                },
            )
            .unwrap()
        };
        set(&mut f, Some("while"));
        apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "go.y".into(),
                to: "car1_study.activate".into(),
            },
        )
        .unwrap();
        let d = Diagram::of(&f, &Scope::Model, &r);
        let n = d.nodes.iter().find(|n| n.name == "car1_study").unwrap();
        assert!(n.inputs.iter().any(|p| p.name == "activate"));
        run(&f);
        set(&mut f, None);
        assert!(f.connections.iter().all(|c| c.to != "car1_study.activate"));
        assert!(f.blocks.iter().all(|b| b.activate.is_none()));
        run(&f);
        // Renaming the model renames what the study studies.
        apply(
            &mut f,
            &r,
            &Edit::RenameModel {
                from: "follower".into(),
                to: "chaser".into(),
            },
        )
        .unwrap();
        let s = f.blocks.iter().find(|b| b.name == "car1_study").unwrap();
        assert_eq!(s.model.as_deref(), Some("chaser"));
        // Still running: mass and v_max, three runs each.
        let outs: Vec<usize> = run(&f)
            .outputs
            .iter()
            .filter(|o| o.name.starts_with("car1_study."))
            .map(|o| o.shape.len())
            .collect();
        assert_eq!(outs, [3, 3]);
        // Only a used model is studied.
        assert!(apply(
            &mut f,
            &r,
            &Edit::Study {
                block: "leader".into()
            }
        )
        .is_err());
    }

    #[test]
    fn duplicated_blocks_and_uses_keep_their_values_and_wires_among_them() {
        let r = registry();
        let mut f = file("Platoon");
        let note = apply(
            &mut f,
            &r,
            &Edit::Duplicate {
                blocks: vec!["car3".into()],
            },
        )
        .unwrap();
        assert!(note.contains("car4"), "{note}");
        let car4 = f.blocks.iter().find(|b| b.name == "car4").unwrap();
        assert_eq!(car4.model.as_deref(), Some("follower"));
        assert_eq!(car4.params["mass"], ParamValue::Number(2500.0));
        // Its input is fed as car3's (ADR 0074); it runs on car3's clock.
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "car2/v" && c.to == "car4/lead"));
        let car3 = f.blocks.iter().find(|b| b.name == "car3").unwrap();
        assert_eq!(car4.clock, car3.clock);
        assert!(final_value(&f, "car4/car.v", 1.0).is_finite());
        // Two at once: the wire between them is copied, the ones from them
        // to the rest are not; car4's input is fed as car1's.
        let mut f = file("Platoon");
        apply(
            &mut f,
            &r,
            &Edit::Duplicate {
                blocks: vec!["car1".into(), "car2".into()],
            },
        )
        .unwrap();
        let names: Vec<&str> = f.blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(&names[4..], ["car4", "car5"]);
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "car4/v" && c.to == "car5/lead"));
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "leader.v" && c.to == "car4/lead"));
        assert!(!f.connections.iter().any(|c| c.from == "car5/v"));
        // A block with equations.
        let mut f = file("Cruise");
        apply(
            &mut f,
            &r,
            &Edit::Duplicate {
                blocks: vec!["car".into()],
            },
        )
        .unwrap();
        let copy = f.blocks.iter().find(|b| b.name == "car1").unwrap();
        assert_eq!(copy.equations, f.blocks[0].equations);
    }

    #[test]
    fn open_nodes_follow_renames_and_deletions() {
        let r = registry();
        let mut f = file("Platoon");
        f.controls.open = vec!["car1".into(), "car2".into()];
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
        assert_eq!(f.controls.open, ["first"]);
    }

    #[test]
    fn grouping_blocks_keeps_the_model_and_its_numbers() {
        let r = registry();
        let before = file("Cruise");
        let v = final_value(&before, "car.v", 5.0);
        // The controller alone: crossing wires become an input block and a
        // declared output of the model made (ADR 0068).
        let mut f = before.clone();
        let note = apply(
            &mut f,
            &r,
            &Edit::GroupModel {
                blocks: vec!["cruise".into()],
            },
        )
        .unwrap();
        assert!(note.contains("model1"), "{note}");
        let def = simsandbox_file::compose::read_definition("model1", &f.models["model1"]).unwrap();
        assert!(def
            .blocks
            .iter()
            .any(|b| b.name == "v" && b.type_name == "input.value"));
        assert_eq!(def.outputs[0].name, "F");
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "car.v" && c.to == "group1/v"));
        assert!(f.controls.params.contains_key("group1/cruise.kp"));
        assert_eq!(final_value(&f, "car.v", 5.0), v);
        // Both blocks: no wires left, the same numbers.
        let mut f = before.clone();
        apply(
            &mut f,
            &r,
            &Edit::GroupModel {
                blocks: vec!["car".into(), "cruise".into()],
            },
        )
        .unwrap();
        assert!(f.connections.is_empty());
        assert_eq!(final_value(&f, "group1/car.v", 5.0), v);
        // Uses whose values are formulas of the model's parameters: they
        // come in as the new model's parameters.
        let mut p = file("Platoon");
        let v3 = final_value(&p, "car3/car.v", 5.0);
        apply(
            &mut p,
            &r,
            &Edit::GroupModel {
                blocks: vec!["car1".into(), "car2".into()],
            },
        )
        .unwrap();
        let def = simsandbox_file::compose::read_definition("model1", &p.models["model1"]).unwrap();
        assert!(def.params.contains_key("kp") && def.params.contains_key("ki"));
        assert_eq!(
            p.blocks.last().unwrap().params["kp"],
            ParamValue::Formula("kp".into())
        );
        assert_eq!(final_value(&p, "car3/car.v", 5.0), v3);
        // And a model can be renamed: its uses follow, inside others too.
        apply(
            &mut p,
            &r,
            &Edit::RenameModel {
                from: "follower".into(),
                to: "chaser".into(),
            },
        )
        .unwrap();
        assert!(p
            .blocks
            .iter()
            .any(|b| b.model.as_deref() == Some("chaser")));
        assert!(p.models["model1"].to_string().contains("chaser"));
        assert_eq!(final_value(&p, "car3/car.v", 5.0), v3);
        // Outputs offered and withdrawn.
        apply(
            &mut p,
            &r,
            &Edit::DeclareOutput {
                name: "speed".into(),
                from: "leader.v".into(),
            },
        )
        .unwrap();
        assert_eq!(p.outputs[0].from, "leader.v");
        assert!(apply(
            &mut p,
            &r,
            &Edit::DeclareOutput {
                name: "speed".into(),
                from: "leader.v".into()
            }
        )
        .is_err());
        apply(
            &mut p,
            &r,
            &Edit::RemoveOutput {
                name: "speed".into(),
            },
        )
        .unwrap();
        assert!(p.outputs.is_empty());
    }

    #[test]
    fn every_example_has_a_diagram_without_dangling_wires() {
        let r = registry();
        for e in EXAMPLES {
            let (f, _) = ModelFile::from_json(e.text).unwrap();
            let d = simsandbox_core::registry::in_folder(e.folder().as_deref(), || {
                Diagram::new(&f, &r)
            });
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
