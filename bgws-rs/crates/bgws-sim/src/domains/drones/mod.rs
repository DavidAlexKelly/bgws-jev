//! Domain systems: comms nets, HQs and C2, EW, PNT, drones and counter-drone
//! (RUST_ENGINE.md §8.5, §8.7, §8.12, §9.2, §10.3–10.4).
//!
//! - [`comms`]: the side's nets as a `bgws_ops::comms::CommsNetwork` built
//!   from the units' radios, link SINR from `bgws_prop` terrain path loss
//!   and jamming, PACE, and every message that matters — contact reports,
//!   situation reports, track forwarding between HQs, orders and target
//!   hand-offs — delivered late, or lost, by the network. Every
//!   transmission is an RF emission for ESM.
//! - [`c2`]: HQ entities with a COP fused by covariance intersection from the
//!   reports that reach them, staff queues, planning time (1/3–2/3 rule),
//!   CCIR triggers, orders that travel the nets, out-of-comms units acting on
//!   intent, succession when a post is lost, seats.
//! - [`ew`]: jammers as emitters with J/S at each receiver, ESM listeners
//!   taking bearings on every emission and fixing emitters by triangulation,
//!   EMCON and exposure.
//! - [`pnt`]: GNSS trackers per unit under jamming, INS drift, self-position
//!   error in reports and precision munitions' CEP.
//! - [`drones`]: drone teams, flights at 10 Hz, camera search, the link
//!   fight at both ends, link-loss autonomy, the FPV strike chain with its
//!   effects, stores, operators, counter-drone, and drone-observed stress.
//!
//! All state lives in one [`C4`] resource. A system takes it out of the world
//! for its run and puts it back before it calls anything that may call back
//! into this module (plans, questions, the HQ's situation).

#[allow(clippy::module_inception)]
pub mod drones;

pub mod c2;
pub mod comms;
pub mod ew;
pub mod params;
pub mod pnt;

use crate::components::*;
use crate::queue::Scheduled;
use crate::sim::{Sim, Trigger};
use bevy_ecs::prelude::{Entity, Resource};
use bgws_behaviour::{Capability, Emcon as GateEmcon, FactValue};
use bgws_catalogue::{Domain, Platform, SensorKind as CatSensor};
use bgws_core::{Enu, EntityId, Figure, Side, SimTime};
use bgws_ops::comms::BearerKind;
use bgws_ops::counter_drone::CuasSensor;
use bgws_ops::pnt::InsGrade;
use bgws_prop::{terrain_path_loss, RadioParams, Terminal};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use bgws_scenario::v4::Emcon;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub use c2::{hq_cop_tracks, hq_post, hq_unit_report, route_plan};
pub use comms::cut_off;
pub use drones::{drone_observed, link_at};
pub use params::{figures, C4Params};
pub use pnt::{nav_error_m, precision_accuracy, self_position};

/// One radio a unit carries, as the nets use it.
#[derive(Clone, Debug)]
pub struct RadioFit {
    pub kind: BearerKind,
    pub freq_hz: f64,
    pub power_w: f64,
    pub gain_dbi: f64,
    pub agl: f64,
    pub rate_bps: f64,
}

/// What an entity is, for these domains (from its catalogue record).
#[derive(Clone, Debug, Default)]
pub struct Role {
    pub land: bool,
    pub radios: Vec<RadioFit>,
    /// A jammer emitter.
    pub jammer: Option<bgws_catalogue::Emitter>,
    /// An ESM receiver: (range, parameters, mast height).
    pub esm: Option<(f64, bgws_catalogue::EsmParams, f64)>,
    /// Drones it carries: (archetype, count).
    pub team: Option<(String, u32)>,
    /// Radars it radiates with while free to emit: (freq, ERP W, duty).
    pub radars: Vec<(f64, f64, f64)>,
    /// Its drone ground-station link: (freq, ERP W, bandwidth).
    pub drone_link: Option<(f64, f64, f64)>,
    pub cuas_sensors: Vec<CuasSensor>,
    /// Mount indices: an air-defence gun, an interceptor-drone launcher, a
    /// short- or medium-range SAM (short preferred).
    pub ad_gun: Option<usize>,
    pub interceptor: Option<usize>,
    pub sam: Option<usize>,
    /// Rifles and MGs it can turn on a drone.
    pub small_arms: bool,
    pub ins: Option<InsGrade>,
    pub artillery: bool,
    pub armoured: bool,
    /// An anti-drone cage (a passive measure).
    pub cage: bool,
}

