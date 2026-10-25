//! Ships at sea (§8.1 Sea, §8.6): vessels on their tasks and lanes with the
//! hull's sea-state limit and cubic fuel law, submarines' depth and battery,
//! ports and VLS reload, tenders and UNREP, sea mines and sweeping, and the
//! withdraw-to-rearm rule [R6.2].

use super::{label, put, take, Mines, NavalParams, NavalState};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::{Sim, Trigger};
use bevy_ecs::prelude::*;
use bgws_behaviour::RulesChoice;
use bgws_catalogue::{Domain, MunitionKind, Platform};
use bgws_core::{Enu, EntityId, Side, SimDuration, SimTime};
use bgws_domains::naval::{should_withdraw_to_rearm, unrep_hours, reload_hours, DamageState, HullClass, HullParams, InfluenceSignature, MineKind, Minefield, ShipDamage, Submarine};
use bgws_scenario::scope::DomainMode;
use bgws_scenario::v4::{EntityTask, ObstacleKind};
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};

/// What sort of vessel it is, for doctrine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum VesselKind {
    Warship,
    Merchant,
    Usv,
    SmallCraft,
    Submarine,
    /// Tenders, oilers and MCM vessels.
    Auxiliary,
}

/// A vessel's task at sea.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum NavTask {
    Idle,
    Route { waypoints: Vec<Enu>, then: Option<Box<NavTask>> },
    Station { at: Enu, radius: f64 },
    /// Run in on a target (USVs, a submarine closing to torpedo range).
    Strike { target: Option<EntityId>, aim: Enu },
    /// A merchant's lane, round and round: the next waypoint.
    Lane { next: usize },
    ToPort { port: EntityId },
    InPort { port: EntityId },
    /// Alongside a tender or oiler for reload or UNREP.
    Alongside { with: EntityId, started: bool },
}

impl NavTask {
    pub fn going_home(&self) -> bool {
        matches!(self, NavTask::ToPort { .. } | NavTask::InPort { .. } | NavTask::Alongside { .. })
    }
}

/// A ship, boat or submarine (§8.1 Sea).
#[derive(Component, Clone, Debug)]
pub struct Vessel {
    pub kind: VesselKind,
    pub hull: HullParams,
    /// Cruise and maximum speed, m/s (catalogue, capped by the hull class).
    pub cruise: f64,
    pub max: f64,
    pub task: NavTask,
    pub fuel_t: f64,
    pub speed: f64,
    pub sprint_until: SimTime,
    pub silent: bool,
    pub silent_until: SimTime,
    pub deep: bool,
    /// Depth below the surface, m (0 for surface ships).
    pub depth: f64,
    pub sub: Option<Submarine>,
    pub home: Option<EntityId>,
    /// The task to resume after a port call.
    pub resume: Option<NavTask>,
    pub lane: Vec<Enu>,
    pub evading_until: SimTime,
    pub mcm: bool,
    pub tender: bool,
    pub oiler: bool,
    /// Radar antenna height, m (0: no radar).
    pub radar_h: f64,
    /// Height of what a radar sees of it, m.
    pub mast_h: f64,
    pub sig: InfluenceSignature,
    /// Fields it is inside (by index), for transit accounting.
    pub in_fields: BTreeSet<usize>,
}

/// A ship's fighting state: damage, magazines and its combat system's queue.
#[derive(Component, Clone, Debug)]
pub struct ShipCombat {
    pub damage: ShipDamage,
    pub state: DamageState,
    /// Rounds by munition id.
    pub magazine: BTreeMap<String, u32>,
    pub full: BTreeMap<String, u32>,
    /// Munitions that are surface-to-air missiles (the cells that matter).
    pub sam_ids: Vec<String>,
    pub conserve: bool,
    pub decoy_until: SimTime,
    pub guns_until: SimTime,
    pub mag_low: bool,
    /// Detected threats not yet engaged (inbound ids).
    pub pending: Vec<u64>,
    pub pending_since: Option<SimTime>,
    /// The layer last chosen, and when.
    pub layer: Option<(super::defence::LayerChoice, SimTime)>,
    pub asked: bool,
    pub staying: f64,
    pub hits: u32,
    /// Its own radar and sonar contacts: entity → last seen.
    pub contacts: BTreeMap<EntityId, SimTime>,
}

impl ShipCombat {
    /// Surface-to-air rounds left and loaded at the start.
    pub fn sams(&self) -> (u32, u32) {
        let left = self.sam_ids.iter().map(|m| self.magazine.get(m).copied().unwrap_or(0)).sum();
        let full = self.sam_ids.iter().map(|m| self.full.get(m).copied().unwrap_or(0)).sum();
        (left, full)
    }
}

