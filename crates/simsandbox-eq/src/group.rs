//! Fusion: several equation systems become one system with one integrator.
//!
//! Internal connections are resolved inside every right-hand-side
//! evaluation, so the coupled equations are solved exactly as one ODE system
//! instead of exchanging values at communication steps.
//!
//! Exposed ports and parameters are named `<member>.<name>`; inputs that are
//! not connected internally become inputs of the group.

use crate::ode::OdeSystem;
use simsandbox_core::block::{BlockSpec, Causality, Domain, Equations, PortSpec, TimeSemantics};
use simsandbox_core::{Result, SimError};

struct Member {
    sys: Box<dyn OdeSystem>,
    x_off: usize,
    p_off: usize,
}

struct Route {
    from_member: usize,
    from_offset: usize,
    to_member: usize,
    to_offset: usize,
    len: usize,
    factor: f64,
}

/// Builder for a [`FusedSystem`].
#[derive(Default)]
pub struct EquationGroup {
    name: String,
    members: Vec<Box<dyn OdeSystem>>,
    routes: Vec<Route>,
}

impl EquationGroup {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Add a member system; returns its index.
    pub fn add(&mut self, sys: impl OdeSystem + 'static) -> usize {
        self.members.push(Box::new(sys));
        self.members.len() - 1
    }

    /// Connect a member's output to another member's input (unit-checked).
    pub fn connect(&mut self, from: (usize, &str), to: (usize, &str)) -> Result<()> {
        let (fo, out) = find_port(self.member(from.0)?.spec(), Causality::Output, from.1)?;
        let (ti, inp) = find_port(self.member(to.0)?.spec(), Causality::Input, to.1)?;
        if out.shape != inp.shape {
            return Err(SimError::ShapeMismatch {
                from: format!("{:?}", out.shape),
                to: format!("{:?}", inp.shape),
            });
        }
        if !out.unit.same_as(&inp.unit) {
            return Err(SimError::UnitMismatch {
                from: out.unit.to_string(),
                to: inp.unit.to_string(),
            });
        }
        let factor = 1.0;
        self.routes.push(Route {
            from_member: from.0,
            from_offset: fo,
            to_member: to.0,
            to_offset: ti,
            len: out.shape.len(),
            factor,
        });
        Ok(())
    }

    fn member(&self, i: usize) -> Result<&dyn OdeSystem> {
        self.members
            .get(i)
            .map(|m| m.as_ref())
            .ok_or_else(|| SimError::BadConnection(format!("no member #{i}")))
    }

    /// Finish the group.
    pub fn build(self) -> Result<FusedSystem> {
        let mut spec = BlockSpec {
            name: self.name.clone(),
            domain: Domain::Equations,
            ports: Vec::new(),
            params: Vec::new(),
            states: Vec::new(),
            tables: Vec::new(),
            time: TimeSemantics::Continuous,
        };
        let mut members = Vec::new();
        let mut external = Vec::new();
        let (mut x_off, mut p_off, mut u_off) = (0, 0, 0);

        for (mi, sys) in self.members.into_iter().enumerate() {
            let s = sys.spec();
            if let Some(p) = s.outputs().find(|p| !p.feedthrough.is_empty()) {
                return Err(SimError::InvalidDefinition(format!(
                    "{}.{} has direct feedthrough, which an EquationGroup cannot order; \
                     connect the blocks in a model, where the kernel orders it (ADR 0040)",
                    s.name, p.name
                )));
            }
            let mut local_in = 0;
            for p in s.inputs() {
                let len = p.shape.len();
                let internal = self
                    .routes
                    .iter()
                    .any(|r| r.to_member == mi && r.to_offset == local_in);
                if !internal {
                    spec.ports.push(PortSpec {
                        name: format!("{}.{}", s.name, p.name),
                        ..p.clone()
                    });
                    external.push(Route {
                        from_member: usize::MAX,
                        from_offset: u_off,
                        to_member: mi,
                        to_offset: local_in,
                        len,
                        factor: 1.0,
                    });
                    u_off += len;
                }
                local_in += len;
            }
            for p in s.outputs() {
                let mut q = p.clone();
                q.name = format!("{}.{}", s.name, p.name);
                q.state = p.state.map(|o| o + x_off);
                for c in &mut q.coordinate {
                    c.port = format!("{}.{}", s.name, c.port);
                }
                spec.ports.push(q);
            }
            for p in &s.params {
                let mut q = p.clone();
                q.name = format!("{}.{}", s.name, p.name);
                spec.params.push(q);
            }
            spec.states
                .extend(s.states.iter().map(|n| format!("{}.{n}", s.name)));
            let (xl, pl) = (s.states.len(), s.params.len());
            members.push(Member { sys, x_off, p_off });
            x_off += xl;
            p_off += pl;
        }

        Ok(FusedSystem {
            spec,
            members,
            internal: self.routes,
            external,
        })
    }
}

