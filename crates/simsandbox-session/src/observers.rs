//! The canvas's data layer (ADR 0032, ADR 0050): filters, views and
//! probes drawn as nodes beside the model's blocks, wired to the recorded
//! signals they read. Filters are `filter:NAME` nodes, views `view:K`
//! (their place in `controls.views`), probes `probe:NAME` (ADR 0062), each
//! with one input, `record`, taking any number of signals, and an output
//! per signal it records (ADR 0063): a view wired from one draws the
//! probe's recording (`probe:NAME/signal`), wired from the model, every
//! step. Data wires carry recorded data, not current values,
//! so a filter never feeds a model block: wiring one into a block's input
//! is refused, and promoting the filter turns it into a formula block that
//! can. Edits to filters and views change only `controls`; the app
//! applies them without restarting the run.

use crate::diagram::{
    apply_in, free_name, tidy, Diagram, Edit, Node, NodeKind, Port, Scope, Wire, NODE_WIDTH,
};
use simsandbox_core::block::Shape;
use simsandbox_core::registry::Registry;
use simsandbox_file::{
    Binding, BlockEntry, ConnectionEntry, FilterEntry, ModelFile, ProbeEntry, ViewEntry,
};
use simsandbox_views::ViewRegistry;
use std::collections::BTreeMap;

/// Space between data-layer nodes placed automatically.
const GAP_X: f64 = 90.0;
const GAP_Y: f64 = 30.0;

pub fn filter_node(name: &str) -> String {
    format!("filter:{name}")
}

pub fn view_node(index: usize) -> String {
    format!("view:{index}")
}

pub fn probe_node(name: &str) -> String {
    format!("probe:{name}")
}

/// A probe node's one input.
pub const RECORD: &str = "record";

/// The recorder's node (ADR 0064): one per model.
pub const RECORDER: &str = "recorder";

/// Whether a node name is on the data layer.
pub fn is_observer(node: &str) -> bool {
    node.starts_with("filter:")
        || node.starts_with("view:")
        || node.starts_with("probe:")
        || node == RECORDER
}

/// Whether probe `name` feeds a block of the model (ADR 0063).
pub fn probe_feeds_model(file: &ModelFile, name: &str) -> bool {
    let prefix = format!("probe:{name}.");
    file.connections.iter().any(|c| c.from.starts_with(&prefix))
}

/// The file with only the part of its data layer that belongs to the
/// model: the probes feeding blocks, which "Hide data" still shows.
pub fn model_part(file: &ModelFile) -> ModelFile {
    let mut f = file.clone();
    f.controls.filters.clear();
    f.controls.views.clear();
    f.controls.recorder = None;
    f.controls
        .probes
        .retain(|p| probe_feeds_model(file, &p.name));
    f
}

/// The recorder's settings in a few words.
pub fn recorder_summary(r: &simsandbox_file::RecorderEntry) -> String {
    format!("every {} steps · {} MB", r.every(), r.mb())
}

/// A probe's settings in a few words: when, how long, its file.
pub fn probe_summary(p: &ProbeEntry) -> String {
    match &p.to {
        Some(to) => format!("{} · {to}", probe_timing(p)),
        None => probe_timing(p),
    }
}

/// When a probe records and how long it keeps samples.
fn probe_timing(p: &ProbeEntry) -> String {
    let every = match &p.every {
        Some(e) if e.steps.is_some_and(|n| n > 1) => {
            format!("every {} steps", e.steps.unwrap_or(1))
        }
        Some(e) if e.seconds.is_some() => format!("every {} s", e.seconds.unwrap_or_default()),
        Some(e) if e.times.is_some() => {
            let n = e.times.as_ref().map_or(0, Vec::len);
            format!("at {n} time{}", if n == 1 { "" } else { "s" })
        }
        _ => "every step".into(),
    };
    let keep = match &p.keep {
        Some(k) if k.all => "all kept".to_string(),
        Some(k) if k.last.is_some() => format!("last {}", k.last.unwrap_or_default()),
        Some(k) if k.mb.is_some() => format!("thinned past {} MB", k.mb.unwrap_or_default()),
        Some(k) if k.samples.is_some() => format!("thinned past {}", k.samples.unwrap_or_default()),
        _ => "thinned".into(),
    };
    format!("{every} · {keep}")
}

/// The node of a `"node.port"` key.
fn node_of(key: &str) -> &str {
    key.split_once('.').map_or(key, |(n, _)| n)
}

/// The flat signal an output at a level carries: `car.v` stays, a
/// subsystem use's `car1.v` becomes the inner `car1/car.v`.
fn flat_output(
    file: &ModelFile,
    blocks: &[BlockEntry],
    prefix: &str,
    key: &str,
    depth: usize,
) -> Option<String> {
    let (node, port) = key.split_once('.')?;
    let b = blocks.iter().find(|b| b.name == node)?;
    match file.subsystems.get(&b.type_name) {
        None => Some(format!("{prefix}{node}.{port}")),
        Some(def) if depth < 16 => {
            let out = def.outputs.iter().find(|o| o.name == port)?;
            flat_output(
                file,
                &def.blocks,
                &format!("{prefix}{node}/"),
                &out.from,
                depth + 1,
            )
        }
        _ => None,
    }
}

/// The recorded signal a canvas output carries: a filter's output by its
/// name, a model output by its flat name.
fn signal_of(file: &ModelFile, from: &str) -> Result<String, String> {
    if let Some(rest) = from.strip_prefix("probe:") {
        return rest
            .split_once('.')
            .map(|(probe, signal)| simsandbox_views::history::from_probe(probe, signal))
            .ok_or_else(|| format!("'{from}' is not an output"));
    }
    if let Some(rest) = from.strip_prefix("filter:") {
        // Bound by name, wherever the filter runs (ADR 0063).
        return rest
            .split_once('.')
            .map(|(_, port)| port.to_string())
            .ok_or_else(|| format!("'{from}' is not an output"));
    }
    flat_output(file, &file.blocks, "", from, 0)
        .ok_or_else(|| format!("'{from}' is not an output on the model's level"))
}

/// Model outputs on the diagram by the signal they record.
fn sources(file: &ModelFile, d: &Diagram) -> BTreeMap<String, (usize, usize)> {
    let mut out = BTreeMap::new();
    for (i, n) in d.nodes.iter().enumerate() {
        if !n.is_block() {
            continue;
        }
        for (k, p) in n.outputs.iter().enumerate() {
            let key = format!("{}.{}", n.name, p.name);
            if let Some(flat) = flat_output(file, &file.blocks, "", &key, 0) {
                out.insert(flat, (i, k));
            }
        }
    }
    out
}

/// Where a recorded signal comes from: the output itself, or for an
/// element (`lorenz.xyz[0]`) or a sweep's grid (`grid.spring.amplitude`)
/// the output it is made from.
fn resolve(sources: &BTreeMap<String, (usize, usize)>, signal: &str) -> Option<(usize, usize)> {
    let base = signal.strip_prefix("grid.").unwrap_or(signal);
    let whole = match base.rfind('[') {
        Some(i) if base.ends_with(']') => &base[..i],
        _ => base,
    };
    [signal, base, whole]
        .into_iter()
        .find_map(|s| sources.get(s).copied())
}

/// What a filter reads: its inputs as saved, or the names in its
/// expression.
pub(crate) fn filter_reads(f: &FilterEntry) -> Vec<String> {
    if !f.inputs.is_empty() {
        return f.inputs.clone();
    }
    f.expr()
        .and_then(|e| simsandbox_expr::parse_signals(e).ok())
        .map(|a| a.names().into_iter().map(|(n, _)| n).collect())
        .unwrap_or_default()
}

/// The filters with a clock of their own (ADR 0063): those with an input
/// from a probe, and those reading their outputs.
pub(crate) fn clocked(entries: &[FilterEntry]) -> Vec<FilterEntry> {
    let mut derived: Vec<FilterEntry> = entries.iter().filter(|e| e.on_probes()).cloned().collect();
    loop {
        let outputs: Vec<String> = derived.iter().flat_map(FilterEntry::outputs).collect();
        let more: Vec<FilterEntry> = entries
            .iter()
            .filter(|e| !derived.iter().any(|d| d.name == e.name))
            .filter(|e| filter_reads(e).iter().any(|r| outputs.contains(r)))
            .cloned()
            .collect();
        if more.is_empty() {
            return derived;
        }
        derived.extend(more);
    }
}

