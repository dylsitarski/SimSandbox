//! Counter-based random numbers (ADR 0034). Every draw is a pure function
//! of where it is made: a Block's stream (the model's seed and the Block's
//! name), the step, an id (e.g. an agent's) and what the draw is for. Runs
//! therefore reproduce bit for bit, rewind replays the same draws with
//! nothing saved, and an agent's draws do not depend on how agents are
//! stored or how many others exist.

/// SplitMix64's finaliser: a bijection of `u64` that spreads every input
/// bit over every output bit.
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A Block's stream: the model's seed combined with the Block's name, so
/// Blocks draw independently and renaming one changes only its own draws.
pub fn stream(seed: u64, name: &str) -> u64 {
    // FNV-1a over the name's bytes.
    let h = name.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3)
    });
    mix(seed ^ mix(h))
}

/// The draws at one place: a stream, a step and an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draws(u64);

impl Draws {
    pub fn new(stream: u64, step: u64, id: u64) -> Self {
        Self(mix(
            mix(stream ^ mix(step)) ^ mix(id ^ 0x5851_F42D_4C95_7F2D)
        ))
    }

    /// 64 random bits for draw number `purpose`.
    pub fn bits(&self, purpose: u64) -> u64 {
        mix(self.0 ^ mix(purpose.wrapping_add(0x2545_F491_4F6C_DD1D)))
    }

    /// Uniform in [0, 1).
    pub fn unit(&self, purpose: u64) -> f64 {
        (self.bits(purpose) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `0..n` (0 if `n` is 0).
    pub fn below(&self, purpose: u64, n: usize) -> usize {
        ((u128::from(self.bits(purpose)) * n as u128) >> 64) as usize
    }

    /// Standard normal, by Box–Muller from draws `2 × purpose` and
    /// `2 × purpose + 1`.
    pub fn normal(&self, purpose: u64) -> f64 {
        let u = 1.0 - self.unit(purpose.wrapping_mul(2));
        let v = self.unit(purpose.wrapping_mul(2).wrapping_add(1));
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_depend_only_on_where_they_are_made() {
        let s = stream(1, "sheep");
        let a = Draws::new(s, 7, 42);
        assert_eq!(a.bits(3), Draws::new(s, 7, 42).bits(3));
        // Every coordinate of the place changes the draw.
        let others = [
            Draws::new(stream(2, "sheep"), 7, 42).bits(3),
            Draws::new(stream(1, "wolves"), 7, 42).bits(3),
            Draws::new(s, 8, 42).bits(3),
            Draws::new(s, 7, 43).bits(3),
            a.bits(4),
        ];
        for o in others {
            assert_ne!(o, a.bits(3));
        }
    }

    #[test]
    fn uniform_and_normal_draws_have_the_right_moments() {
        let s = stream(1, "b");
        let n = 200_000;
        let (mut sum, mut sq, mut nsum, mut nsq) = (0.0, 0.0, 0.0, 0.0);
        let mut counts = [0usize; 10];
        for k in 0..n {
            let d = Draws::new(s, k, k * 31 + 5);
            let u = d.unit(0);
            assert!((0.0..1.0).contains(&u));
            sum += u;
            sq += u * u;
            counts[d.below(1, 10)] += 1;
            let z = d.normal(2);
            nsum += z;
            nsq += z * z;
        }
        let nf = n as f64;
        // Standard errors: 1/sqrt(12 n) for the mean, about 1/sqrt(n) for
        // the normal's; checked at five of them.
        assert!((sum / nf - 0.5).abs() < 5.0 / (12.0 * nf).sqrt());
        assert!((sq / nf - 1.0 / 3.0).abs() < 0.003);
        assert!((nsum / nf).abs() < 5.0 / nf.sqrt());
        assert!((nsq / nf - 1.0).abs() < 0.02);
        for c in counts {
            assert!((c as f64 / nf - 0.1).abs() < 0.004, "{counts:?}");
        }
        assert_eq!(Draws::new(s, 0, 0).below(0, 0), 0);
    }
}
