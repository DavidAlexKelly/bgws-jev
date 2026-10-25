//! Terrain (§7.1): elevation, land cover, soil, wetness and obstacle layers
//! on one georeferenced grid in a theatre's local ENU frame, with bilinear
//! height, slope and aspect, and a multi-resolution pyramid (30, 90, 270,
//! 810 m by default) so far-away and aggregate queries read coarse tiles.

use crate::landcover::LandCover;
use crate::raster::{GridSpec, Raster};
use crate::soil::{CellGoing, SoilType};
use bgws_core::{EnuFrame, Geodetic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One terrain grid. Layers share `spec`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terrain {
    /// Origin of the theatre's ENU frame (not of this grid).
    pub frame_origin: Geodetic,
    pub spec: GridSpec,
    /// Metres above the datum.
    pub elevation: Raster<f32>,
    pub land_cover: Raster<LandCover>,
    pub soil: Raster<SoilType>,
    /// 0 dry … 1 saturated.
    pub wetness: Raster<f32>,
    /// Mean clear gap between obstacles, metres; 0 = no obstacles.
    pub obstacle_gap: Raster<f32>,
}

impl Terrain {
    /// A flat, open, dry terrain.
    pub fn flat(frame_origin: Geodetic, spec: GridSpec, h: f32) -> Self {
        Self {
            frame_origin,
            spec,
            elevation: Raster::filled(spec, h),
            land_cover: Raster::filled(spec, LandCover::Open),
            soil: Raster::filled(spec, SoilType::Loam),
            wetness: Raster::filled(spec, 0.2),
            obstacle_gap: Raster::filled(spec, 0.0),
        }
    }

    pub fn frame(&self) -> EnuFrame {
        EnuFrame::new(self.frame_origin)
    }

    /// Bilinear ground height at a point (clamped at the edges).
    pub fn height(&self, e: f64, n: f64) -> f64 {
        self.elevation.bilinear(e, n)
    }

    /// Ground height under a geodetic position.
    pub fn height_geodetic(&self, g: Geodetic) -> f64 {
        let p = self.frame().geodetic_to_enu(Geodetic { h: 0.0, ..g });
        self.height(p.e, p.n)
    }

    /// Gradient (dh/de, dh/dn) by central differences over one cell.
    pub fn gradient(&self, e: f64, n: f64) -> (f64, f64) {
        let c = self.spec.cell;
        let gx = (self.height(e + c, n) - self.height(e - c, n)) / (2.0 * c);
        let gy = (self.height(e, n + c) - self.height(e, n - c)) / (2.0 * c);
        (gx, gy)
    }

    /// Slope as rise over run.
    pub fn slope(&self, e: f64, n: f64) -> f64 {
        let (gx, gy) = self.gradient(e, n);
        gx.hypot(gy)
    }

    /// Slope angle, radians.
    pub fn slope_angle(&self, e: f64, n: f64) -> f64 {
        self.slope(e, n).atan()
    }

    /// Aspect: the downhill direction, radians clockwise from north.
    pub fn aspect(&self, e: f64, n: f64) -> f64 {
        let (gx, gy) = self.gradient(e, n);
        (-gx).atan2(-gy).rem_euclid(std::f64::consts::TAU)
    }

    /// Slope in a given direction of travel (positive uphill).
    pub fn directional_slope(&self, e: f64, n: f64, bearing: f64) -> f64 {
        let (gx, gy) = self.gradient(e, n);
        gx * bearing.sin() + gy * bearing.cos()
    }

    pub fn land_cover_at(&self, e: f64, n: f64) -> LandCover {
        self.land_cover.nearest(e, n)
    }

    /// The going at a point, ready for `soil::trafficability`.
    pub fn going(&self, e: f64, n: f64) -> CellGoing {
        let (i, j) = self.spec.nearest_clamped(e, n);
        let gap = self.obstacle_gap.get(i, j);
        CellGoing {
            soil: self.soil.get(i, j),
            wetness: self.wetness.get(i, j) as f64,
            slope: self.slope(e, n),
            land_cover: self.land_cover.get(i, j),
            obstacle_spacing_m: if gap > 0.0 { Some(gap as f64) } else { None },
        }
    }

    /// A coarser copy: every `factor`-th node, elevation and wetness
    /// box-averaged around it, categorical layers by majority (ties to the
    /// lower class code, so the result is deterministic).
    pub fn downsample(&self, factor: usize) -> Terrain {
        let f = factor.max(1);
        let w = (self.spec.width - 1) / f + 1;
        let h = (self.spec.height - 1) / f + 1;
        let spec = GridSpec::new(w, h, self.spec.cell * f as f64, self.spec.origin_e, self.spec.origin_n);
        let half = (f / 2) as i64;
        let block = |i: usize, j: usize| {
            let (ci, cj) = ((i * f) as i64, (j * f) as i64);
            let mut cells = Vec::with_capacity(f * f);
            for dj in -half..=half {
                for di in -half..=half {
                    let (x, y) = (ci + di, cj + dj);
                    if x >= 0 && y >= 0 && (x as usize) < self.spec.width && (y as usize) < self.spec.height {
                        cells.push((x as usize, y as usize));
                    }
                }
            }
            cells
        };
        let mean = |r: &Raster<f32>, i, j| {
            let c = block(i, j);
            (c.iter().map(|&(x, y)| r.get(x, y) as f64).sum::<f64>() / c.len() as f64) as f32
        };
        fn mode<T: Copy + Ord>(vals: impl Iterator<Item = T>) -> T {
            let mut m: BTreeMap<T, usize> = BTreeMap::new();
            let mut first = None;
            for v in vals {
                first.get_or_insert(v);
                *m.entry(v).or_default() += 1;
            }
            let best = m.values().copied().max().unwrap_or(0);
            m.into_iter().find(|(_, c)| *c == best).map(|(k, _)| k).or(first).expect("non-empty block")
        }
        Terrain {
            frame_origin: self.frame_origin,
            spec,
            elevation: Raster::from_fn(spec, |i, j| mean(&self.elevation, i, j)),
            wetness: Raster::from_fn(spec, |i, j| mean(&self.wetness, i, j)),
            obstacle_gap: Raster::from_fn(spec, |i, j| self.obstacle_gap.get(i * f, j * f)),
            land_cover: Raster::from_fn(spec, |i, j| mode(block(i, j).into_iter().map(|(x, y)| self.land_cover.get(x, y)))),
            soil: Raster::from_fn(spec, |i, j| mode(block(i, j).into_iter().map(|(x, y)| self.soil.get(x, y)))),
        }
    }

    /// A rectangular sub-grid of nodes `[i0, i0+w) × [j0, j0+h)`.
    pub fn crop(&self, i0: usize, j0: usize, w: usize, h: usize) -> Terrain {
        let w = w.min(self.spec.width - i0);
        let h = h.min(self.spec.height - j0);
        let (e0, n0) = self.spec.pos(i0, j0);
        let spec = GridSpec::new(w, h, self.spec.cell, e0, n0);
        Terrain {
            frame_origin: self.frame_origin,
            spec,
            elevation: Raster::from_fn(spec, |i, j| self.elevation.get(i0 + i, j0 + j)),
            land_cover: Raster::from_fn(spec, |i, j| self.land_cover.get(i0 + i, j0 + j)),
            soil: Raster::from_fn(spec, |i, j| self.soil.get(i0 + i, j0 + j)),
            wetness: Raster::from_fn(spec, |i, j| self.wetness.get(i0 + i, j0 + j)),
            obstacle_gap: Raster::from_fn(spec, |i, j| self.obstacle_gap.get(i0 + i, j0 + j)),
        }
    }

    /// Heights as a flat f32 array (row-major), for the kernel layer.
    pub fn heights_f32(&self) -> &[f32] {
        &self.elevation.data
    }
}

/// A resolution pyramid: level 0 is the finest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pyramid {
    pub levels: Vec<Terrain>,
}

