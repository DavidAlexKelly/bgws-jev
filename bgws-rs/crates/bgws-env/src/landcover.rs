//! Land cover and micro-terrain (§7.1). ESA WorldCover classes collapse to
//! ten engine classes. Each class carries cover (protection from fire) and
//! concealment (protection from observation) as distributions — the mean
//! and spread of the fraction a unit in that cell gets from hedgerows,
//! ditches, rubble and the like — plus a canopy height and the probability
//! that 100 m of sight line through the class's canopy or buildings is
//! blocked. Concealment-first infantry behaviour [R4.1] reads these.
//!
//! All per-class numbers are figures (`env.landcover.<class>.*`), all
//! **[default]** until calibrated.

use bgws_core::rng::mix64;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// The engine's land-cover classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum LandCover {
    #[default]
    Open = 0,
    Crops = 1,
    Scrub = 2,
    ForestLight = 3,
    ForestDense = 4,
    UrbanLight = 5,
    UrbanDense = 6,
    Water = 7,
    Wetland = 8,
    Bare = 9,
}

impl LandCover {
    pub const ALL: [LandCover; 10] = [
        LandCover::Open,
        LandCover::Crops,
        LandCover::Scrub,
        LandCover::ForestLight,
        LandCover::ForestDense,
        LandCover::UrbanLight,
        LandCover::UrbanDense,
        LandCover::Water,
        LandCover::Wetland,
        LandCover::Bare,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }

    /// Lossy decode for raster bytes: unknown values read as `Open`.
    pub fn from_u8_or_open(v: u8) -> Self {
        Self::from_u8(v).unwrap_or(LandCover::Open)
    }

    pub fn key(self) -> &'static str {
        match self {
            LandCover::Open => "open",
            LandCover::Crops => "crops",
            LandCover::Scrub => "scrub",
            LandCover::ForestLight => "forest_light",
            LandCover::ForestDense => "forest_dense",
            LandCover::UrbanLight => "urban_light",
            LandCover::UrbanDense => "urban_dense",
            LandCover::Water => "water",
            LandCover::Wetland => "wetland",
            LandCover::Bare => "bare",
        }
    }

    pub fn is_forest(self) -> bool {
        matches!(self, LandCover::ForestLight | LandCover::ForestDense)
    }
    pub fn is_urban(self) -> bool {
        matches!(self, LandCover::UrbanLight | LandCover::UrbanDense)
    }

    /// ESA WorldCover v200 class code → engine class. Urban density is not
    /// in WorldCover; built-up maps to light urban and a later pass upgrades
    /// dense blocks from OSM. Tree cover maps to light forest likewise.
    pub fn from_worldcover(code: u8) -> LandCover {
        match code {
            10 => LandCover::ForestLight,
            20 => LandCover::Scrub,
            30 => LandCover::Open,
            40 => LandCover::Crops,
            50 => LandCover::UrbanLight,
            60 => LandCover::Bare,
            70 => LandCover::Bare, // snow and ice
            80 => LandCover::Water,
            90 | 95 => LandCover::Wetland,
            100 => LandCover::Open, // moss and lichen
            _ => LandCover::Open,
        }
    }
}

/// Per-class properties, read from figures.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverProps {
    /// Mean fraction of a unit's exposure removed by cover from fire.
    pub cover_mean: f64,
    pub cover_sd: f64,
    /// Mean concealment from observation (0 = fully exposed, 1 = hidden).
    pub conceal_mean: f64,
    pub conceal_sd: f64,
    /// Canopy or roof height, metres.
    pub canopy_m: f64,
    /// Probability that 100 m of sight line through the canopy is blocked.
    pub los_block_per_100m: f64,
}

impl CoverProps {
    /// Probability that `path_m` of sight line through this class is blocked.
    pub fn block_probability(&self, path_m: f64) -> f64 {
        if path_m <= 0.0 {
            return 0.0;
        }
        1.0 - (1.0 - self.los_block_per_100m.clamp(0.0, 1.0)).powf(path_m / 100.0)
    }
}

/// The table of all classes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LandCoverTable {
    pub props: [CoverProps; 10],
}

impl LandCoverTable {
    pub fn from_figures(f: &Figures) -> Self {
        let mut props = [CoverProps { cover_mean: 0.0, cover_sd: 0.0, conceal_mean: 0.0, conceal_sd: 0.0, canopy_m: 0.0, los_block_per_100m: 0.0 }; 10];
        for c in LandCover::ALL {
            let k = |s: &str| f.get(&format!("env.landcover.{}.{s}", c.key()));
            props[c as usize] = CoverProps {
                cover_mean: k("cover_mean"),
                cover_sd: k("cover_sd"),
                conceal_mean: k("conceal_mean"),
                conceal_sd: k("conceal_sd"),
                canopy_m: k("canopy_m"),
                los_block_per_100m: k("los_block_per_100m"),
            };
        }
        Self { props }
    }

