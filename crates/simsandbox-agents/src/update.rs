//! Update orders (ADR 0034): when agents compete within a step, who acts
//! first changes results, so the order is part of the model, chosen per
//! block and saved in its file entry as `update`.

use simsandbox_core::random::Draws;
use simsandbox_core::{Result, SimError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Update {
    /// One after another in a random order drawn anew each step.
    #[default]
    RandomOrder,
    /// All decide from the same start-of-step state; what they compete for
    /// is shared in proportion to what each wants.
    Synchronous,
    /// One after another, oldest first: deterministic, and biased towards
    /// the old.
    IdOrder,
}

impl Update {
    pub const ALL: [Update; 3] = [Update::RandomOrder, Update::Synchronous, Update::IdOrder];

    /// The name used in model files.
    pub fn name(self) -> &'static str {
        match self {
            Update::RandomOrder => "random_order",
            Update::Synchronous => "synchronous",
            Update::IdOrder => "id_order",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Update::RandomOrder => "Agents act one after another in a new random order each step",
            Update::Synchronous => {
                "Agents decide together from the start of the step and share what they compete for"
            }
            Update::IdOrder => "Agents act one after another, oldest first (favours the old)",
        }
    }

    /// From a block entry's `update`; the default if absent.
    pub fn from_setting(setting: Option<&str>) -> Result<Self> {
        let Some(name) = setting else {
            return Ok(Self::default());
        };
        Self::ALL
            .into_iter()
            .find(|u| u.name() == name)
            .ok_or_else(|| SimError::InvalidSetting {
                setting: "update".into(),
                reason: format!(
                    "unknown update order '{name}' (expected {})",
                    Self::ALL.map(Update::name).join(", ")
                ),
            })
    }

    /// The order `n` agents act in this step. `draws` gives the random
    /// order (a seeded shuffle); the others are by row, oldest first.
    pub fn order(self, n: usize, draws: Draws) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        if self == Update::RandomOrder {
            // Fisher–Yates.
            for i in (1..n).rev() {
                order.swap(i, draws.below(i as u64, i + 1));
            }
        }
        order
    }

    /// How much each agent gets: agent `a` wants `want[a]` from pool
    /// `pool[a]`, which holds `available[pool]`. In turn (in `order`), each
    /// takes what it wants while there is enough; synchronously, a pool
    /// asked for more than it holds is shared in proportion to what each
    /// wants. `available` is left with what remains.
    pub fn share(
        self,
        order: &[usize],
        want: &[f64],
        pool: &[usize],
        available: &mut [f64],
    ) -> Vec<f64> {
        let mut got = vec![0.0; want.len()];
        if self == Update::Synchronous {
            let mut asked = vec![0.0; available.len()];
            for (a, &w) in want.iter().enumerate() {
                asked[pool[a]] += w;
            }
            let scale: Vec<f64> = asked
                .iter()
                .zip(available.iter())
                .map(|(&ask, &have)| if ask > have { have / ask } else { 1.0 })
                .collect();
            for (a, &w) in want.iter().enumerate() {
                got[a] = w * scale[pool[a]];
                available[pool[a]] -= got[a];
            }
            for v in available.iter_mut() {
                // Rounding must not leave a pool below zero.
                *v = v.max(0.0);
            }
            return got;
        }
        for &a in order {
            let have = &mut available[pool[a]];
            got[a] = want[a].min(*have);
            *have -= got[a];
        }
        got
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_core::random::stream;

    #[test]
    fn competing_agents_share_by_the_update_order() {
        // Four agents want 1 each from a pool of 2; a fifth has its own.
        let want = [1.0, 1.0, 1.0, 1.0, 0.5];
        let pool = [0, 0, 0, 0, 1];
        let draws = Draws::new(stream(1, "test"), 0, 0);

        let mut available = [2.0, 10.0];
        let got = Update::Synchronous.share(&[], &want, &pool, &mut available);
        assert_eq!(got, [0.5, 0.5, 0.5, 0.5, 0.5]);
        assert_eq!(available, [0.0, 9.5]);

        let mut available = [2.0, 10.0];
        let order = Update::IdOrder.order(5, draws);
        let got = Update::IdOrder.share(&order, &want, &pool, &mut available);
        assert_eq!(got, [1.0, 1.0, 0.0, 0.0, 0.5]);

        // A random order: two of the four eat fully, and which two depends
        // on the draws; the total is the pool either way.
        let mut available = [2.0, 10.0];
        let order = Update::RandomOrder.order(5, draws);
        let got = Update::RandomOrder.share(&order, &want, &pool, &mut available);
        assert_eq!(got[..4].iter().filter(|&&g| g == 1.0).count(), 2);
        assert_eq!(got[..4].iter().sum::<f64>(), 2.0);
        assert_eq!(available[0], 0.0);
    }

    #[test]
    fn random_orders_are_permutations_that_change_with_the_draws() {
        let s = stream(1, "test");
        let mut seen_first = std::collections::BTreeSet::new();
        for step in 0..50 {
            let mut o = Update::RandomOrder.order(10, Draws::new(s, step, 0));
            seen_first.insert(o[0]);
            o.sort();
            assert_eq!(o, (0..10).collect::<Vec<_>>());
        }
        assert!(seen_first.len() > 5, "{seen_first:?}");
        assert_eq!(
            Update::RandomOrder.order(10, Draws::new(s, 3, 0)),
            Update::RandomOrder.order(10, Draws::new(s, 3, 0))
        );
        assert_eq!(Update::from_setting(None).unwrap(), Update::RandomOrder);
        assert_eq!(
            Update::from_setting(Some("synchronous")).unwrap(),
            Update::Synchronous
        );
    }
}
