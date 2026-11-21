//! The mechanism block (ADR 0103 point 1, ADR 0105): a tree of bodies from
//! its `bodies` rows, every number a parameter, solved as an ODE in its
//! joint coordinates by the equation domain's methods.

use crate::rows::{self, Row};
use crate::tree::{self, At, Kind, Load, Numbers, Pose, Scratch, Tree};
use simsandbox_core::block::{
    Block, BlockSpec, Domain, Equations, MethodSpec, ParamSpec, PortSpec, Shape, StepContext,
    StepIo,
};
use simsandbox_core::registry::BlockConfig;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_eq::{Integrator, OdeBlock, OdeSystem};
use std::cell::RefCell;
use std::f64::consts::PI;

/// The block type (ADR 0105 point 1).
pub const PLANAR: &str = "mech.planar";

/// The default of gravity's parameter `g` (m/s²), acting along −y.
pub const STANDARD_GRAVITY: f64 = 9.81;

/// The rows outputs, in port order: each body's frame in the world.
pub const POSES: [(&str, &str); 6] = [
    ("x", "m"),
    ("y", "m"),
    ("angle", "rad"),
    ("vx", "m/s"),
    ("vy", "m/s"),
    ("omega", "rad/s"),
];

fn unit(text: &str) -> Unit {
    Unit::parse(text).expect("the mechanism's units parse")
}

/// Where one row's numbers are among the block's parameters.
#[derive(Clone, Copy, Debug)]
struct Place {
    mass: usize,
    inertia: usize,
    centre: usize,
    at: usize,
    turn: usize,
    /// Damping, stiffness, rest, then the start's coordinate and rate;
    /// `None` for a fixed joint.
    spring: Option<usize>,
    /// The joint's force or torque (ADR 0106); `None` for a fixed joint.
    force: Option<usize>,
    /// The body's load: force x, force y, torque (ADR 0106).
    load: usize,
}

/// What [`Equations`] work in, kept between evaluations.
#[derive(Default)]
struct Work {
    nums: Vec<Numbers>,
    tau: Vec<f64>,
    loads: Vec<Load>,
    scratch: Scratch,
}

/// A mechanism as an ODE system: state `[q, q̇]`, the joints' coordinates
/// then their rates.
pub struct Mechanism {
    spec: BlockSpec,
    tree: Tree,
    places: Vec<Place>,
    work: RefCell<Work>,
}

