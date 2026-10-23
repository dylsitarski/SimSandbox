//! # simsandbox-agents
//!
//! The agent domain (ADR 0034): populations of agents kept as kernel-owned
//! tables, stepped by rules with counter-based randomness, in an update
//! order chosen per block.
//!
//! - [`update`]: update orders and how agents share what they compete for.
//! - [`grazers`]: `agents.grazers`, animals that wander a field, eat its
//!   grass, breed and die (the grazing model, ADR 0033).

pub mod grazers;
pub mod update;

use simsandbox_core::block::{AccuracyClass, MethodLabels, MethodSpec, SpeedClass};
use simsandbox_core::registry::{BlockConfig, Registry, TypeInfo};
use simsandbox_core::{Result, SimError};

pub use grazers::Grazers;
pub use update::Update;

/// Animals grazing a field ([`Grazers`]).
pub const GRAZERS: &str = "agents.grazers";

/// The one method agent rules have: they are computed as stated
/// (ADR 0020: only approximations offer a choice).
pub const RULES: &str = "rules";

/// Register every agent block type.
pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(GRAZERS, |name, c| {
        rules_only(c)?;
        if !c.definition.is_empty() {
            return Err(SimError::InvalidDefinition(format!(
                "{GRAZERS} has built-in rules and takes no definition lines"
            )));
        }
        if let Some(k) = c.settings.keys().next() {
            return Err(SimError::InvalidSetting {
                setting: format!("settings.{k}"),
                reason: format!("{GRAZERS} has no settings besides 'update'"),
            });
        }
        let update = Update::from_setting(c.update.as_deref())?;
        Ok(Box::new(Grazers::new(name, update)))
    })?;
    registry.describe(
        GRAZERS,
        TypeInfo::new(
            "Animals on a square field that wander, eat the grass under them (input grass, a \
             40 × 40 grid), breed and die; output positions, energies, count and the grazing \
             rate for the grass (connect to eq.grass).",
        )
        .method(RULES, &[])
        .setting("update", &Update::ALL.map(Update::name)),
    )
}

fn rules_only(c: &BlockConfig) -> Result<()> {
    if c.method.name == RULES && c.method.options.is_empty() {
        Ok(())
    } else {
        Err(SimError::InvalidMethod(format!(
            "agent rules are computed as stated: the method is '{RULES}', with no options"
        )))
    }
}

/// Labels of agent rules: real-time; quantitative, since the rules are
/// computed exactly and the model is checked statistically (ADR 0034).
pub(crate) fn rules_method() -> MethodSpec {
    MethodSpec {
        name: RULES.into(),
        labels: MethodLabels {
            speed: SpeedClass::RealTime,
            accuracy: AccuracyClass::Quantitative,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_core::registry::MethodSettings;

    #[test]
    fn grazers_build_from_a_file_entry_and_check_their_settings() {
        let mut r = Registry::new();
        register(&mut r).unwrap();
        let config = |method: &str, update: Option<&str>| BlockConfig {
            method: MethodSettings::new(method),
            update: update.map(str::to_string),
            ..Default::default()
        };
        let b = r.build(GRAZERS, "sheep", &config(RULES, None)).unwrap();
        assert_eq!(b.spec().tables[0].columns, ["x", "y", "energy"]);
        for order in Update::ALL {
            r.build(GRAZERS, "sheep", &config(RULES, Some(order.name())))
                .unwrap();
        }
        assert!(matches!(
            r.build(GRAZERS, "s", &config(RULES, Some("alphabetical"))),
            Err(SimError::InvalidSetting { .. })
        ));
        assert!(matches!(
            r.build(GRAZERS, "s", &config("rk4", None)),
            Err(SimError::InvalidMethod(_))
        ));
    }
}
