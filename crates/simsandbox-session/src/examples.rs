//! Example models built into the app, so they work wherever it is installed.

use simsandbox_core::registry::Registry;
use simsandbox_views::ViewRegistry;

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
    Example {
        name: "Predator and prey (typed equations)",
        text: include_str!("../../../examples/predator-prey.ssbx.json"),
    },
    Example {
        name: "Cruise control (a controller with feedthrough)",
        text: include_str!("../../../examples/cruise-control.ssbx.json"),
    },
    Example {
        name: "Steer the car with keys (input)",
        text: include_str!("../../../examples/cruise-steer.ssbx.json"),
    },
    Example {
        name: "Units (a shaker in mm drives springs in m)",
        text: include_str!("../../../examples/units.ssbx.json"),
    },
    Example {
        name: "Platoon (a subsystem used three times)",
        text: include_str!("../../../examples/platoon.ssbx.json"),
    },
    Example {
        name: "Logistic map (a discrete map)",
        text: include_str!("../../../examples/logistic-map.ssbx.json"),
    },
    Example {
        name: "Resonance curve (formulas, swept)",
        text: include_str!("../../../examples/resonance.ssbx.json"),
    },
    Example {
        name: "Resonance map (two parameters swept)",
        text: include_str!("../../../examples/resonance-map.ssbx.json"),
    },
    Example {
        name: "Resonance samples (a Sobol sweep)",
        text: include_str!("../../../examples/resonance-samples.ssbx.json"),
    },
    Example {
        name: "Pendulum (with its own view)",
        text: include_str!("../../../examples/pendulum.ssbx.json"),
    },
    Example {
        name: "Drag the bob (input from a view)",
        text: include_str!("../../../examples/pendulum-drag.ssbx.json"),
    },
    Example {
        name: "Heat along a rod (a grid field)",
        text: include_str!("../../../examples/heat-rod.ssbx.json"),
    },
    Example {
        name: "Orbiting particles (a point set)",
        text: include_str!("../../../examples/orbits.ssbx.json"),
    },
    Example {
        name: "Sheep grazing (agents and an ODE)",
        text: include_str!("../../../examples/grazing.ssbx.json"),
    },
];

/// Every block type the app can load.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).expect("equation types register once");
    simsandbox_agents::register(&mut r).expect("agent types register once");
    r
}

/// Every view the app can show: the built-in ones and the domains' own.
pub fn view_registry() -> ViewRegistry {
    let mut r = ViewRegistry::builtin();
    simsandbox_eq::register_views(&mut r).expect("equation views register once");
    r
}
