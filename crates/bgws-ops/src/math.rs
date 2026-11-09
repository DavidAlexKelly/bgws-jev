//! Small numerical helpers shared by the ops models: the error function, the
//! normal distribution, the logistic, the incomplete gamma function (for
//! chi-squared p-values), the Kolmogorov distribution (for KS p-values),
//! decibel arithmetic and a deterministic bisection root finder.
//!
//! Everything is plain f64 with no fast-math, so results are bit-stable for
//! a given build and machine (§12).

/// Complementary error function, Chebyshev fit (Numerical Recipes `erfcc`),
/// fractional error below 1.2e-7 everywhere.
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// Error function.
pub fn erf(x: f64) -> f64 {
    1.0 - erfc(x)
}

/// Standard normal CDF Φ(x).
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Gaussian Q-function, Q(x) = 1 − Φ(x).
pub fn q_function(x: f64) -> f64 {
    0.5 * erfc(x / std::f64::consts::SQRT_2)
}

/// The logistic function 1 / (1 + e^(−x)).
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// A smooth threshold: ≈0 well below `threshold`, 0.5 at it, ≈1 well above;
/// `width` is the distance over which it goes from 27% to 73%.
pub fn soft_step(x: f64, threshold: f64, width: f64) -> f64 {
    sigmoid((x - threshold) / width.max(1e-9))
}

/// Decibels to a power ratio.
pub fn db_to_ratio(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

/// A power ratio to decibels (−∞ for zero).
pub fn ratio_to_db(r: f64) -> f64 {
    10.0 * r.log10()
}

/// Power sum of levels in dB (dBW + dBW → dBW).
pub fn db_sum(levels: impl IntoIterator<Item = f64>) -> f64 {
    ratio_to_db(levels.into_iter().map(db_to_ratio).sum())
}

/// ln Γ(x) for x > 0 (Lanczos, g = 7, n = 9).
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection.
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularised lower incomplete gamma P(a, x).
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 || a <= 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        // Series.
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..1000 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (sum.ln() - x + a * x.ln() - ln_gamma(a)).exp().clamp(0.0, 1.0)
    } else {
        1.0 - gamma_q_cf(a, x)
    }
}

/// Upper incomplete gamma Q(a, x) by Lentz's continued fraction (x ≥ a + 1).
fn gamma_q_cf(a: f64, x: f64) -> f64 {
    let tiny = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / tiny;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..1000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < tiny {
            d = tiny;
        }
        c = b + an / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    ((-x + a * x.ln() - ln_gamma(a)).exp() * h).clamp(0.0, 1.0)
}

/// Survival function of the chi-squared distribution: P(X ≥ x) with `dof` degrees of freedom.
pub fn chi2_sf(x: f64, dof: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    1.0 - gamma_p(dof / 2.0, x / 2.0)
}

/// Asymptotic p-value of the one-sample Kolmogorov–Smirnov statistic `d`
/// with sample size `n` (Stephens' small-sample correction).
pub fn ks_p_value(d: f64, n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let sn = (n as f64).sqrt();
    let lambda = (sn + 0.12 + 0.11 / sn) * d;
    if lambda < 1e-3 {
        return 1.0;
    }
    let mut sum = 0.0;
    for j in 1..=100 {
        let jf = j as f64;
        let term = 2.0 * (-1f64).powi(j - 1) * (-2.0 * jf * jf * lambda * lambda).exp();
        sum += term;
        if term.abs() < 1e-12 {
            break;
        }
    }
    sum.clamp(0.0, 1.0)
}

/// Deterministic bisection for a root of `f` in [lo, hi]. Returns `None` when
/// `f(lo)` and `f(hi)` have the same sign.
pub fn bisect(mut lo: f64, mut hi: f64, f: impl Fn(f64) -> f64) -> Option<f64> {
    let mut flo = f(lo);
    let fhi = f(hi);
    if flo == 0.0 {
        return Some(lo);
    }
    if fhi == 0.0 {
        return Some(hi);
    }
    if flo.signum() == fhi.signum() {
        return None;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let fm = f(mid);
        if fm == 0.0 || (hi - lo).abs() <= 1e-9 * (1.0 + mid.abs()) {
            return Some(mid);
        }
        if fm.signum() == flo.signum() {
            lo = mid;
            flo = fm;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Free-space path loss in dB at distance `d_m` metres and frequency `f_hz`.
pub fn free_space_loss_db(d_m: f64, f_hz: f64) -> f64 {
    let lambda = bgws_core::units::SPEED_OF_LIGHT / f_hz;
    20.0 * (4.0 * std::f64::consts::PI * d_m.max(1e-3) / lambda).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn erf_known_values() {
        assert_relative_eq!(erf(0.5), 0.520_499_877_8, epsilon = 2e-7);
        assert_relative_eq!(erfc(-1.0), 1.842_700_792_9, epsilon = 2e-7);
        assert_relative_eq!(normal_cdf(1.96), 0.975, epsilon = 1e-4);
    }

    #[test]
    fn gamma_and_chi2() {
        assert_relative_eq!(ln_gamma(5.0), (24f64).ln(), epsilon = 1e-10);
        assert_relative_eq!(ln_gamma(0.5), std::f64::consts::PI.sqrt().ln(), epsilon = 1e-10);
        // chi2(1) at 3.841 → p = 0.05; chi2(10) at 18.307 → 0.05.
        assert_relative_eq!(chi2_sf(3.841_459, 1.0), 0.05, epsilon = 1e-5);
        assert_relative_eq!(chi2_sf(18.307_04, 10.0), 0.05, epsilon = 1e-5);
    }

    #[test]
    fn ks_critical_value() {
        // Asymptotic 5% critical value is 1.358 / sqrt(n).
        let n = 10_000;
        let p = ks_p_value(1.358 / (n as f64).sqrt(), n);
        assert!((p - 0.05).abs() < 0.005, "{p}");
    }

    #[test]
    fn bisection_and_db() {
        let r = bisect(0.0, 10.0, |x| x * x - 2.0).unwrap();
        assert_relative_eq!(r, 2f64.sqrt(), epsilon = 1e-8);
        assert!(bisect(0.0, 1.0, |x| x + 1.0).is_none());
        assert_relative_eq!(db_sum([0.0, 0.0]), 3.0103, epsilon = 1e-4);
        // FSPL at 1 km, 1 GHz ≈ 92.45 dB.
        assert_relative_eq!(free_space_loss_db(1000.0, 1e9), 92.45, epsilon = 0.01);
    }
}
