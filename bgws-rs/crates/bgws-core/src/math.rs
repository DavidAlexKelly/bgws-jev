//! Small numerical helpers used across the models.

/// Standard normal CDF.
#[inline]
pub fn phi(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

/// P(a ≤ X ≤ b) for X ~ N(mu, sigma²).
#[inline]
pub fn normal_interval(mu: f64, sigma: f64, a: f64, b: f64) -> f64 {
    if sigma <= 1e-12 {
        return if mu >= a && mu <= b { 1.0 } else { 0.0 };
    }
    (phi((b - mu) / sigma) - phi((a - mu) / sigma)).max(0.0)
}

/// Probability that a round with bias (bx, by) and independent dispersion
/// (sx, sy) lands inside a centred rectangle of width w and height h. The
/// classic single-shot hit probability against a presented area (§8.8).
#[inline]
pub fn rect_hit_p(bx: f64, by: f64, sx: f64, sy: f64, w: f64, h: f64) -> f64 {
    normal_interval(bx, sx, -w / 2.0, w / 2.0) * normal_interval(by, sy, -h / 2.0, h / 2.0)
}

/// The chance an event with hazard rate `lambda` (per second) happens within `dt` seconds.
#[inline]
pub fn hazard_p(lambda: f64, dt: f64) -> f64 {
    if lambda <= 0.0 || dt <= 0.0 {
        0.0
    } else {
        1.0 - (-lambda * dt).exp()
    }
}

/// The hazard rate that gives probability `p` over `tau` seconds.
#[inline]
pub fn rate_for(p: f64, tau: f64) -> f64 {
    if p <= 0.0 {
        0.0
    } else if p >= 1.0 {
        1e6
    } else {
        -(1.0 - p).ln() / tau.max(1e-9)
    }
}

#[inline]
pub fn db(x: f64) -> f64 {
    10.0 * x.max(1e-300).log10()
}
#[inline]
pub fn from_db(d: f64) -> f64 {
    10f64.powf(d / 10.0)
}
/// Watts to dBm.
#[inline]
pub fn dbm(w: f64) -> f64 {
    db(w) + 30.0
}

#[inline]
pub fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Smallest signed difference between two angles, degrees, in (-180, 180].
#[inline]
pub fn angle_diff_deg(a: f64, b: f64) -> f64 {
    let mut d = (b - a) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// Logistic function.
#[inline]
pub fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Speed of light, m/s.
pub const C: f64 = 299_792_458.0;
/// Boltzmann constant, J/K.
pub const K_BOLTZ: f64 = 1.380_649e-23;
/// Standard noise temperature, K.
pub const T0: f64 = 290.0;
/// Gravity, m/s².
pub const G0: f64 = 9.806_65;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_and_hazard() {
        assert!((phi(0.0) - 0.5).abs() < 1e-12);
        assert!((phi(1.96) - 0.975).abs() < 1e-3);
        let p = hazard_p(rate_for(0.6, 10.0), 10.0);
        assert!((p - 0.6).abs() < 1e-12);
        // Two half-steps give the same as one full step: tick-independence.
        let l = 0.03;
        let two = 1.0 - (1.0 - hazard_p(l, 5.0)).powi(2);
        assert!((two - hazard_p(l, 10.0)).abs() < 1e-12);
        assert!((rect_hit_p(0.0, 0.0, 1.0, 1.0, 1e9, 1e9) - 1.0).abs() < 1e-9);
        assert!((angle_diff_deg(350.0, 10.0) - 20.0).abs() < 1e-9);
    }
}