impl Role {
    pub fn has(&self, k: BearerKind) -> bool {
        self.radios.iter().any(|r| r.kind == k)
    }
    pub fn radio(&self, k: BearerKind) -> Option<&RadioFit> {
        self.radios.iter().find(|r| r.kind == k)
    }
}

/// Why a question was raised, for the action that answers it.
#[derive(Clone, Debug, Default)]
pub struct QCtx {
    pub flight: Option<u64>,
    pub target: Option<EntityId>,
    pub at: Option<Enu>,
    pub emitter: Option<EntityId>,
    pub sub: Option<EntityId>,
}

/// A path-loss query between two ends (entity ids, or drone flights).
#[derive(Clone, Copy, Debug)]
pub struct LossQuery {
    pub a: u64,
    pub b: u64,
    pub f_hz: f64,
    pub pa: Enu,
    pub agl_a: f64,
    pub pb: Enu,
    pub agl_b: f64,
}

/// One cached loss: both ends (position, agl), the loss, when last used.
type CachedLoss = (Enu, f64, Enu, f64, f64, SimTime);

/// Terrain path losses, recomputed only when an end has moved.
#[derive(Clone, Debug, Default)]
pub struct LossCache {
    map: BTreeMap<(u64, u64, u64), CachedLoss>,
}

/// Cache id of a drone flight (entity ids are below 2^32).
pub fn flight_key(flight: u64) -> u64 {
    (1u64 << 40) + flight
}

impl LossCache {
    fn canonical(q: &LossQuery) -> ((u64, u64, u64), Enu, f64, Enu, f64) {
        let fk = (q.f_hz / 1e3).round().max(0.0) as u64;
        if q.a <= q.b {
            ((q.a, q.b, fk), q.pa, q.agl_a, q.pb, q.agl_b)
        } else {
            ((q.b, q.a, fk), q.pb, q.agl_b, q.pa, q.agl_a)
        }
    }

    /// Losses for these queries, in order; stale or missing ones computed in parallel.
    pub fn resolve(&mut self, terrain: &bgws_env::Terrain, rp: &RadioParams, tol: f64, now: SimTime, qs: &[LossQuery]) -> Vec<f64> {
        let moved = |a: Enu, ha: f64, b: Enu, hb: f64| a.sub(b).horizontal() > tol || (ha - hb).abs() > tol;
        let mut need: Vec<usize> = Vec::new();
        let mut out = vec![0.0; qs.len()];
        for (i, q) in qs.iter().enumerate() {
            let (k, pa, ha, pb, hb) = Self::canonical(q);
            match self.map.get_mut(&k) {
                Some(e) if !moved(e.0, e.1, pa, ha) && !moved(e.2, e.3, pb, hb) => {
                    out[i] = e.4;
                    e.5 = now;
                }
                _ => need.push(i),
            }
        }
        if need.is_empty() {
            return out;
        }
        let computed: Vec<f64> = need
            .par_iter()
            .map(|&i| {
                let q = &qs[i];
                let pl = terrain_path_loss(terrain, Terminal { e: q.pa.e, n: q.pa.n, agl: q.agl_a.max(0.5) }, Terminal { e: q.pb.e, n: q.pb.n, agl: q.agl_b.max(0.5) }, q.f_hz.max(1e5), rp);
                pl.total_db
            })
            .collect();
        for (&i, l) in need.iter().zip(computed) {
            let (k, pa, ha, pb, hb) = Self::canonical(&qs[i]);
            self.map.insert(k, (pa, ha, pb, hb, l, now));
            out[i] = l;
        }
        out
    }

    /// Drop entries not used since `before`.
    pub fn prune(&mut self, before: SimTime) {
        self.map.retain(|_, v| v.5 >= before);
    }
}

/// All the state of these domains.
#[derive(Resource)]
pub struct C4 {
    pub p: C4Params,
    pub radio: RadioParams,
    pub roles: BTreeMap<EntityId, Role>,
    pub emcon: BTreeMap<EntityId, Emcon>,
    /// Seconds on the air since it last moved.
    pub on_air: BTreeMap<EntityId, f64>,
    pub qctx: BTreeMap<EntityId, QCtx>,
    pub losses: LossCache,
    pub comms: comms::CommsState,
    pub c2: c2::C2State,
    pub ew: ew::EwState,
    pub pnt: pnt::PntState,
    pub drones: drones::DroneState,
    /// Where each entity was at the last base tick (to tell who moved).
    pub last_pos: BTreeMap<EntityId, Enu>,
}

