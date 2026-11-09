//! The road network from vector-tile roads (a port of the project's
//! Protomaps road-graph builder). Every stage keeps the original's rules:
//!
//! 1. Keep drivable roads (`highway`, `major_road`, `minor_road`, without
//!    driveways, parking aisles, construction and the like), plus any
//!    drivable bridge or tunnel the filter would drop. Unlike the original's
//!    country-scale default, tracks and service roads stay: a tactical game
//!    needs forest tracks. Lines arrive clipped to their own tile.
//! 2. T-junctions: where a line ends on the middle of another, a vertex is
//!    inserted there. Crossings without an end point (bridges, tunnels) are
//!    left alone.
//! 3. Snap vertices within a tolerance into nodes. Motorways connect only to
//!    motorways and slip roads, except end to end where a motorway really
//!    ends (not where a tile edge cut it).
//! 4. Segments, de-duplicated, then degree-2 chains contracted so nodes are
//!    junctions, dead ends and changes of attributes. Edges keep their shape.
//!
//! Output (little-endian, ENU metres in the pack's frame), in `roads/`:
//! - `nodes.bin`: per node `f32 e, f32 n`.
//! - `edges.bin`: per edge `u32 from, u32 to, f32 length_m, u8 class (engine
//!   RoadClass), u8 flags (1 bridge, 2 tunnel, 4 slip road), u16 detail
//!   (index into `graph.json` details), u32 name (index into names, or
//!   u32::MAX), u32 geometry start (point index into edge_geom.bin)`: 24 bytes.
//! - `edge_geom.bin`: `f32 e, f32 n` points; edge k's run from its start to edge k+1's.
//! - `adjacency.bin`: CSR, `u32` start per node (nodes + 1 of them) then `u32` edge ids.
//! - `graph.json`: counts, details, names and this layout.

use crate::build::{Ground, RawRoad};
use bgws_env::{Bridge, RoadClass, RoadNetwork};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Normal,
    Controlled,
    Link,
}

const DRIVABLE: [&str; 3] = ["highway", "major_road", "minor_road"];
const EXCLUDE: [&str; 13] = ["driveway", "parking_aisle", "drive-through", "emergency_access", "raceway", "pedestrian", "busway", "bus_guideway", "construction", "proposed", "disused", "abandoned", "alley"];
const NEVER_RESCUE: [&str; 6] = ["construction", "proposed", "disused", "abandoned", "raceway", "pedestrian"];

fn keep(r: &RawRoad) -> bool {
    if !DRIVABLE.contains(&r.kind.as_str()) {
        return false;
    }
    !EXCLUDE.contains(&r.detail.as_str()) || ((r.bridge || r.tunnel) && !NEVER_RESCUE.contains(&r.detail.as_str()))
}

fn access(r: &RawRoad) -> Access {
    if r.link || r.detail.ends_with("_link") {
        Access::Link
    } else if r.detail == "motorway" {
        Access::Controlled
    } else {
        Access::Normal
    }
}

/// Engine class from Protomaps' kind and detail.
pub fn engine_class(kind: &str, detail: &str) -> RoadClass {
    let base = detail.trim_end_matches("_link");
    match (kind, base) {
        (_, "motorway") => RoadClass::Motorway,
        (_, "trunk" | "primary") => RoadClass::Primary,
        (_, "secondary" | "tertiary" | "residential" | "unclassified" | "living_street" | "road") => RoadClass::Secondary,
        (_, "service" | "track") => RoadClass::Track,
        ("highway", _) => RoadClass::Motorway,
        ("major_road", _) => RoadClass::Primary,
        _ => RoadClass::Secondary,
    }
}

/// May a vertex of class `a` merge with one of class `b`? Motorways touch
/// only motorways and slip roads, unless both are real line ends.
fn may_connect(a: Access, b: Access, both_true_ends: bool) -> bool {
    let mixed = (a == Access::Controlled) != (b == Access::Controlled);
    !mixed || a == Access::Link || b == Access::Link || both_true_ends
}

