//! Exact clocks for models stepping at different rates (ADR 0065): each
//! communication step `dt` is read as the decimal it is written as, and
//! every clock counts whole ticks of the largest time dividing all of
//! them, so points of different clocks compare exactly (3 × 0.1 and 0.3
//! are the same tick).

use crate::{Result, SimError};

/// A positive decimal as a fraction `num / den`, in lowest terms.
fn fraction(dt: f64) -> Result<(u128, u128)> {
    let bad = || {
        SimError::InvalidSetting {
        setting: "run.dt".into(),
        reason: format!(
            "{dt} cannot be a clock's step or time: a positive decimal of at most 18 digits after the point is needed"
        ),
    }
    };
    if !(dt.is_finite() && dt > 0.0) {
        return Err(bad());
    }
    // Rust writes the shortest decimal that reads back as `dt`.
    let text = format!("{dt}");
    let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
    if frac.len() > 18 || whole.len() > 18 {
        return Err(bad());
    }
    let den = 10u128.pow(frac.len() as u32);
    let num: u128 = format!("{whole}{frac}").parse().map_err(|_| bad())?;
    let g = gcd(num, den);
    Ok((num / g, den / g))
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Clocks counted in a common tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticks {
    /// The tick in seconds, as `num / den`.
    pub tick: (u128, u128),
    /// Each clock's step in ticks, in the order given. Counts are 128-bit:
    /// at the smallest tick (10⁻¹⁸ s) they last over 10¹² years.
    pub steps: Vec<u128>,
}

impl Ticks {
    /// The common tick of these steps and each step's length in it.
    pub fn new(dts: &[f64]) -> Result<Self> {
        Self::with_times(dts, &[])
    }

    /// The common tick of these steps and of listed instants (ADR 0067
    /// step 4: data's sample times, a use's `times`), so each instant is
    /// a whole number of ticks ([`Ticks::at`]).
    pub fn with_times(dts: &[f64], times: &[f64]) -> Result<Self> {
        let mut fractions = dts
            .iter()
            .map(|&d| fraction(d))
            .collect::<Result<Vec<_>>>()?;
        let steps_of = fractions.len();
        for &t in times {
            fractions.push(fraction(t)?);
        }
        // gcd(a/b, c/d) = gcd(a·(L/b), c·(L/d)) / L with L = lcm(b, d).
        let lcm = fractions.iter().fold(1u128, |l, &(_, d)| l / gcd(l, d) * d);
        let scaled: Vec<u128> = fractions.iter().map(|&(n, d)| n * (lcm / d)).collect();
        let g = scaled.iter().fold(0u128, |g, &s| gcd(g, s)).max(1);
        let steps = scaled[..steps_of].iter().map(|&s| s / g).collect();
        let h = gcd(g, lcm);
        Ok(Self {
            tick: (g / h, lcm / h),
            steps,
        })
    }

    /// How many whole steps of clock `k` fit in `t` seconds, `t` read as
    /// its decimal: the last step at or before `t`, never past it.
    pub fn steps_within(&self, k: usize, t: f64) -> Result<u128> {
        let (n, d) = fraction(t)?;
        let (a, b) = self.tick;
        // t / tick = (n / d) / (a / b) = n·b / (d·a), in whole ticks.
        let too_long = || SimError::InvalidSetting {
            setting: "run.t_end".into(),
            reason: format!("{t} s is too long for these clocks' tick"),
        };
        let ticks =
            n.checked_mul(b).ok_or_else(too_long)? / d.checked_mul(a).ok_or_else(too_long)?;
        Ok(ticks / self.steps[k])
    }

    /// The instant `t` seconds (read as its decimal) in ticks; an error if
    /// it is not a whole number of them.
    pub fn at(&self, t: f64) -> Result<u128> {
        let (n, d) = fraction(t)?;
        let (a, b) = self.tick;
        let off = || SimError::InvalidSetting {
            setting: "times".into(),
            reason: format!("{t} s is not a whole number of these clocks' ticks"),
        };
        let num = n.checked_mul(b).ok_or_else(off)?;
        let den = d.checked_mul(a).ok_or_else(off)?;
        if num % den != 0 {
            return Err(off());
        }
        Ok(num / den)
    }

    /// The whole ticks in `t` seconds, read as its decimal: the last tick
    /// at or before `t`.
    pub fn within(&self, t: f64) -> Result<u128> {
        let (n, d) = fraction(t)?;
        let (a, b) = self.tick;
        let too_long = || SimError::InvalidSetting {
            setting: "run.t_end".into(),
            reason: format!("{t} s is too long for these clocks' tick"),
        };
        Ok(n.checked_mul(b).ok_or_else(too_long)? / d.checked_mul(a).ok_or_else(too_long)?)
    }

    /// The tick in seconds, rounded to the nearest `f64`.
    pub fn seconds(&self) -> f64 {
        self.tick.0 as f64 / self.tick.1 as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_share_an_exact_tick() {
        let t = Ticks::new(&[0.1, 0.3]).unwrap();
        assert_eq!(t.tick, (1, 10));
        assert_eq!(t.steps, [1, 3]);
        let t = Ticks::new(&[0.25, 0.1, 2.0]).unwrap();
        assert_eq!(t.tick, (1, 20));
        assert_eq!(t.steps, [5, 2, 40]);
        let t = Ticks::new(&[0.07, 0.1]).unwrap();
        assert_eq!(t.tick, (1, 100));
        assert_eq!(t.steps, [7, 10]);
        assert_eq!(Ticks::new(&[0.01]).unwrap().steps, [1]);
        assert!(Ticks::new(&[0.0]).is_err());
        // Clocks that rarely meet: a tick of 10⁻¹⁶ s.
        let t = Ticks::new(&[0.1, 0.3333333333333333]).unwrap();
        assert_eq!(t.tick, (1, 10u128.pow(16)));
        assert_eq!(t.steps_within(1, 1.0).unwrap(), 3);
        // The last step at or before the end, never past it.
        let t = Ticks::new(&[0.4, 0.2]).unwrap();
        assert_eq!(t.steps_within(0, 1.0).unwrap(), 2);
        assert_eq!(t.steps_within(1, 0.6).unwrap(), 3);
        assert!(Ticks::new(&[f64::NAN]).is_err());
    }

    #[test]
    fn listed_times_join_the_tick() {
        // Data at 0.25 and 0.7 s beside a clock of 0.1 s: a tick of 0.05.
        let t = Ticks::with_times(&[0.1], &[0.25, 0.7]).unwrap();
        assert_eq!(t.tick, (1, 20));
        assert_eq!(t.steps, [2]);
        assert_eq!((t.at(0.25).unwrap(), t.at(0.7).unwrap()), (5, 14));
        // Only times: their own tick.
        let t = Ticks::with_times(&[], &[0.3, 1.2]).unwrap();
        assert_eq!(t.tick, (3, 10));
        assert!(t.at(0.5).is_err());
    }
}
