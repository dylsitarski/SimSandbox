//! Direct feedthrough and algebraic loops (ADR 0040).
//!
//! An output that lists inputs in [`PortSpec::feedthrough`] depends on them
//! at the same instant. Given a set of units (Blocks, or the members of a
//! fused group) and the routes between them, an [`OutputPlan`] orders such
//! outputs so each is computed from inputs whose sources are already final,
//! and groups those that depend on each other in a cycle into loops, which
//! [`OutputPlan::solve`] solves by the model's [`LoopMethod`].
//!
//! [`PortSpec::feedthrough`]: crate::block::PortSpec::feedthrough

use crate::block::{BlockSpec, Causality};
use crate::{Result, SimError};

/// How algebraic loops are solved (`run.loops`, ADR 0040).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMethod {
    /// Newton's method on the loop's values, Jacobian by finite differences.
    #[default]
    Newton,
    /// Repeat the loop until it settles; fails when its gain exceeds one.
    FixedPoint,
    /// A model with a loop does not run.
    Refuse,
}

impl LoopMethod {
    pub const ALL: [LoopMethod; 3] = [
        LoopMethod::Newton,
        LoopMethod::FixedPoint,
        LoopMethod::Refuse,
    ];

    /// The name used in model files.
    pub fn name(self) -> &'static str {
        match self {
            LoopMethod::Newton => "newton",
            LoopMethod::FixedPoint => "fixed_point",
            LoopMethod::Refuse => "refuse",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// Computes all of a unit's outputs from its inputs, its state held:
/// `(unit, inputs, outputs)`.
pub(crate) type Eval<'a> = dyn FnMut(usize, &[f64], &mut [f64]) -> Result<()> + 'a;

/// Converged when the change is below this, relative to the values.
const TOLERANCE: f64 = 1e-10;
const NEWTON_ITERATIONS: usize = 50;
const FIXED_POINT_ITERATIONS: usize = 500;

/// A route from a unit's outputs to another's inputs (offsets within the
/// fixed arrays).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Link {
    pub from: usize,
    pub from_offset: usize,
    pub to: usize,
    pub to_offset: usize,
    pub len: usize,
    pub factor: f64,
}

/// An output port of a unit: its index among the unit's outputs and where
/// its values are.
#[derive(Debug, Clone, PartialEq)]
struct Port {
    unit: usize,
    offset: usize,
    len: usize,
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    One(Port),
    Loop(Vec<Port>),
}

/// The order feedthrough outputs are computed in, with their loops.
#[derive(Debug, Clone, Default)]
pub(crate) struct OutputPlan {
    items: Vec<Item>,
    /// Routes into each unit.
    into: Vec<Vec<Link>>,
}

impl OutputPlan {
    /// Plan for `units` (their specs, named for messages) linked by `links`.
    /// Fails if a feedthrough declaration names something that is not an
    /// input of its Block.
    pub fn new(units: &[(&BlockSpec, &str)], links: &[Link]) -> Result<Self> {
        // Nodes: outputs with feedthrough.
        let mut nodes: Vec<Port> = Vec::new();
        let mut depends: Vec<Vec<(usize, usize)>> = Vec::new(); // (unit, input offset..len) pairs
        for (u, (spec, name)) in units.iter().enumerate() {
            let mut offset = 0;
            for p in spec.outputs() {
                let len = p.shape.len();
                if !p.feedthrough.is_empty() {
                    if p.shape.is_variable() {
                        return Err(SimError::InvalidDefinition(format!(
                            "{name}.{}: populations cannot have direct feedthrough yet",
                            p.name
                        )));
                    }
                    let mut deps = Vec::new();
                    for i in &p.feedthrough {
                        let (off, n) = input_range(spec, i).ok_or_else(|| {
                            SimError::InvalidDefinition(format!(
                                "{name}.{} depends on '{i}', which is not an input of '{name}'",
                                p.name
                            ))
                        })?;
                        deps.push((off, n));
                    }
                    nodes.push(Port {
                        unit: u,
                        offset,
                        len,
                        name: format!("{name}.{}", p.name),
                    });
                    depends.push(deps);
                }
                offset += len;
            }
        }
        // Edges: P -> Q when a route from P's values reaches an input Q
        // depends on.
        let overlaps = |a: usize, an: usize, b: usize, bn: usize| a < b + bn && b < a + an;
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
        for (q, node) in nodes.iter().enumerate() {
            for &(off, n) in &depends[q] {
                for l in links.iter().filter(|l| l.to == node.unit) {
                    if !overlaps(l.to_offset, l.len, off, n) {
                        continue;
                    }
                    for (p, src) in nodes.iter().enumerate() {
                        if src.unit == l.from && overlaps(src.offset, src.len, l.from_offset, l.len)
                        {
                            edges[p].push(q);
                        }
                    }
                }
            }
        }
        let items = components(&edges)
            .into_iter()
            .map(|c| {
                let looped = c.len() > 1 || edges[c[0]].contains(&c[0]);
                if looped {
                    Item::Loop(c.into_iter().map(|i| nodes[i].clone()).collect())
                } else {
                    Item::One(nodes[c[0]].clone())
                }
            })
            .collect();
        let mut into = vec![Vec::new(); units.len()];
        for l in links {
            into[l.to].push(*l);
        }
        Ok(Self { items, into })
    }

