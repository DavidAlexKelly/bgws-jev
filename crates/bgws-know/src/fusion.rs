//! Covariance intersection (§9.2): how an HQ fuses track reports from
//! several sources whose errors may be correlated in unknown ways (the same
//! sighting relayed by two paths, two units both told by the same patrol).
//!
//! P⁻¹ = ω·Pa⁻¹ + (1 − ω)·Pb⁻¹ and P⁻¹x = ω·Pa⁻¹xa + (1 − ω)·Pb⁻¹xb, with ω
//! chosen to minimise the trace or the determinant of P. The result is
//! consistent for any cross-correlation, so double counting never makes an
//! HQ falsely certain. Naive Kalman fusion (which assumes independence) is
//! here only to show the difference.

use crate::filter::StateEstimate;
use nalgebra::{DMatrix, DVector, Matrix6, Vector6};
use serde::{Deserialize, Serialize};

/// What covariance intersection minimises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiCriterion {
    Trace,
    Determinant,
}

/// The fused estimate, with the weight chosen (for the AAR).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CiResult {
    pub x: DVector<f64>,
    pub p: DMatrix<f64>,
    /// Weight on the first estimate, in [0, 1].
    pub omega: f64,
    /// The minimised trace or ln det.
    pub cost: f64,
}

fn fused(ia: &DMatrix<f64>, ib: &DMatrix<f64>, w: f64) -> Option<DMatrix<f64>> {
    let info = ia * w + ib * (1.0 - w);
    info.cholesky().map(|c| c.inverse())
}

fn cost(p: &DMatrix<f64>, criterion: CiCriterion) -> f64 {
    match criterion {
        CiCriterion::Trace => p.trace(),
        CiCriterion::Determinant => match p.clone().cholesky() {
            Some(c) => 2.0 * c.l().diagonal().iter().map(|d| d.ln()).sum::<f64>(),
            None => f64::INFINITY,
        },
    }
}

/// Covariance intersection of two estimates of the same state. `None` if a
/// covariance is not positive-definite or the sizes differ.
pub fn covariance_intersection(
    xa: &DVector<f64>,
    pa: &DMatrix<f64>,
    xb: &DVector<f64>,
    pb: &DMatrix<f64>,
    criterion: CiCriterion,
) -> Option<CiResult> {
    if xa.len() != xb.len() || pa.nrows() != xa.len() || pb.nrows() != xb.len() {
        return None;
    }
    let ia = pa.clone().cholesky()?.inverse();
    let ib = pb.clone().cholesky()?.inverse();
    let eval = |w: f64| fused(&ia, &ib, w).map(|p| cost(&p, criterion)).unwrap_or(f64::INFINITY);
    // Golden-section search (both criteria are convex in ω), fixed iterations.
    let g = 0.5 * (5f64.sqrt() - 1.0);
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut c = hi - g * (hi - lo);
    let mut d = lo + g * (hi - lo);
    let (mut fc, mut fd) = (eval(c), eval(d));
    for _ in 0..60 {
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - g * (hi - lo);
            fc = eval(c);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + g * (hi - lo);
            fd = eval(d);
        }
    }
    // Compare with the end points: one source may simply dominate.
    let mut best = (0.5 * (lo + hi), eval(0.5 * (lo + hi)));
    for w in [0.0, 1.0] {
        let f = eval(w);
        if f < best.1 - 1e-12 {
            best = (w, f);
        }
    }
    let w = best.0;
    let mut p = fused(&ia, &ib, w)?;
    crate::stats::symmetrize(&mut p);
    let x = &p * (&ia * xa * w + &ib * xb * (1.0 - w));
    Some(CiResult { x, p, omega: w, cost: best.1 })
}

