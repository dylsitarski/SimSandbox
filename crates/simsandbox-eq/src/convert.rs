//! `units.convert`: a signal from one unit to another (ADR 0045), the one
//! way units change between blocks, so every conversion shows on the
//! diagram. Its settings `from` and `to` are unit texts such as `km/h` and
//! `m/s`; they must measure the same kind of quantity.

use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, Shape, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::BlockConfig;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};

/// The type name.
pub const CONVERT: &str = "units.convert";
/// Its one method: the conversion is exact.
pub const EXACT: &str = "exact";

/// `y = factor · u`, from the input's unit to the output's.
pub struct Convert {
    spec: BlockSpec,
    method: MethodSpec,
    factor: f64,
}

impl Convert {
    pub fn new(name: &str, from: Unit, to: Unit) -> Result<Self> {
        let factor = from
            .conversion_to(&to)
            .ok_or_else(|| SimError::InvalidSetting {
                setting: "settings.to".into(),
                reason: format!(
                    "{from} and {to} measure different things; a conversion keeps the kind of \
                 quantity (use typed equations to compute a new one)"
                ),
            })?;
        Ok(Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![
                    PortSpec::input("u", from, Shape::Scalar),
                    PortSpec::output("y", to, Shape::Scalar).depends_on(&["u"]),
                ],
                params: Vec::new(),
                states: Vec::new(),
                tables: Vec::new(),
                time: TimeSemantics::Stateless,
            },
            method: MethodSpec {
                name: EXACT.into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Verified,
                },
            },
            factor,
        })
    }

    /// From a file's settings: `from` and `to`, each dimensionless if
    /// absent.
    pub fn from_config(name: &str, c: &BlockConfig) -> Result<Self> {
        if let Some(k) = c
            .settings
            .keys()
            .find(|k| !matches!(k.as_str(), "from" | "to"))
        {
            return Err(SimError::InvalidSetting {
                setting: format!("settings.{k}"),
                reason: "units.convert has the settings 'from' and 'to'".into(),
            });
        }
        if c.method.name != EXACT || !c.method.options.is_empty() {
            return Err(SimError::InvalidMethod(format!(
                "a unit conversion is exact: its method is '{EXACT}', with no options"
            )));
        }
        if !c.definition.is_empty() || c.update.is_some() {
            return Err(SimError::InvalidDefinition(
                "units.convert takes only the settings 'from' and 'to'".into(),
            ));
        }
        let unit = |key: &str| -> Result<Unit> {
            let text = c.settings.get(key).map_or("1", String::as_str);
            Unit::parse(text).map_err(|e| SimError::InvalidSetting {
                setting: format!("settings.{key}"),
                reason: e.to_string(),
            })
        };
        Self::new(name, unit("from")?, unit("to")?)
    }
}

impl Block for Convert {
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
        io.outputs[0] = self.factor * io.inputs[0];
        Ok(())
    }
}
