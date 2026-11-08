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
use bgws_env::RoadClass;
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
    /// Time to ford a river the vehicle can cross, s.
    pub ford_s: f64,
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
        figure!("sim.route.ford_s", 90.0, "s", 0.0, 1_800.0, "default", Default, "time to ford a river shallow enough to cross (banks, entry and exit)"),
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
            ford_s: g("ford_s"),
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
        // whose bridge lies outside it). When none does (the end is in ground
        // the vehicle cannot enter, or across water with no way over), the
        // route goes as near as it can: the wood's edge, the near bank.
        let mut margin = p.corridor_min_m.max(p.corridor_frac * dist);
        let mut closest: Option<Plan> = None;
        for _ in 0..3 {
            match self.plan_in(r, p, margin) {
                Some((plan, true)) => return plan,
                Some((plan, false)) => closest = Some(plan),
                None => {}
            }
            margin *= 3.0;
        }
        closest.unwrap_or_else(|| straight(0.0))
    }

    /// The plan in one corridor, and whether it reaches the destination.
    fn plan_in(&self, r: &RouteRequest, p: &RouteParams, margin: f64) -> Option<(Plan, bool)> {
        let ts = self.terrain.spec;
        // The corridor, in terrain cells, thinned if it would be too big.
        let lo = (r.from.e.min(r.to.e) - margin, r.from.n.min(r.to.n) - margin);
        let hi = (r.from.e.max(r.to.e) + margin, r.from.n.max(r.to.n) + margin);
        let (i0, j0) = ts.nearest_clamped(lo.0, lo.1);
        let (i1, j1) = ts.nearest_clamped(hi.0, hi.1);
        let cells = (i1 - i0 + 1) * (j1 - j0 + 1);
        let stride = ((cells as f64 / p.max_cells.max(1) as f64).sqrt().ceil() as usize).max(1);
        let (w, h) = ((i1 - i0) / stride + 1, (j1 - j0) / stride + 1);
        let n_cells = w * h;
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
        let layer = self.road_layer.as_deref();
        let rivers = self.rivers.as_deref();

        // What each search cell holds, over its block of terrain cells: the
        // best ground road and structure, and the deepest river.
        let mut g_road: Vec<Option<(RoadClass, u32)>> = vec![None; n_cells];
        let mut d_road: Vec<Option<(RoadClass, u32)>> = vec![None; n_cells];
        let mut river = vec![0.0f32; n_cells];
        let mut d_end = vec![false; n_cells];
        for k in 0..n_cells {
            let (bi, bj) = (i0 + (k % w) * stride, j0 + (k / w) * stride);
            for dj in 0..stride {
                for di in 0..stride {
                    let (ti, tj) = (bi + di, bj + dj);
                    if ti >= ts.width || tj >= ts.height {
                        continue;
                    }
                    let tk = ts.idx(ti, tj);
                    if let Some(l) = layer {
                        if let Some(g) = l.ground(tk) {
                            if g_road[k].is_none_or(|c| (g.0 as u8) < (c.0 as u8)) {
                                g_road[k] = Some(g);
                            }
                        }
                        d_end[k] |= l.deck_end[tk];
                        if let Some(d) = l.deck(tk) {
                            if d_road[k].is_none_or(|c| (d.0 as u8) < (c.0 as u8)) {
                                d_road[k] = Some(d);
                            }
                        }
                    }
                    if let Some(rv) = rivers {
                        river[k] = river[k].max(rv.data[tk]);
                    }
                }
            }
        }

        // Seconds per metre on each level, and whether the ground cell is taken by road.
        let (mi, ci) = (mode_ix(r.mode), class_ix(r.class));
        // The enemy at the destination is what the move closes with: only
        // threats elsewhere (a flank, a position on the way) are steered round.
        let keep_clear = p.threat_radius_m * p.objective_frac;
        let threats: Vec<(f64, f64)> = r.threats.iter().copied().filter(|t| (t.0 - r.to.e).hypot(t.1 - r.to.n) > keep_clear).collect();
        let march = r.mode == MoveModeS::March;
        let r2 = p.threat_radius_m * p.threat_radius_m;
        let ford_limit = self.ford.depth_m[ci] as f32;
        // Exposure as seconds per metre at the reference pace.
        let per_m = 3.6 / p.exposure_ref_kmh.max(0.1);
        let mut g_spm = vec![f64::INFINITY; n_cells];
        let mut d_spm = vec![f64::INFINITY; n_cells];
        let mut on_road = vec![false; n_cells];
        let mut best_spm = f64::INFINITY;
        let road_cost = |class: RoadClass, near: f64, exposure: f64| -> Option<f64> {
            let v = self.road_speeds.speed(r.class, class).min(r.road_kmh);
            (v >= 1.0).then(|| {
                let m = p.road_mult[mi] * if march { p.march_road[ci] } else { 1.0 };
                let t = 3.6 / v;
                // A road is open ground (no concealment), and predictable near the enemy.
                t * m + (exposure + p.threat_road[mi] * near) * t.max(per_m)
            })
        };
        for k in 0..n_cells {
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
            // Across country, a river deeper than the vehicle fords stops it.
            let open = if off >= 1.0 && river[k] <= ford_limit {
                let conceal = self.cover.props[self.land_cover(e, n) as usize].conceal_mean;
                let m = if march { p.march_open[ci] } else { 1.0 };
                let t = 3.6 / off;
                Some(t * m + exposure * (1.0 - conceal) * t.max(per_m))
            } else {
                None
            };
            // A road on the ground crosses what it crosses (culverts, causeways, fords).
            let road = g_road[k].and_then(|(c, _)| road_cost(c, near, exposure));
            match (road, open) {
                (Some(rv), Some(ov)) if ov < rv => g_spm[k] = ov,
                (Some(rv), _) => {
                    g_spm[k] = rv;
                    on_road[k] = true;
                }
                (None, Some(ov)) => g_spm[k] = ov,
                (None, None) => {}
            }
            if let Some(dv) = d_road[k].and_then(|(c, _)| road_cost(c, near, exposure)) {
                d_spm[k] = dv;
            }
            best_spm = best_spm.min(g_spm[k]).min(d_spm[k]);
        }
        let (start, goal) = (cell_of(r.from.e, r.from.n), cell_of(r.to.e, r.to.n));
        // Where the ends sit in water or a wall, let them out.
        g_spm[start] = g_spm[start].min(1e3);
        g_spm[goal] = g_spm[goal].min(1e3);
        if !best_spm.is_finite() {
            return None;
        }

        // A* over two levels: state k is the ground of cell k, n_cells + k its
        // bridge or tunnel. A step costs the mean of its two cells, plus the
        // verge when it joins or leaves a road on the ground, plus a ford.
        // A structure is entered and left only along its road: from an edge
        // that joins it, never from the side.
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        let joined = |a: u32, b: u32| layer.is_some_and(|l| l.joined(a, b));
        let (gx, gy) = ((goal % w) as f64, (goal / w) as f64);
        let heur = |k: usize| ((k % w) as f64 - gx).hypot((k / w) as f64 - gy) * step * best_spm;
        let mut g = vec![f64::INFINITY; 2 * n_cells];
        let mut came = vec![u32::MAX; 2 * n_cells];
        let mut heap = BinaryHeap::new();
        let push = |heap: &mut BinaryHeap<Reverse<(u64, usize)>>, g: &mut Vec<f64>, came: &mut Vec<u32>, from: usize, to: usize, c: f64| {
            if c < g[to] {
                g[to] = c;
                came[to] = from as u32;
                heap.push(Reverse(((c + heur(to % n_cells)).to_bits(), to)));
            }
        };
        g[start] = 0.0;
        heap.push(Reverse((heur(start).to_bits(), start)));
        if d_spm[start].is_finite() {
            g[n_cells + start] = 0.0;
            heap.push(Reverse((heur(start).to_bits(), n_cells + start)));
        }
        let mut reached = None;
        // The nearest the search came, for when the destination cannot be reached.
        let mut nearest = (heur(start), start);
        while let Some(Reverse((_, st))) = heap.pop() {
            let (deck, k) = (st >= n_cells, st % n_cells);
            let gst = g[st];
            if k == goal {
                reached = Some(st);
                break;
            }
            let hk = heur(k);
            if hk < nearest.0 {
                nearest = (hk, st);
            }
            // Between the levels in the same cell: where the ground road joins the
            // structure, or there is no road on the ground (an abutment).
            if let Some((_, de)) = d_road[k] {
                if d_spm[k].is_finite() && g_spm[k].is_finite() && g_road[k].is_none_or(|(_, ge)| joined(ge, de)) && (d_end[k] || g_road[k].is_some()) {
                    let other = if deck { k } else { n_cells + k };
                    push(&mut heap, &mut g, &mut came, st, other, gst);
                }
            }
            let (x, y) = ((k % w) as isize, (k / w) as isize);
            for (d, &(dx, dy)) in DIRS.iter().enumerate() {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let nk = ny as usize * w + nx as usize;
                let len = if d >= 4 { step * std::f64::consts::SQRT_2 } else { step };
                let here = if deck { d_spm[k] } else { g_spm[k] };
                if deck {
                    let de = d_road[k].map(|x| x.1).unwrap_or(u32::MAX);
                    // Along the structure.
                    if d_spm[nk].is_finite() && d_road[nk].is_some_and(|(_, e2)| joined(de, e2)) {
                        push(&mut heap, &mut g, &mut came, st, n_cells + nk, gst + len * 0.5 * (here + d_spm[nk]));
                    }
                    // Off it onto a road that joins it, or onto open ground at its ends.
                    if g_spm[nk].is_finite() && (g_road[nk].is_some_and(|(_, e2)| joined(de, e2)) || (g_road[nk].is_none() && d_end[k])) {
                        push(&mut heap, &mut g, &mut came, st, nk, gst + len * 0.5 * (here + g_spm[nk]));
                    }
                    continue;
                }
                // On to a structure from a road that joins it, or from open ground at its ends.
                if d_spm[nk].is_finite() {
                    if let Some((_, de)) = d_road[nk] {
                        let from_road = on_road[k] && g_road[k].is_some_and(|(_, ge)| joined(ge, de));
                        if from_road || (g_road[k].is_none() && d_end[nk]) {
                            push(&mut heap, &mut g, &mut came, st, n_cells + nk, gst + len * 0.5 * (here + d_spm[nk]));
                        }
                    }
                }
                if !g_spm[nk].is_finite() {
                    continue;
                }
                // No cutting the corner of water or a wall.
                if d >= 4 && (!g_spm[y as usize * w + nx as usize].is_finite() || !g_spm[ny as usize * w + x as usize].is_finite()) {
                    continue;
                }
                let mut c = gst + len * 0.5 * (here + g_spm[nk]);
                if on_road[k] != on_road[nk] {
                    c += p.verge_s;
                }
                if !on_road[nk] && river[nk] > 0.0 && river[k] == 0.0 {
                    c += p.ford_s;
                }
                push(&mut heap, &mut g, &mut came, st, nk, c);
            }
        }
        let arrived = reached.is_some();
        let end = match reached {
            Some(st) => st,
            // Nowhere nearer than where it stands: no plan.
            None if nearest.1 == start => return None,
            None => nearest.1,
        };
        let mut states = vec![end];
        while let Some(&st) = states.last() {
            if st % n_cells == start && came[st] == u32::MAX {
                break;
            }
            let prev = came[st];
            if prev == u32::MAX {
                break;
            }
            states.push(prev as usize);
        }
        states.reverse();

        // Waypoints: corners, road joins and level changes, straight runs
        // merged where the straight line stays on passable ground; then the
        // time and the mix at the vehicle's own speeds.
        let pts: Vec<(f64, f64, u8)> = states
            .iter()
            .map(|&st| {
                let k = st % n_cells;
                let (e, n) = pos(k);
                (e, n, if st >= n_cells { 2 } else { u8::from(on_road[k]) })
            })
            .collect();
        let mut keep = vec![false; pts.len()];
        keep[0] = true;
        *keep.last_mut().expect("a path") = true;
        for i in 1..pts.len() - 1 {
            keep[i] = pts[i].2 != pts[i - 1].2 || pts[i].2 != pts[i + 1].2;
        }
        // A straight line across country is as good as the staircase, if it crosses nothing impassable.
        let clear = |a: (f64, f64), b: (f64, f64)| {
            let n = ((b.0 - a.0).hypot(b.1 - a.1) / (step * 0.5)).ceil().max(1.0) as usize;
            (0..=n).all(|q| {
                let t = q as f64 / n as f64;
                g_spm[cell_of(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)].is_finite()
            })
        };
        let mut out: Vec<usize> = Vec::new();
        let mut a = 0;
        for b in 1..pts.len() {
            if keep[b] {
                let tol = if pts[a].2 > 0 && pts[b].2 > 0 { ts.cell * 0.5 } else { step * 1.5 };
                simplify(&pts, a, b, tol, &|x, y| pts[x].2 > 0 || clear((pts[x].0, pts[x].1), (pts[y].0, pts[y].1)), &mut out);
                a = b;
            }
        }
        out.push(pts.len() - 1);
        out.dedup();
        let mut waypoints: Vec<Enu> = out.iter().skip(1).map(|&i| Enu::new(pts[i].0, pts[i].1, 0.0)).collect();
        if arrived {
            if let Some(last) = waypoints.last_mut() {
                *last = r.to;
            }
        }
        let (mut road_m, mut open_m, mut time_s) = (0.0, 0.0, 0.0);
        for wnd in states.windows(2) {
            let (pa, pb) = (pos(wnd[0] % n_cells), pos(wnd[1] % n_cells));
            let len = (pa.0 - pb.0).hypot(pa.1 - pb.1);
            let (e, n) = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
            let off = self.going(e, n, r.class).speed_kmh.min(r.cross_kmh).max(1.0);
            let kb = wnd[1] % n_cells;
            let on = if wnd[1] >= n_cells { d_road[kb] } else if on_road[kb] { g_road[kb] } else { None };
            let kmh = match on {
                Some((c, _)) => {
                    road_m += len;
                    self.road_speeds.speed(r.class, c).min(r.road_kmh).max(off)
                }
                None => {
                    open_m += len;
                    off
                }
            };
            time_s += len / (kmh / 3.6);
        }
        Some((Plan { waypoints, time_s, road_m, open_m }, arrived))
    }
}