impl Pyramid {
    /// Build `n_levels` levels, each `factor` times coarser (3 gives 30/90/270/810 m from 30 m).
    pub fn build(base: Terrain, n_levels: usize, factor: usize) -> Self {
        let mut levels = vec![base];
        while levels.len() < n_levels.max(1) {
            let next = levels.last().expect("non-empty").downsample(factor);
            if next.spec.width < 2 || next.spec.height < 2 {
                break;
            }
            levels.push(next);
        }
        Self { levels }
    }

    /// The coarsest level whose spacing is no coarser than `resolution_m`
    /// (the finest level if none qualifies).
    pub fn level_for(&self, resolution_m: f64) -> &Terrain {
        self.levels.iter().rev().find(|t| t.spec.cell <= resolution_m + 1e-9).unwrap_or(&self.levels[0])
    }

    pub fn cells(&self) -> Vec<f64> {
        self.levels.iter().map(|t| t.spec.cell).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn plane() -> Terrain {
        let spec = GridSpec::new(82, 82, 30.0, 0.0, 0.0);
        let mut t = Terrain::flat(Geodetic::new(48.0, 37.0, 0.0), spec, 0.0);
        // h = 0.1·e: a 10% slope facing west (downhill to the west).
        t.elevation = Raster::from_fn(spec, |i, _| (0.1 * i as f64 * 30.0) as f32);
        t
    }

    #[test]
    fn height_slope_aspect() {
        let t = plane();
        assert_relative_eq!(t.height(45.0, 100.0), 4.5, epsilon = 1e-5);
        assert_relative_eq!(t.slope(600.0, 600.0), 0.1, epsilon = 1e-5);
        assert_relative_eq!(t.aspect(600.0, 600.0), 1.5 * std::f64::consts::PI, epsilon = 1e-6);
        assert_relative_eq!(t.directional_slope(600.0, 600.0, std::f64::consts::FRAC_PI_2), 0.1, epsilon = 1e-5);
    }

    #[test]
    fn pyramid_levels() {
        let mut t = plane();
        t.land_cover.set(3, 3, LandCover::Water);
        let p = Pyramid::build(t, 4, 3);
        assert_eq!(p.cells(), vec![30.0, 90.0, 270.0, 810.0]);
        assert_eq!(p.level_for(300.0).spec.cell, 270.0);
        assert_eq!(p.level_for(10.0).spec.cell, 30.0);
        // A linear surface survives averaging.
        assert_relative_eq!(p.levels[2].height(1200.0, 1200.0), 120.0, epsilon = 1e-3);
        assert_eq!(p.levels[1].land_cover.get(1, 1), LandCover::Open);
    }
}
