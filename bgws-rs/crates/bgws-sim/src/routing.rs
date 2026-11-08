//! Route planning over roads and open ground together (§8.1).
//!
//! One A* search runs over the terrain grid at its own resolution, in a
//! corridor round the move. A road cell costs the road's time, an open cell
//! the time its going allows, and a step on or off a road costs the time to
//! cross a verge. What a route is for decides the rest:
//!
//! | Mode | Cares about |
//! |---|---|
//! | March | time; wheeled vehicles keep to roads (a strong preference), tracked ones cut across fields when that is quicker (a soft one) |
//! | Tactical | time, open ground (by the land cover's concealment), and roads near the enemy it believes in |
//! | Assault | time only: straight at it |
//! | Stealth | concealment above all; roads are exposed everywhere |
//!
//! A cell's cost is its travel time plus exposure: open ground (1 − the land
//! cover's concealment) weighted by the mode, counted over the time spent in
//! it, but never less than at a reference pace. Troops on foot are exposed
//! for as long as they take to cross; a fast vehicle is no less conspicuous
//! for being quick. Roads
//! are where mines, registered fire and drones wait, so inside the threat
//! radius of an enemy the side believes in (its tracks and its units'
//! sightings, never the truth) a road cell costs more, the closer the more.
//! So a column marches up the road and leaves it as it nears the front. All
//! weights are figures (`sim.route.*`), all [default] until calibrated.