    pub fn get(&self, c: LandCover) -> &CoverProps {
        &self.props[c as usize]
    }

    /// The micro-terrain of one cell: a deterministic draw from the class
    /// distributions keyed by (seed, cell index), so the same hedgerow is
    /// there every time anyone looks.
    pub fn micro_terrain(&self, c: LandCover, seed: u64, cell: u64) -> MicroTerrain {
        let p = self.get(c);
        let z1 = hash_normal(seed, cell, 0);
        let z2 = hash_normal(seed, cell, 1);
        MicroTerrain {
            cover: (p.cover_mean + p.cover_sd * z1).clamp(0.0, 1.0),
            concealment: (p.conceal_mean + p.conceal_sd * z2).clamp(0.0, 1.0),
        }
    }
}

/// Cover and concealment available in one cell.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MicroTerrain {
    pub cover: f64,
    pub concealment: f64,
}

fn hash_unit(seed: u64, cell: u64, dim: u64) -> f64 {
    let h = mix64(seed ^ mix64(cell.wrapping_mul(4).wrapping_add(dim)));
    ((h >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
}

/// A standard normal from a hash (Box–Muller, libm).
fn hash_normal(seed: u64, cell: u64, dim: u64) -> f64 {
    let u1 = hash_unit(seed, cell, 2 * dim);
    let u2 = hash_unit(seed, cell, 2 * dim + 1);
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(std::f64::consts::TAU * u2)
}

/// Defaults: (cover mean, sd, conceal mean, sd, canopy m, LOS block per 100 m).
const DEFAULTS: [(LandCover, [f64; 6]); 10] = [
    (LandCover::Open, [0.10, 0.08, 0.10, 0.08, 0.0, 0.00]),
    (LandCover::Crops, [0.10, 0.08, 0.35, 0.15, 1.5, 0.05]),
    (LandCover::Scrub, [0.20, 0.10, 0.50, 0.15, 2.5, 0.25]),
    (LandCover::ForestLight, [0.40, 0.15, 0.65, 0.15, 15.0, 0.55]),
    (LandCover::ForestDense, [0.55, 0.15, 0.85, 0.10, 22.0, 0.90]),
    (LandCover::UrbanLight, [0.55, 0.15, 0.60, 0.15, 8.0, 0.70]),
    (LandCover::UrbanDense, [0.75, 0.10, 0.80, 0.10, 18.0, 0.95]),
    (LandCover::Water, [0.0, 0.0, 0.0, 0.0, 0.0, 0.00]),
    (LandCover::Wetland, [0.10, 0.08, 0.35, 0.15, 1.5, 0.10]),
    (LandCover::Bare, [0.05, 0.05, 0.05, 0.05, 0.0, 0.00]),
];

pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    for (c, d) in DEFAULTS {
        let k = c.key();
        v.push(figure!(format!("env.landcover.{k}.cover_mean"), d[0], "fraction", 0.0, 1.0, "R4.1", Default, "mean cover from fire in this class"));
        v.push(figure!(format!("env.landcover.{k}.cover_sd"), d[1], "fraction", 0.0, 0.5, "default", Default, "cell-to-cell spread of cover"));
        v.push(figure!(format!("env.landcover.{k}.conceal_mean"), d[2], "fraction", 0.0, 1.0, "R4.1", Default, "mean concealment from observation"));
        v.push(figure!(format!("env.landcover.{k}.conceal_sd"), d[3], "fraction", 0.0, 0.5, "default", Default, "cell-to-cell spread of concealment"));
        v.push(figure!(format!("env.landcover.{k}.canopy_m"), d[4], "m", 0.0, 60.0, "default", Default, "canopy or roof height"));
        v.push(figure!(format!("env.landcover.{k}.los_block_per_100m"), d[5], "fraction", 0.0, 1.0, "default", Default,
            "probability 100 m of sight line through the canopy/buildings is blocked"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_and_micro_terrain() {
        let f = Figures::from_list(figures()).unwrap();
        let t = LandCoverTable::from_figures(&f);
        assert!(t.get(LandCover::ForestDense).block_probability(100.0) > t.get(LandCover::ForestLight).block_probability(100.0));
        assert!((t.get(LandCover::ForestLight).block_probability(200.0) - (1.0 - 0.45f64.powi(2))).abs() < 1e-12);
        let a = t.micro_terrain(LandCover::ForestDense, 1, 42);
        assert_eq!(a, t.micro_terrain(LandCover::ForestDense, 1, 42));
        // The mean over many cells approaches the class mean.
        let m: f64 = (0..4000).map(|k| t.micro_terrain(LandCover::Scrub, 9, k).concealment).sum::<f64>() / 4000.0;
        assert!((m - 0.5).abs() < 0.02, "{m}");
        for c in LandCover::ALL {
            assert_eq!(LandCover::from_u8(c as u8), Some(c));
        }
        assert_eq!(LandCover::from_worldcover(80), LandCover::Water);
    }
}