fn hull_class(p: &Platform) -> HullClass {
    let id = p.id.to_lowercase();
    let class = p.class.to_lowercase();
    if p.domain == Domain::Subsurface {
        HullClass::Submarine
    } else if id.contains("usv") {
        HullClass::Usv
    } else if id.contains("destroyer") || id.contains("cruiser") {
        HullClass::Destroyer
    } else if id.contains("corvette") {
        HullClass::Corvette
    } else if class.contains("smallcraft") || id.contains("fac") {
        HullClass::SmallCraft
    } else if class.contains("warship") {
        HullClass::Frigate
    } else {
        HullClass::Merchant
    }
}

fn vessel_kind(p: &Platform, class: HullClass) -> VesselKind {
    let role = p.role.to_lowercase();
    let id = p.id.to_lowercase();
    if role.contains("replenish") || role.contains("tender") || id.contains("oiler") || id.contains("mcm") || role.contains("mine countermeasure") {
        return VesselKind::Auxiliary;
    }
    match class {
        HullClass::Submarine => VesselKind::Submarine,
        HullClass::Usv => VesselKind::Usv,
        HullClass::SmallCraft => VesselKind::SmallCraft,
        HullClass::Merchant => VesselKind::Merchant,
        _ => VesselKind::Warship,
    }
}

/// Broadband radiated level at a speed, from the catalogue's water levels.
pub(crate) fn radiated_db(p: &Platform, speed_ms: f64) -> f64 {
    let lv = &p.signatures.acoustic.water_db;
    if lv.is_empty() {
        return 140.0;
    }
    let kmh = speed_ms * 3.6;
    if kmh <= lv[0].kmh {
        return lv[0].db;
    }
    for w in lv.windows(2) {
        if kmh <= w[1].kmh {
            let f = (kmh - w[0].kmh) / (w[1].kmh - w[0].kmh).max(1e-9);
            return w[0].db + f * (w[1].db - w[0].db);
        }
    }
    lv[lv.len() - 1].db
}

pub(crate) fn is_port(p: &Platform) -> bool {
    p.domain == Domain::Node && p.id.to_lowercase().contains("port")
}

/// The domain state from the scenario (sea state, sound speed, water, mines).
pub(crate) fn build_state(s: &mut Sim, mode: DomainMode) -> NavalState {
    let f = s.ctx.figures.clone();
    let p = NavalParams::from_figures(&f);
    let sc = &s.ctx.scenario;
    let sea_state_fixed = sc.environment.sea_state;
    let sea = if sea_state_fixed.is_none() {
        Some(bgws_env::SeaStateField::from_weather(&s.ctx.weather, p.g("fetch_m"), 0.5, &bgws_env::sea::WaveParams::from_figures(&f)))
    } else {
        None
    };
    let lat = sc.theatre.origin.lat;
    let region = if lat.abs() < 23.5 {
        bgws_env::OceanRegion::Tropical
    } else if lat.abs() < 60.0 {
        bgws_env::OceanRegion::MidLatitude
    } else {
        bgws_env::OceanRegion::Polar
    };
    let water_depth = p.g("water_depth_m");
    let profile = bgws_env::SoundSpeedProfile::climatology(region, bgws_env::Season::from_month(s.ctx.start.month, lat < 0.0), water_depth);
    let waters: Vec<Vec<(f64, f64)>> = sc
        .world
        .sea
        .as_ref()
        .map(|a| a.waters.iter().map(|poly| poly.iter().map(|g| { let v = s.ctx.frame.geodetic_to_enu(*g); (v.e, v.n) }).collect()).collect())
        .unwrap_or_default();
    let mut mines = Vec::new();
    for (i, o) in sc.world.obstacles.iter().enumerate() {
        if o.kind != ObstacleKind::SeaMines {
            continue;
        }
        let id = o.id.clone().unwrap_or_else(|| format!("seamines-{i}"));
        let kind = if id.contains("contact") {
            MineKind::Contact
        } else if id.contains("acoustic") {
            MineKind::Acoustic
        } else if id.contains("pressure") {
            MineKind::Pressure
        } else {
            MineKind::Magnetic
        };
        mines.push(Mines {
            id,
            owner: o.owner,
            field: Minefield { kind, density: o.density * p.g("mine_density_scale"), cleared: 0.0 },
            centre: s.ctx.frame.geodetic_to_enu(o.at),
            half_w: o.width_m / 2.0,
            half_d: o.depth_m / 2.0,
            bearing: o.bearing_deg.to_radians(),
            area_km2: o.width_m * o.depth_m / 1e6,
            known_to: [o.owner].into_iter().collect(),
        });
    }
    NavalState {
        p,
        mode,
        sea_state_fixed,
        sea,
        profile,
        water_depth,
        waters,
        mines,
        inbound: BTreeMap::new(),
        torpedoes: BTreeMap::new(),
        next_id: 0,
        raids: Vec::new(),
        stats: Default::default(),
        asked: BTreeMap::new(),
        recent_hits: Vec::new(),
        deferred: Vec::new(),
        seen: BTreeSet::new(),
        fresh_sonar: Vec::new(),
        ports: BTreeMap::new(),
    }
}

