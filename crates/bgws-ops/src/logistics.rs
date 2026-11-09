//! Logistics and maintenance (RUST_ENGINE.md §8.11; research R7, P17–P19, R5.2).
//!
//! - **The supply network** is a graph of depots, railheads, ports,
//!   airfields and supply points with stock and throughput, joined by road,
//!   rail, sea, air, drone and UGV links with capacity, transit time and
//!   cost. Each cycle's flows are planned as a **min-cost flow**
//!   ([`MinCostFlow`]: successive shortest paths with Johnson potentials,
//!   integer flows, deterministic tie-breaks), node throughput by node
//!   splitting, then decomposed into paths and executed as convoys, trains,
//!   sorties, sailings and drone/UGV runs ([`Movement`]).
//! - **Consumption from activity:** fuel per km by terrain, per idle hour and
//!   per flight hour (Abrams-class figures **[P17]**), ammunition per
//!   engagement, batteries per drone sortie, food and water per person-day.
//! - **Interdiction:** a struck site loses stock and throughput and repairs
//!   over days; [`hub_standoff`] picks how far back to put hubs given the
//!   enemy's reach (about 120 km **[P18]** emerges from GMLRS-class reach plus
//!   a margin), with the throughput penalty past 150–190 km from rail.
//! - **The last mile** ([`last_mile_plan`]): inside the kill zone, drones and
//!   UGVs carry the load **[P19]**.
//! - **Maintenance** ([`MaintenanceShop`]): failures by operating hours
//!   (exponential, MTBF by class), battle-damage repair times by echelon,
//!   recovery assets and spares as stores.

use bgws_core::{figure, hazard_probability, EntityId, Figure, Figures, SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use thiserror::Error;

/// Figures for logistics and maintenance (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        // Consumption [P17].
        figure!("log.fuel.mbt_l_per_km", 4.5, "L/km", 3.5, 11.8, "P17", Medium, "MBT fuel on roads; 1.5–5 gal/mile"),
        figure!("log.fuel.mbt_idle_l_per_h", 37.9, "L/h", 20.0, 60.0, "P17", High, "Abrams idle ≈10 gal/h"),
        figure!("log.fuel.ifv_l_per_km", 2.0, "L/km", 1.0, 4.0, "default", Default, "IFV fuel on roads"),
        figure!("log.fuel.ifv_idle_l_per_h", 10.0, "L/h", 3.0, 20.0, "default", Default, "IFV idle"),
        figure!("log.fuel.truck_l_per_km", 0.5, "L/km", 0.25, 1.0, "default", Default, "logistics truck"),
        figure!("log.fuel.truck_idle_l_per_h", 3.0, "L/h", 1.0, 6.0, "default", Default, "truck idle"),
        figure!("log.fuel.heli_l_per_flight_h", 600.0, "L/h", 200.0, 2_000.0, "default", Default, "medium helicopter burn"),
        figure!("log.terrain.cross_country", 1.6, "1", 1.0, 3.0, "R7", Low, "cross-country multiplier (Abrams ~60 gal/h cross-country)"),
        figure!("log.terrain.mud", 2.2, "1", 1.0, 4.0, "default", Default, "mud/snow multiplier"),
        figure!("log.terrain.urban", 1.3, "1", 1.0, 2.0, "default", Default, "stop-start urban multiplier"),
        figure!("log.ammo.direct_rounds_per_engagement", 6.0, "1", 1.0, 40.0, "default", Default, "main-gun rounds per engagement"),
        figure!("log.ammo.small_arms_per_engagement", 600.0, "1", 50.0, 5_000.0, "default", Default, "small-arms rounds per squad engagement"),
        figure!("log.ammo.artillery_per_mission", 6.0, "1", 1.0, 50.0, "default", Default, "shells per fire mission"),
        figure!("log.food_kg_per_person_day", 1.8, "kg", 1.0, 3.0, "default", Default, "class I subsistence"),
        figure!("log.water_l_per_person_day", 20.0, "L", 5.0, 40.0, "default", Default, "drinking, cooking, hygiene"),
        // Planning and transport.
        figure!("log.cost_per_t_h", 1.0, "1/(t·h)", 0.0, 100.0, "default", Default, "planning cost weight per tonne-hour in transit"),
        figure!("log.flow_quantum_t", 0.1, "t", 0.01, 10.0, "default", Default, "integer flow unit for min-cost flow"),
        figure!("log.payload.road_t", 10.0, "t", 2.0, 25.0, "R7", Low, "truck payload (HEMTT fueller 2,500 gal ≈ 9.5 t)"),
        figure!("log.payload.rail_t", 1_000.0, "t", 200.0, 3_000.0, "default", Default, "train payload"),
        figure!("log.payload.sea_t", 5_000.0, "t", 500.0, 50_000.0, "default", Default, "ship payload"),
        figure!("log.payload.air_t", 20.0, "t", 5.0, 80.0, "default", Default, "airlift sortie payload"),
        figure!("log.payload.drone_t", 0.02, "t", 0.005, 0.1, "R7", Low, "heavy bomber-drone cargo, ~20 kg"),
        figure!("log.payload.ugv_t", 0.3, "t", 0.05, 1.0, "R5.2", Low, "logistics UGV cargo"),
        figure!("log.convoy_size", 10.0, "1", 2.0, 60.0, "default", Default, "trucks per convoy"),
        // Interdiction and reach [P18].
        figure!("log.enemy_reach_margin_km", 36.0, "km", 0.0, 100.0, "P18", Medium, "margin beyond enemy precision-fires reach for hubs (84 km GMLRS + 36 → ~120 km)"),
        figure!("log.rail_penalty_start_km", 150.0, "km", 100.0, 200.0, "P18", Medium, "throughput penalty starts 150–190 km from rail"),
        figure!("log.rail_penalty_per_km", 0.0125, "1/km", 0.001, 0.05, "P18", Low, "throughput fraction lost per km past the penalty start"),
        figure!("log.rail_penalty_floor", 0.1, "fraction", 0.0, 0.5, "default", Default, "minimum throughput fraction at extreme distance"),
        figure!("log.strike.throughput_loss_per_stock_loss", 1.0, "1", 0.0, 3.0, "default", Default, "throughput damage per fraction of stock destroyed"),
        figure!("log.repair_per_day", 0.1, "1/day", 0.0, 1.0, "default", Default, "fraction of site throughput damage repaired per day"),
        // Last mile [P19].
        figure!("log.kill_zone_depth_km", 10.0, "km", 5.0, 20.0, "P8", Medium, "depth trucks can't enter; 5–20 km"),
        figure!("log.drone.sortie_h", 0.5, "h", 0.1, 2.0, "default", Default, "resupply drone round trip"),
        figure!("log.drone.loss_per_sortie", 0.05, "fraction", 0.0, 0.5, "R7", Low, "resupply drone lost per sortie"),
        figure!("log.ugv.run_h", 2.0, "h", 0.5, 8.0, "default", Default, "UGV round trip"),
        figure!("log.ugv.loss_per_run", 0.08, "fraction", 0.0, 0.5, "R7", Low, "UGV lost per run (>10/month in one unit)"),
        // Maintenance.
        figure!("maint.mtbf.mbt_h", 150.0, "h", 30.0, 1_000.0, "default", Default, "MBT mean operating hours between failures"),
        figure!("maint.mtbf.ifv_h", 200.0, "h", 30.0, 1_000.0, "default", Default, "IFV MTBF"),
        figure!("maint.mtbf.truck_h", 400.0, "h", 50.0, 3_000.0, "default", Default, "truck MTBF"),
        figure!("maint.mtbf.heli_h", 50.0, "h", 10.0, 500.0, "default", Default, "helicopter MTBF per flight hour"),
        figure!("maint.repair.crew_h", 2.0, "h", 0.5, 8.0, "default", Default, "crew-level repair"),
        figure!("maint.repair.field_h", 12.0, "h", 4.0, 48.0, "default", Default, "field (unit) maintenance repair"),
        figure!("maint.repair.support_h", 48.0, "h", 12.0, 168.0, "default", Default, "support maintenance repair"),
        figure!("maint.repair.depot_h", 336.0, "h", 72.0, 2_000.0, "default", Default, "depot repair"),
        figure!("maint.spares.field", 1.0, "1", 0.0, 10.0, "default", Default, "spare-part kits for a field repair"),
        figure!("maint.spares.support", 3.0, "1", 0.0, 20.0, "default", Default, "spare-part kits for a support repair"),
        figure!("maint.spares.depot", 6.0, "1", 0.0, 50.0, "default", Default, "spare-part kits for a depot repair"),
        figure!("maint.recovery_speed_kmh", 15.0, "km/h", 5.0, 40.0, "default", Default, "recovery vehicle average speed with tow"),
    ]
}

