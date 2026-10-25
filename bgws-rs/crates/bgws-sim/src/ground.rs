//! The ground the land fight happens on (§7.1): the environment's terrain
//! (generated from the scenario's seed when there is no theatre pack), its
//! land-cover table, trafficability, micro-terrain cover, and optical line of
//! sight through `bgws-prop` on the kernel layer (GPU when present, else the
//! CPU twin, §11).

use bgws_core::rng::fnv1a;
use bgws_core::{Enu, Figures, Geodetic};
use bgws_env::soil::{trafficability, TrafficParams, VehicleClass as EnvVehicle};
use bgws_env::synth::{generate, SynthParams};
use bgws_env::{LandCover, LandCoverTable, Terrain};
use bgws_gpu::{Backend, HeightGrid, LosQuery};
use bgws_prop::los::{optical_los, optical_los_batch, OpticalLos, OpticalParams, TerrainGrid};
use bgws_env::{cost_astar, GridSpec, Raster};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub struct Ground {
    pub terrain: Terrain,
    pub heights: Arc<HeightGrid>,
    pub cover: LandCoverTable,
    pub traffic: TrafficParams,
    pub optical: OpticalParams,
    pub micro_seed: u64,
    pub backend: Backend,
    /// Route-planning cost rasters by vehicle class, built on first use.
    costs: Mutex<BTreeMap<u8, Arc<Raster<f64>>>>,
}

impl std::fmt::Debug for Ground {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ground").field("spec", &self.terrain.spec).finish()
    }
}

impl Ground {
    /// Generated ground covering `half_extent_m` around the theatre origin.
    pub fn generated(origin: Geodetic, seed_text: &str, half_extent_m: f64, figures: &Figures) -> Ground {
        let cell = (2.0 * half_extent_m / 1024.0).max(30.0);
        let n = ((2.0 * half_extent_m / cell).ceil() as usize + 1).max(16);
        let seed = fnv1a(seed_text.as_bytes());
        let mut p = SynthParams::new(seed, n, n, cell, figures);
        p.frame_origin = origin;
        let mut terrain = generate(&p, figures).terrain;
        // Centre the grid on the frame origin.
        let shift = cell * (n as f64 - 1.0) / 2.0;
        terrain.spec.origin_e -= shift;
        terrain.spec.origin_n -= shift;
        for r in [&mut terrain.elevation.spec, &mut terrain.land_cover.spec, &mut terrain.soil.spec, &mut terrain.wetness.spec, &mut terrain.obstacle_gap.spec] {
            r.origin_e = terrain.spec.origin_e;
            r.origin_n = terrain.spec.origin_n;
        }
        Self::from_terrain(terrain, seed, figures)
    }

    /// Flat open ground (tests, calibration vignettes).
    pub fn flat(origin: Geodetic, half_extent_m: f64, figures: &Figures) -> Ground {
        let cell = (2.0 * half_extent_m / 512.0).max(30.0);
        let n = (2.0 * half_extent_m / cell).ceil() as usize + 1;
        let spec = bgws_env::raster::GridSpec::new(n, n, cell, -half_extent_m, -half_extent_m);
        Self::from_terrain(Terrain::flat(origin, spec, 0.0), 0, figures)
    }

