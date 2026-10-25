//! Summary statistics for batch outcomes (RUST_ENGINE.md §15.3, §16): mean,
//! standard deviation, quantiles, the t confidence interval of a mean, the
//! Wilson interval of a proportion, a distribution-free interval for a
//! quantile, histograms and empirical CDFs.
//!
//! Results are ranges, never point predictions (the TypeScript `batch.ts`
//! rule carried over): every estimate here comes with its interval.
//!
//! Quantiles use the linear-interpolation definition (Hyndman–Fan type 7,
//! the default of R and NumPy). The TypeScript build used the nearest rank;
//! the two differ by at most one order statistic.

use serde::{Deserialize, Serialize};
use statrs::distribution::{Binomial, ContinuousCDF, DiscreteCDF, Normal, StudentsT};

/// Keep the finite values of a sample, sorted ascending. NaN and ±∞ are
/// dropped (a replicate that produced nothing to measure).
pub fn sorted_finite(values: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    v
}

/// Arithmetic mean (NaN for an empty sample).
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    // Fixed left-to-right order: a deterministic reduction (§12).
    values.iter().sum::<f64>() / values.len() as f64
}

/// Sample standard deviation with Bessel's correction (NaN below two values).
pub fn std_dev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return f64::NAN;
    }
    let m = mean(values);
    let ss: f64 = values.iter().map(|x| (x - m) * (x - m)).sum();
    (ss / (values.len() - 1) as f64).sqrt()
}

/// The `p` quantile of an ascending-sorted sample, linear interpolation
/// between order statistics (type 7). NaN for an empty sample.
pub fn quantile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let p = p.clamp(0.0, 1.0);
    let h = p * (sorted.len() - 1) as f64;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

/// The `p` quantile of an unsorted sample (non-finite values dropped).
pub fn quantile(values: &[f64], p: f64) -> f64 {
    quantile_sorted(&sorted_finite(values), p)
}

/// A two-sided interval with its confidence level.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
    /// Confidence level, e.g. 0.95.
    pub level: f64,
}

impl Interval {
    pub fn contains(&self, x: f64) -> bool {
        x >= self.lo && x <= self.hi
    }
    pub fn width(&self) -> f64 {
        self.hi - self.lo
    }
}

/// Standard normal quantile.
pub fn z_quantile(p: f64) -> f64 {
    Normal::new(0.0, 1.0)
        .map(|n| n.inverse_cdf(p))
        .unwrap_or(f64::NAN)
}

/// The t confidence interval of the mean: x̄ ± t(1−α/2, n−1)·s/√n.
/// `None` with fewer than two values.
pub fn mean_ci_t(values: &[f64], level: f64) -> Option<Interval> {
    let n = values.len();
    if n < 2 {
        return None;
    }
    let m = mean(values);
    let s = std_dev(values);
    let t = StudentsT::new(0.0, 1.0, (n - 1) as f64)
        .ok()?
        .inverse_cdf(0.5 + level / 2.0);
    let half = t * s / (n as f64).sqrt();
    Some(Interval {
        lo: m - half,
        hi: m + half,
        level,
    })
}

/// The Wilson score interval for `successes` out of `n` trials. Well behaved
/// near 0 and 1, unlike the Wald interval. `None` when `n == 0`.
pub fn wilson(successes: u64, n: u64, level: f64) -> Option<Interval> {
    if n == 0 {
        return None;
    }
    let z = z_quantile(0.5 + level / 2.0);
    let nf = n as f64;
    let p = successes as f64 / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let centre = (p + z2 / (2.0 * nf)) / denom;
    let half = z / denom * (p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)).sqrt();
    // At the edges the interval ends exactly at 0 or 1.
    let lo = if successes == 0 {
        0.0
    } else {
        (centre - half).max(0.0)
    };
    let hi = if successes >= n {
        1.0
    } else {
        (centre + half).min(1.0)
    };
    Some(Interval { lo, hi, level })
}