// ───────────────────────────── min-cost flow ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FlowEdge {
    to: usize,
    rev: usize,
    cap: i64,
    cost: i64,
    flow: i64,
}

/// An edge handle returned by [`MinCostFlow::add_edge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowEdgeId(pub usize);

#[derive(Debug, Error, PartialEq)]
pub enum FlowError {
    #[error("negative-cost cycle in the network")]
    NegativeCycle,
    #[error("node {0} out of range")]
    BadNode(usize),
    #[error("negative capacity on an edge")]
    NegativeCapacity,
}

/// Integer min-cost flow by successive shortest paths with potentials.
/// Dijkstra breaks ties by node index and edges are scanned in insertion
/// order, so the same problem always gives the same flows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MinCostFlow {
    adj: Vec<Vec<usize>>,
    edges: Vec<FlowEdge>,
    from: Vec<usize>,
}

/// The solution's totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowResult {
    pub flow: i64,
    pub cost: i64,
    pub augmentations: u32,
}

impl MinCostFlow {
    pub fn new(n: usize) -> Self {
        Self { adj: vec![Vec::new(); n], edges: Vec::new(), from: Vec::new() }
    }

    pub fn node_count(&self) -> usize {
        self.adj.len()
    }

    pub fn add_node(&mut self) -> usize {
        self.adj.push(Vec::new());
        self.adj.len() - 1
    }

    pub fn add_edge(&mut self, u: usize, v: usize, cap: i64, cost: i64) -> Result<FlowEdgeId, FlowError> {
        let n = self.adj.len();
        if u >= n {
            return Err(FlowError::BadNode(u));
        }
        if v >= n {
            return Err(FlowError::BadNode(v));
        }
        if cap < 0 {
            return Err(FlowError::NegativeCapacity);
        }
        let e = self.edges.len();
        self.edges.push(FlowEdge { to: v, rev: e + 1, cap, cost, flow: 0 });
        self.from.push(u);
        self.edges.push(FlowEdge { to: u, rev: e, cap: 0, cost: -cost, flow: 0 });
        self.from.push(v);
        self.adj[u].push(e);
        self.adj[v].push(e + 1);
        Ok(FlowEdgeId(e))
    }

    /// Flow on an edge.
    pub fn flow(&self, e: FlowEdgeId) -> i64 {
        self.edges[e.0].flow
    }

    fn residual(&self, e: usize) -> i64 {
        self.edges[e].cap - self.edges[e].flow
    }

