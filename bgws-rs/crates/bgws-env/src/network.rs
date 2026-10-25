//! Networks (§7.1): the road graph with classes and bridges (load class),
//! rivers with width and depth for fording, power lines (a rotary-wing
//! hazard), A* routing on roads, and a cost-raster A* for tactical
//! cross-country movement whose cost can carry a caller-supplied exposure
//! term (e.g. a viewshed count from `bgws-gpu`), so a route can trade
//! distance against being seen [R4.1].
//!
//! Graphs are `petgraph` with stable insertion order; ties in the searches
//! break by node index, so routes are deterministic.

use crate::raster::{GridSpec, Raster};
use crate::soil::VehicleClass;
use bgws_core::{figure, Figure, Figures};
use petgraph::graph::{EdgeIndex, NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoadClass {
    Motorway,
    Primary,
    Secondary,
    Track,
}

impl RoadClass {
    pub const ALL: [RoadClass; 4] = [RoadClass::Motorway, RoadClass::Primary, RoadClass::Secondary, RoadClass::Track];
    pub fn key(self) -> &'static str {
        match self {
            RoadClass::Motorway => "motorway",
            RoadClass::Primary => "primary",
            RoadClass::Secondary => "secondary",
            RoadClass::Track => "track",
        }
    }
}

/// A bridge on a road edge.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    /// Military load class (MLC) the bridge carries.
    pub load_class: u16,
    pub span_m: f64,
    pub blown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadNode {
    pub e: f64,
    pub n: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadEdge {
    pub class: RoadClass,
    pub length_m: f64,
    pub bridge: Option<Bridge>,
    /// Cratered or blocked: impassable until repaired.
    pub blocked: bool,
}

/// The road network.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoadNetwork {
    pub graph: UnGraph<RoadNode, RoadEdge>,
}

/// Who is routing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadUser {
    pub class: VehicleClass,
    /// Vehicle MLC; bridges rated below it are refused.
    pub load_class: u16,
}

/// Road speeds by class, from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoadSpeeds {
    /// km/h by [vehicle class][road class].
    pub kmh: [[f64; 4]; 3],
}

impl RoadSpeeds {
    pub fn from_figures(f: &Figures) -> Self {
        let kmh = VehicleClass::ALL.map(|v| RoadClass::ALL.map(|r| f.get(&format!("env.road.{}.{}_kmh", v.key(), r.key()))));
        Self { kmh }
    }
    pub fn speed(&self, v: VehicleClass, r: RoadClass) -> f64 {
        let vi = match v {
            VehicleClass::Tracked => 0,
            VehicleClass::Wheeled => 1,
            VehicleClass::Foot => 2,
        };
        self.kmh[vi][r as usize]
    }
    fn max(&self, v: VehicleClass) -> f64 {
        RoadClass::ALL.iter().map(|&r| self.speed(v, r)).fold(0.0, f64::max)
    }
}

/// A road route with its cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub nodes: Vec<usize>,
    pub edges: Vec<usize>,
    pub length_m: f64,
    pub time_s: f64,
}

impl RoadNetwork {
    pub fn add_node(&mut self, e: f64, n: f64) -> NodeIndex {
        self.graph.add_node(RoadNode { e, n })
    }

    /// Add a road between two nodes; length is the straight distance.
    pub fn add_road(&mut self, a: NodeIndex, b: NodeIndex, class: RoadClass, bridge: Option<Bridge>) -> EdgeIndex {
        let (pa, pb) = (self.graph[a], self.graph[b]);
        let length_m = (pa.e - pb.e).hypot(pa.n - pb.n);
        self.graph.add_edge(a, b, RoadEdge { class, length_m, bridge, blocked: false })
    }

