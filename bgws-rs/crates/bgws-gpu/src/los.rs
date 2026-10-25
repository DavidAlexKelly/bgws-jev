//! Batched line of sight over a height grid (§11.1, CPU twin).
//!
//! Each query is an observer and a target, both as a ground position plus a
//! height above ground. The ray is marched at a fixed fraction of the grid
//! spacing; at every interior sample the terrain is raised by the
//! effective-Earth bulge `t(1−t)D²/(2kR)` (curvature with refraction factor
//! `k`: 1 geometric, ≈1.15 optical, 4/3 radio — §5.2), and the clearance is
//! the smallest gap between the chord and that raised terrain.
//!
//! Outputs feed decisions, so they are quantised (§12): clearance in whole
//! centimetres, and `visible` is decided from the quantised value. The GPU
//! twin (feature `gpu`) re-runs any query within a small margin of zero on
//! the CPU in f64, so `visible` agrees bit-for-bit between the twins.

use crate::grid::HeightGrid;
use bgws_core::geo::EARTH_RADIUS;
use bgws_core::Figures;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// One observer–target pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LosQuery {
    pub obs_x: f64,
    pub obs_y: f64,
    /// Observer eye or antenna height above ground, metres.
    pub obs_agl: f64,
    pub tgt_x: f64,
    pub tgt_y: f64,
    /// Target height above ground, metres.
    pub tgt_agl: f64,
}

/// The answer for one pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LosResult {
    pub visible: bool,
    /// Smallest clearance of the ray over the (curvature-raised) terrain, in
    /// centimetres; negative when masked. With no interior sample (adjacent
    /// points) it is the smaller of the two heights above ground.
    pub clearance_cm: i32,
}

impl LosResult {
    pub fn clearance_m(self) -> f64 {
        self.clearance_cm as f64 / 100.0
    }
}

/// Parameters of the march.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LosParams {
    /// Effective Earth radius factor (1, ≈1.15 optical, 4/3 radio).
    pub k_factor: f64,
    /// Sample spacing as a fraction of the grid spacing.
    pub step_cells: f64,
    /// Clearance quantum, metres (0.01 = centimetres).
    pub quantum_m: f64,
    /// GPU results within this margin of zero are recomputed on the CPU.
    pub refine_margin_m: f64,
}

impl LosParams {
    /// Defaults from the figures registry, with the caller's `k`.
    pub fn from_figures(f: &Figures, k_factor: f64) -> Self {
        Self {
            k_factor,
            step_cells: f.get("gpu.los.step_cells"),
            quantum_m: f.get("gpu.los.clearance_quantum_m"),
            refine_margin_m: f.get("gpu.los.refine_margin_m"),
        }
    }
}

impl Default for LosParams {
    fn default() -> Self {
        Self { k_factor: 4.0 / 3.0, step_cells: 0.5, quantum_m: 0.01, refine_margin_m: 0.5 }
    }
}

/// Number of march intervals for a ground distance `d`.
#[inline]
pub(crate) fn steps_for(d: f64, cell: f64, step_cells: f64) -> u32 {
    let step = (cell * step_cells).max(1e-6);
    ((d / step).ceil() as u32).max(1)
}

/// Unquantised clearance in metres (f64 reference implementation).
pub fn clearance_f64(grid: &HeightGrid, q: &LosQuery, p: &LosParams) -> f64 {
    let dx = q.tgt_x - q.obs_x;
    let dy = q.tgt_y - q.obs_y;
    let d = dx.hypot(dy);
    let zo = grid.height_at(q.obs_x, q.obs_y) + q.obs_agl;
    let zt = grid.height_at(q.tgt_x, q.tgt_y) + q.tgt_agl;
    let n = steps_for(d, grid.cell, p.step_cells);
    if n < 2 {
        return q.obs_agl.min(q.tgt_agl);
    }
    let inv_2kr = 1.0 / (2.0 * p.k_factor * EARTH_RADIUS);
    let mut min_clear = f64::INFINITY;
    for s in 1..n {
        let t = s as f64 / n as f64;
        let ground = grid.height_at(q.obs_x + t * dx, q.obs_y + t * dy);
        let bulge = t * (1.0 - t) * d * d * inv_2kr;
        let line = zo + t * (zt - zo);
        let c = line - (ground + bulge);
        if c < min_clear {
            min_clear = c;
        }
    }
    min_clear
}