    /// Push up to `limit` units from `s` to `t` at minimum cost.
    pub fn solve(&mut self, s: usize, t: usize, limit: i64) -> Result<FlowResult, FlowError> {
        let n = self.adj.len();
        if s >= n || t >= n {
            return Err(FlowError::BadNode(s.max(t)));
        }
        // Initial potentials by Bellman–Ford over edges with residual capacity.
        let mut h = vec![0i64; n];
        for round in 0..=n {
            let mut changed = false;
            for (e, edge) in self.edges.iter().enumerate() {
                if edge.cap - edge.flow > 0 {
                    let u = self.from[e];
                    if h[u] + edge.cost < h[edge.to] {
                        h[edge.to] = h[u] + edge.cost;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            if round == n {
                return Err(FlowError::NegativeCycle);
            }
        }
        let mut total = FlowResult { flow: 0, cost: 0, augmentations: 0 };
        while total.flow < limit {
            // Dijkstra on reduced costs.
            let mut dist = vec![i64::MAX; n];
            let mut prev: Vec<Option<usize>> = vec![None; n];
            dist[s] = 0;
            let mut heap = BinaryHeap::new();
            heap.push(Reverse((0i64, s)));
            while let Some(Reverse((d, u))) = heap.pop() {
                if d > dist[u] {
                    continue;
                }
                for &e in &self.adj[u] {
                    if self.residual(e) <= 0 {
                        continue;
                    }
                    let v = self.edges[e].to;
                    let nd = d + self.edges[e].cost + h[u] - h[v];
                    if nd < dist[v] {
                        dist[v] = nd;
                        prev[v] = Some(e);
                        heap.push(Reverse((nd, v)));
                    }
                }
            }
            if dist[t] == i64::MAX {
                break;
            }
            for v in 0..n {
                if dist[v] != i64::MAX {
                    h[v] += dist[v];
                }
            }
            // Bottleneck.
            let mut push = limit - total.flow;
            let mut v = t;
            while let Some(e) = prev[v] {
                push = push.min(self.residual(e));
                v = self.from[e];
            }
            let mut v = t;
            while let Some(e) = prev[v] {
                self.edges[e].flow += push;
                let r = self.edges[e].rev;
                self.edges[r].flow -= push;
                total.cost += push * self.edges[e].cost;
                v = self.from[e];
            }
            total.flow += push;
            total.augmentations += 1;
        }
        Ok(total)
    }

    /// True if the residual graph has no negative-cost cycle: the
    /// certificate that the current flow is cost-optimal for its value.
    pub fn is_optimal(&self) -> bool {
        let n = self.adj.len();
        let mut d = vec![0i64; n];
        for _ in 0..n {
            let mut changed = false;
            for (e, edge) in self.edges.iter().enumerate() {
                if self.residual(e) > 0 {
                    let u = self.from[e];
                    if d[u] + edge.cost < d[edge.to] {
                        d[edge.to] = d[u] + edge.cost;
                        changed = true;
                    }
                }
            }
            if !changed {
                return true;
            }
        }
        false
    }

    /// Net flow out of each node (supply positive).
    pub fn net_outflow(&self) -> Vec<i64> {
        let mut out = vec![0i64; self.adj.len()];
        for (e, edge) in self.edges.iter().enumerate().step_by(2) {
            out[self.from[e]] += edge.flow;
            out[edge.to] -= edge.flow;
        }
        out
    }
}

// ─────────────────────────── the supply network ───────────────────────────

/// A logistics site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SiteId(pub u32);

/// A link between sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Depot,
    Railhead,
    Port,
    Airfield,
    SupplyPoint,
    Unit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkMode {
    Road,
    Rail,
    Sea,
    Air,
    Drone,
    Ugv,
}

/// Classes of supply (R7: I subsistence, III POL, V ammunition, VIII medical, IX parts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplyClass {
    I,
    III,
    V,
    VIII,
    IX,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub id: SiteId,
    pub kind: SiteKind,
    /// Tonnes on hand by class.
    pub stock_t: BTreeMap<SupplyClass, f64>,
    /// Tonnes per day it can handle when undamaged.
    pub throughput_t_per_day: f64,
    /// 0 = intact, 1 = out of action.
    pub damage: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupplyLink {
    pub id: LinkId,
    pub from: SiteId,
    pub to: SiteId,
    pub mode: LinkMode,
    pub capacity_t_per_day: f64,
    pub transit_h: f64,
    pub cost_per_t: f64,
    pub damage: f64,
}

#[derive(Debug, Error, PartialEq)]
pub enum LogisticsError {
    #[error("unknown site {0:?}")]
    UnknownSite(SiteId),
    #[error("flow: {0}")]
    Flow(#[from] FlowError),
}

/// The supply network (§8.11).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SupplyNetwork {
    pub sites: BTreeMap<SiteId, Site>,
    pub links: BTreeMap<LinkId, SupplyLink>,
}

/// One planned flow on a link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedFlow {
    pub link: LinkId,
    pub from: SiteId,
    pub to: SiteId,
    pub mode: LinkMode,
    pub tonnes: f64,
    pub transit_h: f64,
}

/// A source-to-sink path of the plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowPath {
    pub source: SiteId,
    pub sink: SiteId,
    pub links: Vec<LinkId>,
    pub tonnes: f64,
    pub transit_h: f64,
}

/// One cycle's plan (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowPlan {
    pub class: SupplyClass,
    pub cycle_days: f64,
    pub flows: Vec<PlannedFlow>,
    pub paths: Vec<FlowPath>,
    pub delivered_t: BTreeMap<SiteId, f64>,
    pub shortfall_t: BTreeMap<SiteId, f64>,
    pub drawn_t: BTreeMap<SiteId, f64>,
    pub total_cost: f64,
}

/// How a planned flow is carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementKind {
    Convoy,
    Train,
    Sailing,
    Sortie,
    DroneDelivery,
    UgvRun,
}

/// A movement to execute: convoys, trains, sorties (entities with signatures, §8.11).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    pub link: LinkId,
    pub kind: MovementKind,
    pub tonnes: f64,
    /// Vehicles (trucks, trains, ships, aircraft, drones, UGVs).
    pub vehicles: u32,
    /// Separate packets on the move (road convoys of `log.convoy_size`; else one per vehicle).
    pub packets: u32,
    pub depart: SimTime,
    pub arrive: SimTime,
}

impl SupplyNetwork {
    pub fn add_site(&mut self, site: Site) {
        self.sites.insert(site.id, site);
    }

    pub fn add_link(&mut self, link: SupplyLink) -> Result<(), LogisticsError> {
        for s in [link.from, link.to] {
            if !self.sites.contains_key(&s) {
                return Err(LogisticsError::UnknownSite(s));
            }
        }
        self.links.insert(link.id, link);
        Ok(())
    }

