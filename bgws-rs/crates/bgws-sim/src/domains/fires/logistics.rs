//! The logistics network, convoys, interdiction, culmination and maintenance
//! (§8.11, the TypeScript `logistics.ts`; research R7, P17–P19).
//!
//! - **Network:** each side's depots, railheads and ports (node entities with
//!   stores) are sites of a `bgws_ops::logistics::SupplyNetwork`, railheads
//!   joined to depots by rail. Each cycle the units that asked for resupply
//!   become sinks joined to the nodes by road (or, by the cycle's decision,
//!   by robot or drone for the last mile), depots that have drawn down ask
//!   their railhead, and the flows of ammunition (class V) and fuel (III) are
//!   planned as a min-cost flow. The node's logistics cell is asked how the
//!   cycle goes (`resupplyRequests`).
//! - **Convoys:** a road flow to a unit is carried by one of the side's
//!   convoy trucks, an entity that drives to the node, loads, drives to the
//!   unit and back. It can be seen and shot; a truck lost loses its load. A
//!   route past known enemies asks the convoy (`routeThreatened`). With no
//!   trucks on the board the load arrives after the transit time.
//! - **Interdiction:** a node that loses strength loses that share of its
//!   stock and throughput (`strike_site`) and asks (`nodeStruck`).
//! - **Culmination:** a side whose combat units average less fuel or
//!   ammunition than the threshold has culminated (logged once).
//! - **Maintenance:** vehicles fail by operating hours (MTBF by class) and
//!   are repaired, like battle damage (mobility and firepower kills), by the
//!   side's `MaintenanceShop` with its recovery vehicles, teams and spares.

use super::sustain::{need, refill};
use super::{alive, is_node, name, raise, secs_to_dark, FiresWorld, Stores, Tally};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_catalogue::Domain;
use bgws_core::{Enu, EntityId, Side, SimDuration, SimTime};
use bgws_ops::logistics::{failure_probability, Echelon, LinkId, LinkMode, MaintenanceShop, RepairJob, Site, SiteId, SiteKind, SupplyClass, SupplyLink, SupplyNetwork};
use bgws_scenario::scope::DomainMode;
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};

/// Delivery ids start here, so another domain's `Delivery` is never ours.
pub const ID_BASE: u64 = 1 << 41;

pub fn owns(flow: u64) -> bool {
    (ID_BASE..2 * ID_BASE).contains(&flow)
}

