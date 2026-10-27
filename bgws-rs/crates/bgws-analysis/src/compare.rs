//! Distribution comparison (RUST_ENGINE.md §15.8, §17 "statistical", §16
//! "comparing two templates or two catalogues"): the two-sample
//! Kolmogorov–Smirnov test (does a new run's distribution differ from the
//! stored one?), the one-sample KS test against a fitted CDF (goodness of fit
//! for the meta-models, §8.13), Pearson's chi-squared test for counts, and the
//! Mann–Whitney U test for a shift in location.
//!
//! p-values use the asymptotic distributions (Kolmogorov with Stephens'
//! small-sample correction; normal for U with tie correction). With parameters
//! estimated from the same data the one-sample KS p-value is conservative
//! (Lilliefors); callers that need exact levels should fit on one half and
//! test on the other.

use crate::stats::{mean, sorted_finite, std_dev, Histogram, Interval, Summary};
use serde::{Deserialize, Serialize};
use statrs::distribution::{ChiSquared, ContinuousCDF, Normal, StudentsT};

/// The Kolmogorov distribution's survival function Q(λ) = P(K > λ)
/// = 2 Σ (−1)^(j−1) e^(−2j²λ²), with the theta-function form for small λ.
pub fn kolmogorov_q(lambda: f64) -> f64 {
    if !lambda.is_finite() || lambda <= 0.0 {
        return 1.0;
    }
    if lambda < 1.18 {
        // P(K ≤ λ) = √(2π)/λ Σ exp(−(2j−1)²π²/(8λ²)).
        let y = (-std::f64::consts::PI.powi(2) / (8.0 * lambda * lambda)).exp();
        let mut s = 0.0;
        let mut j = 1i32;
        loop {
            let term = y.powi((2 * j - 1) * (2 * j - 1));
            s += term;
            if term < 1e-16 * s || j > 100 {
                break;
            }
            j += 1;
        }
        (1.0 - (2.0 * std::f64::consts::PI).sqrt() / lambda * s).clamp(0.0, 1.0)
    } else {
        let mut s = 0.0;
        let mut sign = 1.0;
        for j in 1..=100 {
            let term = (-2.0 * (j * j) as f64 * lambda * lambda).exp();
            s += sign * term;
            if term < 1e-16 {
                break;
            }
            sign = -sign;
        }
        (2.0 * s).clamp(0.0, 1.0)
    }
}

/// A Kolmogorov–Smirnov result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KsResult {
    /// The largest gap between the two (empirical) CDFs.
    pub d: f64,
    /// Sizes of the samples (m = 0 for the one-sample test).
    pub n: usize,
    pub m: usize,
    /// Asymptotic p-value with Stephens' correction.
    pub p_value: f64,
}

impl KsResult {
    /// The distributions differ at significance `alpha`.
    pub fn rejects(&self, alpha: f64) -> bool {
        self.p_value < alpha
    }
}

/// Two-sample KS test. Ties are handled by stepping through each distinct
/// value once. Non-finite values are ignored. `None` if either sample is empty.
pub fn ks_two_sample(a: &[f64], b: &[f64]) -> Option<KsResult> {
    let a = sorted_finite(a);
    let b = sorted_finite(b);
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return None;
    }
    let (mut i, mut j, mut d) = (0usize, 0usize, 0.0f64);
    while i < n && j < m {
        let x = a[i].min(b[j]);
        while i < n && a[i] <= x {
            i += 1;
        }
        while j < m && b[j] <= x {
            j += 1;
        }
        d = d.max((i as f64 / n as f64 - j as f64 / m as f64).abs());
    }
    let en = ((n * m) as f64 / (n + m) as f64).sqrt();
    let p_value = kolmogorov_q((en + 0.12 + 0.11 / en) * d);
    Some(KsResult { d, n, m, p_value })
}

/// One-sample KS test of `samples` against a continuous `cdf`.
pub fn ks_one_sample(samples: &[f64], cdf: impl Fn(f64) -> f64) -> Option<KsResult> {
    let v = sorted_finite(samples);
    let n = v.len();
    if n == 0 {
        return None;
    }
    let mut d = 0.0f64;
    for (i, &x) in v.iter().enumerate() {
        let f = cdf(x);
        d = d
            .max(f - i as f64 / n as f64)
            .max((i + 1) as f64 / n as f64 - f);
    }
    let en = (n as f64).sqrt();
    Some(KsResult {
        d,
        n,
        m: 0,
        p_value: kolmogorov_q((en + 0.12 + 0.11 / en) * d),
    })
}

