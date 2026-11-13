//! Compiled typed equations (ADR 0084): large arrays' phases run as
//! machine code giving the interpreter's bits, on any split among cores,
//! with parameters changed after compiling.

use simsandbox_core::block::Equations;
use simsandbox_eq::ode::OdeSystem;
use simsandbox_eq::symbolic::EquationSet;

fn gray_scott(n: usize) -> Vec<String> {
    [
        format!("grid x [m] = 0 .. size, {n}"),
        format!("grid y [m] = 0 .. size, {n}"),
        "param size [m] = 1.25".into(),
        "param Du [m^2/s] = 2e-5".into(),
        "param Dv [m^2/s] = 1e-5".into(),
        "param F [1/s] = 0.035".into(),
        "param k [1/s] = 0.065".into(),
        "param r [1/s] = 1".into(),
        "param w [m] = 0.04".into(),
        "param ax [m] = 0.55".into(),
        "param ay [m] = 0.6".into(),
        "state u[x, y] = 1 - 0.5*exp(-((x - ax)^2 + (y - ay)^2)/w^2)".into(),
        "state v[x, y] = 0.25*exp(-((x - ax)^2 + (y - ay)^2)/w^2)".into(),
        "edge u: x periodic, y periodic".into(),
        "edge v: x periodic, y periodic".into(),
        "reaction = r*u*v^2".into(),
        "du/dt = Du*lap(u) - reaction + F*(1 - u)".into(),
        "dv/dt = Dv*lap(v) + reaction - (F + k)*v".into(),
    ]
    .into()
}

/// Every kind of operation Gray–Scott does not use: value, gradient and
/// periodic edges, a first derivative, both shifts, a comparison, a
/// power, a function and an axis.
fn mixed(n: usize) -> Vec<String> {
    [
        format!("grid x [m] = 0 .. 1, {n}"),
        format!("grid y [m] = 0 .. 1, {n}"),
        "param D [m^2/s] = 0.01".into(),
        "param vel [m/s] = 0.2".into(),
        "param T0 [K] = 1".into(),
        "param left [K] = 1".into(),
        "param hot [K] = 1.2".into(),
        "param a [1/s] = 0.3".into(),
        "param g [K/(m*s)] = 0.1".into(),
        "param kx [1/m] = 6".into(),
        "param ky [1/m] = 4".into(),
        "state T[x, y] [K] = T0 + 0.5*T0*sin(kx*x)*cos(ky*y)".into(),
        "edge T: x- = left, x+ gradient = 0, y periodic".into(),
        "heat = a*T0*(T > hot) + a*T0*(T/T0)^1.5 - a*T0*sin(T/T0)".into(),
        "dT/dt = D*lap(T) - vel*ddx(T) + heat + a*(prev(T, y) - next(T, x)) + g*x".into(),
    ]
    .into()
}

fn bits(x: &[f64]) -> Vec<u64> {
    x.iter().map(|v| v.to_bits()).collect()
}

/// The model compiled and interpreted give the same rates and outputs,
/// with its parameters as given and all moved.
fn same_compiled(name: &str, lines: &[String]) {
    let compiled = EquationSet::parse(name, lines).expect("valid");
    assert!(compiled.compiled_phases() > 0, "{name}: compiled");
    let mut interpreted = EquationSet::parse(name, lines).expect("valid");
    interpreted.interpret();
    assert_eq!(interpreted.compiled_phases(), 0);
    let p: Vec<f64> = compiled.spec().params.iter().map(|q| q.default).collect();
    let states = compiled.spec().states.len();
    let mut x = vec![0.0; states];
    compiled.initial_state(&p, &mut x);
    let mut x_interpreted = vec![0.0; states];
    interpreted.initial_state(&p, &mut x_interpreted);
    assert_eq!(bits(&x), bits(&x_interpreted), "{name}: starting values");
    let outputs: usize = OdeSystem::spec(&compiled)
        .outputs()
        .map(|o| o.shape.len())
        .sum();
    // Moving a slider compiles nothing: the same code reads new values.
    let moved: Vec<f64> = p.iter().map(|v| v * 1.37 + 0.01).collect();
    for p in [&p, &moved] {
        let (mut a, mut b) = (vec![0.0; states], vec![0.0; states]);
        compiled.rhs(0.5, &x, &[], p, &mut a);
        interpreted.rhs(0.5, &x, &[], p, &mut b);
        assert_eq!(bits(&a), bits(&b), "{name}: rates");
        let (mut a, mut b) = (vec![0.0; outputs], vec![0.0; outputs]);
        compiled.outputs(0.5, &x, &[], p, &mut a);
        interpreted.outputs(0.5, &x, &[], p, &mut b);
        assert_eq!(bits(&a), bits(&b), "{name}: outputs");
    }
}

#[test]
fn compiled_phases_give_the_interpreters_bits() {
    // Large enough to be shared among cores: compiled code runs on
    // chunks starting anywhere in the grid.
    same_compiled("gs", &gray_scott(256));
    same_compiled("mixed", &mixed(64));
}