/// Give every ship, boat and submarine its `Vessel` (and a warship or
/// merchant its `ShipCombat`), and every port its stores.
pub(crate) fn setup_vessels(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let f = s.ctx.figures.clone();
    let order = s.ctx.order.clone();
    // Ports first: ships name them as home.
    let mut ports: Vec<(EntityId, Side, Enu)> = Vec::new();
    for &e in &order {
        let (i, k) = (s.world.get::<Ident>(e).expect("ident"), s.world.get::<Kin>(e).expect("kin"));
        if is_port(&i.platform) {
            let mut stores: BTreeMap<String, f64> = i.platform.stores.ammo.clone();
            stores.insert("fuel".into(), 1.0e9);
            st.ports.insert(i.id, stores);
            ports.push((i.id, i.side, k.pos));
        }
    }
    for (idx, &e) in order.iter().enumerate() {
        let (id, side, platform, pos) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side, i.platform.clone(), s.world.get::<Kin>(e).expect("kin").pos)
        };
        if !matches!(platform.domain, Domain::Surface | Domain::Subsurface) {
            continue;
        }
        // Entities spawned during play (munitions, flights) have no scenario force.
        let Some(force) = s.ctx.scenario.forces.get(idx) else { continue };
        let class = hull_class(&platform);
        let hull = HullParams::from_figures(class, &f);
        let kind = vessel_kind(&platform, class);
        let cruise = (platform.mover.cruise_kmh / 3.6).min(hull.max_speed).max(0.5);
        let max = (platform.mover.max_kmh / 3.6).min(hull.max_speed).max(cruise);
        let enu = |g: bgws_core::Geodetic| s.ctx.frame.geodetic_to_enu(g);
        let task = match &force.task {
            Some(EntityTask::Station { at, radius_m }) => NavTask::Station { at: enu(*at), radius: *radius_m },
            Some(EntityTask::Orbit { at, radius_m, .. }) | Some(EntityTask::LoiterSearch { at, radius_m, .. }) => NavTask::Station { at: enu(*at), radius: *radius_m },
            Some(EntityTask::Route { waypoints, .. }) => NavTask::Route { waypoints: waypoints.iter().map(|g| enu(*g)).collect(), then: None },
            Some(EntityTask::Strike { target_id, aim_at, .. }) => NavTask::Strike { target: target_id.as_ref().and_then(|t| s.ctx.by_name.get(t).copied()), aim: enu(*aim_at) },
            Some(EntityTask::ReturnToBase { base_id }) | Some(EntityTask::Reload { port_id: base_id, .. }) => match s.ctx.by_name.get(base_id) {
                Some(p) => NavTask::ToPort { port: *p },
                None => NavTask::Idle,
            },
            _ if !force.lane.is_empty() => NavTask::Lane { next: 1 },
            _ => NavTask::Idle,
        };
        let lane: Vec<Enu> = force.lane.iter().map(|g| enu(*g)).collect();
        let home = force
            .parent
            .as_ref()
            .and_then(|p| s.ctx.by_name.get(p).copied())
            .filter(|p| ports.iter().any(|x| x.0 == *p))
            .or_else(|| ports.iter().filter(|x| x.1 == side).min_by(|a, b| a.2.sub(pos).horizontal().total_cmp(&b.2.sub(pos).horizontal()).then(a.0.cmp(&b.0))).map(|x| x.0));
        let mass = platform.body.mass_t.max(1.0);
        let warship = kind == VesselKind::Warship;
        let sig = InfluenceSignature {
            magnetic: (mass / 50_000.0).cbrt().min(1.5) * if warship { st.p.g("degauss") } else { 1.0 },
            acoustic: 10f64.powf((radiated_db(&platform, cruise) - 170.0) / 20.0).clamp(0.01, 2.0),
            pressure: (mass / 50_000.0).sqrt().min(1.5),
            beam: platform.body.width_m.max(3.0),
        };
        let radar_h = platform.sensors.iter().filter(|x| x.kind == bgws_catalogue::SensorKind::Radar).map(|x| x.height_m).fold(0.0, f64::max);
        let sub = (kind == VesselKind::Submarine).then(|| Submarine { nuclear: platform.id.contains("ssn") || platform.mover.endurance_h > 1000.0, battery: 1.0, snorting: false, depth: st.p.g("patrol_depth_m") });
        let role = platform.role.to_lowercase();
        let v = Vessel {
            kind,
            cruise,
            max,
            task: if kind == VesselKind::Warship && matches!(task, NavTask::Idle) { NavTask::Station { at: pos, radius: 2_000.0 } } else { task },
            fuel_t: hull.fuel_capacity,
            hull,
            speed: 0.0,
            sprint_until: SimTime::ZERO,
            silent: false,
            silent_until: SimTime::ZERO,
            deep: false,
            depth: if kind == VesselKind::Submarine { st.p.g("patrol_depth_m").min(st.water_depth - st.p.g("keel_m")).max(10.0) } else { 0.0 },
            sub,
            home,
            resume: None,
            lane,
            evading_until: SimTime::ZERO,
            mcm: role.contains("mine countermeasure") || platform.id.contains("mcm"),
            tender: role.contains("tender") || role.contains("replenish"),
            oiler: role.contains("replenish") || platform.id.contains("oiler"),
            radar_h,
            mast_h: if platform.body.height_m > 0.0 { platform.body.height_m } else if matches!(kind, VesselKind::Usv | VesselKind::SmallCraft) { 1.5 } else { st.p.g("mast_m") },
            sig,
            in_fields: BTreeSet::new(),
        };
        let combat = (!matches!(kind, VesselKind::Usv)).then(|| {
            let mut magazine = BTreeMap::new();
            let mut sams = Vec::new();
            for m in &platform.mounts {
                for mu in &m.munitions {
                    let n = platform.stores.ammo.get(mu).map(|x| *x as u32).unwrap_or(m.magazine);
                    magazine.insert(mu.clone(), n);
                    if s.ctx.catalogue.munition(mu).map(|x| x.kind == MunitionKind::Sam).unwrap_or(false) && !sams.contains(mu) {
                        sams.push(mu.clone());
                    }
                }
            }
            let dc = if class == HullClass::Destroyer { st.p.g("dc_destroyer") } else { st.p.g("dc_small") } as u32;
            let damage = ShipDamage::combatant(dc);
            let state = damage.state(&st.p.damage);
            ShipCombat {
                damage,
                state,
                full: magazine.clone(),
                magazine,
                sam_ids: sams,
                conserve: false,
                decoy_until: SimTime::ZERO,
                guns_until: SimTime::ZERO,
                mag_low: false,
                pending: Vec::new(),
                pending_since: None,
                layer: None,
                asked: false,
                staying: platform.body.staying_power.max(0.3),
                hits: 0,
                contacts: BTreeMap::new(),
            }
        });
        let mut em = s.world.entity_mut(e);
        em.insert(v);
        if let Some(c) = combat {
            em.insert(c);
        }
        if let Some(mut k) = s.world.get_mut::<Kin>(e) {
            if platform.domain == Domain::Subsurface {
                k.pos.u = -st.p.g("patrol_depth_m");
            }
        }
        let _ = id;
    }
    put(s, st);
}