impl Diagram {
    /// Add the data layer to a model-level diagram: a node per filter and
    /// view, wired from the outputs they read (ADR 0050). Nodes without a
    /// `canvas` place go to the right of the model.
    pub fn observe(&mut self, file: &ModelFile, views: &ViewRegistry) {
        if self.scope != Scope::Model {
            return;
        }
        self.view_types = views
            .types()
            .iter()
            .map(|t| (t.name.to_string(), t.label.to_string()))
            .collect();
        let mut sources = sources(file, self);
        let model_nodes = self.nodes.len();
        // The model's filters first, so filters reading filters find
        // them; then probes, which may record them; then filters on
        // probes, which read the probes (ADR 0063).
        let mut reads: Vec<(usize, Vec<(String, String)>)> = Vec::new();
        let clocked = clocked(&file.controls.filters);
        for on_probe in [false, true] {
            if on_probe {
                for pr in &file.controls.probes {
                    let name = probe_node(&pr.name);
                    self.many.insert(format!("{name}.{RECORD}"));
                    // An output per signal it records, shaped like its source.
                    let i = self.nodes.len();
                    let outputs: Vec<Port> = pr
                        .record
                        .iter()
                        .map(|r| {
                            let (unit, shape) = resolve(&sources, r).map_or_else(
                                || (String::new(), Shape::Scalar),
                                |(n, k)| {
                                    let p = &self.nodes[n].outputs[k];
                                    (p.unit.clone(), p.shape)
                                },
                            );
                            Port {
                                name: r.clone(),
                                unit,
                                shape,
                            }
                        })
                        .collect();
                    for (k, r) in pr.record.iter().enumerate() {
                        sources.insert(simsandbox_views::history::from_probe(&pr.name, r), (i, k));
                    }
                    self.nodes.push(Node {
                        name,
                        kind: NodeKind::Probe,
                        type_name: "probe".into(),
                        method: if pr.record.is_empty() {
                            "records nothing yet".into()
                        } else {
                            probe_timing(pr)
                        },
                        inputs: vec![Port {
                            name: RECORD.into(),
                            unit: String::new(),
                            shape: Shape::Scalar,
                        }],
                        outputs,
                        at: pr.canvas.unwrap_or_default(),
                        placed: pr.canvas.is_some(),
                        error: None,
                        unreached: Vec::new(),
                        values: Vec::new(),
                        open: false,
                        // Its file, shown beside its input; whether it is
                        // part of the model (feeding a block, ADR 0063).
                        settings: pr
                            .to
                            .iter()
                            .map(|t| ("to".to_string(), t.clone()))
                            .chain(
                                probe_feeds_model(file, &pr.name)
                                    .then(|| ("feeds".to_string(), "model".to_string())),
                            )
                            .collect(),
                    });
                }
            }
            for f in file
                .controls
                .filters
                .iter()
                .filter(|f| clocked.iter().any(|c| c.name == f.name) == on_probe)
            {
                let i = self.nodes.len();
                let field = f.type_name == "cut";
                let outputs: Vec<Port> = f
                    .outputs()
                    .into_iter()
                    .map(|name| Port {
                        name,
                        unit: String::new(),
                        shape: if field {
                            Shape::Grid { nx: 0, ny: 1 }
                        } else {
                            Shape::Scalar
                        },
                    })
                    .collect();
                // Outputs are bound by name, wherever the filter runs.
                for (k, p) in outputs.iter().enumerate() {
                    sources.insert(p.name.clone(), (i, k));
                }
                let method = match f.type_name.as_str() {
                    "expression" if f.on_any() => {
                        format!("{} · on any", f.expr().unwrap_or_default())
                    }
                    "expression" => f.expr().unwrap_or_default().to_string(),
                    "cut" => {
                        let mode = f.settings.get("mode").and_then(|m| m.as_str());
                        format!("cut {}", mode.unwrap_or("along_x").replace('_', " "))
                    }
                    other => format!("unknown filter type '{other}'"),
                };
                self.nodes.push(Node {
                    name: filter_node(&f.name),
                    kind: NodeKind::Filter,
                    type_name: f.type_name.clone(),
                    method,
                    inputs: Vec::new(),
                    outputs,
                    at: f.canvas.unwrap_or_default(),
                    placed: f.canvas.is_some(),
                    error: None,
                    unreached: Vec::new(),
                    values: Vec::new(),
                    open: false,
                    // The probe each input comes from; `clock` marks a
                    // filter with a clock of its own.
                    settings: filter_reads(f)
                        .iter()
                        .filter_map(|r| {
                            f.source_of(r).map(|p| (format!("from.{r}"), p.to_string()))
                        })
                        .chain(on_probe.then(|| ("clock".to_string(), "own".to_string())))
                        .collect(),
                });
                let keys = filter_reads(f)
                    .into_iter()
                    .map(|r| {
                        let key = match f.source_of(&r) {
                            Some(p) => simsandbox_views::history::from_probe(p, &r),
                            None => r.clone(),
                        };
                        (r, key)
                    })
                    .collect();
                reads.push((i, keys));
            }
        }
        for (i, signals) in reads {
            let mut inputs = Vec::new();
            for (s, key) in signals {
                let (unit, shape) = match resolve(&sources, &key) {
                    Some((n, k)) => {
                        let p = &self.nodes[n].outputs[k];
                        (p.unit.clone(), p.shape)
                    }
                    None => (String::new(), Shape::Scalar),
                };
                inputs.push(Port {
                    name: s,
                    unit,
                    shape,
                });
            }
            self.nodes[i].inputs = inputs;
        }
        for (k, v) in file.controls.views.iter().enumerate() {
            let t = views.get(&v.type_name);
            let name = view_node(k);
            let mut slots: Vec<String> = t
                .map(|t| t.slots.iter().map(|s| s.name.to_string()).collect())
                .unwrap_or_default();
            for s in v.bind.keys() {
                if !slots.contains(s) {
                    slots.push(s.clone());
                }
            }
            if let Some(t) = t {
                for s in t.slots.iter().filter(|s| s.many) {
                    self.many.insert(format!("{name}.{}", s.name));
                }
            }
            self.nodes.push(Node {
                name,
                kind: NodeKind::View,
                type_name: v.type_name.clone(),
                method: match (t, &v.probe) {
                    (None, _) => "unknown view type".into(),
                    (Some(t), None) => t.label.to_string(),
                    (Some(t), Some(p)) => format!("{} · probe {p}", t.label),
                },
                inputs: slots
                    .into_iter()
                    .map(|name| Port {
                        name,
                        unit: String::new(),
                        shape: Shape::Scalar,
                    })
                    .collect(),
                outputs: Vec::new(),
                at: v.canvas.unwrap_or_default(),
                placed: v.canvas.is_some(),
                error: t
                    .is_none()
                    .then(|| format!("no view type '{}' in this build", v.type_name)),
                unreached: Vec::new(),
                values: Vec::new(),
                open: false,
                settings: Default::default(),
            });
        }
        if let Some(r) = &file.controls.recorder {
            self.nodes.push(Node {
                name: RECORDER.into(),
                kind: NodeKind::Recorder,
                type_name: RECORDER.into(),
                method: recorder_summary(r),
                inputs: Vec::new(),
                outputs: Vec::new(),
                at: r.canvas.unwrap_or_default(),
                placed: r.canvas.is_some(),
                error: None,
                unreached: Vec::new(),
                values: Vec::new(),
                open: false,
                settings: Default::default(),
            });
        }
        // Probes feeding blocks (ADR 0063): model wires, a value held
        // between the probe's samples.
        for c in &file.connections {
            let Some(rest) = c.from.strip_prefix("probe:") else {
                continue;
            };
            let Some((probe, signal)) = rest.split_once('.') else {
                continue;
            };
            let from = self.node(&probe_node(probe)).and_then(|n| {
                let k = self.nodes[n]
                    .outputs
                    .iter()
                    .position(|p| p.name == signal)?;
                Some((n, k))
            });
            let to = c.to.split_once('.').and_then(|(b, port)| {
                let n = self.node(b)?;
                let k = self.nodes[n].inputs.iter().position(|p| p.name == port)?;
                Some((n, k))
            });
            if let (Some(from), Some(to)) = (from, to) {
                self.wires.push(Wire {
                    from,
                    to,
                    from_name: c.from.clone(),
                    to_name: c.to.clone(),
                    signal: String::new(),
                });
            }
        }
        // Data wires: what each filter input, view slot and probe reads.
        let mut wires = Vec::new();
        for i in model_nodes..self.nodes.len() {
            let n = &self.nodes[i];
            let mut reads: Vec<(usize, String)> = Vec::new();
            match n.kind {
                NodeKind::Filter => reads.extend(n.inputs.iter().enumerate().map(|(k, p)| {
                    match n.settings.get(&format!("from.{}", p.name)) {
                        Some(probe) => (k, simsandbox_views::history::from_probe(probe, &p.name)),
                        None => (k, p.name.clone()),
                    }
                })),
                NodeKind::Probe => {
                    let name = &n.name["probe:".len()..];
                    if let Some(pr) = file.controls.probes.iter().find(|p| p.name == name) {
                        reads.extend(pr.record.iter().map(|s| (0, s.clone())));
                    }
                }
                _ => {
                    let k = n.name["view:".len()..].parse::<usize>().unwrap_or(0);
                    for (slot, b) in &file.controls.views[k].bind {
                        let port = n.inputs.iter().position(|p| &p.name == slot);
                        for s in b.signals() {
                            reads.extend(port.map(|p| (p, s)));
                        }
                    }
                }
            }
            for (port, signal) in reads {
                match resolve(&sources, &signal) {
                    Some(from) => wires.push(Wire {
                        from,
                        to: (i, port),
                        from_name: format!(
                            "{}.{}",
                            self.nodes[from.0].name, self.nodes[from.0].outputs[from.1].name
                        ),
                        to_name: format!(
                            "{}.{}",
                            self.nodes[i].name, self.nodes[i].inputs[port].name
                        ),
                        signal,
                    }),
                    None => {
                        let slot = self.nodes[i].inputs[port].name.clone();
                        let entry = if self.nodes[i].kind != NodeKind::Filter {
                            format!("{slot}: {signal}")
                        } else {
                            signal
                        };
                        self.nodes[i].unreached.push(entry);
                    }
                }
            }
        }
        self.wires.extend(wires);
        self.place_observers(model_nodes);
    }