    /// The nearest node to a point (lowest index on ties).
    pub fn nearest_node(&self, e: f64, n: f64) -> Option<NodeIndex> {
        self.graph
            .node_indices()
            .map(|i| (i, (self.graph[i].e - e).hypot(self.graph[i].n - n)))
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(i, _)| i)
    }

    /// Whether `u` can use edge `ed`.
    pub fn passable(&self, ed: &RoadEdge, u: &RoadUser) -> bool {
        if ed.blocked {
            return false;
        }
        match ed.bridge {
            Some(b) if b.blown => false,
            Some(b) => u.class == VehicleClass::Foot || b.load_class >= u.load_class,
            None => true,
        }
    }

    /// Fastest route by A* (time cost, straight-line / top-speed heuristic).
    pub fn route(&self, from: NodeIndex, to: NodeIndex, u: &RoadUser, speeds: &RoadSpeeds) -> Option<Route> {
        let vmax = speeds.max(u.class).max(1e-6) / 3.6;
        let goal = self.graph[to];
        let (cost, path) = petgraph::algo::astar(
            &self.graph,
            from,
            |n| n == to,
            |e| {
                let ed = e.weight();
                if !self.passable(ed, u) {
                    return f64::INFINITY;
                }
                let v = speeds.speed(u.class, ed.class) / 3.6;
                if v <= 0.0 {
                    f64::INFINITY
                } else {
                    ed.length_m / v
                }
            },
            |n| {
                let p = self.graph[n];
                (p.e - goal.e).hypot(p.n - goal.n) / vmax
            },
        )?;
        if !cost.is_finite() {
            return None;
        }
        let mut edges = Vec::new();
        let mut length = 0.0;
        for w in path.windows(2) {
            // The cheapest passable edge between consecutive nodes.
            let e = self
                .graph
                .edges_connecting(w[0], w[1])
                .filter(|e| self.passable(e.weight(), u))
                .min_by(|a, b| a.weight().length_m.total_cmp(&b.weight().length_m).then(a.id().cmp(&b.id())))?;
            length += e.weight().length_m;
            edges.push(e.id().index());
        }
        Some(Route { nodes: path.iter().map(|n| n.index()).collect(), edges, length_m: length, time_s: cost })
    }
}

/// A river reach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct River {
    pub name: String,
    /// Centre line, ENU metres.
    pub line: Vec<(f64, f64)>,
    pub width_m: f64,
    pub depth_m: f64,
    pub current_m_s: f64,
}

/// Fording limits by class, from figures.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FordLimits {
    pub depth_m: [f64; 3],
    pub current_m_s: f64,
}

impl FordLimits {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            depth_m: VehicleClass::ALL.map(|v| f.get(&format!("env.ford.{}.depth_m", v.key()))),
            current_m_s: f.get("env.ford.max_current_m_s"),
        }
    }
}

/// Why a crossing can or can't be forded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fording {
    pub fordable: bool,
    pub depth_m: f64,
    pub limit_m: f64,
    pub current_m_s: f64,
}

impl River {
    pub fn ford(&self, v: VehicleClass, lim: &FordLimits) -> Fording {
        let vi = match v {
            VehicleClass::Tracked => 0,
            VehicleClass::Wheeled => 1,
            VehicleClass::Foot => 2,
        };
        Fording {
            fordable: self.depth_m <= lim.depth_m[vi] && self.current_m_s <= lim.current_m_s,
            depth_m: self.depth_m,
            limit_m: lim.depth_m[vi],
            current_m_s: self.current_m_s,
        }
    }

    /// Distance from a point to the centre line.
    pub fn distance_to(&self, e: f64, n: f64) -> f64 {
        polyline_distance(&self.line, e, n)
    }
}

/// A power line: a wire hazard for rotary-wing flight below its height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PowerLine {
    pub line: Vec<(f64, f64)>,
    pub height_m: f64,
}

impl PowerLine {
    /// Whether flying at `agl` within `buffer` metres of the line is a wire strike risk.
    pub fn hazard(&self, e: f64, n: f64, agl: f64, buffer: f64) -> bool {
        agl <= self.height_m + buffer && polyline_distance(&self.line, e, n) <= buffer
    }
}