pub(crate) fn alive(s: &Sim, e: Entity) -> bool {
    crate::systems::alive(s, e)
}

fn port_pos(s: &Sim, port: EntityId) -> Option<Enu> {
    let e = s.ctx.entity(port);
    if !alive(s, e) {
        return None;
    }
    s.world.get::<Kin>(e).map(|k| k.pos)
}

/// One step of every vessel on its task (base rate).
pub(crate) fn movement(s: &mut Sim, dt: f64) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    let order = s.ctx.order.clone();
    for &e in &order {
        let Some(v) = s.world.get::<Vessel>(e).cloned() else { continue };
        if !alive(s, e) {
            continue;
        }
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side)
        };
        let k = *s.world.get::<Kin>(e).expect("kin");
        let mobile = s.world.get::<Health>(e).map(|h| h.can_move()).unwrap_or(false);
        let ss = st.sea_state(k.pos.e, k.pos.n, now);
        let sprint = v.sprint_until > now;
        let mut speed = if sprint { v.max } else { v.cruise };
        if v.silent && v.silent_until > now {
            speed *= st.p.g("quiet_speed");
        }
        let mut task = v.task.clone();
        let mut goal: Option<Enu> = None;
        let arrive = st.p.g("arrive_m");
        let mut log: Option<(String, &'static str)> = None;
        match &mut task {
            NavTask::Idle | NavTask::InPort { .. } => {}
            NavTask::Route { waypoints, then } => {
                if let Some(w) = waypoints.first().copied() {
                    if w.sub(k.pos).horizontal() <= arrive.max(speed * dt * 2.0) {
                        waypoints.remove(0);
                    }
                    goal = Some(w);
                }
                if waypoints.is_empty() {
                    task = then.take().map(|b| *b).unwrap_or(if v.kind == VesselKind::Warship { NavTask::Station { at: k.pos, radius: 2_000.0 } } else { NavTask::Idle });
                }
            }
            NavTask::Station { at, radius } => {
                let d = at.sub(k.pos).horizontal();
                if d > *radius {
                    goal = Some(*at);
                } else {
                    let b = k.heading + 0.15;
                    goal = Some(at.add(Enu::new(b.sin(), b.cos(), 0.0).scale(*radius * 0.8)));
                    if !sprint {
                        speed *= st.p.g("station_speed");
                    }
                }
            }
            NavTask::Strike { target, aim } => {
                let tpos = target.and_then(|t| {
                    let te = s.ctx.entity(t);
                    if alive(s, te) {
                        s.world.get::<Kin>(te).map(|k| k.pos)
                    } else {
                        None
                    }
                });
                goal = Some(tpos.unwrap_or(*aim));
                if v.kind == VesselKind::Usv {
                    speed = v.max;
                }
            }
            NavTask::Lane { next } => {
                if v.lane.len() >= 2 {
                    if recent_ship_hit(&st, k.pos, now) {
                        goal = None;
                    } else {
                        let w = v.lane[(*next).min(v.lane.len() - 1)];
                        goal = Some(w);
                        if w.sub(k.pos).horizontal() <= arrive.max(speed * dt * 2.0) {
                            *next += 1;
                            if *next >= v.lane.len() {
                                *st.stats.transits.entry(id).or_insert(0) += 1;
                                log = Some((format!("{} completes a transit of its lane ({} so far)", label(s, id), st.stats.transits[&id]), "naval.transit"));
                                *next = 1;
                                // Round again from the start of the lane.
                                if let Some(mut km) = s.world.get_mut::<Kin>(e) {
                                    km.pos = v.lane[0];
                                }
                                goal = None;
                            }
                        }
                    }
                }
            }
            NavTask::ToPort { port } => match port_pos(s, *port) {
                Some(pp) => {
                    if pp.sub(k.pos).horizontal() <= st.p.g("in_port_m") {
                        let port = *port;
                        task = NavTask::InPort { port };
                        start_reload(s, &mut st, e, id, port, true);
                    } else {
                        goal = Some(pp);
                    }
                }
                None => task = v.resume.clone().unwrap_or(NavTask::Idle),
            },
            NavTask::Alongside { with, started } => {
                let we = s.ctx.entity(*with);
                match s.world.get::<Kin>(we).map(|k| k.pos).filter(|_| alive(s, we)) {
                    Some(wp) => {
                        if wp.sub(k.pos).horizontal() <= st.p.g("unrep_m") {
                            if !*started {
                                *started = true;
                                let w = *with;
                                start_reload(s, &mut st, e, id, w, false);
                            }
                        } else {
                            goal = Some(wp);
                        }
                    }
                    None => task = v.resume.clone().unwrap_or(NavTask::Idle),
                }
            }
        }
        if let Some((text, kind)) = log {
            s.ctx.log.push(now, kind).side(side).entity(id).text(text);
        }
        // Sea state: small craft stay in harbour; everything slows [R6.3].
        speed = speed.min(v.hull.speed_in(ss));
        if v.fuel_t <= 0.0 || !mobile {
            speed = 0.0;
        }
        let mut nv = v.clone();
        nv.task = task;
        let k_now = *s.world.get::<Kin>(e).expect("kin");
        let mut new_pos = k_now.pos;
        let mut heading = k_now.heading;
        let mut moved = 0.0;
        if let Some(g) = goal {
            let d = Enu::new(g.e - k_now.pos.e, g.n - k_now.pos.n, 0.0);
            let dist = d.horizontal();
            let step = (speed * dt).min(dist);
            if step > 1e-6 {
                let head = d.bearing();
                let mut chosen = None;
                for turn in [0.0f64, 30.0, -30.0, 60.0, -60.0, 90.0, -90.0, 135.0, -135.0] {
                    let b = head + turn.to_radians();
                    let p = Enu::new(k_now.pos.e + b.sin() * step, k_now.pos.n + b.cos() * step, k_now.pos.u);
                    if st.is_water(p.e, p.n) {
                        chosen = Some((p, b));
                        break;
                    }
                }
                if let Some((p, b)) = chosen {
                    new_pos = p;
                    heading = b;
                    moved = step;
                }
            }
        }
        nv.speed = moved / dt.max(1e-9);
        nv.fuel_t = (nv.fuel_t - nv.hull.fuel_per_hour(nv.speed) * dt / 3600.0).max(0.0);
        // Submarines: depth off the bottom, battery and snorting.
        if let Some(sub) = nv.sub.as_mut() {
            let floor = (st.water_depth - st.p.g("keel_m")).max(5.0);
            let want = if sub.snorting { st.p.sub.snort_depth } else if nv.deep { floor } else { st.p.g("patrol_depth_m").min(floor) };
            nv.depth = want;
            sub.depth = want;
            sub.step(&st.p.sub, nv.speed / 0.514_444, dt / 3600.0);
            if !sub.snorting && sub.needs_snort(0.2) {
                sub.snorting = true;
            } else if sub.snorting && sub.battery >= 0.99 {
                sub.snorting = false;
            }
            new_pos.u = -nv.depth;
        }
        {
            let mut km = s.world.get_mut::<Kin>(e).expect("kin");
            km.vel = Enu::new((new_pos.e - km.pos.e) / dt.max(1e-9), (new_pos.n - km.pos.n) / dt.max(1e-9), 0.0);
            if moved == 0.0 {
                km.vel = Enu::default();
            }
            km.pos = new_pos;
            km.heading = heading;
        }
        if moved > 0.0 {
            if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
                sig.last_moved = Some(now);
            }
        }
        if nv.silent && nv.silent_until <= now {
            nv.silent = false;
        }
        *s.world.get_mut::<Vessel>(e).expect("vessel") = nv;
    }
    put(s, st);
}