use crate::components::MoveModeS;
use crate::ground::Ground;
use bgws_core::{figure, Enu, Figure, Figures};
use bgws_env::soil::VehicleClass;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The planner's weights.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteParams {
    /// Multiplier on a road's time on a march: [tracked, wheeled, foot].
    pub march_road: [f64; 3],
    /// Multiplier on open ground's time on a march: [tracked, wheeled, foot].
    pub march_open: [f64; 3],
    /// Weight on lack of concealment, per mode [march, tactical, assault, stealth].
    pub exposure: [f64; 4],
    /// The share of that weight that applies far from any believed threat, per mode
    /// (all of it inside a threat's radius, rising with nearness).
    pub exposure_baseline: [f64; 4],
    /// Extra multiplier on a road's time everywhere, per mode (stealth avoids them).
    pub road_mult: [f64; 4],
    /// How much more a road cell costs next to a believed threat, per mode.
    pub threat_road: [f64; 4],
    /// Radius of a believed threat's reach over roads (direct fire, ATGMs, FPV drones), m.
    pub threat_radius_m: f64,
    /// Threats this close to the destination (as a fraction of the threat radius)
    /// are the objective itself, not steered round.
    pub objective_frac: f64,
    /// The fastest pace exposure is counted at (slower movers count their own time), km/h.
    pub exposure_ref_kmh: f64,
    /// Time to get on or off a road across a verge or ditch, s.
    pub verge_s: f64,
    /// The corridor round a move: at least this far either side, m.
    pub corridor_min_m: f64,
    /// And this fraction of the move's length.
    pub corridor_frac: f64,
    /// The most cells a search may cover before the grid is thinned.
    pub max_cells: usize,
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.route.march.road_tracked", 0.85, "ratio", 0.2, 2.0, "default", Default, "multiplier on road time for a tracked vehicle on a march (soft preference)"),
        figure!("sim.route.march.road_wheeled", 0.5, "ratio", 0.2, 2.0, "default", Default, "multiplier on road time for a wheeled vehicle on a march (strong preference)"),
        figure!("sim.route.march.road_foot", 0.9, "ratio", 0.2, 2.0, "default", Default, "multiplier on road time for troops on foot on a march"),
        figure!("sim.route.march.open_tracked", 1.0, "ratio", 0.5, 5.0, "default", Default, "multiplier on cross-country time for a tracked vehicle on a march"),
        figure!("sim.route.march.open_wheeled", 1.6, "ratio", 0.5, 5.0, "default", Default, "multiplier on cross-country time for a wheeled vehicle on a march"),
        figure!("sim.route.march.open_foot", 1.0, "ratio", 0.5, 5.0, "default", Default, "multiplier on cross-country time for troops on foot on a march"),
        figure!("sim.route.exposure.march", 0.0, "ratio", 0.0, 10.0, "default", Default, "weight on open ground (1 − concealment) on a march"),
        figure!("sim.route.exposure.tactical", 0.6, "ratio", 0.0, 10.0, "default", Default, "weight on open ground (1 − concealment) in a tactical move"),
        figure!("sim.route.exposure.assault", 0.0, "ratio", 0.0, 10.0, "default", Default, "weight on open ground (1 − concealment) in an assault"),
        figure!("sim.route.exposure.stealth", 2.5, "ratio", 0.0, 10.0, "default", Default, "weight on open ground (1 − concealment) when moving by stealth"),
        figure!("sim.route.exposure_baseline.tactical", 0.3, "ratio", 0.0, 1.0, "default", Default, "share of a tactical move's exposure weight that applies far from any believed threat"),
        figure!("sim.route.exposure_baseline.stealth", 1.0, "ratio", 0.0, 1.0, "default", Default, "share of a stealthy move's exposure weight that applies far from any believed threat"),
        figure!("sim.route.road.stealth", 2.0, "ratio", 1.0, 10.0, "default", Default, "multiplier on road time when moving by stealth (roads are watched)"),
        figure!("sim.route.threat_road.march", 0.5, "ratio", 0.0, 20.0, "default", Default, "extra road cost next to a believed threat on a march"),
        figure!("sim.route.threat_road.tactical", 3.0, "ratio", 0.0, 20.0, "default", Default, "extra road cost next to a believed threat in a tactical move"),
        figure!("sim.route.threat_road.assault", 0.0, "ratio", 0.0, 20.0, "default", Default, "extra road cost next to a believed threat in an assault"),
        figure!("sim.route.threat_road.stealth", 5.0, "ratio", 0.0, 20.0, "default", Default, "extra road cost next to a believed threat when moving by stealth"),
        figure!("sim.route.threat_radius_m", 4_000.0, "m", 500.0, 20_000.0, "default", Default, "reach of a believed threat over roads (direct fire, ATGM, FPV)"),
        figure!("sim.route.objective_frac", 0.5, "ratio", 0.0, 2.0, "default", Default, "believed threats within this share of the threat radius of a move's destination are its objective, not steered round"),
        figure!("sim.route.exposure_ref_kmh", 10.0, "km/h", 1.0, 60.0, "default", Default, "fastest pace exposure is counted at; slower movers count the time they take"),
        figure!("sim.route.verge_s", 12.0, "s", 0.0, 120.0, "default", Default, "time to get on or off a road across a verge or ditch"),
        figure!("sim.route.corridor_min_m", 1_500.0, "m", 200.0, 20_000.0, "default", Default, "least corridor either side of a planned move"),
        figure!("sim.route.corridor_frac", 0.35, "ratio", 0.05, 2.0, "default", Default, "corridor either side of a planned move, as a fraction of its length"),
        figure!("sim.route.max_cells", 400_000.0, "cells", 10_000.0, 20_000_000.0, "default", Default, "largest search grid before it is thinned"),
    ]
}

