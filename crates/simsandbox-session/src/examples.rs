//! Example models built into the app, so they work wherever it is installed.

use simsandbox_core::registry::Registry;
use simsandbox_views::ViewRegistry;

pub struct Example {
    pub name: &'static str,
    pub text: &'static str,
    /// The model's file name and the files it reads from its folder (a
    /// table's starting rows, an array's values: ADR 0069 step 3), by name
    /// and contents; none for most examples.
    pub files: &'static [(&'static str, &'static str)],
}

impl Example {
    /// Write the model (as `file_name`) and its files into `dir`, made if
    /// needed; the model's path.
    pub fn unpack(&self, dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)?;
        for (name, text) in self.files {
            std::fs::write(dir.join(name), text)?;
        }
        let model = dir.join(self.file_name());
        std::fs::write(&model, self.text)?;
        Ok(model)
    }

    /// A file name for the model, from its name: lower case, words joined
    /// by dashes.
    pub fn file_name(&self) -> String {
        let words: Vec<String> = self
            .name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        format!("{}.ssbx.json", words.join("-"))
    }

    /// A folder holding the example and its files, written once per
    /// process under the system's temporary folder; `None` when it reads
    /// no files. For loading it headless: `registry::in_folder(e.folder()…)`.
    pub fn folder(&self) -> Option<std::path::PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        let dir = std::env::temp_dir()
            .join(format!("simsandbox-examples-{}", std::process::id()))
            .join(self.file_name().trim_end_matches(".ssbx.json"));
        if !dir.join(self.file_name()).exists() {
            self.unpack(&dir).ok()?;
        }
        Some(dir)
    }
}

pub const EXAMPLES: &[Example] = &[
    Example {
        name: "Lorenz system",
        text: include_str!("../../../examples/lorenz.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Coupled oscillators",
        text: include_str!("../../../examples/coupled-oscillators.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Predator and prey (typed equations)",
        text: include_str!("../../../examples/predator-prey.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Cruise control (a controller with feedthrough)",
        text: include_str!("../../../examples/cruise-control.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Steer the car with keys (input)",
        text: include_str!("../../../examples/cruise-steer.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Tuning a controller's gain (a study of a model)",
        text: include_str!("../../../examples/cruise-tuning.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Searching for a controller's best gains (a strategy model)",
        text: include_str!("../../../examples/cruise-search.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Fitting a model to measurements (Levenberg–Marquardt as a model)",
        text: include_str!("../../../examples/oscillator-fit.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Following a branch through hysteresis (continuation as a model)",
        text: include_str!("../../../examples/bistable-continuation.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Finding where a state's fate changes (refinement as a model)",
        text: include_str!("../../../examples/bistable-refinement.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Units (a shaker in mm drives springs in m)",
        text: include_str!("../../../examples/units.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Platoon (a model defined once, used three times)",
        text: include_str!("../../../examples/platoon.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Forced spring (a parameter driven by a wire)",
        text: include_str!("../../../examples/forced-spring.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Logistic map (a discrete map)",
        text: include_str!("../../../examples/logistic-map.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Resonance curve (a study of formulas)",
        text: include_str!("../../../examples/resonance.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Resonance map (a study over two parameters)",
        text: include_str!("../../../examples/resonance-map.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Resonance samples (a Sobol study)",
        text: include_str!("../../../examples/resonance-samples.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Pendulum (with its own view)",
        text: include_str!("../../../examples/pendulum.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Drag the bob (input from a view)",
        text: include_str!("../../../examples/pendulum-drag.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Heat along a rod (a grid field)",
        text: include_str!("../../../examples/heat-rod.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Typed heat equation (arrays on a grid)",
        text: include_str!("../../../examples/heat-rod-typed.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Gray–Scott spots (reaction–diffusion on a 2D grid)",
        text: include_str!("../../../examples/gray-scott.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Orbiting particles (a point set)",
        text: include_str!("../../../examples/orbits.ssbx.json"),
        files: &[],
    },
    Example {
        name: "Sheep grazing (agents and an ODE)",
        text: include_str!("../../../examples/grazing.ssbx.json"),
        files: &[],
    },
    Example {
        name: "A Wheatstone bridge (a network from files)",
        text: include_str!("../../../examples/networks/bridge.ssbx.json"),
        files: &[
            (
                "nodes.csv",
                include_str!("../../../examples/networks/nodes.csv"),
            ),
            (
                "wires.csv",
                include_str!("../../../examples/networks/wires.csv"),
            ),
        ],
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
            let folder = e.folder();
            let loaded = simsandbox_file::load_any(e.text, &r, folder.as_deref())
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
            let ran = simsandbox_core::registry::in_folder(folder.as_deref(), || {
                crate::run::run(e.text, &r, Some(0.5), None)
                    // A swept model takes no end time: run it as written.
                    .or_else(|_| crate::run::run(e.text, &r, None, None))
            });
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
