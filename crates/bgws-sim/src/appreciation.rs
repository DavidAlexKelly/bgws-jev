//! Working out a pack's terrain appreciation (`bgws_env::appreciation`) from
//! its ground, once, when the pack is built (or the first time a pack
//! without one is opened).
//!
//! - **Exposure:** from each 60 m cell, a man-high eye looks out along 48
//!   rays to `sim.appreciation.exposure_radius_m` over the ground, roofs
//!   and canopy (woods and roofs block as solid, a little coarser than the
//!   game's own sight lines). The share of the ground it sees, by area, is
//!   the cell's exposure.
//! - **Mobility:** each 30 m cell is go, slow-go (under
//!   `sim.appreciation.slow_go_frac` of the class's open-country speed) or
//!   no-go, by the game's own going (soil, slope, woods, buildings, rivers
//!   it cannot ford; roads are go). Width is twice the distance to the
//!   nearest ground that is not go.
//! - **Chokepoints:** open corridors are go-ground at least
//!   `sim.appreciation.corridor_m` wide, in areas of a square kilometre or
//!   more. Spreading out from each through the go-ground, where two meet is
//!   the gap between them.
//! - **Crossings:** road bridges over rivers, by road class and depth.
//! - **Observation:** local high points, scored by how much they see out to
//!   `sim.appreciation.observe_radius_m`.

use crate::ground::Ground;
use bgws_core::{figure, Figure, Figures};
use bgws_env::appreciation::{AnalysisSettings, Appreciation, KeyKind, KeyTerrain, CLASSES, VERSION};
use bgws_env::{GridSpec, LandCover, RoadClass, VehicleClass};
use rayon::prelude::*;
use std::collections::{BTreeMap, VecDeque};

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.appreciation.exposure_radius_m", 2_000.0, "m", 500.0, 6_000.0, "default", Default, "radius over which a cell's exposure (the share of the ground it sees, and that sees it) is worked out"),
        figure!("sim.appreciation.eye_m", 2.0, "m", 0.5, 5.0, "default", Default, "eye and target height for the exposure analysis"),
        figure!("sim.appreciation.corridor_m", 400.0, "m", 100.0, 2_000.0, "default", Low, "narrowest go-ground that counts as a corridor (about a company's frontage)"),
        figure!("sim.appreciation.slow_go_frac", 0.4, "fraction", 0.1, 0.9, "default", Low, "going below this share of a class's open-country speed is slow-go"),
        figure!("sim.appreciation.observe_radius_m", 4_000.0, "m", 1_000.0, 10_000.0, "default", Default, "radius an observation point's view is scored over"),
        figure!("sim.appreciation.dead_ground_share", 0.3, "fraction", 0.0, 1.0, "default", Low, "share of a route's open-ground exposure weight that applies in fully dead ground (none of the ground round it sees it)"),
        figure!("sim.appreciation.cover_exposure_weight", 0.3, "ratio", 0.0, 2.0, "default", Low, "how much a unit looking for a position prefers ground less open to view, against cover"),
    ]
}

#[derive(Clone, Debug)]
pub struct AnalyseParams {
    pub exposure_radius_m: f64,
    pub eye_m: f64,
    pub corridor_m: f64,
    pub slow_go_frac: f64,
    pub observe_radius_m: f64,
}

impl AnalyseParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            exposure_radius_m: f.get("sim.appreciation.exposure_radius_m"),
            eye_m: f.get("sim.appreciation.eye_m"),
            corridor_m: f.get("sim.appreciation.corridor_m"),
            slow_go_frac: f.get("sim.appreciation.slow_go_frac"),
            observe_radius_m: f.get("sim.appreciation.observe_radius_m"),
        }
    }
}

/// Ground and obstacle tops on the terrain grid, for fast rays.
struct Heights {
    spec: GridSpec,
    ground: Vec<f32>,
    /// Height of roofs or canopy above the ground (0 none).
    over: Vec<f32>,
}