/// A chi-squared test result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChiSquaredResult {
    pub statistic: f64,
    pub dof: usize,
    pub p_value: f64,
    /// Cells actually used (after merging or dropping empty ones).
    pub cells: usize,
}

impl ChiSquaredResult {
    pub fn rejects(&self, alpha: f64) -> bool {
        self.p_value < alpha
    }
}

fn chi2_p(stat: f64, dof: usize) -> f64 {
    if dof == 0 {
        return 1.0;
    }
    ChiSquared::new(dof as f64)
        .map(|c| c.sf(stat))
        .unwrap_or(f64::NAN)
}

/// Pearson goodness-of-fit: observed counts against expected counts, with
/// `estimated` parameters fitted from the data (dof = cells − 1 − estimated).
/// Adjacent cells are merged left to right until each expected count is at
/// least `min_expected` (Cochran's rule). `None` if fewer than two cells remain
/// or the degrees of freedom vanish.
pub fn chi_squared_gof(
    observed: &[f64],
    expected: &[f64],
    estimated: usize,
    min_expected: f64,
) -> Option<ChiSquaredResult> {
    if observed.len() != expected.len() {
        return None;
    }
    let mut cells: Vec<(f64, f64)> = Vec::new();
    let (mut o, mut e) = (0.0, 0.0);
    for (&oi, &ei) in observed.iter().zip(expected) {
        o += oi;
        e += ei;
        if e >= min_expected {
            cells.push((o, e));
            o = 0.0;
            e = 0.0;
        }
    }
    if e > 0.0 || o > 0.0 {
        match cells.last_mut() {
            Some(last) => {
                last.0 += o;
                last.1 += e;
            }
            None => cells.push((o, e)),
        }
    }
    if cells.len() < 2 || cells.len() <= 1 + estimated {
        return None;
    }
    let statistic: f64 = cells.iter().map(|(o, e)| (o - e) * (o - e) / e).sum();
    let dof = cells.len() - 1 - estimated;
    Some(ChiSquaredResult {
        statistic,
        dof,
        p_value: chi2_p(statistic, dof),
        cells: cells.len(),
    })
}

/// Chi-squared test of homogeneity for two vectors of counts over the same
/// categories (a 2 × k contingency table). Categories empty in both are dropped.
pub fn chi_squared_homogeneity(a: &[u64], b: &[u64]) -> Option<ChiSquaredResult> {
    if a.len() != b.len() {
        return None;
    }
    let cols: Vec<(f64, f64)> = a
        .iter()
        .zip(b)
        .filter(|(x, y)| **x + **y > 0)
        .map(|(&x, &y)| (x as f64, y as f64))
        .collect();
    let na: f64 = cols.iter().map(|c| c.0).sum();
    let nb: f64 = cols.iter().map(|c| c.1).sum();
    let total = na + nb;
    if cols.len() < 2 || na == 0.0 || nb == 0.0 {
        return None;
    }
    let mut statistic = 0.0;
    for &(x, y) in &cols {
        let col = x + y;
        let ea = na * col / total;
        let eb = nb * col / total;
        statistic += (x - ea).powi(2) / ea + (y - eb).powi(2) / eb;
    }
    let dof = cols.len() - 1;
    Some(ChiSquaredResult {
        statistic,
        dof,
        p_value: chi2_p(statistic, dof),
        cells: cols.len(),
    })
}

/// A Mann–Whitney U test result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MannWhitneyResult {
    /// U for the first sample.
    pub u: f64,
    /// Normal approximation with tie and continuity corrections.
    pub z: f64,
    /// Two-sided p-value.
    pub p_value: f64,
    /// P(A > B) + ½P(A = B): the common-language effect size, U / (n·m).
    pub effect: f64,
}