/// A distribution-free confidence interval for the `p` quantile of a sorted
/// sample: the order statistics x(l), x(u) with P(l ≤ B < u) ≥ level for
/// B ~ Binomial(n, p). Searches outward from the median rank, so it is exact,
/// not a normal approximation. `None` when n is too small to reach `level`
/// (then the full sample range is the best available, reported by callers).
pub fn quantile_ci_sorted(sorted: &[f64], p: f64, level: f64) -> Option<Interval> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let b = Binomial::new(p.clamp(0.0, 1.0), n as u64).ok()?;
    // Coverage of [x(l), x(u)] (1-based ranks) = F(u−1) − F(l−1).
    let cover = |l: usize, u: usize| -> f64 {
        let fu = b.cdf((u - 1) as u64);
        let fl = if l >= 2 { b.cdf((l - 2) as u64) } else { 0.0 };
        fu - fl
    };
    let centre = ((p * n as f64).round() as usize).clamp(1, n);
    let (mut l, mut u) = (centre, centre);
    while cover(l, u) < level {
        // Widen on the side whose step gains more coverage; stop at the ends.
        let can_l = l > 1;
        let can_u = u < n;
        if !can_l && !can_u {
            return None;
        }
        let gain_l = if can_l { cover(l - 1, u) } else { -1.0 };
        let gain_u = if can_u { cover(l, u + 1) } else { -1.0 };
        if gain_l >= gain_u {
            l -= 1;
        } else {
            u += 1;
        }
    }
    Some(Interval {
        lo: sorted[l - 1],
        hi: sorted[u - 1],
        level,
    })
}

/// Serde helper: read `null` (how JSON writes NaN) back as NaN.
pub(crate) fn nan_from_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
}

/// The descriptive summary of one metric over a batch: the TypeScript
/// `Distribution` (n, mean, p10, p50, p90, min, max) plus spread and the
/// confidence interval of the mean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Values that were measured (finite).
    pub n: usize,
    /// Replicates that produced nothing to measure.
    pub missing: usize,
    #[serde(deserialize_with = "nan_from_null")]
    pub mean: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub sd: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub min: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p05: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p10: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p25: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p50: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p75: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p90: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub p95: f64,
    #[serde(deserialize_with = "nan_from_null")]
    pub max: f64,
    /// t interval of the mean, when n ≥ 2.
    pub mean_ci: Option<Interval>,
}

impl Summary {
    /// Summarise a sample; non-finite values count as missing.
    pub fn of(values: &[f64], level: f64) -> Summary {
        let v = sorted_finite(values);
        let q = |p| quantile_sorted(&v, p);
        Summary {
            n: v.len(),
            missing: values.len() - v.len(),
            mean: mean(&v),
            sd: std_dev(&v),
            min: v.first().copied().unwrap_or(f64::NAN),
            p05: q(0.05),
            p10: q(0.10),
            p25: q(0.25),
            p50: q(0.50),
            p75: q(0.75),
            p90: q(0.90),
            p95: q(0.95),
            max: v.last().copied().unwrap_or(f64::NAN),
            mean_ci: mean_ci_t(&v, level),
        }
    }
}

/// A fixed-width histogram: `counts[i]` holds values in
/// `[edges[i], edges[i+1])`, the last bin closed on the right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Histogram {
    pub edges: Vec<f64>,
    pub counts: Vec<u64>,
    /// Values outside the edges (only with an explicit range).
    pub outside: u64,
}

impl Histogram {
    /// `bins` equal bins spanning the sample's range.
    pub fn of(values: &[f64], bins: usize) -> Histogram {
        let v = sorted_finite(values);
        let (lo, hi) = match (v.first(), v.last()) {
            (Some(&a), Some(&b)) if b > a => (a, b),
            (Some(&a), Some(_)) => (a - 0.5, a + 0.5),
            _ => (0.0, 1.0),
        };
        Self::with_range(&v, lo, hi, bins)
    }

    /// `bins` equal bins over `[lo, hi]`; shared edges let two batches be
    /// drawn on one chart (§16).
    pub fn with_range(values: &[f64], lo: f64, hi: f64, bins: usize) -> Histogram {
        let bins = bins.max(1);
        let width = (hi - lo) / bins as f64;
        let edges: Vec<f64> = (0..=bins).map(|i| lo + width * i as f64).collect();
        let mut counts = vec![0u64; bins];
        let mut outside = 0;
        for &x in values.iter().filter(|x| x.is_finite()) {
            if x < lo || x > hi || width <= 0.0 {
                outside += 1;
                continue;
            }
            let i = (((x - lo) / width).floor() as usize).min(bins - 1);
            counts[i] += 1;
        }
        Histogram {
            edges,
            counts,
            outside,
        }
    }

    pub fn total(&self) -> u64 {
        self.counts.iter().sum()
    }
}