fn find_port<'a>(
    spec: &'a BlockSpec,
    causality: Causality,
    name: &str,
) -> Result<(usize, &'a PortSpec)> {
    let mut off = 0;
    for p in spec.ports.iter().filter(|p| p.causality == causality) {
        if p.name == name {
            return Ok((off, p));
        }
        off += p.shape.len();
    }
    Err(SimError::BadConnection(format!(
        "'{}' has no {causality:?} port '{name}'",
        spec.name
    )))
}

/// Several equation systems fused into one. Itself an [`OdeSystem`], so it
/// can be wrapped in an [`OdeBlock`](crate::OdeBlock) with any integrator.
pub struct FusedSystem {
    spec: BlockSpec,
    members: Vec<Member>,
    internal: Vec<Route>,
    external: Vec<Route>,
}

impl FusedSystem {
    fn slices<'a>(&self, m: &Member, x: &'a [f64], p: &'a [f64]) -> (&'a [f64], &'a [f64]) {
        let s = m.sys.spec();
        (
            &x[m.x_off..m.x_off + s.states.len()],
            &p[m.p_off..m.p_off + s.params.len()],
        )
    }
}

impl Equations for FusedSystem {
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        // Members have no feedthrough (checked when built), so their outputs
        // depend only on state and can be evaluated first.
        let ys: Vec<Vec<f64>> = self
            .members
            .iter()
            .map(|m| {
                let (xm, pm) = self.slices(m, x, p);
                let mut y = vec![0.0; m.sys.spec().output_len()];
                m.sys.outputs(t, xm, &[], pm, &mut y);
                y
            })
            .collect();
        let mut us: Vec<Vec<f64>> = self
            .members
            .iter()
            .map(|m| vec![0.0; m.sys.spec().input_len()])
            .collect();
        for r in &self.internal {
            for k in 0..r.len {
                us[r.to_member][r.to_offset + k] = r.factor * ys[r.from_member][r.from_offset + k];
            }
        }
        for r in &self.external {
            us[r.to_member][r.to_offset..r.to_offset + r.len]
                .copy_from_slice(&u[r.from_offset..r.from_offset + r.len]);
        }
        for (mi, m) in self.members.iter().enumerate() {
            let (xm, pm) = self.slices(m, x, p);
            let n = xm.len();
            m.sys.rhs(t, xm, &us[mi], pm, &mut dx[m.x_off..m.x_off + n]);
        }
    }

    fn outputs(&self, t: f64, x: &[f64], _u: &[f64], p: &[f64], y: &mut [f64]) {
        let mut off = 0;
        for m in &self.members {
            let (xm, pm) = self.slices(m, x, p);
            let len = m.sys.spec().output_len();
            m.sys.outputs(t, xm, &[], pm, &mut y[off..off + len]);
            off += len;
        }
    }
}

impl OdeSystem for FusedSystem {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        for m in &self.members {
            let s = m.sys.spec();
            let pm = &p[m.p_off..m.p_off + s.params.len()];
            m.sys
                .initial_state(pm, &mut x0[m.x_off..m.x_off + s.states.len()]);
        }
    }
}
