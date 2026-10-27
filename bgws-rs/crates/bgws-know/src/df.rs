//! Direction finding (§8.7, §9.1): bearing-only detections cannot start a
//! track alone. The store keeps unassociated bearings for a while and,
//! when bearings from different places cross at a useful angle, fixes a
//! position by weighted least squares and starts a track from it.
//!
//! Each bearing θ from origin o is the line n·p = n·o with normal
//! n = (cos θ, −sin θ) in (east, north). Its weight is 1/(σθ·r)², the
//! cross-range variance at range r; the fix is iterated once with ranges
//! from the first solution. The fix covariance is (Σ w n nᵀ)⁻¹ **[ref P32]**.

use bgws_core::Enu;
use nalgebra::{Matrix2, Vector2};
use serde::{Deserialize, Serialize};

/// One line of bearing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BearingLine {
    pub origin: Enu,
    /// Radians clockwise from north.
    pub bearing: f64,
    pub sigma: f64,
}

/// A DF fix, with what went into it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Triangulation {
    /// East, north.
    pub pos: Vector2<f64>,
    pub cov: Matrix2<f64>,
    /// The widest crossing angle between any two lines used, radians in [0, π/2].
    pub crossing_angle: f64,
    /// Weighted sum of squared residuals (perpendicular miss / cross-range σ)².
    pub residual: f64,
    pub lines: usize,
}

/// The crossing angle of two bearings, folded to [0, π/2].
pub fn crossing_angle(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::PI);
    d.min(std::f64::consts::PI - d)
}

/// Fix a position from two or more bearings. `None` if they are (nearly)
/// parallel, if the fix lies behind any observer, or it is beyond `max_range`.
pub fn triangulate(lines: &[BearingLine], min_crossing: f64, max_range: f64) -> Option<Triangulation> {
    if lines.len() < 2 {
        return None;
    }
    let mut widest: f64 = 0.0;
    for i in 0..lines.len() {
        for j in (i + 1)..lines.len() {
            widest = widest.max(crossing_angle(lines[i].bearing, lines[j].bearing));
        }
    }
    if widest < min_crossing {
        return None;
    }
    let solve = |ranges: Option<&[f64]>| -> Option<(Vector2<f64>, Matrix2<f64>)> {
        let mut a = Matrix2::zeros();
        let mut b = Vector2::zeros();
        for (k, l) in lines.iter().enumerate() {
            let (s, c) = l.bearing.sin_cos();
            let n = Vector2::new(c, -s);
            let r = ranges.map(|r| r[k].max(1.0)).unwrap_or(1.0);
            let w = 1.0 / (l.sigma.max(1e-6) * r).powi(2);
            a += n * n.transpose() * w;
            b += n * (n.dot(&Vector2::new(l.origin.e, l.origin.n))) * w;
        }
        let inv = a.try_inverse()?;
        Some((inv * b, inv))
    };
    let (p0, _) = solve(None)?;
    let ranges: Vec<f64> = lines.iter().map(|l| (p0.x - l.origin.e).hypot(p0.y - l.origin.n)).collect();
    let (p, cov) = solve(Some(&ranges))?;
    let mut residual = 0.0;
    for l in lines {
        let (s, c) = l.bearing.sin_cos();
        let d = Vector2::new(p.x - l.origin.e, p.y - l.origin.n);
        // In front of the observer, and within range.
        if d.dot(&Vector2::new(s, c)) <= 0.0 || d.norm() > max_range {
            return None;
        }
        let miss = d.dot(&Vector2::new(c, -s));
        residual += (miss / (l.sigma * d.norm().max(1.0))).powi(2);
    }
    let cov = 0.5 * (cov + cov.transpose());
    Some(Triangulation { pos: p, cov, crossing_angle: widest, residual, lines: lines.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn line(o: (f64, f64), to: (f64, f64), sigma: f64) -> BearingLine {
        BearingLine { origin: Enu::new(o.0, o.1, 0.0), bearing: (to.0 - o.0).atan2(to.1 - o.1), sigma }
    }

    #[test]
    fn two_bearings_fix_the_emitter() {
        let t = (5000.0, 8000.0);
        let a = line((0.0, 0.0), t, 0.01);
        let b = line((10_000.0, 0.0), t, 0.01);
        let fix = triangulate(&[a, b], 0.2, 50_000.0).unwrap();
        assert_relative_eq!(fix.pos.x, t.0, epsilon = 1e-6);
        assert_relative_eq!(fix.pos.y, t.1, epsilon = 1e-6);
        // Error ellipse of order σθ·r ≈ 0.01 × 9.4 km ≈ 94 m, inflated by the crossing geometry.
        let e = crate::stats::Ellipse::from_cov(&fix.cov);
        assert!(e.semi_minor > 40.0 && e.semi_major < 400.0, "{e:?}");
    }

    #[test]
    fn rejects_parallel_and_behind() {
        let a = line((0.0, 0.0), (0.0, 10_000.0), 0.01);
        let b = line((100.0, 0.0), (100.0, 10_000.0), 0.01);
        assert!(triangulate(&[a, b], 0.2, 50_000.0).is_none());
        // Lines that would cross behind both observers.
        let c = BearingLine { origin: Enu::new(0.0, 0.0, 0.0), bearing: -0.5, sigma: 0.01 };
        let d = BearingLine { origin: Enu::new(1000.0, 0.0, 0.0), bearing: 0.5, sigma: 0.01 };
        assert!(triangulate(&[c, d], 0.2, 50_000.0).is_none());
    }
}