impl Heights {
    fn of(g: &Ground) -> Self {
        let spec = g.terrain.spec;
        let ground = g.terrain.elevation.data.clone();
        let with_surface = g.sight.is_some();
        let over: Vec<f32> = (0..spec.len())
            .into_par_iter()
            .map(|k| {
                let (e, n) = spec.pos(k % spec.width, k / spec.width);
                let lc = g.terrain.land_cover.data[k];
                let canopy = g.cover.get(lc).canopy_m;
                let building = if with_surface {
                    if g.built_share(e, n) >= 0.3 {
                        g.block(e, n).map(|b| b.mean_height_m).unwrap_or(8.0)
                    } else {
                        0.0
                    }
                } else if lc.is_urban() {
                    canopy
                } else {
                    0.0
                };
                let tree = if lc.is_urban() || matches!(lc, LandCover::Water) { 0.0 } else { canopy };
                building.max(tree) as f32
            })
            .collect();
        Self { spec, ground, over }
    }

    /// How far a point stands above the lowest ground 0.5–1 km round it, m.
    fn rise(&self, e: f64, n: f64) -> f64 {
        let Some(k0) = self.at(e, n) else { return 0.0 };
        let mut lo = self.ground[k0];
        for d in [500.0, 1_000.0] {
            for b in 0..8 {
                let a = b as f64 * std::f64::consts::FRAC_PI_4;
                if let Some(q) = self.at(e + a.sin() * d, n + a.cos() * d) {
                    lo = lo.min(self.ground[q]);
                }
            }
        }
        (self.ground[k0] - lo) as f64
    }

    #[inline]
    fn at(&self, e: f64, n: f64) -> Option<usize> {
        let s = &self.spec;
        let i = ((e - s.origin_e) / s.cell).round();
        let j = ((n - s.origin_n) / s.cell).round();
        if i < 0.0 || j < 0.0 || i >= s.width as f64 || j >= s.height as f64 {
            return None;
        }
        Some(j as usize * s.width + i as usize)
    }

    /// The share of the ground within `radius` an eye at (e, n) sees, by
    /// area, along `rays` rays: man-high targets, solid roofs and canopy.
    /// With `whole`, the share of the whole disc (ground off the map counts
    /// as unseen), else of the part on the map.
    fn view(&self, e: f64, n: f64, radius: f64, rays: usize, eye: f64, whole: bool) -> f64 {
        let Some(k0) = self.at(e, n) else { return 0.0 };
        let z0 = self.ground[k0] as f64 + eye;
        let step = self.spec.cell;
        let inv_2kr = 1.0 / (2.0 * 1.15 * bgws_core::geo::EARTH_RADIUS);
        let clear = 30.0;
        let (mut seen, mut all) = (0.0, 0.0);
        for r in 0..rays {
            let b = r as f64 * std::f64::consts::TAU / rays as f64;
            let (dx, dy) = (b.sin(), b.cos());
            let mut max = f64::NEG_INFINITY;
            let mut d = step;
            while d <= radius {
                let Some(k) = self.at(e + dx * d, n + dy * d) else { break };
                let drop = d * d * inv_2kr;
                let g = self.ground[k] as f64 - drop;
                let over = self.over[k] as f64;
                let target = (g + eye - z0) / d;
                // Under a roof or canopy a man is hidden, beyond the first few metres.
                let hidden = d > clear && over > eye + 0.5;
                if !hidden && target >= max {
                    seen += d;
                }
                all += d;
                let top = if d > clear { g + over } else { g };
                max = max.max((top - z0) / d);
                d += step;
            }
        }
        if whole {
            let steps = (radius / step).floor();
            let disc = rays as f64 * step * steps * (steps + 1.0) / 2.0;
            seen / disc
        } else if all > 0.0 {
            seen / all
        } else {
            0.0
        }
    }
}

fn class_of(c: usize) -> VehicleClass {
    [VehicleClass::Tracked, VehicleClass::Wheeled, VehicleClass::Foot][c]
}