/// Quantise a clearance to the result type.
#[inline]
pub fn quantise(clear_m: f64, quantum_m: f64) -> LosResult {
    let q = (clear_m / quantum_m).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    LosResult { visible: q >= 0, clearance_cm: ((q as f64) * quantum_m * 100.0).round() as i32 }
}

/// One query on the CPU.
pub fn los_one(grid: &HeightGrid, q: &LosQuery, p: &LosParams) -> LosResult {
    quantise(clearance_f64(grid, q, p), p.quantum_m)
}

/// The CPU twin: rayon-parallel, order-preserving, deterministic.
pub fn los_batch_cpu(grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Vec<LosResult> {
    queries.par_iter().map(|q| los_one(grid, q, p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridge() -> HeightGrid {
        // 101×3 nodes at 10 m, a 50 m ridge at x = 500.
        let mut h = vec![0f32; 101 * 3];
        for j in 0..3 {
            h[j * 101 + 50] = 50.0;
        }
        HeightGrid::new(101, 3, 10.0, 0.0, 0.0, h).unwrap()
    }

    #[test]
    fn ridge_masks_low_and_not_high() {
        let g = ridge();
        let p = LosParams { k_factor: 1e9, ..Default::default() };
        let low = los_one(&g, &LosQuery { obs_x: 0.0, obs_y: 10.0, obs_agl: 2.0, tgt_x: 1000.0, tgt_y: 10.0, tgt_agl: 2.0 }, &p);
        assert!(!low.visible);
        assert_eq!(low.clearance_cm, -4800);
        let high = los_one(&g, &LosQuery { obs_x: 0.0, obs_y: 10.0, obs_agl: 60.0, tgt_x: 1000.0, tgt_y: 10.0, tgt_agl: 60.0 }, &p);
        assert!(high.visible);
        assert_eq!(high.clearance_cm, 1000);
    }

    #[test]
    fn curvature_hides_over_flat_ground() {
        // 40 km over flat ground, both at 2 m: the bulge (≈ 23.5 m geometric at mid-path) masks.
        let g = HeightGrid::flat(401, 2, 100.0, 0.0);
        let q = LosQuery { obs_x: 0.0, obs_y: 0.0, obs_agl: 2.0, tgt_x: 40_000.0, tgt_y: 0.0, tgt_agl: 2.0 };
        let geo = los_one(&g, &q, &LosParams { k_factor: 1.0, ..Default::default() });
        assert!(!geo.visible);
        let expected = 2.0 - 20_000.0f64 * 20_000.0 / (2.0 * EARTH_RADIUS);
        assert!((geo.clearance_m() - expected).abs() < 0.05, "{} vs {expected}", geo.clearance_m());
        // From 100 m masts the radio horizon (2 × 41 km) clears it.
        let q2 = LosQuery { obs_agl: 100.0, tgt_agl: 100.0, ..q };
        assert!(los_one(&g, &q2, &LosParams::default()).visible);
    }

    #[test]
    fn batch_preserves_order() {
        let g = ridge();
        let qs: Vec<_> = (0..50)
            .map(|k| LosQuery { obs_x: 0.0, obs_y: 10.0, obs_agl: k as f64 * 2.0, tgt_x: 1000.0, tgt_y: 10.0, tgt_agl: k as f64 * 2.0 })
            .collect();
        let r = los_batch_cpu(&g, &qs, &LosParams::default());
        for (q, r) in qs.iter().zip(&r) {
            assert_eq!(*r, los_one(&g, q, &LosParams::default()));
        }
    }
}