    /// The loops, each as the names of its outputs (`block.port`).
    pub fn loops(&self) -> Vec<Vec<String>> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Loop(ports) => Some(ports.iter().map(|p| p.name.clone()).collect()),
                Item::One(_) => None,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Make the feedthrough outputs consistent with their inputs. Other
    /// outputs must already be final. `eval(unit, inputs, outputs)`
    /// computes all of a unit's outputs from its inputs (its state held).
    pub fn solve(
        &self,
        method: LoopMethod,
        outputs: &mut [Vec<f64>],
        inputs: &mut [Vec<f64>],
        eval: &mut Eval<'_>,
    ) -> Result<()> {
        let mut scratch = Vec::new();
        for item in &self.items {
            match item {
                Item::One(p) => {
                    self.compute(p, outputs, inputs, eval, &mut scratch)?;
                    outputs[p.unit][p.offset..p.offset + p.len]
                        .copy_from_slice(&scratch[p.offset..p.offset + p.len]);
                }
                Item::Loop(ports) => self.solve_loop(method, ports, outputs, inputs, eval)?,
            }
        }
        Ok(())
    }

    /// Route a unit's inputs from the current outputs and evaluate it into
    /// `scratch`.
    fn compute(
        &self,
        p: &Port,
        outputs: &[Vec<f64>],
        inputs: &mut [Vec<f64>],
        eval: &mut Eval<'_>,
        scratch: &mut Vec<f64>,
    ) -> Result<()> {
        for l in &self.into[p.unit] {
            for k in 0..l.len {
                inputs[p.unit][l.to_offset + k] = l.factor * outputs[l.from][l.from_offset + k];
            }
        }
        scratch.clear();
        scratch.resize(outputs[p.unit].len(), 0.0);
        eval(p.unit, &inputs[p.unit], scratch)
    }

    /// The loop's values after one pass: every port computed from `v`.
    fn pass(
        &self,
        ports: &[Port],
        v: &[f64],
        outputs: &mut [Vec<f64>],
        inputs: &mut [Vec<f64>],
        eval: &mut Eval<'_>,
    ) -> Result<Vec<f64>> {
        scatter(ports, v, outputs);
        let mut out = Vec::with_capacity(v.len());
        let mut scratch = Vec::new();
        for p in ports {
            self.compute(p, outputs, inputs, eval, &mut scratch)?;
            out.extend_from_slice(&scratch[p.offset..p.offset + p.len]);
        }
        Ok(out)
    }

    fn solve_loop(
        &self,
        method: LoopMethod,
        ports: &[Port],
        outputs: &mut [Vec<f64>],
        inputs: &mut [Vec<f64>],
        eval: &mut Eval<'_>,
    ) -> Result<()> {
        let names = || {
            ports
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" → ")
        };
        let mut v: Vec<f64> = ports
            .iter()
            .flat_map(|p| outputs[p.unit][p.offset..p.offset + p.len].iter().copied())
            .collect();
        let converged = |v: &[f64], d: &[f64]| {
            let scale = v.iter().fold(1.0f64, |m, x| m.max(x.abs()));
            d.iter().all(|x| x.abs() <= TOLERANCE * scale)
        };
        let fail = |what: String| SimError::AlgebraicLoop(format!("the loop {} {what}", names()));
        match method {
            LoopMethod::Refuse => return Err(fail("is not solved (run.loops is 'refuse')".into())),
            LoopMethod::FixedPoint => {
                for _ in 0..FIXED_POINT_ITERATIONS {
                    let g = self.pass(ports, &v, outputs, inputs, eval)?;
                    let d: Vec<f64> = g.iter().zip(&v).map(|(a, b)| a - b).collect();
                    v = g;
                    if !v.iter().all(|x| x.is_finite()) {
                        break;
                    }
                    if converged(&v, &d) {
                        scatter(ports, &v, outputs);
                        return Ok(());
                    }
                }
                return Err(fail(format!(
                    "did not settle in {FIXED_POINT_ITERATIONS} passes by fixed-point \
                     iteration; try run.loops 'newton'"
                )));
            }
            LoopMethod::Newton => {}
        }
        let n = v.len();
        for _ in 0..NEWTON_ITERATIONS {
            let g = self.pass(ports, &v, outputs, inputs, eval)?;
            let r: Vec<f64> = g.iter().zip(&v).map(|(a, b)| a - b).collect();
            if converged(&v, &r) {
                v = g;
                scatter(ports, &v, outputs);
                return Ok(());
            }
            // J = dG/dv − I by forward differences; solve J Δ = −r.
            let mut j = vec![0.0; n * n];
            for c in 0..n {
                let h = 1e-7 * v[c].abs().max(1.0);
                let mut w = v.clone();
                w[c] += h;
                let gh = self.pass(ports, &w, outputs, inputs, eval)?;
                for row in 0..n {
                    j[row * n + c] = (gh[row] - g[row]) / h - if row == c { 1.0 } else { 0.0 };
                }
            }
            let rhs: Vec<f64> = r.iter().map(|x| -x).collect();
            let delta = gauss(&mut j, rhs, n)
                .ok_or_else(|| fail("has no unique solution here (singular Jacobian)".into()))?;
            for (x, d) in v.iter_mut().zip(&delta) {
                *x += d;
            }
            if !v.iter().all(|x| x.is_finite()) {
                break;
            }
        }
        Err(fail(format!(
            "did not converge in {NEWTON_ITERATIONS} Newton iterations"
        )))
    }
}

