//! The models on MuJoCo through its C API, with bindings written by hand for
//! the few functions used. Every value goes through `mj_getState` and
//! `mj_setState`, so no struct layout is declared here: `mjModel` and
//! `mjData` stay opaque. Reading `xpos`/`xquat` (bodies as rows in general)
//! would need the layout, from the headers by bindgen, as `mujoco-rs` does.

#![allow(non_camel_case_types, non_upper_case_globals)]

use crate::common::{self, cartpole as cp, pendulum as pd, SnapKind, Snapshot};
use crate::rapier_2d::Model;
use std::ffi::{c_char, c_int, CStr, CString};

#[repr(C)]
pub struct mjModel {
    _opaque: [u8; 0],
}
#[repr(C)]
pub struct mjData {
    _opaque: [u8; 0],
}

// mjtState (mjtype.h of MuJoCo 4.0.1).
pub const mjSTATE_TIME: c_int = 1 << 0;
pub const mjSTATE_QPOS: c_int = 1 << 1;
pub const mjSTATE_QVEL: c_int = 1 << 2;
pub const mjSTATE_ACT: c_int = 1 << 3;
pub const mjSTATE_HISTORY: c_int = 1 << 4;
pub const mjSTATE_WARMSTART: c_int = 1 << 5;
pub const mjSTATE_CTRL: c_int = 1 << 6;
pub const mjSTATE_QFRC_APPLIED: c_int = 1 << 7;
pub const mjSTATE_XFRC_APPLIED: c_int = 1 << 8;
pub const mjSTATE_EQ_ACTIVE: c_int = 1 << 9;
pub const mjSTATE_MOCAP_POS: c_int = 1 << 10;
pub const mjSTATE_MOCAP_QUAT: c_int = 1 << 11;
pub const mjSTATE_USERDATA: c_int = 1 << 12;
pub const mjSTATE_PLUGIN: c_int = 1 << 13;
pub const mjSTATE_PLASTIC: c_int = 1 << 14;
pub const mjSTATE_PHYSICS: c_int =
    mjSTATE_QPOS | mjSTATE_QVEL | mjSTATE_ACT | mjSTATE_HISTORY | mjSTATE_PLASTIC;
pub const mjSTATE_FULLPHYSICS: c_int = mjSTATE_TIME | mjSTATE_PHYSICS | mjSTATE_PLUGIN;
pub const mjSTATE_USER: c_int = mjSTATE_CTRL
    | mjSTATE_QFRC_APPLIED
    | mjSTATE_XFRC_APPLIED
    | mjSTATE_EQ_ACTIVE
    | mjSTATE_MOCAP_POS
    | mjSTATE_MOCAP_QUAT
    | mjSTATE_USERDATA;
pub const mjSTATE_INTEGRATION: c_int = mjSTATE_FULLPHYSICS | mjSTATE_USER | mjSTATE_WARMSTART;

/// The version the constants above were copied from (`mjVERSION_HEADER`).
pub const HEADER_VERSION: c_int = 4000001;

extern "C" {
    fn mj_loadXML(
        filename: *const c_char,
        vfs: *const u8,
        error: *mut c_char,
        error_sz: c_int,
    ) -> *mut mjModel;
    fn mj_makeData(m: *const mjModel) -> *mut mjData;
    fn mj_copyData(dest: *mut mjData, m: *const mjModel, src: *const mjData) -> *mut mjData;
    fn mj_step(m: *const mjModel, d: *mut mjData);
    fn mj_forward(m: *const mjModel, d: *mut mjData);
    fn mj_resetData(m: *const mjModel, d: *mut mjData);
    fn mj_deleteData(d: *mut mjData);
    fn mj_deleteModel(m: *mut mjModel);
    fn mj_stateSize(m: *const mjModel, sig: c_int) -> c_int;
    fn mj_getState(m: *const mjModel, d: *const mjData, state: *mut f64, sig: c_int);
    fn mj_setState(m: *const mjModel, d: *mut mjData, state: *const f64, sig: c_int);
    fn mj_version() -> c_int;
    fn mj_versionString() -> *const c_char;
    static mut mju_user_error: Option<unsafe extern "C" fn(*const c_char)>;
    static mut mju_user_warning: Option<unsafe extern "C" fn(*const c_char)>;
}

