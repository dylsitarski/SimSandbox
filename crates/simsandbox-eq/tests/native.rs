//! Native code for typed equations (ADR 0089): Rust generated from the
//! phases, built with this machine's toolchain, gives the interpreter's
//! bits. Skipped where `cargo` is not installed.

use simsandbox_core::block::Equations;
use simsandbox_eq::ode::OdeSystem;
use simsandbox_eq::symbolic::EquationSet;
use std::time::Duration;

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

/// Every kind of operation: edges of each kind, a first derivative, both
/// shifts, a comparison, a power, a function, an axis.
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

#[test]
fn native_code_gives_the_interpreters_bits() {
    let has_cargo = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_cargo {
        eprintln!("no cargo: native code not tested");
        return;
    }
    // A cache of its own, built afresh.
    let cache = std::env::temp_dir().join(format!("simsandbox-native-test-{}", std::process::id()));
    std::env::set_var("SIMSANDBOX_CACHE", &cache);
    std::env::remove_var("SIMSANDBOX_NATIVE");
    simsandbox_compiled::native::set_mode(simsandbox_compiled::native::Mode::Build);
    let models = [("gs", gray_scott(128)), ("mixed", mixed(64))];
    let sets: Vec<EquationSet> = models
        .iter()
        .map(|(name, lines)| EquationSet::parse(name, lines).expect("valid"))
        .collect();
    assert!(
        simsandbox_compiled::native::wait(Duration::from_secs(600)),
        "built in time"
    );
    for ((name, lines), set) in models.iter().zip(&sets) {
        let mut interpreted = EquationSet::parse(name, lines).expect("valid");
        interpreted.interpret();
        let p: Vec<f64> = set.spec().params.iter().map(|q| q.default).collect();
        let n = set.spec().states.len();
        let mut x = vec![0.0; n];
        interpreted.initial_state(&p, &mut x);
        let moved: Vec<f64> = p.iter().map(|v| v * 1.37 + 0.01).collect();
        // The first call checks the library against Cranelift's code; the
        // next run it alone.
        for p in [&p, &p, &moved] {
            let (mut a, mut b) = (vec![0.0; n], vec![0.0; n]);
            set.rhs(0.5, &x, &[], p, &mut a);
            interpreted.rhs(0.5, &x, &[], p, &mut b);
            assert_eq!(bits(&a), bits(&b), "{name}: rates");
        }
    }
    // `check` says so (ADR 0089).
    for (set, (name, _)) in sets.iter().zip(&models) {
        assert_eq!(set.code(), "native", "{name}");
    }
    let (ready, building, failed) = simsandbox_compiled::native::summary();
    assert!(ready >= 2, "libraries in use: {ready}");
    assert_eq!((building, failed), (0, 0));
    let _ = std::fs::remove_dir_all(&cache);
}