/// The empirical CDF as step points (x, F(x)) for charts, one per distinct value.
pub fn ecdf(values: &[f64]) -> Vec<(f64, f64)> {
    let v = sorted_finite(values);
    let n = v.len() as f64;
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (i, &x) in v.iter().enumerate() {
        let f = (i + 1) as f64 / n;
        match out.last_mut() {
            Some(last) if last.0 == x => last.1 = f,
            _ => out.push((x, f)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn mean_sd_quantiles() {
        let v = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_relative_eq!(mean(&v), 5.0);
        assert_relative_eq!(std_dev(&v), (32.0f64 / 7.0).sqrt(), epsilon = 1e-12);
        let s = sorted_finite(&v);
        assert_relative_eq!(quantile_sorted(&s, 0.5), 4.5);
        assert_relative_eq!(quantile_sorted(&s, 0.0), 2.0);
        assert_relative_eq!(quantile_sorted(&s, 1.0), 9.0);
        // numpy.quantile([2,4,4,4,5,5,7,9], 0.9) = 7.6
        assert_relative_eq!(quantile_sorted(&s, 0.9), 7.6, epsilon = 1e-12);
    }

    #[test]
    fn t_interval_matches_tables() {
        // n = 10, mean 0, s = 1: t(0.975, 9) = 2.262157.
        let v: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let ci = mean_ci_t(&v, 0.95).unwrap();
        let half = 2.262157 * std_dev(&v) / 10f64.sqrt();
        assert_relative_eq!(ci.hi - 4.5, half, epsilon = 1e-5);
        assert!(mean_ci_t(&[1.0], 0.95).is_none());
    }

    #[test]
    fn wilson_known_values() {
        // 273/322 (P20): Wilson 95% = [0.80451, 0.88294].
        let w = wilson(273, 322, 0.95).unwrap();
        assert_relative_eq!(w.lo, 0.80451, epsilon = 1e-5);
        assert_relative_eq!(w.hi, 0.88294, epsilon = 1e-5);
        let z = wilson(0, 20, 0.95).unwrap();
        assert_eq!(z.lo, 0.0);
        assert!(z.hi > 0.1 && z.hi < 0.2);
    }

    #[test]
    fn quantile_ci_covers_median() {
        let v: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let ci = quantile_ci_sorted(&v, 0.5, 0.95).unwrap();
        // Classic result for n = 100: ranks 40 and 61.
        assert!(ci.lo >= 39.0 && ci.lo <= 41.0, "{ci:?}");
        assert!(ci.hi >= 60.0 && ci.hi <= 62.0, "{ci:?}");
        assert!(quantile_ci_sorted(&[1.0, 2.0], 0.5, 0.95).is_none());
    }

    #[test]
    fn summary_and_histogram() {
        let v = [1.0, 2.0, f64::NAN, 3.0, 4.0];
        let s = Summary::of(&v, 0.95);
        assert_eq!(s.n, 4);
        assert_eq!(s.missing, 1);
        assert_relative_eq!(s.p50, 2.5);
        let h = Histogram::of(&v, 3);
        assert_eq!(h.total(), 4);
        assert_eq!(h.counts, vec![1, 1, 2]);
        let h2 = Histogram::with_range(&v, 0.0, 2.0, 2);
        assert_eq!(h2.outside, 2);
        let e = ecdf(&[1.0, 1.0, 2.0]);
        assert_eq!(e, vec![(1.0, 2.0 / 3.0), (2.0, 1.0)]);
    }

    proptest::proptest! {
        #[test]
        fn quantiles_are_monotone_and_bounded(v in proptest::collection::vec(-1e6f64..1e6, 1..60), p in 0.0f64..1.0, q in 0.0f64..1.0) {
            let s = sorted_finite(&v);
            let (lo, hi) = if p <= q { (p, q) } else { (q, p) };
            proptest::prop_assert!(quantile_sorted(&s, lo) <= quantile_sorted(&s, hi));
            proptest::prop_assert!(quantile_sorted(&s, p) >= s[0] && quantile_sorted(&s, p) <= s[s.len() - 1]);
        }

        #[test]
        fn wilson_contains_the_estimate(n in 1u64..500, k in 0u64..500) {
            let k = k.min(n);
            let w = wilson(k, n, 0.95).unwrap();
            let p = k as f64 / n as f64;
            proptest::prop_assert!(w.lo <= p + 1e-12 && p <= w.hi + 1e-12 && w.lo >= 0.0 && w.hi <= 1.0);
        }
    }
}