impl RouteParams {
    pub fn from_figures(f: &Figures) -> Self {
        let g = |k: &str| f.get(&format!("sim.route.{k}"));
        let one = |k: &str| if f.figure(&format!("sim.route.{k}")).is_some() { g(k) } else { 1.0 };
        RouteParams {
            march_road: [g("march.road_tracked"), g("march.road_wheeled"), g("march.road_foot")],
            march_open: [g("march.open_tracked"), g("march.open_wheeled"), g("march.open_foot")],
            exposure: [g("exposure.march"), g("exposure.tactical"), g("exposure.assault"), g("exposure.stealth")],
            exposure_baseline: [1.0, g("exposure_baseline.tactical"), 1.0, g("exposure_baseline.stealth")],
            road_mult: [1.0, 1.0, 1.0, one("road.stealth")],
            threat_road: [g("threat_road.march"), g("threat_road.tactical"), g("threat_road.assault"), g("threat_road.stealth")],
            threat_radius_m: g("threat_radius_m"),
            exposure_ref_kmh: g("exposure_ref_kmh"),
            objective_frac: g("objective_frac"),
            verge_s: g("verge_s"),
            corridor_min_m: g("corridor_min_m"),
            corridor_frac: g("corridor_frac"),
            max_cells: g("max_cells") as usize,
        }
    }
}

/// What a route is for and who drives it.
#[derive(Clone, Debug)]
pub struct RouteRequest<'a> {
    pub from: Enu,
    pub to: Enu,
    pub class: VehicleClass,
    pub mode: MoveModeS,
    /// The vehicle's own top speeds, km/h.
    pub road_kmh: f64,
    pub cross_kmh: f64,
    /// Enemy positions the side believes in (ENU e, n).
    pub threats: &'a [(f64, f64)],
    /// Time-critical (orders "now", or getting a weapon into range): exposure
    /// counts only near a believed threat, not everywhere.
    pub urgent: bool,
}

fn mode_ix(m: MoveModeS) -> usize {
    match m {
        MoveModeS::March => 0,
        MoveModeS::Tactical => 1,
        MoveModeS::Assault => 2,
        MoveModeS::Stealth => 3,
    }
}

fn class_ix(c: VehicleClass) -> usize {
    match c {
        VehicleClass::Tracked => 0,
        VehicleClass::Wheeled => 1,
        VehicleClass::Foot => 2,
    }
}

/// A planned route, with what it is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub waypoints: Vec<Enu>,
    /// Expected time at the vehicle's own speeds, s (before mode caps and suppression).
    pub time_s: f64,
    /// Metres on roads and across country.
    pub road_m: f64,
    pub open_m: f64,
}

impl Ground {
    /// Plan a route: waypoints at turns and where it joins or leaves a road.
    /// Falls back to the straight line when either end is off the ground or
    /// nothing joins them.
    pub fn plan(&self, r: &RouteRequest, p: &RouteParams) -> Plan {
        let straight = |t: f64| Plan { waypoints: vec![r.to], time_s: t, road_m: 0.0, open_m: r.from.sub(r.to).horizontal() };
        let dist = r.from.sub(r.to).horizontal();
        if dist < 60.0 || !self.contains(r.from) || !self.contains(r.to) {
            return straight(0.0);
        }
        // A wider corridor when nothing joins the ends in a narrow one (a river
        // whose bridge lies outside it).
        let mut margin = p.corridor_min_m.max(p.corridor_frac * dist);
        for _ in 0..3 {
            if let Some(plan) = self.plan_in(r, p, margin) {
                return plan;
            }
            margin *= 3.0;
        }
        straight(0.0)
    }

