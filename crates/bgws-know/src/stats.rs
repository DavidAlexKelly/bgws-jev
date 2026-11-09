//! Small numerical helpers the trackers need (§9.1): the chi-squared
//! distribution (gates and consistency bounds), angle wrapping, Gaussian
//! draws from a caller's stream, covariance ellipses and positive-definite
//! checks. Everything is plain f64 and deterministic: quantiles come from a
//! fixed-iteration bisection, not a platform library.

use nalgebra::{DMatrix, Matrix2};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// ln Γ(x) for x > 0 (Lanczos, g = 7, nine coefficients; ~1e-15 relative).
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
        // Reflection: Γ(x)Γ(1−x) = π / sin(πx).
        return (PI / (PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (TAU).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularised lower incomplete gamma function P(a, x).
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let gln = ln_gamma(a);
    if x < a + 1.0 {
        // Series.
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..500 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (sum * (-x + a * x.ln() - gln).exp()).clamp(0.0, 1.0)
    } else {
        // Continued fraction (modified Lentz) for Q, then P = 1 − Q.
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..500 {
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
        (1.0 - (-x + a * x.ln() - gln).exp() * h).clamp(0.0, 1.0)
    }
}

/// CDF of the chi-squared distribution with `dof` degrees of freedom.
pub fn chi2_cdf(x: f64, dof: f64) -> f64 {
    gamma_p(dof / 2.0, x / 2.0)
}

/// The `p`-quantile of chi-squared with `dof` degrees of freedom, by
/// bisection (fixed 200 iterations, so it is bit-stable).
pub fn chi2_quantile(p: f64, dof: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let (mut lo, mut hi) = (0.0, dof + 60.0 * (2.0 * dof).sqrt() + 100.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if chi2_cdf(mid, dof) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Wrap an angle to (−π, π].
pub fn wrap_pi(a: f64) -> f64 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI {
        w + TAU
    } else {
        w
    }
}

/// One standard normal draw (Box–Muller) from the caller's stream.
pub fn std_normal(rng: &mut impl Rng) -> f64 {
    let u1: f64 = rng.gen::<f64>().max(1e-300);
    let u2: f64 = rng.gen::<f64>();
    (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
}

/// A draw from N(0, cov) via the Cholesky factor. `None` if `cov` is not
/// positive-definite.
pub fn gaussian_vector(cov: &DMatrix<f64>, rng: &mut impl Rng) -> Option<Vec<f64>> {
    let l = cov.clone().cholesky()?.l();
    let z: Vec<f64> = (0..cov.nrows()).map(|_| std_normal(rng)).collect();
    Some((0..cov.nrows()).map(|i| (0..=i).map(|j| l[(i, j)] * z[j]).sum()).collect())
}

/// Make a matrix exactly symmetric (after a Joseph update or a prediction).
pub fn symmetrize(p: &mut DMatrix<f64>) {
    let n = p.nrows();
    for i in 0..n {
        for j in (i + 1)..n {
            let m = 0.5 * (p[(i, j)] + p[(j, i)]);
            p[(i, j)] = m;
            p[(j, i)] = m;
        }
    }
}

/// Symmetric and positive-definite (Cholesky succeeds).
pub fn is_spd(p: &DMatrix<f64>) -> bool {
    let n = p.nrows();
    if n != p.ncols() {
        return false;
    }
    for i in 0..n {
        for j in 0..n {
            let a = p[(i, j)];
            if !a.is_finite() || (a - p[(j, i)]).abs() > 1e-9 * (1.0 + a.abs()) {
                return false;
            }
        }
    }
    p.clone().cholesky().is_some()
}

/// A horizontal error ellipse at one standard deviation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ellipse {
    /// Semi-major axis, metres (1σ).
    pub semi_major: f64,
    /// Semi-minor axis, metres (1σ).
    pub semi_minor: f64,
    /// Orientation of the major axis, radians clockwise from north, in [0, π).
    pub orientation: f64,
}

impl Ellipse {
    /// From the east/north covariance block.
    pub fn from_cov(c: &Matrix2<f64>) -> Ellipse {
        let (a, b, d) = (c[(0, 0)], 0.5 * (c[(0, 1)] + c[(1, 0)]), c[(1, 1)]);
        let mid = 0.5 * (a + d);
        let rad = (0.25 * (a - d) * (a - d) + b * b).sqrt();
        let (l1, l2) = (mid + rad, (mid - rad).max(0.0));
        // Major eigenvector (east, north).
        let (ve, vn) = if b.abs() > 1e-12 {
            (b, l1 - a)
        } else if a >= d {
            (1.0, 0.0)
        } else {
            (0.0, 1.0)
        };
        Ellipse { semi_major: l1.sqrt(), semi_minor: l2.sqrt(), orientation: ve.atan2(vn).rem_euclid(PI) }
    }

    /// Scale to a containment probability `p` (2 dof chi-squared).
    pub fn scaled(self, p: f64) -> Ellipse {
        let k = chi2_quantile(p, 2.0).sqrt();
        Ellipse { semi_major: self.semi_major * k, semi_minor: self.semi_minor * k, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn chi2_matches_tables() {
        // Standard table values.
        assert_relative_eq!(chi2_quantile(0.95, 1.0), 3.841, epsilon = 2e-3);
        assert_relative_eq!(chi2_quantile(0.99, 2.0), 9.210, epsilon = 2e-3);
        assert_relative_eq!(chi2_quantile(0.99, 4.0), 13.277, epsilon = 2e-3);
        assert_relative_eq!(chi2_quantile(0.025, 100.0), 74.222, epsilon = 1e-2);
        assert_relative_eq!(chi2_cdf(2.0 * (-(0.05f64).ln()), 2.0), 0.95, epsilon = 1e-9);
        assert_relative_eq!(ln_gamma(5.0), (24.0f64).ln(), epsilon = 1e-12);
    }

    #[test]
    fn wrap_and_ellipse() {
        assert_relative_eq!(wrap_pi(3.0 * PI / 2.0), -PI / 2.0, epsilon = 1e-12);
        assert_relative_eq!(wrap_pi(PI), PI);
        let e = Ellipse::from_cov(&Matrix2::new(100.0, 0.0, 0.0, 400.0));
        assert_relative_eq!(e.semi_major, 20.0);
        assert_relative_eq!(e.semi_minor, 10.0);
        assert_relative_eq!(e.orientation, 0.0); // along north
        // Elongated along the north-east diagonal: orientation 45°.
        let e = Ellipse::from_cov(&Matrix2::new(250.0, 150.0, 150.0, 250.0));
        assert_relative_eq!(e.orientation, PI / 4.0, epsilon = 1e-9);
        assert_relative_eq!(e.semi_major, 20.0, epsilon = 1e-9);
    }
}