const UNIT_SITE: u32 = 1_000_000;
const DYN_LINK: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Means {
    Convoy,
    Ugv,
    Drone,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub entity: EntityId,
    pub side: Side,
    pub site: SiteId,
    pub kind: SiteKind,
    pub dispersed: bool,
    /// Stock at the start (what a depot is topped back up to).
    pub initial: BTreeMap<SupplyClass, f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leg {
    ToSource,
    ToUnit,
    Return,
}

/// A convoy truck's run.
#[derive(Clone, Debug)]
pub struct Job {
    pub truck: EntityId,
    pub side: Side,
    pub source: EntityId,
    pub unit: EntityId,
    pub v_t: f64,
    pub iii_t: f64,
    pub leg: Leg,
    pub wait_until: Option<SimTime>,
    pub detour: bool,
    pub issued: SimTime,
}

#[derive(Clone, Debug)]
pub enum DeliverTo {
    Unit(EntityId),
    Site(Side, SiteId),
}

#[derive(Clone, Debug)]
pub struct Delivery {
    pub to: DeliverTo,
    pub v_t: f64,
    pub iii_t: f64,
    /// Chance the carrier is lost on the way (robots, drones).
    pub loss_p: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepairKind {
    Mobility,
    Firepower,
}

#[derive(Default)]
pub struct LogisticsState {
    pub nets: BTreeMap<Side, SupplyNetwork>,
    pub nodes: BTreeMap<EntityId, Node>,
    pub requests: BTreeMap<Side, BTreeSet<EntityId>>,
    pub jobs: BTreeMap<u64, Job>,
    pub deliveries: BTreeMap<u64, Delivery>,
    pub next: u64,
    pub last_cycle: Option<SimTime>,
    /// A cycle whose means is being decided, by side: (since, the node asked).
    pub deciding: BTreeMap<Side, (SimTime, EntityId)>,
    /// Cycles held for dark.
    pub dark: BTreeMap<Side, SimTime>,
    pub culminated: BTreeSet<Side>,
    shops: BTreeMap<Side, MaintenanceShop>,
    repair_kinds: BTreeMap<EntityId, Vec<RepairKind>>,
    repairs: Vec<(SimTime, EntityId)>,
    /// Failures caused here (so the loss tracker does not count them as battle damage).
    pub failures: BTreeMap<EntityId, u32>,
}

fn site_kind(p: &bgws_catalogue::Platform) -> Option<SiteKind> {
    let t = format!("{} {} {}", p.id, p.class, p.role).to_lowercase();
    if !is_node(p) {
        return None;
    }
    if t.contains("railhead") {
        Some(SiteKind::Railhead)
    } else if t.contains("depot") {
        Some(SiteKind::Depot)
    } else if t.contains("port") {
        Some(SiteKind::Port)
    } else {
        None
    }
}

fn site_of(id: EntityId) -> SiteId {
    SiteId(id.0)
}

pub fn setup(s: &mut Sim, st: &mut FiresWorld) {
    let p = st.p.clone();
    for &e in &s.ctx.order.clone() {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        let Some(kind) = site_kind(&i.platform) else { continue };
        let (id, side) = (i.id, i.side);
        let stores = &i.platform.stores;
        let mut v = 0.0;
        for (mun, n) in &stores.ammo {
            let fam = s.ctx.catalogue.munition(mun).map(|m| m.family.clone()).unwrap_or_default();
            v += n * p.kg_per_round(&fam) / 1000.0;
        }
        let iii = stores.fuel_l * p.fuel_kg_per_l / 1000.0;
        let initial: BTreeMap<SupplyClass, f64> = [(SupplyClass::V, v), (SupplyClass::III, iii)].into_iter().collect();
        let through = match kind {
            SiteKind::Railhead | SiteKind::Port => p.railhead_t_per_day,
            _ => p.depot_t_per_day,
        };
        st.logistics.nets.entry(side).or_default().add_site(Site { id: site_of(id), kind, stock_t: initial.clone(), throughput_t_per_day: through, damage: 0.0 });
        st.logistics.nodes.insert(id, Node { entity: id, side, site: site_of(id), kind, dispersed: false, initial });
    }
    // Rail from every railhead to every depot of its side.
    let nodes: Vec<Node> = st.logistics.nodes.values().cloned().collect();
    let mut link = 0u32;
    for a in nodes.iter().filter(|n| n.kind == SiteKind::Railhead) {
        for b in nodes.iter().filter(|n| n.kind == SiteKind::Depot && n.side == a.side) {
            let d = pos_of(s, a.entity).sub(pos_of(s, b.entity)).horizontal() / 1000.0;
            let l = SupplyLink { id: LinkId(link), from: a.site, to: b.site, mode: LinkMode::Rail, capacity_t_per_day: p.rail_t_per_day, transit_h: d / p.rail_kmh, cost_per_t: 1.0, damage: 0.0 };
            link += 1;
            let _ = st.logistics.nets.get_mut(&a.side).expect("net").add_link(l);
        }
    }
    // Maintenance: a shop per side, its spares from the side's nodes.
    for side in Side::PLAYERS {
        let spares: f64 = s.ctx.order.iter().filter_map(|&e| s.world.get::<Ident>(e)).filter(|i| i.side == side && is_node(&i.platform)).map(|i| i.platform.stores.spares).sum();
        let spares = if spares > 0.0 { spares as u32 } else { p.maint_spares };
        st.logistics.shops.insert(side, MaintenanceShop::new(p.maint_recovery, &[(Echelon::Field, p.maint_field_teams.max(1))], spares));
    }
}

fn pos_of(s: &Sim, id: EntityId) -> Enu {
    s.world.get::<Kin>(s.ctx.entity(id)).map(|k| k.pos).unwrap_or_default()
}

/// The side's nearest standing supply node.
pub fn nearest_node(s: &Sim, st: &FiresWorld, side: Side, near: Enu) -> Option<EntityId> {
    st.logistics.nodes.values().filter(|n| n.side == side && alive(s, s.ctx.entity(n.entity))).min_by(|a, b| pos_of(s, a.entity).sub(near).horizontal().total_cmp(&pos_of(s, b.entity).sub(near).horizontal()).then(a.entity.cmp(&b.entity))).map(|n| n.entity)
}

pub(crate) fn schedule_delivery(s: &mut Sim, st: &mut FiresWorld, to: DeliverTo, v_t: f64, iii_t: f64, at: SimTime, loss_p: f64) {
    let id = ID_BASE + st.logistics.next;
    st.logistics.next += 1;
    st.logistics.deliveries.insert(id, Delivery { to, v_t, iii_t, loss_p });
    s.ctx.queue.schedule(at, Priority::Physical, Scheduled::Delivery { flow: id });
}

/// A scheduled load arrives (off-board convoys, robots, drones, trains, abstracted resupply).
pub fn delivery(s: &mut Sim, st: &mut FiresWorld, id: u64) {
    let Some(d) = st.logistics.deliveries.remove(&id) else { return };
    let now = s.ctx.now;
    if d.loss_p > 0.0 {
        let mut rng = s.ctx.rng("logistics.lastMile", id, 0);
        if rng.gen::<f64>() < d.loss_p {
            s.ctx.log.push(now, "logistics.lost").text("a last-mile robot or drone was lost with its load").data(serde_json::json!({ "vT": d.v_t, "iiiT": d.iii_t }));
            return;
        }
    }
    match d.to {
        DeliverTo::Unit(u) => hand_over(s, st, u, d.v_t, d.iii_t, None),
        DeliverTo::Site(side, site) => {
            if let Some(net) = st.logistics.nets.get_mut(&side) {
                net.receive(site, SupplyClass::V, d.v_t);
                net.receive(site, SupplyClass::III, d.iii_t);
            }
            s.ctx.log.push(now, "logistics.restock").side(side).text(format!("a train restocks a depot: {:.1} t ammunition, {:.1} t fuel", d.v_t, d.iii_t)).data(serde_json::json!({ "site": site.0, "vT": d.v_t, "iiiT": d.iii_t }));
        }
    }
}

fn hand_over(s: &mut Sim, st: &mut FiresWorld, unit: EntityId, v_t: f64, iii_t: f64, by: Option<EntityId>) {
    let e = s.ctx.entity(unit);
    if !alive(s, e) {
        return;
    }
    let (rounds, litres) = refill(s, st, e, v_t, iii_t);
    let side = s.world.get::<Ident>(e).expect("i").side;
    let (nv, niii) = need(s, st, e);
    if nv < 0.05 && niii < 0.05 {
        if let Some(r) = st.logistics.requests.get_mut(&side) {
            r.remove(&unit);
        }
    }
    let text = format!("{} is resupplied: {rounds} rounds, {:.0} L fuel", name(s, unit), litres);
    let b = s.ctx.log.push(s.ctx.now, "logistics.resupplied").side(side).entity(unit).text(text).data(serde_json::json!({ "rounds": rounds, "litres": litres, "vT": v_t, "iiiT": iii_t, "by": by.map(|x| x.0) }));
    drop(b);
}

// ── Each second: convoys on the move ─────────────────────────────────────

pub fn base(s: &mut Sim, st: &mut FiresWorld) {
    if st.logistics_mode != DomainMode::Simulated {
        return;
    }
    let now = s.ctx.now;
    // Cycles whose decision never came go by convoy; held cycles go at dark.
    let overdue: Vec<Side> = st.logistics.deciding.iter().filter(|(_, (t, _))| (now - *t).secs() >= st.p.decide_wait_min * 60.0).map(|(s, _)| *s).collect();
    for side in overdue {
        st.logistics.deciding.remove(&side);
        execute(s, st, side, Means::Convoy);
    }
    let dark: Vec<Side> = st.logistics.dark.iter().filter(|(_, t)| **t <= now).map(|(s, _)| *s).collect();
    for side in dark {
        st.logistics.dark.remove(&side);
        execute(s, st, side, Means::Convoy);
    }
    convoys(s, st);
}

fn convoys(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    let ids: Vec<u64> = st.logistics.jobs.keys().copied().collect();
    for jid in ids {
        let job = st.logistics.jobs[&jid].clone();
        let te = s.ctx.entity(job.truck);
        if !alive(s, te) {
            st.logistics.jobs.remove(&jid);
            let text = format!("{} is lost with {:.1} t aboard", name(s, job.truck), job.v_t + job.iii_t);
            s.ctx.log.push(now, "logistics.convoyLost").side(job.side).entity(job.truck).target(job.unit).text(text).data(serde_json::json!({ "vT": job.v_t, "iiiT": job.iii_t }));
            continue;
        }
        if job.wait_until.map(|t| t > now).unwrap_or(false) {
            continue;
        }
        let tpos = pos_of(s, job.truck);
        let dest = match job.leg {
            Leg::ToSource | Leg::Return => pos_of(s, job.source),
            Leg::ToUnit => pos_of(s, job.unit),
        };
        if tpos.sub(dest).horizontal() <= st.p.unload_m {
            match job.leg {
                Leg::ToSource => {
                    // Loaded; the truck refuels itself at the node.
                    if let Some(mut x) = s.world.get_mut::<Stores>(te) {
                        x.fuel_l = x.fuel_max_l;
                    }
                    st.logistics.jobs.get_mut(&jid).expect("job").leg = Leg::ToUnit;
                }
                Leg::ToUnit => {
                    hand_over(s, st, job.unit, job.v_t, job.iii_t, Some(job.truck));
                    let j = st.logistics.jobs.get_mut(&jid).expect("job");
                    j.v_t = 0.0;
                    j.iii_t = 0.0;
                    j.leg = Leg::Return;
                }
                Leg::Return => {
                    st.logistics.jobs.remove(&jid);
                    if let Some(mut x) = s.world.get_mut::<Stores>(te) {
                        x.fuel_l = x.fuel_max_l;
                    }
                    if let Some(mut a) = s.world.get_mut::<Act>(te) {
                        a.activity = Activity::Hold;
                    }
                }
            }
            continue;
        }
        if job.leg == Leg::ToUnit && !alive(s, s.ctx.entity(job.unit)) {
            st.logistics.jobs.get_mut(&jid).expect("job").leg = Leg::Return;
            continue;
        }
        // Keep it driving (its own decisions may have stopped it), re-aiming at a moving unit each minute.
        let moving = s.world.get::<Act>(te).map(|a| a.activity.moving()).unwrap_or(false);
        let stale = (now - job.issued).secs() >= 60.0 && job.leg == Leg::ToUnit;
        if !moving || stale {
            let mut route = crate::decide::route_for(s, te, dest);
            if job.detour {
                let mid = tpos.add(dest).scale(0.5);
                let d = dest.sub(tpos);
                let side_v = Enu::new(d.n, -d.e, 0.0).scale(st.p.detour_m / d.horizontal().max(1.0));
                route = vec![mid.add(side_v), dest];
            }
            if let Some(mut a) = s.world.get_mut::<Act>(te) {
                a.activity = Activity::Move { route, mode: MoveModeS::March, then: Some(Box::new(Activity::Hold)) };
            }
            st.logistics.jobs.get_mut(&jid).expect("job").issued = now;
        }
    }
}

// ── Each minute: the cycle, interdiction, culmination, maintenance ───────

pub fn minute(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    culmination(s, st);
    if st.logistics_mode != DomainMode::Simulated {
        return;
    }
    let now = s.ctx.now;
    maintenance(s, st, dt);
    let due = st.logistics.last_cycle.map(|t| (now - t).secs() >= st.p.cycle_min * 60.0).unwrap_or(true);
    if !due {
        return;
    }
    st.logistics.last_cycle = Some(now);
    let days = st.p.cycle_min / 1440.0;
    let figs = s.ctx.figures.clone();
    for net in st.logistics.nets.values_mut() {
        net.repair(days, &figs);
    }
    for side in Side::PLAYERS {
        if !st.logistics.nets.contains_key(&side) || st.logistics.deciding.contains_key(&side) || st.logistics.dark.contains_key(&side) {
            continue;
        }
        let asking = st.logistics.requests.get(&side).map(|r| !r.is_empty()).unwrap_or(false);
        if !asking {
            // Depots still draw on their railhead between requests.
            execute(s, st, side, Means::Convoy);
            continue;
        }
        let node = st.logistics.nodes.values().filter(|n| n.side == side && n.kind == SiteKind::Depot && alive(s, s.ctx.entity(n.entity))).map(|n| n.entity).next().or_else(|| nearest_node(s, st, side, Enu::default()));
        let Some(node) = node else {
            execute(s, st, side, Means::Convoy);
            continue;
        };
        let threat = route_threatened(s, st, side, pos_of(s, node), st.logistics.requests[&side].iter().map(|u| pos_of(s, *u)).collect());
        let rule = if threat && !super::is_night(s, now) { "dark" } else { "convoy" };
        if raise(s, node, "resupplyRequests", None, Some(rule)) {
            st.logistics.deciding.insert(side, (now, node));
        } else {
            execute(s, st, side, Means::Convoy);
        }
    }
}

/// Is any known enemy near the line from `from` to any of `to`?
fn route_threatened(s: &Sim, st: &FiresWorld, side: Side, from: Enu, to: Vec<Enu>) -> bool {
    let tracks = s.ctx.tracks[&side].confirmed(&s.ctx.p.know);
    to.iter().any(|b| {
        tracks.iter().any(|t| {
            let p = t.position;
            let ab = b.sub(from);
            let len2 = (ab.e * ab.e + ab.n * ab.n).max(1e-9);
            let k = (((p.e - from.e) * ab.e + (p.n - from.n) * ab.n) / len2).clamp(0.0, 1.0);
            let q = from.add(ab.scale(k));
            p.sub(q).horizontal() <= st.p.threat_m
        })
    })
}

/// Plan one cycle's flows for a side and set them moving.
pub(crate) fn execute(s: &mut Sim, st: &mut FiresWorld, side: Side, means: Means) {
    let now = s.ctx.now;
    let p = st.p.clone();
    let figs = s.ctx.figures.clone();
    let Some(mut net) = st.logistics.nets.remove(&side) else { return };
    net.sites.retain(|id, _| id.0 < UNIT_SITE);
    net.links.retain(|id, _| id.0 < DYN_LINK);
    let sources: Vec<Node> = st.logistics.nodes.values().filter(|n| n.side == side && n.kind == SiteKind::Depot && alive(s, s.ctx.entity(n.entity))).cloned().collect();
    let sources = if sources.is_empty() { st.logistics.nodes.values().filter(|n| n.side == side && alive(s, s.ctx.entity(n.entity))).cloned().collect() } else { sources };
    let mut demand: BTreeMap<SupplyClass, BTreeMap<SiteId, f64>> = BTreeMap::new();
    let mut link = DYN_LINK;
    let units: Vec<EntityId> = st.logistics.requests.get(&side).map(|r| r.iter().copied().collect()).unwrap_or_default();
    let busy: BTreeSet<EntityId> = st.logistics.jobs.values().map(|j| j.unit).collect();
    for u in &units {
        let ue = s.ctx.entity(*u);
        if !alive(s, ue) {
            st.logistics.requests.get_mut(&side).expect("r").remove(u);
            continue;
        }
        if busy.contains(u) {
            continue;
        }
        let (v, iii) = need(s, st, ue);
        if v < 0.05 && iii < 0.05 {
            st.logistics.requests.get_mut(&side).expect("r").remove(u);
            continue;
        }
        let site = SiteId(UNIT_SITE + u.0);
        net.add_site(Site { id: site, kind: SiteKind::Unit, stock_t: BTreeMap::new(), throughput_t_per_day: 1e6, damage: 0.0 });
        for n in &sources {
            let d = pos_of(s, n.entity).sub(pos_of(s, *u)).horizontal() / 1000.0;
            let (mode, cap, transit) = match means {
                Means::Convoy => (LinkMode::Road, p.road_t_per_day, d / p.convoy_kmh),
                Means::Ugv => (LinkMode::Ugv, p.ugv_fleet * figs.get("log.payload.ugv_t") * 24.0 / figs.get("log.ugv.run_h"), figs.get("log.ugv.run_h") / 2.0),
                Means::Drone => (LinkMode::Drone, p.drone_fleet * figs.get("log.payload.drone_t") * 24.0 / figs.get("log.drone.sortie_h"), figs.get("log.drone.sortie_h") / 2.0),
            };
            let _ = net.add_link(SupplyLink { id: LinkId(link), from: n.site, to: site, mode, capacity_t_per_day: cap, transit_h: transit, cost_per_t: 1.0, damage: 0.0 });
            link += 1;
        }
        demand.entry(SupplyClass::V).or_default().insert(site, v);
        demand.entry(SupplyClass::III).or_default().insert(site, iii);
    }
    // Depots draw back up to their starting stock from the railhead.
    for n in st.logistics.nodes.values().filter(|n| n.side == side && n.kind == SiteKind::Depot) {
        for class in [SupplyClass::V, SupplyClass::III] {
            let have = net.sites.get(&n.site).and_then(|x| x.stock_t.get(&class).copied()).unwrap_or(0.0);
            let want = n.initial.get(&class).copied().unwrap_or(0.0) - have;
            if want > 1.0 && net.links.values().any(|l| l.to == n.site) {
                demand.entry(class).or_default().insert(n.site, want);
            }
        }
    }
    if demand.is_empty() {
        st.logistics.nets.insert(side, net);
        return;
    }
    // Plan and draw, class by class; collect each unit's load by source.
    let mut loads: BTreeMap<(EntityId, EntityId), (f64, f64, f64)> = BTreeMap::new();
    for (class, d) in &demand {
        let plan = match net.plan_cycle(*class, d, p.cycle_min / 1440.0, &figs) {
            Ok(x) => x,
            Err(err) => {
                s.ctx.log.push(now, "logistics.error").side(side).text(format!("plan failed: {err}"));
                continue;
            }
        };
        let moves = net.dispatch(&plan, now, &figs);
        let delivered: f64 = plan.delivered_t.values().sum();
        let short: f64 = plan.shortfall_t.values().sum();
        let movements: Vec<serde_json::Value> = moves.iter().flat_map(|(_, legs)| legs.iter().map(|m| serde_json::json!({ "kind": format!("{:?}", m.kind), "tonnes": m.tonnes, "vehicles": m.vehicles }))).collect();
        s.ctx.log.push(now, "logistics.plan").side(side).text(format!("{side} plans {class:?}: {delivered:.1} t to move, {short:.1} t short")).data(serde_json::json!({
            "class": format!("{class:?}"), "deliveredT": delivered, "shortfallT": short, "cost": plan.total_cost, "movements": movements, "means": format!("{means:?}"),
        }));
        for (path, _) in moves {
            if path.sink.0 >= UNIT_SITE {
                let unit = EntityId(path.sink.0 - UNIT_SITE);
                let e = loads.entry((EntityId(path.source.0), unit)).or_insert((0.0, 0.0, path.transit_h));
                match class {
                    SupplyClass::V => e.0 += path.tonnes,
                    _ => e.1 += path.tonnes,
                }
                e.2 = e.2.max(path.transit_h);
            } else {
                // A train to a depot.
                let (v, iii) = if *class == SupplyClass::V { (path.tonnes, 0.0) } else { (0.0, path.tonnes) };
                let at = now + SimDuration::from_hours(path.transit_h.max(0.1));
                st.logistics.nets.insert(side, net.clone());
                schedule_delivery(s, st, DeliverTo::Site(side, path.sink), v, iii, at, 0.0);
                net = st.logistics.nets.remove(&side).expect("net");
            }
        }
    }
    st.logistics.nets.insert(side, net);
    // Set the loads moving.
    for ((source, unit), (v, iii, transit_h)) in loads {
        match means {
            Means::Convoy => {
                let truck = free_truck(s, st, side, pos_of(s, source));
                match truck {
                    Some(t) => {
                        let jid = ID_BASE + st.logistics.next;
                        st.logistics.next += 1;
                        let at_source = pos_of(s, t).sub(pos_of(s, source)).horizontal() <= p.unload_m;
                        st.logistics.jobs.insert(jid, Job { truck: t, side, source, unit, v_t: v, iii_t: iii, leg: if at_source { Leg::ToUnit } else { Leg::ToSource }, wait_until: None, detour: false, issued: SimTime(i64::MIN / 2) });
                        let text = format!("{} sets off for {} with {:.1} t", name(s, t), name(s, unit), v + iii);
                        s.ctx.log.push(now, "logistics.convoy").side(side).entity(t).target(unit).text(text).data(serde_json::json!({ "vT": v, "iiiT": iii, "source": source.0 }));
                        if route_threatened(s, st, side, pos_of(s, source), vec![pos_of(s, unit)]) {
                            raise(s, t, "routeThreatened", Some(unit), Some("alternate"));
                        }
                    }
                    None => {
                        let at = now + SimDuration::from_hours(transit_h.max(0.1));
                        schedule_delivery(s, st, DeliverTo::Unit(unit), v, iii, at, 0.0);
                    }
                }
            }
            Means::Ugv | Means::Drone => {
                let loss = if means == Means::Ugv { figs.get("log.ugv.loss_per_run") } else { figs.get("log.drone.loss_per_sortie") };
                let at = now + SimDuration::from_hours(transit_h.max(0.05));
                schedule_delivery(s, st, DeliverTo::Unit(unit), v, iii, at, loss);
            }
        }
    }
}

fn free_truck(s: &Sim, st: &FiresWorld, side: Side, near: Enu) -> Option<EntityId> {
    let busy: BTreeSet<EntityId> = st.logistics.jobs.values().map(|j| j.truck).collect();
    s.ctx
        .order
        .iter()
        .filter(|&&e| alive(s, e))
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            let cargo = s.world.get::<Stores>(e)?.carrier_t;
            if i.side != side || i.domain != Domain::Land || cargo < 5.0 || busy.contains(&i.id) || !s.world.get::<Health>(e)?.can_move() {
                return None;
            }
            Some((s.world.get::<Kin>(e)?.pos.sub(near).horizontal(), i.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|x| x.1)
}

/// Losses this tick: nodes struck lose stock and throughput; damage goes to the shop.
pub(crate) fn on_losses(s: &mut Sim, st: &mut FiresWorld, losses: &[super::medical::Loss]) {
    let now = s.ctx.now;
    for l in losses {
        if let Some(node) = st.logistics.nodes.get(&l.id).cloned() {
            if l.lost == 0 {
                continue;
            }
            let frac = (l.lost as f64 / l.start.max(1) as f64).min(1.0) * if node.dispersed { st.p.disperse_factor } else { 1.0 };
            let figs = s.ctx.figures.clone();
            let Some(net) = st.logistics.nets.get_mut(&node.side) else { continue };
            let Ok(fx) = net.strike_site(node.site, if l.destroyed { 1.0 } else { frac }, &figs) else { continue };
            let text = format!("{} is struck: {:.0} t of stock lost, throughput down to {:.0}%", name(s, l.id), fx.stock_lost_t, (1.0 - fx.damage_after) * 100.0);
            s.ctx.log.push(now, "logistics.nodeStruck").side(node.side).entity(l.id).text(text).data(serde_json::to_value(&fx).unwrap_or_default());
            if !l.destroyed {
                raise(s, l.id, "nodeStruck", None, Some(if fx.damage_after >= 0.5 { "relocate" } else { "disperse" }));
            }
            continue;
        }
        if st.logistics_mode != DomainMode::Simulated || l.destroyed {
            continue;
        }
        let side = l.side;
        let failures = st.logistics.failures.remove(&l.id).unwrap_or(0);
        let mob = l.mobility_new.saturating_sub(failures);
        for (n, kind) in [(mob, RepairKind::Mobility), (l.firepower_new, RepairKind::Firepower)] {
            for _ in 0..n {
                submit(st, side, l.id, kind, now);
            }
        }
    }
}

fn submit(st: &mut FiresWorld, side: Side, vehicle: EntityId, kind: RepairKind, at: SimTime) {
    if let Some(shop) = st.logistics.shops.get_mut(&side) {
        shop.submit(RepairJob { vehicle, echelon: Echelon::Field, needs_recovery: true, distance_km: st.p.maint_recovery_km, at });
        st.logistics.repair_kinds.entry(vehicle).or_default().push(kind);
    }
}

/// Failures by operating hours; repairs started and finished.
fn maintenance(s: &mut Sim, st: &mut FiresWorld, _dt: f64) {
    let now = s.ctx.now;
    let figs = s.ctx.figures.clone();
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let Some(stores) = s.world.get::<Stores>(e).cloned() else { continue };
        let Some(class) = stores.class else { continue };
        if stores.op_hours <= 0.0 {
            continue;
        }
        s.world.get_mut::<Stores>(e).expect("stores").op_hours = 0.0;
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("i");
            (i.id, i.side)
        };
        let (fit, mob) = {
            let h = s.world.get::<Health>(e).expect("h");
            (h.fit, h.mobility_kills)
        };
        let p = failure_probability(class, stores.op_hours, &figs);
        let mut rng = s.ctx.rng("maint.fail", id.0 as u64, 0);
        let failed = (0..fit.saturating_sub(mob)).filter(|_| rng.gen::<f64>() < p).count() as u32;
        if failed == 0 {
            continue;
        }
        s.world.get_mut::<Health>(e).expect("h").mobility_kills += failed;
        if let Some(mut t) = s.world.get_mut::<Tally>(e) {
            t.mobility += failed;
        }
        for _ in 0..failed {
            submit(st, side, id, RepairKind::Mobility, now);
        }
        let text = format!("{}: {failed} vehicle(s) break down", name(s, id));
        s.ctx.log.push(now, "maint.failure").side(side).entity(id).text(text).data(serde_json::json!({ "vehicles": failed, "p": p }));
    }
    for shop in st.logistics.shops.values_mut() {
        for done in shop.step(now, &figs) {
            st.logistics.repairs.push((done.ready_at, done.vehicle));
        }
    }
    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut st.logistics.repairs).into_iter().partition(|(t, _)| *t <= now);
    st.logistics.repairs = later;
    for (_, id) in due {
        let kind = st.logistics.repair_kinds.get_mut(&id).and_then(|v| (!v.is_empty()).then(|| v.remove(0)));
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        {
            let mut h = s.world.get_mut::<Health>(e).expect("h");
            match kind {
                Some(RepairKind::Firepower) => h.firepower_kills = h.firepower_kills.saturating_sub(1),
                _ => h.mobility_kills = h.mobility_kills.saturating_sub(1),
            }
        }
        let h = s.world.get::<Health>(e).expect("h").clone();
        if let Some(mut t) = s.world.get_mut::<Tally>(e) {
            t.mobility = h.mobility_kills;
            t.firepower = h.firepower_kills;
        }
        let side = s.world.get::<Ident>(e).expect("i").side;
        let text = format!("{}: a vehicle is repaired", name(s, id));
        s.ctx.log.push(now, "maint.repaired").side(side).entity(id).text(text).data(serde_json::json!({ "kind": format!("{kind:?}") }));
    }
}