impl Mechanism {
    /// Built from its rows (ADR 0105 points 2–4, ADR 0106: no inputs, no
    /// settings); the dynamics are checked at the start state (point 6).
    pub fn new(
        name: &str,
        rows: &[Row],
        settings: &std::collections::BTreeMap<String, String>,
    ) -> std::result::Result<Self, String> {
        if let Some(k) = settings.keys().next() {
            return Err(format!(
                "settings.{k}: a mechanism has no settings; its forces and loads are \
                 parameters, driven by wires (ADR 0106)"
            ));
        }
        let tree = rows::tree(rows);
        let mut spec = BlockSpec::new(name, Domain::Equations);
        // States: every coordinate, then every rate.
        let dof = tree.dof();
        let moving: Vec<(&Row, Kind)> = rows
            .iter()
            .map(|r| (r, Kind::from(r.kind)))
            .filter(|(_, k)| *k != Kind::Fixed)
            .collect();
        let state_names = |rate: bool| {
            moving.iter().map(move |(r, k)| {
                let (q, v, _) = rows::words(*k).expect("moving");
                format!("{}_{}", r.joint, if rate { v } else { q })
            })
        };
        for s in state_names(false).chain(state_names(true)) {
            spec = spec.state(&s);
        }
        // Outputs: each joint's coordinate and rate, the energy, then the
        // bodies' frames over rows.
        for (k, (r, kind)) in moving.iter().enumerate() {
            let (q, v, _) = rows::words(*kind).expect("moving");
            let (uq, uv) = if *kind == Kind::Revolute {
                ("rad", "rad/s")
            } else {
                ("m", "m/s")
            };
            spec = spec
                .port(
                    PortSpec::output(&format!("{}_{q}", r.joint), unit(uq), Shape::Scalar)
                        .from_state(k),
                )
                .port(
                    PortSpec::output(&format!("{}_{v}", r.joint), unit(uv), Shape::Scalar)
                        .from_state(dof + k),
                );
        }
        spec = spec.output("energy", unit("J"), Shape::Scalar);
        for (port, u) in POSES {
            spec = spec.output(port, unit(u), Shape::Rows { dim: 1 });
        }
        // Parameters: gravity, then each row's numbers.
        spec = spec.param(
            ParamSpec::new("g", unit("m/s^2"), STANDARD_GRAVITY, [0.0, 30.0])
                .described("Gravitational acceleration, along −y"),
        );
        let mut places = Vec::new();
        for (r, link) in rows.iter().zip(tree.links()) {
            let kind = Kind::from(r.kind);
            let mut next = spec.params.len();
            let mut take = |n: usize| {
                next += n;
                next - n
            };
            let place = Place {
                mass: take(1),
                inertia: take(1),
                centre: take(2),
                at: take(2),
                turn: take(1),
                spring: link.coord.map(|_| take(5)),
                force: link.coord.map(|_| take(1)),
                load: take(3),
            };
            let (b, j) = (&r.name, &r.joint);
            let [cx, cy] = r.centre.unwrap_or_default();
            let [ax, ay] = r.at.unwrap_or_default();
            spec = spec
                .param(nonneg(&format!("{b}_mass"), "kg", r.mass, "Mass"))
                .param(nonneg(
                    &format!("{b}_inertia"),
                    "kg*m^2",
                    r.inertia.unwrap_or(0.0),
                    "Moment of inertia about the centre of mass",
                ))
                .param(signed(
                    &format!("{b}_centre_x"),
                    "m",
                    cx,
                    1.0,
                    "Centre of mass, x in the body's frame",
                ))
                .param(signed(
                    &format!("{b}_centre_y"),
                    "m",
                    cy,
                    1.0,
                    "Centre of mass, y in the body's frame",
                ))
                .param(signed(
                    &format!("{j}_at_x"),
                    "m",
                    ax,
                    1.0,
                    "The joint's point, x in the parent's frame",
                ))
                .param(signed(
                    &format!("{j}_at_y"),
                    "m",
                    ay,
                    1.0,
                    "The joint's point, y in the parent's frame",
                ))
                .param(signed(
                    &format!("{j}_turn"),
                    "rad",
                    r.turn.unwrap_or(0.0),
                    PI,
                    "The joint frame's rotation from the parent's",
                ));
            if let Some((q, v, act)) = rows::words(kind) {
                let rev = kind == Kind::Revolute;
                let (ud, us, uq, uv) = if rev {
                    ("N*m*s", "N*m", "rad", "rad/s")
                } else {
                    ("N*s/m", "N/m", "m", "m/s")
                };
                let [q0, v0] = r.start.unwrap_or_default();
                let span = if rev { PI } else { 1.0 };
                spec = spec
                    .param(nonneg(
                        &format!("{j}_damping"),
                        ud,
                        r.damping.unwrap_or(0.0),
                        "Viscous friction in the joint",
                    ))
                    .param(nonneg(
                        &format!("{j}_stiffness"),
                        us,
                        r.stiffness.unwrap_or(0.0),
                        "A spring in the joint toward rest",
                    ))
                    .param(signed(
                        &format!("{j}_rest"),
                        uq,
                        r.rest.unwrap_or(0.0),
                        span,
                        "The spring's rest coordinate",
                    ))
                    .param(
                        signed(
                            &format!("{j}_{q}0"),
                            uq,
                            q0,
                            span,
                            "The coordinate at the start",
                        )
                        .sets_initial_state(),
                    )
                    .param(
                        signed(&format!("{j}_{v}0"), uv, v0, 10.0, "The rate at the start")
                            .sets_initial_state(),
                    )
                    .param(signed(
                        &format!("{j}_{act}"),
                        if rev { "N*m" } else { "N" },
                        0.0,
                        10.0,
                        "Drives the joint; a wire may drive it (ADR 0106)",
                    ));
            }
            spec = spec
                .param(signed(
                    &format!("{b}_fx"),
                    "N",
                    0.0,
                    10.0,
                    "A force at the centre of mass, x in world axes",
                ))
                .param(signed(
                    &format!("{b}_fy"),
                    "N",
                    0.0,
                    10.0,
                    "A force at the centre of mass, y in world axes",
                ))
                .param(signed(
                    &format!("{b}_torque"),
                    "N*m",
                    0.0,
                    10.0,
                    "A torque on the body",
                ));
            places.push(place);
        }
        // The energy and the poses read every number but the forces, so a
        // wire driving a force from them makes no loop (ADR 0106 point 3).
        let numbers: Vec<String> = spec
            .params
            .iter()
            .enumerate()
            .filter(|(k, _)| {
                !places
                    .iter()
                    .any(|p| p.force == Some(*k) || (p.load..p.load + 3).contains(k))
            })
            .map(|(_, q)| q.name.clone())
            .collect();
        let numbers: Vec<&str> = numbers.iter().map(String::as_str).collect();
        for port in spec.ports.iter_mut().filter(|p| p.state.is_none()) {
            *port = port.clone().depends_on_params(&numbers);
        }
        unique_names(&spec)?;
        let mech = Self {
            spec,
            tree,
            places,
            work: RefCell::new(Work::default()),
        };
        mech.check_start(rows)?;
        Ok(mech)
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The defaults of every parameter, in order.
    pub fn defaults(&self) -> Vec<f64> {
        self.spec.params.iter().map(|p| p.default).collect()
    }

    /// Each body's numbers from the parameters.
    fn numbers(&self, p: &[f64], out: &mut Vec<Numbers>) {
        out.clear();
        out.extend(self.places.iter().map(|pl| {
            let (damping, stiffness, rest) = pl
                .spring
                .map_or((0.0, 0.0, 0.0), |s| (p[s], p[s + 1], p[s + 2]));
            Numbers {
                mass: p[pl.mass],
                inertia: p[pl.inertia],
                centre: [p[pl.centre], p[pl.centre + 1]],
                at: [p[pl.at], p[pl.at + 1]],
                turn: p[pl.turn],
                damping,
                stiffness,
                rest,
            }
        }));
    }

    /// The numbers, joint forces and loads for an evaluation, into `w`:
    /// all from the parameters (ADR 0106).
    fn prepare(&self, p: &[f64], w: &mut Work) {
        self.numbers(p, &mut w.nums);
        w.tau.clear();
        w.tau.resize(self.tree.dof(), 0.0);
        w.loads.clear();
        for (body, (pl, l)) in self.places.iter().zip(self.tree.links()).enumerate() {
            if let (Some(k), Some(f)) = (l.coord, pl.force) {
                w.tau[k] = p[f];
            }
            w.loads.push(Load {
                body,
                force: [p[pl.load], p[pl.load + 1]],
                torque: p[pl.load + 2],
            });
        }
    }

    /// `q̈` at the state `x`, into `qdd`.
    pub fn accelerations(&self, x: &[f64], u: &[f64], p: &[f64], qdd: &mut [f64]) {
        let n = self.tree.dof();
        let _ = u;
        let w = &mut *self.work.borrow_mut();
        self.prepare(p, w);
        let at = At {
            nums: &w.nums,
            gravity: [0.0, -p[0]],
            q: &x[..n],
            qd: &x[n..],
            tau: &w.tau,
            loads: &w.loads,
        };
        tree::aba(&self.tree, at, qdd, &mut w.scratch);
    }

    /// Every body's frame in the world at the state `x`, into `out`.
    pub fn poses(&self, x: &[f64], p: &[f64], out: &mut Vec<Pose>) {
        let n = self.tree.dof();
        let w = &mut *self.work.borrow_mut();
        self.numbers(p, &mut w.nums);
        out.resize(self.tree.len(), Pose::default());
        tree::poses(&self.tree, &w.nums, &x[..n], &x[n..], out, &mut w.scratch);
    }

    /// Kinetic and potential energy at the state `x`.
    pub fn energy(&self, x: &[f64], p: &[f64]) -> f64 {
        let n = self.tree.dof();
        let w = &mut *self.work.borrow_mut();
        self.numbers(p, &mut w.nums);
        tree::energy(
            &self.tree,
            &w.nums,
            [0.0, -p[0]],
            &x[..n],
            &x[n..],
            &mut w.scratch,
        )
    }

    /// Point 6: every joint can be moved at the start: the bodies beyond
    /// it give it mass or inertia (`q̈` is a number with the defaults).
    fn check_start(&self, rows: &[Row]) -> std::result::Result<(), String> {
        let p = self.defaults();
        let mut x = vec![0.0; 2 * self.tree.dof()];
        self.initial_state(&p, &mut x);
        let mut qdd = vec![0.0; self.tree.dof()];
        self.accelerations(&x, &[], &p, &mut qdd);
        for (r, l) in rows.iter().zip(self.tree.links()) {
            if l.coord.is_some_and(|k| !qdd[k].is_finite()) {
                return Err(format!(
                    "joint '{}': the bodies beyond it have no mass or inertia it can move",
                    r.joint
                ));
            }
        }
        Ok(())
    }
}

/// A parameter that is 0 or more.
fn nonneg(name: &str, u: &str, default: f64, what: &str) -> ParamSpec {
    ParamSpec::new(name, unit(u), default, [0.0, (2.0 * default).max(1.0)]).described(what)
}

/// A parameter of either sign, its slider over at least `±span`.
fn signed(name: &str, u: &str, default: f64, span: f64, what: &str) -> ParamSpec {
    let r = (2.0 * default.abs()).max(span);
    ParamSpec::new(name, unit(u), default, [-r, r]).described(what)
}

/// Point 4: no two ports, parameters or states share a name.
fn unique_names(spec: &BlockSpec) -> std::result::Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    let ports = spec.ports.iter().map(|p| &p.name);
    let params = spec.params.iter().map(|p| &p.name);
    for n in ports.chain(params) {
        if !seen.insert(n.as_str()) {
            return Err(format!(
                "'{n}' would name two of this mechanism's ports or parameters: rename a body or joint"
            ));
        }
    }
    Ok(())
}

