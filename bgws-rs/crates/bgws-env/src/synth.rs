//! A deterministic synthetic theatre (§7.1, §14.2): hills and valleys from
//! seeded fractal value noise, rivers traced downhill and carved into their
//! valleys, forests, towns with urban cores and farmland, a road network
//! joining the towns (with bridges where it crosses rivers), soil and a
//! power line — so scenarios and tests have terrain without downloads.
//!
//! Everything derives from a counter-based hash of the seed (no RNG state),
//! so the same parameters always give the same world, bit for bit.

use crate::landcover::LandCover;
use crate::network::{cost_astar, Bridge, PowerLine, RoadClass, RoadNetwork, River};
use crate::raster::{GridSpec, Raster};
use crate::soil::SoilType;
use crate::terrain::Terrain;
use bgws_core::rng::mix64;
use bgws_core::{figure, Figure, Figures, Geodetic};
use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

/// Generator settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SynthParams {
    pub seed: u64,
    pub width: usize,
    pub height: usize,
    pub cell: f64,
    pub frame_origin: Geodetic,
    pub base_elevation: f64,
    /// Peak-to-trough scale of the hills, metres.
    pub relief: f64,
    /// Wavelength of the largest hills, metres.
    pub feature_scale_m: f64,
    pub octaves: u32,
    /// Amplitude ratio between octaves.
    pub persistence: f64,
    pub rivers: usize,
    pub towns: usize,
    /// Target forest share of the land.
    pub forest_fraction: f64,
}

impl SynthParams {
    /// A default world of `width × height` nodes at `cell` metres.
    pub fn new(seed: u64, width: usize, height: usize, cell: f64, f: &Figures) -> Self {
        Self {
            seed,
            width,
            height,
            cell,
            frame_origin: Geodetic::new(48.5, 37.5, 0.0),
            base_elevation: f.get("env.synth.base_elevation_m"),
            relief: f.get("env.synth.relief_m"),
            feature_scale_m: f.get("env.synth.feature_scale_m"),
            octaves: f.get("env.synth.octaves") as u32,
            persistence: f.get("env.synth.persistence"),
            rivers: f.get("env.synth.rivers") as usize,
            towns: f.get("env.synth.towns") as usize,
            forest_fraction: f.get("env.synth.forest_fraction"),
        }
    }
}

/// A town.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Town {
    pub name: String,
    pub e: f64,
    pub n: f64,
    pub radius_m: f64,
    pub road_node: usize,
}

/// The generated world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthWorld {
    pub terrain: Terrain,
    pub roads: RoadNetwork,
    pub rivers: Vec<River>,
    pub towns: Vec<Town>,
    pub power_lines: Vec<PowerLine>,
}

