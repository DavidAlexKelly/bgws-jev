//! Numerical helpers shared by the models: the normal distribution through
//! `libm` (stable across platforms, §12), its inverse, the interval
//! probability that every rectangle-integral in the crate is built from, the
//! scaled Bessel function I₀ for the Rician (Marcum Q) integral, and Simpson
//! integration. No figures: these are mathematics, not model constants.

use std::f64::consts::{PI, SQRT_2};

/// The error function (libm, so the last bit is the same on every machine).
pub fn erf(x: f64) -> f64 {
    libm::erf(x)
}

/// The complementary error function.
pub fn erfc(x: f64) -> f64 {
    libm::erfc(x)
}

/// Standard normal CDF Φ(x).
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / SQRT_2)
}

/// Standard normal density φ(x).
pub fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

/// Inverse standard normal CDF (Acklam's rational approximation refined by
/// one Halley step: |error| < 1e-12 on (0, 1)).
pub fn norm_inv(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let plow = 0.02425;
    let x = if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - plow {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // One Halley refinement.
    let e = norm_cdf(x) - p;
    let u = e * (2.0 * PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

/// P(lo ≤ X ≤ hi) for X ~ N(mu, sigma²). With sigma = 0 it is an indicator.
pub fn normal_interval(lo: f64, hi: f64, mu: f64, sigma: f64) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    if sigma <= 0.0 {
        return if mu >= lo && mu <= hi { 1.0 } else { 0.0 };
    }
    let s = sigma * SQRT_2;
    (0.5 * (erf((hi - mu) / s) - erf((lo - mu) / s))).clamp(0.0, 1.0)
}

/// Mean of exp(−(m − μ)²/c) for m uniform on [−L/2, L/2] (exact, via erf).
/// With L → 0 it is the point value exp(−μ²/c).
pub fn mean_gauss_over_interval(len: f64, mu: f64, c: f64) -> f64 {
    if c <= 0.0 {
        return if mu.abs() <= len / 2.0 { 1.0 } else { 0.0 };
    }
    if len <= 1e-9 {
        return (-mu * mu / c).exp();
    }
    let sc = c.sqrt();
    (PI * c).sqrt() / (2.0 * len) * (erf((len / 2.0 - mu) / sc) - erf((-len / 2.0 - mu) / sc))
}

/// Exponentially scaled modified Bessel function e^(−|x|)·I₀(x)
/// (Abramowitz & Stegun 9.8.1–9.8.2, |relative error| < 2e-7).
pub fn bessel_i0e(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let t = (x / 3.75).powi(2);
        let i0 = 1.0
            + t * (3.515_622_9 + t * (3.089_942_4 + t * (1.206_749_2 + t * (0.265_973_2 + t * (0.036_076_8 + t * 0.004_581_3)))));
        i0 * (-ax).exp()
    } else {
        let t = 3.75 / ax;
        (0.398_942_28
            + t * (0.013_285_92
                + t * (0.002_253_19
                    + t * (-0.001_575_65
                        + t * (0.009_162_81 + t * (-0.020_577_06 + t * (0.026_355_37 + t * (-0.016_476_33 + t * 0.003_923_77))))))))
            / ax.sqrt()
    }
}

/// Composite Simpson integration of `f` on [a, b] with `n` (made even) panels.
pub fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, n: usize) -> f64 {
    let n = (n.max(2) + 1) & !1;
    let h = (b - a) / n as f64;
    let mut s = f(a) + f(b);
    for i in 1..n {
        let x = a + h * i as f64;
        s += if i % 2 == 1 { 4.0 * f(x) } else { 2.0 * f(x) };
    }
    s * h / 3.0
}

/// Marcum Q₁(a, b): the probability a Rician variable with parameter `a`
/// exceeds `b`. Integrated numerically on the scaled integrand
/// x·exp(−(x − a)²/2)·e^(−ax)I₀(ax), which never overflows.
pub fn marcum_q1(a: f64, b: f64) -> f64 {
    if b <= 0.0 {
        return 1.0;
    }
    let a = a.max(0.0);
    let upper = (a + 12.0).max(b + 12.0);
    if b >= upper {
        return 0.0;
    }
    let f = |x: f64| x * (-(x - a) * (x - a) / 2.0).exp() * bessel_i0e(a * x);
    simpson(f, b, upper, 2000).clamp(0.0, 1.0)
}

/// Linear interpolation in a table of (x, y) points sorted by x, clamped at
/// both ends. Returns `None` for an empty table.
pub fn interp(table: &[(f64, f64)], x: f64) -> Option<f64> {
    let first = table.first()?;
    if x <= first.0 {
        return Some(first.1);
    }
    for w in table.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x <= b.0 {
            if b.0 <= a.0 {
                return Some(b.1);
            }
            return Some(a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0));
        }
    }
    table.last().map(|l| l.1)
}

/// Logistic function 1/(1 + e^(−x)).
pub fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn normal_functions() {
        assert_relative_eq!(norm_cdf(0.0), 0.5, epsilon = 1e-15);
        assert_relative_eq!(norm_cdf(1.959_964), 0.975, epsilon = 1e-6);
        for p in [1e-6, 0.01, 0.3, 0.5, 0.8, 0.999] {
            assert_relative_eq!(norm_cdf(norm_inv(p)), p, epsilon = 1e-10);
        }
        assert_relative_eq!(normal_interval(-1.0, 1.0, 0.0, 1.0), 0.682_689_5, epsilon = 1e-6);
    }

    #[test]
    fn gaussian_interval_mean_matches_quadrature() {
        let (l, mu, c) = (30.0, 7.0, 200.0);
        let q = simpson(|m| (-(m - mu) * (m - mu) / c).exp(), -l / 2.0, l / 2.0, 400) / l;
        assert_relative_eq!(mean_gauss_over_interval(l, mu, c), q, epsilon = 1e-9);
    }

    #[test]
    fn marcum_q_limits() {
        // Q1(0, b) = exp(−b²/2).
        assert_relative_eq!(marcum_q1(0.0, 2.0), (-2.0f64).exp(), epsilon = 1e-6);
        // Large a: Q1 ≈ Φ(a − b).
        assert_relative_eq!(marcum_q1(20.0, 20.0), 0.5, epsilon = 0.02);
        assert_relative_eq!(bessel_i0e(1.0), 1.266_065_878 * (-1.0f64).exp(), epsilon = 1e-6);
    }

    #[test]
    fn interpolation() {
        let t = [(0.0, 10.0), (1000.0, 8.0), (2000.0, 5.0)];
        assert_eq!(interp(&t, -5.0), Some(10.0));
        assert_eq!(interp(&t, 1500.0), Some(6.5));
        assert_eq!(interp(&t, 9e9), Some(5.0));
        assert_eq!(interp(&[], 1.0), None);
    }
}