    fn plan_in(&self, r: &RouteRequest, p: &RouteParams, margin: f64) -> Option<Plan> {
        let ts = self.terrain.spec;
        // The corridor, in terrain cells, thinned if it would be too big.
        let lo = (r.from.e.min(r.to.e) - margin, r.from.n.min(r.to.n) - margin);
        let hi = (r.from.e.max(r.to.e) + margin, r.from.n.max(r.to.n) + margin);
        let (i0, j0) = ts.nearest_clamped(lo.0, lo.1);
        let (i1, j1) = ts.nearest_clamped(hi.0, hi.1);
        let cells = (i1 - i0 + 1) * (j1 - j0 + 1);
        let stride = ((cells as f64 / p.max_cells.max(1) as f64).sqrt().ceil() as usize).max(1);
        let (w, h) = ((i1 - i0) / stride + 1, (j1 - j0) / stride + 1);
        let step = ts.cell * stride as f64;
        let pos = |k: usize| {
            let (i, j) = (k % w, k / w);
            ts.pos(i0 + i * stride, j0 + j * stride)
        };
        let cell_of = |e: f64, n: f64| -> usize {
            let i = (((e - ts.origin_e) / ts.cell).round() as isize - i0 as isize).div_euclid(stride as isize).clamp(0, w as isize - 1) as usize;
            let j = (((n - ts.origin_n) / ts.cell).round() as isize - j0 as isize).div_euclid(stride as isize).clamp(0, h as isize - 1) as usize;
            j * w + i
        };

        // Seconds per metre in each cell, and whether it is road.
        let (mi, ci) = (mode_ix(r.mode), class_ix(r.class));
        // The enemy at the destination is what the move closes with: only
        // threats elsewhere (a flank, a position on the way) are steered round.
        let keep_clear = p.threat_radius_m * p.objective_frac;
        let threats: Vec<(f64, f64)> = r.threats.iter().copied().filter(|t| (t.0 - r.to.e).hypot(t.1 - r.to.n) > keep_clear).collect();
        let march = r.mode == MoveModeS::March;
        let r2 = p.threat_radius_m * p.threat_radius_m;
        let mut spm = vec![f64::INFINITY; w * h];
        let mut on_road = vec![false; w * h];
        let mut best_spm = f64::INFINITY;
        // Exposure as seconds per metre at the reference pace.
        let per_m = 3.6 / p.exposure_ref_kmh.max(0.1);
        for (k, (cost, is_road)) in spm.iter_mut().zip(on_road.iter_mut()).enumerate() {
            let (e, n) = pos(k);
            // How near the nearest believed threat is: 0 outside its radius, 1 on it.
            let near = if threats.is_empty() {
                0.0
            } else {
                let d2 = threats.iter().map(|t| (t.0 - e).powi(2) + (t.1 - n).powi(2)).fold(f64::INFINITY, f64::min);
                if d2 < r2 {
                    1.0 - d2.sqrt() / p.threat_radius_m
                } else {
                    0.0
                }
            };
            // Exposure weight here, per second spent in the open.
            let base = if r.urgent { 0.0 } else { p.exposure_baseline[mi] };
            let exposure = p.exposure[mi] * (base + (1.0 - base) * near);
            let off = self.going(e, n, r.class).speed_kmh.min(r.cross_kmh);
            let road_kmh = self.road_kmh(e, n, r.class).map(|v| v.min(r.road_kmh));
            let open = if off >= 1.0 {
                let conceal = self.cover.props[self.land_cover(e, n) as usize].conceal_mean;
                let m = if march { p.march_open[ci] } else { 1.0 };
                let t = 3.6 / off;
                Some(t * m + exposure * (1.0 - conceal) * t.max(per_m))
            } else {
                None
            };
            let road = road_kmh.filter(|v| *v >= 1.0).map(|v| {
                let m = p.road_mult[mi] * if march { p.march_road[ci] } else { 1.0 };
                let t = 3.6 / v;
                // A road is open ground (no concealment), and predictable near the enemy.
                t * m + (exposure + p.threat_road[mi] * near) * t.max(per_m)
            });
            match (road, open) {
                (Some(rv), Some(ov)) if ov < rv => *cost = ov,
                (Some(rv), _) => {
                    *cost = rv;
                    *is_road = true;
                }
                (None, Some(ov)) => *cost = ov,
                (None, None) => {}
            }
            best_spm = best_spm.min(*cost);
        }
        let (start, goal) = (cell_of(r.from.e, r.from.n), cell_of(r.to.e, r.to.n));
        // Where the ends sit in water or a wall, let them out.
        spm[start] = spm[start].min(1e3);
        spm[goal] = spm[goal].min(1e3);
        if !best_spm.is_finite() {
            return None;
        }

        // A* over eight neighbours; a step costs the mean of its two cells,
        // plus the verge when it joins or leaves a road.
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        let (gx, gy) = ((goal % w) as f64, (goal / w) as f64);
        let heur = |k: usize| ((k % w) as f64 - gx).hypot((k / w) as f64 - gy) * step * best_spm;
        let mut g = vec![f64::INFINITY; w * h];
        let mut came = vec![u32::MAX; w * h];
        let mut heap = BinaryHeap::new();
        g[start] = 0.0;
        heap.push(Reverse((heur(start).to_bits(), start)));
        while let Some(Reverse((_, k))) = heap.pop() {
            if k == goal {
                break;
            }
            let (x, y) = ((k % w) as isize, (k / w) as isize);
            for (d, &(dx, dy)) in DIRS.iter().enumerate() {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let nk = ny as usize * w + nx as usize;
                if !spm[nk].is_finite() {
                    continue;
                }
                // No cutting the corner of water or a wall.
                if d >= 4 && (!spm[y as usize * w + nx as usize].is_finite() || !spm[ny as usize * w + x as usize].is_finite()) {
                    continue;
                }
                let len = if d >= 4 { step * std::f64::consts::SQRT_2 } else { step };
                let mut c = g[k] + len * 0.5 * (spm[k] + spm[nk]);
                if on_road[k] != on_road[nk] {
                    c += p.verge_s;
                }
                if c < g[nk] {
                    g[nk] = c;
                    came[nk] = k as u32;
                    heap.push(Reverse(((c + heur(nk)).to_bits(), nk)));
                }
            }
        }
        if !g[goal].is_finite() {
            return None;
        }
        let mut cells_path = vec![goal];
        while let Some(&k) = cells_path.last() {
            if k == start {
                break;
            }
            cells_path.push(came[k] as usize);
        }
        cells_path.reverse();

        // Waypoints: corners and road joins, with straight runs merged; then
        // the time and the mix at the vehicle's own speeds.
        let pts: Vec<(f64, f64, bool)> = cells_path.iter().map(|&k| {
            let (e, n) = pos(k);
            (e, n, on_road[k])
        })
        .collect();
        let mut keep = vec![false; pts.len()];
        keep[0] = true;
        *keep.last_mut().expect("a path") = true;
        for i in 1..pts.len() - 1 {
            keep[i] = pts[i].2 != pts[i - 1].2 || pts[i].2 != pts[i + 1].2;
        }
        // Off the road a straight line is as good as the staircase: simplify
        // each run between kept points, keeping road bends to within a cell.
        let mut out: Vec<usize> = Vec::new();
        let mut a = 0;
        for b in 1..pts.len() {
            if keep[b] {
                simplify(&pts, a, b, if pts[a].2 && pts[b].2 { ts.cell * 0.5 } else { step * 1.5 }, &mut out);
                a = b;
            }
        }
        out.push(pts.len() - 1);
        out.dedup();
        let mut waypoints: Vec<Enu> = out.iter().skip(1).map(|&i| Enu::new(pts[i].0, pts[i].1, 0.0)).collect();
        if let Some(last) = waypoints.last_mut() {
            *last = r.to;
        }
        let (mut road_m, mut open_m, mut time_s) = (0.0, 0.0, 0.0);
        for wnd in cells_path.windows(2) {
            let (pa, pb) = (pos(wnd[0]), pos(wnd[1]));
            let len = (pa.0 - pb.0).hypot(pa.1 - pb.1);
            let (e, n) = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
            let off = self.going(e, n, r.class).speed_kmh.min(r.cross_kmh).max(1.0);
            let kmh = if on_road[wnd[1]] {
                road_m += len;
                self.road_kmh(pb.0, pb.1, r.class).map(|v| v.min(r.road_kmh)).unwrap_or(off).max(off)
            } else {
                open_m += len;
                off
            };
            time_s += len / (kmh / 3.6);
        }
        Some(Plan { waypoints, time_s, road_m, open_m })
    }
}

