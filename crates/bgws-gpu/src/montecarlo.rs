//! Monte Carlo hit distributions (§11.1, §8.13): many independent shots at a
//! rectangular target with a bias (aim error) and a dispersion, giving hit
//! and kill probabilities and a radial miss-distance histogram. Used for
//! calibration and for fitting meta-models, not for live dice (rule 9).
//!
//! Each sample draws from a counter-based hash of (seed, sample, dimension)
//! rather than a sequential stream, so samples are independent of thread
//! order and a GPU port would draw the same numbers. Counts are integers and
//! the reduction is a sum, so the result is bit-for-bit deterministic.

use bgws_core::rng::mix64;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitParams {
    pub samples: u32,
    pub seed: u64,
    /// Mean point of impact relative to the target centre, metres.
    pub bias_x: f64,
    pub bias_y: f64,
    /// Round-to-round dispersion (1σ), metres.
    pub sigma_x: f64,
    pub sigma_y: f64,
    /// Target presented half-width and half-height, metres.
    pub half_w: f64,
    pub half_h: f64,
    /// Probability that a hit kills.
    pub p_kill_given_hit: f64,
    /// Histogram bin width for radial miss distance, metres.
    pub bin_m: f64,
    pub bins: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitDistribution {
    pub samples: u64,
    pub hits: u64,
    pub kills: u64,
    pub p_hit: f64,
    pub p_kill: f64,
    /// Counts of radial miss distance per bin; the last bin collects overflow.
    pub miss_histogram: Vec<u64>,
}

/// Uniform in [0, 1) from a counter-based hash.
#[inline]
pub fn hash_uniform(seed: u64, sample: u64, dim: u64) -> f64 {
    let h = mix64(seed ^ mix64(sample.wrapping_mul(8).wrapping_add(dim)));
    (h >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Standard normal pair by Box–Muller over two hashed uniforms (libm, for
/// cross-platform stability).
#[inline]
pub fn hash_normal_pair(seed: u64, sample: u64) -> (f64, f64) {
    let u1 = 1.0 - hash_uniform(seed, sample, 0); // (0, 1]
    let u2 = hash_uniform(seed, sample, 1);
    let r = libm::sqrt(-2.0 * libm::log(u1));
    let a = std::f64::consts::TAU * u2;
    (r * libm::cos(a), r * libm::sin(a))
}

/// The CPU twin.
pub fn hit_distribution_cpu(p: &HitParams) -> HitDistribution {
    let bins = p.bins.max(1);
    let chunk = 4096u64;
    let n = p.samples as u64;
    let chunks = n.div_ceil(chunk);
    let (hits, kills, hist) = (0..chunks)
        .into_par_iter()
        .map(|c| {
            let mut h = vec![0u64; bins];
            let (mut hits, mut kills) = (0u64, 0u64);
            for s in c * chunk..((c + 1) * chunk).min(n) {
                let (zx, zy) = hash_normal_pair(p.seed, s);
                let x = p.bias_x + p.sigma_x * zx;
                let y = p.bias_y + p.sigma_y * zy;
                if x.abs() <= p.half_w && y.abs() <= p.half_h {
                    hits += 1;
                    if hash_uniform(p.seed, s, 2) < p.p_kill_given_hit {
                        kills += 1;
                    }
                }
                let b = ((x.hypot(y) / p.bin_m.max(1e-9)) as usize).min(bins - 1);
                h[b] += 1;
            }
            (hits, kills, h)
        })
        .reduce(
            || (0, 0, vec![0u64; bins]),
            |a, b| (a.0 + b.0, a.1 + b.1, a.2.iter().zip(&b.2).map(|(x, y)| x + y).collect()),
        );
    let nn = n.max(1) as f64;
    HitDistribution { samples: n, hits, kills, p_hit: hits as f64 / nn, p_kill: kills as f64 / nn, miss_histogram: hist }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn erf(x: f64) -> f64 {
        libm::erf(x)
    }

    #[test]
    fn matches_analytic_rectangle_probability() {
        let p = HitParams {
            samples: 200_000,
            seed: 7,
            bias_x: 0.5,
            bias_y: 0.0,
            sigma_x: 1.0,
            sigma_y: 0.8,
            half_w: 1.5,
            half_h: 1.0,
            p_kill_given_hit: 0.4,
            bin_m: 0.5,
            bins: 20,
        };
        let r = hit_distribution_cpu(&p);
        let px = 0.5 * (erf((1.5 - 0.5) / (1.0 * 2f64.sqrt())) - erf((-1.5 - 0.5) / (1.0 * 2f64.sqrt())));
        let py = erf(1.0 / (0.8 * 2f64.sqrt()));
        assert!((r.p_hit - px * py).abs() < 0.005, "{} vs {}", r.p_hit, px * py);
        assert!((r.p_kill - 0.4 * px * py).abs() < 0.005);
        assert_eq!(r.miss_histogram.iter().sum::<u64>(), 200_000);
        assert_eq!(r, hit_distribution_cpu(&p), "deterministic");
    }
}