impl C4 {
    pub fn emcon_of(&self, id: EntityId) -> Emcon {
        self.emcon.get(&id).copied().unwrap_or(Emcon::Free)
    }
    /// May it transmit on its radios?
    pub fn may_transmit(&self, id: EntityId) -> bool {
        self.emcon_of(id) != Emcon::Silent
    }
    /// May it jam or radiate a radar?
    pub fn may_emit(&self, id: EntityId) -> bool {
        self.emcon_of(id) == Emcon::Free
    }
    pub fn role(&self, id: EntityId) -> Option<&Role> {
        self.roles.get(&id)
    }
    pub fn add_on_air(&mut self, id: EntityId, s: f64) {
        *self.on_air.entry(id).or_insert(0.0) += s;
    }
}

pub(crate) fn take(s: &mut Sim) -> Option<C4> {
    s.world.remove_resource::<C4>()
}

pub(crate) fn put(s: &mut Sim, c: C4) {
    s.world.insert_resource(c);
}

pub(crate) fn mode(s: &Sim, d: ScopeDomain) -> DomainMode {
    s.ctx.scenario.scope.mode(d)
}

/// The entity's ECS handle, if the id is in range.
pub(crate) fn ent(s: &Sim, id: EntityId) -> Option<Entity> {
    s.ctx.order.get(id.index()).copied()
}

pub(crate) fn alive_id(s: &Sim, id: EntityId) -> bool {
    ent(s, id).map(|e| crate::systems::alive(s, e)).unwrap_or(false)
}

pub(crate) fn pos_of(s: &Sim, id: EntityId) -> Option<Enu> {
    ent(s, id).and_then(|e| s.world.get::<Kin>(e)).map(|k| k.pos)
}

pub(crate) fn side_of(s: &Sim, id: EntityId) -> Option<Side> {
    ent(s, id).and_then(|e| s.world.get::<Ident>(e)).map(|i| i.side)
}

pub(crate) fn push_trigger(s: &mut Sim, c4: &mut C4, entity: EntityId, point: &'static str, about: Option<EntityId>, q: QCtx) {
    c4.qctx.insert(entity, q);
    s.ctx.triggers.push(Trigger { entity, point, about });
}

fn bearer_of(b: bgws_catalogue::Bearer) -> BearerKind {
    use bgws_catalogue::Bearer as B;
    match b {
        B::Hf => BearerKind::Hf,
        B::Vhf => BearerKind::Vhf,
        B::Uhf => BearerKind::Uhf,
        B::SatcomMil => BearerKind::SatcomGeo,
        B::SatcomLeo => BearerKind::SatcomLeo,
        B::Tdl => BearerKind::Tdl,
        B::Mesh => BearerKind::Mesh,
        B::Wire => BearerKind::Wire,
        B::Fibre => BearerKind::Fibre,
        B::Courier => BearerKind::Courier,
    }
}