/// MuJoCo's default error handler prints and ends the process (`exit`), and
/// the handler must not return. A panic may not unwind through C, so this
/// one prints and aborts: the spike only needs the message. A host that must
/// survive a model error has to avoid the calls that raise one (check the
/// model first, `mj_loadXML` returns null rather than erring) or run MuJoCo
/// out of process.
unsafe extern "C" fn on_error(msg: *const c_char) {
    let m = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    eprintln!("MuJoCo error: {m}");
    std::process::abort();
}

/// Warnings (MuJoCo prints them and goes on; an unstable simulation is
/// reset to its initial state with a warning), counted here.
pub static WARNINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C" fn on_warning(msg: *const c_char) {
    let m = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    if std::env::var("SPIKE_MJ_WARNINGS").is_ok() {
        eprintln!("MuJoCo warning: {}", m.trim());
    }
    WARNINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn version() -> String {
    let v = unsafe { CStr::from_ptr(mj_versionString()) }
        .to_string_lossy()
        .into_owned();
    let n = unsafe { mj_version() };
    assert_eq!(
        n, HEADER_VERSION,
        "libmujoco {v} is not the version these bindings were written for"
    );
    v
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Integrator {
    Euler,
    Rk4,
    ImplicitFast,
}

impl Integrator {
    fn xml(self) -> &'static str {
        match self {
            Integrator::Euler => "Euler",
            Integrator::Rk4 => "RK4",
            Integrator::ImplicitFast => "implicitfast",
        }
    }
}

pub struct Mj {
    m: *mut mjModel,
    d: *mut mjData,
    pub model: Model,
    pub integrator: Integrator,
    dt: f64,
    nbody: usize,
    input: Vec<f64>,
}

impl Drop for Mj {
    fn drop(&mut self) {
        unsafe {
            mj_deleteData(self.d);
            mj_deleteModel(self.m);
        }
    }
}

fn xml(model: &Model, integ: Integrator, dt: f64) -> (String, usize) {
    let head = format!(
        r#"<mujoco><option timestep="{dt}" integrator="{}" gravity="0 0 -{}"/>"#,
        integ.xml(),
        common::G
    );
    let tiny = 1e-6;
    // Hinges about −y, so a positive angle turns +z toward −x and +x
    // toward +z: counter-clockwise seen with x right and z up, as in 2D.
    match model {
        Model::Pendulum { .. } => (
            format!(
                r#"{head}<worldbody><body name="bob"><joint type="hinge" axis="0 -1 0"/>
                <inertial pos="0 0 -{l}" mass="{m}" diaginertia="{i} {i} {i}"/></body></worldbody></mujoco>"#,
                l = pd::L,
                m = pd::M,
                i = pd::IC
            ),
            2,
        ),
        Model::CartPole => {
            let ic = cp::MC * (cp::CART_W.powi(2) + cp::CART_H.powi(2)) / 12.0;
            (
                format!(
                    r#"{head}<worldbody><body name="cart"><joint name="slide" type="slide" axis="1 0 0"/>
                    <inertial pos="0 0 0" mass="{mc}" diaginertia="{ic} {ic} {ic}"/>
                    <body name="pole"><joint type="hinge" axis="0 -1 0"/>
                    <inertial pos="0 0 {lh}" mass="{mp}" diaginertia="{ip} {ip} {tiny}"/></body></body></worldbody>
                    <actuator><motor joint="slide" gear="1"/></actuator></mujoco>"#,
                    mc = cp::MC,
                    lh = cp::LH,
                    mp = cp::MP,
                    ip = cp::IC
                ),
                3,
            )
        }
        Model::Chain { links } => {
            let (len, m) = (0.1, 0.1);
            let i = m * len * len / 12.0;
            let mut s = format!("{head}<worldbody>");
            for k in 0..*links {
                let pos = if k == 0 { 0.0 } else { len };
                s += &format!(
                    r#"<body pos="{pos} 0 0"><joint type="hinge" axis="0 -1 0"/><inertial pos="{c} 0 0" mass="{m}" diaginertia="{tiny} {i} {i}"/>"#,
                    c = len / 2.0
                );
            }
            s += &"</body>".repeat(*links);
            s += "</worldbody></mujoco>";
            (s, links + 1)
        }
        Model::BoxOnGround | Model::BoxOnBox | Model::Stack | Model::Pile { .. } => {
            let h = 0.1;
            let mut s = format!(
                r#"{head}<worldbody><geom type="plane" size="20 20 0.1" friction="1 0.005 0.0001"/>"#
            );
            // Rapier's (x, y up, z) is MuJoCo's (x, −y, z up).
            let mut add = |x: f64, up: f64, z: f64, a: f64| {
                s += &format!(
                    r#"<body pos="{x} {} {up}" axisangle="0 -1 0 {a}"><freejoint/><geom type="box" size="{h} {h} {h}" mass="1" friction="1 0.005 0.0001"/></body>"#,
                    -z
                );
            };
            let n = match model {
                Model::BoxOnGround => {
                    add(0.0, h, 0.0, 0.0);
                    1
                }
                Model::BoxOnBox => {
                    add(0.0, h, 0.0, 0.0);
                    add(0.05, 3.0 * h + 0.3, 0.0, 0.1);
                    2
                }
                Model::Stack => {
                    add(0.0, h, 0.0, 0.0);
                    add(0.05, 3.0 * h, 0.0, 0.0);
                    2
                }
                Model::Pile { boxes } => {
                    for i in 0..*boxes {
                        let (col, lay, row) = (i % 5, (i / 5) % 5, i / 25);
                        let off = if row % 2 == 0 { 0.03 } else { -0.03 };
                        add(
                            (col as f64 - 2.0) * 0.25 + off,
                            0.3 + row as f64 * 0.25,
                            (lay as f64 - 2.0) * 0.25 - off,
                            0.05 * ((i * 7 % 11) as f64 - 5.0) / 5.0,
                        );
                    }
                    *boxes
                }
                _ => unreachable!(),
            };
            s += "</worldbody></mujoco>";
            (s, n + 1)
        }
    }
}

