//! Block types of the equation domain, registered for model files.

use crate::integrators::Integrator;
use crate::ode::OdeBlock;
use crate::systems::{CoupledOscillator, Lorenz};
use simsandbox_core::block::Block;
use simsandbox_core::registry::{MethodSettings, Registry};
use simsandbox_core::Result;

/// The Lorenz system ([`Lorenz`]).
pub const LORENZ: &str = "eq.lorenz";
/// One coupled oscillator ([`CoupledOscillator`]); its initial position is a
/// parameter (`x0`, default 0).
pub const COUPLED_OSCILLATOR: &str = "eq.coupled_oscillator";

/// Register every equation-domain block type.
pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(LORENZ, |name, m| ode_block(Lorenz::new(name), m))?;
    registry.register(COUPLED_OSCILLATOR, |name, m| {
        ode_block(CoupledOscillator::new(name, 0.0), m)
    })?;
    Ok(())
}

fn ode_block(
    system: impl crate::OdeSystem + 'static,
    method: &MethodSettings,
) -> Result<Box<dyn Block>> {
    Ok(Box::new(OdeBlock::new(
        system,
        Integrator::from_settings(method)?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let b = r.build(LORENZ, "l", &MethodSettings::new("rk4")).unwrap();
        assert_eq!(b.spec().name, "l");
        assert!(matches!(
            r.build("eq.nope", "x", &MethodSettings::new("rk4")),
            Err(SimError::UnknownBlockType(_))
        ));
    }
}