/// A side whose combat units average less fuel or ammunition than the threshold has culminated.
fn culmination(s: &mut Sim, st: &mut FiresWorld) {
    if st.logistics_mode == DomainMode::Off {
        return;
    }
    for side in Side::PLAYERS {
        if st.logistics.culminated.contains(&side) {
            continue;
        }
        let (mut fuel, mut nf, mut ammo, mut na) = (0.0, 0.0, 0.0, 0.0);
        for &e in &s.ctx.order {
            let (Some(i), Some(x), Some(arms)) = (s.world.get::<Ident>(e), s.world.get::<Stores>(e), s.world.get::<Arms>(e)) else { continue };
            if i.side != side || !alive(s, e) || x.carrier_t > 0.0 || i.hq {
                continue;
            }
            if x.fuel_max_l > 0.0 {
                fuel += x.fuel_pct();
                nf += 1.0;
            }
            if x.ammo_max.values().sum::<u32>() > 0 {
                ammo += x.ammo_pct(arms);
                na += 1.0;
            }
        }
        let fuel = if nf > 0.0 { fuel / nf } else { 1.0 };
        let ammo = if na > 0.0 { ammo / na } else { 1.0 };
        if nf + na > 0.0 && (fuel < st.p.culmination || ammo < st.p.culmination) {
            st.logistics.culminated.insert(side);
            let text = format!("{side} has culminated: its units average {:.0}% fuel and {:.0}% ammunition", fuel * 100.0, ammo * 100.0);
            s.ctx.log.push(s.ctx.now, "logistics.culmination").side(side).text(text).data(serde_json::json!({ "fuelPct": fuel * 100.0, "ammoPct": ammo * 100.0 }));
        }
    }
}