/// Classify one platform for these domains.
pub(crate) fn role_of(p: &Platform, catalogue: &bgws_catalogue::Catalogue, pr: &C4Params, drone_front: bool) -> Role {
    let mut r = Role { land: p.domain == Domain::Land, ..Default::default() };
    for radio in &p.radios {
        r.radios.push(RadioFit {
            kind: bearer_of(radio.bearer),
            freq_hz: radio.freq_hz,
            power_w: radio.power_w,
            gain_dbi: radio.antenna_gain_dbi,
            agl: if radio.antenna_height_m > 0.0 { radio.antenna_height_m } else { pr.ground_antenna_m },
            rate_bps: radio.data_rate_bps,
        });
    }
    for e in &p.emitters {
        match e.kind {
            bgws_catalogue::EmitterKind::Jammer if !e.jams.is_empty() => r.jammer = Some(e.clone()),
            bgws_catalogue::EmitterKind::Radar => r.radars.push((e.freq_hz, e.erp_w, e.duty_cycle)),
            bgws_catalogue::EmitterKind::DroneLink => r.drone_link = Some((if e.freq_hz > 0.0 { e.freq_hz } else { pr.drone_link_hz }, e.erp_w.max(0.01), if e.bandwidth_hz > 0.0 { e.bandwidth_hz } else { pr.jam_width_l })),
            _ => {}
        }
    }
    for s in &p.sensors {
        match s.kind {
            CatSensor::Esm => {
                if let Some(esm) = &s.esm {
                    r.esm = Some((s.range_max_m, *esm, s.height_m.max(2.0)));
                }
                r.cuas_sensors.push(CuasSensor::RfDetector);
            }
            CatSensor::Radar => r.cuas_sensors.push(CuasSensor::Radar),
            CatSensor::Acoustic => r.cuas_sensors.push(CuasSensor::Acoustic),
            k if k.is_imaging_eo() => r.cuas_sensors.push(CuasSensor::Eo),
            _ => {}
        }
    }
    r.cuas_sensors.sort();
    r.cuas_sensors.dedup();
    if r.cuas_sensors.is_empty() && r.land {
        r.cuas_sensors.push(CuasSensor::Eo);
    }
    for (i, m) in p.mounts.iter().enumerate() {
        use bgws_catalogue::WeaponKind as W;
        match m.kind {
            W::GunAd => r.ad_gun = r.ad_gun.or(Some(i)),
            W::InterceptorDrone => r.interceptor = r.interceptor.or(Some(i)),
            W::SamShort => r.sam = Some(r.sam.filter(|&j| p.mounts[j].kind == W::SamShort).unwrap_or(i)),
            W::SamMedium => r.sam = r.sam.or(Some(i)),
            W::SmallArms => r.small_arms = true,
            _ => {}
        }
    }
    if let Some(d) = &p.carried.drones {
        if catalogue.platform(&d.archetype).is_some() && d.count > 0 {
            r.team = Some((d.archetype.clone(), d.count));
        }
    }
    let role = p.role.to_lowercase();
    r.artillery = p.mounts.iter().any(|m| matches!(m.kind, bgws_catalogue::WeaponKind::Howitzer | bgws_catalogue::WeaponKind::Mortar | bgws_catalogue::WeaponKind::Rocket)) || role.contains("artillery");
    r.armoured = matches!(p.body.vulnerability, bgws_catalogue::Vulnerability::Armoured | bgws_catalogue::Vulnerability::Light);
    r.cage = drone_front && r.armoured && pr.cage_share >= 1.0;
    r.ins = if !r.land {
        None
    } else if r.armoured || p.class.contains("cp") || role.contains("command") || r.artillery {
        Some(InsGrade::Tactical)
    } else {
        Some(InsGrade::Mems)
    };
    r
}

// ── Hooks (crate::world) ────────────────────────────────────────────────

/// After the forces are spawned: roles, nets, HQs, jammers, trackers, teams.
pub fn setup(s: &mut Sim) {
    let p = C4Params::from_figures(&s.ctx.figures);
    let radio = RadioParams::from_figures(&s.ctx.figures);
    let drone_front = s.ctx.scenario.scope.kind == bgws_scenario::scope::ScenarioType::DroneLine;
    let mut roles = BTreeMap::new();
    let mut emcon = BTreeMap::new();
    let mut last_pos = BTreeMap::new();
    for (i, f) in s.ctx.scenario.forces.iter().enumerate() {
        let id = EntityId(i as u32);
        let Some(e) = ent(s, id) else { continue };
        let Some(ident) = s.world.get::<Ident>(e) else { continue };
        let mut r = role_of(&ident.platform, &s.ctx.catalogue, &p, drone_front);
        // A share of cages below one: the first vehicles of each side by id get them.
        if drone_front && r.armoured && p.cage_share < 1.0 {
            r.cage = (i as f64 * 0.618_033_988_75).fract() < p.cage_share;
        }
        roles.insert(id, r);
        emcon.insert(id, s.ctx.scenario.emitters.emcon_of(f));
        if let Some(k) = s.world.get::<Kin>(e) {
            last_pos.insert(id, k.pos);
        }
    }
    let mut c4 = C4 {
        p,
        radio,
        roles,
        emcon,
        on_air: BTreeMap::new(),
        qctx: BTreeMap::new(),
        losses: LossCache::default(),
        comms: comms::CommsState::default(),
        c2: c2::C2State::default(),
        ew: ew::EwState::default(),
        pnt: pnt::PntState::default(),
        drones: drones::DroneState::default(),
        last_pos,
    };
    c2::setup(s, &mut c4);
    comms::setup(s, &mut c4);
    ew::setup(s, &mut c4);
    pnt::setup(s, &mut c4);
    drones::setup(s, &mut c4);
    put(s, c4);
}

pub fn fast_active(_s: &Sim) -> bool {
    false
}

/// Drones in the air: 10 Hz.
pub fn air_active(s: &Sim) -> bool {
    s.world.get_resource::<C4>().map(|c| !c.drones.flights.is_empty()).unwrap_or(false)
}