    /// Plan one cycle's flows of `class` to meet `demand_t` by site.
    pub fn plan_cycle(&self, class: SupplyClass, demand_t: &BTreeMap<SiteId, f64>, cycle_days: f64, f: &Figures) -> Result<FlowPlan, LogisticsError> {
        let q = f.get("log.flow_quantum_t");
        let time_w = f.get("log.cost_per_t_h");
        let to_units = |t: f64| (t / q).floor().max(0.0) as i64;
        // Nodes: 0 = source, 1 = sink, then (in, out) per site.
        let mut g = MinCostFlow::new(2);
        let mut index: BTreeMap<SiteId, (usize, usize)> = BTreeMap::new();
        let mut throughput_edges = Vec::new();
        for (id, s) in &self.sites {
            let i = g.add_node();
            let o = g.add_node();
            index.insert(*id, (i, o));
            let cap = to_units(s.throughput_t_per_day * cycle_days * (1.0 - s.damage).clamp(0.0, 1.0));
            throughput_edges.push(g.add_edge(i, o, cap, 0)?);
        }
        let mut source_edges = Vec::new();
        for (id, s) in &self.sites {
            let stock = s.stock_t.get(&class).copied().unwrap_or(0.0);
            if stock > 0.0 {
                source_edges.push((*id, g.add_edge(0, index[id].0, to_units(stock), 0)?));
            }
        }
        let mut sink_edges = Vec::new();
        for (id, d) in demand_t {
            let (_, o) = *index.get(id).ok_or(LogisticsError::UnknownSite(*id))?;
            sink_edges.push((*id, g.add_edge(o, 1, to_units(*d), 0)?));
        }
        let mut link_edges = Vec::new();
        for l in self.links.values() {
            let cap = to_units(l.capacity_t_per_day * cycle_days * (1.0 - l.damage).clamp(0.0, 1.0));
            // Integer cost per quantum, in thousandths for resolution.
            let cost = ((l.cost_per_t + time_w * l.transit_h) * q * 1000.0).round() as i64;
            link_edges.push((l.id, g.add_edge(index[&l.from].1, index[&l.to].0, cap, cost)?));
        }
        let res = g.solve(0, 1, i64::MAX)?;
        let mut flows = Vec::new();
        for (lid, e) in &link_edges {
            let fl = g.flow(*e);
            if fl > 0 {
                let l = &self.links[lid];
                flows.push(PlannedFlow { link: *lid, from: l.from, to: l.to, mode: l.mode, tonnes: fl as f64 * q, transit_h: l.transit_h });
            }
        }
        let mut delivered = BTreeMap::new();
        let mut shortfall = BTreeMap::new();
        for (id, e) in &sink_edges {
            let got = g.flow(*e) as f64 * q;
            delivered.insert(*id, got);
            let short = demand_t[id] - got;
            if short > 1e-9 {
                shortfall.insert(*id, short);
            }
        }
        let drawn = source_edges.iter().filter(|(_, e)| g.flow(*e) > 0).map(|(id, e)| (*id, g.flow(*e) as f64 * q)).collect();
        let paths = self.decompose(&g, &source_edges, &sink_edges, &link_edges, q);
        Ok(FlowPlan { class, cycle_days, flows, paths, delivered_t: delivered, shortfall_t: shortfall, drawn_t: drawn, total_cost: res.cost as f64 / 1000.0 })
    }

    /// Flow decomposition into source→sink paths (deterministic: lowest ids first).
    fn decompose(&self, g: &MinCostFlow, sources: &[(SiteId, FlowEdgeId)], sinks: &[(SiteId, FlowEdgeId)], links: &[(LinkId, FlowEdgeId)], q: f64) -> Vec<FlowPath> {
        let mut rem: BTreeMap<LinkId, i64> = links.iter().map(|(l, e)| (*l, g.flow(*e))).collect();
        let mut src: BTreeMap<SiteId, i64> = sources.iter().map(|(s, e)| (*s, g.flow(*e))).collect();
        let mut snk: BTreeMap<SiteId, i64> = sinks.iter().map(|(s, e)| (*s, g.flow(*e))).collect();
        let mut paths = Vec::new();
        for _ in 0..10_000 {
            let Some((&start, _)) = src.iter().find(|(_, v)| **v > 0) else { break };
            let mut at = start;
            let mut used: Vec<LinkId> = Vec::new();
            let mut amount = src[&start];
            let mut ok = false;
            for _ in 0..=self.sites.len() {
                if snk.get(&at).copied().unwrap_or(0) > 0 {
                    amount = amount.min(snk[&at]);
                    ok = true;
                    break;
                }
                let next = self.links.values().find(|l| l.from == at && rem.get(&l.id).copied().unwrap_or(0) > 0 && !used.contains(&l.id));
                match next {
                    Some(l) => {
                        amount = amount.min(rem[&l.id]);
                        used.push(l.id);
                        at = l.to;
                    }
                    None => break,
                }
            }
            if !ok || amount <= 0 {
                // Stock drawn but not routed (shouldn't happen): drop it.
                src.insert(start, 0);
                continue;
            }
            *src.get_mut(&start).expect("source") -= amount;
            *snk.get_mut(&at).expect("sink") -= amount;
            for l in &used {
                *rem.get_mut(l).expect("link") -= amount;
            }
            let transit = used.iter().map(|l| self.links[l].transit_h).sum();
            paths.push(FlowPath { source: start, sink: at, links: used, tonnes: amount as f64 * q, transit_h: transit });
        }
        paths
    }

    /// Execute a plan at `t`: draw stock at sources and return the movements
    /// per path leg. Deliveries land when the host calls [`SupplyNetwork::receive`].
    pub fn dispatch(&mut self, plan: &FlowPlan, t: SimTime, f: &Figures) -> Vec<(FlowPath, Vec<Movement>)> {
        for (id, tonnes) in &plan.drawn_t {
            if let Some(s) = self.sites.get_mut(id) {
                let e = s.stock_t.entry(plan.class).or_insert(0.0);
                *e = (*e - tonnes).max(0.0);
            }
        }
        let mut out = Vec::new();
        for p in &plan.paths {
            let mut depart = t;
            let mut legs = Vec::new();
            for lid in &p.links {
                let l = &self.links[lid];
                let arrive = depart + SimDuration::from_hours(l.transit_h);
                legs.push(movement_for(l, p.tonnes, depart, arrive, f));
                depart = arrive;
            }
            out.push((p.clone(), legs));
        }
        out
    }