/// Scripted strikes on the supply line: `cutSupply` / `strikeNode` with a target force id.
pub fn script_event(s: &mut Sim, _st: &mut FiresWorld, ev: &bgws_scenario::v4::ScriptedEvent) {
    if !matches!(ev.kind.as_str(), "cutSupply" | "strikeNode" | "logistics.cut") {
        return;
    }
    let Some(target) = ev.target.as_ref().and_then(|t| s.ctx.by_name.get(t).copied()) else { return };
    let e = s.ctx.entity(target);
    let share = ev.quality.unwrap_or(1.0).clamp(0.0, 1.0);
    let mut h = s.world.get_mut::<Health>(e).expect("h");
    let lose = ((h.start as f64 * share).ceil() as u32).min(h.fit);
    h.fit -= lose;
    if h.fit == 0 {
        h.destroyed = true;
    }
}

/// `logistics.*` actions, for a node's cycle or a convoy's run.
pub fn apply(s: &mut Sim, st: &mut FiresWorld, e: Entity, id: EntityId, action: &str) -> bool {
    let now = s.ctx.now;
    let side = s.world.get::<Ident>(e).expect("i").side;
    let cycle = st.logistics.deciding.get(&side).map(|x| x.1 == id).unwrap_or(false);
    let job = st.logistics.jobs.iter().find(|(_, j)| j.truck == id).map(|(k, _)| *k);
    match action {
        "logistics.convoy" | "logistics.ugv" | "logistics.drone" => {
            if cycle {
                st.logistics.deciding.remove(&side);
                let means = match action {
                    "logistics.ugv" => Means::Ugv,
                    "logistics.drone" => Means::Drone,
                    _ => Means::Convoy,
                };
                execute(s, st, side, means);
            }
        }
        "logistics.waitForDark" => {
            let until = now + SimDuration::from_secs(secs_to_dark(s));
            if cycle {
                st.logistics.deciding.remove(&side);
                st.logistics.dark.insert(side, until);
            } else if let Some(j) = job {
                st.logistics.jobs.get_mut(&j).expect("job").wait_until = Some(until);
                if let Some(mut a) = s.world.get_mut::<Act>(e) {
                    a.activity = Activity::Hold;
                }
            }
        }
        "logistics.pushOn" => {}
        "logistics.alternateRoute" => {
            if let Some(j) = job {
                let jb = st.logistics.jobs.get_mut(&j).expect("job");
                jb.detour = true;
                jb.issued = SimTime(i64::MIN / 2);
                if let Some(mut a) = s.world.get_mut::<Act>(e) {
                    a.activity = Activity::Hold;
                }
            }
        }
        "logistics.scatter" | "logistics.takeCover" => {
            if let Some(j) = job {
                st.logistics.jobs.get_mut(&j).expect("job").wait_until = Some(now + SimDuration::from_mins(10.0));
            }
            let next = if action == "logistics.scatter" { Activity::Withdraw { route: vec![pos_of(s, id).add(Enu::new(150.0, -150.0, 0.0))], then: Some(Box::new(Activity::Hold)) } } else { Activity::Quiet };
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = next;
            }
        }
        "logistics.relocateNode" | "logistics.disperseStocks" => {
            if let Some(n) = st.logistics.nodes.get_mut(&id) {
                n.dispersed = true;
            }
        }
        "logistics.repairAndStay" => {
            if let Some(n) = st.logistics.nodes.get(&id).cloned() {
                let k = s.ctx.figures.get("log.repair_per_day");
                if let Some(site) = st.logistics.nets.get_mut(&n.side).and_then(|net| net.sites.get_mut(&n.site)) {
                    site.damage *= (-k * st.p.repair_days).exp();
                }
            }
        }
        _ => return false,
    }
    true
}