/// Distance from a point to a polyline.
pub fn polyline_distance(line: &[(f64, f64)], e: f64, n: f64) -> f64 {
    if line.len() == 1 {
        return (line[0].0 - e).hypot(line[0].1 - n);
    }
    line.windows(2)
        .map(|w| {
            let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
            let (dx, dy) = (bx - ax, by - ay);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 { (((e - ax) * dx + (n - ay) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            (ax + t * dx - e).hypot(ay + t * dy - n)
        })
        .fold(f64::INFINITY, f64::min)
}

/// A cross-country path on a cost raster.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridPath {
    /// Node indices from start to goal.
    pub cells: Vec<usize>,
    pub cost: f64,
    pub length_m: f64,
    /// The exposure part of the cost.
    pub exposure_cost: f64,
}

#[derive(PartialEq)]
struct Open(f64, usize);
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

/// Cost-raster A*. `cost` is seconds (or any unit) per metre at each node —
/// e.g. `1 / speed` from trafficability — with ≤ 0 or non-finite meaning
/// impassable. `exposure` (optional) adds `exposure_weight × exposure` per
/// metre, so a route can buy concealment with distance.
pub fn cost_astar(cost: &Raster<f64>, exposure: Option<(&Raster<f64>, f64)>, start: usize, goal: usize) -> Option<GridPath> {
    let s: GridSpec = cost.spec;
    let n = s.len();
    if start >= n || goal >= n {
        return None;
    }
    let ok = |k: usize| cost.data[k].is_finite() && cost.data[k] > 0.0;
    if !ok(start) || !ok(goal) {
        return None;
    }
    let min_cost = cost.data.iter().copied().filter(|c| c.is_finite() && *c > 0.0).fold(f64::INFINITY, f64::min);
    let per_m = |k: usize| -> (f64, f64) {
        let x = exposure.map(|(r, w)| w * r.data[k].max(0.0)).unwrap_or(0.0);
        (cost.data[k] + x, x)
    };
    let (gi, gj) = s.ij(goal);
    let h = |k: usize| {
        let (i, j) = s.ij(k);
        let (di, dj) = ((i as f64 - gi as f64).abs(), (j as f64 - gj as f64).abs());
        // Octile distance × cheapest cost: admissible.
        s.cell * ((di.max(dj) - di.min(dj)) + std::f64::consts::SQRT_2 * di.min(dj)) * min_cost
    };
    let mut g = vec![f64::INFINITY; n];
    let mut came = vec![usize::MAX; n];
    let mut heap = BinaryHeap::new();
    g[start] = 0.0;
    heap.push(Open(h(start), start));
    const D: [(i64, i64); 8] = [(0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1)];
    while let Some(Open(f, k)) = heap.pop() {
        if k == goal {
            break;
        }
        if f > g[k] + h(k) + 1e-9 {
            continue;
        }
        let (i, j) = s.ij(k);
        for (di, dj) in D {
            let (ni, nj) = (i as i64 + di, j as i64 + dj);
            if ni < 0 || nj < 0 || ni >= s.width as i64 || nj >= s.height as i64 {
                continue;
            }
            let nk = s.idx(ni as usize, nj as usize);
            if !ok(nk) {
                continue;
            }
            let len = if di != 0 && dj != 0 { s.cell * std::f64::consts::SQRT_2 } else { s.cell };
            let nd = g[k] + len * 0.5 * (per_m(k).0 + per_m(nk).0);
            if nd < g[nk] {
                g[nk] = nd;
                came[nk] = k;
                heap.push(Open(nd + h(nk), nk));
            }
        }
    }
    if !g[goal].is_finite() {
        return None;
    }
    let mut cells = vec![goal];
    while *cells.last().expect("non-empty") != start {
        cells.push(came[*cells.last().expect("non-empty")]);
    }
    cells.reverse();
    let (mut length, mut expo) = (0.0, 0.0);
    for w in cells.windows(2) {
        let ((a, b), (c, d)) = (s.ij(w[0]), s.ij(w[1]));
        let len = if a != c && b != d { s.cell * std::f64::consts::SQRT_2 } else { s.cell };
        length += len;
        expo += len * 0.5 * (per_m(w[0]).1 + per_m(w[1]).1);
    }
    Some(GridPath { cells, cost: g[goal], length_m: length, exposure_cost: expo })
}

pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    // km/h: motorway, primary, secondary, track.
    let speeds = [
        (VehicleClass::Tracked, [50.0, 45.0, 35.0, 25.0]),
        (VehicleClass::Wheeled, [90.0, 70.0, 50.0, 25.0]),
        (VehicleClass::Foot, [4.0, 4.0, 4.0, 3.5]),
    ];
    for (c, s) in speeds {
        for (r, kmh) in RoadClass::ALL.iter().zip(s) {
            let (src, note) = if c == VehicleClass::Foot { ("P26", "dismounted road march (P26: ~4 km/h on road)") } else { ("default", "column road speed") };
            v.push(figure!(format!("env.road.{}.{}_kmh", c.key(), r.key()), kmh, "km/h", 0.5, 150.0, src, Default, note));
        }
    }
    for (c, d) in VehicleClass::ALL.iter().zip([1.2, 0.75, 0.9]) {
        v.push(figure!(format!("env.ford.{}.depth_m", c.key()), d, "m", 0.1, 5.0, "default", Default, "deepest unprepared ford"));
    }
    v.push(figure!("env.ford.max_current_m_s", 1.5, "m/s", 0.1, 5.0, "default", Default, "fastest current that can be forded"));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speeds() -> RoadSpeeds {
        RoadSpeeds::from_figures(&Figures::from_list(figures()).unwrap())
    }

    #[test]
    fn route_avoids_weak_bridge_for_heavy_vehicles() {
        let mut net = RoadNetwork::default();
        let a = net.add_node(0.0, 0.0);
        let b = net.add_node(1000.0, 0.0);
        let c = net.add_node(2000.0, 0.0);
        let d = net.add_node(1000.0, 3000.0);
        net.add_road(a, b, RoadClass::Primary, None);
        net.add_road(b, c, RoadClass::Primary, Some(Bridge { load_class: 30, span_m: 40.0, blown: false }));
        net.add_road(a, d, RoadClass::Secondary, None);
        net.add_road(d, c, RoadClass::Secondary, None);
        let s = speeds();
        let car = RoadUser { class: VehicleClass::Wheeled, load_class: 20 };
        let tank = RoadUser { class: VehicleClass::Tracked, load_class: 70 };
        let r1 = net.route(a, c, &car, &s).unwrap();
        assert_eq!(r1.nodes, vec![0, 1, 2]);
        assert!((r1.length_m - 2000.0).abs() < 1e-9);
        let r2 = net.route(a, c, &tank, &s).unwrap();
        assert_eq!(r2.nodes, vec![0, 3, 2]);
        // Blow the detour too: no route.
        let e = net.graph.find_edge(d, c).unwrap();
        net.graph[e].bridge = Some(Bridge { load_class: 100, span_m: 10.0, blown: true });
        assert!(net.route(a, c, &tank, &s).is_none());
        assert_eq!(net.nearest_node(990.0, 2900.0), Some(d));
    }

    #[test]
    fn cost_astar_trades_distance_for_concealment() {
        let spec = GridSpec::new(30, 30, 10.0, 0.0, 0.0);
        let cost = Raster::filled(spec, 1.0);
        // A band of exposure across the direct line.
        let expo = Raster::from_fn(spec, |i, j| if (10..20).contains(&i) && j < 25 { 1.0 } else { 0.0 });
        let start = spec.idx(2, 5);
        let goal = spec.idx(27, 5);
        let direct = cost_astar(&cost, None, start, goal).unwrap();
        assert!((direct.length_m - 250.0).abs() < 1e-9);
        let hidden = cost_astar(&cost, Some((&expo, 50.0)), start, goal).unwrap();
        assert!(hidden.length_m > direct.length_m);
        assert!(hidden.exposure_cost < 50.0 * 100.0);
        assert!(hidden.cells.iter().all(|&k| {
            let (i, j) = spec.ij(k);
            !(10..20).contains(&i) || j >= 25
        }));
    }

    #[test]
    fn fording_and_wires() {
        let f = Figures::from_list(figures()).unwrap();
        let lim = FordLimits::from_figures(&f);
        let r = River { name: "x".into(), line: vec![(0.0, 0.0), (100.0, 0.0)], width_m: 30.0, depth_m: 1.0, current_m_s: 0.5 };
        assert!(r.ford(VehicleClass::Tracked, &lim).fordable);
        assert!(!r.ford(VehicleClass::Wheeled, &lim).fordable);
        assert!((r.distance_to(50.0, 7.0) - 7.0).abs() < 1e-12);
        let p = PowerLine { line: vec![(0.0, 0.0), (0.0, 1000.0)], height_m: 25.0 };
        assert!(p.hazard(10.0, 500.0, 20.0, 30.0));
        assert!(!p.hazard(10.0, 500.0, 80.0, 30.0));
    }
}
