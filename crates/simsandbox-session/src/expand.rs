//! Uses expanded in place on the diagram (ADR 0066 step 4d): a used model's
//! inner nodes drawn inside an outline where its node was, one level each,
//! wires to its ports drawn to the inner nodes they reach, neighbours
//! pushed aside on screen only. Edits made inside an outline are edits of
//! that model ([`route`]), which take the edit question as when it is
//! opened.

use crate::diagram::{Diagram, Edit, NodeKind, Port, Wire, HEADER, NODE_WIDTH};
use simsandbox_core::block::Shape;
use simsandbox_core::registry::Registry;
use simsandbox_file::ModelFile;
use std::path::Path;

/// Space around an expanded model's nodes inside its outline.
const PAD: f64 = 24.0;

/// An expanded use: where it is drawn and what it says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    /// Its path of uses (`car1`, `vehicle/plant`).
    pub path: String,
    /// What it uses: its file, or `model NAME`.
    pub label: String,
    /// Its clock as the use node says it ("shared clock", "every 0.05 s").
    pub clock: String,
    /// `[min_x, min_y, max_x, max_y]` in canvas units.
    pub rect: [f64; 4],
    /// What was added to its nodes' places, written in its own model.
    pub shift: [f64; 2],
}

impl Diagram {
    /// Expand the uses at `paths` (by path from the model, one level each:
    /// `vehicle/plant` only inside an expanded `vehicle`), `file` being the
    /// model drawn, saved in its folder.
    pub fn expand(&mut self, file: &ModelFile, registry: &Registry, paths: &[String]) {
        let folder = file
            .folder
            .clone()
            .or_else(simsandbox_core::registry::current_folder);
        expand_in(self, file, folder.as_deref(), registry, paths, &[]);
    }
}

fn expand_in(
    d: &mut Diagram,
    top: &ModelFile,
    folder: Option<&Path>,
    registry: &Registry,
    paths: &[String],
    chain: &[String],
) {
    loop {
        // One use at a time: indices change as each is spliced in.
        let Some(i) = d.nodes.iter().position(|n| {
            n.kind == NodeKind::Use && {
                let mut p = chain.to_vec();
                p.push(n.name.clone());
                paths.contains(&p.join("/"))
            }
        }) else {
            return;
        };
        let name = d.nodes[i].name.clone();
        let mut path = chain.to_vec();
        path.push(name.clone());
        let Ok(inner) = crate::edit_use::model_at(top, folder, &path) else {
            // Not readable: it stays a node, with its error.
            return;
        };
        let mut sub = Diagram::new(&inner, registry);
        expand_in(&mut sub, top, folder, registry, paths, &path);
        splice(d, i, sub, &path.join("/"));
    }
}