/// A ship hit near here recently: merchants stop sailing.
fn recent_ship_hit(st: &NavalState, pos: Enu, now: SimTime) -> bool {
    let gap = st.p.g("shipping_pause_s");
    let r = st.p.g("shipping_pause_m");
    st.recent_hits.iter().any(|(t, at)| (now - *t).secs() < gap && at.sub(pos).horizontal() <= r)
}

/// Start a reload alongside a port (or a tender): VLS cells by the R6.2
/// rates, fuel in the same call.
fn start_reload(s: &mut Sim, st: &mut NavalState, e: Entity, id: EntityId, at: EntityId, port: bool) {
    let Some(c) = s.world.get::<ShipCombat>(e) else {
        // No magazine: just fuel.
        let h = unrep_hours(s.world.get::<Vessel>(e).map(|v| v.hull.fuel_capacity - v.fuel_t).unwrap_or(0.0), 0.0, &s.ctx.figures);
        s.ctx.queue.schedule(s.ctx.now + SimDuration::from_hours(h), Priority::Process, Scheduled::Timer { system: "naval.reloaded".into(), key: id.0 as u64 });
        return;
    };
    let cells: u32 = c.sam_ids.iter().map(|m| c.full.get(m).copied().unwrap_or(0).saturating_sub(c.magazine.get(m).copied().unwrap_or(0))).sum();
    let fuel = s.world.get::<Vessel>(e).map(|v| v.hull.fuel_capacity - v.fuel_t).unwrap_or(0.0);
    let hours = reload_hours(cells, port, !port, &s.ctx.figures).unwrap_or(0.0).max(unrep_hours(fuel, 0.0, &s.ctx.figures) * if port { 0.0 } else { 1.0 });
    let now = s.ctx.now;
    s.ctx.queue.schedule(now + SimDuration::from_hours(hours.max(0.1)), Priority::Process, Scheduled::Timer { system: "naval.reloaded".into(), key: id.0 as u64 });
    let text = format!("{} alongside {}: reloading {} cells and refuelling, {:.1} h", label(s, id), label(s, at), cells, hours);
    let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
    s.ctx.log.push(now, "naval.inPort").side(side).entity(id).target(at).text(text).data(serde_json::json!({ "cells": cells, "hours": hours, "port": port }));
    let _ = st;
}