    /// Place data-layer nodes without a saved place, in columns to the right
    /// of the model: filters, probes, views.
    fn place_observers(&mut self, first: usize) {
        let model = &self.nodes[..first];
        let right = model
            .iter()
            .map(|n| n.at[0] + NODE_WIDTH)
            .fold(f64::NEG_INFINITY, f64::max);
        let top = model.iter().map(|n| n.at[1]).fold(f64::INFINITY, f64::min);
        let x0 = if right.is_finite() {
            right + GAP_X
        } else {
            0.0
        };
        let y0 = if top.is_finite() { top } else { 0.0 };
        // Filters, then probes (which read the model and filters), then
        // filters on probes, then views (which read all of them), skipping
        // empty columns.
        let column = |n: &Node| match n.kind {
            NodeKind::Filter if n.settings.contains_key("clock") => 2,
            NodeKind::Filter => 0,
            NodeKind::Probe | NodeKind::Recorder => 1,
            _ => 3,
        };
        let mut x = x0;
        let mut columns = Vec::new();
        for c in 0..4 {
            if self.nodes[first..].iter().any(|n| column(n) == c) {
                columns.push((c, x));
                x += NODE_WIDTH + GAP_X;
            }
        }
        for (c, x) in columns {
            let mut y = y0;
            for n in self.nodes[first..].iter_mut().filter(|n| column(n) == c) {
                if !n.placed {
                    n.at = [x, y];
                }
                y = y.max(n.at[1]) + n.height() + GAP_Y;
            }
        }
    }
}

/// Why a filter cannot feed a model input.
pub fn feeds_model(filter: &str) -> String {
    format!(
        "{filter} is a filter: it reads recorded data after each step, so it cannot feed the \
         model; promote it to a block first (ADR 0032)"
    )
}

/// Apply an edit that touches the data layer; `None` if it does not.
pub fn apply(
    file: &mut ModelFile,
    scope: &Scope,
    registry: &Registry,
    edit: &Edit,
) -> Option<Result<String, String>> {
    let touches = match edit {
        Edit::Move { block, .. } => is_observer(block),
        Edit::Promote { .. } | Edit::PromoteConnect { .. } => true,
        Edit::Connect { from, .. } if is_observer(node_of(from)) => true,
        e => e.data_only(),
    };
    if !touches {
        return None;
    }
    if *scope != Scope::Model {
        return Some(Err(
            "filters and views belong to the whole model: edit them on its level".into(),
        ));
    }
    Some(apply_model(file, registry, edit))
}

fn filter_index(file: &ModelFile, node: &str) -> Result<usize, String> {
    let name = node.strip_prefix("filter:").unwrap_or(node);
    file.controls
        .filters
        .iter()
        .position(|f| f.name == name)
        .ok_or_else(|| format!("no filter '{name}'"))
}

fn probe_index(file: &ModelFile, node: &str) -> Result<usize, String> {
    let name = node.strip_prefix("probe:").unwrap_or(node);
    file.controls
        .probes
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| format!("no probe '{name}'"))
}

/// Record `signal` with probe node `node` too.
fn record(file: &mut ModelFile, node: &str, signal: String) -> Result<String, String> {
    let k = probe_index(file, node)?;
    let p = &mut file.controls.probes[k];
    if p.record.contains(&signal) {
        return Ok(format!("probe {} already records {signal}", p.name));
    }
    p.record.push(signal.clone());
    Ok(format!("probe {} records {signal}", p.name))
}

fn view_index(file: &ModelFile, node: &str) -> Result<usize, String> {
    node.strip_prefix("view:")
        .and_then(|k| k.parse::<usize>().ok())
        .filter(|&k| k < file.controls.views.len())
        .ok_or_else(|| format!("no view '{node}'"))
}

