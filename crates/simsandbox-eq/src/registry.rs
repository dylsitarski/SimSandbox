//! Block types of the equation domain, registered for model files.

use crate::fields::{Grass, HeatRod, Orbits};
use crate::integrators::{GroupIntegrator, Integrator};
use crate::ode::OdeBlock;
use crate::symbolic::{EquationSet, ExplicitBlock, Kind};
use crate::systems::{CoupledOscillator, Lorenz, Pendulum};
use simsandbox_core::block::{Block, Domain};
use simsandbox_core::registry::{BlockConfig, Registry, TypeInfo};
use simsandbox_core::{Result, SimError};

/// The Lorenz system ([`Lorenz`]).
pub const LORENZ: &str = "eq.lorenz";
/// One coupled oscillator ([`CoupledOscillator`]); its initial position is a
/// parameter (`x0`, default 0).
pub const COUPLED_OSCILLATOR: &str = "eq.coupled_oscillator";
/// A damped pendulum ([`Pendulum`]); it has its own view, `eq.pendulum`.
pub const PENDULUM: &str = "eq.pendulum";
/// Heat along a rod, a grid output ([`HeatRod`]).
pub const HEAT_ROD: &str = "eq.heat_rod";
/// Grass on a field, the environment of the grazing model ([`Grass`]).
pub const GRASS: &str = "eq.grass";
/// Test particles around a central mass, a point-set output ([`Orbits`]).
pub const ORBITS: &str = "eq.orbits";
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
    registry.register(PENDULUM, |name, c| {
        fixed(PENDULUM, c)?;
        ode_block(Pendulum::new(name), c)
    })?;
    registry.register(HEAT_ROD, |name, c| {
        fixed(HEAT_ROD, c)?;
        ode_block(HeatRod::new(name), c)
    })?;
    registry.register(GRASS, |name, c| {
        fixed(GRASS, c)?;
        ode_block(Grass::new(name), c)
    })?;
    registry.register(ORBITS, |name, c| {
        fixed(ORBITS, c)?;
        ode_block(Orbits::new(name), c)
    })?;
    registry.register_group_solver(Domain::Equations, |m| {
        Ok(Box::new(GroupIntegrator::new(Integrator::from_settings(
            m,
        )?)))
    })?;
    registry.register(EQUATIONS, |name, c| {
        no_update(c)?;
        let set = EquationSet::parse(name, &c.definition)?;
        // No method given: the one the equations call for.
        let c = &BlockConfig {
            method: if c.method.name.is_empty() {
                set.kind().default_method()
            } else {
                c.method.clone()
            },
            ..c.clone()
        };
        match set.kind() {
            Kind::Continuous => ode_block(set, c),
            Kind::Map | Kind::Formulas => Ok(Box::new(ExplicitBlock::new(set, &c.method)?)),
        }
    })?;
    registry.register(crate::convert::CONVERT, |name, c| {
        Ok(Box::new(crate::convert::Convert::from_config(name, c)?))
    })?;
    registry.register(crate::input::VALUE, |name, c| {
        let ty = crate::input::VALUE;
        Ok(Box::new(crate::input::Input::from_config(ty, name, c)?))
    })?;
    registry.register(crate::input::POINTER, |name, c| {
        let ty = crate::input::POINTER;
        Ok(Box::new(crate::input::Input::from_config(ty, name, c)?))
    })?;
    describe(registry)
}

/// The ODE methods every continuous type accepts.
fn ode(summary: &str) -> TypeInfo {
    TypeInfo::new(summary)
        .method("rk4", &["substeps"])
        .method("euler", &["substeps"])
        .method("dopri5", &["rtol", "atol"])
}