/// Put `sub` (the inside of the use node `i`, at `full` path) in place of
/// node `i`.
fn splice(d: &mut Diagram, i: usize, mut sub: Diagram, full: &str) {
    let used = d.nodes[i].clone();
    let name = used.name.clone();
    let (min, size) = match sub.bounds() {
        Some(b) => ([b[0], b[1]], [b[2] - b[0], b[3] - b[1]]),
        None => ([0.0, 0.0], [NODE_WIDTH, HEADER]),
    };
    let origin = [used.at[0] + PAD, used.at[1] + HEADER + PAD / 2.0];
    let shift = [origin[0] - min[0], origin[1] - min[1]];
    let w = size[0] + 2.0 * PAD;
    let h = size[1] + HEADER + PAD * 1.5;
    // Neighbours make room, on screen only.
    let (dx, dy) = ((w - NODE_WIDTH).max(0.0), (h - used.height()).max(0.0));
    let right = used.at[0] + NODE_WIDTH - 1.0;
    let below = used.at[1] + used.height() - 1.0;
    let mut moved: Vec<(usize, [f64; 2])> = Vec::new();
    for (j, n) in d.nodes.iter().enumerate() {
        if j == i {
            continue;
        }
        let by = if n.at[0] >= right {
            [dx, 0.0]
        } else if n.at[1] >= below && n.at[0] + NODE_WIDTH > used.at[0] {
            [0.0, dy]
        } else {
            continue;
        };
        moved.push((j, by));
    }
    for (j, by) in moved {
        let n = &mut d.nodes[j];
        n.at = [n.at[0] + by[0], n.at[1] + by[1]];
        let s = d.shifted.entry(n.name.clone()).or_insert([0.0, 0.0]);
        *s = [s[0] + by[0], s[1] + by[1]];
    }
    for o in &mut d.outlines {
        if o.rect[0] >= right {
            o.rect[0] += dx;
            o.rect[2] += dx;
            o.shift[0] += dx;
        } else if o.rect[1] >= below {
            o.rect[1] += dy;
            o.rect[3] += dy;
            o.shift[1] += dy;
        }
    }
    // Its nodes, named by path, at their places shifted into the outline.
    let base = d.nodes.len();
    let prefix = |s: &str| format!("{name}/{s}");
    for mut n in std::mem::take(&mut sub.nodes) {
        n.name = prefix(&n.name);
        n.at = [n.at[0] + shift[0], n.at[1] + shift[1]];
        for v in &mut n.values {
            v.key = prefix(&v.key);
        }
        d.nodes.push(n);
    }
    for (k, v) in std::mem::take(&mut sub.exposed) {
        if let Some((kind, rest)) = k.split_once(':') {
            d.exposed.insert(format!("{kind}:{}", prefix(rest)), v);
        }
    }
    for w0 in std::mem::take(&mut sub.wires) {
        d.wires.push(Wire {
            from: (w0.from.0 + base, w0.from.1),
            to: (w0.to.0 + base, w0.to.1),
            from_name: prefix(&w0.from_name),
            to_name: prefix(&w0.to_name),
            signal: w0.signal,
            param: w0.param,
        });
    }
    // Outlines inside it carry their full paths already.
    for mut o in std::mem::take(&mut sub.outlines) {
        o.rect = [
            o.rect[0] + shift[0],
            o.rect[1] + shift[1],
            o.rect[2] + shift[0],
            o.rect[3] + shift[1],
        ];
        o.shift = [o.shift[0] + shift[0], o.shift[1] + shift[1]];
        d.outlines.push(o);
    }
    d.outlines.push(Outline {
        path: full.to_string(),
        label: used.type_name.clone(),
        clock: used.method.clone(),
        rect: [used.at[0], used.at[1], used.at[0] + w, used.at[1] + h],
        shift,
    });
    // Wires to its ports: to the inner node each reaches; an exposed
    // input (ADR 0074) to every port it feeds.
    let inner = |d: &mut Diagram, input: bool, port: &str| -> Vec<(usize, usize)> {
        if input {
            let head = format!("in:{name}/");
            let fed: Vec<String> = d
                .exposed
                .iter()
                .filter(|(k, v)| k.starts_with(&head) && *v == port)
                .map(|(k, _)| k[head.len()..].to_string())
                .collect();
            if !fed.is_empty() {
                return fed
                    .iter()
                    .filter_map(|t| find(d, base, &name, t, true))
                    .collect();
            }
        }
        let rel = if input {
            port.to_string()
        } else {
            used.settings
                .get(&format!("output.{port}"))
                .cloned()
                .unwrap_or_else(|| port.to_string())
        };
        find(d, base, &name, &rel, input).into_iter().collect()
    };
    let mut keep = Vec::new();
    for w0 in std::mem::take(&mut d.wires) {
        let froms = if w0.from.0 == i {
            let port = used
                .outputs
                .get(w0.from.1)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            inner(d, false, &port)
        } else {
            vec![w0.from]
        };
        let tos = if w0.to.0 == i && w0.param {
            // A driven parameter of the use: the inner values exposed as
            // it (ADR 0075).
            let port = used
                .values
                .get(w0.to.1)
                .map(|v| v.name.clone())
                .unwrap_or_default();
            (base..d.nodes.len())
                .flat_map(|k| {
                    d.nodes[k]
                        .values
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| v.exposed.as_deref() == Some(port.as_str()))
                        .map(move |(j, _)| (k, j))
                        .collect::<Vec<_>>()
                })
                .collect()
        } else if w0.to.0 == i {
            let port = used
                .inputs
                .get(w0.to.1)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            inner(d, true, &port)
        } else {
            vec![w0.to]
        };
        for &from in froms.iter().take(1) {
            for &to in &tos {
                keep.push(Wire {
                    from,
                    to,
                    ..w0.clone()
                });
            }
        }
    }
    d.wires = keep;
    crate::diagram::drive_values(&mut d.nodes, &mut d.wires);
    // The use's node goes; indices above it move down one.
    d.nodes.remove(i);
    let down = |k: usize| if k > i { k - 1 } else { k };
    for w0 in &mut d.wires {
        w0.from.0 = down(w0.from.0);
        w0.to.0 = down(w0.to.0);
    }
}

