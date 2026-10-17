//! Block types of the equation domain, registered for model files.

use crate::integrators::Integrator;
use crate::ode::OdeBlock;
use crate::symbolic::{EquationSet, ExplicitBlock, Kind};
use crate::systems::{CoupledOscillator, Lorenz};
use simsandbox_core::block::Block;
use simsandbox_core::registry::{BlockConfig, Registry};
use simsandbox_core::{Result, SimError};

/// The Lorenz system ([`Lorenz`]).
pub const LORENZ: &str = "eq.lorenz";
/// One coupled oscillator ([`CoupledOscillator`]); its initial position is a
/// parameter (`x0`, default 0).
pub const COUPLED_OSCILLATOR: &str = "eq.coupled_oscillator";
/// Equations typed as text ([`EquationSet`]); the definition lines are the
/// equations.
pub const EQUATIONS: &str = "eq.equations";

/// Register every equation-domain block type.
pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(LORENZ, |name, c| {
        fixed(LORENZ, c)?;
        ode_block(Lorenz::new(name), c)
    })?;
    registry.register(COUPLED_OSCILLATOR, |name, c| {
        fixed(COUPLED_OSCILLATOR, c)?;
        ode_block(CoupledOscillator::new(name, 0.0), c)
    })?;
    registry.register(EQUATIONS, |name, c| {
        let set = EquationSet::parse(name, &c.definition)?;
        match set.kind() {
            Kind::Continuous => ode_block(set, c),
            Kind::Map | Kind::Formulas => Ok(Box::new(ExplicitBlock::new(set, &c.method)?)),
        }
    })?;
    Ok(())
}

/// Types with built-in equations take no definition lines.
fn fixed(type_name: &str, c: &BlockConfig) -> Result<()> {
    if c.definition.is_empty() {
        Ok(())
    } else {
        Err(SimError::InvalidDefinition(format!(
            "{type_name} has built-in equations; use {EQUATIONS} to type your own"
        )))
    }
}

fn ode_block(system: impl crate::OdeSystem + 'static, c: &BlockConfig) -> Result<Box<dyn Block>> {
    Ok(Box::new(OdeBlock::new(
        system,
        Integrator::from_settings(&c.method)?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_core::registry::MethodSettings;
    use simsandbox_core::SimError;

    #[test]
    fn settings_round_trip_and_reject_bad_options() {
        for i in [
            Integrator::Euler { substeps: 3 },
            Integrator::Rk4 { substeps: 1 },
            Integrator::Dopri5 {
                rtol: 1e-6,
                atol: 1e-9,
            },
        ] {
            assert_eq!(Integrator::from_settings(&i.settings()).unwrap(), i);
        }
        assert_eq!(
            Integrator::from_settings(&MethodSettings::new("rk4")).unwrap(),
            Integrator::Rk4 { substeps: 1 }
        );
        for bad in [
            MethodSettings::new("rk5"),
            MethodSettings::new("rk4").with("rtol", 1e-6),
            MethodSettings::new("rk4").with("substeps", 0.5),
            MethodSettings::new("euler").with("substeps", 0.0),
            MethodSettings::new("dopri5").with("atol", -1.0),
        ] {
            assert!(matches!(
                Integrator::from_settings(&bad),
                Err(SimError::InvalidMethod(_))
            ));
        }
    }

    #[test]
    fn registered_types_build_blocks() {
        let mut r = Registry::new();
        register(&mut r).unwrap();
        assert!(register(&mut r).is_err(), "double registration must fail");
        let rk4 = BlockConfig {
            method: MethodSettings::new("rk4"),
            definition: Vec::new(),
        };
        let b = r.build(LORENZ, "l", &rk4).unwrap();
        assert_eq!(b.spec().name, "l");
        // Typed equations need definition lines; fixed types refuse them.
        let typed = BlockConfig {
            definition: vec!["dx/dt = -k*x".into()],
            ..rk4.clone()
        };
        assert_eq!(
            r.build(EQUATIONS, "m", &typed).unwrap().spec().states,
            ["x"]
        );
        assert!(matches!(
            r.build(EQUATIONS, "m", &rk4),
            Err(SimError::InvalidDefinition(_))
        ));
        assert!(matches!(
            r.build(LORENZ, "l", &typed),
            Err(SimError::InvalidDefinition(_))
        ));
        assert!(matches!(
            r.build("eq.nope", "x", &rk4),
            Err(SimError::UnknownBlockType(_))
        ));
    }
}