/// A reload is done: cells refilled from the port's (or tender's) stores,
/// fuel topped up, and back to the station it left.
pub(crate) fn reloaded(s: &mut Sim, id: EntityId) {
    let Some(mut st) = take(s) else { return };
    let e = s.ctx.entity(id);
    let Some(v) = s.world.get::<Vessel>(e).cloned() else {
        put(s, st);
        return;
    };
    let from = match v.task {
        NavTask::InPort { port } => Some(port),
        NavTask::Alongside { with, .. } => Some(with),
        _ => None,
    };
    let mut loaded = 0u32;
    if let Some(mut c) = s.world.get_mut::<ShipCombat>(e) {
        let full = c.full.clone();
        for (m, n) in full {
            let have = c.magazine.get(&m).copied().unwrap_or(0);
            let want = n.saturating_sub(have);
            let give = match from.and_then(|p| st.ports.get_mut(&p)) {
                Some(stores) => {
                    let avail = stores.get(&m).copied().unwrap_or(0.0).max(0.0) as u32;
                    let g = want.min(avail);
                    if let Some(x) = stores.get_mut(&m) {
                        *x -= g as f64;
                    }
                    g
                }
                // A tender carries what the ship needs [default].
                None => want,
            };
            if c.sam_ids.contains(&m) {
                loaded += give;
            }
            c.magazine.insert(m, have + give);
        }
        c.mag_low = false;
        c.conserve = false;
    }
    let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
    if let Some(mut vm) = s.world.get_mut::<Vessel>(e) {
        vm.fuel_t = vm.hull.fuel_capacity;
        vm.task = vm.resume.take().unwrap_or(NavTask::Idle);
    }
    let text = format!("{} reloaded: {} interceptors back in its cells, fuel full", label(s, id), loaded);
    s.ctx.log.push(s.ctx.now, "naval.reloaded").side(side).entity(id).text(text).data(serde_json::json!({ "interceptors": loaded }));
    put(s, st);
}