/// Every stop of the clock: drones fly.
pub fn fast(s: &mut Sim, dt: f64) {
    drones::fly(s, dt);
}

/// 1 Hz: the nets, the staffs, EW, the drone fight.
pub fn base(s: &mut Sim, dt: f64) {
    moved(s);
    comms::base(s);
    c2::base(s);
    ew::base(s, dt);
    drones::base(s, dt);
}

/// 0.1 Hz: link budgets, GNSS, the COPs.
pub fn slow(s: &mut Sim, _dt: f64) {
    comms::slow(s);
    pnt::slow(s);
    c2::slow(s);
    ew::slow(s);
}

/// Each minute: operators tire or rest, caches age.
pub fn minute(s: &mut Sim, dt: f64) {
    drones::minute(s, dt);
    if let Some(mut c4) = take(s) {
        let before = SimTime(s.ctx.now.0 - 120_000_000);
        c4.losses.prune(before);
        put(s, c4);
    }
}

/// Theatre events: GNSS and satcom jamming, cuts, restores.
pub fn script_event(s: &mut Sim, e: &bgws_scenario::v4::ScriptedEvent) {
    let Some(mut c4) = take(s) else { return };
    let until = e.duration_s.map(|d| s.ctx.now + bgws_core::SimDuration::from_secs(d));
    match e.kind.as_str() {
        "jamGnss" => {
            c4.pnt.side_jam.insert(e.side, (c4.p.side_jam_dbw, until));
        }
        "jamSatcom" | "cutLeoSatcom" => {
            let leo_only = e.kind == "cutLeoSatcom";
            c4.comms.satcom_down.insert((e.side, leo_only), until);
            comms::apply_satcom_state(s, &mut c4);
        }
        "restore" => {
            c4.pnt.side_jam.remove(&e.side);
            c4.comms.satcom_down.retain(|(side, _), _| *side != e.side);
            comms::apply_satcom_state(s, &mut c4);
        }
        _ => {}
    }
    put(s, c4);
}

/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    match ev {
        Scheduled::HqProduct { hq, product } => {
            c2::product_due(s, hq, product);
            None
        }
        Scheduled::Timer { system, key } if system.starts_with("c4.") => {
            match system.as_str() {
                "c4.handover" => c2::handover_due(s, key as usize),
                "c4.unsilence" => {
                    if let Some(mut c4) = take(s) {
                        c4.emcon.insert(EntityId(key as u32), Emcon::Free);
                        put(s, c4);
                    }
                }
                "c4.relocate" => drones::relocate(s, EntityId(key as u32)),
                _ => {}
            }
            None
        }
        other => Some(other),
    }
}

/// World facts for a unit's template walk.
pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    let Some(c4) = s.world.get_resource::<C4>() else { return };
    let Some(i) = s.world.get::<Ident>(e) else { return };
    let id = i.id;
    m.insert("agent.domain".into(), FactValue::Text(format!("{:?}", i.domain).to_lowercase()));
    m.insert("agent.archetype".into(), FactValue::Text(i.platform.id.clone()));
    let on_air = c4.on_air.get(&id).copied().unwrap_or(0.0);
    m.insert("agent.onAirS".into(), FactValue::Num(on_air.round()));
    let role = c4.roles.get(&id);
    m.insert("agent.armed".into(), FactValue::Bool(s.world.get::<Arms>(e).map(|a| a.rounds() > 0).unwrap_or(false)));
    if let Some(t) = c4.drones.teams.get(&id) {
        let full = t.batteries_start.max(1) as f64;
        m.insert("agent.batteryPct".into(), FactValue::Num((t.stores.batteries as f64 / full * 100.0).round()));
        m.insert("agent.hasFibreDrones".into(), FactValue::Bool(t.kind == bgws_ops::drones::DroneType::FpvFibre));
        m.insert("agent.canAutonomous".into(), FactValue::Bool(matches!(t.spec.on_link_loss, bgws_ops::drones::LinkLossBehaviour::AiTerminal | bgws_ops::drones::LinkLossBehaviour::ContinueToAimPoint)));
    }
    let q = c4.qctx.get(&id);
    if let Some(f) = q.and_then(|q| q.flight).and_then(|f| c4.drones.flights.get(&f)) {
        m.insert("link.quality".into(), FactValue::Num((f.link.quality * 100.0).round() / 100.0));
        let full = f.spec.endurance_s.max(1.0);
        m.insert("agent.batteryPct".into(), FactValue::Num((f.endurance_s / full * 100.0).round()));
    }
    if let Some(n) = c4.pnt.recs.get(&id) {
        m.insert("pnt.quality".into(), FactValue::Num(if n.has_fix { 1.0 } else { ((n.cn0 - 20.0) / 25.0).clamp(0.0, 1.0) }));
    }
    if let (Some(at), Some(k)) = (q.and_then(|q| q.at), s.world.get::<Kin>(e)) {
        m.insert("target.rangeKm".into(), FactValue::Num((at.sub(k.pos).horizontal() / 100.0).round() / 10.0));
    }
    let limit = if c4.c2.post_of(id).is_some() {
        c4.p.emit_limit_s
    } else if role.map(|r| r.team.is_some()).unwrap_or(false) {
        c4.p.operator_exposed_s
    } else {
        c4.p.expose_s
    };
    m.insert("threat.located".into(), FactValue::Bool(on_air >= limit));
    m.insert("roe.weaponsFree".into(), FactValue::Bool(s.world.get::<Act>(e).map(|a| a.roe == RoeS::Always).unwrap_or(true)));
}