/// Ramer–Douglas–Peucker between `a` and `b`, pushing the kept indices
/// (from `a`); a run whose straight line is not `valid` is split too.
fn simplify(pts: &[(f64, f64, u8)], a: usize, b: usize, tol: f64, valid: &dyn Fn(usize, usize) -> bool, out: &mut Vec<usize>) {
    out.push(a);
    if b <= a + 1 {
        return;
    }
    let (ax, ay, bx, by) = (pts[a].0, pts[a].1, pts[b].0, pts[b].1);
    let (dx, dy) = (bx - ax, by - ay);
    let len = dx.hypot(dy).max(1e-9);
    let (mut far, mut at) = (0.0, a + (b - a) / 2);
    for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
        let d = ((p.0 - ax) * dy - (p.1 - ay) * dx).abs() / len;
        if d > far {
            far = d;
            at = i;
        }
    }
    if far > tol || !valid(a, b) {
        simplify(pts, a, at, tol, valid, out);
        simplify(pts, at, b, tol, valid, out);
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
        let road = RoadLine::new(RoadClass::Primary, vec![(-4_400.0, -3_000.0), (4_000.0, -3_000.0), (4_000.0, 4_400.0)]);
        Ground::from_terrain(t, 1, &f).with_roads(&[road])
    }

    /// The same fields with the road as a U: east, north, then back west (9.6 km round, 6 km across).
    fn u_road() -> Ground {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        let road = RoadLine::new(RoadClass::Primary, vec![(-4_000.0, -3_000.0), (-2_200.0, -3_000.0), (-2_200.0, 3_000.0), (-4_000.0, 3_000.0)]);
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

    /// Fields cut east–west by a deep river (40 m wide, 2.5 m deep) with two road
    /// bridges, at e = −3 km and e = +3 km, and a shallow stream further south.
    fn river_ground() -> Ground {
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        let road = |edge: u32, from: u32, to: u32, bridge: bool, e: f64, n0: f64, n1: f64| RoadLine { class: RoadClass::Secondary, bridge, tunnel: false, edge, from, to, line: vec![(e, n0), (e, n1)] };
        let roads = vec![
            road(0, 0, 1, false, -3_000.0, -4_400.0, -60.0),
            road(1, 1, 2, true, -3_000.0, -60.0, 60.0),
            road(2, 2, 3, false, -3_000.0, 60.0, 4_400.0),
            road(3, 4, 5, false, 3_000.0, -4_400.0, -60.0),
            road(4, 5, 6, true, 3_000.0, -60.0, 60.0),
            road(5, 6, 7, false, 3_000.0, 60.0, 4_400.0),
        ];
        let river = bgws_env::River { name: "deep".into(), line: vec![(-4_500.0, 0.0), (4_500.0, 0.0)], width_m: 40.0, depth_m: 2.5, current_m_s: 0.5 };
        let stream = bgws_env::River { name: "stream".into(), line: vec![(-4_500.0, -2_500.0), (4_500.0, -2_500.0)], width_m: 3.0, depth_m: 0.6, current_m_s: 0.3 };
        Ground::from_terrain(t, 1, &f).with_roads(&roads).with_rivers(&[river, stream])
    }

    /// Where a route crosses n = 0, if it does.
    fn crossing(p: &Plan, from: (f64, f64)) -> Option<f64> {
        let mut a = Enu::new(from.0, from.1, 0.0);
        for w in &p.waypoints {
            if (a.n < 0.0) != (w.n < 0.0) {
                let t = -a.n / (w.n - a.n);
                return Some(a.e + (w.e - a.e) * t);
            }
            a = *w;
        }
        None
    }

    #[test]
    fn a_deep_river_is_crossed_at_a_bridge_and_a_blown_one_is_not() {
        let mut g = river_ground();
        let (a, b) = ((-1_000.0, -2_000.0), (-1_000.0, 2_000.0));
        let p = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Assault, &[]);
        let x = crossing(&p, a).expect("it crosses");
        assert!((x + 3_000.0).abs() < 100.0, "by the western bridge, nearest: crossed at e = {x:.0}");
        g.blow_bridge(1);
        let p = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Assault, &[]);
        let x = crossing(&p, a).expect("it still crosses");
        assert!((x - 3_000.0).abs() < 100.0, "by the eastern bridge once the western one is down: e = {x:.0}");
        g.blow_bridge(4);
        let p = plan_from(&g, a, b, VehicleClass::Tracked, MoveModeS::Assault, &[]);
        let end = p.waypoints.last().unwrap();
        assert!(crossing(&p, a).is_none() && end.n < 0.0 && end.n > -150.0, "no way over: it goes to the near bank and stops, at {end:?}");
    }

    #[test]
    fn a_shallow_stream_is_forded_where_the_river_is_not() {
        let g = river_ground();
        // South of the river, across the stream (0.6 m): every class fords it, straight on.
        let (a, b) = ((0.0, -3_500.0), (0.0, -1_500.0));
        for class in [VehicleClass::Tracked, VehicleClass::Wheeled, VehicleClass::Foot] {
            let p = plan_from(&g, a, b, class, MoveModeS::Assault, &[]);
            let length = p.road_m + p.open_m;
            assert!(length < 2_300.0, "{class:?} fords the stream: {length:.0} m");
        }
        assert!(g.river_depth(0.0, -2_500.0) > 0.5 && g.river_depth(0.0, 0.0) > 2.0 && g.river_depth(0.0, -1_000.0) == 0.0);
    }

    #[test]
    fn an_overpass_is_not_a_junction() {
        // Water everywhere but the roads: a motorway east–west on the ground, and
        // a minor road north–south crossing over it on a bridge.
        let f = crate::params::default_figures();
        let spec = GridSpec::new(301, 301, 30.0, -4_500.0, -4_500.0);
        let mut t = Terrain::flat(Geodetic::new(54.7, 20.5, 0.0), spec, 0.0);
        t.land_cover.data.iter_mut().for_each(|c| *c = LandCover::Water);
        let road = |class, edge, from, to, bridge, line: Vec<(f64, f64)>| RoadLine { class, bridge, tunnel: false, edge, from, to, line };
        let g = Ground::from_terrain(t, 1, &f).with_roads(&[
            road(RoadClass::Motorway, 0, 0, 1, false, vec![(-4_000.0, 0.0), (4_000.0, 0.0)]),
            road(RoadClass::Secondary, 1, 10, 11, false, vec![(0.0, -4_000.0), (0.0, -90.0)]),
            road(RoadClass::Secondary, 2, 11, 12, true, vec![(0.0, -90.0), (0.0, 90.0)]),
            road(RoadClass::Secondary, 3, 12, 13, false, vec![(0.0, 90.0), (0.0, 4_000.0)]),
        ]);
        // Along the minor road, over the bridge: fine.
        let p = plan_from(&g, (0.0, -3_800.0), (0.0, 3_800.0), VehicleClass::Wheeled, MoveModeS::March, &[]);
        assert!(p.road_m > 7_000.0 && p.waypoints.len() > 1, "{p:?}");
        // From the motorway onto the minor road: they do not meet.
        let p = plan_from(&g, (-3_800.0, 0.0), (0.0, 3_800.0), VehicleClass::Wheeled, MoveModeS::March, &[]);
        assert!(p.waypoints.iter().all(|w| w.n.abs() < 100.0), "no junction at the overpass: it stays on the motorway, {p:?}");
        // Along the motorway under the bridge: fine.
        let p = plan_from(&g, (-3_800.0, 0.0), (3_800.0, 0.0), VehicleClass::Wheeled, MoveModeS::March, &[]);
        assert!(p.road_m > 7_000.0, "{p:?}");
    }

    #[test]
    fn ends_off_the_ground_go_straight() {
        let g = ground();
        let r = RouteRequest { from: Enu::new(0.0, 0.0, 0.0), to: Enu::new(50_000.0, 0.0, 0.0), class: VehicleClass::Tracked, mode: MoveModeS::Tactical, road_kmh: 60.0, cross_kmh: 40.0, threats: &[], urgent: false };
        assert_eq!(g.plan(&r, &g.route_params).waypoints, vec![Enu::new(50_000.0, 0.0, 0.0)]);
    }
}
