//! Keyed random streams (§12): every draw comes from a stream keyed by
//! (game seed, purpose, entity, tick or event id, extra), so the order threads
//! run in never changes the outcome. ChaCha8 seeded from a stable hash of the
//! key. There is no global RNG.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Seed(pub u64);

/// What a stream is for. Two different purposes with the same entity and
/// tick never share draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamKey {
    pub purpose: &'static str,
    pub a: u64,
    pub b: u64,
    pub c: i64,
}

impl StreamKey {
    pub fn new(purpose: &'static str, a: u64, b: u64, c: i64) -> Self {
        Self { purpose, a, b, c }
    }
}

/// SplitMix64 finaliser: a stable, platform-independent mixer.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over bytes: stable across platforms and builds.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Seed {
    /// A ChaCha8 stream for this key.
    pub fn stream(self, key: StreamKey) -> ChaCha8Rng {
        let mut h = mix64(self.0 ^ fnv1a(key.purpose.as_bytes()));
        h = mix64(h ^ key.a);
        h = mix64(h ^ key.b.rotate_left(17));
        h = mix64(h ^ (key.c as u64).rotate_left(41));
        let mut seed = [0u8; 32];
        let mut s = h;
        for chunk in seed.chunks_mut(8) {
            s = mix64(s);
            chunk.copy_from_slice(&s.to_le_bytes());
        }
        ChaCha8Rng::from_seed(seed)
    }

    /// Shorthand for a single uniform draw in [0, 1).
    pub fn uniform(self, key: StreamKey) -> f64 {
        use rand::Rng;
        self.stream(key).gen::<f64>()
    }

    /// A derived seed for a sub-run (a batch replicate).
    pub fn derive(self, n: u64) -> Seed {
        Seed(mix64(self.0 ^ mix64(n)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn streams_are_keyed_and_reproducible() {
        let s = Seed(42);
        let k = StreamKey::new("detect", 1, 2, 3);
        let a: f64 = s.stream(k).gen();
        let b: f64 = s.stream(k).gen();
        assert_eq!(a, b);
        assert_ne!(a, s.uniform(StreamKey::new("detect", 1, 2, 4)));
        assert_ne!(a, s.uniform(StreamKey::new("fire", 1, 2, 3)));
        assert_ne!(a, Seed(43).uniform(k));
    }

    #[test]
    fn known_value_is_stable_across_builds() {
        // Locks the derivation: a change here breaks every saved replay.
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(mix64(0), 0xe220_a839_7b1d_cdaf);
    }
}
