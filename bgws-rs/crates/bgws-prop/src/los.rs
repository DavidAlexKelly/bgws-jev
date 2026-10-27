//! Optical line of sight over the environment's terrain (§8.4): a ray
//! march over the DEM with Earth curvature and optical refraction (k ≈ 1.15
//! **[default]**, §5.2), then partial masking by canopy and buildings as a
//! probability.
//!
//! The terrain test runs through the kernel layer (`bgws_gpu::Kernels`), so
//! it batches on the GPU when one is selected and on the CPU twin otherwise.
//! The canopy pass is CPU-only and runs only for pairs the terrain does not
//! mask: wherever the ray passes below the canopy (or roof) height of the
//! cell beneath it, that length counts toward the class's blocking
//! probability, and `p_clear = Π (1 − P_block(class, length))`. The last few
//! metres at each end are excluded, so a unit at a wood's edge can see out.
//! A decision draws once against `p_visible` with a keyed stream (§12).

use bgws_core::geo::EARTH_RADIUS;
use bgws_core::Figures;
use bgws_env::{LandCover, LandCoverTable, Terrain};
use bgws_gpu::{HeightGrid, Kernels, LosParams, LosQuery, LosResult};
use rand::Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Convert a terrain's elevation layer for the kernel layer (one copy).
pub fn height_grid(terrain: &Terrain) -> HeightGrid {
    let s = terrain.spec;
    HeightGrid { width: s.width, height: s.height, cell: s.cell, origin_x: s.origin_e, origin_y: s.origin_n, heights: terrain.elevation.data.clone() }
}

/// Terrain for the kernel layer, with its land cover. The height grid is
/// shared, so a caller can build it once and reuse it every tick.
#[derive(Clone, Debug)]
pub struct TerrainGrid<'a> {
    pub terrain: &'a Terrain,
    pub heights: Arc<HeightGrid>,
}

impl<'a> TerrainGrid<'a> {
    /// Build the height grid from the terrain (copies the elevation once).
    pub fn new(terrain: &'a Terrain) -> Self {
        Self { terrain, heights: Arc::new(height_grid(terrain)) }
    }

    /// Reuse an already-built height grid (it must match `terrain`).
    pub fn with_heights(terrain: &'a Terrain, heights: Arc<HeightGrid>) -> Self {
        Self { terrain, heights }
    }
}

/// Optical LOS settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpticalParams {
    pub los: LosParams,
    /// Canopy within this distance of either end is ignored, metres.
    pub endpoint_clear_m: f64,
    pub cover: LandCoverTable,
}

impl OpticalParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            los: LosParams::from_figures(f, f.get("prop.optical.k_factor")),
            endpoint_clear_m: f.get("prop.optical.endpoint_clear_m"),
            cover: LandCoverTable::from_figures(f),
        }
    }
}

/// An optical LOS answer with its reasons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpticalLos {
    pub range_m: f64,
    /// Terrain (with curvature and refraction) alone.
    pub terrain_visible: bool,
    pub clearance_m: f64,
    /// Sight-line length through each class's canopy or buildings, metres.
    pub canopy_path_m: BTreeMap<LandCover, f64>,
    /// Probability the canopy does not block the line.
    pub p_clear: f64,
    /// Probability of an unobstructed line: 0 if the terrain masks.
    pub p_visible: f64,
}

impl OpticalLos {
    /// Resolve with one draw from a keyed stream.
    pub fn resolve(&self, rng: &mut impl Rng) -> bool {
        self.p_visible > 0.0 && rng.gen::<f64>() < self.p_visible
    }
}

/// Canopy pass for one query on a terrain the ray already clears.
fn canopy(t: &Terrain, q: &LosQuery, p: &OpticalParams) -> BTreeMap<LandCover, f64> {
    let (dx, dy) = (q.tgt_x - q.obs_x, q.tgt_y - q.obs_y);
    let d = dx.hypot(dy);
    let mut out = BTreeMap::new();
    if d <= 2.0 * p.endpoint_clear_m {
        return out;
    }
    let zo = t.height(q.obs_x, q.obs_y) + q.obs_agl;
    let zt = t.height(q.tgt_x, q.tgt_y) + q.tgt_agl;
    let step = (t.spec.cell * p.los.step_cells).max(0.5);
    let n = (d / step).ceil().max(1.0) as usize;
    let seg = d / n as f64;
    let inv_2kr = 1.0 / (2.0 * p.los.k_factor * EARTH_RADIUS);
    for s in 0..n {
        let tt = (s as f64 + 0.5) / n as f64;
        let along = tt * d;
        if along < p.endpoint_clear_m || d - along < p.endpoint_clear_m {
            continue;
        }
        let (x, y) = (q.obs_x + tt * dx, q.obs_y + tt * dy);
        let lc = t.land_cover_at(x, y);
        let canopy_h = p.cover.get(lc).canopy_m;
        if canopy_h <= 0.0 {
            continue;
        }
        let ground = t.height(x, y) + tt * (1.0 - tt) * d * d * inv_2kr;
        let ray = zo + tt * (zt - zo);
        if ray - ground < canopy_h {
            *out.entry(lc).or_insert(0.0) += seg;
        }
    }
    out
}