/// The inner node and port that `rel` (relative to the use `name`) names
/// among the spliced nodes: an input block for an input (given an input
/// port to draw the wire to), `block.port` or a path into a use for an
/// output.
fn find(
    d: &mut Diagram,
    base: usize,
    name: &str,
    rel: &str,
    input: bool,
) -> Option<(usize, usize)> {
    let node = |d: &Diagram, n: &str| {
        (base..d.nodes.len()).find(|&k| d.nodes[k].name == format!("{name}/{n}"))
    };
    if input {
        // `force` (an input block), or a path into a use inside.
        if let Some(k) = node(d, rel) {
            if d.nodes[k].kind == NodeKind::Block {
                if d.nodes[k].inputs.is_empty() {
                    let unit = d.nodes[k]
                        .outputs
                        .first()
                        .map(|p| p.unit.clone())
                        .unwrap_or_default();
                    d.nodes[k].inputs.push(Port {
                        name: "in".into(),
                        unit,
                        shape: Shape::Scalar,
                    });
                }
                return Some((k, 0));
            }
        }
    }
    if let Some((first, rest)) = rel.split_once('/') {
        if let Some(k) = node(d, first).filter(|&k| d.nodes[k].kind == NodeKind::Use) {
            let ports = if input {
                &d.nodes[k].inputs
            } else {
                &d.nodes[k].outputs
            };
            return ports.iter().position(|p| p.name == rest).map(|p| (k, p));
        }
        // Expanded further: its inner nodes carry the longer name.
        return find(d, base, &format!("{name}/{first}"), rest, input);
    }
    let (b, p) = rel.rsplit_once('.')?;
    let k = node(d, b)?;
    let ports = if input {
        &d.nodes[k].inputs
    } else {
        &d.nodes[k].outputs
    };
    ports.iter().position(|q| q.name == p).map(|q| (k, q))
}