    /// A path's cargo arrives.
    pub fn receive(&mut self, sink: SiteId, class: SupplyClass, tonnes: f64) {
        if let Some(s) = self.sites.get_mut(&sink) {
            *s.stock_t.entry(class).or_insert(0.0) += tonnes;
        }
    }

    /// A strike destroys `fraction` of a site's stock; throughput is damaged
    /// in proportion (rule 10: returns what happened).
    pub fn strike_site(&mut self, id: SiteId, fraction: f64, f: &Figures) -> Result<StrikeEffect, LogisticsError> {
        let s = self.sites.get_mut(&id).ok_or(LogisticsError::UnknownSite(id))?;
        let fr = fraction.clamp(0.0, 1.0);
        let lost: f64 = s.stock_t.values().map(|v| v * fr).sum();
        for v in s.stock_t.values_mut() {
            *v *= 1.0 - fr;
        }
        let before = s.damage;
        s.damage = (s.damage + fr * f.get("log.strike.throughput_loss_per_stock_loss")).clamp(0.0, 1.0);
        Ok(StrikeEffect { site: id, stock_lost_t: lost, damage_before: before, damage_after: s.damage })
    }

    /// Repairs over `days`: damage falls exponentially.
    pub fn repair(&mut self, days: f64, f: &Figures) {
        let k = f.get("log.repair_per_day");
        for s in self.sites.values_mut() {
            s.damage *= (-k * days).exp();
        }
        for l in self.links.values_mut() {
            l.damage *= (-k * days).exp();
        }
    }
}

/// What a site strike did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeEffect {
    pub site: SiteId,
    pub stock_lost_t: f64,
    pub damage_before: f64,
    pub damage_after: f64,
}

fn movement_for(l: &SupplyLink, tonnes: f64, depart: SimTime, arrive: SimTime, f: &Figures) -> Movement {
    let (kind, payload) = match l.mode {
        LinkMode::Road => (MovementKind::Convoy, f.get("log.payload.road_t")),
        LinkMode::Rail => (MovementKind::Train, f.get("log.payload.rail_t")),
        LinkMode::Sea => (MovementKind::Sailing, f.get("log.payload.sea_t")),
        LinkMode::Air => (MovementKind::Sortie, f.get("log.payload.air_t")),
        LinkMode::Drone => (MovementKind::DroneDelivery, f.get("log.payload.drone_t")),
        LinkMode::Ugv => (MovementKind::UgvRun, f.get("log.payload.ugv_t")),
    };
    let vehicles = (tonnes / payload).ceil().max(1.0) as u32;
    let packets = if kind == MovementKind::Convoy { (vehicles as f64 / f.get("log.convoy_size")).ceil().max(1.0) as u32 } else { vehicles };
    Movement { link: l.id, kind, tonnes, vehicles, packets, depart, arrive }
}

/// Hub placement against the enemy's reach **[P18]**.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HubPlacement {
    pub distance_km: f64,
    pub enemy_reach_km: f64,
    /// Throughput fraction left at this distance from the railhead.
    pub throughput_factor: f64,
}

/// Throughput fraction at `km` from the railhead: 1 to the penalty start,
/// then falling linearly to a floor **[P18]**.
pub fn rail_distance_factor(km: f64, f: &Figures) -> f64 {
    let start = f.get("log.rail_penalty_start_km");
    (1.0 - f.get("log.rail_penalty_per_km") * (km - start).max(0.0)).max(f.get("log.rail_penalty_floor"))
}

/// Pull rail-fed hubs back beyond the enemy's precision-fires reach plus a
/// margin; the forward units then pay the haul from rail in throughput.
pub fn hub_standoff(enemy_reach_km: f64, f: &Figures) -> HubPlacement {
    let d = enemy_reach_km + f.get("log.enemy_reach_margin_km");
    HubPlacement { distance_km: d, enemy_reach_km, throughput_factor: rail_distance_factor(d, f) }
}

// ───────────────────────────── consumption ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleClass {
    Mbt,
    Ifv,
    Truck,
    Helicopter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    Road,
    CrossCountry,
    Mud,
    Urban,
}

/// What a unit did over a period.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub km: Vec<(Terrain, f64)>,
    pub idle_h: f64,
    pub flight_h: f64,
    pub direct_engagements: f64,
    pub squad_engagements: f64,
    pub fire_missions: f64,
    pub drone_sorties: f64,
    pub person_days: f64,
}

/// What it consumed (rule 10).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Consumption {
    pub fuel_l: f64,
    pub main_gun_rounds: f64,
    pub small_arms_rounds: f64,
    pub shells: f64,
    pub batteries: f64,
    pub food_kg: f64,
    pub water_l: f64,
}

pub fn terrain_factor(t: Terrain, f: &Figures) -> f64 {
    match t {
        Terrain::Road => 1.0,
        Terrain::CrossCountry => f.get("log.terrain.cross_country"),
        Terrain::Mud => f.get("log.terrain.mud"),
        Terrain::Urban => f.get("log.terrain.urban"),
    }
}

/// Consumption from activity for one vehicle of `class` (plus its share of people).
pub fn consumption(class: VehicleClass, a: &Activity, f: &Figures) -> Consumption {
    let (per_km, idle) = match class {
        VehicleClass::Mbt => (f.get("log.fuel.mbt_l_per_km"), f.get("log.fuel.mbt_idle_l_per_h")),
        VehicleClass::Ifv => (f.get("log.fuel.ifv_l_per_km"), f.get("log.fuel.ifv_idle_l_per_h")),
        VehicleClass::Truck => (f.get("log.fuel.truck_l_per_km"), f.get("log.fuel.truck_idle_l_per_h")),
        VehicleClass::Helicopter => (0.0, 0.0),
    };
    let moving: f64 = a.km.iter().map(|(t, km)| km * per_km * terrain_factor(*t, f)).sum();
    let flight = if class == VehicleClass::Helicopter { a.flight_h * f.get("log.fuel.heli_l_per_flight_h") } else { 0.0 };
    Consumption {
        fuel_l: moving + a.idle_h * idle + flight,
        main_gun_rounds: a.direct_engagements * f.get("log.ammo.direct_rounds_per_engagement"),
        small_arms_rounds: a.squad_engagements * f.get("log.ammo.small_arms_per_engagement"),
        shells: a.fire_missions * f.get("log.ammo.artillery_per_mission"),
        batteries: a.drone_sorties * f.get("drones.batteries_per_sortie"),
        food_kg: a.person_days * f.get("log.food_kg_per_person_day"),
        water_l: a.person_days * f.get("log.water_l_per_person_day"),
    }
}