fn apply_model(file: &mut ModelFile, registry: &Registry, edit: &Edit) -> Result<String, String> {
    match edit {
        Edit::Move { block, at } => {
            let at = Some(at.map(tidy));
            if block.starts_with("filter:") {
                let k = filter_index(file, block)?;
                file.controls.filters[k].canvas = at;
            } else if block.starts_with("probe:") {
                let k = probe_index(file, block)?;
                file.controls.probes[k].canvas = at;
            } else if block == RECORDER {
                let r = file
                    .controls
                    .recorder
                    .as_mut()
                    .ok_or("there is no recorder")?;
                r.canvas = at;
            } else {
                let k = view_index(file, block)?;
                file.controls.views[k].canvas = at;
            }
            Ok(String::new())
        }
        Edit::Connect { from, to } => {
            if from.starts_with("probe:") && !to.starts_with("view:") && !to.starts_with("filter:")
            {
                let probe = node_of(from).trim_start_matches("probe:");
                if to.starts_with("probe:") {
                    return Err(format!(
                        "probe {probe}: a probe records the model and its filters, not another probe"
                    ));
                }
                // Into a block (ADR 0063): its samples, held between them.
                // This changes the model, which is rebuilt.
                let signal = from.split_once('.').map_or("", |(_, s)| s);
                let own = file
                    .blocks
                    .iter()
                    .any(|b| signal.starts_with(&format!("{}.", b.name)));
                if !own {
                    return Err(format!(
                        "probe {probe} can feed blocks the model's own outputs, not '{signal}'"
                    ));
                }
                file.connections.retain(|c| c.to != *to);
                file.connections.push(ConnectionEntry {
                    from: from.clone(),
                    to: to.clone(),
                });
                return Ok(format!(
                    "{to} reads {signal} as probe {probe} samples it, held between samples"
                ));
            }
            if is_observer(node_of(from)) && !is_observer(node_of(to)) {
                let name = node_of(from).trim_start_matches("filter:");
                return Err(feeds_model(name));
            }
            if to.starts_with("probe:") {
                let signal = signal_of(file, from)?;
                return record(file, node_of(to), signal);
            }
            if to.starts_with("view:") {
                // Without the view registry: a slot holding several takes
                // one more.
                let k = view_index(file, node_of(to))?;
                let slot = to.split_once('.').map_or("", |(_, s)| s);
                let many = matches!(
                    file.controls.views[k].bind.get(slot),
                    Some(Binding::Many(_))
                );
                return apply_model(
                    file,
                    registry,
                    &Edit::Bind {
                        from: from.clone(),
                        to: to.clone(),
                        many,
                    },
                );
            }
            let k = filter_index(file, node_of(to))?;
            let signal = signal_of(file, from)?;
            // From a probe, the filter runs on its samples (ADR 0063); from
            // the model, on every step.
            let (probe, plain) = match simsandbox_views::history::split_source(&signal) {
                (Some(p), s) => (Some(p.to_string()), s.to_string()),
                (None, s) => (None, s.to_string()),
            };
            let f = &mut file.controls.filters[k];
            if f.type_name != "cut" {
                // Into the input of that name: where it comes from.
                let port = to.split_once('.').map_or("", |(_, p)| p);
                if port != plain {
                    return Err(format!(
                        "{} reads the names in its formula: change it in Data…",
                        f.name
                    ));
                }
                set_source(f, &plain, probe.clone());
                return Ok(match probe {
                    Some(p) => format!("{} reads {plain} as probe {p} records it", f.name),
                    None => format!("{} reads {plain} from the model at every step", f.name),
                });
            }
            f.inputs = vec![plain.clone()];
            f.from.clear();
            f.probe = None;
            set_source(f, &plain, probe.clone());
            Ok(match probe {
                Some(p) => format!("{} now cuts {plain} as probe {p} records it", f.name),
                None => format!("{} now cuts {plain}", f.name),
            })
        }
        Edit::Bind { from, to, .. } if to.starts_with("probe:") => {
            let signal = signal_of(file, from)?;
            let clocked_filter = from
                .strip_prefix("filter:")
                .map(|r| r.split_once('.').map_or(r, |(n, _)| n))
                .is_some_and(|n| clocked(&file.controls.filters).iter().any(|c| c.name == n));
            if signal.starts_with("probe:") || clocked_filter {
                return Err(
                    "a probe records the model and its filters at every step, not other recordings"
                        .into(),
                );
            }
            record(file, node_of(to), signal)
        }
        Edit::Bind { from, to, many } => {
            let k = view_index(file, node_of(to))?;
            let slot = to
                .split_once('.')
                .map(|(_, s)| s.to_string())
                .ok_or("a view input is 'view:K.slot'")?;
            let signal = signal_of(file, from)?;
            let bind = &mut file.controls.views[k].bind;
            let mut now = bind.get(&slot).map(Binding::signals).unwrap_or_default();
            if *many {
                if now.contains(&signal) {
                    return Ok(format!("view {} already shows {signal}", k + 1));
                }
                now.push(signal.clone());
            } else {
                now = vec![signal.clone()];
            }
            bind.insert(slot.clone(), Binding::from(now));
            Ok(format!("view {} shows {signal} as {slot}", k + 1))
        }
        Edit::Unbind { to, signal } => unbind(file, to, Some(signal)),
        Edit::Disconnect { to } => unbind(file, to, None),
        Edit::Delete { block } if block == RECORDER => {
            file.controls.recorder = None;
            Ok("Removed the recorder: the model can no longer be rewound".into())
        }
        Edit::Delete { block } if block.starts_with("probe:") => {
            let k = probe_index(file, block)?;
            let p = file.controls.probes.remove(k);
            // What it fed is left unconnected (ADR 0063).
            let fed = format!("probe:{}.", p.name);
            file.connections.retain(|c| !c.from.starts_with(&fed));
            // Inputs from it come from the model again.
            for f in &mut file.controls.filters {
                if f.probe.as_deref() == Some(p.name.as_str()) {
                    f.probe = None;
                }
                f.from.retain(|_, q| *q != p.name);
            }
            let prefix = simsandbox_views::history::from_probe(&p.name, "");
            unbind_views(file, &|s| s.starts_with(&prefix));
            Ok(format!("Deleted the probe {}", p.name))
        }
        Edit::Delete { block } => {
            if block.starts_with("filter:") {
                let k = filter_index(file, block)?;
                let f = file.controls.filters.remove(k);
                let outputs = f.outputs();
                for p in &mut file.controls.probes {
                    p.record.retain(|s| !outputs.contains(s));
                }
                for v in &mut file.controls.views {
                    for b in v.bind.values_mut() {
                        let kept: Vec<String> = b
                            .signals()
                            .into_iter()
                            .filter(|s| {
                                let inner = simsandbox_views::history::split_source(s).1;
                                !outputs.iter().any(|o| o == inner)
                            })
                            .collect();
                        *b = Binding::from(kept);
                    }
                    v.bind.retain(|_, b| !b.signals().is_empty());
                }
                Ok(format!("Deleted the filter {}", f.name))
            } else {
                let k = view_index(file, block)?;
                file.controls.views.remove(k);
                Ok(format!("Deleted view {}", k + 1))
            }
        }
        Edit::Rename { from, .. } if from == RECORDER => {
            Err("the recorder has no name: a model has one".into())
        }
        Edit::Rename { from, to } if from.starts_with("probe:") => {
            let k = probe_index(file, from)?;
            crate::filters::plain_name(to)?;
            if file.controls.probes.iter().any(|p| &p.name == to) {
                return Err(format!("there is already a probe '{to}'"));
            }
            let old = std::mem::replace(&mut file.controls.probes[k].name, to.clone());
            let (was_fed, now_fed) = (format!("probe:{old}."), format!("probe:{to}."));
            for c in &mut file.connections {
                if let Some(rest) = c.from.strip_prefix(&was_fed) {
                    c.from = format!("{now_fed}{rest}");
                }
            }
            for f in &mut file.controls.filters {
                if f.probe.as_deref() == Some(old.as_str()) {
                    f.probe = Some(to.clone());
                }
                for q in f.from.values_mut() {
                    if *q == old {
                        *q = to.clone();
                    }
                }
            }
            let (was, now) = (
                simsandbox_views::history::from_probe(&old, ""),
                simsandbox_views::history::from_probe(to, ""),
            );
            for v in &mut file.controls.views {
                for b in v.bind.values_mut() {
                    let renamed: Vec<String> = b
                        .signals()
                        .into_iter()
                        .map(|s| match s.strip_prefix(&was) {
                            Some(rest) => format!("{now}{rest}"),
                            None => s,
                        })
                        .collect();
                    *b = Binding::from(renamed);
                }
            }
            Ok(format!("Renamed the probe {old} to {to}"))
        }
        Edit::Rename { from, to } => {
            if from.starts_with("view:") {
                return Err("views have no names; they are numbered in order".into());
            }
            let k = filter_index(file, from)?;
            let old = file.controls.filters[k].name.clone();
            crate::filters::plain_name(to)?;
            let taken = file.controls.filters.iter().any(|f| &f.name == to)
                || file.blocks.iter().any(|b| &b.name == to);
            if taken {
                return Err(format!("'{to}' is already used"));
            }
            rename_signal(file, &old, to);
            file.controls.filters[k].name = to.clone();
            Ok(format!("Renamed the filter {old} to {to}"))
        }
        Edit::AddFilter { at } => {
            let name = free_name("signal", |n| {
                file.controls.filters.iter().any(|f| f.name == n)
                    || file.blocks.iter().any(|b| b.name == n)
            });
            let mut f = FilterEntry::expression(&name, "t", Vec::new());
            f.canvas = Some(at.map(tidy));
            file.controls.filters.push(f);
            Ok(format!(
                "Added the filter {name} (edit its formula in Data…)"
            ))
        }
        Edit::SetFilterWhen { filter, any } => {
            let k = filter_index(file, filter)?;
            let f = &mut file.controls.filters[k];
            f.when = any.then(|| "any".to_string());
            Ok(if *any {
                format!("{} runs whenever one of its inputs records", f.name)
            } else {
                format!("{} runs where its inputs' times pair", f.name)
            })
        }
        Edit::AddRecorder { at } => {
            if file.controls.recorder.is_some() {
                return Err("the model has a recorder already".into());
            }
            file.controls.recorder = Some(simsandbox_file::RecorderEntry {
                canvas: Some(at.map(tidy)),
                ..Default::default()
            });
            Ok("Added a recorder: the model can be rewound from now on".into())
        }
        Edit::SetRecorder { recorder } => {
            recorder.check()?;
            let canvas = file.controls.recorder.as_ref().and_then(|r| r.canvas);
            file.controls.recorder = Some(simsandbox_file::RecorderEntry {
                canvas,
                ..recorder.clone()
            });
            Ok(format!("Recorder: {}", recorder_summary(recorder)))
        }
        Edit::AddProbe { at } => {
            let name = free_name("probe", |n| {
                file.controls.probes.iter().any(|p| p.name == n)
            });
            file.controls.probes.push(ProbeEntry {
                name: name.clone(),
                record: Vec::new(),
                every: None,
                keep: None,
                to: None,
                stream: false,
                canvas: Some(at.map(tidy)),
            });
            Ok(format!(
                "Added the probe {name}: wire the signals it records into it"
            ))
        }
        Edit::SetProbe { probe } => {
            let k = probe_index(file, &probe.name)?;
            probe.check()?;
            let mut p = probe.clone();
            p.canvas = file.controls.probes[k].canvas;
            file.controls.probes[k] = p;
            Ok(format!("probe {}: {}", probe.name, probe_summary(probe)))
        }
        Edit::AddView { type_name, at } => {
            file.controls.views.push(ViewEntry {
                type_name: type_name.clone(),
                bind: BTreeMap::new(),
                style: BTreeMap::new(),
                camera: None,
                selection: None,
                canvas: Some(at.map(tidy)),
                probe: None,
            });
            Ok(format!("Added a {type_name} view"))
        }
        Edit::Promote { filter } => {
            let (block, _) = promote(file, registry, filter)?;
            Ok(format!("Promoted the filter {filter} to the block {block}"))
        }
        Edit::PromoteConnect { filter, to } => {
            let (block, output) = promote(file, registry, filter)?;
            apply_in(
                file,
                &Scope::Model,
                registry,
                &Edit::Connect {
                    from: output.clone(),
                    to: to.clone(),
                },
            )?;
            Ok(format!(
                "Promoted the filter {filter} to the block {block} and connected {output} to {to}"
            ))
        }
        _ => Err("not a data-layer edit".into()),
    }
}