#[inline]
fn lattice(seed: u64, x: i64, y: i64, oct: u32) -> f64 {
    let h = mix64(seed ^ mix64((x as u64).wrapping_mul(0x9E37_79B9) ^ (y as u64).rotate_left(29) ^ ((oct as u64) << 56)));
    (h >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Value noise in [−1, 1] at continuous lattice coordinates.
pub fn value_noise(seed: u64, x: f64, y: f64, oct: u32) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (fade(x - x0), fade(y - y0));
    let (i, j) = (x0 as i64, y0 as i64);
    let a = lattice(seed, i, j, oct) + (lattice(seed, i + 1, j, oct) - lattice(seed, i, j, oct)) * fx;
    let b = lattice(seed, i, j + 1, oct) + (lattice(seed, i + 1, j + 1, oct) - lattice(seed, i, j + 1, oct)) * fx;
    a + (b - a) * fy
}

/// Fractal (fBm) value noise, normalised to about [−1, 1].
pub fn fbm(seed: u64, x: f64, y: f64, octaves: u32, persistence: f64) -> f64 {
    let (mut amp, mut freq, mut sum, mut norm) = (1.0, 1.0, 0.0, 0.0);
    for o in 0..octaves.max(1) {
        sum += amp * value_noise(seed, x * freq, y * freq, o);
        norm += amp;
        amp *= persistence;
        freq *= 2.0;
    }
    sum / norm
}

/// Build a world.
pub fn generate(p: &SynthParams, f: &Figures) -> SynthWorld {
    let spec = GridSpec::new(p.width, p.height, p.cell, 0.0, 0.0);
    let s1 = mix64(p.seed ^ 0x7465_7272);
    let mut elev: Raster<f32> = Raster::from_fn(spec, |i, j| {
        let (x, y) = (i as f64 * p.cell / p.feature_scale_m, j as f64 * p.cell / p.feature_scale_m);
        (p.base_elevation + p.relief * fbm(s1, x, y, p.octaves, p.persistence)) as f32
    });

    // Rivers: from well-separated high points, steepest descent to the edge,
    // carving a monotone bed and a valley.
    let river_depth = f.get("env.synth.river_depth_m");
    let valley_m = f.get("env.synth.valley_halfwidth_m");
    let mut order: Vec<usize> = (0..spec.len()).collect();
    order.sort_by(|&a, &b| elev.data[b].total_cmp(&elev.data[a]).then(a.cmp(&b)));
    let mut sources: Vec<usize> = Vec::new();
    let min_sep = (p.width.min(p.height) as f64 * 0.3).max(3.0);
    for &k in &order {
        if sources.len() >= p.rivers {
            break;
        }
        let (i, j) = spec.ij(k);
        let margin = 3;
        if i < margin || j < margin || i + margin >= p.width || j + margin >= p.height {
            continue;
        }
        if sources.iter().all(|&s| {
            let (a, b) = spec.ij(s);
            ((a as f64 - i as f64).powi(2) + (b as f64 - j as f64).powi(2)).sqrt() > min_sep
        }) {
            sources.push(k);
        }
    }
    let mut water = vec![false; spec.len()];
    let mut rivers = Vec::new();
    for (ri, &src) in sources.iter().enumerate() {
        let mut path = vec![src];
        let mut visited = vec![false; spec.len()];
        visited[src] = true;
        let mut bed = elev.data[src] as f64;
        let mut k = src;
        loop {
            let (i, j) = spec.ij(k);
            if i == 0 || j == 0 || i + 1 == p.width || j + 1 == p.height || water[k] && k != src {
                break;
            }
            let mut best: Option<(f32, usize)> = None;
            for (di, dj) in [(0i64, 1i64), (1, 0), (0, -1), (-1, 0), (1, 1), (1, -1), (-1, -1), (-1, 1)] {
                let nk = spec.idx((i as i64 + di) as usize, (j as i64 + dj) as usize);
                if visited[nk] {
                    continue;
                }
                let h = elev.data[nk];
                if best.is_none_or(|(bh, bk)| h < bh || (h == bh && nk < bk)) {
                    best = Some((h, nk));
                }
            }
            let Some((_, nk)) = best else { break };
            visited[nk] = true;
            bed = (elev.data[nk] as f64).min(bed - 0.05);
            elev.data[nk] = bed as f32;
            path.push(nk);
            k = nk;
            if path.len() > spec.len() {
                break;
            }
        }
        for &c in &path {
            water[c] = true;
        }
        let line: Vec<(f64, f64)> = path.iter().map(|&c| spec.pos(spec.ij(c).0, spec.ij(c).1)).collect();
        rivers.push(River {
            name: format!("river-{ri}"),
            width_m: (p.cell * 0.8).max(8.0),
            depth_m: river_depth,
            current_m_s: f.get("env.synth.river_current_m_s"),
            line,
        });
    }
    // Carve valleys: lower land near rivers towards the bed.
    if !rivers.is_empty() {
        let bed = elev.clone();
        let reach = (valley_m / p.cell).ceil() as i64;
        let mut lowered = elev.data.clone();
        for k in (0..spec.len()).filter(|&k| water[k]) {
            let (i, j) = spec.ij(k);
            for dj in -reach..=reach {
                for di in -reach..=reach {
                    let (x, y) = (i as i64 + di, j as i64 + dj);
                    if x < 0 || y < 0 || x >= p.width as i64 || y >= p.height as i64 {
                        continue;
                    }
                    let d = ((di * di + dj * dj) as f64).sqrt() * p.cell;
                    if d > valley_m {
                        continue;
                    }
                    let nk = spec.idx(x as usize, y as usize);
                    let w = (d / valley_m).powi(2);
                    let target = bed.data[k] as f64 + river_depth + (bed.data[nk] as f64 - bed.data[k] as f64) * w;
                    if (target as f32) < lowered[nk] && !water[nk] {
                        lowered[nk] = target as f32;
                    }
                }
            }
        }
        elev.data = lowered;
    }

    let mut terrain = Terrain::flat(p.frame_origin, spec, 0.0);
    terrain.elevation = elev;

    // Land cover from slope, height and a forest noise field.
    let s2 = mix64(p.seed ^ 0x666f_7265);
    let forest_noise: Vec<f64> = (0..spec.len())
        .map(|k| {
            let (i, j) = spec.ij(k);
            fbm(s2, i as f64 * p.cell / 2500.0, j as f64 * p.cell / 2500.0, 4, 0.5)
        })
        .collect();
    let mut sorted = forest_noise.clone();
    sorted.sort_by(f64::total_cmp);
    let thresh = sorted[((1.0 - p.forest_fraction.clamp(0.0, 1.0)) * (sorted.len() - 1) as f64) as usize];
    let dense_thresh = sorted[((1.0 - 0.4 * p.forest_fraction.clamp(0.0, 1.0)) * (sorted.len() - 1) as f64) as usize];
    let steep = f.get("env.synth.bare_slope");
    for k in 0..spec.len() {
        let (i, j) = spec.ij(k);
        let (x, y) = spec.pos(i, j);
        let slope = terrain.slope(x, y);
        let lc = if water[k] {
            LandCover::Water
        } else if slope > steep {
            LandCover::Bare
        } else if forest_noise[k] >= dense_thresh {
            LandCover::ForestDense
        } else if forest_noise[k] >= thresh {
            LandCover::ForestLight
        } else if slope > steep * 0.5 {
            LandCover::Scrub
        } else {
            LandCover::Open
        };
        terrain.land_cover.data[k] = lc;
    }
    // Wetland on flat ground next to rivers.
    for k in 0..spec.len() {
        if water[k] {
            continue;
        }
        let (i, j) = spec.ij(k);
        let near = [(0i64, 1i64), (1, 0), (0, -1), (-1, 0)].iter().any(|(di, dj)| {
            let (x, y) = (i as i64 + di, j as i64 + dj);
            x >= 0 && y >= 0 && x < p.width as i64 && y < p.height as i64 && water[spec.idx(x as usize, y as usize)]
        });
        let (x, y) = spec.pos(i, j);
        if near && terrain.slope(x, y) < 0.02 && (mix64(p.seed ^ k as u64) & 3) == 0 {
            terrain.land_cover.data[k] = LandCover::Wetland;
        }
    }

    // Towns: flat dry sites, preferring low ground, spaced apart.
    let town_r = f.get("env.synth.town_radius_m");
    let mut cand: Vec<(f64, usize)> = (0..spec.len())
        .filter(|&k| {
            let (i, j) = spec.ij(k);
            let (x, y) = spec.pos(i, j);
            let m = (town_r / p.cell).ceil() as usize + 1;
            !water[k] && i > m && j > m && i + m < p.width && j + m < p.height && terrain.slope(x, y) < 0.06
        })
        .map(|k| (terrain.elevation.data[k] as f64 + 30.0 * lattice(p.seed, k as i64, 7, 9), k))
        .collect();
    cand.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let town_sep = (p.width.min(p.height) as f64 * p.cell / (p.towns as f64).sqrt().max(1.0)) * 0.6;
    let mut roads = RoadNetwork::default();
    let mut towns: Vec<Town> = Vec::new();
    for (_, k) in cand {
        if towns.len() >= p.towns {
            break;
        }
        let (x, y) = spec.pos(spec.ij(k).0, spec.ij(k).1);
        if towns.iter().all(|t| (t.e - x).hypot(t.n - y) > town_sep) {
            let node = roads.add_node(x, y).index();
            let radius_m = town_r * (0.6 + 0.8 * (lattice(p.seed, k as i64, 3, 11) * 0.5 + 0.5));
            towns.push(Town { name: format!("town-{}", towns.len()), e: x, n: y, radius_m, road_node: node });
        }
    }
    for t in &towns {
        for k in 0..spec.len() {
            let (i, j) = spec.ij(k);
            let (x, y) = spec.pos(i, j);
            let d = (x - t.e).hypot(y - t.n);
            if water[k] {
                continue;
            }
            if d < t.radius_m * 0.45 {
                terrain.land_cover.data[k] = LandCover::UrbanDense;
            } else if d < t.radius_m {
                terrain.land_cover.data[k] = LandCover::UrbanLight;
            } else if d < t.radius_m * 4.0 && matches!(terrain.land_cover.data[k], LandCover::Open | LandCover::Scrub) {
                terrain.land_cover.data[k] = LandCover::Crops;
            }
        }
    }

    // Soil and wetness.
    let s3 = mix64(p.seed ^ 0x736f_696c);
    for k in 0..spec.len() {
        let (i, j) = spec.ij(k);
        let (x, y) = spec.pos(i, j);
        let n = fbm(s3, i as f64 * p.cell / 4000.0, j as f64 * p.cell / 4000.0, 3, 0.5);
        let rel = (terrain.elevation.data[k] as f64 - p.base_elevation) / p.relief.max(1.0);
        let lc = terrain.land_cover.data[k];
        let soil = if lc == LandCover::Wetland {
            SoilType::Peat
        } else if terrain.slope(x, y) > steep * 0.7 || rel > 0.6 {
            SoilType::Rock
        } else if rel < -0.2 {
            if n > 0.0 {
                SoilType::Clay
            } else {
                SoilType::Silt
            }
        } else if n > 0.45 {
            SoilType::Sand
        } else {
            SoilType::Loam
        };
        terrain.soil.data[k] = soil;
        terrain.wetness.data[k] = match lc {
            LandCover::Water => 1.0,
            LandCover::Wetland => 0.85,
            _ => (0.25 - 0.15 * rel).clamp(0.05, 0.6) as f32,
        };
        if lc == LandCover::ForestDense {
            terrain.obstacle_gap.data[k] = f.get("env.synth.dense_forest_gap_m") as f32;
        }
    }

    // Roads: a minimum spanning tree over the towns (Prim, deterministic),
    // each leg routed across a cost raster that dislikes slope, forest and
    // water; water crossings become bridge edges.
    let water_cost = f.get("env.synth.road_water_cost");
    let cost = Raster::from_fn(spec, |i, j| {
        let k = spec.idx(i, j);
        let (x, y) = spec.pos(i, j);
        let s = terrain.slope(x, y);
        let base = 1.0 + 25.0 * s * s * 100.0;
        match terrain.land_cover.data[k] {
            LandCover::Water => water_cost,
            LandCover::ForestDense => base * 2.5,
            LandCover::ForestLight | LandCover::Wetland => base * 1.6,
            _ => base,
        }
    });
    let mut in_tree = vec![false; towns.len()];
    let mut legs: Vec<(usize, usize)> = Vec::new();
    if !towns.is_empty() {
        in_tree[0] = true;
        for _ in 1..towns.len() {
            let mut best: Option<(f64, usize, usize)> = None;
            for a in (0..towns.len()).filter(|&a| in_tree[a]) {
                for b in (0..towns.len()).filter(|&b| !in_tree[b]) {
                    let d = (towns[a].e - towns[b].e).hypot(towns[a].n - towns[b].n);
                    if best.is_none_or(|x| d < x.0) {
                        best = Some((d, a, b));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                in_tree[b] = true;
                legs.push((a, b));
            }
        }
    }
    let bridge_mlc = f.get("env.synth.bridge_mlc") as u16;
    let waypoint_every = 8;
    for (li, &(a, b)) in legs.iter().enumerate() {
        let class = if li == 0 { RoadClass::Primary } else { RoadClass::Secondary };
        let sa = spec.nearest_clamped(towns[a].e, towns[a].n);
        let sb = spec.nearest_clamped(towns[b].e, towns[b].n);
        let Some(path) = cost_astar(&cost, None, spec.idx(sa.0, sa.1), spec.idx(sb.0, sb.1)) else { continue };
        let cells = &path.cells;
        let mut prev = NodeIndex::new(towns[a].road_node);
        let mut last = 0usize;
        let mut idx = 1usize;
        let node_at = |roads: &mut RoadNetwork, k: usize| {
            let (x, y) = spec.pos(spec.ij(k).0, spec.ij(k).1);
            roads.add_node(x, y)
        };
        while idx + 1 < cells.len() {
            if water[cells[idx]] {
                let start = idx - 1;
                let mut end = idx;
                while end + 1 < cells.len() && water[cells[end]] {
                    end += 1;
                }
                let ns = if start == last { prev } else { node_at(&mut roads, cells[start]) };
                if ns != prev {
                    roads.add_road(prev, ns, class, None);
                }
                let ne = node_at(&mut roads, cells[end]);
                let span = {
                    let (p0, p1) = (roads.graph[ns], roads.graph[ne]);
                    (p0.e - p1.e).hypot(p0.n - p1.n)
                };
                roads.add_road(ns, ne, class, Some(Bridge { load_class: bridge_mlc, span_m: span, blown: false }));
                prev = ne;
                last = end;
                idx = end + 1;
                continue;
            }
            if idx - last >= waypoint_every {
                let nn = node_at(&mut roads, cells[idx]);
                roads.add_road(prev, nn, class, None);
                prev = nn;
                last = idx;
            }
            idx += 1;
        }
        roads.add_road(prev, NodeIndex::new(towns[b].road_node), class, None);
    }

    let mut power_lines = Vec::new();
    if towns.len() >= 2 {
        power_lines.push(PowerLine { line: vec![(towns[0].e, towns[0].n), (towns[1].e, towns[1].n)], height_m: f.get("env.synth.power_line_height_m") });
    }

    SynthWorld { terrain, roads, rivers, towns, power_lines }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("env.synth.base_elevation_m", 150.0, "m", -100.0, 4000.0, "default", Default, "mean elevation of a synthetic theatre"),
        figure!("env.synth.relief_m", 120.0, "m", 0.0, 3000.0, "default", Default, "hill amplitude"),
        figure!("env.synth.feature_scale_m", 6000.0, "m", 200.0, 100_000.0, "default", Default, "wavelength of the largest hills"),
        figure!("env.synth.octaves", 6.0, "1", 1.0, 10.0, "default", Default, "noise octaves"),
        figure!("env.synth.persistence", 0.5, "1", 0.1, 0.9, "default", Default, "amplitude ratio between octaves (roughness)"),
        figure!("env.synth.rivers", 2.0, "1", 0.0, 20.0, "default", Default, "rivers traced"),
        figure!("env.synth.towns", 5.0, "1", 0.0, 100.0, "default", Default, "towns placed"),
        figure!("env.synth.forest_fraction", 0.3, "fraction", 0.0, 0.9, "default", Default, "share of land under forest"),
        figure!("env.synth.river_depth_m", 1.5, "m", 0.3, 15.0, "default", Default, "river depth (fording)"),
        figure!("env.synth.river_current_m_s", 0.8, "m/s", 0.0, 4.0, "default", Default, "river current"),
        figure!("env.synth.valley_halfwidth_m", 240.0, "m", 0.0, 3000.0, "default", Default, "half-width of the carved valley"),
        figure!("env.synth.bare_slope", 0.45, "1", 0.1, 2.0, "default", Default, "slopes steeper than this are bare rock"),
        figure!("env.synth.town_radius_m", 450.0, "m", 50.0, 5000.0, "default", Default, "typical town radius"),
        figure!("env.synth.dense_forest_gap_m", 4.0, "m", 0.5, 20.0, "default", Default, "clear gap between trunks in dense forest"),
        figure!("env.synth.road_water_cost", 40.0, "1", 1.0, 1000.0, "default", Default, "road-planning cost multiplier for crossing water"),
        figure!("env.synth.bridge_mlc", 40.0, "MLC", 4.0, 120.0, "default", Default, "load class of generated bridges"),
        figure!("env.synth.power_line_height_m", 25.0, "m", 5.0, 80.0, "default", Default, "power line height"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soil::VehicleClass;

    pub(crate) fn world(seed: u64) -> SynthWorld {
        let f = crate::default_figures();
        let p = SynthParams::new(seed, 160, 160, 30.0, &f);
        generate(&p, &f)
    }

    #[test]
    fn deterministic_and_complete() {
        let a = world(11);
        let b = world(11);
        assert_eq!(a.terrain, b.terrain);
        assert_ne!(a.terrain.elevation, world(12).terrain.elevation);
        let lc = &a.terrain.land_cover.data;
        for c in [LandCover::Water, LandCover::ForestLight, LandCover::ForestDense, LandCover::UrbanDense, LandCover::Crops] {
            assert!(lc.contains(&c), "missing {c:?}");
        }
        assert_eq!(a.towns.len(), 5);
        assert_eq!(a.rivers.len(), 2);
        assert!(a.roads.graph.edge_count() >= 4);
        assert!(a.roads.graph.edge_weights().any(|e| e.bridge.is_some()), "a road crosses a river on a bridge");
        // Every town is reachable from town 0 by road for a light vehicle.
        let f = crate::default_figures();
        let speeds = crate::network::RoadSpeeds::from_figures(&f);
        let u = crate::network::RoadUser { class: VehicleClass::Wheeled, load_class: 20 };
        for t in &a.towns[1..] {
            assert!(a.roads.route(NodeIndex::new(a.towns[0].road_node), NodeIndex::new(t.road_node), &u, &speeds).is_some());
        }
        // Rivers run downhill.
        for r in &a.rivers {
            let hs: Vec<f64> = r.line.iter().map(|&(e, n)| a.terrain.height(e, n)).collect();
            assert!(hs.windows(2).all(|w| w[1] <= w[0] + 1e-3));
        }
        let h = &a.terrain.elevation.data;
        let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 50.0, "hills and valleys: {lo}..{hi}");
    }
}
