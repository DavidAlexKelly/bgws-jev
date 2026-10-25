//! Keyed dice (§12). Every random draw comes from a stream keyed by the
//! game's seed, what the draw is for, and who and when it concerns. The
//! order threads run in can't change any result, and adding a new kind of
//! draw never shifts the others.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

/// A purpose's key, from its name, at compile time: `purpose("detect")`.
pub const fn purpose(name: &str) -> u64 {
    // FNV-1a, 64-bit.
    let bytes = name.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

#[inline]
fn mix(mut x: u64) -> u64 {
    // splitmix64 finaliser.
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dice {
    pub seed: u64,
}

impl Dice {
    pub fn new(seed: u64) -> Dice {
        Dice { seed }
    }
    /// A seed from text: scenario seeds are strings in the files.
    pub fn from_text(s: &str) -> Dice {
        Dice { seed: mix(purpose(s)) }
    }
    fn key(&self, purpose: u64, a: u64, b: u64) -> [u8; 32] {
        let k0 = mix(self.seed ^ mix(purpose));
        let k1 = mix(k0 ^ mix(a.wrapping_add(0x51)));
        let k2 = mix(k1 ^ mix(b.wrapping_add(0xa3)));
        let k3 = mix(k2 ^ k0.rotate_left(17));
        let mut out = [0u8; 32];
        out[0..8].copy_from_slice(&k0.to_le_bytes());
        out[8..16].copy_from_slice(&k1.to_le_bytes());
        out[16..24].copy_from_slice(&k2.to_le_bytes());
        out[24..32].copy_from_slice(&k3.to_le_bytes());
        out
    }
    /// A stream for several draws about one thing.
    pub fn stream(&self, purpose: u64, a: u64, b: u64) -> ChaCha8Rng {
        ChaCha8Rng::from_seed(self.key(purpose, a, b))
    }
    /// One uniform draw in [0, 1). Cheap: no stream is built.
    #[inline]
    pub fn u(&self, purpose: u64, a: u64, b: u64) -> f64 {
        let x = mix(mix(self.seed ^ mix(purpose)) ^ mix(a.wrapping_mul(0x9e37_79b9).wrapping_add(b.rotate_left(29)) ^ b));
        (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    /// Does an event of probability `p` happen?
    #[inline]
    pub fn chance(&self, p: f64, purpose: u64, a: u64, b: u64) -> bool {
        p > 0.0 && (p >= 1.0 || self.u(purpose, a, b) < p)
    }
    /// A standard normal draw (Box–Muller on two keyed uniforms).
    pub fn normal(&self, purpose: u64, a: u64, b: u64) -> f64 {
        let u1 = self.u(purpose, a, b).max(1e-300);
        let u2 = self.u(purpose ^ 0x5555, a, b);
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
    /// A child dice for a sub-game (a batch run, a disaggregated formation).
    pub fn child(&self, k: u64) -> Dice {
        Dice { seed: mix(self.seed ^ mix(k ^ 0x00c0_ffee)) }
    }
}

/// Draw from a stream: a convenience for `rand` users.
pub fn draw<R: Rng>(r: &mut R) -> f64 {
    r.gen::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyed_draws_are_stable_and_independent() {
        let d = Dice::new(42);
        let a = d.u(purpose("detect"), 7, 100);
        assert_eq!(a, d.u(purpose("detect"), 7, 100));
        assert_ne!(a, d.u(purpose("detect"), 8, 100));
        assert_ne!(a, d.u(purpose("hit"), 7, 100));
        // Roughly uniform.
        let n = 100_000;
        let mean: f64 = (0..n).map(|i| d.u(purpose("x"), i, 0)).sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.005, "{mean}");
        let var: f64 = (0..n).map(|i| d.normal(purpose("n"), i, 3).powi(2)).sum::<f64>() / n as f64;
        assert!((var - 1.0).abs() < 0.03, "{var}");
    }
}
