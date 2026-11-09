//! Spectrum fields (§7.5, §11.1): a path-loss raster from one emitter, and
//! the received-power / J/S rasters that sum emitters over it.
//!
//! The path loss per node is free space plus a single dominant knife edge
//! (the sample with the largest Fresnel–Kirchhoff parameter ν, loss J(ν) in
//! the ITU-R P.526 approximation) over the curvature-raised profile. That is
//! the field-grade model for coverage and jamming maps; exact per-link values
//! use the multiple-edge model in `bgws-prop` (§7.5: "exact per-link values
//! are computed on the CPU at the receivers that matter").
//!
//! Sums of powers run in a fixed emitter order so the result does not depend
//! on thread scheduling (§12). Outputs are quantised to `quantum_db`.

use crate::grid::HeightGrid;
use crate::los::steps_for;
use bgws_core::geo::EARTH_RADIUS;
use bgws_core::units::SPEED_OF_LIGHT;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// An emitter for a path-loss field.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldEmitter {
    pub x: f64,
    pub y: f64,
    /// Antenna height above ground, metres.
    pub agl: f64,
    pub freq_hz: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathLossParams {
    /// Receiver antenna height above ground at every node, metres.
    pub rx_agl: f64,
    /// Effective Earth radius factor (4/3 for radio).
    pub k_factor: f64,
    /// Profile sample spacing as a fraction of the grid spacing.
    pub step_cells: f64,
    /// Output quantum, dB.
    pub quantum_db: f64,
}

impl PathLossParams {
    pub fn from_figures(f: &bgws_core::Figures, rx_agl: f64) -> Self {
        Self {
            rx_agl,
            k_factor: bgws_core::geo::K_RADIO,
            step_cells: f.get("gpu.los.step_cells"),
            quantum_db: f.get("gpu.field.quantum_db"),
        }
    }
}

impl Default for PathLossParams {
    fn default() -> Self {
        Self { rx_agl: 2.0, k_factor: bgws_core::geo::K_RADIO, step_cells: 0.5, quantum_db: 0.01 }
    }
}

/// A raster of values in dB (path loss) or dBW (power).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DbField {
    pub width: usize,
    pub height: usize,
    pub db: Vec<f64>,
}

impl DbField {
    pub fn at(&self, i: usize, j: usize) -> f64 {
        self.db[j * self.width + i]
    }
}

/// Free-space path loss, dB, for slant distance `d` metres at `f` Hz.
#[inline]
pub fn fspl_db(d: f64, f: f64) -> f64 {
    let lambda = SPEED_OF_LIGHT / f;
    20.0 * (4.0 * std::f64::consts::PI * d.max(1.0) / lambda).log10()
}

/// Knife-edge diffraction loss J(ν), dB (ITU-R P.526 eq. 31; zero below ν = −0.78).
#[inline]
pub fn knife_edge_j(v: f64) -> f64 {
    if v <= -0.78 {
        0.0
    } else {
        6.9 + 20.0 * (((v - 0.1).powi(2) + 1.0).sqrt() + v - 0.1).log10()
    }
}

#[inline]
fn q(x: f64, quantum: f64) -> f64 {
    (x / quantum).round() * quantum
}

/// Path loss from `e` to one point: FSPL + J(ν_max).
pub fn path_loss_point(grid: &HeightGrid, e: &FieldEmitter, x: f64, y: f64, p: &PathLossParams) -> f64 {
    let lambda = SPEED_OF_LIGHT / e.freq_hz;
    let dx = x - e.x;
    let dy = y - e.y;
    let d = dx.hypot(dy);
    let zo = grid.height_at(e.x, e.y) + e.agl;
    let zt = grid.height_at(x, y) + p.rx_agl;
    let slant = (d * d + (zt - zo).powi(2)).sqrt();
    let n = steps_for(d, grid.cell, p.step_cells);
    let inv_2kr = 1.0 / (2.0 * p.k_factor * EARTH_RADIUS);
    let mut v_max = f64::NEG_INFINITY;
    for s in 1..n {
        let t = s as f64 / n as f64;
        let ground = grid.height_at(e.x + t * dx, e.y + t * dy) + t * (1.0 - t) * d * d * inv_2kr;
        let h = ground - (zo + t * (zt - zo));
        let d1 = (t * d).max(1e-3);
        let d2 = ((1.0 - t) * d).max(1e-3);
        let v = h * (2.0 / lambda * (1.0 / d1 + 1.0 / d2)).sqrt();
        v_max = v_max.max(v);
    }
    fspl_db(slant, e.freq_hz) + knife_edge_j(v_max)
}