/// After an evasion sprint, the boat drifts quiet.
pub(crate) fn drift(s: &mut Sim, id: EntityId) {
    let now = s.ctx.now;
    let silent = s.ctx.figures.get("sim.naval.silent_s");
    if let Some(mut v) = s.world.get_mut::<Vessel>(s.ctx.entity(id)) {
        v.deep = false;
        v.silent = true;
        v.silent_until = now + SimDuration::from_secs(silent);
    }
}

/// Sea mines: actuation per transit by the hull's influence signatures,
/// sweeping and hunting by MCM vessels inside a field (§8.6).
pub(crate) fn mines(s: &mut Sim, dt: f64) {
    let Some(mut st) = take(s) else { return };
    if st.mines.is_empty() {
        put(s, st);
        return;
    }
    let now = s.ctx.now;
    let f = s.ctx.figures.clone();
    for &e in &s.ctx.order.clone() {
        let Some(v) = s.world.get::<Vessel>(e).cloned() else { continue };
        if !alive(s, e) {
            continue;
        }
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side)
        };
        let pos = s.world.get::<Kin>(e).expect("kin").pos;
        let mut inside = BTreeSet::new();
        for (fi, m) in st.mines.iter_mut().enumerate() {
            if !m.contains(pos) {
                continue;
            }
            inside.insert(fi);
            if v.mcm && m.owner != side {
                m.field.clear(true, st.p.g("mcm_assets"), dt / 3600.0, m.area_km2, &f);
            }
            // The owner knows its own lanes; a deep submarine passes under.
            if m.owner == side || v.depth > 30.0 {
                continue;
            }
            let length = v.speed * dt;
            if length <= 0.0 {
                continue;
            }
            let p = m.field.actuation_probability(&v.sig, length, &f);
            let mut rng = s.ctx.rng("naval.mine", id.0 as u64, fi as u64);
            if rng.gen::<f64>() < p {
                m.known_to.insert(side);
                st.stats.mine_strikes += 1;
                let field = m.id.clone();
                let text = format!("{} strikes a mine in {}", label(s, id), field);
                s.ctx.log.push(now, "naval.mine").side(side).entity(id).text(text).data(serde_json::json!({ "field": field, "p": p }));
                let w = st.p.g("mine_warhead");
                super::defence::apply_hit(s, &mut st, id, w, None, "a mine");
                break;
            }
        }
        if let Some(mut vm) = s.world.get_mut::<Vessel>(e) {
            vm.in_fields = inside;
        }
    }
    put(s, st);
}