// ───────────────────────────── the last mile ─────────────────────────────

/// Uncrewed last-mile assets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastMileFleet {
    pub drones: u32,
    pub ugvs: u32,
    /// Hours available per day per asset.
    pub hours_per_day: f64,
}

/// The day's last-mile plan (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastMilePlan {
    pub truck_kg: f64,
    pub drone_kg: f64,
    pub ugv_kg: f64,
    pub undelivered_kg: f64,
    pub drone_sorties: u32,
    pub ugv_runs: u32,
    pub expected_drone_losses: f64,
    pub expected_ugv_losses: f64,
    /// Share of delivered weight carried by drones and UGVs [P19].
    pub uncrewed_share: f64,
}

/// Plan a day's deliveries to positions given as (km from the line, kg).
/// Trucks serve positions behind the kill zone; inside it, UGVs take the
/// bulk and drones the rest, within the fleet's daily capacity **[P19]**.
pub fn last_mile_plan(positions: &[(f64, f64)], fleet: &LastMileFleet, f: &Figures) -> LastMilePlan {
    let depth = f.get("log.kill_zone_depth_km");
    let drone_kg = f.get("log.payload.drone_t") * 1000.0;
    let ugv_kg = f.get("log.payload.ugv_t") * 1000.0;
    let drone_cap_sorties = (fleet.drones as f64 * fleet.hours_per_day / f.get("log.drone.sortie_h")).floor();
    let ugv_cap_runs = (fleet.ugvs as f64 * fleet.hours_per_day / f.get("log.ugv.run_h")).floor();
    let mut truck = 0.0;
    let mut kz = 0.0;
    for (km, kg) in positions {
        if *km > depth {
            truck += kg;
        } else {
            kz += kg;
        }
    }
    let ugv_load = kz.min(ugv_cap_runs * ugv_kg);
    let ugv_runs = (ugv_load / ugv_kg).ceil();
    let rest = kz - ugv_load;
    let drone_load = rest.min(drone_cap_sorties * drone_kg);
    let sorties = (drone_load / drone_kg).ceil();
    let undelivered = rest - drone_load;
    let delivered = truck + ugv_load + drone_load;
    LastMilePlan {
        truck_kg: truck,
        drone_kg: drone_load,
        ugv_kg: ugv_load,
        undelivered_kg: undelivered,
        drone_sorties: sorties as u32,
        ugv_runs: ugv_runs as u32,
        expected_drone_losses: sorties * f.get("log.drone.loss_per_sortie"),
        expected_ugv_losses: ugv_runs * f.get("log.ugv.loss_per_run"),
        uncrewed_share: if delivered > 0.0 { (ugv_load + drone_load) / delivered } else { 0.0 },
    }
}

// ───────────────────────────── maintenance ─────────────────────────────

/// Probability a vehicle of `class` fails within `op_hours` of operation
/// (exponential, MTBF by class).
pub fn failure_probability(class: VehicleClass, op_hours: f64, f: &Figures) -> f64 {
    let mtbf = match class {
        VehicleClass::Mbt => f.get("maint.mtbf.mbt_h"),
        VehicleClass::Ifv => f.get("maint.mtbf.ifv_h"),
        VehicleClass::Truck => f.get("maint.mtbf.truck_h"),
        VehicleClass::Helicopter => f.get("maint.mtbf.heli_h"),
    };
    hazard_probability(1.0 / mtbf, op_hours)
}

/// Repair echelon by damage level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Echelon {
    Crew,
    Field,
    Support,
    Depot,
}

impl Echelon {
    pub fn repair_h(self, f: &Figures) -> f64 {
        match self {
            Echelon::Crew => f.get("maint.repair.crew_h"),
            Echelon::Field => f.get("maint.repair.field_h"),
            Echelon::Support => f.get("maint.repair.support_h"),
            Echelon::Depot => f.get("maint.repair.depot_h"),
        }
    }
    pub fn spares(self, f: &Figures) -> u32 {
        match self {
            Echelon::Crew => 0,
            Echelon::Field => f.get("maint.spares.field").round() as u32,
            Echelon::Support => f.get("maint.spares.support").round() as u32,
            Echelon::Depot => f.get("maint.spares.depot").round() as u32,
        }
    }
}

/// A repair job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepairJob {
    pub vehicle: EntityId,
    pub echelon: Echelon,
    /// Needs a recovery vehicle to tow it back.
    pub needs_recovery: bool,
    pub distance_km: f64,
    pub at: SimTime,
}

/// A finished repair (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepairDone {
    pub vehicle: EntityId,
    pub echelon: Echelon,
    pub submitted: SimTime,
    pub recovered_at: SimTime,
    pub repair_start: SimTime,
    pub ready_at: SimTime,
    pub spares_used: u32,
}

/// A maintenance organisation: recovery vehicles, repair teams per echelon,
/// and spares as stores. Jobs are served in submission order; a job that
/// lacks spares waits (and blocks its team's queue) until spares arrive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceShop {
    pub recovery_free: Vec<SimTime>,
    pub teams_free: BTreeMap<Echelon, Vec<SimTime>>,
    pub spares: u32,
    queue: Vec<(RepairJob, Option<SimTime>)>,
}

impl MaintenanceShop {
    pub fn new(recovery_vehicles: usize, teams: &[(Echelon, usize)], spares: u32) -> Self {
        Self {
            recovery_free: vec![SimTime::ZERO; recovery_vehicles],
            teams_free: teams.iter().map(|(e, n)| (*e, vec![SimTime::ZERO; *n])).collect(),
            spares,
            queue: Vec::new(),
        }
    }

    pub fn submit(&mut self, job: RepairJob) {
        self.queue.push((job, None));
    }