/// Stop a data-layer input reading `signal` (every signal if `None`).
fn unbind(file: &mut ModelFile, to: &str, signal: Option<&String>) -> Result<String, String> {
    let (node, slot) = to.split_once('.').ok_or("an input is 'node.port'")?;
    if node.starts_with("probe:") {
        let k = probe_index(file, node)?;
        let p = &mut file.controls.probes[k];
        match signal {
            Some(s) => p.record.retain(|x| x != s),
            None => p.record.clear(),
        }
        // What it no longer records, views no longer read from it.
        let (prefix, kept) = (
            simsandbox_views::history::from_probe(&p.name, ""),
            p.record.clone(),
        );
        let p = file.controls.probes[k].clone();
        let fed = format!("probe:{}.", p.name);
        file.connections.retain(|c| {
            c.from
                .strip_prefix(&fed)
                .is_none_or(|s| kept.iter().any(|k| k == s))
        });
        unbind_views(file, &|s| {
            s.strip_prefix(&prefix)
                .is_some_and(|r| !kept.iter().any(|k| k == r))
        });
        return Ok(match signal {
            Some(s) => format!("probe {} no longer records {s}", p.name),
            None => format!("probe {} records nothing", p.name),
        });
    }
    if node.starts_with("view:") {
        let k = view_index(file, node)?;
        let bind = &mut file.controls.views[k].bind;
        let now = bind.get(slot).map(Binding::signals).unwrap_or_default();
        let kept: Vec<String> = match signal {
            Some(s) => now.into_iter().filter(|x| x != s).collect(),
            None => Vec::new(),
        };
        if kept.is_empty() {
            bind.remove(slot);
        } else {
            bind.insert(slot.to_string(), Binding::from(kept));
        }
        return Ok(format!("view {} no longer shows that as {slot}", k + 1));
    }
    let k = filter_index(file, node)?;
    let f = &mut file.controls.filters[k];
    if f.type_name != "cut" {
        // An input from a probe comes from the model again (ADR 0063).
        if f.source_of(slot).is_some() {
            set_source(f, slot, None);
            return Ok(format!(
                "{} reads {slot} from the model at every step",
                f.name
            ));
        }
        return Err(format!(
            "{} reads the names in its formula: change it in Data…",
            f.name
        ));
    }
    f.inputs.clear();
    Ok(format!(
        "{} cuts nothing until a grid is wired to it",
        f.name
    ))
}

/// Set where filter `f`'s input `input` comes from: a probe, or the model
/// (ADR 0063). A shared `probe` is spelled out per input first.
fn set_source(f: &mut FilterEntry, input: &str, probe: Option<String>) {
    if let Some(p) = f.probe.take() {
        for r in filter_reads(f) {
            f.from.entry(r).or_insert_with(|| p.clone());
        }
    }
    match probe {
        Some(p) => {
            f.from.insert(input.to_string(), p);
        }
        None => {
            f.from.remove(input);
        }
    }
}

/// Remove view bindings to the signals `gone` says.
fn unbind_views(file: &mut ModelFile, gone: &dyn Fn(&str) -> bool) {
    for v in &mut file.controls.views {
        for b in v.bind.values_mut() {
            let kept: Vec<String> = b.signals().into_iter().filter(|s| !gone(s)).collect();
            *b = Binding::from(kept);
        }
        v.bind.retain(|_, b| !b.signals().is_empty());
    }
}

/// Replace the names a filter's outputs go by (`old`, `old[k]`,
/// `old.x`) in filters and views.
fn rename_signal(file: &mut ModelFile, old: &str, new: &str) {
    let plain = |s: &str| -> Option<String> {
        let rest = s.strip_prefix(old)?;
        (rest.is_empty() || rest.starts_with('[') || rest.starts_with('.'))
            .then(|| format!("{new}{rest}"))
    };
    // Also as recorded by a probe (`probe:NAME/old`).
    let map = |s: &str| -> Option<String> {
        match simsandbox_views::history::split_source(s) {
            (Some(p), inner) => plain(inner).map(|n| simsandbox_views::history::from_probe(p, &n)),
            (None, _) => plain(s),
        }
    };
    for f in &mut file.controls.filters {
        for i in &mut f.inputs {
            if let Some(n) = map(i) {
                *i = n;
            }
        }
        if let Some(e) = f.expr().map(str::to_string) {
            f.settings.insert(
                "expr".into(),
                serde_json::Value::String(map_names(&e, &map)),
            );
        }
    }
    for p in &mut file.controls.probes {
        for s in &mut p.record {
            if let Some(n) = map(s) {
                *s = n;
            }
        }
    }
    for v in &mut file.controls.views {
        for b in v.bind.values_mut() {
            let renamed: Vec<String> = b
                .signals()
                .into_iter()
                .map(|s| map(&s).unwrap_or(s))
                .collect();
            *b = Binding::from(renamed);
        }
    }
}