/// Mann–Whitney U (Wilcoxon rank-sum) test, two-sided. `None` if a sample is empty.
pub fn mann_whitney(a: &[f64], b: &[f64]) -> Option<MannWhitneyResult> {
    let a = sorted_finite(a);
    let b = sorted_finite(b);
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return None;
    }
    // Pool, sort with a stable tie-break on group, assign average ranks.
    let mut pooled: Vec<(f64, bool)> = a
        .iter()
        .map(|&x| (x, true))
        .chain(b.iter().map(|&x| (x, false)))
        .collect();
    pooled.sort_by(|x, y| x.0.total_cmp(&y.0));
    let total = pooled.len();
    let mut rank_sum_a = 0.0;
    let mut tie_term = 0.0;
    let mut i = 0;
    while i < total {
        let mut j = i;
        while j + 1 < total && pooled[j + 1].0 == pooled[i].0 {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        let t = (j - i + 1) as f64;
        tie_term += t * t * t - t;
        for p in &pooled[i..=j] {
            if p.1 {
                rank_sum_a += avg;
            }
        }
        i = j + 1;
    }
    let (nf, mf) = (n as f64, m as f64);
    let u = rank_sum_a - nf * (nf + 1.0) / 2.0;
    let mu = nf * mf / 2.0;
    let nt = nf + mf;
    let var = nf * mf / 12.0 * ((nt + 1.0) - tie_term / (nt * (nt - 1.0)).max(1.0));
    let z = if var > 0.0 {
        ((u - mu).abs() - 0.5).max(0.0) / var.sqrt() * (u - mu).signum()
    } else {
        0.0
    };
    let p_value = if var > 0.0 {
        2.0 * Normal::new(0.0, 1.0)
            .map(|d| d.sf(z.abs()))
            .unwrap_or(f64::NAN)
    } else {
        1.0
    };
    Some(MannWhitneyResult {
        u,
        z,
        p_value: p_value.min(1.0),
        effect: u / (nf * mf),
    })
}

/// Welch's interval for the difference of means (B − A).
pub fn welch_difference_ci(a: &[f64], b: &[f64], level: f64) -> Option<Interval> {
    let (a, b) = (sorted_finite(a), sorted_finite(b));
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let (va, vb) = (
        std_dev(&a).powi(2) / a.len() as f64,
        std_dev(&b).powi(2) / b.len() as f64,
    );
    let se = (va + vb).sqrt();
    let diff = mean(&b) - mean(&a);
    if se == 0.0 {
        return Some(Interval {
            lo: diff,
            hi: diff,
            level,
        });
    }
    let dof = (va + vb).powi(2) / (va * va / (a.len() - 1) as f64 + vb * vb / (b.len() - 1) as f64);
    let t = StudentsT::new(0.0, 1.0, dof)
        .ok()?
        .inverse_cdf(0.5 + level / 2.0);
    Some(Interval {
        lo: diff - t * se,
        hi: diff + t * se,
        level,
    })
}

/// Two batches side by side (§16: compare two templates or two catalogues):
/// both summaries, histograms on shared edges, and the tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleComparison {
    pub label_a: String,
    pub label_b: String,
    pub a: Summary,
    pub b: Summary,
    pub histogram_a: Histogram,
    pub histogram_b: Histogram,
    pub ks: Option<KsResult>,
    pub mann_whitney: Option<MannWhitneyResult>,
    /// Interval for mean(B) − mean(A).
    pub mean_difference: Option<Interval>,
}

