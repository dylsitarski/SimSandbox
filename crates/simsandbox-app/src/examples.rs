//! Example models built into the app, so they work wherever it is installed.

use simsandbox_core::registry::Registry;

pub struct Example {
    pub name: &'static str,
    pub text: &'static str,
}

pub const EXAMPLES: &[Example] = &[
    Example {
        name: "Lorenz system",
        text: include_str!("../../../examples/lorenz.ssbx.json"),
    },
    Example {
        name: "Coupled oscillators",
        text: include_str!("../../../examples/coupled-oscillators.ssbx.json"),
    },
];

/// Every block type the app can load.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).expect("equation types register once");
    r
}