/// An expression with each (dotted, indexed) name passed through `f`;
/// numbers and the rest are kept as written.
pub fn map_names(expr: &str, f: &dyn Fn(&str) -> Option<String>) -> String {
    let c: Vec<char> = expr.chars().collect();
    let mut out = String::with_capacity(expr.len());
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
            // A number, exponent included.
            let start = i;
            while i < c.len()
                && (c[i].is_ascii_digit()
                    || c[i] == '.'
                    || matches!(c[i], 'e' | 'E')
                    || (matches!(c[i], '+' | '-') && matches!(c[i - 1], 'e' | 'E')))
            {
                i += 1;
            }
            out.extend(&c[start..i]);
        } else if ch.is_alphabetic() || ch == '_' {
            // A name, as signal expressions read it: `a.v`, `lorenz.xyz[0]`.
            let start = i;
            let word = |i: &mut usize| {
                while *i < c.len() && (c[*i].is_alphanumeric() || c[*i] == '_') {
                    *i += 1;
                }
            };
            word(&mut i);
            while c.get(i) == Some(&'.')
                && c.get(i + 1).is_some_and(|d| d.is_alphabetic() || *d == '_')
            {
                i += 1;
                word(&mut i);
            }
            if c.get(i) == Some(&'[') {
                let mut j = i + 1;
                while c.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                if j > i + 1 && c.get(j) == Some(&']') {
                    i = j + 1;
                }
            }
            let name: String = c[start..i].iter().collect();
            out.push_str(&f(&name).unwrap_or(name));
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// Turn an expression filter into a formula block (ADR 0032): an
/// `eq.equations` block with an input (in the source's unit) for each
/// output it reads, wired from there, defining the filter's name. Views
/// and filters that read the filter then read the block. Returns the block
/// and its output (`block.name`).
fn promote(
    file: &mut ModelFile,
    registry: &Registry,
    name: &str,
) -> Result<(String, String), String> {
    let k = filter_index(file, name)?;
    let f = file.controls.filters[k].clone();
    if f.type_name != "expression" {
        return Err(format!(
            "{name} is a {} filter; only expression filters can become blocks",
            f.type_name
        ));
    }
    let text = f
        .expr()
        .ok_or_else(|| format!("{name} has no formula"))?
        .to_string();
    let ast = simsandbox_expr::parse_signals(&text).map_err(|e| format!("{name}: {e}"))?;
    let d = Diagram::of(file, &Scope::Model, registry);
    let sources = sources(file, &d);
    let filtered: Vec<String> = file
        .controls
        .filters
        .iter()
        .flat_map(FilterEntry::outputs)
        .collect();
    // (signal, input name, unit) for each output read.
    let mut inputs: Vec<(String, String, String)> = Vec::new();
    let read: Vec<String> = ast
        .names()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n != "t" && !simsandbox_expr::is_reserved(n))
        .collect();
    for n in &read {
        if filtered.contains(n) {
            return Err(format!("{name} reads the filter {n}: promote {n} first"));
        }
        let Some(&(i, p)) = sources.get(n) else {
            return Err(format!(
                "{name} reads {n}, which is not an output of a block on the model's level (an \
                 element, a parameter or a sweep's value); a block cannot read it"
            ));
        };
        let port = &d.nodes[i].outputs[p];
        if port.shape != Shape::Scalar {
            return Err(format!("{name} reads {n}, which is not a single number"));
        }
        let mut input = n.replace(['.', '/'], "_");
        while input == name || inputs.iter().any(|(_, q, _)| *q == input) {
            input.push('_');
        }
        // The block's input connects to where the signal comes from.
        let source = format!("{}.{}", d.nodes[i].name, port.name);
        inputs.push((source, input, port.unit.clone()));
    }
    let block = if file.blocks.iter().any(|b| b.name == name) {
        free_name(name, |c| file.blocks.iter().any(|b| b.name == c))
    } else {
        name.to_string()
    };
    let rename: BTreeMap<String, String> = read
        .into_iter()
        .zip(inputs.iter())
        .map(|(n, (_, input, _))| (n, input.clone()))
        .collect();
    let formula = map_names(&text, &|n: &str| rename.get(n).cloned());
    let mut equations = vec![format!("# promoted from the filter {name}")];
    if !inputs.is_empty() {
        let list: Vec<String> = inputs
            .iter()
            .map(|(_, input, unit)| {
                if unit.is_empty() || unit == "1" {
                    input.clone()
                } else {
                    format!("{input} [{unit}]")
                }
            })
            .collect();
        equations.push(format!("input {}", list.join(", ")));
    }
    equations.push(format!("{name} = {formula}"));
    file.blocks.push(BlockEntry {
        name: block.clone(),
        type_name: "eq.equations".into(),
        equations,
        canvas: f.canvas,
        ..Default::default()
    });
    for (source, input, _) in &inputs {
        file.connections.push(ConnectionEntry {
            from: source.clone(),
            to: format!("{block}.{input}"),
        });
    }
    file.controls.filters.remove(k);
    let output = format!("{block}.{name}");
    rename_signal(file, name, &output);
    Ok((block, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::apply;
    use crate::examples::{registry, view_registry, EXAMPLES};

    fn file(part: &str) -> ModelFile {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        ModelFile::from_json(e.text).unwrap().0
    }

    fn observed(f: &ModelFile) -> Diagram {
        let mut d = Diagram::new(f, &registry());
        d.observe(f, &view_registry());
        d
    }

    /// The final value of a recorded signal after `t` seconds.
    fn last(f: &ModelFile, signal: &str, t: f64) -> f64 {
        let r = crate::run::run(&f.to_json(), &registry(), Some(t), None).unwrap();
        let o = r.outputs.iter().find(|o| o.name == signal).unwrap();
        *o.values.last().unwrap()
    }

    #[test]
    fn filters_and_views_are_nodes_wired_from_what_they_read() {
        let f = file("oscillators");
        let d = observed(&f);
        let energy = d.node("filter:energy").unwrap();
        let n = &d.nodes[energy];
        assert_eq!(n.kind, NodeKind::Filter);
        assert_eq!(n.inputs.len(), 4);
        assert!(n.inputs.iter().any(|p| p.name == "a.v" && p.unit == "m/s"));
        let into = |i: usize| d.wires.iter().filter(move |w| w.to.0 == i);
        assert_eq!(into(energy).count(), 4);
        assert!(into(energy).all(|w| w.is_data() && d.nodes[w.from.0].kind == NodeKind::Block));
        // The third view plots the filter's output against t, which has no
        // port: it is listed instead.
        let v = d.node("view:2").unwrap();
        let wires: Vec<_> = into(v).collect();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].from.0, energy);
        assert_eq!(d.nodes[v].unreached, ["x: t"]);
        assert!(d.many.contains("view:0.y") && !d.many.contains("view:0.x"));
        // Placed to the right of the model, filters before views.
        let right = d
            .nodes
            .iter()
            .filter(|n| n.is_block())
            .map(|n| n.at[0])
            .fold(0.0, f64::max);
        assert!(d.nodes[energy].at[0] > right && d.nodes[v].at[0] > d.nodes[energy].at[0]);
        // A model-level diagram without the data layer is as before.
        assert!(Diagram::new(&f, &registry())
            .nodes
            .iter()
            .all(|n| !n.is_observer()));
    }

    #[test]
    fn grids_elements_and_subsystem_outputs_find_their_ports() {
        // A cut through a sweep's grid reads the swept block's output.
        let d = observed(&file("Resonance map"));
        let cut = d.node("filter:cuts1").unwrap();
        let w = d.wires.iter().find(|w| w.to.0 == cut).unwrap();
        assert_eq!(d.nodes[w.from.0].name, "spring");
        assert_eq!(w.signal, "grid.spring.amplitude");
        // Vector elements come from the vector; a use's inner signal from
        // the use's port.
        let d = observed(&file("Lorenz"));
        assert!(d
            .wires
            .iter()
            .any(|w| w.signal == "lorenz.xyz[0]" && d.nodes[w.from.0].name == "lorenz"));
        let d = observed(&file("Platoon"));
        let w = d.wires.iter().find(|w| w.signal == "car1/car.v").unwrap();
        assert_eq!(w.from_name, "car1.v");
    }

    #[test]
    fn data_edits_change_only_filters_and_views() {
        let r = registry();
        let mut f = file("oscillators");
        let before = (f.blocks.clone(), f.connections.clone());
        // A view slot that takes several gains one; one that takes one
        // swaps it.
        let bind = |to: &str, from: &str, many: bool| Edit::Bind {
            from: from.into(),
            to: to.into(),
            many,
        };
        apply(&mut f, &r, &bind("view:0.y", "a.v", true)).unwrap();
        assert_eq!(
            f.controls.views[0].bind["y"].signals(),
            ["a.x", "b.x", "a.v"]
        );
        apply(&mut f, &r, &bind("view:1.x", "filter:energy.energy", false)).unwrap();
        assert_eq!(f.controls.views[1].bind["x"].signals(), ["energy"]);
        apply(
            &mut f,
            &r,
            &Edit::Unbind {
                to: "view:0.y".into(),
                signal: "a.x".into(),
            },
        )
        .unwrap();
        assert_eq!(f.controls.views[0].bind["y"].signals(), ["b.x", "a.v"]);
        // Renaming a filter follows it into views.
        apply(
            &mut f,
            &r,
            &Edit::Rename {
                from: "filter:energy".into(),
                to: "total".into(),
            },
        )
        .unwrap();
        assert_eq!(f.controls.filters[0].name, "total");
        assert_eq!(f.controls.views[2].bind["y"].signals(), ["total"]);
        // Placing, adding and deleting.
        apply(
            &mut f,
            &r,
            &Edit::Move {
                block: "view:2".into(),
                at: [700.04, 10.0],
            },
        )
        .unwrap();
        assert_eq!(f.controls.views[2].canvas, Some([700.0, 10.0]));
        apply(&mut f, &r, &Edit::AddFilter { at: [0.0, 300.0] }).unwrap();
        assert_eq!(f.controls.filters[1].name, "signal1");
        apply(
            &mut f,
            &r,
            &Edit::AddView {
                type_name: "plot.xy".into(),
                at: [0.0, 0.0],
            },
        )
        .unwrap();
        assert_eq!(f.controls.views.len(), 4);
        apply(
            &mut f,
            &r,
            &Edit::Delete {
                block: "filter:total".into(),
            },
        )
        .unwrap();
        assert!(
            !f.controls.views[2].bind.contains_key("y"),
            "its views stop showing it"
        );
        apply(
            &mut f,
            &r,
            &Edit::Delete {
                block: "view:3".into(),
            },
        )
        .unwrap();
        assert_eq!(f.controls.views.len(), 3);
        assert_eq!((f.blocks.clone(), f.connections.clone()), before);
        // They are told apart from model edits.
        assert!(bind("view:0.y", "a.v", true).data_only());
        assert!(Edit::Delete {
            block: "view:0".into()
        }
        .data_only());
        assert!(!Edit::Delete { block: "a".into() }.data_only());
        assert!(!Edit::Promote { filter: "x".into() }.data_only());
        // Not inside a subsystem.
        let mut p = file("Platoon");
        let err = crate::diagram::apply_in(
            &mut p,
            &Scope::Subsystem("follower".into()),
            &r,
            &Edit::AddFilter { at: [0.0, 0.0] },
        )
        .unwrap_err();
        assert!(err.contains("whole model"), "{err}");
    }

    #[test]
    fn a_filter_cannot_feed_the_model_until_it_is_promoted() {
        let r = registry();
        let mut f = file("Cruise");
        // Car and controller are solved together; the promoted block joins
        // them (ADR 0051), so the run is the same.
        f.controls
            .filters
            .push(FilterEntry::expression("push", "cruise.F", Vec::new()));
        f.controls.views[1]
            .bind
            .insert("y".into(), Binding::One("push".into()));
        let v = last(&f, "car.v", 5.0);
        apply(&mut f, &r, &Edit::Disconnect { to: "car.F".into() }).unwrap();
        let err = apply(
            &mut f,
            &r,
            &Edit::Connect {
                from: "filter:push.push".into(),
                to: "car.F".into(),
            },
        )
        .unwrap_err();
        assert!(
            err.contains("cannot feed the model") && err.contains("promote"),
            "{err}"
        );
        // Promoted and connected: a formula block with an input in N, and
        // the run is the same.
        apply(
            &mut f,
            &r,
            &Edit::PromoteConnect {
                filter: "push".into(),
                to: "car.F".into(),
            },
        )
        .unwrap();
        let b = f.blocks.iter().find(|b| b.name == "push").unwrap();
        assert_eq!(b.type_name, "eq.equations");
        assert!(
            b.equations.iter().any(|l| l == "input cruise_F [kg·m/s^2]"),
            "{:?}",
            b.equations
        );
        assert!(b.equations.iter().any(|l| l == "push = cruise_F"));
        assert!(f.controls.filters.is_empty());
        assert_eq!(f.controls.views[1].bind["y"].signals(), ["push.push"]);
        assert_eq!(last(&f, "car.v", 5.0), v);
        assert_eq!(last(&f, "push.push", 5.0), last(&f, "cruise.F", 5.0));
    }

    #[test]
    fn what_cannot_become_a_block_says_why() {
        let r = registry();
        let mut f = file("oscillators");
        // Reads parameters (a.kappa), which a block cannot.
        let err = apply(
            &mut f,
            &r,
            &Edit::Promote {
                filter: "energy".into(),
            },
        )
        .unwrap_err();
        assert!(
            err.contains("a.omega0_sq") && err.contains("cannot read"),
            "{err}"
        );
        f.controls
            .filters
            .push(FilterEntry::expression("e2", "2*energy", Vec::new()));
        let err = apply(
            &mut f,
            &r,
            &Edit::Promote {
                filter: "e2".into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("promote energy first"), "{err}");
        let err = apply(
            &mut f,
            &r,
            &Edit::Promote {
                filter: "cuts".into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("no filter"), "{err}");
    }

    #[test]
    fn names_in_expressions_are_found_as_the_parser_reads_them() {
        let map = |e: &str| {
            map_names(e, &|n: &str| {
                (n == "e" || n.starts_with("e[")).then(|| format!("blk.{n}"))
            })
        };
        assert_eq!(
            map("2*e + e2 - a.e + 1.5e3*e[1]"),
            "2*blk.e + e2 - a.e + 1.5e3*blk.e[1]"
        );
        assert_eq!(map("sin(e)^2"), "sin(blk.e)^2");
    }

    #[test]
    fn probes_are_nodes_that_record_what_is_wired_into_them() {
        let mut f = file("oscillators");
        apply(&mut f, &registry(), &Edit::AddProbe { at: [600.0, 0.0] }).unwrap();
        assert_eq!(f.controls.probes[0].name, "probe1");
        let d = observed(&f);
        let i = d.node("probe:probe1").unwrap();
        assert_eq!(d.nodes[i].kind, NodeKind::Probe);
        assert_eq!(d.nodes[i].method, "records nothing yet");
        assert!(d.many.contains("probe:probe1.record"));
        // Wire a block output and a filter output into it.
        for from in ["a.x", "filter:energy.energy"] {
            let e = Edit::Bind {
                from: from.into(),
                to: "probe:probe1.record".into(),
                many: true,
            };
            assert!(e.data_only());
            apply(&mut f, &registry(), &e).unwrap();
        }
        assert_eq!(f.controls.probes[0].record, ["a.x", "energy"]);
        let d = observed(&f);
        let i = d.node("probe:probe1").unwrap();
        assert_eq!(d.wires.iter().filter(|w| w.to.0 == i).count(), 2);
        // Renaming the filter follows; deleting it drops what it gave.
        let rename = Edit::Rename {
            from: "filter:energy".into(),
            to: "e".into(),
        };
        apply(&mut f, &registry(), &rename).unwrap();
        assert_eq!(f.controls.probes[0].record, ["a.x", "e"]);
        let unbind = Edit::Unbind {
            to: "probe:probe1.record".into(),
            signal: "a.x".into(),
        };
        apply(&mut f, &registry(), &unbind).unwrap();
        apply(
            &mut f,
            &registry(),
            &Edit::Delete {
                block: "filter:e".into(),
            },
        )
        .unwrap();
        assert!(f.controls.probes[0].record.is_empty());
        // Settings, renaming and deleting the probe itself.
        let mut p = f.controls.probes[0].clone();
        p.to = Some("osc.csv".into());
        p.every = Some(simsandbox_file::ProbeEvery {
            seconds: Some(0.5),
            ..Default::default()
        });
        apply(&mut f, &registry(), &Edit::SetProbe { probe: p.clone() }).unwrap();
        assert_eq!(
            probe_summary(&f.controls.probes[0]),
            "every 0.5 s · thinned · osc.csv"
        );
        p.to = Some("../x.csv".into());
        assert!(apply(&mut f, &registry(), &Edit::SetProbe { probe: p }).is_err());
        let rename = Edit::Rename {
            from: "probe:probe1".into(),
            to: "osc".into(),
        };
        apply(&mut f, &registry(), &rename).unwrap();
        apply(
            &mut f,
            &registry(),
            &Edit::Delete {
                block: "probe:osc".into(),
            },
        )
        .unwrap();
        assert!(f.controls.probes.is_empty());
    }

    #[test]
    fn probe_outputs_feed_views_with_the_probes_recording() {
        let mut f = file("oscillators");
        apply(&mut f, &registry(), &Edit::AddProbe { at: [600.0, 0.0] }).unwrap();
        let bind = |from: &str, to: &str| Edit::Bind {
            from: from.into(),
            to: to.into(),
            many: true,
        };
        apply(&mut f, &registry(), &bind("a.x", "probe:probe1.record")).unwrap();
        let d = observed(&f);
        let p = d.node("probe:probe1").unwrap();
        assert_eq!(d.nodes[p].outputs.len(), 1);
        assert_eq!(d.nodes[p].outputs[0].unit, "m");
        // Into a view: the view reads the probe's recording.
        apply(&mut f, &registry(), &bind("probe:probe1.a.x", "view:0.y")).unwrap();
        assert!(f.controls.views[0].bind["y"]
            .signals()
            .contains(&"probe:probe1/a.x".to_string()));
        let d = observed(&f);
        let (p, v) = (d.node("probe:probe1").unwrap(), d.node("view:0").unwrap());
        assert!(d.wires.iter().any(|w| w.from.0 == p && w.to.0 == v));
        // Into a block (ADR 0063 step 3): a model connection, drawn as a
        // model wire from the probe.
        let e = Edit::Connect {
            from: "probe:probe1.a.x".into(),
            to: "b.neighbor_x".into(),
        };
        assert!(e.changes_model());
        apply(&mut f, &registry(), &e).unwrap();
        assert!(f
            .connections
            .iter()
            .any(|c| c.from == "probe:probe1.a.x" && c.to == "b.neighbor_x"));
        assert_eq!(
            f.connections
                .iter()
                .filter(|c| c.to == "b.neighbor_x")
                .count(),
            1
        );
        let d = observed(&f);
        let (p, b) = (d.node("probe:probe1").unwrap(), d.node("b").unwrap());
        assert!(d
            .wires
            .iter()
            .any(|w| w.from.0 == p && w.to.0 == b && !w.is_data()));
        // The model loads with the hold in place.
        assert!(simsandbox_file::load(&f.to_json(), &registry()).is_ok());
        // Renaming the probe follows into the view; deleting it unbinds.
        let rename = Edit::Rename {
            from: "probe:probe1".into(),
            to: "pos".into(),
        };
        apply(&mut f, &registry(), &rename).unwrap();
        assert!(f.controls.views[0].bind["y"]
            .signals()
            .contains(&"probe:pos/a.x".to_string()));
        apply(
            &mut f,
            &registry(),
            &Edit::Delete {
                block: "probe:pos".into(),
            },
        )
        .unwrap();
        assert!(!f.controls.views[0]
            .bind
            .values()
            .flat_map(Binding::signals)
            .any(|s| s.starts_with("probe:")));
    }

    #[test]
    fn filter_inputs_come_from_probes_one_by_one() {
        let mut f = file("oscillators");
        for _ in 0..2 {
            apply(&mut f, &registry(), &Edit::AddProbe { at: [600.0, 0.0] }).unwrap();
        }
        let bind = |from: &str, to: &str| Edit::Bind {
            from: from.into(),
            to: to.into(),
            many: true,
        };
        for s in ["a.x", "a.v"] {
            apply(&mut f, &registry(), &bind(s, "probe:probe1.record")).unwrap();
        }
        apply(&mut f, &registry(), &bind("b.x", "probe:probe2.record")).unwrap();
        // energy reads a.x, a.v, b.x, b.v: two from probe1, one from probe2.
        let connect = |from: &str, to: &str| Edit::Connect {
            from: from.into(),
            to: to.into(),
        };
        for (from, to) in [
            ("probe:probe1.a.x", "filter:energy.a.x"),
            ("probe:probe1.a.v", "filter:energy.a.v"),
            ("probe:probe2.b.x", "filter:energy.b.x"),
        ] {
            let note = apply(&mut f, &registry(), &connect(from, to)).unwrap();
            assert!(note.contains("as probe"), "{note}");
        }
        let e = &f.controls.filters[0];
        assert_eq!(e.source_of("a.x"), Some("probe1"));
        assert_eq!(e.source_of("b.x"), Some("probe2"));
        assert_eq!(e.source_of("b.v"), None);
        // A wire into an input of another name is refused.
        assert!(apply(
            &mut f,
            &registry(),
            &connect("probe:probe1.a.x", "filter:energy.b.v")
        )
        .is_err());
        // The view still binds energy by name, now from its own recording.
        assert!(f.controls.views[2]
            .bind
            .values()
            .flat_map(Binding::signals)
            .any(|s| s == "energy"));
        let d = observed(&f);
        let (p1, p2, en) = (
            d.node("probe:probe1").unwrap(),
            d.node("probe:probe2").unwrap(),
            d.node("filter:energy").unwrap(),
        );
        let into = |from: usize| {
            d.wires
                .iter()
                .filter(|w| w.from.0 == from && w.to.0 == en)
                .count()
        };
        assert_eq!((into(p1), into(p2)), (2, 1));
        assert!(d.nodes[en].settings.contains_key("clock"));
        // On any input; then an input back to the model; deleting a probe
        // puts its inputs back on the model.
        let when = Edit::SetFilterWhen {
            filter: "filter:energy".into(),
            any: true,
        };
        apply(&mut f, &registry(), &when).unwrap();
        assert!(f.controls.filters[0].on_any());
        let back = Edit::Disconnect {
            to: "filter:energy.a.v".into(),
        };
        apply(&mut f, &registry(), &back).unwrap();
        assert_eq!(f.controls.filters[0].source_of("a.v"), None);
        apply(
            &mut f,
            &registry(),
            &Edit::Delete {
                block: "probe:probe2".into(),
            },
        )
        .unwrap();
        assert_eq!(f.controls.filters[0].source_of("b.x"), None);
        assert_eq!(f.controls.filters[0].source_of("a.x"), Some("probe1"));
    }

    #[test]
    fn the_recorder_is_a_node_added_once_and_set_on_the_canvas() {
        let mut f = file("oscillators");
        assert!(observed(&f).node(RECORDER).is_none());
        apply(&mut f, &registry(), &Edit::AddRecorder { at: [600.0, 0.0] }).unwrap();
        assert!(apply(&mut f, &registry(), &Edit::AddRecorder { at: [0.0, 0.0] }).is_err());
        let d = observed(&f);
        let r = d.node(RECORDER).unwrap();
        assert_eq!(d.nodes[r].kind, NodeKind::Recorder);
        assert_eq!(d.nodes[r].method, "every 10 steps · 256 MB");
        let set = Edit::SetRecorder {
            recorder: simsandbox_file::RecorderEntry {
                every: Some(5),
                ..Default::default()
            },
        };
        assert!(set.data_only());
        apply(&mut f, &registry(), &set).unwrap();
        let r = f.controls.recorder.clone().unwrap();
        assert_eq!((r.every, r.canvas), (Some(5), Some([600.0, 0.0])));
        apply(
            &mut f,
            &registry(),
            &Edit::Delete {
                block: RECORDER.into(),
            },
        )
        .unwrap();
        assert!(f.controls.recorder.is_none());
    }

    #[test]
    fn hidden_data_keeps_the_probes_feeding_blocks() {
        let mut f = file("oscillators");
        for _ in 0..2 {
            apply(&mut f, &registry(), &Edit::AddProbe { at: [600.0, 0.0] }).unwrap();
        }
        let bind = Edit::Bind {
            from: "a.x".into(),
            to: "probe:probe1.record".into(),
            many: true,
        };
        apply(&mut f, &registry(), &bind).unwrap();
        let feed = Edit::Connect {
            from: "probe:probe1.a.x".into(),
            to: "b.neighbor_x".into(),
        };
        apply(&mut f, &registry(), &feed).unwrap();
        let hidden = model_part(&f);
        let d = observed(&hidden);
        assert!(d.node("probe:probe1").is_some());
        assert!(d.node("probe:probe2").is_none());
        assert!(d.node("filter:energy").is_none() && d.node("view:0").is_none());
        let n = d.node("probe:probe1").unwrap();
        assert_eq!(
            d.nodes[n].settings.get("feeds").map(String::as_str),
            Some("model")
        );
    }
}
