//! Tests of the typed-equations language.

use super::*;
use crate::ode::OdeSystem;
use simsandbox_core::block::{Block, Equations, Shape, StepContext, StepIo};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use std::f64::consts::PI;

fn set(lines: &[&str]) -> Result<EquationSet> {
    let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    EquationSet::parse("m", &lines)
}

fn error(lines: &[&str]) -> String {
    match set(lines) {
        Err(SimError::InvalidDefinition(m)) => m,
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn states_parameters_initial_values_and_outputs_are_found() {
    let s = set(&[
        "# a damped oscillator",
        "dx/dt = v",
        "",
        "v' = -k*x - c*v",
        "E = 0.5*v^2 + 0.5*k*x^2",
    ])
    .unwrap();
    let spec = s.spec();
    assert_eq!(spec.states, ["x", "v"]);
    let params: Vec<_> = spec.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(params, ["k", "c", "x0", "v0"]);
    assert!(spec.params[2].initial_condition && !spec.params[0].initial_condition);
    let outs: Vec<_> = spec.outputs().map(|p| p.name.as_str()).collect();
    assert_eq!(outs, ["x", "v", "E"]);

    // k = 2, c = 0, x = 1, v = 3.
    let p = [2.0, 0.0, 1.0, 3.0];
    let mut x0 = [0.0; 2];
    s.initial_state(&p, &mut x0);
    assert_eq!(x0, [1.0, 3.0]);
    let mut dx = [0.0; 2];
    s.rhs(0.0, &x0, &[], &p, &mut dx);
    assert_eq!(dx, [3.0, -2.0]);
    let mut y = [0.0; 3];
    s.outputs(0.0, &x0, &[], &p, &mut y);
    assert_eq!(y, [1.0, 3.0, 0.5 * 9.0 + 0.5 * 2.0]);
}

#[test]
fn maps_iterate_all_states_together() {
    // A rotation by 90 degrees: needs the old x to compute the new y.
    let s = set(&["x[n+1] = -y[n]", "y[n+1] = x"]).unwrap();
    assert_eq!(s.kind(), Kind::Map);
    let mut b = ExplicitBlock::new(s, &Kind::Map.default_method()).unwrap();
    let (p, mut x, mut y) = ([1.0, 0.0], [0.0; 2], [0.0; 2]);
    b.init(&p, &mut x).unwrap();
    let ctx = StepContext {
        t: 0.0,
        dt: 1.0,
        step: 0,
        seed: 0,
    };
    let mut step = |x: &mut [f64], y: &mut [f64]| {
        b.do_step(
            ctx,
            StepIo {
                params: &p,
                inputs: &[],
                state: x,
                outputs: y,
                tables: &mut [],
                population_inputs: &[],
                population_outputs: &mut [],
            },
        )
        .unwrap();
    };
    step(&mut x, &mut y);
    assert_eq!(x, [0.0, 1.0]);
    step(&mut x, &mut y);
    assert_eq!(x, [-1.0, 0.0]);
    assert_eq!(y, x);
}

#[test]
fn formulas_have_no_state_and_need_the_evaluate_method() {
    let s = set(&["y = a*x^2 + b"]).unwrap();
    assert_eq!(s.kind(), Kind::Formulas);
    assert!(s.spec().states.is_empty());
    let names: Vec<_> = s.spec().params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["a", "x", "b"]);
    assert!(ExplicitBlock::new(set(&["y = 1"]).unwrap(), &Kind::Map.default_method()).is_err());
    assert!(ExplicitBlock::new(set(&["dx/dt = 1"]).unwrap(), &Kind::Map.default_method()).is_err());
    let b = ExplicitBlock::new(s, &Kind::Formulas.default_method()).unwrap();
    let mut y = [0.0];
    b.compute_outputs(
        0.0,
        StepIo {
            params: &[2.0, 3.0, 1.0],
            inputs: &[],
            state: &mut [],
            outputs: &mut y,
            tables: &mut [],
            population_inputs: &[],
            population_outputs: &mut [],
        },
    )
    .unwrap();
    assert_eq!(y, [19.0]);
}

#[test]
fn time_and_defined_quantities_can_be_used() {
    let s = set(&["a = sin(t)", "b = 2*a", "dx/dt = b"]).unwrap();
    let mut dx = [0.0];
    s.rhs(1.0, &[0.0], &[], &[0.0], &mut dx);
    assert_eq!(dx[0], 2.0 * 1.0f64.sin());
}

#[test]
fn mistakes_are_reported_with_line_and_column() {
    assert_eq!(
        error(&["dx/dt = -k*"]),
        "line 1, column 12: expected a number, name or '(', found the end"
    );
    assert!(error(&["dx/dt = 2k"]).starts_with("line 1, column 10: missing operator"));
    assert!(error(&["dx/dt = 1", "dx/dt = 2"]).contains("defined twice"));
    assert!(error(&["dt/dt = 1"]).contains("reserved"));
    assert!(error(&["dsin/dt = 1"]).contains("reserved"));
    assert!(error(&["a = b", "b = 1", "dx/dt = a"]).contains("defined further down"));
    assert!(error(&["x + 1"]).contains("expected an equation"));
    assert!(error(&["2x = 1"]).contains("is not"));
    assert!(error(&["# only a comment"]).contains("no equations"));
    assert!(error(&["dx/dt = 1", "y[n+1] = y"]).contains("not both"));
    assert!(error(&["x[n+1] = x + n"]).contains("step number"));
}

fn unit(text: &str) -> Unit {
    Unit::parse(text).unwrap()
}

#[test]
fn units_are_declared_where_names_appear() {
    let s = set(&[
        "input F [N]",
        "param m [kg] = 1000",
        "param c [N*s/m] = 50",
        "state x [m] = 2",
        "dx/dt = v",
        "dv/dt = (F - c*v)/m [m/s^2]",
        "E = m*v^2/2",
    ])
    .unwrap();
    let port = |n: &str| s.spec.ports.iter().find(|p| p.name == n).unwrap().unit;
    assert!(port("F").same_as(&Unit::NEWTON));
    assert!(port("x").same_as(&Unit::METER));
    assert!(port("v").same_as(&unit("m/s")));
    // A defined quantity without a unit takes its right side's: J.
    assert!(port("E").same_as(&unit("J")));
    let param = |n: &str| s.spec.params.iter().find(|p| p.name == n).unwrap();
    assert_eq!(
        (param("m").default, param("m").unit),
        (1000.0, Unit::KILOGRAM)
    );
    assert!(param("x0").unit.same_as(&Unit::METER) && param("x0").initial_condition);
    // The state's declaration gives x0 its default; v0 stays 1.
    assert_eq!((param("x0").default, param("v0").default), (2.0, 1.0));
    assert!(param("v0").unit.same_as(&unit("m/s")));
}

#[test]
fn a_rate_s_unit_is_its_own_and_states_can_be_declared() {
    // [m/s] on dx/dt labels the rate: x is in m.
    let s = set(&["input v [m/s]", "dx/dt = v [m/s]"]).unwrap();
    let x = s.spec.ports.iter().find(|p| p.name == "x").unwrap().unit;
    assert!(x.same_as(&Unit::METER));
    // A rate in mm/s gives mm; one in km/h, whose product with s has
    // no name, gives SI.
    let port = |lines: &[&str], n: &str| {
        let s = set(lines).unwrap();
        s.spec.ports.iter().find(|p| p.name == n).unwrap().unit
    };
    assert!(port(&["param u [mm/s] = 1", "dx/dt = u [mm/s]"], "x").same_as(&unit("mm")));
    assert!(port(&["param u [km/h] = 1", "dx/dt = u [km/h]"], "x").same_as(&Unit::METER));
    // A declared state's unit wins, and the rate must agree with it.
    assert!(port(&["state x [km]", "param u [m/s] = 1", "dx/dt = u"], "x").same_as(&unit("km")));
    let e = error(&["state x [m]", "param u [m/s] = 1", "dx/dt = u [m]"]);
    assert!(
        e.contains("dx/dt is in m, so x would be in m·s, but x is in m"),
        "{e}"
    );
    // The old meaning (the state's unit on its rate) no longer balances.
    let e = error(&["input v [m/s]", "dx/dt = v [m]"]);
    assert!(
        e.contains("dx/dt is in m but the right side is in m/s"),
        "{e}"
    );
    // Declarations need dynamics, and are not formulas.
    let e = error(&["state y [m]", "dx/dt = 1"]);
    assert!(e.contains("no line gives dy/dt"), "{e}");
    let e = error(&["state y [m]", "y = 2 [m]"]);
    assert!(e.contains("not a state"), "{e}");
    let e = error(&["state x = 1", "param x0 = 2", "dx/dt = -x"]);
    assert!(e.contains("x0 is declared twice"), "{e}");
    // Maps declare states too.
    let s = set(&["state x [m] = 0.5", "param r = 2", "x[n+1] = r*x[n]"]).unwrap();
    let x0 = s.spec.params.iter().find(|p| p.name == "x0").unwrap();
    assert_eq!(x0.default, 0.5);
    // 'state', 'param' and 'input' are still names on a left side.
    set(&["state = 2", "param = 3", "input = state*param"]).unwrap();
}

#[test]
fn equations_must_balance() {
    let e = error(&["state x [m]", "dx/dt = v", "dv/dt = -x [m/s^2]"]);
    assert!(
        e.contains("line 3") && e.contains("dv/dt is in m/s^2 but the right side is in m"),
        "{e}"
    );
    let e = error(&["input F [N]", "dx/dt = F - x [m/s]"]);
    assert!(e.contains("cannot subtract"), "{e}");
    let e = error(&["param L [m] = 1", "y = sin(L)"]);
    assert!(e.contains("dimensionless"), "{e}");
    let e = error(&["param L [m] = 1", "y = sqrt(L)"]);
    assert!(e.contains("sqrt"), "{e}");
    assert!(error(&["dx/dt = 1 [furlong]"]).contains("unknown symbol 'furlong'"));
    // Names without a unit are dimensionless, which can be what is meant.
    set(&["param L [m] = 2", "param k = 3", "y = k*L [m]"]).unwrap();
    set(&[
        "param A [m^2] = 4",
        "side = sqrt(A) [m]",
        "theta = atan2(side, side)",
    ])
    .unwrap();
}

#[test]
fn values_are_computed_in_si_across_scales() {
    let s = set(&[
        "param d [km] = 2",
        "param t_go [min] = 1",
        "y = d [m]",
        "speed = d/t_go [m/s]",
    ])
    .unwrap();
    let mut y = vec![0.0; 2];
    s.outputs(0.0, &[], &[], &[2.0, 1.0], &mut y);
    assert_eq!(y[0], 2000.0);
    assert!((y[1] - 2000.0 / 60.0).abs() < 1e-12);
    // x in km from a speed in m/s: 1 m/s is 1e-3 km per second.
    let s = set(&["input v [m/s]", "state x [km]", "dx/dt = v"]).unwrap();
    let mut dx = vec![0.0];
    s.rhs(0.0, &[0.0], &[1.0], &[0.0], &mut dx);
    assert!((dx[0] - 1e-3).abs() < 1e-15, "km per second: {}", dx[0]);
}

#[test]
fn maps_and_models_without_units_are_as_before() {
    // `[n]` is the current value, not a unit.
    set(&["x[n+1] = r*x[n]"]).unwrap();
    set(&["x[n+1] = r*x[n] [m]", "param r = 2"]).unwrap();
    // No units anywhere: dimensionless, time included, unchecked.
    let s = set(&["dx/dt = -k*x + t"]).unwrap();
    assert!(s.spec.ports.iter().all(|p| p.unit == Unit::DIMENSIONLESS));
    assert!(error(&["input u v [m]", "y = u"]).contains("commas"));
}

/// The block's rates, outputs and initial state at `p`.
fn eval(s: &EquationSet, p: &[f64], u: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let n = s.spec.states.len();
    let mut x = vec![0.0; n];
    s.initial_state(p, &mut x);
    let mut dx = vec![0.0; n];
    s.rhs(0.0, &x, u, p, &mut dx);
    let outs: usize = s.spec.outputs().map(|o| o.shape.len()).sum();
    let mut y = vec![0.0; outs];
    s.outputs(0.0, &x, u, p, &mut y);
    (x, dx, y)
}

const HEAT: &[&str] = &[
    "grid x [m] = 0 .. length, 4",
    "param length [m] = 2",
    "param alpha [m^2/s] = 0.5",
    "param peak [K/m] = 1",
    "param left [K] = 10",
    "state T[x] [K] = peak*x",
    "edge T: x- = left",
    "dT/dt = alpha*lap(T)",
    "total = sum(T) [K]",
    "slope = ddx(T)",
];

#[test]
fn arrays_on_a_grid_have_a_value_per_cell() {
    let s = set(HEAT).unwrap();
    let spec = s.spec();
    assert_eq!(spec.states, ["T[0]", "T[1]", "T[2]", "T[3]"]);
    let outs: Vec<(&str, Shape)> = spec.outputs().map(|p| (p.name.as_str(), p.shape)).collect();
    let grid = Shape::Grid { nx: 4, ny: 1 };
    assert_eq!(
        outs,
        [
            ("T", grid),
            ("total", Shape::Scalar),
            ("slope", grid),
            ("x", grid)
        ]
    );
    let x = spec.ports.iter().find(|p| p.name == "x").unwrap();
    let of: Vec<&str> = x.coordinate.iter().map(|c| c.port.as_str()).collect();
    assert_eq!(of, ["T", "slope"]);
    assert_eq!(x.unit, Unit::METER);
    let slope = spec.ports.iter().find(|p| p.name == "slope").unwrap();
    assert_eq!(slope.unit.to_string(), "K/m");
    // No T0: the parameters in the initial value apply on restart.
    let params: Vec<&str> = spec.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(params, ["length", "alpha", "peak", "left"]);
    assert!(spec.params[2].initial_condition && !spec.params[0].initial_condition);

    // Cells of 0.5 m centred at 0.25 .. 1.75; T = x initially.
    let p = [2.0, 0.5, 1.0, 10.0];
    let (t, dt, y) = eval(&s, &p, &[]);
    assert_eq!(t, [0.25, 0.75, 1.25, 1.75]);
    // Held at 10 on the left face (ghost 19.75), insulated on the right.
    let lap = |a: f64, b: f64, c: f64| 0.5 * (a - 2.0 * b + c) / 0.25;
    assert_eq!(
        dt,
        [lap(19.75, 0.25, 0.75), 0.0, 0.0, lap(1.25, 1.75, 1.75)]
    );
    assert_eq!(y[..4], t[..]);
    assert_eq!(y[4], 4.0);
    assert_eq!(
        y[5..9],
        [(0.75 - 19.75) / 1.0, 1.0, 1.0, (1.75 - 1.25) / 1.0]
    );
    assert_eq!(y[9..13], [0.25, 0.75, 1.25, 1.75]);
    // A longer grid moves its cells at once (no restart).
    let p2 = [4.0, 0.5, 1.0, 10.0];
    let mut y2 = vec![0.0; 13];
    s.outputs(0.0, &t, &[], &p2, &mut y2);
    assert_eq!(y2[9..13], [0.5, 1.5, 2.5, 3.5]);
}

#[test]
fn arrays_mix_with_scalars_inputs_and_units() {
    let s = set(&[
        "grid r [cm] = 0 .. 4, 2",
        "input q[r] [K/s], boost",
        "param tau [s] = 1",
        "state u[r] [K] = 1",
        "state v [K] = 2",
        "edge u: r periodic",
        "du/dt = q*boost + mean(u)/tau - u/tau",
        "dv/dt = (maxof(u) - v)/tau",
        "first = at(u, 0) [K]",
        "dr = spacing(r) [cm]",
    ])
    .unwrap();
    let spec = s.spec();
    assert_eq!(spec.states, ["u[0]", "u[1]", "v"]);
    let ins: Vec<(&str, Shape)> = spec.inputs().map(|p| (p.name.as_str(), p.shape)).collect();
    assert_eq!(
        ins,
        [
            ("q", Shape::Grid { nx: 2, ny: 1 }),
            ("boost", Shape::Scalar)
        ]
    );
    // tau, v0: v starts from v0, u from its expression.
    let p = [1.0, 2.0];
    let (x, dx, y) = eval(&s, &p, &[3.0, 4.0, 0.5]);
    assert_eq!(x, [1.0, 1.0, 2.0]);
    assert_eq!(dx, [1.5, 2.0, -1.0]);
    assert_eq!(y[..3], [1.0, 1.0, 2.0]);
    assert_eq!(y[3], 1.0);
    // The axis in cm: 2 cm cells centred at 1 and 3.
    assert_eq!(y[4], 2.0);
    assert_eq!(y[5..7], [1.0, 3.0]);
    let dr = spec.ports.iter().find(|p| p.name == "dr").unwrap();
    assert_eq!(dr.unit.to_string(), "cm");
}

#[test]
fn index_axes_argmax_and_computed_cells() {
    // ADR 0070: cells numbered 0..n-1, the cell of a peak, and the
    // value of another array there; comparisons give 1 or 0.
    let s = set(&[
        "grid k = 4",
        "input a[k] [m], w[k] [1/s]",
        "best = argmaxof(a)",
        "low = argminof(a)",
        "w_peak = at(w, argmaxof(a)) [1/s]",
        "outside = at(a, best + 10) [m]",
        "high = maxof(a) > at(a, 0)",
    ])
    .unwrap();
    let spec = s.spec();
    let ins: Vec<Shape> = spec.inputs().map(|p| p.shape).collect();
    assert_eq!(ins, [Shape::Grid { nx: 4, ny: 1 }; 2]);
    let (_, _, y) = eval(&s, &[], &[1.0, 5.0, 5.0, -2.0, 10.0, 20.0, 30.0, 40.0]);
    let mut at = 0;
    let mut out = std::collections::HashMap::new();
    for p in spec.outputs() {
        out.insert(p.name.clone(), y[at..at + p.shape.len()].to_vec());
        at += p.shape.len();
    }
    // The first of two equal peaks.
    assert_eq!(out["best"], [1.0]);
    assert_eq!(out["low"], [3.0]);
    assert_eq!(out["w_peak"], [20.0]);
    assert!(out["outside"][0].is_nan());
    assert_eq!(out["high"], [1.0]);
    // The axis numbers its cells; cell numbers have no unit.
    assert_eq!(out["k"], [0.0, 1.0, 2.0, 3.0]);
    let unit = |n: &str| {
        spec.ports
            .iter()
            .find(|p| p.name == n)
            .unwrap()
            .unit
            .to_string()
    };
    assert_eq!(unit("best"), "1");
    assert_eq!(unit("w_peak"), "1/s");
    // Refusals.
    let err = |lines: &[&str]| set(lines).err().unwrap().to_string();
    assert!(err(&["grid k [m] = 3", "input a[k]", "b = sum(a)"]).contains("no unit"));
    assert!(err(&["grid k = 3", "input a[k], i[k]", "b = at(a, i)"]).contains("one value"));
    assert!(err(&["grid k = 3", "input a[k] [m]", "b = at(a, a)"]).contains("one value"));
}

#[test]
fn heat_diffuses_and_is_conserved_with_insulated_edges() {
    use crate::ode::OdeBlock;
    use simsandbox_core::graph::Model;
    use simsandbox_core::sim::Simulator;
    let lines: Vec<String> = [
        "grid x [m] = 0 .. 1, 20",
        "param alpha [m^2/s] = 0.01",
        "state T[x] [K] = exp(-((x - 0.5)/0.1)^2 [m^2/m^2])",
        "dT/dt = alpha*lap(T)",
        "total = sum(T) [K]",
        "peak = maxof(T) [K]",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    // The unit annotation inside an expression is not allowed: a
    // dimensionless exponent is written without one.
    assert!(EquationSet::parse("rod", &lines).is_err());
    let mut lines = lines;
    lines[2] = "state T[x] [K] = exp(-((x - 0.5 [m])/0.1)^2)".into();
    assert!(EquationSet::parse("rod", &lines).is_err());
    lines[1] = "param alpha [m^2/s] = 0.01, w [m] = 0.1, c [m] = 0.5".into();
    assert!(EquationSet::parse("rod", &lines).is_err());
    let lines: Vec<String> = [
        "grid x [m] = 0 .. 1, 20",
        "param alpha [m^2/s] = 0.01",
        "param w [m] = 0.1",
        "param c [m] = 0.5",
        "param top [K] = 1",
        "state T[x] [K] = top*exp(-((x - c)/w)^2)",
        "dT/dt = alpha*lap(T)",
        "total = sum(T) [K]",
        "peak = maxof(T) [K]",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    let s = EquationSet::parse("rod", &lines).unwrap();
    let mut m = Model::new();
    let id = m.add(Box::new(OdeBlock::new(
        s,
        crate::integrators::Integrator::Rk4 { substeps: 1 },
    )));
    let mut sim = Simulator::new(m, 0.0).unwrap();
    let total0 = sim.output(id, "total").unwrap()[0];
    let peak0 = sim.output(id, "peak").unwrap()[0];
    for _ in 0..100 {
        sim.step(0.05).unwrap();
    }
    let total = sim.output(id, "total").unwrap()[0];
    let peak = sim.output(id, "peak").unwrap()[0];
    assert!(
        (total - total0).abs() < 1e-12 * total0,
        "{total0} -> {total}"
    );
    assert!(peak < 0.8 * peak0, "{peak0} -> {peak}");
}

#[test]
fn array_mistakes_are_explained() {
    let with = |extra: &[&str]| {
        let mut l = vec![
            "grid x [m] = 0 .. 1, 4",
            "state T[x] [K] = 0",
            "param k [1/s] = 0",
        ];
        l.extend_from_slice(extra);
        error(&l)
    };
    assert!(with(&["dT/dt = lap(2*T)"]).contains("takes the name of an array"));
    assert!(with(&["dT/dt = ddy(T)"]).contains("write ddx"));
    assert!(with(&["dT/dt = k*T", "a = at(T, 4)"]).contains("from 0 to 3"));
    assert!(with(&["dT/dt = k*T", "edge q: x- = 1"]).contains("not an array"));
    assert!(with(&["dT/dt = k*T", "edge T: x- = 1, x- gradient = 0"]).contains("two conditions"));
    assert!(with(&["dT/dt = k*T", "edge T: y- = 1"]).contains("an end is an axis of the grid (x)"));
    assert!(with(&["dT/dt = k*T", "s = sum(t)"]).contains("takes an array"));
    assert!(with(&["dT/dt = k*T", "state v", "dv/dt = T"]).contains("must be an array"));
    // Axes are independent (ADR 0080): T stays over x beside y.
    assert!(set(&[
        "grid x [m] = 0 .. 1, 4",
        "grid y [m] = 0 .. 1, 3",
        "state T[x] [K] = 0",
        "param k [1/s] = 0",
        "dT/dt = k*T"
    ])
    .is_ok());
    let m = with(&["dT/dt = k*T", "grid y [m] = 0 .. 1, 3", "edge T: y- = 1"]);
    assert!(m.contains("T is over x, so it has no edges along y"), "{m}");
    let m = with(&["grid y = 3", "dT/dt = k*T*y"]);
    assert!(
        m.contains("but T is over x: declare 'state T[x, y]'"),
        "{m}"
    );
    let m = with(&[
        "grid y = 3",
        "dT/dt = k*T",
        "state u[y, x] = 0",
        "du/dt = u",
    ]);
    assert!(
        m.contains("in the order of the grid lines: write u[x, y]"),
        "{m}"
    );
    // Quantities over three axes are ports of three axes (ADR 0081).
    let three = set(&[
        "grid x = 4",
        "grid y = 3",
        "grid z = 2",
        "state u[x, y, z] = 0",
        "du/dt = 0*u",
        "q = u*y*z",
    ])
    .unwrap();
    let q = three.spec().ports.iter().find(|p| p.name == "q").unwrap();
    assert_eq!(q.shape.dims(), Some(vec![4, 3, 2]));
    assert_eq!(q.axis_names, ["x", "y", "z"]);
    let m = with(&["grid y = 3", "dT/dt = k*T", "q = sum(T, y)"]);
    assert!(m.contains("not y"), "{m}");
    let m = with(&["grid y = 3", "dT/dt = k*T", "q = on(T, x, y)"]);
    assert!(m.contains("x has 4 cells and y has 3"), "{m}");
    let m = with(&["grid y = 3", "dT/dt = k*T", "q = at(T*y, 1)"]);
    assert!(m.contains("'at' takes 3 argument(s)"), "{m}");
    assert!(with(&["dT/dt = lap(T)"]).contains("dT/dt is in K/s but the right side is in K/m^2"));
    let m = with(&["dT/dt = k*T", "param g [K] = 1", "edge T: x- gradient = g"]);
    assert!(m.contains("edge gradient is in K/m"), "{m}");
    assert!(error(&["grid x [m] = 0 .. 1, 0", "y = 1"]).contains("at least 1"));
    assert!(error(&["grid x [m] = 0 .. L(1), 4", "y = 1"]).contains("unknown function"));
    // Maps use grids too (ADR 0078): a scalar state stepped by an array
    // must be declared an array.
    assert!(error(&["grid z = 0 .. 1, 4", "w[n+1] = w + z"]).contains("declare 'state w[z]'"));
    assert!(set(&["grid z = 0 .. 1, 4", "state w[z] = z", "w[n+1] = w + z"]).is_ok());
    assert!(set(&["grid k = 3", "input c[k]", "x[n+1] = at(c, argminof(c))"]).is_ok());
    assert!(error(&["y = lap(T)"]).contains("declare a grid"));
    // Without a grid, brackets after a name are a unit as before.
    assert!(set(&["input F[N]", "y = F"]).is_ok());
}

#[test]
fn two_axes_make_a_2d_grid() {
    // 4 by 3 cells of 0.5 m by 1 m; u = cos(πx/2)·cos(πy/3), insulated
    // everywhere, is a mode of the Laplacian on this grid.
    let s = set(&[
        "grid x [m] = 0 .. 2, 4",
        "grid y [m] = 0 .. 3, 3",
        "param a [1/s] = 1",
        "param one [m^2] = 1",
        "param amp [K] = 1",
        "state u[x, y] [K] = amp*cos(pi*x/(2 [m]))",
        "du/dt = a*one*lap(u)",
        "corner = at(u, 3, 2)",
        "slope = ddy(u)",
    ]);
    // Units are not written inside expressions.
    assert!(s.is_err());
    let s = set(&[
        "grid x [m] = 0 .. lx, 4",
        "grid y [m] = 0 .. ly, 3",
        "param lx [m] = 2",
        "param ly [m] = 3",
        "param a [m^2/s] = 1",
        "param amp [K] = 1",
        "state u[x, y] [K] = amp*cos(pi*x/lx)*cos(pi*y/ly)",
        "edge u: y+ = 0",
        "du/dt = a*lap(u)",
        "corner = at(u, 3, 2)",
        "slope = ddy(u)",
    ])
    .unwrap();
    let spec = s.spec();
    let grid = Shape::Grid { nx: 4, ny: 3 };
    let outs: Vec<(&str, Shape)> = spec.outputs().map(|p| (p.name.as_str(), p.shape)).collect();
    assert_eq!(
        outs,
        [
            ("u", grid),
            ("corner", Shape::Scalar),
            ("slope", grid),
            ("x", Shape::Grid { nx: 4, ny: 1 }),
            ("y", Shape::Grid { nx: 3, ny: 1 })
        ]
    );
    let y = spec.ports.iter().find(|p| p.name == "y").unwrap();
    assert_eq!(y.coordinate[0].axis, 1);
    let p = [2.0, 3.0, 1.0, 1.0];
    let (u, du, out) = eval(&s, &p, &[]);
    // Cell centres: x at 0.25, 0.75, 1.25, 1.75 (fastest); y at 0.5, 1.5, 2.5.
    // Outputs: u (12), corner, slope (12), x (4), y (3): each axis over
    // itself alone (ADR 0080).
    assert_eq!(out.len(), 32);
    assert_eq!(out[25..29], [0.25, 0.75, 1.25, 1.75]);
    assert_eq!(out[29..32], [0.5, 1.5, 2.5]);
    let exact = |i: usize, j: usize| {
        ((i as f64 + 0.5) * 0.5 * PI / 2.0).cos() * ((j as f64 + 0.5) * PI / 3.0).cos()
    };
    assert!((u[4 * 2 + 3] - exact(3, 2)).abs() < 1e-15);
    assert_eq!(out[12], u[11]);
    // Interior rows (j = 0, 1 are away from the held y+ edge) decay
    // at the discrete rate of the mode along each axis.
    let rate = |h: f64, k: f64| -4.0 / (h * h) * (k * h / 2.0).sin().powi(2);
    let r = rate(0.5, PI / 2.0) + rate(1.0, PI / 3.0);
    for k in 0..4 {
        assert!(
            (du[k] - r * u[k]).abs() < 1e-12,
            "cell {k}: {} vs {}",
            du[k],
            r * u[k]
        );
    }
    // The top row feels the edge held at 0 instead.
    assert!((du[8] - r * u[8]).abs() > 1e-3);
    // The structure: each cell depends on itself and its 4 neighbours
    // (fewer at the edges).
    use crate::ode::OdeSystem;
    let rows = s.sparsity().unwrap();
    assert_eq!(rows[5], [1, 4, 5, 6, 9]);
    assert_eq!(rows[0], [0, 1, 4]);
}

#[test]
fn named_axes_broadcast_reduce_and_relabel() {
    // ADR 0080: a over i and b over k make m over [i, k]; reductions
    // along one axis leave the others; on() meets m with itself.
    let s = set(&[
        "grid i = 2",
        "grid k = 3",
        "grid j = 2",
        "input a[i], b[k]",
        "m = a + b",
        "s = sum(m, i)",
        "r = sum(m, k)",
        "mk = mean(m, k)",
        "w = argmaxof(m, k)",
        "lo = minof(m, i)",
        "g = sum(m*on(m, i, j), k)",
        "c = at(g, 1, 0)",
        "tot = sum(m)",
        "e = at(m, 1, 2)",
    ])
    .unwrap();
    let shapes: Vec<(&str, usize)> = s
        .spec()
        .outputs()
        .map(|p| (p.name.as_str(), p.shape.len()))
        .collect();
    assert_eq!(
        shapes,
        [
            ("m", 6),
            ("s", 3),
            ("r", 2),
            ("mk", 2),
            ("w", 2),
            ("lo", 3),
            ("g", 4),
            ("c", 1),
            ("tot", 1),
            ("e", 1),
            ("i", 2),
            ("k", 3),
            ("j", 2)
        ]
    );
    let (_, _, y) = eval(&s, &[], &[1.0, 2.0, 10.0, 20.0, 30.0]);
    assert_eq!(y[..6], [11.0, 12.0, 21.0, 22.0, 31.0, 32.0]);
    assert_eq!(y[6..9], [23.0, 43.0, 63.0]);
    assert_eq!(y[9..11], [63.0, 66.0]);
    assert_eq!(y[11..13], [21.0, 22.0]);
    assert_eq!(y[13..15], [2.0, 2.0]);
    assert_eq!(y[15..18], [11.0, 21.0, 31.0]);
    assert_eq!(y[18..22], [1523.0, 1586.0, 1586.0, 1652.0]);
    assert_eq!(y[22..25], [1586.0, 129.0, 32.0]);
    assert_eq!(y[25..], [0.0, 1.0, 0.0, 1.0, 2.0, 0.0, 1.0]);
    // The axis port of k is a coordinate of m (its second axis) and s
    // (its first).
    let k = s.spec().ports.iter().find(|p| p.name == "k").unwrap();
    let of: Vec<(&str, usize)> = k
        .coordinate
        .iter()
        .map(|c| (c.port.as_str(), c.axis))
        .collect();
    assert_eq!(of, [("m", 1), ("s", 0), ("lo", 0)]);
}

#[test]
fn solve_finds_x_with_pivoting_by_axis_names() {
    // A over [i, j] (row i, column j); A x = b gives x over j, and a
    // vector over j solves the transposed system (ADR 0080).
    let s = set(&[
        "grid i = 3",
        "grid j = 3",
        "input A[i, j], b[i], c[j]",
        "x = solve(A, b)",
        "y = solve(A, c)",
        "res = sum(A*x, j) - b",
        "z = solve(0*A, b)",
        "rc = rcond(A)",
        "r0 = rcond(0*A)",
        "r1 = rcond(abs(i - j) < 0.5)",
    ])
    .unwrap();
    // The first column is 0, 1, 4: row 2 is the first pivot.
    let a = [[0.0, 1.0, 1.0], [1.0, 3.0, 2.0], [4.0, 0.0, 1.0]];
    let mut u: Vec<f64> = (0..9).map(|k| a[k % 3][k / 3]).collect();
    u.extend([5.0, 13.0, 7.0]); // A·(1, 2, 3)
    u.extend([14.0, 7.0, 8.0]); // Aᵀ·(1, 2, 3)
    let (_, _, out) = eval(&s, &[], &u);
    for (k, want) in [1.0, 2.0, 3.0].iter().enumerate() {
        assert!((out[k] - want).abs() < 1e-14, "x {:?}", &out[..3]);
        assert!((out[3 + k] - want).abs() < 1e-14, "y {:?}", &out[3..6]);
        assert!(out[6 + k].abs() < 1e-14, "residual {:?}", &out[6..9]);
        assert!(out[9 + k].is_nan(), "singular gives NaN");
    }
    // rcond: 1/22 for this A (numpy: 1/(‖A‖₁‖A⁻¹‖₁)), 0 when singular,
    // 1 for the identity.
    assert!((out[12] - 1.0 / 22.0).abs() < 1e-15, "{}", out[12]);
    assert_eq!(out[13], 0.0);
    assert_eq!(out[14], 1.0);
    // Units: x in b's unit over A's.
    let s = set(&[
        "grid i = 2",
        "grid j = 2",
        "input K[i, j] [N/m], F[i] [N]",
        "x = solve(K, F)",
    ])
    .unwrap();
    let x = s.spec().ports.iter().find(|p| p.name == "x").unwrap();
    assert_eq!(x.unit.dim, unit("m").dim);
    // Mistakes.
    let base = [
        "grid i = 3",
        "grid j = 3",
        "grid k = 2",
        "input A[i, j], b[i], v[k], w[i, k]",
    ];
    let err = |line: &str| {
        let mut l = base.to_vec();
        l.push(line);
        error(&l)
    };
    assert!(err("x = solve(w, b)").contains("same length, not over i, k"));
    assert!(err("x = solve(A, v)").contains("b over one of A's axes (i, j), not over k"));
    assert!(err("x = solve(A)").contains("a matrix and a vector"));
    assert!(err("x = rcond(w)").contains("same length, not over i, k"));
}

#[test]
fn dd_names_its_axis_and_shifts_name_theirs_over_two_axes() {
    let s = set(&[
        "grid x = 0 .. 4, 4",
        "grid y = 0 .. 3, 3",
        "state v[x, y] = x*y*y",
        "state T[x] = x*x",
        "dv/dt = 0*v",
        "dT/dt = 0*T",
        "a = ddy(v)",
        "b = dd(v, y)",
        "c = dd(T, x) - ddx(T)",
        "p = prev(T)",
        "q = prev(v, y)",
    ])
    .unwrap();
    let (_, _, out) = eval(&s, &[], &[]);
    // Outputs: v (12), T (4), a (12), b (12), c (4), p (4), q (12).
    assert_eq!(out[16..28], out[28..40]);
    assert!(out[40..44].iter().all(|&d| d == 0.0));
    let base = [
        "grid x = 0 .. 4, 4",
        "grid y = 0 .. 3, 3",
        "state v[x, y] = 0",
        "dv/dt = 0*v",
    ];
    let err = |line: &str| {
        let mut l = base.to_vec();
        l.push(line);
        error(&l)
    };
    let m = err("p = prev(v)");
    assert!(
        m.contains("v is over x, y: name the axis, prev(v, x) or prev(v, y)"),
        "{m}"
    );
    assert!(err("p = next(v)").contains("next(v, y)"));
    assert!(err("d = dd(v)").contains("an array and an axis"));
    assert!(err("d = dd(v, z)").contains("differentiates along an axis of the grid (x, y)"));
}

#[test]
fn states_over_some_of_the_axes() {
    // T over x alone beside v over [x, y] (ADR 0080).
    let s = set(&[
        "grid x = 0 .. 4, 4",
        "grid y = 0 .. 3, 3",
        "state T[x] = x",
        "state v[x, y] = x*y",
        "dT/dt = sum(v, y) - T",
        "dv/dt = T - y",
        "q = ddy(v)",
        "d = ddx(T)",
    ])
    .unwrap();
    assert_eq!(s.spec().states.len(), 16);
    let (x, dx, y) = eval(&s, &[], &[]);
    let xs = [0.5, 1.5, 2.5, 3.5];
    let ys = [0.5, 1.5, 2.5];
    assert_eq!(x[..4], xs);
    for (j, yv) in ys.iter().enumerate() {
        for (i, xv) in xs.iter().enumerate() {
            assert_eq!(x[4 + i + 4 * j], xv * yv);
            assert_eq!(dx[4 + i + 4 * j], xv - yv);
        }
    }
    assert_eq!(dx[..4], xs.map(|v| 3.5 * v));
    // Outputs: T (4), v (12), q (12), d (4), x (4), y (3).
    assert_eq!(y.len(), 39);
    // Along y in the middle row: (v(x, 2.5) - v(x, 0.5)) / 2 = x.
    assert_eq!(y[16 + 4..16 + 8], xs);
    assert_eq!(y[28 + 1..28 + 3], [1.0, 1.0]);
    assert_eq!(y[32..36], xs);
    assert_eq!(y[36..], ys);
}

#[test]
fn two_axes_need_the_same_unit_for_lap() {
    let m = error(&[
        "grid x [m] = 0 .. 1, 4",
        "grid y [s] = 0 .. 1, 4",
        "state u[x, y] = 0",
        "du/dt = lap(u)",
    ]);
    assert!(m.contains("the axes need the same unit"), "{m}");
    assert!(error(&[
        "grid x [m] = 0 .. 1, 4",
        "grid y [m] = 0 .. 1, 4",
        "state u[x, y] = 0",
        "du/dt = ddz(u)"
    ])
    .contains("not an axis of the grid (x, y)"));
    assert!(error(&[
        "grid x = 0 .. 1, 4",
        "grid y = 0 .. 1, 3",
        "state u[x, y] = 0",
        "du/dt = u",
        "c = at(u, 1, 3)"
    ])
    .contains("along y is a whole number from 0 to 2"));
}

/// Quads over [0, 2] × [0, 1] with their sides, coordinates in metres.
const QUADS: [&str; 8] = [
    "table pts",
    "state x[pts] [m]",
    "state y[pts] [m]",
    "x[n+1] = x",
    "y[n+1] = y",
    "cells c of pts: quad",
    "start c = grid x 0 .. 2 by 2, y 0 .. 1 by 1",
    "sides f of c",
];

fn with<'a>(base: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(extra).copied().collect()
}

#[test]
fn a_measure_is_in_the_coordinates_unit_to_the_cells_dimension() {
    let s = set(&with(
        &QUADS,
        &[
            "A = measure(c, x, y) [m^2]",
            "L = measure(f, x, y) [m]",
            "V = sum(measure(c, x, y))",
            "nx = normal(f, 0, x, y) + 1",
            "xc = centre(x, c) [m]",
        ],
    ))
    .unwrap();
    // Without a unit written, the right side's.
    let v = s.spec().ports.iter().find(|p| p.name == "V").unwrap();
    assert_eq!(v.unit.dim, unit("m^2").dim);
    let e = error(&with(&QUADS, &["A = measure(c, x, y) [m]"]));
    assert!(e.contains("A is in m but the right side is in m^2"), "{e}");
    // A segment's side is a point, which counts 1.
    set(&[
        "table pts",
        "state x[pts] [m]",
        "x[n+1] = x",
        "cells c of pts: segment",
        "start c = grid x 0 .. 1 by 2",
        "sides p of c",
        "L = measure(c, x) [m]",
        "k = measure(p, x) + 1",
    ])
    .unwrap();
    // The coordinates share one unit.
    let mut mixed = QUADS.to_vec();
    mixed[2] = "state y[pts] [s]";
    let e = error(&with(&mixed, &["A = measure(c, x, y)"]));
    assert!(
        e.starts_with("line 9, column 19: 'measure' takes coordinates in one unit, not m and s"),
        "{e}"
    );
    let e = error(&with(&mixed, &["nx = normal(f, 0, x, y)"]));
    assert!(
        e.contains("'normal' takes coordinates in one unit, not m and s"),
        "{e}"
    );
}

#[test]
fn a_row_s_id_has_no_unit() {
    let rows = [
        "table pts",
        "start pts = 2",
        "state a[pts] [K] = 1",
        "a[n+1] = a",
    ];
    set(&with(&rows, &["param i = 1", "b = of(a, i) [K]"])).unwrap();
    let e = error(&with(&rows, &["param i [m] = 1", "b = of(a, i)"]));
    assert!(
        e.starts_with("line 6, column 5: a row's id has no unit"),
        "{e}"
    );
}

#[test]
fn names_used_only_in_rules_are_parameters_after_the_others() {
    let s = set(&[
        "table pts max 12",
        "start pts = 3",
        "state age[pts] = id(pts)",
        "age[n+1] = age + step",
        "remove pts where age > lim",
        "add pts where rows(pts) < low: age = fresh",
        "edges segs of pts",
        "start segs = chain",
        "split segs where 0: age = copy",
    ])
    .unwrap();
    let names: Vec<&str> = s.spec().params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["step", "lim", "low", "fresh"]);
    assert!(s.spec().params.iter().all(|p| p.default == 1.0));
}

#[test]
fn a_parameter_over_rows_holds_beside_states_however_written() {
    for rate in ["x' = -g*x", "d x / dt = -g*x", "dx/dt = -g*x"] {
        let s = set(&[
            "table pts",
            "start pts = 2",
            "state x[pts] = 1",
            rate,
            "param g[pts] = 2",
        ])
        .unwrap_or_else(|e| panic!("{rate}: {e}"));
        assert_eq!(s.tables()[0].columns, ["x", "g"], "{rate}");
    }
    // A mistake on the line that holds it is one of its `param` line.
    let e = error(&[
        "table pts",
        "start pts = 2",
        "state x[pts] = 1",
        "x' = -x",
        "param g[pts] = 2",
        "g = 3",
    ]);
    assert_eq!(e, "line 5, column 1: 'g' is defined twice");
}

#[test]
fn a_map_s_rules_read_the_present_value_as_its_equations_do() {
    let lines = |now: &str| {
        [
            "table pts max 12".to_string(),
            "start pts = 3".to_string(),
            "state age[pts] = id(pts)".to_string(),
            format!("age[n+1] = {now} + 1"),
            format!("remove pts where {now} > 6"),
            format!("add pts from pts where {now} < 1.5: age = {now}/2"),
        ]
    };
    let a = EquationSet::parse("m", &lines("age[n]")).unwrap();
    let b = EquationSet::parse("m", &lines("age")).unwrap();
    assert_eq!(a.code(), b.code());
    // Columns stay put.
    let e = error(&[
        "table pts",
        "start pts = 3",
        "state age[pts]",
        "age[n+1] = age",
        "remove pts where age[n] > sum(2)",
    ]);
    assert!(e.starts_with("line 5, column 27:"), "{e}");
    let e = error(&[
        "table pts",
        "start pts = 3",
        "state age[pts]",
        "age[n+1] = age",
        "remove pts where age > n",
    ]);
    assert!(e.contains("'n' is the step number"), "{e}");
}

#[test]
fn a_grid_starts_cells_not_edges() {
    let e = error(&[
        "table pts",
        "start pts = 3",
        "state p[pts]",
        "p[n+1] = p",
        "edges segs of pts",
        "start segs = grid x 0 .. 1 by 2",
    ]);
    assert!(
        e.starts_with(
            "line 6, column 1: 'segs' is an edge table, which starts as a chain or from a file"
        ),
        "{e}"
    );
    // Cells that are segments start from a grid.
    set(&[
        "table pts",
        "state x[pts]",
        "x[n+1] = x",
        "cells segs of pts: segment",
        "start segs = grid x 0 .. 1 by 2",
    ])
    .unwrap();
}

#[test]
fn rows_have_no_spacing_and_a_normal_s_component_is_a_whole_number() {
    let e = error(&with(&QUADS, &["h = spacing(pts)"]));
    assert!(
        e.starts_with("line 9, column 5: 'spacing' takes a grid's axis; pts is a table"),
        "{e}"
    );
    let e = error(&with(&QUADS, &["k = normal(f, 0.7, x, y)"]));
    assert!(
        e.contains("a normal's component is a whole number from 0, not 0.7"),
        "{e}"
    );
    set(&with(&QUADS, &["k = normal(f, 1, x, y)"])).unwrap();
}

#[test]
fn a_table_has_one_start() {
    let e = error(&[
        "table pts",
        "start pts = 2",
        "start pts = 3",
        "state a[pts]",
        "a[n+1] = a",
    ]);
    assert_eq!(
        e,
        "line 3, column 1: pts has two starts, on lines 2 and 3: give it one"
    );
}

#[test]
fn element_names_are_as_format_writes_them() {
    for k in [0, 1, 9, 10, 99, 100, 262_143, 1_000_000, usize::MAX] {
        assert_eq!(element_name("T", k), format!("T[{k}]"));
    }
    assert_eq!(element_name("θ_1", 42), "θ_1[42]");
}

/// Three mistakes found while the front end was taken apart: each is an
/// error at its line, where one stopped the program and two were taken
/// silently.
#[test]
fn more_points_than_a_table_holds_and_names_given_twice_are_errors() {
    let error = |lines: &[&str]| {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        match EquationSet::parse("b", &lines) {
            Err(e) => e.to_string().replace("invalid definition: ", ""),
            Ok(_) => "accepted".into(),
        }
    };
    // A starting grid of 3 × 3 points over a table of at most 3 rows.
    let e = error(&[
        "table pts max 3",
        "state x[pts]",
        "state y[pts]",
        "x[n+1] = x",
        "y[n+1] = y",
        "cells c of pts: quad",
        "start c = grid x 0 .. 1 by 2, y 0 .. 1 by 2",
    ]);
    assert!(
        e.ends_with(
            "line 7, column 1: pts starts with 9 rows (the points of c's starting mesh) but \
             has at most 3"
        ),
        "{e}"
    );
    // A parameter per cell declared twice, and defined as a formula too.
    let e = error(&[
        "grid k = 2",
        "param m[k] = 1",
        "param m[k] = 1",
        "y = sum(m)",
    ]);
    assert!(
        e.ends_with("line 3, column 1: 'm' is declared twice"),
        "{e}"
    );
    let e = error(&["grid k = 2", "param m[k] = 1", "m = 3"]);
    assert!(e.ends_with("line 3, column 1: 'm' is defined twice"), "{e}");
    // What stays allowed: the table large enough, the parameter once.
    assert_eq!(
        error(&["grid k = 2", "param m[k] = 1", "y = sum(m)"]),
        "accepted"
    );
}

/// A rule is held to units as an equation is (where any line has one):
/// its condition compares like with like and is a number without a unit,
/// and each value is in its column's unit.
#[test]
fn rules_balance_like_equations() {
    let lines = |rule: &str| {
        let mut l = vec![
            "table pts max 8",
            "start pts = 3",
            "state age[pts] [s] = 0",
            "param tick [s] = 1",
            "param life [s] = 6",
            "param reach [m] = 2",
            "age[n+1] = age + tick",
        ];
        l.push(rule);
        l.iter().map(|s| s.to_string()).collect::<Vec<String>>()
    };
    let error = |rule: &str| match EquationSet::parse("m", &lines(rule)) {
        Err(SimError::InvalidDefinition(m)) => m,
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => "accepted".into(),
    };
    assert_eq!(error("remove pts where age > life"), "accepted");
    assert_eq!(
        error("remove pts where age > reach"),
        "line 8, column 24: cannot compare s and m"
    );
    assert_eq!(
        error("remove pts where age"),
        "line 8, column 18: a rule's condition holds or not: a comparison, or a number \
         without a unit, not a value in s"
    );
    assert_eq!(
        error("add pts where rows(pts) < 4: age = life/2"),
        "accepted"
    );
    assert_eq!(
        error("add pts where rows(pts) < 4: age = reach"),
        "line 8, column 36: age's value is in s but the right side is in m"
    );
    // A plain number is in its column's unit, as an initial value is.
    assert_eq!(error("add pts where rows(pts) < 4: age = 2"), "accepted");
    // Without units nothing is checked, as for equations.
    let plain = [
        "table pts",
        "start pts = 3",
        "state age[pts] = 0",
        "age[n+1] = age + 1",
        "remove pts where age",
    ];
    assert!(set(&plain).is_ok());
}

/// `copy`, `share`, `mean` and `sum` written alone are transfers; a
/// quantity of one of those names is given in parentheses, as any
/// expression is, and is a value like any other.
#[test]
fn a_quantity_named_like_a_transfer_is_written_in_parentheses() {
    let cells = |value: &str| {
        let lines: Vec<String> = [
            "table pts",
            "state x[pts]",
            "state y[pts]",
            "x[n+1] = x",
            "y[n+1] = y",
            "cells c of pts: triangle",
            "start c = grid x 0 .. 1 by 2, y 0 .. 1 by 2",
            "state u[c] = 1",
            "u[n+1] = u",
            "param share = 0.25",
            &format!("split c where u > 0.7: u = {value}"),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let set = EquationSet::parse("m", &lines).unwrap();
        let cell_transfer = set.rules[0]
            .transfers
            .iter()
            .find(|t| t.0)
            .map(|t| t.2)
            .unwrap();
        let params: Vec<String> = set.spec().params.iter().map(|p| p.name.clone()).collect();
        (cell_transfer, params)
    };
    let (bare, params) = cells("share");
    assert!(matches!(bare, CellTransfer::Share), "{bare:?}");
    assert!(params.contains(&"share".to_string()));
    let (quantity, _) = cells("(share)");
    assert!(matches!(quantity, CellTransfer::Given(_)), "{quantity:?}");
    // Over an edge table's points the same holds; run in
    // `simsandbox-session`'s tests (`tables.rs`).
}

/// Of several wrong `start` lines, the first written is told, whatever
/// their kinds and however the starts are kept.
#[test]
fn the_first_wrong_start_is_the_one_told() {
    let lines = [
        "table pts",
        "grid k = 3",
        "start k = 4",
        "start pts = chain",
        "start nowhere = grid x 0 .. 1 by 2",
        "state a[pts] = 0",
        "a[n+1] = a",
    ];
    let first = error(&lines);
    assert!(
        first.starts_with("line 3, column 1: 'k' is not a table"),
        "{first}"
    );
    for _ in 0..20 {
        assert_eq!(error(&lines), first);
    }
    // Two meshes started wrongly: the first by line.
    let meshes = [
        "table pts",
        "state x[pts]",
        "x[n+1] = x",
        "cells b of pts: triangle",
        "cells a of pts: triangle",
        "start b = grid x 0 .. 1 by 2",
        "start a = grid x 0 .. 1 by 2",
    ];
    let first = error(&meshes);
    assert!(first.starts_with("line 6, column 1:"), "{first}");
    for _ in 0..20 {
        assert_eq!(error(&meshes), first);
    }
}

#[test]
fn every_array_function_has_a_name_and_only_those() {
    use super::functions::Function;
    for name in simsandbox_expr::ARRAY_OPS {
        assert!(Function::by_name(name).is_some(), "{name}");
    }
    for f in FUNCTIONS.iter().filter(|f| f.name != DD_AXIS) {
        assert!(Function::by_name(f.name).is_some(), "{}", f.name);
    }
    assert_eq!(Function::by_name("dd"), Some(Function::Dd));
    assert_eq!(
        Function::by_name("ddx"),
        Some(Function::DdAlong("x".into()))
    );
    // The `dd<axis>` rule is the parser's.
    for name in [
        "ddx", "dd_y", "ddé", "dd", "dd1", "d", "ddd", "sin", "copy", "max", "",
    ] {
        assert_eq!(
            Function::by_name(name).is_some(),
            simsandbox_expr::is_array_op(name),
            "{name}"
        );
    }
}