impl Equations for Mechanism {
    fn rhs(&self, _t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        let n = self.tree.dof();
        dx[..n].copy_from_slice(&x[n..]);
        self.accelerations(x, u, p, &mut dx[n..]);
    }

    fn outputs(&self, _t: f64, x: &[f64], _u: &[f64], p: &[f64], y: &mut [f64]) {
        let n = self.tree.dof();
        for k in 0..n {
            y[2 * k] = x[k];
            y[2 * k + 1] = x[n + k];
        }
        y[2 * n] = self.energy(x, p);
    }
}

impl OdeSystem for Mechanism {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        let n = self.tree.dof();
        for (pl, l) in self.places.iter().zip(self.tree.links()) {
            if let (Some(k), Some(s)) = (l.coord, pl.spring) {
                x0[k] = p[s + 3];
                x0[n + k] = p[s + 4];
            }
        }
    }

    /// A coordinate's rate is its own rate; a rate's rate depends on every
    /// state (ADR 0104: given, not probed).
    fn sparsity(&self) -> Option<Vec<Vec<usize>>> {
        let n = self.tree.dof();
        Some(
            (0..2 * n)
                .map(|i| {
                    if i < n {
                        vec![n + i]
                    } else {
                        (0..2 * n).collect()
                    }
                })
                .collect(),
        )
    }
}

