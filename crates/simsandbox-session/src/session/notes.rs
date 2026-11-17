//! Notes on how a model's loops are solved (ADR 0040, ADR 0095), given
//! as warnings when it loads.

use simsandbox_core::sim::Simulator;

/// One note per algebraic loop (ADR 0040), saying how it is solved, and
/// one per loop of equation blocks stepped apart ([`apart_notes`]).
pub fn loop_notes(sim: &Simulator) -> Vec<String> {
    let method = sim.model().loop_method().name();
    let mut notes: Vec<String> = sim
        .loops()
        .into_iter()
        .map(|l| algebraic_note(&l, method))
        .collect();
    notes.extend(apart_notes(sim.model()));
    notes
}

/// The note of [`loop_notes`] for one algebraic loop, by its outputs.
pub(crate) fn algebraic_note(outputs: &[String], method: &str) -> String {
    format!(
        "algebraic loop {}: solved at every step by '{method}' (run.loops)",
        outputs.join(" -> ")
    )
}

/// One note per loop of equation blocks that could be solved together
/// and are not (ADR 0095): blocks wired in a circle, continuous or
/// stateless, at least one with states, not all in one fused group
/// (different methods, `fusion: off`, different clocks, an activated
/// part's edge). Each then reads the others' outputs as they were at the
/// start of the step, an error of the order of the step whatever the
/// methods' own accuracy, which their labels do not show.
pub fn apart_notes(model: &simsandbox_core::graph::Model) -> Vec<String> {
    apart_loops(model).iter().map(|l| apart_note(l)).collect()
}

/// The note of [`apart_notes`] for one loop, by its blocks' names.
pub(crate) fn apart_note(names: &[String]) -> String {
    format!(
        "{} depend on each other and are stepped apart: each reads the others' outputs \
         from the start of the step, an error of the order of run.dt that the accuracy \
         label does not show. To solve them together give them one method and leave \
         fusion on, or list them in 'groups'",
        names.join(", ")
    )
}

/// The loops [`apart_notes`] reports, each as its blocks' names.
pub(crate) fn apart_loops(model: &simsandbox_core::graph::Model) -> Vec<Vec<String>> {
    let n = model.len();
    let mut next = vec![Vec::new(); n];
    let mut prev = vec![Vec::new(); n];
    for c in model.connections() {
        let (a, b) = (c.from.block.0, c.to.block.0);
        if a != b && model.is_fusable(c.from.block) && model.is_fusable(c.to.block) {
            next[a].push(b);
            prev[b].push(a);
        }
    }
    // Strongly connected parts (Kosaraju, without recursion): blocks in
    // the order they are finished going forward, then gathered going
    // backward from the last finished.
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((v, k)) = stack.last_mut() {
            if let Some(&w) = next[*v].get(*k) {
                *k += 1;
                if !seen[w] {
                    seen[w] = true;
                    stack.push((w, 0));
                }
            } else {
                order.push(*v);
                stack.pop();
            }
        }
    }
    let mut part = vec![usize::MAX; n];
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for &start in order.iter().rev() {
        if part[start] != usize::MAX {
            continue;
        }
        let k = parts.len();
        part[start] = k;
        let mut members = vec![start];
        let mut at = 0;
        while at < members.len() {
            for &w in &prev[members[at]] {
                if part[w] == usize::MAX {
                    part[w] = k;
                    members.push(w);
                }
            }
            at += 1;
        }
        parts.push(members);
    }
    let id = simsandbox_core::graph::BlockId;
    let mut notes = Vec::new();
    for mut members in parts {
        let together = model
            .group_of(id(members[0]))
            .is_some_and(|g| members.iter().all(|&m| model.group_of(id(m)) == Some(g)));
        if members.len() < 2 || together || members.iter().all(|&m| model.is_algebraic(id(m))) {
            continue;
        }
        members.sort_unstable();
        let names: Vec<String> = members
            .iter()
            .filter_map(|&m| model.block(id(m)).ok().map(|b| b.spec().name.clone()))
            .collect();
        notes.push(names);
    }
    notes
}