/// Domain capabilities for the gates.
pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    let Some(c4) = s.world.get_resource::<C4>() else { return };
    let Some(i) = s.world.get::<Ident>(e) else { return };
    let id = i.id;
    if let Some(r) = c4.roles.get(&id) {
        if r.jammer.is_some() {
            c.insert(Capability::Jammer);
            c.insert(Capability::Emitter);
        }
        if !r.radars.is_empty() {
            c.insert(Capability::Radar);
            c.insert(Capability::Emitter);
        }
        if r.radios.is_empty() {
            c.remove(&Capability::Radio);
        }
    }
    if c4.comms.cut.contains(&id) {
        c.remove(&Capability::Radio);
    }
    if let Some(t) = c4.drones.teams.get(&id) {
        let aloft = c4.drones.flights.values().any(|f| f.team == id);
        if t.stores.drones.values().sum::<u32>() > 0 || aloft {
            c.insert(Capability::Drones);
        }
        if t.kind == bgws_ops::drones::DroneType::FpvFibre || t.fibre_stock > 0 {
            c.insert(Capability::FibreDrones);
        }
        if matches!(t.spec.on_link_loss, bgws_ops::drones::LinkLossBehaviour::AiTerminal | bgws_ops::drones::LinkLossBehaviour::ContinueToAimPoint) {
            c.insert(Capability::Autonomy);
        }
    }
}

/// The EMCON gate for a unit's options.
pub fn emcon_gate(s: &Sim, e: Entity) -> GateEmcon {
    let Some(c4) = s.world.get_resource::<C4>() else { return GateEmcon::Unrestricted };
    let Some(i) = s.world.get::<Ident>(e) else { return GateEmcon::Unrestricted };
    match c4.emcon_of(i.id) {
        Emcon::Free => GateEmcon::Unrestricted,
        Emcon::Limited => GateEmcon::Restricted,
        Emcon::Silent => GateEmcon::Silent,
    }
}

/// Carry out a `drones.*`, `ew.*` or `c2.*` action; false if it is not one.
pub fn apply_action(s: &mut Sim, e: Entity, id: EntityId, action: &str, params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    if let Some(a) = action.strip_prefix("drones.") {
        return drones::apply_action(s, e, id, a, params, about);
    }
    if let Some(a) = action.strip_prefix("ew.") {
        return ew::apply_action(s, e, id, a, about);
    }
    if let Some(a) = action.strip_prefix("c2.") {
        return c2::apply_action(s, e, id, a, about);
    }
    false
}

pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}

/// Who moved since the last base tick: their time on the air starts again.
fn moved(s: &mut Sim) {
    let Some(mut c4) = take(s) else { return };
    let tol = c4.p.move_tolerance_m;
    for &e in &s.ctx.order {
        let (Some(i), Some(k)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e)) else { continue };
        let last = c4.last_pos.entry(i.id).or_insert(k.pos);
        if last.sub(k.pos).horizontal() > tol {
            *last = k.pos;
            c4.on_air.insert(i.id, 0.0);
        }
    }
    put(s, c4);
}

/// Every figure this module declares (also in `crate::params::all_figures`).
pub fn all_figures() -> Vec<Figure> {
    figures()
}

/// Shared platform handle for a catalogue id.
pub(crate) fn platform(s: &Sim, id: &str) -> Option<Arc<Platform>> {
    s.ctx.catalogue.platform(id).cloned().map(Arc::new)
}