/// What each type is, for discovery (ADR 0039).
fn describe(r: &mut Registry) -> Result<()> {
    r.describe(
        LORENZ,
        ode("The Lorenz system: dx/dt = σ(y − x), dy/dt = x(ρ − z) − y, dz/dt = xy − βz; output xyz."),
    )?;
    r.describe(
        COUPLED_OSCILLATOR,
        ode("A mass on a spring coupled to a neighbour: input neighbor_x, outputs x and v."),
    )?;
    r.describe(
        PENDULUM,
        ode("A damped pendulum; outputs the bob's position and the angle. Has its own view, eq.pendulum."),
    )?;
    r.describe(
        HEAT_ROD,
        ode("Heat along a rod with held ends (method of lines); outputs temperature and position as one-row grids."),
    )?;
    r.describe(
        GRASS,
        ode("Grass regrowing logistically on a 40 × 40 field, grazed at a per-cell rate (input grazing); the grazing model's environment."),
    )?;
    r.describe(
        ORBITS,
        ode("Test particles orbiting a central mass; outputs their positions as a point set."),
    )?;
    r.describe(
        EQUATIONS,
        TypeInfo::new(
            "Equations typed as lines (ADR 0017): 'dx/dt = ...' for states (an ODE method), \
             'x[n+1] = ...' for a map (method iterate), only 'name = ...' for formulas (method \
             evaluate); 'input u, F' declares inputs, 'param k = 2' a parameter with its default, \
             'state x = 0' a state with its initial value (ADR 0049); every other name is a \
             parameter. Units in brackets where a name appears ('input F [N]', 'param m [kg] = \
             2', 'state x [m]') or at the end of a line for its left side ('E = m*v^2/2 [J]', \
             'dv/dt = F/m [m/s^2]') make every line checked (ADR 0045); names without one are \
             dimensionless. The method may be left out.",
        )
        .method("rk4", &["substeps"])
        .method("euler", &["substeps"])
        .method("dopri5", &["rtol", "atol"])
        .method(crate::symbolic::ITERATE, &[])
        .method(crate::symbolic::EVALUATE, &[])
        .definition(&["input u", "dx/dt = -k*x + u", "E = x^2/2"]),
    )?;
    r.describe(
        crate::convert::CONVERT,
        TypeInfo::new(
            "A signal from one unit to another (ADR 0045): input u in 'from', output y in 'to' \
             (settings, e.g. from 'km/h' to 'm/s'). Connections need equal units, so every \
             conversion is one of these.",
        )
        .method(crate::convert::EXACT, &[])
        .setting("from", &[])
        .setting("to", &[]),
    )?;
    r.describe(
        crate::input::VALUE,
        TypeInfo::new(
            "A value set from outside the running model (ADR 0058): output y, its parameter, \
             in the unit of the setting 'unit' (dimensionless if absent), suggested between \
             'min' and 'max' (-1 and 1 if absent). Driven by a slider, keys (controls.inputs), \
             a schedule (run.inputs) or a recorded run.",
        )
        .method(crate::input::HELD, &[])
        .setting("unit", &[])
        .setting("min", &[])
        .setting("max", &[]),
    )?;
    r.describe(
        crate::input::POINTER,
        TypeInfo::new(
            "A pointer in a view's coordinates (ADR 0058): outputs x and y in the setting \
             'unit' (m if absent), and down (1 while pressed). A view that sends input feeds \
             it (controls.inputs); the model turns it into what it does, e.g. a spring force \
             towards the pointer.",
        )
        .method(crate::input::HELD, &[])
        .setting("unit", &[])
        .setting("min", &[])
        .setting("max", &[]),
    )
}

/// Equation blocks have no agents, so no update order (ADR 0034).
fn no_update(c: &BlockConfig) -> Result<()> {
    if let Some(k) = c.settings.keys().next() {
        return Err(SimError::InvalidSetting {
            setting: format!("settings.{k}"),
            reason: "this type has no settings".into(),
        });
    }
    match c.update {
        None => Ok(()),
        Some(_) => Err(SimError::InvalidSetting {
            setting: "update".into(),
            reason: "only agent blocks have an update order".into(),
        }),
    }
}

/// Types with built-in equations take no definition lines.
fn fixed(type_name: &str, c: &BlockConfig) -> Result<()> {
    no_update(c)?;
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
            ..Default::default()
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