/// Compare two samples of the same metric.
pub fn compare_samples(
    label_a: &str,
    a: &[f64],
    label_b: &str,
    b: &[f64],
    level: f64,
    bins: usize,
) -> SampleComparison {
    let all: Vec<f64> = sorted_finite(&[a, b].concat());
    let (lo, hi) = match (all.first(), all.last()) {
        (Some(&l), Some(&h)) if h > l => (l, h),
        (Some(&l), _) => (l - 0.5, l + 0.5),
        _ => (0.0, 1.0),
    };
    SampleComparison {
        label_a: label_a.into(),
        label_b: label_b.into(),
        a: Summary::of(a, level),
        b: Summary::of(b, level),
        histogram_a: Histogram::with_range(a, lo, hi, bins),
        histogram_b: Histogram::with_range(b, lo, hi, bins),
        ks: ks_two_sample(a, b),
        mann_whitney: mann_whitney(a, b),
        mean_difference: welch_difference_ci(a, b, level),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use bgws_core::{Seed, StreamKey};
    use rand::Rng;
    use statrs::distribution::Normal as N;

    fn normals(seed: u64, n: usize, mu: f64) -> Vec<f64> {
        let mut rng = Seed(seed).stream(StreamKey::new("test.normal", 0, 0, 0));
        let d = N::new(mu, 1.0).unwrap();
        (0..n)
            .map(|_| d.inverse_cdf(rng.gen::<f64>().clamp(1e-12, 1.0 - 1e-12)))
            .collect()
    }

    #[test]
    fn kolmogorov_tail_known_values() {
        // Q(1.36) ≈ 0.049, Q(1.63) ≈ 0.0098: the classic 5% and 1% critical values.
        assert_relative_eq!(kolmogorov_q(1.358), 0.05, epsilon = 2e-3);
        assert_relative_eq!(kolmogorov_q(1.628), 0.01, epsilon = 1e-3);
        // Continuity across the two series.
        assert_relative_eq!(kolmogorov_q(1.1799), kolmogorov_q(1.1801), epsilon = 1e-3);
        assert_eq!(kolmogorov_q(0.0), 1.0);
    }

    #[test]
    fn ks_two_sample_detects_shift_not_noise() {
        let a = normals(1, 400, 0.0);
        let b = normals(2, 400, 0.0);
        let c = normals(3, 400, 0.4);
        assert!(!ks_two_sample(&a, &b).unwrap().rejects(0.01));
        assert!(ks_two_sample(&a, &c).unwrap().rejects(0.01));
        // Identical samples: D = 0.
        assert_eq!(ks_two_sample(&a, &a).unwrap().d, 0.0);
        // Hand-checked: D between {1,2,3} and {2,3,4} is 1/3.
        assert_relative_eq!(
            ks_two_sample(&[1.0, 2.0, 3.0], &[2.0, 3.0, 4.0]).unwrap().d,
            1.0 / 3.0,
            epsilon = 1e-12
        );
    }

    #[test]
    fn ks_one_sample_against_uniform() {
        let mut rng = Seed(5).stream(StreamKey::new("test.u", 0, 0, 0));
        let u: Vec<f64> = (0..500).map(|_| rng.gen()).collect();
        assert!(!ks_one_sample(&u, |x| x.clamp(0.0, 1.0))
            .unwrap()
            .rejects(0.01));
        assert!(ks_one_sample(&u, |x| x.clamp(0.0, 1.0).powi(2))
            .unwrap()
            .rejects(0.01));
    }

    #[test]
    fn chi_squared_tests() {
        // A fair die rolled 60 times, counts {5,8,9,8,10,20}: χ² = 13.4, dof 5, p ≈ 0.0199.
        let obs = [5.0, 8.0, 9.0, 8.0, 10.0, 20.0];
        let r = chi_squared_gof(&obs, &[10.0; 6], 0, 5.0).unwrap();
        assert_relative_eq!(r.statistic, 13.4, epsilon = 1e-9);
        assert_eq!(r.dof, 5);
        assert_relative_eq!(r.p_value, 0.0199, epsilon = 5e-4);
        // Merging of sparse cells.
        let m = chi_squared_gof(&[1.0, 1.0, 10.0, 8.0], &[2.0, 2.0, 8.0, 8.0], 0, 5.0).unwrap();
        assert_eq!(m.cells, 2);
        let h = chi_squared_homogeneity(&[10, 20, 30], &[10, 20, 30]).unwrap();
        assert_relative_eq!(h.statistic, 0.0);
        assert!(chi_squared_homogeneity(&[50, 5], &[5, 50])
            .unwrap()
            .rejects(0.001));
    }

    #[test]
    fn mann_whitney_matches_hand_calculation() {
        // a = {1,2,3}, b = {4,5,6}: U_a = 0.
        let r = mann_whitney(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]).unwrap();
        assert_eq!(r.u, 0.0);
        assert_eq!(r.effect, 0.0);
        let a = normals(10, 200, 0.0);
        let c = normals(11, 200, 0.5);
        assert!(mann_whitney(&a, &c).unwrap().p_value < 0.001);
        let same = mann_whitney(&a, &a).unwrap();
        assert!(same.p_value > 0.9);
        assert_relative_eq!(same.effect, 0.5, epsilon = 1e-12);
    }

    #[test]
    fn comparison_shares_edges() {
        let a = normals(20, 100, 0.0);
        let b = normals(21, 100, 1.0);
        let c = compare_samples("A", &a, "B", &b, 0.95, 12);
        assert_eq!(c.histogram_a.edges, c.histogram_b.edges);
        assert_eq!(c.histogram_a.total() + c.histogram_b.total(), 200);
        let d = c.mean_difference.unwrap();
        assert!(d.lo > 0.5 && d.hi < 1.5, "{d:?}");
    }

    proptest::proptest! {
        #[test]
        fn ks_is_symmetric_and_bounded(a in proptest::collection::vec(-100f64..100.0, 1..40), b in proptest::collection::vec(-100f64..100.0, 1..40)) {
            let ab = ks_two_sample(&a, &b).unwrap();
            let ba = ks_two_sample(&b, &a).unwrap();
            proptest::prop_assert!((ab.d - ba.d).abs() < 1e-12);
            proptest::prop_assert!((0.0..=1.0).contains(&ab.d) && (0.0..=1.0).contains(&ab.p_value));
        }
    }
}