impl Mj {
    pub fn new(model: Model, integrator: Integrator, dt: f64) -> Self {
        unsafe {
            mju_user_error = Some(on_error);
            mju_user_warning = Some(on_warning);
        }
        let (text, nbody) = xml(&model, integrator, dt);
        let path =
            std::env::temp_dir().join(format!("rigid-body-spike-{}.xml", std::process::id()));
        std::fs::write(&path, text).expect("write MJCF");
        let cpath = CString::new(path.to_str().unwrap()).unwrap();
        let mut err = vec![0 as c_char; 1000];
        let m = unsafe {
            mj_loadXML(
                cpath.as_ptr(),
                std::ptr::null(),
                err.as_mut_ptr(),
                err.len() as c_int,
            )
        };
        let _ = std::fs::remove_file(&path);
        if m.is_null() {
            panic!(
                "mj_loadXML: {}",
                unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy()
            );
        }
        let d = unsafe { mj_makeData(m) };
        let mut mj = Mj {
            m,
            d,
            model,
            integrator,
            dt,
            nbody,
            input: vec![],
        };
        unsafe { mj_resetData(m, d) };
        let start = match mj.model {
            Model::Pendulum { amplitude } => Some(vec![amplitude]),
            Model::CartPole => Some(vec![0.0, cp::THETA0]),
            _ => None,
        };
        if let Some(q) = start {
            mj.set(mjSTATE_QPOS, &q);
        }
        unsafe { mj_forward(m, d) };
        mj
    }
    pub fn size(&self, sig: c_int) -> usize {
        unsafe { mj_stateSize(self.m, sig) as usize }
    }
    pub fn get(&self, sig: c_int) -> Vec<f64> {
        let mut v = vec![0.0; self.size(sig)];
        unsafe { mj_getState(self.m, self.d, v.as_mut_ptr(), sig) };
        v
    }
    fn set(&mut self, sig: c_int, v: &[f64]) {
        assert_eq!(v.len(), self.size(sig));
        unsafe { mj_setState(self.m, self.d, v.as_ptr(), sig) };
    }
}