    pub fn add_spares(&mut self, n: u32) {
        self.spares += n;
    }

    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// Schedule every job submitted by `until` that can start (recovery free,
    /// spares on hand); return them with their ready times.
    pub fn step(&mut self, until: SimTime, f: &Figures) -> Vec<RepairDone> {
        let speed = f.get("maint.recovery_speed_kmh");
        let mut done = Vec::new();
        let mut keep = Vec::new();
        let mut queue = std::mem::take(&mut self.queue);
        queue.sort_by(|a, b| a.0.at.cmp(&b.0.at).then(a.0.vehicle.cmp(&b.0.vehicle)));
        let mut blocked = false;
        for (job, recovered) in queue {
            if job.at > until || blocked {
                keep.push((job, recovered));
                continue;
            }
            // Recovery (once).
            let recovered_at = match recovered {
                Some(t) => t,
                None if job.needs_recovery => {
                    let Some((i, free)) = self.recovery_free.iter().copied().enumerate().min_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0))) else {
                        keep.push((job, None));
                        continue;
                    };
                    let start = free.max(job.at);
                    let back = start + SimDuration::from_hours(2.0 * job.distance_km / speed);
                    self.recovery_free[i] = back;
                    back
                }
                None => job.at,
            };
            let need = job.echelon.spares(f);
            if need > self.spares {
                keep.push((job, Some(recovered_at)));
                blocked = true;
                continue;
            }
            let Some(teams) = self.teams_free.get_mut(&job.echelon) else {
                keep.push((job, Some(recovered_at)));
                continue;
            };
            let Some((i, free)) = teams.iter().copied().enumerate().min_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0))) else {
                keep.push((job, Some(recovered_at)));
                continue;
            };
            let start = free.max(recovered_at);
            let ready = start + SimDuration::from_hours(job.echelon.repair_h(f));
            teams[i] = ready;
            self.spares -= need;
            done.push(RepairDone { vehicle: job.vehicle, echelon: job.echelon, submitted: job.at, recovered_at, repair_start: start, ready_at: ready, spares_used: need });
        }
        self.queue = keep;
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn figs() -> Figures {
        crate::default_figures()
    }

    #[test]
    fn min_cost_flow_textbook() {
        // s=0, t=3; two routes, cheap one capacity-limited.
        let mut g = MinCostFlow::new(4);
        let a = g.add_edge(0, 1, 4, 1).unwrap();
        let b = g.add_edge(0, 2, 2, 2).unwrap();
        g.add_edge(1, 2, 2, 1).unwrap();
        let c = g.add_edge(1, 3, 3, 3).unwrap();
        g.add_edge(2, 3, 5, 1).unwrap();
        let r = g.solve(0, 3, i64::MAX).unwrap();
        assert_eq!(r.flow, 6);
        assert_eq!(r.cost, 20);
        assert_eq!((g.flow(a), g.flow(b), g.flow(c)), (4, 2, 2));
        assert!(g.is_optimal());
        let mut h = MinCostFlow::new(2);
        h.add_edge(0, 1, 5, 2).unwrap();
        assert_eq!(h.solve(0, 1, 3).unwrap(), FlowResult { flow: 3, cost: 6, augmentations: 1 });
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn min_cost_flow_is_feasible_and_optimal(edges in proptest::collection::vec((0usize..6, 0usize..6, 0i64..20, 0i64..30), 1..25)) {
            let mut g = MinCostFlow::new(6);
            let mut ids = Vec::new();
            for (u, v, c, w) in &edges {
                if u != v { ids.push((g.add_edge(*u, *v, *c, *w).unwrap(), *c)); }
            }
            let r = g.solve(0, 5, i64::MAX).unwrap();
            for (e, c) in &ids { prop_assert!(g.flow(*e) >= 0 && g.flow(*e) <= *c); }
            let net = g.net_outflow();
            prop_assert_eq!(net[0], r.flow);
            prop_assert_eq!(net[5], -r.flow);
            for x in &net[1..5] { prop_assert_eq!(*x, 0); }
            prop_assert!(g.is_optimal());
            // Deterministic.
            let mut g2 = MinCostFlow::new(6);
            for (u, v, c, w) in &edges { if u != v { g2.add_edge(*u, *v, *c, *w).unwrap(); } }
            prop_assert_eq!(g2.solve(0, 5, i64::MAX).unwrap(), r);
        }
    }

    fn network() -> SupplyNetwork {
        let mut n = SupplyNetwork::default();
        let site = |id, kind, stock: f64, tp| Site { id: SiteId(id), kind, stock_t: [(SupplyClass::III, stock)].into(), throughput_t_per_day: tp, damage: 0.0 };
        n.add_site(site(1, SiteKind::Railhead, 1_000.0, 800.0));
        n.add_site(site(2, SiteKind::Depot, 200.0, 300.0));
        n.add_site(site(3, SiteKind::SupplyPoint, 0.0, 200.0));
        n.add_site(site(4, SiteKind::SupplyPoint, 0.0, 200.0));
        let link = |id, a, b, mode, cap, h, c| SupplyLink { id: LinkId(id), from: SiteId(a), to: SiteId(b), mode, capacity_t_per_day: cap, transit_h: h, cost_per_t: c, damage: 0.0 };
        n.add_link(link(1, 1, 2, LinkMode::Rail, 500.0, 6.0, 1.0)).unwrap();
        n.add_link(link(2, 2, 3, LinkMode::Road, 150.0, 3.0, 2.0)).unwrap();
        n.add_link(link(3, 2, 4, LinkMode::Road, 150.0, 4.0, 2.0)).unwrap();
        n.add_link(link(4, 1, 3, LinkMode::Road, 60.0, 12.0, 5.0)).unwrap();
        n
    }

    #[test]
    fn supply_plan_meets_demand_cheaply_and_executes() {
        let f = figs();
        let mut n = network();
        let demand: BTreeMap<SiteId, f64> = [(SiteId(3), 120.0), (SiteId(4), 100.0)].into();
        let plan = n.plan_cycle(SupplyClass::III, &demand, 1.0, &f).unwrap();
        assert!(plan.shortfall_t.is_empty(), "{plan:?}");
        assert_relative_eq!(plan.delivered_t[&SiteId(3)], 120.0, epsilon = 1e-9);
        // Depot stock (closest) is drawn first.
        assert_relative_eq!(plan.drawn_t[&SiteId(2)], 200.0, epsilon = 1e-9);
        let total: f64 = plan.paths.iter().map(|p| p.tonnes).sum();
        assert_relative_eq!(total, 220.0, epsilon = 1e-9);
        let moves = n.dispatch(&plan, SimTime::ZERO, &f);
        assert!(moves.iter().all(|(_, legs)| legs.iter().all(|m| m.arrive > m.depart && m.vehicles >= 1 && m.packets <= m.vehicles)));
        let convoy = moves.iter().flat_map(|(_, l)| l).find(|m| m.kind == MovementKind::Convoy).unwrap();
        assert_eq!(convoy.packets, (convoy.vehicles as f64 / 10.0).ceil() as u32);
        assert_relative_eq!(n.sites[&SiteId(2)].stock_t[&SupplyClass::III], 0.0, epsilon = 1e-9);
        // Strike the depot: next cycle must route around it at higher cost.
        n.sites.get_mut(&SiteId(2)).unwrap().stock_t.insert(SupplyClass::III, 100.0);
        let e = n.strike_site(SiteId(2), 0.9, &f).unwrap();
        assert!(e.damage_after > 0.8 && e.stock_lost_t > 80.0);
        let plan2 = n.plan_cycle(SupplyClass::III, &demand, 1.0, &f).unwrap();
        let short: f64 = plan2.shortfall_t.values().sum();
        assert!(short > 100.0, "{plan2:?}");
        assert!(plan2.flows.iter().any(|fl| fl.link == LinkId(4)));
        n.repair(30.0, &f);
        assert!(n.sites[&SiteId(2)].damage < 0.1);
    }

    #[test]
    fn hubs_pull_back_to_about_120_km() {
        let f = figs();
        let h = hub_standoff(84.0, &f);
        assert_relative_eq!(h.throughput_factor, 1.0);
        assert!(hub_standoff(300.0, &f).throughput_factor < 0.5);
        assert_relative_eq!(h.distance_km, 120.0, epsilon = 1e-9);
        assert_relative_eq!(rail_distance_factor(100.0, &f), 1.0);
        assert!(rail_distance_factor(190.0, &f) < 0.6);
        assert!(rail_distance_factor(1_000.0, &f) >= 0.1);
    }

    #[test]
    fn consumption_from_activity() {
        let f = figs();
        // Abrams-class: 8 h of mixed use ~ 300 gal (1,136 L) [P17].
        let a = Activity { km: vec![(Terrain::CrossCountry, 80.0), (Terrain::Road, 60.0)], idle_h: 4.0, person_days: 4.0, ..Default::default() };
        let c = consumption(VehicleClass::Mbt, &a, &f);
        assert!((700.0..1_500.0).contains(&c.fuel_l), "{}", c.fuel_l);
        assert_relative_eq!(c.water_l, 80.0);
        let road = consumption(VehicleClass::Mbt, &Activity { km: vec![(Terrain::Road, 10.0)], ..Default::default() }, &f);
        let mud = consumption(VehicleClass::Mbt, &Activity { km: vec![(Terrain::Mud, 10.0)], ..Default::default() }, &f);
        assert!(mud.fuel_l > 2.0 * road.fuel_l);
        let heli = consumption(VehicleClass::Helicopter, &Activity { flight_h: 2.0, drone_sorties: 3.0, ..Default::default() }, &f);
        assert_relative_eq!(heli.fuel_l, 1_200.0);
        assert_relative_eq!(heli.batteries, 3.0);
    }

    /// Acceptance [P19]: with positions spread through 0–20 km and heavier
    /// demand forward, drones and UGVs carry 50–80% of deliveries.
    #[test]
    fn last_mile_is_mostly_uncrewed() {
        let f = figs();
        let positions: Vec<(f64, f64)> = (0..20).map(|i| (i as f64 + 0.5, if i < 10 { 400.0 } else { 250.0 })).collect();
        let fleet = LastMileFleet { drones: 12, ugvs: 6, hours_per_day: 16.0 };
        let p = last_mile_plan(&positions, &fleet, &f);
        assert!((0.5..=0.8).contains(&p.uncrewed_share), "{p:?}");
        assert_eq!(p.undelivered_kg, 0.0);
        assert!(p.expected_ugv_losses > 0.0);
        // A small fleet can't carry it all.
        let small = last_mile_plan(&positions, &LastMileFleet { drones: 1, ugvs: 1, hours_per_day: 8.0 }, &f);
        assert!(small.undelivered_kg > 0.0);
    }

    #[test]
    fn maintenance_queues_recovery_and_spares() {
        let f = figs();
        assert!(failure_probability(VehicleClass::Mbt, 150.0, &f) > 0.6);
        assert!(failure_probability(VehicleClass::Truck, 10.0, &f) < 0.03);
        let mut shop = MaintenanceShop::new(1, &[(Echelon::Field, 1), (Echelon::Support, 1)], 1);
        let job = |v, e, rec, at| RepairJob { vehicle: EntityId(v), echelon: e, needs_recovery: rec, distance_km: 7.5, at: SimTime::from_secs_i(at) };
        shop.submit(job(1, Echelon::Field, true, 0));
        shop.submit(job(2, Echelon::Field, false, 0));
        shop.submit(job(3, Echelon::Support, false, 10));
        let done = shop.step(SimTime::from_secs_i(3_600), &f);
        assert_eq!(done.len(), 1, "{done:?}");
        // Recovery: 2 × 7.5 km at 15 km/h = 1 h; repair 12 h.
        assert_eq!(done[0].recovered_at, SimTime::from_secs_i(3_600));
        assert_eq!(done[0].ready_at, SimTime::from_secs_i(3_600 + 12 * 3_600));
        assert_eq!(shop.waiting(), 2);
        shop.add_spares(4);
        let done = shop.step(SimTime::from_secs_i(3_600), &f);
        assert_eq!(done.len(), 2);
        // Vehicle 2 waits for the single field team.
        assert_eq!(done[0].repair_start, SimTime::from_secs_i(3_600 + 12 * 3_600));
    }
}