/// The CPU twin: path loss from `e` to every node.
pub fn path_loss_field_cpu(grid: &HeightGrid, e: &FieldEmitter, p: &PathLossParams) -> DbField {
    let w = grid.width;
    let db = (0..grid.height)
        .into_par_iter()
        .flat_map_iter(|j| {
            (0..w).map(move |i| {
                let (x, y) = grid.node_pos(i, j);
                q(path_loss_point(grid, e, x, y, p), p.quantum_db)
            })
        })
        .collect();
    DbField { width: w, height: grid.height, db }
}

/// One emitter's contribution to a power field: ERP (dBW) over its loss field.
#[derive(Clone, Copy, Debug)]
pub struct PowerSource<'a> {
    pub erp_dbw: f64,
    pub loss: &'a DbField,
}

/// Received power per node (dBW), summing sources in the given order in
/// linear watts; `rx_gain_db` is the receiving antenna gain.
/// Nodes with no power read −300 dBW.
pub fn received_power_field_cpu(sources: &[PowerSource<'_>], rx_gain_db: f64, quantum_db: f64) -> Option<DbField> {
    let first = sources.first()?;
    let (w, h) = (first.loss.width, first.loss.height);
    if sources.iter().any(|s| s.loss.width != w || s.loss.height != h) {
        return None;
    }
    let db = (0..w * h)
        .into_par_iter()
        .map(|k| {
            let mut sum = 0.0;
            for s in sources {
                sum += 10f64.powf((s.erp_dbw + rx_gain_db - s.loss.db[k]) / 10.0);
            }
            if sum > 0.0 {
                q(10.0 * sum.log10(), quantum_db)
            } else {
                -300.0
            }
        })
        .collect();
    Some(DbField { width: w, height: h, db })
}

/// J/S per node: jamming power minus wanted-signal power, dB.
pub fn js_field_cpu(jamming: &DbField, signal: &DbField, quantum_db: f64) -> Option<DbField> {
    if jamming.width != signal.width || jamming.height != signal.height {
        return None;
    }
    let db = jamming.db.par_iter().zip(&signal.db).map(|(j, s)| q(j - s, quantum_db)).collect();
    Some(DbField { width: jamming.width, height: jamming.height, db })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn fspl_and_j() {
        // 1 km at 300 MHz (λ = 1 m): 20 log10(4π·1000) = 81.98 dB.
        assert_relative_eq!(fspl_db(1000.0, SPEED_OF_LIGHT), 81.984, epsilon = 1e-3);
        assert_relative_eq!(knife_edge_j(0.0), 6.03, epsilon = 0.02);
        assert_relative_eq!(knife_edge_j(-1.0), 0.0);
    }

    #[test]
    fn hill_shadow_and_sum() {
        let mut h = vec![0f32; 61 * 3];
        for j in 0..3 {
            h[j * 61 + 30] = 80.0;
        }
        let g = HeightGrid::new(61, 3, 50.0, 0.0, 0.0, h).unwrap();
        let e = FieldEmitter { x: 0.0, y: 50.0, agl: 10.0, freq_hz: 300e6 };
        let f = path_loss_field_cpu(&g, &e, &PathLossParams::default());
        let before = f.at(25, 1);
        let after = f.at(35, 1);
        assert!(after - before > 15.0, "shadowed node loses more: {before} → {after}");
        let two = received_power_field_cpu(&[PowerSource { erp_dbw: 10.0, loss: &f }, PowerSource { erp_dbw: 10.0, loss: &f }], 0.0, 0.01).unwrap();
        assert_relative_eq!(two.at(25, 1), 10.0 - before + 3.01, epsilon = 0.02);
        let js = js_field_cpu(&two, &two, 0.01).unwrap();
        assert_eq!(js.db[0], 0.0);
    }
}