/// Two-pass chamfer distance (m) to the nearest cell that is not `go`.
fn distance_to_not_go(spec: &GridSpec, go: &[bool]) -> Vec<f64> {
    let (w, h) = (spec.width, spec.height);
    let (a, b) = (spec.cell, spec.cell * std::f64::consts::SQRT_2);
    let mut d: Vec<f64> = go.iter().map(|&g| if g { f64::INFINITY } else { 0.0 }).collect();
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + a);
            }
            if j > 0 {
                v = v.min(d[k - w] + a);
                if i > 0 {
                    v = v.min(d[k - w - 1] + b);
                }
                if i + 1 < w {
                    v = v.min(d[k - w + 1] + b);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..h).rev() {
        for i in (0..w).rev() {
            let k = j * w + i;
            let mut v = d[k];
            if i + 1 < w {
                v = v.min(d[k + 1] + a);
            }
            if j + 1 < h {
                v = v.min(d[k + w] + a);
                if i + 1 < w {
                    v = v.min(d[k + w + 1] + b);
                }
                if i > 0 {
                    v = v.min(d[k + w - 1] + b);
                }
            }
            d[k] = v;
        }
    }
    d
}

fn neighbours(w: usize, h: usize, k: usize) -> impl Iterator<Item = usize> {
    let (i, j) = ((k % w) as i64, (k / w) as i64);
    (-1..=1i64).flat_map(move |dj| (-1..=1i64).map(move |di| (di, dj))).filter_map(move |(di, dj)| {
        let (x, y) = (i + di, j + dj);
        ((di, dj) != (0, 0) && x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize)
    })
}

/// Gaps between wide corridors: (cell, width m, the two areas' sizes m²).
fn chokepoints(spec: &GridSpec, going: &[u8], width: &[u16], corridor_m: f64) -> Vec<(usize, f64, f64, f64)> {
    let (w, h) = (spec.width, spec.height);
    let n = spec.len();
    let cell_m2 = spec.cell * spec.cell;
    // Wide go-ground, labelled by area.
    let mut label = vec![0u32; n];
    let mut areas = vec![0.0];
    for s in 0..n {
        if label[s] != 0 || (width[s] as f64) < corridor_m {
            continue;
        }
        let id = areas.len() as u32;
        let mut stack = vec![s];
        label[s] = id;
        let mut count = 0usize;
        while let Some(k) = stack.pop() {
            count += 1;
            for q in neighbours(w, h, k) {
                if label[q] == 0 && width[q] as f64 >= corridor_m {
                    label[q] = id;
                    stack.push(q);
                }
            }
        }
        areas.push(count as f64 * cell_m2);
    }
    let big: Vec<bool> = areas.iter().map(|&a| a >= 1.0e6).collect();
    // Spread each big area's label through the go-ground, up to 3 km.
    let mut owner = vec![0u32; n];
    let mut dist = vec![u32::MAX; n];
    let mut q = VecDeque::new();
    for k in 0..n {
        if label[k] != 0 && big[label[k] as usize] {
            owner[k] = label[k];
            dist[k] = 0;
            q.push_back(k);
        }
    }
    let reach = (3_000.0 / spec.cell) as u32;
    let mut meets: BTreeMap<(u32, u32, i64, i64), (usize, f64)> = BTreeMap::new();
    while let Some(k) = q.pop_front() {
        let (a, dk) = (owner[k], dist[k]);
        for v in neighbours(w, h, k) {
            if going[v] != 2 {
                continue;
            }
            if owner[v] == 0 {
                if dk < reach {
                    owner[v] = a;
                    dist[v] = dk + 1;
                    q.push_back(v);
                }
            } else if owner[v] != a {
                // Two fronts meet: the narrower of the two cells is the gap.
                let c = if width[v] <= width[k] { v } else { k };
                let (x, y) = spec.pos(c % w, c / w);
                let key = (a.min(owner[v]), a.max(owner[v]), (x / 500.0).floor() as i64, (y / 500.0).floor() as i64);
                let wd = width[c] as f64;
                let e = meets.entry(key).or_insert((c, wd));
                if wd < e.1 {
                    *e = (c, wd);
                }
            }
        }
    }
    meets.into_iter().map(|((a, b, _, _), (c, wd))| (c, wd, areas[a as usize], areas[b as usize])).filter(|x| x.1 < corridor_m).collect()
}

