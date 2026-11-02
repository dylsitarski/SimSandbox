//! Exact clocks for models stepping at different rates (ADR 0065): each
//! communication step `dt` is read as the decimal it is written as, and
//! every clock counts whole ticks of the largest time dividing all of
//! them, so points of different clocks compare exactly (3 × 0.1 and 0.3
//! are the same tick).

use crate::{Result, SimError};

/// A positive decimal as a fraction `num / den`, in lowest terms.
fn fraction(dt: f64) -> Result<(u128, u128)> {
    let bad = || SimError::InvalidSetting {
        setting: "run.dt".into(),
        reason: format!("{dt} cannot be a clock's step: a positive decimal is needed"),
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
    /// Each clock's step in ticks, in the order given.
    pub steps: Vec<u64>,
}

impl Ticks {
    /// The common tick of these steps and each step's length in it.
    pub fn new(dts: &[f64]) -> Result<Self> {
        let fractions = dts
            .iter()
            .map(|&d| fraction(d))
            .collect::<Result<Vec<_>>>()?;
        // gcd(a/b, c/d) = gcd(a·(L/b), c·(L/d)) / L with L = lcm(b, d).
        let lcm = fractions.iter().fold(1u128, |l, &(_, d)| l / gcd(l, d) * d);
        let scaled: Vec<u128> = fractions.iter().map(|&(n, d)| n * (lcm / d)).collect();
        let g = scaled.iter().fold(0u128, |g, &s| gcd(g, s)).max(1);
        let steps = scaled
            .iter()
            .map(|&s| u64::try_from(s / g))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| SimError::InvalidSetting {
                setting: "run.dt".into(),
                reason: "these clocks share too small a tick".into(),
            })?;
        let h = gcd(g, lcm);
        Ok(Self {
            tick: (g / h, lcm / h),
            steps,
        })
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
        assert!(Ticks::new(&[f64::NAN]).is_err());
    }
}