impl common::Sim for Mj {
    fn label(&self) -> String {
        let i = match self.integrator {
            Integrator::Euler => "Euler",
            Integrator::Rk4 => "RK4",
            Integrator::ImplicitFast => "implicitfast",
        };
        format!("MuJoCo, {i}")
    }
    fn dt(&self) -> f64 {
        self.dt
    }
    fn step(&mut self, input: &[f64]) {
        if let Model::CartPole = self.model {
            // ctrl (1), then xfrc_applied (nbody × [force, torque]).
            let s = self.coords();
            let th = s[2];
            let r = [-cp::LH * th.sin(), cp::LH * th.cos()]; // centre to tip, (x, z)
            let (fx, fz) = (input[1], input[2]);
            let ty = r[1] * fx - r[0] * fz; // (r × f)_y with r, f in the x–z plane
            let n = 1 + 6 * self.nbody;
            self.input.clear();
            self.input.resize(n, 0.0);
            self.input[0] = input[0];
            let pole = 1 + 6 * 2;
            self.input[pole] = fx;
            self.input[pole + 2] = fz;
            self.input[pole + 4] = ty;
            let v = std::mem::take(&mut self.input);
            self.set(mjSTATE_CTRL | mjSTATE_XFRC_APPLIED, &v);
            self.input = v;
        }
        unsafe { mj_step(self.m, self.d) };
    }
    fn coords(&self) -> Vec<f64> {
        let q = self.get(mjSTATE_QPOS | mjSTATE_QVEL);
        match self.model {
            Model::Pendulum { .. } => vec![q[0], q[1]],
            Model::CartPole => vec![q[0], q[2], q[1], q[3]],
            _ => q,
        }
    }
    fn warnings(&self) -> usize {
        WARNINGS.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn rows(&self) -> Vec<f64> {
        // Generalized coordinates and velocities: every body's pose and
        // velocity follows from them by forward kinematics.
        self.get(mjSTATE_QPOS | mjSTATE_QVEL)
    }
    fn energy(&self) -> f64 {
        let s = self.coords();
        pd::energy(s[0], s[1])
    }
    fn box_poses(&self) -> Vec<[f64; 2]> {
        let q = self.get(mjSTATE_QPOS);
        q.chunks(7).map(|c| [c[0], c[2]]).collect()
    }
    fn snapshot(&self, kind: SnapKind) -> Snapshot {
        match kind {
            SnapKind::Memory => {
                let d = unsafe { mj_makeData(self.m) };
                unsafe { mj_copyData(d, self.m, self.d) };
                Snapshot {
                    kind,
                    bytes: 0,
                    data: Box::new(DataCopy(d)),
                }
            }
            SnapKind::Bytes | SnapKind::BytesFresh => {
                let v = self.get(mjSTATE_INTEGRATION);
                Snapshot {
                    kind,
                    bytes: v.len() * 8,
                    data: Box::new(v),
                }
            }
            SnapKind::Minimal => {
                let v = self.get(mjSTATE_FULLPHYSICS);
                Snapshot {
                    kind,
                    bytes: v.len() * 8,
                    data: Box::new(v),
                }
            }
        }
    }
    fn restore(&mut self, snap: &Snapshot) {
        match snap.kind {
            SnapKind::Memory => {
                let c: &DataCopy = snap.data.downcast_ref().unwrap();
                unsafe { mj_copyData(self.d, self.m, c.0) };
            }
            SnapKind::Bytes => {
                let v: &Vec<f64> = snap.data.downcast_ref().unwrap();
                self.set(mjSTATE_INTEGRATION, v);
            }
            SnapKind::BytesFresh => {
                let v: &Vec<f64> = snap.data.downcast_ref().unwrap();
                unsafe {
                    mj_deleteData(self.d);
                    self.d = mj_makeData(self.m);
                }
                self.set(mjSTATE_INTEGRATION, v);
            }
            SnapKind::Minimal => {
                let v: &Vec<f64> = snap.data.downcast_ref().unwrap();
                self.set(mjSTATE_FULLPHYSICS, v);
            }
        }
    }
}

struct DataCopy(*mut mjData);
impl Drop for DataCopy {
    fn drop(&mut self) {
        unsafe { mj_deleteData(self.0) };
    }
}