/// Ramer–Douglas–Peucker between `a` and `b`, pushing the kept indices (from `a`).
fn simplify(pts: &[(f64, f64, bool)], a: usize, b: usize, tol: f64, out: &mut Vec<usize>) {
    out.push(a);
    if b <= a + 1 {
        return;
    }
    let (ax, ay, bx, by) = (pts[a].0, pts[a].1, pts[b].0, pts[b].1);
    let (dx, dy) = (bx - ax, by - ay);
    let len = dx.hypot(dy).max(1e-9);
    let (mut far, mut at) = (0.0, a);
    for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
        let d = ((p.0 - ax) * dy - (p.1 - ay) * dx).abs() / len;
        if d > far {
            far = d;
            at = i;
        }
    }
    if far > tol {
        simplify(pts, a, at, tol, out);
        simplify(pts, at, b, tol, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Geodetic;
    use bgws_env::{GridSpec, LandCover, RoadClass, RoadLine, Terrain};

    /// Open fields 9 km square with an L of main road along the south and east sides.
    fn ground() -> Ground {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        let road = RoadLine { class: RoadClass::Primary, bridge: false, line: vec![(-4_400.0, -3_000.0), (4_000.0, -3_000.0), (4_000.0, 4_400.0)] };
        Ground::from_terrain(t, 1, &f).with_roads(&[road])
    }

    /// The same fields with the road as a U: east, north, then back west (9.6 km round, 6 km across).
    fn u_road() -> Ground {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        let road = RoadLine { class: RoadClass::Primary, bridge: false, line: vec![(-4_000.0, -3_000.0), (-2_200.0, -3_000.0), (-2_200.0, 3_000.0), (-4_000.0, 3_000.0)] };
        Ground::from_terrain(t, 1, &f).with_roads(&[road])
    }

    /// Open fields with a belt of light wood down the west side and along the top.
    fn wooded_edges() -> Ground {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let mut t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        for k in 0..spec.len() {
            let (e, n) = spec.pos(spec.ij(k).0, spec.ij(k).1);
            t.land_cover.data[k] = if e < -3_300.0 || n > 3_300.0 { LandCover::ForestLight } else { LandCover::Open };
        }
        Ground::from_terrain(t, 1, &f)
    }

    fn plan_from(g: &Ground, from: (f64, f64), to: (f64, f64), class: VehicleClass, mode: MoveModeS, threats: &[(f64, f64)]) -> Plan {
        let (road_kmh, cross_kmh) = match class {
            VehicleClass::Wheeled => (90.0, 40.0),
            _ => (60.0, 40.0),
        };
        let r = RouteRequest { from: Enu::new(from.0, from.1, 0.0), to: Enu::new(to.0, to.1, 0.0), class, mode, road_kmh, cross_kmh, threats, urgent: false };
        g.plan(&r, &g.route_params)
    }

    fn plan(g: &Ground, class: VehicleClass, mode: MoveModeS, threats: &[(f64, f64)]) -> Plan {
        plan_from(g, (-4_000.0, -3_000.0), (4_000.0, 3_000.0), class, mode, threats)
    }

    fn road_share(p: &Plan) -> f64 {
        p.road_m / (p.road_m + p.open_m)
    }

    #[test]
    fn a_march_keeps_wheels_on_the_road_and_lets_tracks_cut_across() {
        // From one end of the U to the other: 9.6 km by road, 6 km straight.
        let g = u_road();
        let (a, b) = ((-4_000.0, -3_000.0), (-4_000.0, 3_000.0));
        let wheeled = plan_from(&g, a, b, VehicleClass::Wheeled, MoveModeS::March, &[]);
        let tracked = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::March, &[]);
        assert!(road_share(&wheeled) > 0.9, "wheeled keep to the road: {:.2}", road_share(&wheeled));
        assert!(road_share(&tracked) < 0.2, "tracked cut across: {:.2}", road_share(&tracked));
        // On the L, where the road is barely longer, tracks take it too.
        let l = plan(&ground(), VehicleClass::Tracked, MoveModeS::March, &[]);
        assert!(road_share(&l) > 0.8, "tracked take a road that is quicker: {:.2}", road_share(&l));
        assert!(wheeled.time_s > 0.0);
        // Waypoints at the corners, not a staircase.
        assert!(wheeled.waypoints.len() < 12, "{} waypoints", wheeled.waypoints.len());
    }

    #[test]
    fn a_tactical_move_leaves_the_road_near_a_believed_threat() {
        // Along the L's southern leg, start to end on the road.
        let g = ground();
        let (a, b) = ((-4_000.0, -3_000.0), (3_500.0, -3_000.0));
        let quiet = plan_from(&g, a, b, VehicleClass::Wheeled, MoveModeS::Tactical, &[]);
        assert!(road_share(&quiet) > 0.9, "with no enemy about, a tactical move takes the road: {:.2}", road_share(&quiet));
        // An enemy believed just north of the road's middle.
        let threat = [(0.0, -2_200.0)];
        let wary = plan_from(&g, a, b, VehicleClass::Wheeled, MoveModeS::Tactical, &threat);
        assert!(road_share(&wary) < road_share(&quiet) - 0.3, "it leaves the road near the threat: {:.2}", road_share(&wary));
        // It goes round on the far side, away from the threat.
        assert!(wary.waypoints.iter().all(|w| w.n < -2_400.0), "{:?}", wary.waypoints);
        // A march weighs the same threat less; an assault not at all.
        let march = plan_from(&g, a, b, VehicleClass::Wheeled, MoveModeS::March, &threat);
        let assault = plan_from(&g, a, b, VehicleClass::Wheeled, MoveModeS::Assault, &threat);
        assert!(road_share(&march) > road_share(&wary));
        assert!(road_share(&assault) > 0.9);
    }

    #[test]
    fn stealth_takes_the_woods_and_an_assault_goes_straight() {
        let g = wooded_edges();
        let (a, b) = ((-3_900.0, -3_900.0), (3_900.0, 3_900.0));
        let woods = |p: &Plan| {
            let mut from = Enu::new(a.0, a.1, 0.0);
            let mut inside = 0.0;
            for w in &p.waypoints {
                let n = 40;
                for k in 0..n {
                    let t = (k as f64 + 0.5) / n as f64;
                    if g.land_cover(from.e + (w.e - from.e) * t, from.n + (w.n - from.n) * t).is_forest() {
                        inside += w.sub(from).horizontal() / n as f64;
                    }
                }
                from = *w;
            }
            inside
        };
        let stealth = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Stealth, &[]);
        let assault = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Assault, &[]);
        let tactical = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Tactical, &[]);
        assert!(woods(&stealth) > 8_000.0, "stealth keeps to the woods: {:.0} m", woods(&stealth));
        assert!(woods(&assault) < 1_500.0, "the assault does not: {:.0} m", woods(&assault));
        let length = assault.road_m + assault.open_m;
        assert!(length < 11_000.0 * 1.1, "the assault is near straight: {length:.0} m");
        assert!(woods(&tactical) < woods(&stealth), "tactical weighs cover less than stealth");
    }

    #[test]
    fn ends_off_the_ground_go_straight() {
        let g = ground();
        let r = RouteRequest { from: Enu::new(0.0, 0.0, 0.0), to: Enu::new(50_000.0, 0.0, 0.0), class: VehicleClass::Tracked, mode: MoveModeS::Tactical, road_kmh: 60.0, cross_kmh: 40.0, threats: &[], urgent: false };
        assert_eq!(g.plan(&r, &g.route_params).waypoints, vec![Enu::new(50_000.0, 0.0, 0.0)]);
    }
}
