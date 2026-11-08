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
        name: "Tuning a controller's gain (a study of a model)",
        text: include_str!("../../../examples/cruise-tuning.ssbx.json"),
    },
    Example {
        name: "Searching for a controller's best gains (a strategy model)",
        text: include_str!("../../../examples/cruise-search.ssbx.json"),
    },
    Example {
        name: "Fitting a model to measurements (Levenberg–Marquardt as a model)",
        text: include_str!("../../../examples/oscillator-fit.ssbx.json"),
    },
    Example {
        name: "Units (a shaker in mm drives springs in m)",
        text: include_str!("../../../examples/units.ssbx.json"),
    },
    Example {
        name: "Platoon (a model defined once, used three times)",
        text: include_str!("../../../examples/platoon.ssbx.json"),
    },
    Example {
        name: "Forced spring (a parameter driven by a wire)",
        text: include_str!("../../../examples/forced-spring.ssbx.json"),
    },
    Example {
        name: "Logistic map (a discrete map)",
        text: include_str!("../../../examples/logistic-map.ssbx.json"),
    },
    Example {
        name: "Resonance curve (a study of formulas)",
        text: include_str!("../../../examples/resonance.ssbx.json"),
    },
    Example {
        name: "Resonance map (a study over two parameters)",
        text: include_str!("../../../examples/resonance-map.ssbx.json"),
    },
    Example {
        name: "Resonance samples (a Sobol study)",
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
        name: "Typed heat equation (arrays on a grid)",
        text: include_str!("../../../examples/heat-rod-typed.ssbx.json"),
    },
    Example {
        name: "Gray–Scott spots (reaction–diffusion on a 2D grid)",
        text: include_str!("../../../examples/gray-scott.ssbx.json"),
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

/// For tests: the driven, damped spring of the resonance examples as a
/// definition with every quantity a parameter (`spring.F`, `m`, `k`, `c`,
/// `w`) and the outputs `spring.amplitude` and `spring.lag`.
#[cfg(test)]
pub(crate) fn spring() -> serde_json::Value {
    serde_json::json!({"blocks": [{"name": "spring", "type": "eq.equations", "equations": [
        "param F [N] = 1", "param m [kg] = 1", "param k [N/m] = 1",
        "param c [N*s/m] = 0.2", "param w [rad/s] = 1",
        "amplitude = F/sqrt((k - m*w^2)^2 + (c*w)^2) [m]",
        "lag = atan2(c*w, k - m*w^2) [rad]"]}]})
}

/// Every block type the app can load.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).expect("equation types register once");
    simsandbox_agents::register(&mut r).expect("agent types register once");
    crate::study::register(&mut r).expect("the study type registers once");
    r
}

/// Every view the app can show: the built-in ones and the domains' own.
pub fn view_registry() -> ViewRegistry {
    let mut r = ViewRegistry::builtin();
    simsandbox_eq::register_views(&mut r).expect("equation views register once");
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_is_current_and_runs() {
        // Every built-in example loads with nothing to convert and no
        // warnings, and runs (each model briefly).
        let r = registry();
        for e in EXAMPLES {
            let loaded = simsandbox_file::load_any(e.text, &r, None)
                .unwrap_or_else(|err| panic!("{}: {err}", e.name));
            assert!(
                loaded.conversions.is_empty(),
                "{}: {:?}",
                e.name,
                loaded.conversions
            );
            assert!(
                loaded.warnings.is_empty(),
                "{}: {:?}",
                e.name,
                loaded.warnings
            );
            let ran = crate::run::run(e.text, &r, Some(0.5), None);
            // A swept model takes no end time: run it as written.
            let ran = ran.or_else(|_| crate::run::run(e.text, &r, None, None));
            assert!(ran.is_ok(), "{}: {:?}", e.name, ran.err());
        }
    }

    #[test]
    fn the_tuning_example_picks_the_fastest_gain_under_the_limit() {
        let r = registry();
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.starts_with("Tuning"))
            .unwrap();
        let rows = crate::run::run(e.text, &r, None, None).unwrap();
        let ki = &rows
            .outputs
            .iter()
            .find(|o| o.name == "tuning.cruise_ki")
            .unwrap()
            .values;
        let reach = &rows
            .outputs
            .iter()
            .find(|o| o.name == "tuning.reach")
            .unwrap()
            .values;
        let peak = &rows
            .outputs
            .iter()
            .find(|o| o.name == "tuning.peak")
            .unwrap()
            .values;
        // The fastest run whose peak stays under 27 m/s, found by hand.
        let best = (0..ki.len())
            .filter(|&k| peak[k] <= 27.0 && !reach[k].is_nan())
            .min_by(|&a, &b| reach[a].total_cmp(&reach[b]))
            .unwrap();
        let note = rows
            .notes
            .iter()
            .find(|n| n.starts_with("best.best_ki = "))
            .unwrap();
        assert_eq!(
            note,
            &format!("best.best_ki = {} (evaluated once)", ki[best])
        );
        // Stronger integral action reaches sooner and peaks higher.
        assert!(reach[2] > reach[10] && peak[2] < peak[10]);
    }
}
