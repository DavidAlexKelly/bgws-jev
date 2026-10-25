//! Association (§9.1): Mahalanobis gating and global nearest neighbour
//! assignment by Kuhn–Munkres.
//!
//! Rows are detections, columns are tracks. Each detection also gets its own
//! "no track" column whose cost is its gate, so the matrix is always at
//! least as wide as it is tall (rectangular and unassigned cases are
//! handled by construction) and leaving a detection unassigned is preferred
//! to any assignment outside its gate. Costs are scaled to integers so the
//! solver is exact and platform-independent.

use pathfinding::kuhn_munkres::kuhn_munkres;
use pathfinding::matrix::Matrix;
use serde::{Deserialize, Serialize};

/// Integer cost resolution: 1/1000 of a unit of Mahalanobis distance squared.
/// (A numerical precision, not a model figure.)
const COST_SCALE: f64 = 1000.0;
/// The cost of a forbidden pairing; larger than any sum of allowed costs.
const FORBIDDEN: i64 = 1_000_000_000_000;

/// One gated candidate pairing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub detection: usize,
    pub track: usize,
    /// Mahalanobis distance squared.
    pub d2: f64,
}

/// The assignment for one scan: for each detection, the track it went to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    /// `Some(track)` or `None` (start a new track / DF fix).
    pub of_detection: Vec<Option<usize>>,
    /// Total cost of the real assignments (d²).
    pub cost: f64,
}

/// Global nearest neighbour over `n_det` detections and `n_trk` tracks.
/// `candidates` are the in-gate pairs; `gates[i]` is detection `i`'s gate
/// (its cost of going unassigned).
pub fn gnn(n_det: usize, n_trk: usize, candidates: &[Candidate], gates: &[f64]) -> Assignment {
    if n_det == 0 {
        return Assignment { of_detection: vec![], cost: 0.0 };
    }
    if candidates.is_empty() {
        return Assignment { of_detection: vec![None; n_det], cost: 0.0 };
    }
    let cols = n_trk + n_det;
    let mut w = vec![-FORBIDDEN; n_det * cols];
    for c in candidates {
        if c.detection < n_det && c.track < n_trk && c.d2.is_finite() {
            w[c.detection * cols + c.track] = -((c.d2 * COST_SCALE).round() as i64);
        }
    }
    for (i, g) in gates.iter().enumerate().take(n_det) {
        w[i * cols + n_trk + i] = -((g * COST_SCALE).round() as i64);
    }
    let Ok(m) = Matrix::from_vec(n_det, cols, w) else {
        return Assignment { of_detection: vec![None; n_det], cost: 0.0 };
    };
    let (_, assign) = kuhn_munkres(&m);
    let mut out = vec![None; n_det];
    let mut cost = 0.0;
    for (i, &j) in assign.iter().enumerate() {
        if j < n_trk && m[(i, j)] > -FORBIDDEN {
            out[i] = Some(j);
            cost += candidates.iter().find(|c| c.detection == i && c.track == j).map(|c| c.d2).unwrap_or(0.0);
        }
    }
    Assignment { of_detection: out, cost }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(d: usize, t: usize, d2: f64) -> Candidate {
        Candidate { detection: d, track: t, d2 }
    }

    #[test]
    fn global_beats_greedy() {
        // Greedy would give d0→t0 (1.0) and leave d1 with t1 (8.0) = 9.0;
        // the global optimum is d0→t1 (2.0) + d1→t0 (2.0) = 4.0.
        let cands = [c(0, 0, 1.0), c(0, 1, 2.0), c(1, 0, 2.0), c(1, 1, 8.0)];
        let a = gnn(2, 2, &cands, &[9.0, 9.0]);
        assert_eq!(a.of_detection, vec![Some(1), Some(0)]);
        assert!((a.cost - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rectangular_and_unassigned() {
        // Three detections, one track: only one can take it.
        let a = gnn(3, 1, &[c(0, 0, 3.0), c(1, 0, 1.0), c(2, 0, 2.0)], &[9.0; 3]);
        assert_eq!(a.of_detection, vec![None, Some(0), None]);
        // More tracks than detections.
        let a = gnn(1, 4, &[c(0, 2, 0.5), c(0, 3, 0.2)], &[9.0]);
        assert_eq!(a.of_detection, vec![Some(3)]);
        // Nothing in gate.
        let a = gnn(2, 2, &[], &[9.0, 9.0]);
        assert_eq!(a.of_detection, vec![None, None]);
        // An out-of-gate cost loses to staying unassigned.
        let a = gnn(1, 1, &[c(0, 0, 12.0)], &[9.0]);
        assert_eq!(a.of_detection, vec![None]);
    }
}