/// Keep the best of points closer than `sep` (best first).
fn thin(mut v: Vec<KeyTerrain>, sep: f64, keep: usize) -> Vec<KeyTerrain> {
    v.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out: Vec<KeyTerrain> = Vec::new();
    for p in v {
        if out.len() >= keep {
            break;
        }
        if let Some(o) = out.iter_mut().find(|o| (o.e - p.e).hypot(o.n - p.n) < sep) {
            for c in p.classes {
                if !o.classes.contains(&c) {
                    o.classes.push(c);
                }
            }
            continue;
        }
        out.push(p);
    }
    out
}

/// The appreciation of a ground (in its own frame).
pub fn analyse(g: &Ground, p: &AnalyseParams, progress: &(dyn Fn(&str, f64) + Sync)) -> Appreciation {
    let hs = Heights::of(g);
    let ts = g.terrain.spec;
    // Exposure, on cells twice the terrain's.
    progress("terrain analysis: exposure", 0.0);
    let k = 2usize;
    let es = GridSpec::new(ts.width.div_ceil(k), ts.height.div_ceil(k), ts.cell * k as f64, ts.origin_e, ts.origin_n);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let exposure: Vec<u8> = (0..es.height)
        .into_par_iter()
        .flat_map_iter(|j| {
            let row: Vec<u8> = (0..es.width)
                .map(|i| {
                    let (e, n) = es.pos(i, j);
                    (hs.view(e, n, p.exposure_radius_m, 48, p.eye_m, false) * 255.0).round() as u8
                })
                .collect();
            let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if d.is_multiple_of(32) {
                progress("terrain analysis: exposure", d as f64 / es.height as f64);
            }
            row
        })
        .collect();
    // Mobility per class.
    progress("terrain analysis: mobility", 0.0);
    let mut going: [Vec<u8>; 3] = Default::default();
    let mut width_m: [Vec<u16>; 3] = Default::default();
    for c in 0..3 {
        let class = class_of(c);
        let ford = g.ford.depth_m[c] as f32;
        let speeds: Vec<(f64, bool)> = (0..ts.len())
            .into_par_iter()
            .map(|k| {
                let (e, n) = ts.pos(k % ts.width, k / ts.width);
                let road = g.roads.as_ref().is_some_and(|r| r.data[k] > 0);
                let river = g.rivers.as_ref().map(|r| r.data[k]).unwrap_or(0.0);
                let v = if river > ford && class != VehicleClass::Foot { 0.0 } else { g.going(e, n, class).speed_kmh };
                (v, road)
            })
            .collect();
        // Open-country speed: the 95th percentile of what moves.
        let mut moving: Vec<f64> = speeds.iter().filter(|x| x.0 >= 1.0).map(|x| x.0).collect();
        moving.sort_by(f64::total_cmp);
        let open = moving.get(moving.len() * 95 / 100).copied().unwrap_or(1.0);
        let gv: Vec<u8> = speeds.iter().map(|&(v, road)| if road || v >= p.slow_go_frac * open { 2 } else if v >= 1.0 { 1 } else { 0 }).collect();
        let d = distance_to_not_go(&ts, &gv.iter().map(|&x| x == 2).collect::<Vec<_>>());
        width_m[c] = d.iter().map(|&x| if x.is_finite() { (2.0 * x).min(65_535.0) as u16 } else { u16::MAX }).collect();
        going[c] = gv;
        progress("terrain analysis: mobility", (c + 1) as f64 / 3.0);
    }
    let mut key = Vec::new();
    let mut crossings_kept: Vec<KeyTerrain> = Vec::new();
    // Bridges over rivers.
    if let (Some(lines), Some(rivers)) = (g.road_lines.as_ref(), g.rivers.as_ref()) {
        let mut crossings = Vec::new();
        for l in lines.iter().filter(|l| l.bridge) {
            let mut deepest = 0.0f32;
            for w in l.line.windows(2) {
                for s in 0..=4 {
                    let t = s as f64 / 4.0;
                    let (e, n) = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                    if let Some((i, j)) = ts.nearest(e, n) {
                        deepest = deepest.max(rivers.data[ts.idx(i, j)]);
                    }
                }
            }
            if deepest <= 0.0 {
                continue;
            }
            let mid = l.line[l.line.len() / 2];
            let class_w = match l.class {
                RoadClass::Motorway => 1.0,
                RoadClass::Primary => 0.85,
                RoadClass::Secondary => 0.6,
                RoadClass::Track => 0.35,
            };
            let no_ford = deepest > g.ford.depth_m[0] as f32;
            crossings.push(KeyTerrain {
                kind: KeyKind::Crossing,
                e: mid.0,
                n: mid.1,
                score: class_w * if no_ford { 1.0 } else { 0.6 },
                classes: vec![],
                note: format!("{} bridge over water {:.1} m deep{}", l.class.key(), deepest, if no_ford { ", too deep to ford" } else { ", fordable by tracks" }),
            });
        }
        crossings_kept = thin(crossings, 200.0, 40);
    }
    // Chokepoints for vehicles.
    progress("terrain analysis: chokepoints", 0.0);
    let mut chokes = Vec::new();
    for c in 0..2 {
        let mut per_pair: BTreeMap<(u64, u64), usize> = BTreeMap::new();
        let mut found = chokepoints(&ts, &going[c], &width_m[c], p.corridor_m);
        found.sort_by(|x, y| x.1.total_cmp(&y.1));
        for (cell, wd, a1, a2) in found {
            let (e, n) = ts.pos(cell % ts.width, cell / ts.width);
            // A bridge is its own key terrain; a river's many crossings are not each a gap.
            if crossings_kept.iter().any(|x| (x.e - e).hypot(x.n - n) < 300.0) {
                continue;
            }
            let pair = per_pair.entry((a1.min(a2) as u64, a1.max(a2) as u64)).or_default();
            if *pair >= 3 {
                continue;
            }
            *pair += 1;
            let small = a1.min(a2);
            chokes.push(KeyTerrain {
                kind: KeyKind::Chokepoint,
                e,
                n,
                score: (small / 1.0e6).ln_1p() * (1.0 - wd / p.corridor_m),
                classes: vec![CLASSES[c].into()],
                note: format!("a gap {:.0} m wide between open areas of {:.1} and {:.1} km²", wd.max(ts.cell), a1.max(a2) / 1.0e6, small / 1.0e6),
            });
        }
    }
    let mut chokes = thin(chokes, 600.0, 40);
    let top = chokes.iter().map(|c| c.score).fold(0.0, f64::max).max(1e-9);
    for c in &mut chokes {
        c.score /= top;
        c.note = format!("{} ({})", c.note, c.classes.join(", "));
    }
    key.extend(chokes);
    key.extend(crossings_kept);
    // Observation: local high points of the exposure grid, scored by what they see.
    progress("terrain analysis: observation", 0.0);
    let ew = es.width;
    let r = 4i64;
    let candidates: Vec<usize> = (0..es.len())
        .filter(|&c| {
            let (i, j) = ((c % ew) as i64, (c / ew) as i64);
            let (e, n) = es.pos(i as usize, j as usize);
            let Some(k0) = hs.at(e, n) else { return false };
            if hs.over[k0] > 0.0 {
                return false;
            }
            let h0 = hs.ground[k0];
            for dj in -r..=r {
                for di in -r..=r {
                    let (x, y) = (i + di, j + dj);
                    if (di, dj) == (0, 0) || x < 0 || y < 0 || x >= ew as i64 || y >= es.height as i64 {
                        continue;
                    }
                    let (pe, pn) = es.pos(x as usize, y as usize);
                    if hs.at(pe, pn).is_some_and(|q| hs.ground[q] > h0) {
                        return false;
                    }
                }
            }
            // Standing clear of the ground round it.
            hs.rise(e, n) >= 5.0
        })
        .collect();
    let mut obs: Vec<KeyTerrain> = candidates
        .par_iter()
        .map(|&c| {
            let (e, n) = es.pos(c % ew, c / ew);
            let v = hs.view(e, n, p.observe_radius_m, 96, p.eye_m, true);
            let around = hs.rise(e, n);
            let km2 = v * std::f64::consts::PI * (p.observe_radius_m / 1_000.0).powi(2);
            KeyTerrain { kind: KeyKind::Observation, e, n, score: km2, classes: vec![], note: format!("sees {km2:.1} km² within {:.0} km, {around:.0} m above the ground round it", p.observe_radius_m / 1_000.0) }
        })
        .collect();
    obs = thin(obs, 1_500.0, 25);
    let top = obs.iter().map(|o| o.score).fold(0.0, f64::max).max(1e-9);
    for o in &mut obs {
        o.score /= top;
    }
    key.extend(obs);
    progress("terrain analysis: done", 1.0);
    Appreciation {
        version: VERSION,
        exposure_spec: es,
        exposure,
        mobility_spec: ts,
        going,
        width_m,
        key,
        settings: AnalysisSettings { exposure_radius_m: p.exposure_radius_m, eye_m: p.eye_m, corridor_m: p.corridor_m },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Geodetic;
    use bgws_env::{GridSpec, Terrain};

    /// A cone hill, a pit, and a wood with a gap between two open areas.
    #[test]
    fn dead_ground_corridors_gaps_and_high_points() {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(200, 200, 30.0, -3_000.0, -3_000.0);
        let mut t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        for j in 0..spec.height {
            for i in 0..spec.width {
                let (e, n) = spec.pos(i, j);
                let k = spec.idx(i, j);
                // A hill 60 m high, straight-sided, at (1500, 1500); a pit 20 m deep at (1500, -1500).
                let hill = 60.0 * (1.0 - (e - 1_500.0).hypot(n - 1_500.0) / 800.0).max(0.0);
                let pit = 20.0 * (1.0 - (e - 1_500.0).hypot(n + 1_500.0) / 300.0).max(0.0);
                t.elevation.data[k] = (hill - pit) as f32;
                // A wood across the map at e ∈ [-300, 300], with a 150 m gap at n ∈ [-1500, -1350].
                if e.abs() <= 300.0 && !(-1_500.0..=-1_350.0).contains(&n) {
                    t.land_cover.data[k] = LandCover::ForestDense;
                }
            }
        }
        let g = Ground::from_terrain(t, 1, &f);
        let a = analyse(&g, &AnalyseParams::from_figures(&f), &|_, _| {});
        let top = a.exposure_at(1_500.0, 1_500.0).unwrap();
        let open = a.exposure_at(2_500.0, 0.0).unwrap();
        let pit = a.exposure_at(1_500.0, -1_500.0).unwrap();
        let wood = a.exposure_at(0.0, 2_000.0).unwrap();
        eprintln!("hill {top:.2} open {open:.2} pit {pit:.2} wood {wood:.2}; key {:#?}", a.key);
        assert!(top > 0.7 && open > 0.7, "a hilltop and open plain are seen from all round");
        assert!(pit < 0.2);
        assert!(wood < 0.1, "inside a dense wood little sees you");
        // The wood is not go for tracks; the open areas are wide corridors.
        assert!(a.mobility_at(0, 0.0, 2_000.0).unwrap().0 < 2, "the wood is not go for tracks");
        assert!(a.mobility_at(0, -2_000.0, -2_000.0).unwrap().1 > 400.0);
        let gap = a.key.iter().filter(|k| k.kind == KeyKind::Chokepoint).find(|k| k.e.abs() < 400.0 && (k.n + 1_425.0).abs() < 300.0);
        assert!(gap.is_some(), "the gap through the wood is a chokepoint");
        let op = a.key.iter().filter(|k| k.kind == KeyKind::Observation).max_by(|x, y| x.score.total_cmp(&y.score)).unwrap();
        assert!((op.e - 1_500.0).hypot(op.n - 1_500.0) < 200.0, "the best observation is the hilltop: {op:?}");
    }
}