/// The kernel block: the ODE block of a [`Mechanism`], which also writes
/// the bodies' frames as rows (ADR 0105 point 4) whenever it writes its
/// outputs.
pub struct MechBlock {
    ode: OdeBlock,
    /// A copy of the mechanism for the rows (the ODE block owns its own).
    mech: Mechanism,
    poses: RefCell<Vec<Pose>>,
}

impl MechBlock {
    pub fn new(mech: Mechanism, copy: Mechanism, integrator: Integrator) -> Self {
        Self {
            ode: OdeBlock::new(mech, integrator),
            mech: copy,
            poses: RefCell::new(Vec::new()),
        }
    }

    /// The rows outputs from the state, ids by row order from 0; the
    /// structure never changes, so its version is 0.
    fn write_rows(&self, state: &[f64], params: &[f64], out: &mut [Vec<f64>]) {
        let mut poses = self.poses.borrow_mut();
        self.mech.poses(state, params, &mut poses);
        let n = poses.len();
        let pick: [fn(&Pose) -> f64; 6] = [
            |p| p.x,
            |p| p.y,
            |p| p.angle,
            |p| p.vx,
            |p| p.vy,
            |p| p.omega,
        ];
        for (frame, f) in out.iter_mut().zip(pick) {
            frame.clear();
            frame.reserve(2 + 2 * n);
            frame.push(0.0);
            frame.push(n as f64);
            frame.extend((0..n).map(|i| i as f64));
            frame.extend(poses.iter().map(f));
        }
    }
}

