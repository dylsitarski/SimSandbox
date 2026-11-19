//! The same models on Rapier 3D (`rapier3d-f64`), moving in the x–y plane
//! (y up, Rapier's default) with hinges about z; the boxes are cubes.

use crate::common::{self, cartpole as cp, pendulum as pd, SnapKind, Snapshot};
use rapier3d_f64::prelude::*;

pub use crate::rapier_2d::Joints;

pub use crate::rapier_2d::Model;

pub struct World {
    pub gravity: Vector,
    pub params: IntegrationParameters,
    pub pipeline: PhysicsPipeline,
    pub islands: IslandManager,
    pub broad: BroadPhaseBvh,
    pub narrow: NarrowPhase,
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse: ImpulseJointSet,
    pub multibody: MultibodyJointSet,
    pub soft: SoftBodySet,
    pub ccd: CCDSolver,
    last_multibody: Option<MultibodyJointHandle>,
}

type Parts = (
    IslandManager,
    BroadPhaseBvh,
    NarrowPhase,
    RigidBodySet,
    ColliderSet,
    ImpulseJointSet,
    MultibodyJointSet,
    SoftBodySet,
    CCDSolver,
);

impl World {
    fn new(dt: f64) -> Self {
        let mut params = IntegrationParameters {
            dt,
            ..Default::default()
        };
        // For experiments: SPIKE_RAPIER_ITERS overrides the solver iterations
        // (substeps) per step; the tables use Rapier's default (4).
        if let Some(n) = std::env::var("SPIKE_RAPIER_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            params.num_solver_iterations = n;
        }
        World {
            gravity: Vector::new(0.0, -common::G, 0.0),
            params,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::default(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            last_multibody: None,
        }
    }
    fn step(&mut self) {
        self.pipeline.step(
            self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse,
            &mut self.multibody,
            &mut self.soft,
            &mut self.ccd,
            &(),
            &(),
        );
    }
    fn parts(&self) -> Parts {
        (
            self.islands.clone(),
            self.broad.clone(),
            self.narrow.clone(),
            self.bodies.clone(),
            self.colliders.clone(),
            self.impulse.clone(),
            self.multibody.clone(),
            self.soft.clone(),
            self.ccd.clone(),
        )
    }
    fn set_parts(&mut self, p: Parts) {
        self.islands = p.0;
        self.broad = p.1;
        self.narrow = p.2;
        self.bodies = p.3;
        self.colliders = p.4;
        self.impulse = p.5;
        self.multibody = p.6;
        self.soft = p.7;
        self.ccd = p.8;
    }
    fn serialize(&self) -> Vec<u8> {
        bincode::serialize(&(
            &self.islands,
            &self.broad,
            &self.narrow,
            &self.bodies,
            &self.colliders,
            &self.impulse,
            &self.multibody,
            &self.soft,
            &self.ccd,
        ))
        .expect("serialize")
    }
    fn joint(
        &mut self,
        joints: Joints,
        a: RigidBodyHandle,
        b: RigidBodyHandle,
        j: impl Into<GenericJoint>,
    ) {
        match joints {
            Joints::Impulse => {
                self.impulse.insert(a, b, j, true);
            }
            Joints::Multibody => {
                let h = self
                    .multibody
                    .insert(a, b, j, true)
                    .expect("multibody joint");
                self.last_multibody = Some(h);
            }
        }
    }
    /// A multibody starts from its joint coordinates, not from where its
    /// bodies were placed: set them, and take off the default damping
    /// (0.1 on every angular degree of freedom) so it matches the others.
    fn start_multibody(&mut self, coords: &[f64]) {
        let Some(h) = self.last_multibody else { return };
        let (mb, _) = self.multibody.get_mut(h).expect("multibody");
        // Until the first step the root counts as free; this makes it fixed.
        mb.forward_kinematics(&self.bodies, true);
        mb.damping_mut().fill(0.0);
        let mut disp = vec![0.0; mb.ndofs()];
        assert_eq!(mb.ndofs(), coords.len().max(mb.ndofs()));
        disp[..coords.len()].copy_from_slice(coords);
        mb.apply_displacements(&disp);
        mb.forward_kinematics(&self.bodies, true);
        mb.update_rigid_bodies(&mut self.bodies, false);
    }
}

pub struct R3 {
    pub w: World,
    pub joints: Joints,
    pub model: Model,
    /// The dynamic bodies, in row order.
    pub handles: Vec<RigidBodyHandle>,
}

fn v2(x: f64, y: f64) -> Vector {
    Vector::new(x, y, 0.0)
}

fn about_z(a: f64) -> AngVector {
    Vector::new(0.0, 0.0, a)
}

fn angle(b: &RigidBody) -> f64 {
    let q = b.rotation();
    2.0 * q.z.atan2(q.w)
}

/// `inertia` about the in-plane axis z; the other two are given values that
/// make a valid body (they do not act: everything turns about z).
fn dynamic(pos: Vector, a: f64, mass: f64, inertia: f64) -> RigidBodyBuilder {
    RigidBodyBuilder::dynamic()
        .translation(pos)
        .rotation(about_z(a))
        .additional_mass_properties(MassProperties::new(
            Vector::ZERO,
            mass,
            Vector::new(inertia, inertia, inertia),
        ))
        .can_sleep(false)
}

impl R3 {
    pub fn new(model: Model, joints: Joints, dt: f64) -> Self {
        let mut w = World::new(dt);
        let mut handles = vec![];
        match &model {
            Model::Pendulum { amplitude } => {
                let a = *amplitude;
                let ground = w.bodies.insert(RigidBodyBuilder::fixed());
                let bob = w.bodies.insert(dynamic(
                    v2(pd::L * a.sin(), -pd::L * a.cos()),
                    a,
                    pd::M,
                    pd::IC,
                ));
                let j = RevoluteJointBuilder::new(Vector::Z)
                    .local_anchor1(Vector::ZERO)
                    .local_anchor2(v2(0.0, pd::L))
                    .contacts_enabled(false);
                w.joint(joints, ground, bob, j);
                w.start_multibody(&[a]);
                handles.push(bob);
            }
            Model::CartPole => {
                let ground = w.bodies.insert(RigidBodyBuilder::fixed());
                let cart = w.bodies.insert(dynamic(
                    Vector::ZERO,
                    0.0,
                    cp::MC,
                    cp::MC * (cp::CART_W.powi(2) + cp::CART_H.powi(2)) / 12.0,
                ));
                let th = cp::THETA0;
                let pole = w.bodies.insert(dynamic(
                    v2(-cp::LH * th.sin(), cp::LH * th.cos()),
                    th,
                    cp::MP,
                    cp::IC,
                ));
                let slide = PrismaticJointBuilder::new(Vector::X).contacts_enabled(false);
                w.joint(joints, ground, cart, slide);
                let hinge = RevoluteJointBuilder::new(Vector::Z)
                    .local_anchor1(Vector::ZERO)
                    .local_anchor2(v2(0.0, -cp::LH))
                    .contacts_enabled(false);
                w.joint(joints, cart, pole, hinge);
                w.start_multibody(&[0.0, th]);
                handles.extend([cart, pole]);
            }
            Model::Chain { links } => {
                // Links 0.1 m long, 0.1 kg, starting horizontal along +x.
                let (len, m) = (0.1, 0.1);
                let mut prev = w.bodies.insert(RigidBodyBuilder::fixed());
                let mut prev_anchor = Vector::ZERO;
                for i in 0..*links {
                    let b = w.bodies.insert(dynamic(
                        v2(len * (i as f64 + 0.5), 0.0),
                        0.0,
                        m,
                        m * len * len / 12.0,
                    ));
                    let j = RevoluteJointBuilder::new(Vector::Z)
                        .local_anchor1(prev_anchor)
                        .local_anchor2(v2(-len / 2.0, 0.0))
                        .contacts_enabled(false);
                    w.joint(joints, prev, b, j);
                    prev = b;
                    prev_anchor = v2(len / 2.0, 0.0);
                    handles.push(b);
                }
                w.start_multibody(&[]);
            }
            Model::BoxOnGround | Model::BoxOnBox | Model::Stack | Model::Pile { .. } => {
                let ground = w
                    .bodies
                    .insert(RigidBodyBuilder::fixed().translation(v2(0.0, -0.5)));
                w.colliders.insert_with_parent(
                    ColliderBuilder::cuboid(20.0, 0.5, 20.0).friction(1.0),
                    ground,
                    &mut w.bodies,
                );
                let h = 0.1; // half size: 0.2 m boxes of 1 kg
                let add = |w: &mut World, x: f64, y: f64, z: f64, a: f64| {
                    let b = w.bodies.insert(
                        RigidBodyBuilder::dynamic()
                            .translation(Vector::new(x, y, z))
                            .rotation(about_z(a))
                            .can_sleep(false),
                    );
                    w.colliders.insert_with_parent(
                        ColliderBuilder::cuboid(h, h, h)
                            .density(1.0 / (8.0 * h * h * h))
                            .friction(1.0),
                        b,
                        &mut w.bodies,
                    );
                    b
                };
                match &model {
                    Model::BoxOnGround => handles.push(add(&mut w, 0.0, h, 0.0, 0.0)),
                    Model::BoxOnBox => {
                        handles.push(add(&mut w, 0.0, h, 0.0, 0.0));
                        handles.push(add(&mut w, 0.05, 3.0 * h + 0.3, 0.0, 0.1));
                    }
                    Model::Stack => {
                        handles.push(add(&mut w, 0.0, h, 0.0, 0.0));
                        handles.push(add(&mut w, 0.05, 3.0 * h, 0.0, 0.0));
                    }
                    Model::Pile { boxes } => {
                        for i in 0..*boxes {
                            let (col, lay, row) = (i % 5, (i / 5) % 5, i / 25);
                            let off = if row % 2 == 0 { 0.03 } else { -0.03 };
                            let x = (col as f64 - 2.0) * 0.25 + off;
                            let z = (lay as f64 - 2.0) * 0.25 - off;
                            let y = 0.3 + row as f64 * 0.25;
                            handles.push(add(
                                &mut w,
                                x,
                                y,
                                z,
                                0.05 * ((i * 7 % 11) as f64 - 5.0) / 5.0,
                            ));
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        R3 {
            w,
            joints,
            model,
            handles,
        }
    }

    fn body(&self, i: usize) -> &RigidBody {
        &self.w.bodies[self.handles[i]]
    }
}

impl common::Sim for R3 {
    fn label(&self) -> String {
        let j = match self.joints {
            Joints::Impulse => "impulse",
            Joints::Multibody => "multibody",
        };
        format!("Rapier 3D, {j} joints")
    }
    fn dt(&self) -> f64 {
        self.w.params.dt
    }
    fn step(&mut self, input: &[f64]) {
        if let Model::CartPole = self.model {
            let s = self.coords();
            let (tip, _) = cp::tip(&[s[0], s[1], s[2], s[3]]);
            let (c, p) = (self.handles[0], self.handles[1]);
            let cart = &mut self.w.bodies[c];
            cart.reset_forces(false);
            cart.add_force(v2(input[0], 0.0), false);
            let pole = &mut self.w.bodies[p];
            pole.reset_forces(false);
            pole.reset_torques(false); // add_force_at_point adds a torque too, kept until reset
            pole.add_force_at_point(v2(input[1], input[2]), v2(tip[0], tip[1]), false);
        }
        self.w.step();
    }
    fn coords(&self) -> Vec<f64> {
        match self.model {
            Model::Pendulum { .. } => {
                let b = self.body(0);
                vec![angle(b), b.angvel().z]
            }
            Model::CartPole => {
                let (c, p) = (self.body(0), self.body(1));
                vec![c.translation().x, c.linvel().x, angle(p), p.angvel().z]
            }
            _ => self.rows(),
        }
    }
    fn rows(&self) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.handles.len() * 13);
        for &h in &self.handles {
            let b = &self.w.bodies[h];
            let (t, q, v, w) = (b.translation(), b.rotation(), b.linvel(), b.angvel());
            out.extend([
                t.x, t.y, t.z, q.x, q.y, q.z, q.w, v.x, v.y, v.z, w.x, w.y, w.z,
            ]);
        }
        out
    }
    fn energy(&self) -> f64 {
        let b = self.body(0);
        let v = b.linvel();
        0.5 * pd::M * v.length_squared()
            + 0.5 * pd::IC * b.angvel().length_squared()
            + pd::M * common::G * (b.translation().y + pd::L)
    }
    fn box_poses(&self) -> Vec<[f64; 2]> {
        self.handles
            .iter()
            .map(|&h| {
                let t = self.w.bodies[h].translation();
                [t.x, t.y]
            })
            .collect()
    }
    fn joint_drift(&self) -> f64 {
        match self.model {
            Model::Pendulum { .. } => {
                let b = self.body(0);
                (b.position() * v2(0.0, pd::L)).length()
            }
            Model::CartPole => {
                let (c, p) = (self.body(0), self.body(1));
                let pivot = p.position() * v2(0.0, -cp::LH);
                (pivot - c.translation()).length()
                    + c.translation().y.abs()
                    + c.translation().z.abs()
                    + 2.0 * c.rotation().w.abs().min(1.0).acos()
            }
            _ => 0.0,
        }
    }
    fn snapshot(&self, kind: SnapKind) -> Snapshot {
        match kind {
            SnapKind::Memory => {
                let p = self.w.parts();
                Snapshot {
                    kind,
                    bytes: self.w.serialize().len(),
                    data: Box::new(p),
                }
            }
            SnapKind::Bytes | SnapKind::BytesFresh => {
                let b = self.w.serialize();
                Snapshot {
                    kind,
                    bytes: b.len(),
                    data: Box::new(b),
                }
            }
            SnapKind::Minimal => {
                let v: Vec<(Pose, Vector, AngVector)> = self
                    .handles
                    .iter()
                    .map(|&h| {
                        let b = &self.w.bodies[h];
                        (*b.position(), b.linvel(), b.angvel())
                    })
                    .collect();
                Snapshot {
                    kind,
                    bytes: v.len() * 13 * 8,
                    data: Box::new(v),
                }
            }
        }
    }
    fn restore(&mut self, snap: &Snapshot) {
        match snap.kind {
            SnapKind::Memory => {
                let p: &Parts = snap.data.downcast_ref().unwrap();
                self.w.set_parts(p.clone());
            }
            SnapKind::Bytes | SnapKind::BytesFresh => {
                let b: &Vec<u8> = snap.data.downcast_ref().unwrap();
                let p: Parts = bincode::deserialize(b).expect("deserialize");
                self.w.set_parts(p);
                if snap.kind == SnapKind::BytesFresh {
                    self.w.pipeline = PhysicsPipeline::new();
                }
            }
            SnapKind::Minimal => {
                let v: &Vec<(Pose, Vector, AngVector)> = snap.data.downcast_ref().unwrap();
                for (&h, (p, l, a)) in self.handles.iter().zip(v) {
                    let b = &mut self.w.bodies[h];
                    b.set_position(*p, false);
                    b.set_linvel(*l, false);
                    b.set_angvel(*a, false);
                }
            }
        }
    }
}