/// The withdraw-to-rearm rule [R6.2]: below the declared share of its
/// interceptors a warship asks (magazineLow); the rule leaves station for a
/// port or tender when one is reachable, else conserves.
pub(crate) fn magazines(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let Some(c) = s.world.get::<ShipCombat>(e) else { continue };
        let (left, full) = c.sams();
        if c.mag_low || full == 0 || !should_withdraw_to_rearm(left, full, st.p.withdraw_fraction) {
            continue;
        }
        let Some(v) = s.world.get::<Vessel>(e) else { continue };
        if v.task.going_home() {
            continue;
        }
        let id = s.world.get::<Ident>(e).expect("ident").id;
        let side = s.world.get::<Ident>(e).expect("ident").side;
        s.world.get_mut::<ShipCombat>(e).expect("combat").mag_low = true;
        let text = format!("{} is down to {left} of {full} interceptors", label(s, id));
        s.ctx.log.push(s.ctx.now, "naval.magazineLow").side(side).entity(id).text(text).data(serde_json::json!({ "left": left, "full": full, "threshold": st.p.withdraw_fraction }));
        let reload = home_for(s, e).is_some();
        let (opt, action) = if reload { ("reload", "sea.leaveToReload") } else { ("conserve", "sea.conserve") };
        put(s, st);
        ask_or_do(s, id, "magazineLow", None, opt, action);
        st = take(s).expect("state");
    }
    put(s, st);
}

/// Where a ship reloads: its home port if it stands, else the nearest own
/// port, else a tender of its side.
pub(crate) fn home_for(s: &Sim, e: Entity) -> Option<(EntityId, bool)> {
    let st = s.world.get_resource::<NavalState>()?;
    let side = s.world.get::<Ident>(e)?.side;
    let pos = s.world.get::<Kin>(e)?.pos;
    let v = s.world.get::<Vessel>(e)?;
    if let Some(h) = v.home.filter(|h| alive(s, s.ctx.entity(*h))) {
        return Some((h, true));
    }
    let mut best: Option<(f64, EntityId, bool)> = None;
    for &o in &s.ctx.order {
        let Some(i) = s.world.get::<Ident>(o) else { continue };
        if i.side != side || !alive(s, o) || o == e {
            continue;
        }
        let port = st.ports.contains_key(&i.id);
        let tender = s.world.get::<Vessel>(o).map(|x| x.tender).unwrap_or(false);
        if !port && !tender {
            continue;
        }
        let d = s.world.get::<Kin>(o).map(|k| k.pos.sub(pos).horizontal()).unwrap_or(f64::INFINITY);
        if best.map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, i.id, port));
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Ask a world question through the unit's template with the domain's rules
/// choice; if no template answers it (or the unit is busy deciding), do the
/// rule's action now. Returns whether a question was raised.
pub(crate) fn ask_or_do(s: &mut Sim, id: EntityId, point: &'static str, about: Option<EntityId>, option: &str, action: &str) -> bool {
    let e = s.ctx.entity(id);
    if !alive(s, e) {
        return false;
    }
    let deciding = s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(false);
    if !deciding && crate::decide::ask_with_rule(s, &Trigger { entity: id, point, about }, RulesChoice::Option(option.to_string())) {
        return true;
    }
    if !deciding {
        let params = BTreeMap::new();
        crate::world::apply_action(s, e, id, action, &params, about);
        let text = format!("{}: {} by standing orders", label(s, id), action);
        let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
        s.ctx.log.push(s.ctx.now, "naval.drill").side(side).entity(id).text(text).data(serde_json::json!({ "point": point, "action": action }));
    }
    false
}

/// Fuel: a ship low on fuel goes to an oiler or its port (minute rate).
pub(crate) fn ports_and_fuel(s: &mut Sim, _dt: f64) {
    let Some(st) = take(s) else { return };
    let low = st.p.g("fuel_low");
    put(s, st);
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let Some(v) = s.world.get::<Vessel>(e).cloned() else { continue };
        if v.task.going_home() || !matches!(v.kind, VesselKind::Warship | VesselKind::Submarine) || v.hull.fuel_capacity <= 0.0 || v.fuel_t / v.hull.fuel_capacity >= low {
            continue;
        }
        let Some((to, port)) = home_for(s, e) else { continue };
        let id = s.world.get::<Ident>(e).expect("ident").id;
        let side = s.world.get::<Ident>(e).expect("ident").side;
        {
            let mut vm = s.world.get_mut::<Vessel>(e).expect("vessel");
            vm.resume = Some(v.task.clone());
            vm.task = if port { NavTask::ToPort { port: to } } else { NavTask::Alongside { with: to, started: false } };
        }
        let text = format!("{} is low on fuel ({:.0}%): to {} to refuel", label(s, id), 100.0 * v.fuel_t / v.hull.fuel_capacity, label(s, to));
        s.ctx.log.push(s.ctx.now, "naval.fuelLow").side(side).entity(id).target(to).text(text);
    }
}