fn scatter(ports: &[Port], v: &[f64], outputs: &mut [Vec<f64>]) {
    let mut k = 0;
    for p in ports {
        outputs[p.unit][p.offset..p.offset + p.len].copy_from_slice(&v[k..k + p.len]);
        k += p.len;
    }
}

/// Offset and length of a named input in a Block's fixed input array.
fn input_range(spec: &BlockSpec, name: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for p in spec
        .ports
        .iter()
        .filter(|p| p.causality == Causality::Input)
    {
        if p.name == name {
            return (!p.shape.is_variable()).then_some((offset, p.shape.len()));
        }
        offset += p.shape.len();
    }
    None
}

/// Solve the dense system `a x = b` (n × n, row-major) by Gaussian
/// elimination with partial pivoting; `None` if singular.
fn gauss(a: &mut [f64], mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let pivot =
            (col..n).max_by(|&i, &k| a[i * n + col].abs().total_cmp(&a[k * n + col].abs()))?;
        if a[pivot * n + col].abs() < 1e-300 {
            return None;
        }
        if pivot != col {
            for c in 0..n {
                a.swap(pivot * n + c, col * n + c);
            }
            b.swap(pivot, col);
        }
        for row in col + 1..n {
            let f = a[row * n + col] / a[col * n + col];
            for c in col..n {
                a[row * n + c] -= f * a[col * n + c];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|c| a[row * n + c] * x[c]).sum();
        x[row] = (b[row] - s) / a[row * n + row];
    }
    Some(x)
}

/// Strongly connected components (Tarjan), sources first: every edge goes
/// from a component to itself or to a later one.
fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(s: &mut State<'_>, v: usize) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for k in 0..s.edges[v].len() {
            let w = s.edges[v][k];
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(i) if s.on_stack[w] => s.low[v] = s.low[v].min(i),
                Some(_) => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut c = Vec::new();
            loop {
                let w = s.stack.pop().expect("on the stack");
                s.on_stack[w] = false;
                c.push(w);
                if w == v {
                    break;
                }
            }
            c.sort_unstable();
            s.out.push(c);
        }
    }
    let n = edges.len();
    let mut s = State {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    // Tarjan finishes sinks first.
    s.out.reverse();
    s.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_come_sources_first_with_cycles_together() {
        // 0 -> 1 -> 2 -> 1, 2 -> 3, 4 alone, 5 -> 5.
        let edges = vec![vec![1], vec![2], vec![1, 3], vec![], vec![], vec![5]];
        let c = components(&edges);
        let pos = |v: usize| c.iter().position(|x| x.contains(&v)).unwrap();
        assert!(c.contains(&vec![1, 2]));
        assert!(pos(0) < pos(1) && pos(1) < pos(3));
        assert_eq!(c.iter().map(Vec::len).sum::<usize>(), 6);
    }

    #[test]
    fn feedthrough_must_name_an_input() {
        use crate::block::{BlockSpec, Domain, PortSpec, Shape, TimeSemantics};
        use crate::units::Unit;
        let spec = BlockSpec {
            name: "b".into(),
            domain: Domain::Equations,
            ports: vec![
                PortSpec::input("u", Unit::DIMENSIONLESS, Shape::Scalar),
                PortSpec::output("y", Unit::DIMENSIONLESS, Shape::Scalar).depends_on(&["v"]),
            ],
            params: vec![],
            states: vec![],
            tables: vec![],
            time: TimeSemantics::Stateless,
        };
        let err = OutputPlan::new(&[(&spec, "b")], &[]).unwrap_err();
        assert!(err.to_string().contains("b.y depends on 'v'"), "{err}");
    }

    #[test]
    fn gaussian_elimination_solves_and_detects_singularity() {
        let mut a = vec![0.0, 2.0, 1.0, 1.0];
        let x = gauss(&mut a, vec![4.0, 3.0], 2).unwrap();
        assert_eq!(x, [1.0, 2.0]);
        let mut s = vec![1.0, 2.0, 2.0, 4.0];
        assert!(gauss(&mut s, vec![1.0, 2.0], 2).is_none());
    }
}