/// A uniform grid over the plane for near-neighbour queries.
struct Buckets {
    size: f64,
    map: HashMap<(i64, i64), Vec<u32>>,
}

impl Buckets {
    fn new(size: f64) -> Self {
        Self { size, map: HashMap::new() }
    }
    fn key(&self, x: f64, y: f64) -> (i64, i64) {
        ((x / self.size).floor() as i64, (y / self.size).floor() as i64)
    }
    fn insert_box(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, id: u32) {
        let (a, b) = (self.key(x0.min(x1), y0.min(y1)), self.key(x0.max(x1), y0.max(y1)));
        for i in a.0..=b.0 {
            for j in a.1..=b.1 {
                self.map.entry((i, j)).or_default().push(id);
            }
        }
    }
    fn near(&self, x: f64, y: f64, r: f64, out: &mut Vec<u32>) {
        out.clear();
        let (a, b) = (self.key(x - r, y - r), self.key(x + r, y + r));
        for i in a.0..=b.0 {
            for j in a.1..=b.1 {
                if let Some(v) = self.map.get(&(i, j)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

fn seg_project(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    let q = [a[0] + t * dx, a[1] + t * dy];
    ((p[0] - q[0]).hypot(p[1] - q[1]), t)
}

struct Dsu(Vec<u32>);
impl Dsu {
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            self.0[x as usize] = self.0[self.0[x as usize] as usize];
            x = self.0[x as usize];
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            // The lower root wins, so labels do not depend on pair order.
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            self.0[hi as usize] = lo;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Props {
    detail: String,
    kind: String,
    bridge: bool,
    tunnel: bool,
    link: bool,
    name: Option<String>,
}

/// The finished graph, in ENU metres.
#[derive(Clone, Debug, Default)]
pub struct RoadGraph {
    pub nodes: Vec<[f64; 2]>,
    pub edges: Vec<GraphEdge>,
    pub details: Vec<String>,
    pub names: Vec<String>,
    pub stats: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub from: u32,
    pub to: u32,
    pub length_m: f64,
    pub class: RoadClass,
    pub bridge: bool,
    pub tunnel: bool,
    pub link: bool,
    pub detail: u16,
    pub name: Option<u32>,
    pub geom: Vec<[f64; 2]>,
}

/// Build the graph from a ground build's roads, snapping within `snap_m` metres.
pub fn build_graph(g: &Ground, snap_m: f64) -> RoadGraph {
    let ext = g.extent as f64;
    // Grid units per metre at the box's centre latitude.
    let lat = g.frame.origin.lat;
    let m_per_unit = crate::mercator::tile_size_m(lat, g.zoom) / ext;
    let tol = snap_m / m_per_unit;

    // 1. Filter.
    let mut lines: Vec<(Vec<[f64; 2]>, Props, Access)> = g
        .roads
        .iter()
        .filter(|r| keep(r) && r.line.len() >= 2)
        .map(|r| (r.line.clone(), Props { detail: r.detail.clone(), kind: r.kind.clone(), bridge: r.bridge, tunnel: r.tunnel, link: r.link, name: r.name.clone() }, access(r)))
        .collect();
    let kept = lines.len();

    // 2. T-junctions.
    let mut segs = Buckets::new((tol * 8.0).max(16.0));
    let mut seg_of: Vec<(u32, u32)> = Vec::new();
    for (li, (l, _, _)) in lines.iter().enumerate() {
        for si in 0..l.len() - 1 {
            let (a, b) = (l[si], l[si + 1]);
            segs.insert_box(a[0], a[1], b[0], b[1], seg_of.len() as u32);
            seg_of.push((li as u32, si as u32));
        }
    }
    let mut inserts: BTreeMap<u32, Vec<(u32, f64, [f64; 2])>> = BTreeMap::new();
    let mut near = Vec::new();
    let mut blocked = 0usize;
    for (li, (l, _, acc)) in lines.iter().enumerate() {
        for end in [l[0], l[l.len() - 1]] {
            segs.near(end[0], end[1], tol, &mut near);
            for &s in &near {
                let (lj, sj) = seg_of[s as usize];
                if lj as usize == li {
                    continue;
                }
                let other = &lines[lj as usize];
                let (a, b) = (other.0[sj as usize], other.0[sj as usize + 1]);
                let (d, t) = seg_project(end, a, b);
                if d > tol {
                    continue;
                }
                let q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                // Next to a vertex already: snapping joins them, nothing to insert.
                if (q[0] - a[0]).hypot(q[1] - a[1]) <= tol || (q[0] - b[0]).hypot(q[1] - b[1]) <= tol {
                    continue;
                }
                // Interior only: the line's own ends snap anyway.
                let first = other.0[0];
                let last = other.0[other.0.len() - 1];
                if (q[0] - first[0]).hypot(q[1] - first[1]) <= tol || (q[0] - last[0]).hypot(q[1] - last[1]) <= tol {
                    continue;
                }
                if !may_connect(*acc, other.2, false) {
                    blocked += 1;
                    continue;
                }
                inserts.entry(lj).or_default().push((sj, t, q));
            }
        }
    }
    let mut t_junctions = 0;
    for (lj, mut ins) in inserts {
        ins.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let old = std::mem::take(&mut lines[lj as usize].0);
        let mut new = Vec::with_capacity(old.len() + ins.len());
        let mut k = 0;
        for (si, &p) in old.iter().enumerate() {
            new.push(p);
            while k < ins.len() && ins[k].0 as usize == si {
                if new.last() != Some(&ins[k].2) {
                    new.push(ins[k].2);
                    t_junctions += 1;
                }
                k += 1;
            }
        }
        lines[lj as usize].0 = new;
    }

    // 3. Snap.
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut pt_acc: Vec<Access> = Vec::new();
    let mut true_end: Vec<bool> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let on_tile_edge = |v: f64| {
        let r = v.rem_euclid(ext);
        r < 1e-6 || r > ext - 1e-6
    };
    for (l, _, acc) in &lines {
        starts.push(pts.len());
        for (i, p) in l.iter().enumerate() {
            pts.push(*p);
            pt_acc.push(*acc);
            let end = i == 0 || i == l.len() - 1;
            true_end.push(end && !on_tile_edge(p[0]) && !on_tile_edge(p[1]));
        }
    }
    let mut buckets = Buckets::new(tol.max(1.0) * 2.0);
    for (i, p) in pts.iter().enumerate() {
        buckets.insert_box(p[0], p[1], p[0], p[1], i as u32);
    }
    let mut dsu = Dsu((0..pts.len() as u32).collect());
    let mut snap_blocked = 0usize;
    for (i, p) in pts.iter().enumerate() {
        buckets.near(p[0], p[1], tol, &mut near);
        for &j in &near {
            let j = j as usize;
            if j <= i || (pts[j][0] - p[0]).hypot(pts[j][1] - p[1]) > tol {
                continue;
            }
            if may_connect(pt_acc[i], pt_acc[j], true_end[i] && true_end[j]) {
                dsu.union(i as u32, j as u32);
            } else {
                snap_blocked += 1;
            }
        }
    }
    let label: Vec<u32> = (0..pts.len() as u32).map(|i| dsu.find(i)).collect();
    let mut centre: HashMap<u32, ([f64; 2], f64)> = HashMap::new();
    for (i, &l) in label.iter().enumerate() {
        let c = centre.entry(l).or_insert(([0.0, 0.0], 0.0));
        c.0[0] += pts[i][0];
        c.0[1] += pts[i][1];
        c.1 += 1.0;
    }

    // 4a. Segments, de-duplicated, with an attribute key each.
    let mut prop_ids: BTreeMap<Props, u32> = BTreeMap::new();
    let mut props: Vec<Props> = Vec::new();
    let mut seen: std::collections::HashSet<(u32, u32, u32)> = Default::default();
    let mut segments: Vec<(u32, u32, u32)> = Vec::new();
    for (li, (l, p, _)) in lines.iter().enumerate() {
        let pk = *prop_ids.entry(p.clone()).or_insert_with(|| {
            props.push(p.clone());
            props.len() as u32 - 1
        });
        let s = starts[li];
        for i in 0..l.len() - 1 {
            let (u, v) = (label[s + i], label[s + i + 1]);
            if u == v {
                continue;
            }
            if seen.insert((u.min(v), u.max(v), pk)) {
                segments.push((u, v, pk));
            }
        }
    }

    // 4b. Contract degree-2 chains with the same attributes.
    let mut inc: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, &(u, v, _)) in segments.iter().enumerate() {
        inc.entry(u).or_default().push(i);
        inc.entry(v).or_default().push(i);
    }
    let anchor = |n: u32| {
        let s = &inc[&n];
        s.len() != 2 || segments[s[0]].2 != segments[s[1]].2
    };
    let mut used = vec![false; segments.len()];
    let mut chains: Vec<(Vec<u32>, u32)> = Vec::new();
    let mut walk = |start: u32, mut sid: usize, used: &mut Vec<bool>| {
        let key = segments[sid].2;
        let (mut path, mut cur) = (vec![start], start);
        loop {
            used[sid] = true;
            let (u, v, _) = segments[sid];
            cur = if u == cur { v } else { u };
            path.push(cur);
            if anchor(cur) {
                break;
            }
            let s = &inc[&cur];
            sid = if s[0] == sid { s[1] } else { s[0] };
            if used[sid] {
                break;
            }
        }
        chains.push((path, key));
    };
    let mut nodes_sorted: Vec<u32> = inc.keys().copied().collect();
    nodes_sorted.sort_unstable();
    for &n in &nodes_sorted {
        if anchor(n) {
            for &sid in &inc[&n].clone() {
                if !used[sid] {
                    walk(n, sid, &mut used);
                }
            }
        }
    }
    for sid in 0..segments.len() {
        if !used[sid] {
            walk(segments[sid].0, sid, &mut used);
        }
    }

    // Renumber nodes compactly, in a spatial order; positions to ENU.
    let mut node_ids: Vec<u32> = chains.iter().flat_map(|(p, _)| [p[0], p[p.len() - 1]]).collect();
    node_ids.sort_unstable();
    node_ids.dedup();
    let enu_of = |l: u32| {
        let (c, n) = centre[&l];
        g.grid_to_enu([c[0] / n, c[1] / n])
    };
    let mut keyed: Vec<(i64, i64, u32)> = node_ids.iter().map(|&l| {
        let (e, n) = enu_of(l);
        ((n / 1000.0).floor() as i64, (e * 10.0).round() as i64, l)
    }).collect();
    keyed.sort_unstable();
    let index: HashMap<u32, u32> = keyed.iter().enumerate().map(|(i, k)| (k.2, i as u32)).collect();
    let nodes: Vec<[f64; 2]> = keyed.iter().map(|k| {
        let (e, n) = enu_of(k.2);
        [e, n]
    }).collect();

    let mut details: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut detail_ix: HashMap<String, u16> = HashMap::new();
    let mut name_ix: HashMap<String, u32> = HashMap::new();
    let mut edges: Vec<GraphEdge> = chains
        .iter()
        .map(|(path, pk)| {
            let p = &props[*pk as usize];
            let geom: Vec<[f64; 2]> = path.iter().map(|&l| {
                let (e, n) = enu_of(l);
                [e, n]
            }).collect();
            let length_m = geom.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let detail = *detail_ix.entry(p.detail.clone()).or_insert_with(|| {
                details.push(p.detail.clone());
                details.len() as u16 - 1
            });
            let name = p.name.as_ref().map(|nm| {
                *name_ix.entry(nm.clone()).or_insert_with(|| {
                    names.push(nm.clone());
                    names.len() as u32 - 1
                })
            });
            let (mut from, mut to) = (index[&path[0]], index[&path[path.len() - 1]]);
            let mut geom = geom;
            if from > to {
                std::mem::swap(&mut from, &mut to);
                geom.reverse();
            }
            GraphEdge { from, to, length_m, class: engine_class(&p.kind, &p.detail), bridge: p.bridge, tunnel: p.tunnel, link: p.link || p.detail.ends_with("_link"), detail, name, geom }
        })
        .collect();
    edges.sort_by(|a, b| (a.from, a.to).cmp(&(b.from, b.to)).then(a.length_m.total_cmp(&b.length_m)));

    // Components, for the report.
    let mut cd = Dsu((0..nodes.len() as u32).collect());
    for e in &edges {
        cd.union(e.from, e.to);
    }
    let mut sizes: HashMap<u32, usize> = HashMap::new();
    for n in 0..nodes.len() as u32 {
        *sizes.entry(cd.find(n)).or_default() += 1;
    }
    let largest = sizes.values().copied().max().unwrap_or(0);
    let km: f64 = edges.iter().map(|e| e.length_m).sum::<f64>() / 1000.0;
    let by_class: BTreeMap<&str, f64> = RoadClass::ALL.iter().map(|c| (c.key(), edges.iter().filter(|e| e.class == *c).map(|e| e.length_m).sum::<f64>() / 1000.0)).collect();
    let stats = serde_json::json!({
        "linePartsKept": kept, "tJunctions": t_junctions, "tJunctionsBlockedByAccess": blocked, "snapsBlockedByAccess": snap_blocked,
        "nodes": nodes.len(), "edges": edges.len(), "km": km, "kmByClass": by_class,
        "bridges": edges.iter().filter(|e| e.bridge).count(), "tunnels": edges.iter().filter(|e| e.tunnel).count(),
        "components": sizes.len(), "largestComponentNodes": largest, "snapM": snap_m,
    });
    RoadGraph { nodes, edges, details, names, stats }
}

/// Default military load class of a bridge by road class [default].
fn bridge_mlc(c: RoadClass) -> u16 {
    match c {
        RoadClass::Motorway => 100,
        RoadClass::Primary => 80,
        RoadClass::Secondary => 50,
        RoadClass::Track => 16,
    }
}

impl RoadGraph {
    /// The engine's road network.
    pub fn to_network(&self) -> RoadNetwork {
        let mut net = RoadNetwork::default();
        let ix: Vec<_> = self.nodes.iter().map(|p| net.add_node(p[0], p[1])).collect();
        for e in &self.edges {
            let bridge = e.bridge.then(|| Bridge { load_class: bridge_mlc(e.class), span_m: e.length_m, blown: false });
            let id = net.add_road(ix[e.from as usize], ix[e.to as usize], e.class, bridge);
            net.graph[id].length_m = e.length_m;
        }
        net
    }

    /// Write `nodes.bin`, `edges.bin`, `edge_geom.bin`, `adjacency.bin` and `graph.json` into `dir`.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut nodes = Vec::with_capacity(self.nodes.len() * 8);
        for p in &self.nodes {
            nodes.extend_from_slice(&(p[0] as f32).to_le_bytes());
            nodes.extend_from_slice(&(p[1] as f32).to_le_bytes());
        }
        let (mut edges, mut geom) = (Vec::with_capacity(self.edges.len() * 24), Vec::new());
        let mut gstart = 0u32;
        for e in &self.edges {
            edges.extend_from_slice(&e.from.to_le_bytes());
            edges.extend_from_slice(&e.to.to_le_bytes());
            edges.extend_from_slice(&(e.length_m as f32).to_le_bytes());
            edges.push(e.class as u8);
            edges.push(u8::from(e.bridge) | (u8::from(e.tunnel) << 1) | (u8::from(e.link) << 2));
            edges.extend_from_slice(&e.detail.to_le_bytes());
            edges.extend_from_slice(&e.name.unwrap_or(u32::MAX).to_le_bytes());
            edges.extend_from_slice(&gstart.to_le_bytes());
            for p in &e.geom {
                geom.extend_from_slice(&(p[0] as f32).to_le_bytes());
                geom.extend_from_slice(&(p[1] as f32).to_le_bytes());
            }
            gstart += e.geom.len() as u32;
        }
        let mut deg = vec![0u32; self.nodes.len() + 1];
        for e in &self.edges {
            deg[e.from as usize + 1] += 1;
            deg[e.to as usize + 1] += 1;
        }
        for i in 1..deg.len() {
            deg[i] += deg[i - 1];
        }
        let mut fill = deg.clone();
        let mut adj = vec![0u32; *deg.last().unwrap_or(&0) as usize];
        for (k, e) in self.edges.iter().enumerate() {
            for n in [e.from, e.to] {
                adj[fill[n as usize] as usize] = k as u32;
                fill[n as usize] += 1;
            }
        }
        let mut csr = Vec::with_capacity((deg.len() + adj.len()) * 4);
        for v in deg.iter().chain(adj.iter()) {
            csr.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(dir.join("nodes.bin"), nodes)?;
        std::fs::write(dir.join("edges.bin"), edges)?;
        std::fs::write(dir.join("edge_geom.bin"), geom)?;
        std::fs::write(dir.join("adjacency.bin"), csr)?;
        let meta = serde_json::json!({
            "format": "bgws.roads", "version": 1,
            "nodes": self.nodes.len(), "edges": self.edges.len(), "geomPoints": gstart,
            "layout": {
                "nodes.bin": "f32 e, f32 n (ENU metres, the pack's frame)",
                "edges.bin": "u32 from, u32 to, f32 length_m, u8 class (0 motorway, 1 primary, 2 secondary, 3 track), u8 flags (1 bridge, 2 tunnel, 4 slip road), u16 detail, u32 name (0xFFFFFFFF none), u32 geometry start: 24 bytes",
                "edge_geom.bin": "f32 e, f32 n; edge k runs from its start to edge k+1's (the last to the end)",
                "adjacency.bin": "u32 start per node (nodes + 1), then u32 edge ids",
            },
            "details": self.details, "names": self.names, "stats": self.stats,
        });
        std::fs::write(dir.join("graph.json"), serde_json::to_vec_pretty(&meta).unwrap_or_default())
    }

    /// Read a graph written by `write`.
    pub fn read(dir: &Path) -> std::io::Result<Self> {
        let bad = |s: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, s.to_string());
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("graph.json"))?).map_err(|e| bad(&e.to_string()))?;
        let f32s = |b: &[u8]| b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c) as f64).collect::<Vec<_>>();
        let nb = f32s(&std::fs::read(dir.join("nodes.bin"))?);
        let nodes: Vec<[f64; 2]> = nb.as_chunks::<2>().0.iter().map(|c| [c[0], c[1]]).collect();
        let gb = f32s(&std::fs::read(dir.join("edge_geom.bin"))?);
        let gpts: Vec<[f64; 2]> = gb.as_chunks::<2>().0.iter().map(|c| [c[0], c[1]]).collect();
        let eb = std::fs::read(dir.join("edges.bin"))?;
        let recs = eb.as_chunks::<24>().0;
        let u32_at = |r: &[u8; 24], i: usize| u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
        let mut edges = Vec::with_capacity(recs.len());
        for (k, r) in recs.iter().enumerate() {
            let start = u32_at(r, 20) as usize;
            let end = recs.get(k + 1).map(|n| u32_at(n, 20) as usize).unwrap_or(gpts.len());
            let class = *RoadClass::ALL.get(r[12] as usize).ok_or_else(|| bad("road class"))?;
            let name = u32_at(r, 16);
            edges.push(GraphEdge {
                from: u32_at(r, 0),
                to: u32_at(r, 4),
                length_m: f32::from_le_bytes([r[8], r[9], r[10], r[11]]) as f64,
                class,
                bridge: r[13] & 1 != 0,
                tunnel: r[13] & 2 != 0,
                link: r[13] & 4 != 0,
                detail: u16::from_le_bytes([r[14], r[15]]),
                name: (name != u32::MAX).then_some(name),
                geom: gpts.get(start..end).ok_or_else(|| bad("edge geometry"))?.to_vec(),
            });
        }
        let strings = |k: &str| meta[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        Ok(RoadGraph { nodes, edges, details: strings("details"), names: strings("names"), stats: meta["stats"].clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Ground;
    use bgws_core::{EnuFrame, Geodetic};

    fn road(kind: &str, detail: &str, line: &[[f64; 2]]) -> RawRoad {
        RawRoad { kind: kind.into(), detail: detail.into(), bridge: false, tunnel: false, link: false, name: None, line: line.to_vec() }
    }

    fn ground(roads: Vec<RawRoad>) -> Ground {
        // Zoom 14 around 54.7°N: one unit is about a third of a metre.
        let frame = EnuFrame::new(Geodetic::new(54.7, 20.5, 0.0));
        let terrain = bgws_env::Terrain::flat(frame.origin, bgws_env::GridSpec::new(4, 4, 30.0, 0.0, 0.0), 0.0);
        let (tx, ty) = crate::mercator::tile_xy(20.5, 54.7, 14);
        let base = [tx.floor() * 4096.0 + 100.0, ty.floor() * 4096.0 + 100.0];
        let roads = roads.into_iter().map(|mut r| {
            for p in &mut r.line {
                p[0] += base[0];
                p[1] += base[1];
            }
            r
        }).collect();
        Ground { frame, terrain, rivers: vec![], towns: vec![], roads, zoom: 14, extent: 4096, basemap: vec![], dem: vec![], planet: String::new(), stats: serde_json::Value::Null, surface: None }
    }

    #[test]
    fn a_t_junction_joins_and_chains_contract() {
        // A main road in two pieces (shared vertex), a side road ending on its middle.
        let g = ground(vec![
            road("major_road", "primary", &[[0.0, 0.0], [300.0, 0.0]]),
            road("major_road", "primary", &[[300.0, 0.0], [600.0, 0.0]]),
            road("minor_road", "residential", &[[150.0, 400.0], [150.0, 1.0]]),
            road("minor_road", "driveway", &[[500.0, 400.0], [500.0, 0.5]]),
        ]);
        let gr = build_graph(&g, 3.0);
        // The main road splits at the side road: three edges, four nodes; the
        // shared vertex at 300 is contracted away; the driveway is dropped.
        assert_eq!(gr.edges.len(), 3, "{:?}", gr.stats);
        assert_eq!(gr.nodes.len(), 4);
        assert_eq!(gr.stats["components"], 1);
        assert_eq!(gr.edges.iter().filter(|e| e.class == RoadClass::Primary).count(), 2);
        let total: f64 = gr.edges.iter().filter(|e| e.class == RoadClass::Primary).map(|e| e.length_m).sum();
        let m_per_unit = crate::mercator::tile_size_m(54.7, 14) / 4096.0;
        assert!((total - 600.0 * m_per_unit).abs() < 1.0, "{total}");

        let dir = std::env::temp_dir().join(format!("bgws-roads-{}", std::process::id()));
        gr.write(&dir).unwrap();
        let back = RoadGraph::read(&dir).unwrap();
        assert_eq!(back.edges.len(), 3);
        assert_eq!(back.edges[1].geom.len(), gr.edges[1].geom.len());
        assert_eq!(gr.to_network().graph.edge_count(), 3);
    }

    #[test]
    fn motorways_join_only_slip_roads() {
        // The slip road joins the motorway at its own junction, between vertices.
        let mut slip = road("highway", "motorway_link", &[[450.0, 300.0], [450.0, 0.5]]);
        slip.link = true;
        let g = ground(vec![
            road("highway", "motorway", &[[0.0, 0.0], [300.0, 0.0], [600.0, 0.0]]),
            // An ordinary road passing a motorway vertex: no junction.
            road("minor_road", "residential", &[[0.0, 300.0], [300.0, 0.4], [600.0, 300.0]]),
            slip,
        ]);
        let gr = build_graph(&g, 3.0);
        assert_eq!(gr.stats["components"], 2, "the residential road stays off the motorway: {:?}", gr.stats);
        assert_eq!(gr.stats["tJunctions"], 1, "the slip road does join it");
        assert!(gr.stats["snapsBlockedByAccess"].as_u64().unwrap() > 0);
    }
}
