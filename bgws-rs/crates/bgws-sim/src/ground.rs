//! The ground the land fight happens on (§7.1): the environment's terrain
//! (generated from the scenario's seed when there is no theatre pack), its
//! land-cover table, trafficability, micro-terrain cover, and optical line of
//! sight through `bgws-prop` on the kernel layer (GPU when present, else the
//! CPU twin, §11). A theatre pack's roads are drawn onto a layer at the
//! terrain's resolution: on a road a unit moves at the road's speed, and
//! route planning prefers roads where they are quicker.

use bgws_core::rng::fnv1a;
use bgws_core::{Enu, Figures, Geodetic};
use bgws_env::soil::{trafficability, TrafficParams, VehicleClass as EnvVehicle};
use bgws_env::synth::{generate, SynthParams};
use bgws_env::{LandCover, LandCoverTable, RoadClass, RoadLine, RoadSpeeds, Terrain};
use bgws_gpu::{Backend, HeightGrid, LosQuery};
use bgws_prop::los::{optical_los, optical_los_batch, OpticalLos, OpticalParams, TerrainGrid};
use bgws_env::Raster;
use std::sync::Arc;

pub struct Ground {
    pub terrain: Terrain,
    pub heights: Arc<HeightGrid>,
    pub cover: LandCoverTable,
    pub traffic: TrafficParams,
    pub optical: OpticalParams,
    pub micro_seed: u64,
    pub backend: Backend,
    /// The best road class in each cell (class + 1; 0 none), when there are roads.
    pub roads: Option<Arc<Raster<u8>>>,
    /// The road lines the layer was drawn from, for the viewer.
    pub road_lines: Option<Arc<Vec<RoadLine>>>,
    pub road_speeds: RoadSpeeds,
    /// The route planner's weights.
    pub route_params: crate::routing::RouteParams,
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
            roads: None,
            road_lines: None,
            road_speeds: RoadSpeeds::from_figures(figures),
            route_params: crate::routing::RouteParams::from_figures(figures),
        }
    }

    /// The ground with roads drawn onto it (lines in this ground's ENU frame).
    pub fn with_roads(mut self, lines: &[RoadLine]) -> Ground {
        let spec = self.terrain.spec;
        let mut r = Raster::filled(spec, 0u8);
        for l in lines {
            let v = l.class as u8 + 1;
            for seg in l.line.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let steps = ((b.0 - a.0).hypot(b.1 - a.1) / (spec.cell * 0.5)).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    if let Some((i, j)) = spec.nearest(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t) {
                        let c = &mut r.data[spec.idx(i, j)];
                        // Lower classes are faster roads: keep the best in the cell.
                        if *c == 0 || v < *c {
                            *c = v;
                        }
                    }
                }
            }
        }
        self.roads = Some(Arc::new(r));
        self.road_lines = Some(Arc::new(lines.to_vec()));
        self
    }

    /// The road under a point, if any.
    pub fn road_at(&self, e: f64, n: f64) -> Option<RoadClass> {
        let r = self.roads.as_ref()?;
        let (i, j) = r.spec.nearest(e, n)?;
        let v = r.data[r.spec.idx(i, j)];
        (v > 0).then(|| RoadClass::ALL[(v - 1) as usize])
    }

    /// Speed on the road under a point for a vehicle class, km/h, if there is one.
    pub fn road_kmh(&self, e: f64, n: f64, class: EnvVehicle) -> Option<f64> {
        self.road_at(e, n).map(|rc| self.road_speeds.speed(class, rc))
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

    /// What an observer at `at` (eye `agl` m) sees of a man-height target
    /// over the terrain within `range` m, on a grid of at most `max_cells`
    /// a side (§16 viewshed layer; canopy is per line, in `los`). Returns
    /// the grid's south-west corner, cell size and row-major seen flags.
    pub fn viewshed(&self, at: Enu, agl: f64, range: f64, max_cells: usize) -> Option<(Enu, f64, usize, Vec<bool>)> {
        let cell = (2.0 * range / max_cells as f64).max(self.terrain.spec.cell);
        let n = ((2.0 * range / cell).ceil() as usize + 1).max(2);
        let sw = Enu::new(at.e - range, at.n - range, 0.0);
        let heights: Vec<f32> = (0..n * n).map(|k| self.height(sw.e + (k % n) as f64 * cell, sw.n + (k / n) as f64 * cell) as f32).collect();
        let grid = bgws_gpu::grid::HeightGrid::new(n, n, cell, sw.e, sw.n, heights).ok()?;
        let o = bgws_gpu::viewshed::ViewshedObserver { x: at.e, y: at.n, agl, max_range: range };
        let v = bgws_gpu::viewshed::viewshed_cpu(&grid, &[o], 1.5, &self.optical.los);
        Some((sw, cell, n, v.count.iter().map(|c| *c > 0).collect()))
    }

    /// Many at once, through the kernel layer.
    pub fn los_batch(&self, queries: &[LosQuery]) -> Vec<OpticalLos> {
        optical_los_batch(&self.backend, &self.grid(), queries, &self.optical)
    }

    /// A tactical route at a vehicle class's typical speeds, with no threats
    /// known (tests and tools; units plan through `decide::route_for`).
    pub fn route(&self, a: Enu, b: Enu, class: EnvVehicle) -> Vec<Enu> {
        let (road_kmh, cross_kmh) = match class {
            EnvVehicle::Tracked => (60.0, 40.0),
            EnvVehicle::Wheeled => (90.0, 40.0),
            EnvVehicle::Foot => (5.0, 4.0),
        };
        let r = crate::routing::RouteRequest { from: a, to: b, class, mode: crate::components::MoveModeS::Tactical, road_kmh, cross_kmh, threats: &[] };
        self.plan(&r, &self.route_params).waypoints
    }

    /// Does `p` lie on the grid?
    pub fn contains(&self, p: Enu) -> bool {
        let s = self.terrain.spec;
        p.e >= s.origin_e && p.n >= s.origin_n && p.e <= s.origin_e + s.cell * (s.width as f64 - 1.0) && p.n <= s.origin_n + s.cell * (s.height as f64 - 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_env::GridSpec;

    #[test]
    fn routes_through_woods_take_the_road_round() {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let mut t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        t.land_cover.data.iter_mut().for_each(|c| *c = LandCover::ForestDense);
        // An L of road: east along n = −3000, then north along e = 4000.
        let road = RoadLine { class: RoadClass::Primary, bridge: false, line: vec![(-4_400.0, -3_000.0), (4_000.0, -3_000.0), (4_000.0, 4_400.0)] };
        let woods = Ground::from_terrain(t.clone(), 1, &f);
        let roads = Ground::from_terrain(t, 1, &f).with_roads(std::slice::from_ref(&road));
        assert_eq!(roads.road_at(0.0, -3_000.0), Some(RoadClass::Primary));
        assert_eq!(roads.road_at(0.0, 0.0), None);
        let tracked = EnvVehicle::Tracked;
        assert!(roads.road_kmh(0.0, -3_000.0, tracked).unwrap() > roads.going(0.0, 0.0, tracked).speed_kmh);
        let (a, b) = (Enu::new(-4_000.0, -3_000.0, 0.0), Enu::new(4_000.0, 3_800.0, 0.0));
        let near_road = |p: &Enu| (p.n + 3_000.0).abs() < 100.0 || (p.e - 4_000.0).abs() < 100.0;
        let on = |r: &[Enu]| r.iter().filter(|p| near_road(p)).count() as f64 / r.len() as f64;
        let with = roads.route(a, b, tracked);
        let without = woods.route(a, b, tracked);
        assert!(on(&with) > 0.8, "the route keeps to the road: {:.2}", on(&with));
        assert!(on(&without) < 0.5, "without roads it cuts through the woods: {:.2}", on(&without));
    }
}
