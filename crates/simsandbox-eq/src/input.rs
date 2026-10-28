//! Input blocks (ADR 0058): values set from outside a running model — a
//! slider, keys, a view's pointer, a schedule or a replayed recording —
//! wired to other blocks like any output. Each is stateless; its outputs
//! are its own parameters, so snapshots, rewind, sliders and formulas cover
//! them with nothing new in the kernel. What an input does (a pointer
//! becoming a force) is written in the model, where it is typed and
//! unit-checked.

use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, Equations, MethodLabels, MethodSpec, ParamSpec,
    PortSpec, Shape, SpeedClass, StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::BlockConfig;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};

/// One value with a unit: a throttle, a steering angle, a push.
pub const VALUE: &str = "input.value";
/// A pointer in a view's coordinates: `x`, `y` and `down`.
pub const POINTER: &str = "input.pointer";
/// The one method: the value is held over each step.
pub const HELD: &str = "held";

/// An input block: its outputs are its parameters, in order.
pub struct Input {
    spec: BlockSpec,
    method: MethodSpec,
}

fn value_param(name: &str, unit: Unit, range: [f64; 2], description: &str) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        unit,
        default: 0.0_f64.clamp(range[0], range[1]),
        suggested_min: range[0],
        suggested_max: range[1],
        description: description.into(),
        initial_condition: false,
    }
}

impl Input {
    fn with(name: &str, params: Vec<ParamSpec>) -> Self {
        let ports = params
            .iter()
            .map(|p| PortSpec::output(&p.name, p.unit, Shape::Scalar))
            .collect();
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params,
                states: Vec::new(),
                tables: Vec::new(),
                time: TimeSemantics::Stateless,
            },
            method: MethodSpec {
                name: HELD.into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    // Exact, but Verified is earned per case (ADR 0061).
                    accuracy: AccuracyClass::Quantitative,
                },
            },
        }
    }

    /// `input.value`: the output `y`, in `unit`, suggested within `range`.
    pub fn value(name: &str, unit: Unit, range: [f64; 2]) -> Self {
        let p = value_param("y", unit, range, "The input's value, set from outside");
        Self::with(name, vec![p])
    }

    /// `input.pointer`: `x` and `y` in `unit` within `range`, and `down`
    /// (1 while pressed, else 0).
    pub fn pointer(name: &str, unit: Unit, range: [f64; 2]) -> Self {
        Self::with(
            name,
            vec![
                value_param("x", unit, range, "Pointer position across the view"),
                value_param("y", unit, range, "Pointer position up the view"),
                value_param(
                    "down",
                    Unit::DIMENSIONLESS,
                    [0.0, 1.0],
                    "1 while the pointer is pressed, else 0",
                ),
            ],
        )
    }

    /// From a file: the settings `unit` (dimensionless for a value, `m`
    /// for a pointer, if absent), `min` and `max` (the suggested range).
    pub fn from_config(type_name: &str, name: &str, c: &BlockConfig) -> Result<Self> {
        if let Some(k) = c
            .settings
            .keys()
            .find(|k| !matches!(k.as_str(), "unit" | "min" | "max"))
        {
            return Err(SimError::InvalidSetting {
                setting: format!("settings.{k}"),
                reason: format!("{type_name} has the settings 'unit', 'min' and 'max'"),
            });
        }
        if c.method.name != HELD || !c.method.options.is_empty() {
            return Err(SimError::InvalidMethod(format!(
                "an input is held over each step: its method is '{HELD}', with no options"
            )));
        }
        if !c.definition.is_empty() || c.update.is_some() {
            return Err(SimError::InvalidDefinition(format!(
                "{type_name} takes only the settings 'unit', 'min' and 'max'"
            )));
        }
        let pointer = type_name == POINTER;
        let unit_text = c
            .settings
            .get("unit")
            .map_or(if pointer { "m" } else { "1" }, String::as_str);
        let unit = Unit::parse(unit_text).map_err(|e| SimError::InvalidSetting {
            setting: "settings.unit".into(),
            reason: e.to_string(),
        })?;
        let bound = |key: &str, default: f64| -> Result<f64> {
            match c.settings.get(key) {
                None => Ok(default),
                Some(t) => t
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| SimError::InvalidSetting {
                        setting: format!("settings.{key}"),
                        reason: format!("'{t}' is not a number"),
                    }),
            }
        };
        let range = [bound("min", -1.0)?, bound("max", 1.0)?];
        if range[0] >= range[1] {
            return Err(SimError::InvalidSetting {
                setting: "settings.max".into(),
                reason: "max must be greater than min".into(),
            });
        }
        Ok(if pointer {
            Self::pointer(name, unit, range)
        } else {
            Self::value(name, unit, range)
        })
    }
}

impl Block for Input {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t + ctx.dt, io)
    }

    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        io.outputs.copy_from_slice(io.params);
        Ok(())
    }

    /// An input joins a fused group it feeds (ADR 0051).
    fn as_equations(&self) -> Option<&dyn Equations> {
        Some(self)
    }
}

impl Equations for Input {
    fn rhs(&self, _t: f64, _x: &[f64], _u: &[f64], _p: &[f64], _dx: &mut [f64]) {}

    fn outputs(&self, _t: f64, _x: &[f64], _u: &[f64], p: &[f64], y: &mut [f64]) {
        y.copy_from_slice(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_core::graph::Model;
    use simsandbox_core::sim::Simulator;

    #[test]
    fn an_input_outputs_its_value_from_the_next_step() {
        let mut m = Model::new();
        let unit = Unit::parse("N").unwrap();
        let push = Input::value("push", unit, [-50.0, 50.0]);
        assert_eq!(push.spec.ports[0].unit, unit);
        let id = m.add(Box::new(push));
        let mut s = Simulator::new(m, 0.0).unwrap();
        assert_eq!(s.output(id, "y"), Some(&[0.0][..]));
        s.set_param(id, "y", 40.0).unwrap();
        s.step(0.1).unwrap();
        assert_eq!(s.output(id, "y"), Some(&[40.0][..]));
    }

    #[test]
    fn settings_give_the_unit_and_range() {
        let mut c = BlockConfig {
            method: simsandbox_core::registry::MethodSettings::new(HELD),
            ..Default::default()
        };
        c.settings.insert("min".into(), "-2".into());
        c.settings.insert("max".into(), "2".into());
        let p = Input::from_config(POINTER, "hand", &c).unwrap();
        let names: Vec<&str> = p.spec.ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["x", "y", "down"]);
        assert_eq!(p.spec.ports[0].unit, Unit::METER);
        assert_eq!(p.spec.params[0].suggested_max, 2.0);
        c.settings.insert("max".into(), "-3".into());
        assert!(Input::from_config(POINTER, "hand", &c).is_err());
        c.settings.insert("colour".into(), "red".into());
        assert!(Input::from_config(VALUE, "v", &c).is_err());
    }
}