impl Block for MechBlock {
    fn spec(&self) -> &BlockSpec {
        self.ode.spec()
    }

    fn method(&self) -> &MethodSpec {
        self.ode.method()
    }

    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        self.ode.init(params, state)
    }

    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        self.ode.initial_params(state, params)
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let StepIo {
            params,
            inputs,
            state,
            outputs,
            tables,
            population_inputs,
            population_outputs,
        } = io;
        self.ode.do_step(
            ctx,
            StepIo {
                params,
                inputs,
                state: &mut *state,
                outputs,
                tables,
                population_inputs,
                population_outputs: &mut *population_outputs,
            },
        )?;
        self.write_rows(state, params, population_outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        let StepIo {
            params,
            inputs,
            state,
            outputs,
            tables,
            population_inputs,
            population_outputs,
        } = io;
        self.ode.compute_outputs(
            t,
            StepIo {
                params,
                inputs,
                state: &mut *state,
                outputs,
                tables,
                population_inputs,
                population_outputs: &mut *population_outputs,
            },
        )?;
        self.write_rows(state, params, population_outputs);
        Ok(())
    }

    fn as_equations(&self) -> Option<&dyn Equations> {
        self.ode.as_equations()
    }
}

/// The block for a model file's entry: its rows from the definition lines,
/// its method from the configuration (RK4 when none is given).
pub fn build(name: &str, c: &BlockConfig) -> Result<Box<dyn Block>> {
    let bad = |e: String| SimError::InvalidDefinition(e);
    if c.update.is_some() {
        return Err(SimError::InvalidSetting {
            setting: "update".into(),
            reason: "only agent blocks have an update order".into(),
        });
    }
    let rows = rows::read(&c.definition).map_err(bad)?;
    let mech = Mechanism::new(name, &rows, &c.settings).map_err(bad)?;
    let copy = Mechanism::new(name, &rows, &c.settings).map_err(bad)?;
    let integrator = if c.method.name.is_empty() {
        Integrator::Rk4 { substeps: 1 }
    } else {
        Integrator::from_settings(&c.method)?
    };
    Ok(Box::new(MechBlock::new(mech, copy, integrator)))
}