fn assemble(t: &Terrain, q: &LosQuery, r: LosResult, p: &OpticalParams) -> OpticalLos {
    let range_m = ((q.tgt_x - q.obs_x).powi(2) + (q.tgt_y - q.obs_y).powi(2)).sqrt();
    let canopy_path_m = if r.visible { canopy(t, q, p) } else { BTreeMap::new() };
    let p_clear = canopy_path_m.iter().map(|(c, l)| 1.0 - p.cover.get(*c).block_probability(*l)).product::<f64>();
    OpticalLos { range_m, terrain_visible: r.visible, clearance_m: r.clearance_m(), p_visible: if r.visible { p_clear } else { 0.0 }, p_clear, canopy_path_m }
}

/// Batched optical LOS: terrain on the kernel backend, canopy on the CPU.
pub fn optical_los_batch<K: Kernels + ?Sized>(k: &K, g: &TerrainGrid<'_>, queries: &[LosQuery], p: &OpticalParams) -> Vec<OpticalLos> {
    let terrain = k.los_batch(&g.heights, queries, &p.los);
    queries.par_iter().zip(terrain.par_iter()).map(|(q, r)| assemble(g.terrain, q, *r, p)).collect()
}

/// One optical LOS query on the CPU.
pub fn optical_los(g: &TerrainGrid<'_>, q: &LosQuery, p: &OpticalParams) -> OpticalLos {
    let r = bgws_gpu::los::los_one(&g.heights, q, &p.los);
    assemble(g.terrain, q, r, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{Geodetic, Seed, StreamKey};
    use bgws_env::{GridSpec, Raster};
    use bgws_gpu::CpuKernels;

    fn setup() -> (Terrain, OpticalParams) {
        let f = crate::default_figures();
        let spec = GridSpec::new(101, 21, 10.0, 0.0, 0.0);
        let mut t = Terrain::flat(Geodetic::new(50.0, 30.0, 0.0), spec, 100.0);
        // A 200 m belt of dense forest across x = 400..600.
        t.land_cover = Raster::from_fn(spec, |i, _| if (40..60).contains(&i) { LandCover::ForestDense } else { LandCover::Open });
        (t, OpticalParams::from_figures(&f))
    }

    #[test]
    fn forest_belt_masks_probabilistically() {
        let (t, p) = setup();
        let g = TerrainGrid::new(&t);
        let low = LosQuery { obs_x: 0.0, obs_y: 100.0, obs_agl: 2.0, tgt_x: 1000.0, tgt_y: 100.0, tgt_agl: 2.0 };
        let r = optical_los(&g, &low, &p);
        assert!(r.terrain_visible);
        let through = r.canopy_path_m.get(&LandCover::ForestDense).copied().unwrap_or(0.0);
        assert!((through - 200.0).abs() <= 10.0, "{through}");
        assert!(r.p_visible < 0.05, "200 m of dense forest nearly always blocks: {}", r.p_visible);
        // From a mast above the canopy the line is clear.
        let high = LosQuery { obs_agl: 40.0, tgt_agl: 40.0, ..low };
        assert_eq!(optical_los(&g, &high, &p).p_visible, 1.0);
        // Standing at the wood's edge looking out: own canopy ignored.
        let edge = LosQuery { obs_x: 585.0, tgt_x: 1000.0, ..low };
        assert!(optical_los(&g, &edge, &p).p_visible > 0.9);
        // Batched through the CPU kernels matches the single query.
        let b = optical_los_batch(&CpuKernels, &g, &[low, high, edge], &p);
        let shared = TerrainGrid::with_heights(&t, g.heights.clone());
        assert_eq!(optical_los(&shared, &low, &p), r);
        assert_eq!(b[0], r);
        // The keyed draw is reproducible.
        let s = Seed(9);
        let k = StreamKey::new("los", 1, 2, 3);
        assert_eq!(b[0].resolve(&mut s.stream(k)), b[0].resolve(&mut s.stream(k)));
    }

    #[test]
    fn refraction_extends_optical_reach() {
        let f = crate::default_figures();
        let spec = GridSpec::new(501, 2, 100.0, 0.0, 0.0);
        let t = Terrain::flat(Geodetic::default(), spec, 0.0);
        let g = TerrainGrid::new(&t);
        let p = OpticalParams::from_figures(&f);
        // Geometric horizon pair for 20 m heights: 2·√(2R·20) ≈ 31.9 km; optical k = 1.15 → ≈ 34.2 km.
        let q = |d: f64| LosQuery { obs_x: 0.0, obs_y: 0.0, obs_agl: 20.0, tgt_x: d, tgt_y: 0.0, tgt_agl: 20.0 };
        assert!(optical_los(&g, &q(33_000.0), &p).terrain_visible);
        assert!(!optical_los(&g, &q(35_500.0), &p).terrain_visible);
    }
}