    pub fn from_terrain(terrain: Terrain, micro_seed: u64, figures: &Figures) -> Ground {
        let s = terrain.spec;
        let heights = Arc::new(HeightGrid { width: s.width, height: s.height, cell: s.cell, origin_x: s.origin_e, origin_y: s.origin_n, heights: terrain.elevation.data.clone() });
        Ground {
            terrain,
            heights,
            cover: LandCoverTable::from_figures(figures),
            traffic: TrafficParams::from_figures(figures),
            optical: OpticalParams::from_figures(figures),
            micro_seed,
            backend: Backend::cpu(),
            costs: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn height(&self, e: f64, n: f64) -> f64 {
        self.terrain.height(e, n)
    }

    pub fn land_cover(&self, e: f64, n: f64) -> LandCover {
        self.terrain.land_cover_at(e, n)
    }

    /// Cover and concealment where a unit stands.
    pub fn micro(&self, e: f64, n: f64) -> bgws_env::landcover::MicroTerrain {
        let s = self.terrain.spec;
        let i = ((e - s.origin_e) / s.cell).round().max(0.0) as u64;
        let j = ((n - s.origin_n) / s.cell).round().max(0.0) as u64;
        self.cover.micro_terrain(self.land_cover(e, n), self.micro_seed, j * s.width as u64 + i)
    }

    /// Off-road speed factor (0–1) and whether the class is stopped.
    pub fn going(&self, e: f64, n: f64, class: EnvVehicle) -> bgws_env::soil::Trafficability {
        trafficability(&self.terrain.going(e, n), class, &self.traffic)
    }

    fn grid(&self) -> TerrainGrid<'_> {
        TerrainGrid::with_heights(&self.terrain, self.heights.clone())
    }

    /// Line of sight between two points with heights above ground.
    pub fn los(&self, a: Enu, a_agl: f64, b: Enu, b_agl: f64) -> OpticalLos {
        let q = LosQuery { obs_x: a.e, obs_y: a.n, obs_agl: a_agl, tgt_x: b.e, tgt_y: b.n, tgt_agl: b_agl };
        optical_los(&self.grid(), &q, &self.optical)
    }

    /// Many at once, through the kernel layer.
    pub fn los_batch(&self, queries: &[LosQuery]) -> Vec<OpticalLos> {
        optical_los_batch(&self.backend, &self.grid(), queries, &self.optical)
    }

    /// Seconds per metre over a coarse grid for one vehicle class.
    fn cost_raster(&self, class: EnvVehicle) -> Arc<Raster<f64>> {
        let key = class as u8;
        if let Some(r) = self.costs.lock().expect("cost cache").get(&key) {
            return r.clone();
        }
        let t = self.terrain.spec;
        let factor = (90.0 / t.cell).round().max(1.0) as usize;
        let spec = GridSpec::new(t.width.div_ceil(factor), t.height.div_ceil(factor), t.cell * factor as f64, t.origin_e, t.origin_n);
        let r = Arc::new(Raster::from_fn(spec, |i, j| {
            let (e, n) = spec.pos(i, j);
            let v = self.going(e, n, class).speed_kmh / 3.6;
            if v < 0.3 {
                f64::INFINITY
            } else {
                1.0 / v
            }
        }));
        self.costs.lock().expect("cost cache").insert(key, r.clone());
        r
    }

    /// A route from `a` to `b` that avoids bad going (§8.1: the cost raster
    /// for tactical movement), as waypoints every few hundred metres.
    pub fn route(&self, a: Enu, b: Enu, class: EnvVehicle) -> Vec<Enu> {
        let straight = vec![b];
        if a.sub(b).horizontal() < 400.0 {
            return straight;
        }
        let cost = self.cost_raster(class);
        let s = cost.spec;
        let (Some(start), Some(goal)) = (s.nearest(a.e, a.n), s.nearest(b.e, b.n)) else { return straight };
        let Some(path) = cost_astar(&cost, None, s.idx(start.0, start.1), s.idx(goal.0, goal.1)) else { return straight };
        let mut out: Vec<Enu> = path
            .cells
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 4 == 3)
            .map(|(_, &k)| {
                let (i, j) = s.ij(k);
                let (e, n) = s.pos(i, j);
                Enu::new(e, n, 0.0)
            })
            .collect();
        out.push(b);
        out
    }

    /// Does `p` lie on the grid?
    pub fn contains(&self, p: Enu) -> bool {
        let s = self.terrain.spec;
        p.e >= s.origin_e && p.n >= s.origin_n && p.e <= s.origin_e + s.cell * (s.width as f64 - 1.0) && p.n <= s.origin_n + s.cell * (s.height as f64 - 1.0)
    }
}