/// Where an edit made on the expanded diagram goes: the model drawn
/// (`None`), or the used model at a path, with the edit in its own names
/// and places. An edit reaching across an outline is refused.
pub fn route(d: &Diagram, edit: &Edit) -> Result<(Option<Vec<String>>, Edit), String> {
    // The innermost outline a name or a place is in.
    let by_name = |n: &str| {
        d.outlines
            .iter()
            .filter(|o| n.starts_with(&format!("{}/", o.path)))
            .max_by_key(|o| o.path.len())
    };
    let by_place = |at: &[f64; 2]| {
        d.outlines
            .iter()
            .filter(|o| {
                at[0] >= o.rect[0] && at[0] <= o.rect[2] && at[1] >= o.rect[1] && at[1] <= o.rect[3]
            })
            .max_by_key(|o| o.path.len())
    };
    let strip = |o: &Outline, n: &str| n[o.path.len() + 1..].to_string();
    let inside = |o: &Outline| o.path.split('/').map(str::to_string).collect::<Vec<_>>();
    let unshift = |o: &Outline, at: &[f64; 2]| [at[0] - o.shift[0], at[1] - o.shift[1]];
    let one = |names: &[&str]| -> Result<Option<&Outline>, String> {
        let found: Vec<Option<&Outline>> = names.iter().map(|n| by_name(n)).collect();
        let first = found.first().copied().flatten();
        if found
            .iter()
            .all(|o| o.map(|o| &o.path) == first.map(|o| &o.path))
        {
            Ok(first)
        } else {
            Err("this reaches into an expanded model from outside it: open it, or wire to its ports".into())
        }
    };
    let top = |e: &Edit| Ok((None, e.clone()));
    match edit {
        Edit::Move { block, at } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::Move {
                    block: strip(o, block),
                    at: unshift(o, at),
                },
            )),
            None => {
                // A node pushed aside keeps its own place.
                let s = d.shifted.get(block).copied().unwrap_or([0.0, 0.0]);
                Ok((
                    None,
                    Edit::Move {
                        block: block.clone(),
                        at: [at[0] - s[0], at[1] - s[1]],
                    },
                ))
            }
        },
        Edit::Add { type_name, at } => match by_place(at) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::Add {
                    type_name: type_name.clone(),
                    at: unshift(o, at),
                },
            )),
            None => top(edit),
        },
        Edit::Delete { block } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::Delete {
                    block: strip(o, block),
                },
            )),
            None => top(edit),
        },
        Edit::Rename { from, to } => match by_name(from) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::Rename {
                    from: strip(o, from),
                    to: to.rsplit('/').next().unwrap_or(to).to_string(),
                },
            )),
            None => top(edit),
        },
        Edit::Connect { from, to } => match one(&[from, to]) {
            // From outside to an inner port the model exposes, or from one
            // it exposes to outside (ADR 0074): a wire to or from the use's
            // port, at the top level.
            Err(e) => {
                let top_port = |kind: &str, end: &str| -> Option<String> {
                    let o = by_name(end).filter(|o| !o.path.contains('/'))?;
                    let name = d.exposed.get(&format!("{kind}:{end}"))?;
                    Some(format!("{}/{name}", o.path))
                };
                let (f, t) = (by_name(from).is_some(), by_name(to).is_some());
                let wire = match (f, t) {
                    (false, true) => top_port("in", to).map(|to| (from.clone(), to)),
                    (true, false) => top_port("out", from).map(|from| (from, to.clone())),
                    _ => None,
                };
                match wire {
                    Some((from, to)) => Ok((None, Edit::Connect { from, to })),
                    None => Err(format!(
                        "{e}; or click the inner port to expose it, then wire to it"
                    )),
                }
            }
            Ok(found) => match found {
                Some(o) => Ok((
                    Some(inside(o)),
                    Edit::Connect {
                        from: strip(o, from),
                        to: strip(o, to),
                    },
                )),
                None => top(edit),
            },
        },
        Edit::Disconnect { to } => match by_name(to) {
            // An exposed input of a model expanded at the top level is fed
            // there, through the use's port (ADR 0074).
            Some(o) if !o.path.contains('/') && d.exposed.contains_key(&format!("in:{to}")) => {
                Ok((
                    None,
                    Edit::Disconnect {
                        to: format!("{}/{}", o.path, d.exposed[&format!("in:{to}")]),
                    },
                ))
            }
            Some(o) => Ok((Some(inside(o)), Edit::Disconnect { to: strip(o, to) })),
            None => top(edit),
        },
        Edit::SetParam {
            block,
            param,
            value,
        } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::SetParam {
                    block: strip(o, block),
                    param: param.clone(),
                    value: *value,
                },
            )),
            None => top(edit),
        },
        Edit::SetFormula {
            block,
            param,
            formula,
        } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::SetFormula {
                    block: strip(o, block),
                    param: param.clone(),
                    formula: formula.clone(),
                },
            )),
            None => top(edit),
        },
        Edit::SetSetting { block, key, text } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::SetSetting {
                    block: strip(o, block),
                    key: key.clone(),
                    text: text.clone(),
                },
            )),
            None => top(edit),
        },
        Edit::GroupModel { blocks } => {
            let names: Vec<&str> = blocks.iter().map(String::as_str).collect();
            match one(&names)? {
                Some(o) => Ok((
                    Some(inside(o)),
                    Edit::GroupModel {
                        blocks: blocks.iter().map(|b| strip(o, b)).collect(),
                    },
                )),
                None => top(edit),
            }
        }
        Edit::Duplicate { blocks } => {
            let names: Vec<&str> = blocks.iter().map(String::as_str).collect();
            match one(&names)? {
                Some(o) => Ok((
                    Some(inside(o)),
                    Edit::Duplicate {
                        blocks: blocks.iter().map(|b| strip(o, b)).collect(),
                    },
                )),
                None => top(edit),
            }
        }
        Edit::ExposeParam { block, param } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::ExposeParam {
                    block: strip(o, block),
                    param: param.clone(),
                },
            )),
            None => top(edit),
        },
        Edit::HideParam { block, param } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::HideParam {
                    block: strip(o, block),
                    param: param.clone(),
                },
            )),
            None => top(edit),
        },
        Edit::ExposeInput { input } => match by_name(input) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::ExposeInput {
                    input: strip(o, input),
                },
            )),
            None => top(edit),
        },
        Edit::HideInput { input } => match by_name(input) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::HideInput {
                    input: strip(o, input),
                },
            )),
            None => top(edit),
        },
        Edit::DeclareOutput { name, from } => match by_name(from) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::DeclareOutput {
                    name: name.clone(),
                    from: strip(o, from),
                },
            )),
            None => top(edit),
        },
        Edit::HideOutput { from } => match by_name(from) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::HideOutput {
                    from: strip(o, from),
                },
            )),
            None => top(edit),
        },
        Edit::SetEquations { block, equations } => match by_name(block) {
            Some(o) => Ok((
                Some(inside(o)),
                Edit::SetEquations {
                    block: strip(o, block),
                    equations: equations.clone(),
                },
            )),
            None => top(edit),
        },
        _ => top(edit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn platoon() -> ModelFile {
        let text = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap()
            .text;
        ModelFile::from_json(text).unwrap().0
    }

    #[test]
    fn an_expanded_use_shows_its_nodes_in_an_outline_wired_through() {
        let r = registry();
        let f = platoon();
        let before = Diagram::new(&f, &r);
        let car2_at = before.nodes[before.node("car2").unwrap()].at;
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["car1".into()]);
        // car1 is gone as a node; its blocks are inside.
        assert!(d.node("car1").is_none());
        for n in ["car1/car", "car1/control"] {
            assert!(d.node(n).is_some(), "{n}");
        }
        let o = &d.outlines[0];
        assert_eq!(
            (o.path.as_str(), o.label.as_str()),
            ("car1", "model follower")
        );
        assert!(o.clock.contains("shared"));
        // Inner nodes lie inside the outline.
        for k in ["car1/car", "car1/control"] {
            let at = d.nodes[d.node(k).unwrap()].at;
            assert!(
                at[0] >= o.rect[0] && at[0] + NODE_WIDTH <= o.rect[2] && at[1] >= o.rect[1],
                "{k}"
            );
        }
        // The leader's wire goes to the exposed input it feeds (ADR 0074);
        // car2 reads the car's v.
        let named = |w: &Wire| (d.nodes[w.from.0].name.clone(), d.nodes[w.to.0].name.clone());
        let wires: Vec<(String, String)> = d.wires.iter().map(named).collect();
        assert!(
            wires.contains(&("leader".into(), "car1/control".into())),
            "{wires:?}"
        );
        assert!(
            wires.contains(&("car1/car".into(), "car2".into())),
            "{wires:?}"
        );
        assert!(
            wires.contains(&("car1/control".into(), "car1/car".into())),
            "{wires:?}"
        );
        // Neighbours make room on screen, and remember by how much.
        let car2 = &d.nodes[d.node("car2").unwrap()];
        assert!(car2.at[0] > car2_at[0]);
        assert_eq!(car2.at[0] - d.shifted["car2"][0], car2_at[0]);
        // Collapsed again, nothing has moved.
        assert_eq!(Diagram::new(&f, &r), before);
    }

    #[test]
    fn wires_across_an_outline_reach_the_ports_it_exposes() {
        let r = registry();
        let f = platoon();
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["car1".into()]);
        let connect = |from: &str, to: &str| {
            route(
                &d,
                &Edit::Connect {
                    from: from.into(),
                    to: to.into(),
                },
            )
        };
        // To an exposed input, and from an exposed output: the use's ports.
        assert_eq!(
            connect("leader.v", "car1/control.lead").unwrap(),
            (
                None,
                Edit::Connect {
                    from: "leader.v".into(),
                    to: "car1/lead".into()
                }
            )
        );
        assert_eq!(
            connect("car1/car.v", "car3/lead").unwrap().1,
            Edit::Connect {
                from: "car1/v".into(),
                to: "car3/lead".into()
            }
        );
        // An input not exposed is refused, saying how to expose it.
        let e = connect("leader.v", "car1/car.F").unwrap_err();
        assert!(e.contains("expose"), "{e}");
        // Lifting the wire into an exposed input disconnects the use's port.
        assert_eq!(
            route(
                &d,
                &Edit::Disconnect {
                    to: "car1/control.lead".into()
                }
            )
            .unwrap(),
            (
                None,
                Edit::Disconnect {
                    to: "car1/lead".into()
                }
            )
        );
    }

    #[test]
    fn edits_inside_an_outline_go_to_the_used_model() {
        let r = registry();
        let f = platoon();
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["car1".into()]);
        let o = d.outlines[0].clone();
        let at = d.nodes[d.node("car1/car").unwrap()].at;
        let (to, e) = route(
            &d,
            &Edit::Move {
                block: "car1/car".into(),
                at,
            },
        )
        .unwrap();
        assert_eq!(to, Some(vec!["car1".to_string()]));
        assert_eq!(
            e,
            Edit::Move {
                block: "car".into(),
                at: [at[0] - o.shift[0], at[1] - o.shift[1]]
            }
        );
        let (to, e) = route(
            &d,
            &Edit::Connect {
                from: "car1/car.v".into(),
                to: "car1/control.v".into(),
            },
        )
        .unwrap();
        assert_eq!(to, Some(vec!["car1".to_string()]));
        assert_eq!(
            e,
            Edit::Connect {
                from: "car.v".into(),
                to: "control.v".into()
            }
        );
        // A block added inside the outline goes inside.
        let (to, _) = route(
            &d,
            &Edit::Add {
                type_name: "eq.equations".into(),
                at: [o.rect[0] + 5.0, o.rect[1] + 5.0],
            },
        )
        .unwrap();
        assert!(to.is_some());
        // Reaching across the outline is refused; outside stays outside.
        assert!(route(
            &d,
            &Edit::Connect {
                from: "leader.v".into(),
                to: "car1/control.v".into()
            }
        )
        .is_err());
        let s = d.shifted["car2"];
        let (to, e) = route(
            &d,
            &Edit::Move {
                block: "car2".into(),
                at: [500.0, 100.0],
            },
        )
        .unwrap();
        assert_eq!(to, None);
        assert_eq!(
            e,
            Edit::Move {
                block: "car2".into(),
                at: [500.0 - s[0], 100.0 - s[1]]
            }
        );
    }

    #[test]
    fn nested_uses_expand_one_level_at_a_time() {
        let r = registry();
        let mut f = platoon();
        // A model using the follower: expanded, its car stays a node until
        // it is expanded too.
        f.models.insert(
            "pair".into(),
            serde_json::json!({"blocks": [{"name": "lead", "type": "input.value"}, {"name": "one", "model": "follower"}],
                               "connections": [{"from": "lead.y", "to": "one/lead"}]}),
        );
        f.blocks.push(simsandbox_file::BlockEntry {
            name: "p".into(),
            model: Some("pair".into()),
            ..Default::default()
        });
        f.resolve_models().unwrap();
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["p".into()]);
        assert_eq!(d.nodes[d.node("p/one").unwrap()].kind, NodeKind::Use);
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["p".into(), "p/one".into()]);
        assert!(d.node("p/one").is_none());
        assert!(d.node("p/one/car").is_some());
        let paths: Vec<&str> = d.outlines.iter().map(|o| o.path.as_str()).collect();
        assert_eq!(paths, ["p/one", "p"]);
        // An edit on the inner car goes to the model inside.
        let at = d.nodes[d.node("p/one/car").unwrap()].at;
        let (to, _) = route(
            &d,
            &Edit::Move {
                block: "p/one/car".into(),
                at,
            },
        )
        .unwrap();
        assert_eq!(to, Some(vec!["p".to_string(), "one".to_string()]));
        // `p/one` alone, without `p`, expands nothing.
        let mut d = Diagram::new(&f, &r);
        d.expand(&f, &r, &["p/one".into()]);
        assert!(d.outlines.is_empty());
    }
}