/// Naive information-form fusion that assumes the two errors independent.
/// Fusing a report with itself halves the covariance: the double-counting
/// error covariance intersection avoids.
pub fn naive_fusion(xa: &DVector<f64>, pa: &DMatrix<f64>, xb: &DVector<f64>, pb: &DMatrix<f64>) -> Option<(DVector<f64>, DMatrix<f64>)> {
    let ia = pa.clone().cholesky()?.inverse();
    let ib = pb.clone().cholesky()?.inverse();
    let p = (&ia + &ib).cholesky()?.inverse();
    let x = &p * (&ia * xa + &ib * xb);
    Some((x, p))
}

/// Covariance intersection of two common-form estimates, after predicting
/// the older one forward to the newer one's time with acceleration PSD `q`.
pub fn ci_estimates(a: &StateEstimate, b: &StateEstimate, q: f64, criterion: CiCriterion) -> Option<(StateEstimate, CiResult)> {
    let t = a.time.max(b.time);
    let (a, b) = (a.predicted(t, q), b.predicted(t, q));
    let to_d = |e: &StateEstimate| {
        (DVector::from_iterator(6, e.x.iter().copied()), DMatrix::from_iterator(6, 6, e.p.iter().copied()))
    };
    let (xa, pa) = to_d(&a);
    let (xb, pb) = to_d(&b);
    let r = covariance_intersection(&xa, &pa, &xb, &pb, criterion)?;
    let est = StateEstimate {
        time: t,
        x: Vector6::from_iterator(r.x.iter().copied()),
        p: Matrix6::from_iterator(r.p.iter().copied()),
    };
    Some((est, r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn est(x: [f64; 2], p: [f64; 4]) -> (DVector<f64>, DMatrix<f64>) {
        (DVector::from_row_slice(&x), DMatrix::from_row_slice(2, 2, &p))
    }

    #[test]
    fn fusing_the_same_report_twice_does_not_shrink() {
        let (x, p) = est([100.0, 50.0], [400.0, 50.0, 50.0, 100.0]);
        for crit in [CiCriterion::Trace, CiCriterion::Determinant] {
            let r = covariance_intersection(&x, &p, &x, &p, crit).unwrap();
            for (a, b) in r.p.iter().zip(p.iter()) {
                assert_relative_eq!(a, b, epsilon = 1e-9);
            }
            assert_relative_eq!(r.x[0], 100.0, epsilon = 1e-9);
        }
        let (_, pn) = naive_fusion(&x, &p, &x, &p).unwrap();
        // The naive filter halves it: falsely certain.
        assert_relative_eq!(pn[(0, 0)], 200.0, epsilon = 1e-9);
    }

    #[test]
    fn ci_uses_complementary_information() {
        // One source good in east, the other in north.
        let (xa, pa) = est([0.0, 0.0], [10.0, 0.0, 0.0, 1000.0]);
        let (xb, pb) = est([5.0, 5.0], [1000.0, 0.0, 0.0, 10.0]);
        let r = covariance_intersection(&xa, &pa, &xb, &pb, CiCriterion::Determinant).unwrap();
        assert_relative_eq!(r.omega, 0.5, epsilon = 1e-4);
        assert!(r.p[(0, 0)] < 25.0 && r.p[(1, 1)] < 25.0);
        // A dominating source: ω goes to it.
        let (xc, pc) = est([0.0, 0.0], [1.0, 0.0, 0.0, 1.0]);
        let (xd, pd) = est([9.0, 9.0], [100.0, 0.0, 0.0, 100.0]);
        let r = covariance_intersection(&xc, &pc, &xd, &pd, CiCriterion::Trace).unwrap();
        assert!(r.omega > 0.95);
        assert!(r.p.trace() <= pc.trace() + 1e-9);
    }

    #[test]
    fn ci_is_consistent_under_full_correlation() {
        // Two estimates that share their whole error: the true error covariance
        // is pa; CI must not report less than the best of the two.
        let (x, pa) = est([0.0, 0.0], [100.0, 0.0, 0.0, 100.0]);
        let pb = &pa * 4.0;
        let r = covariance_intersection(&x, &pa, &x, &pb, CiCriterion::Trace).unwrap();
        assert!(r.p.trace() >= pa.trace() - 1e-6);
    }
}
