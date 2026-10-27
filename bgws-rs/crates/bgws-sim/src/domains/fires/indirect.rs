//! Indirect fire and counter-battery (§8.8 "Indirect fire", the TypeScript
//! `world/domains/fires.ts` chain):
//!
//! request → fires cell → shooter (its `fireRequestReceived` decision) → lay
//! → fire (time of flight from the ballistic solution) → rounds land
//! (a scheduled event) → effects → the gun decides whether to move
//! (`counterBatteryThreat`).
//!
//! - **Who calls:** a steady unit that sees an enemy beyond the reach of its
//!   own weapons, or one firing on it, when a shooter on its side can reach.
//!   The aim is where the observer believes the target is.
//! - **Delivery:** a fixed bias per mission (met error growing with the age
//!   of the met message, the gun's survey/PNT error, orientation) and
//!   per-round dispersion, from `indirect_fire::delivery_errors`; guided
//!   rounds use their CEP. While the observer still sees the target the
//!   rounds are adjusted (`AdjustFire`) before fire for effect.
//! - **Effects:** every entity near a burst is hit with the Carleton damage
//!   function for its posture (standing, prone, dug in, soft or armoured
//!   vehicle), integrated over the element's footprint; every burst close by
//!   is felt as a blast (suppression).
//! - **Counter-battery:** a weapon-locating radar in whose cover a gun fires
//!   tracks the rounds and, after its time to locate, backtracks the gun with
//!   an error that shrinks with rounds tracked; the fix is fed to its side's
//!   tracks and its side's fires answer **[ref R4.3]**. A gun moves only
//!   after its displacement time **[P23]**.

use super::{alive, mode, name, raise, FiresWorld, Gun};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_behaviour::FactValue;
use bgws_catalogue::{Guidance, MoverKind, Munition, MunitionKind, Platform, SensorKind as CatSensor, Vulnerability, WeaponKind};
use bgws_core::{Enu, EntityId, Side, SimDuration, SimTime};
use bgws_know::{ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::indirect_fire::{self as ifm, AdjustFire, Carleton, FiringSystem, RoundClass, TargetPosture};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use nalgebra::Matrix3;
use rand::Rng;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Mission ids start here, so a `RoundsLand` of another domain is never ours.
pub const ID_BASE: u64 = 1 << 40;

pub fn owns(mission: u64) -> bool {
    (ID_BASE..2 * ID_BASE).contains(&mission)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MState {
    Requested,
    Assigned,
    Laying,
    Firing,
    InFlight,
    Complete,
    Refused,
}

/// Rounds of one salvo on their way.
#[derive(Clone, Debug)]
pub struct Salvo {
    pub land_at: SimTime,
    pub munition: String,
    pub round: RoundClass,
    pub bearing: f64,
    pub impacts: Vec<Enu>,
    pub adjusting: bool,
}

/// One fire mission (rule 10: what happened at each step is kept).
#[derive(Clone, Debug)]
pub struct Mission {
    pub id: u64,
    pub side: Side,
    pub observer: Option<EntityId>,
    pub target: Option<EntityId>,
    /// Where the caller believes the target is.
    pub aim: Enu,
    pub requested_at: SimTime,
    pub heard_at: SimTime,
    pub priority: u8,
    pub state: MState,
    pub shooter: Option<EntityId>,
    pub assigned_at: SimTime,
    pub refused_by: BTreeSet<EntityId>,
    pub fire_at: SimTime,
    pub next_at: SimTime,
    pub careful: bool,
    pub scoot: bool,
    pub counter_battery: bool,
    pub rounds_total: u32,
    pub rounds_left: u32,
    pub fired: u32,
    pub adjust: Option<AdjustFire>,
    /// Delivery bias (range, deflection), m, drawn at the first round.
    pub bias: Option<(f64, f64)>,
    pub salvos: Vec<Salvo>,
    pub first_fired: Option<SimTime>,
    pub kills: u32,
    pub reason: String,
}

/// A trajectory, cached by round and range bucket.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Traj {
    pub tof_s: f64,
    pub qe_rad: f64,
    pub charge: usize,
    pub apex_m: f64,
}

/// Rounds a radar has tracked from one gun since its last fix.
#[derive(Clone, Debug, Default)]
pub struct CbTrack {
    pub rounds: u32,
    pub last_fix: Option<SimTime>,
}

/// A radar's backtracked gun position, delivered after its time to locate.
#[derive(Clone, Debug)]
pub struct CbFix {
    pub radar: EntityId,
    pub side: Side,
    pub gun: EntityId,
    pub pos: Enu,
    pub truth: Enu,
    pub loc: ifm::CbLocation,
    pub range_m: f64,
    pub rounds: u32,
}

pub struct FiresState {
    pub missions: BTreeMap<u64, Mission>,
    pub next: u64,
    pub recall: BTreeMap<(EntityId, EntityId), SimTime>,
    /// Shooter → the mission its `fireRequestReceived` question is about.
    pub asking: BTreeMap<EntityId, u64>,
    pub cb: BTreeMap<(EntityId, EntityId), CbTrack>,
    pub fixes: BTreeMap<u64, CbFix>,
    pub next_fix: u64,
    pub traj: BTreeMap<(u8, i64), Traj>,
}

impl FiresState {
    pub fn new(_p: &super::FParams) -> Self {
        FiresState { missions: BTreeMap::new(), next: ID_BASE, recall: BTreeMap::new(), asking: BTreeMap::new(), cb: BTreeMap::new(), fixes: BTreeMap::new(), next_fix: 0, traj: BTreeMap::new() }
    }
}

// ── Classification ───────────────────────────────────────────────────────

/// A weapon-locating (counter-battery) radar: a `cbr` sensor, or the role says so.
pub fn is_wlr(p: &Platform) -> bool {
    p.sensors.iter().any(|s| s.kind == CatSensor::Cbr) || {
        let r = p.role.to_lowercase();
        r.contains("counter-battery") || r.contains("weapon-locating")
    }
}

fn firing_system(p: &Platform) -> FiringSystem {
    let kinds: Vec<WeaponKind> = p.mounts.iter().map(|m| m.kind).collect();
    if kinds.contains(&WeaponKind::Rocket) {
        FiringSystem::Mlrs
    } else if kinds.contains(&WeaponKind::Howitzer) {
        if matches!(p.mover.kind, MoverKind::Tracked | MoverKind::Wheeled) && !p.class.to_lowercase().contains("towed") {
            FiringSystem::SelfPropelledGun
        } else {
            FiringSystem::TowedGun
        }
    } else {
        FiringSystem::Mortar
    }
}

pub fn round_class(m: &Munition) -> RoundClass {
    match (m.kind, m.family.as_str()) {
        (_, "mortar81") => RoundClass::Mortar81,
        (MunitionKind::Mortar, _) | (_, "mortar120") => RoundClass::Mortar120,
        (MunitionKind::Rocket, _) => RoundClass::Rocket122,
        _ => RoundClass::Howitzer155,
    }
}

fn guided(m: &Munition) -> bool {
    !matches!(m.guidance, Guidance::None)
}

/// Unit vectors: along the line of fire, and to its right.
fn frame(bearing: f64, r: f64, d: f64) -> Enu {
    Enu::new(bearing.sin() * r + bearing.cos() * d, bearing.cos() * r - bearing.sin() * d, 0.0)
}
fn unframe(bearing: f64, v: Enu) -> (f64, f64) {
    (v.e * bearing.sin() + v.n * bearing.cos(), v.e * bearing.cos() - v.n * bearing.sin())
}

/// A normal draw (Box–Muller; two uniforms, always).
pub(crate) fn normal(rng: &mut impl Rng, sigma: f64) -> f64 {
    let (u1, u2): (f64, f64) = (rng.gen::<f64>().max(1e-12), rng.gen());
    sigma * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

pub fn setup(s: &mut Sim, _st: &mut FiresWorld) {
    for &e in &s.ctx.order.clone() {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        let p = i.platform.clone();
        if (0..p.mounts.len()).any(|k| super::indirect_mount(&p, k)) {
            s.world.entity_mut(e).insert(Gun { system: firing_system(&p), fired_here: 0, located: false, displace_at: None });
        }
    }
}

// ── Shooters ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Shooter {
    id: EntityId,
    pos: Enu,
    /// (mount index, munition, min, max, guided, rounds).
    mounts: Vec<(usize, Munition, u32)>,
}

impl Shooter {
    fn reaches(&self, p: Enu) -> bool {
        let d = p.sub(self.pos).horizontal();
        self.mounts.iter().any(|(_, m, r)| *r > 0 && d >= m.min_range_m && d <= m.max_range_m)
    }
}

fn shooter_of(s: &Sim, e: Entity) -> Option<Shooter> {
    let i = s.world.get::<Ident>(e)?;
    let g = s.world.get::<Gun>(e)?;
    let h = s.world.get::<Health>(e)?;
    let c = s.world.get::<Crew>(e)?;
    let a = s.world.get::<Act>(e)?;
    if h.destroyed || !h.can_fire() || g.displace_at.is_some() || a.activity.moving() || c.morale.state != bgws_models::people::MoraleState::Steady {
        return None;
    }
    let arms = s.world.get::<Arms>(e)?;
    let mounts: Vec<(usize, Munition, u32)> = arms
        .mounts
        .iter()
        .filter(|m| super::indirect_mount(&i.platform, m.index) && m.rounds > 0)
        .filter_map(|m| Some((m.index, s.ctx.catalogue.munition(&m.munition)?.clone(), m.rounds)))
        .collect();
    if mounts.is_empty() {
        return None;
    }
    Some(Shooter { id: i.id, pos: s.world.get::<Kin>(e)?.pos, mounts })
}

fn shooters_of(s: &Sim, side: Side) -> Vec<Shooter> {
    s.ctx.order.iter().filter(|&&e| s.world.get::<Ident>(e).map(|i| i.side == side).unwrap_or(false)).filter_map(|&e| shooter_of(s, e)).collect()
}

fn busy(st: &FiresWorld, id: EntityId) -> bool {
    st.fires.asking.contains_key(&id) || st.fires.missions.values().any(|m| m.shooter == Some(id) && matches!(m.state, MState::Assigned | MState::Laying | MState::Firing))
}

// ── The base tick ────────────────────────────────────────────────────────

pub fn base(s: &mut Sim, st: &mut FiresWorld) {
    guns(s, st);
    if st.fires_mode == DomainMode::Off {
        return;
    }
    calls_for_fire(s, st);
    assign(s, st);
    fire(s, st);
    complete(s, st);
}

/// Guns coming out of action move off; a gun that moved is no longer where it fired from.
fn guns(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(g) = s.world.get::<Gun>(e).cloned() else { continue };
        if !alive(s, e) {
            continue;
        }
        let id = s.world.get::<Ident>(e).expect("ident").id;
        if let Some(t) = g.displace_at {
            if t <= now {
                let pos = s.world.get::<Kin>(e).expect("kin").pos;
                let mut rng = s.ctx.rng("fires.displace", id.0 as u64, 0);
                let away = rng.gen_range(0..8) as f64 * std::f64::consts::FRAC_PI_4;
                let to = pos.add(Enu::new(away.sin(), away.cos(), 0.0).scale(st.p.displace_m));
                let to = if s.ctx.ground.contains(to) { to } else { pos.sub(Enu::new(away.sin(), away.cos(), 0.0).scale(st.p.displace_m)) };
                if let Some(mut a) = s.world.get_mut::<Act>(e) {
                    a.activity = Activity::Move { route: vec![to], mode: MoveModeS::March, then: Some(Box::new(Activity::Overwatch)) };
                }
                if let Some(mut g) = s.world.get_mut::<Gun>(e) {
                    g.displace_at = None;
                    g.fired_here = 0;
                    g.located = false;
                }
                let text = format!("{} moves off to a new position", name(s, id));
                let side = s.world.get::<Ident>(e).expect("ident").side;
                s.ctx.log.push(now, "fires.displace").side(side).entity(id).text(text).data(serde_json::json!({ "phase": "move", "toE": to.e, "toN": to.n }));
            }
            continue;
        }
        if s.world.get::<Act>(e).map(|a| a.activity.moving()).unwrap_or(false) && (g.fired_here > 0 || g.located) {
            if let Some(mut g) = s.world.get_mut::<Gun>(e) {
                g.fired_here = 0;
                g.located = false;
            }
        }
    }
}

/// Observers call for fire on what they see (TS `callsForFire`).
fn calls_for_fire(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    for side in Side::PLAYERS {
        let shooters = shooters_of(s, side);
        if shooters.is_empty() {
            continue;
        }
        let mut open: BTreeSet<EntityId> = st.fires.missions.values().filter(|m| m.side == side && !matches!(m.state, MState::Complete | MState::Refused)).filter_map(|m| m.target).collect();
        for &e in &s.ctx.order.clone() {
            let Some(i) = s.world.get::<Ident>(e) else { continue };
            if i.side != side || !alive(s, e) || i.domain != bgws_catalogue::Domain::Land || s.world.get::<Gun>(e).is_some() || super::is_node(&i.platform) {
                continue;
            }
            let (oid, platform) = (i.id, i.platform.clone());
            let crew = s.world.get::<Crew>(e).expect("crew");
            if crew.morale.state != bgws_models::people::MoraleState::Steady {
                continue;
            }
            let last_incoming = crew.last_incoming;
            let pos = s.world.get::<Kin>(e).expect("kin").pos;
            let own_reach = platform.mounts.iter().enumerate().filter(|(k, _)| !super::indirect_mount(&platform, *k)).filter_map(|(_, m)| m.munitions.first()).filter_map(|m| s.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max);
            let seen: Vec<(EntityId, Seen)> = s.world.get::<Aware>(e).expect("aware").seen.iter().filter(|(_, v)| (now - v.last).secs() <= 1.0).map(|(k, v)| (*k, v.clone())).collect();
            for (tid, v) in seen {
                if open.contains(&tid) || !alive(s, s.ctx.entity(tid)) {
                    continue;
                }
                if st.fires.recall.get(&(oid, tid)).map(|t| (now - *t).secs() < st.p.recall_s).unwrap_or(false) {
                    continue;
                }
                let range = v.at.sub(pos).horizontal();
                let firing_on_me = last_incoming.map(|(t, by)| by == tid && (now - t).secs() < 60.0).unwrap_or(false);
                if range <= own_reach * st.p.own_reach_frac && !firing_on_me {
                    continue;
                }
                if !shooters.iter().any(|sh| sh.reaches(v.at)) {
                    continue;
                }
                let armoured = v.level >= 1 && s.world.get::<Ident>(s.ctx.entity(tid)).map(|t| t.platform.body.vulnerability == Vulnerability::Armoured).unwrap_or(false);
                let priority = if firing_on_me { 3 } else { 1 } + armoured as u8;
                let mut rng = s.ctx.rng("fires.call", oid.0 as u64, tid.0 as u64);
                let sig = st.p.observer_sigma_m;
                let aim = v.at.add(Enu::new(normal(&mut rng, sig), normal(&mut rng, sig), 0.0));
                st.fires.recall.insert((oid, tid), now);
                open.insert(tid);
                let reason = if firing_on_me { "it is firing on us" } else { "seen beyond our reach" };
                request(s, st, side, Some(oid), Some(tid), aim, priority, false, reason);
            }
        }
    }
}

/// Put in a call for fire; it reaches the fires cell after the call delay.
#[allow(clippy::too_many_arguments)]
pub(crate) fn request(s: &mut Sim, st: &mut FiresWorld, side: Side, observer: Option<EntityId>, target: Option<EntityId>, aim: Enu, priority: u8, counter_battery: bool, reason: &str) -> u64 {
    let now = s.ctx.now;
    let id = st.fires.next;
    st.fires.next += 1;
    st.fires.missions.insert(
        id,
        Mission {
            id,
            side,
            observer,
            target,
            aim,
            requested_at: now,
            heard_at: now + SimDuration::from_secs(st.p.call_delay_s),
            priority,
            state: MState::Requested,
            shooter: None,
            assigned_at: now,
            refused_by: BTreeSet::new(),
            fire_at: now,
            next_at: now,
            careful: false,
            scoot: false,
            counter_battery,
            rounds_total: 0,
            rounds_left: 0,
            fired: 0,
            adjust: None,
            bias: None,
            salvos: vec![],
            first_fired: None,
            kills: 0,
            reason: reason.into(),
        },
    );
    let who = observer.map(|o| name(s, o)).unwrap_or_else(|| "the fires cell".into());
    let on = target.map(|t| format!(" on {}", name(s, t))).unwrap_or_default();
    let text = format!("{who} calls for fire{on} ({reason})");
    let mut b = s.ctx.log.push(now, "fires.request").side(side).text(text).data(serde_json::json!({ "mission": id - ID_BASE, "priority": priority, "aimE": aim.e, "aimN": aim.n, "counterBattery": counter_battery }));
    if let Some(o) = observer {
        b = b.entity(o);
    }
    if let Some(t) = target {
        b = b.target(t);
    }
    drop(b);
    id
}

/// The fires cell gives each waiting call to the nearest free shooter that reaches.
fn assign(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    // Questions that came to nothing (the shooter died, the answer went elsewhere): back in the queue.
    let stale: Vec<u64> = st
        .fires
        .missions
        .values()
        .filter(|m| m.state == MState::Assigned)
        .filter(|m| {
            let sh = m.shooter.expect("assigned");
            let e = s.ctx.entity(sh);
            !alive(s, e) || (!s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(false) && (now - m.assigned_at).secs() > 30.0)
        })
        .map(|m| m.id)
        .collect();
    for id in stale {
        let m = st.fires.missions.get_mut(&id).expect("mission");
        if let Some(sh) = m.shooter.take() {
            if st.fires.asking.get(&sh) == Some(&id) {
                st.fires.asking.remove(&sh);
            }
        }
        m.state = MState::Requested;
    }
    for side in Side::PLAYERS {
        let mut waiting: Vec<(u8, SimTime, u64)> = st.fires.missions.values().filter(|m| m.side == side && m.state == MState::Requested && m.heard_at <= now).map(|m| (m.priority, m.requested_at, m.id)).collect();
        if waiting.is_empty() {
            continue;
        }
        waiting.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut free: Vec<Shooter> = shooters_of(s, side).into_iter().filter(|sh| !busy(st, sh.id) && !s.world.get::<Act>(s.ctx.entity(sh.id)).map(|a| a.deciding).unwrap_or(true)).collect();
        for (_, requested_at, mid) in waiting {
            let (aim, refused) = {
                let m = &st.fires.missions[&mid];
                (m.aim, m.refused_by.clone())
            };
            let pick = free.iter().enumerate().filter(|(_, sh)| sh.reaches(aim) && !refused.contains(&sh.id)).min_by(|a, b| a.1.pos.sub(aim).horizontal().total_cmp(&b.1.pos.sub(aim).horizontal()).then(a.1.id.cmp(&b.1.id))).map(|(k, _)| k);
            let Some(k) = pick else {
                if (now - requested_at).secs() > st.p.give_up_s {
                    st.fires.missions.get_mut(&mid).expect("m").state = MState::Refused;
                    s.ctx.log.push(now, "fires.refused").side(side).text("no shooter could take a call for fire").data(serde_json::json!({ "mission": mid - ID_BASE }));
                }
                continue;
            };
            let sh = free.remove(k);
            {
                let m = st.fires.missions.get_mut(&mid).expect("m");
                m.state = MState::Assigned;
                m.shooter = Some(sh.id);
                m.assigned_at = now;
            }
            let target = st.fires.missions[&mid].target;
            if st.fires_mode == DomainMode::Abstracted {
                accept(s, st, mid, false, false);
                continue;
            }
            let fired_here = s.world.get::<Gun>(s.ctx.entity(sh.id)).map(|g| g.fired_here).unwrap_or(0);
            let rule = if fired_here >= st.p.located_after { "fireThenScoot" } else { "fireNow" };
            st.fires.asking.insert(sh.id, mid);
            if !raise(s, sh.id, "fireRequestReceived", target, Some(rule)) {
                st.fires.asking.remove(&sh.id);
                accept(s, st, mid, false, fired_here >= st.p.located_after);
            }
        }
    }
}

/// The shooter takes the mission: it stops where it is and lays.
fn accept(s: &mut Sim, st: &mut FiresWorld, mid: u64, careful: bool, scoot: bool) {
    let now = s.ctx.now;
    let Some(m) = st.fires.missions.get_mut(&mid) else { return };
    let Some(sh) = m.shooter else { return };
    let e = s.ctx.entity(sh);
    let mortar = s.world.get::<Gun>(e).map(|g| g.system == FiringSystem::Mortar).unwrap_or(false);
    let lay = if mortar { st.p.lay_mortar_s } else { st.p.lay_gun_s } * if careful { st.p.careful_lay_mult } else { 1.0 };
    m.state = MState::Laying;
    m.fire_at = now + SimDuration::from_secs(lay);
    m.careful = careful;
    m.scoot = scoot;
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        if a.activity.moving() {
            a.activity = Activity::Hold;
        }
    }
}

/// The trajectory for a round class at a range (cached by 250 m bucket).
fn trajectory(st: &mut FiresWorld, round: RoundClass, range: f64, mun: &Munition) -> Traj {
    let bucket = (range / 250.0).round().max(1.0) as i64;
    if let Some(t) = st.fires.traj.get(&(round as u8, bucket)) {
        return *t;
    }
    let r = bucket as f64 * 250.0;
    let ip = &st.p.ip;
    let sol = match round {
        RoundClass::Howitzer155 => ifm::solve(r, &ip.charges155, &ip.shell155, false, ip).ok(),
        RoundClass::Mortar120 => ifm::solve(r, &st.p.mortar120.1, &st.p.mortar120.0, true, ip).ok(),
        RoundClass::Mortar81 => ifm::solve(r, &st.p.mortar81.1, &st.p.mortar81.0, true, ip).ok(),
        RoundClass::Rocket122 => None,
    };
    let t = match sol {
        Some(sol) => Traj { tof_s: sol.trajectory.time_of_flight_s, qe_rad: sol.quadrant_elevation_rad, charge: sol.charge, apex_m: sol.trajectory.apex_m },
        // Rockets (and anything past the top charge): average speed from the catalogue.
        None => Traj { tof_s: r / mun.speed_ms.max(100.0), qe_rad: 0.0, charge: 0, apex_m: r * 0.25 },
    };
    st.fires.traj.insert((round as u8, bucket), t);
    t
}

/// The met message's age now, hours.
fn met_age_h(s: &Sim, st: &FiresWorld) -> f64 {
    (st.p.met_age0_h + s.ctx.now.secs() / 3600.0).rem_euclid(st.p.met_refresh_h)
}

/// Laid missions fire their salvos.
fn fire(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    let ids: Vec<u64> = st.fires.missions.values().filter(|m| matches!(m.state, MState::Laying | MState::Firing)).map(|m| m.id).collect();
    for mid in ids {
        let m = st.fires.missions[&mid].clone();
        let sh = m.shooter.expect("laying missions have a shooter");
        let e = s.ctx.entity(sh);
        if !alive(s, e) {
            st.fires.missions.get_mut(&mid).expect("m").state = if m.fired > 0 { MState::InFlight } else { MState::Refused };
            continue;
        }
        if m.fire_at > now || m.next_at > now {
            continue;
        }
        // A gun on the move does not fire; one that moved off has left the mission.
        if s.world.get::<Act>(e).map(|a| a.activity.moving()).unwrap_or(false) || s.world.get::<Gun>(e).map(|g| g.displace_at.is_some()).unwrap_or(false) {
            let mm = st.fires.missions.get_mut(&mid).expect("m");
            if mm.fired > 0 {
                mm.state = MState::InFlight;
                mm.rounds_left = 0;
            } else {
                mm.state = MState::Requested;
                mm.shooter = None;
            }
            continue;
        }
        let Some(shooter) = shooter_of(s, e) else {
            let mm = st.fires.missions.get_mut(&mid).expect("m");
            mm.state = if mm.fired > 0 { MState::InFlight } else { MState::Refused };
            mm.rounds_left = 0;
            continue;
        };
        let range = m.aim.sub(shooter.pos).horizontal();
        // A gun told its GNSS is denied (pntDegraded) keeps to its answer.
        let pnt = crate::domains::drones::pnt::policy(s, e);
        let gnss_ok = !matches!(pnt, Some(crate::domains::drones::pnt::PntPolicy::SwitchMunition | crate::domains::drones::pnt::PntPolicy::Wait));
        let want_guided = m.priority >= 4 && gnss_ok;
        let mut usable: Vec<&(usize, Munition, u32)> = shooter.mounts.iter().filter(|(_, mu, r)| *r > 0 && range >= mu.min_range_m && range <= mu.max_range_m).collect();
        usable.sort_by_key(|(k, mu, _)| (guided(mu) != want_guided, *k));
        let Some(&(mount_i, ref mun, mount_rounds)) = usable.first().copied() else {
            let mm = st.fires.missions.get_mut(&mid).expect("m");
            let text = format!("{} cannot fire: out of range or out of rounds", name(s, sh));
            mm.state = if mm.fired > 0 { MState::InFlight } else { MState::Refused };
            mm.rounds_left = 0;
            s.ctx.log.push(now, "fires.refused").side(m.side).entity(sh).text(text).data(serde_json::json!({ "mission": mid - ID_BASE, "rangeM": range }));
            continue;
        };
        let mun = mun.clone();
        let (fit, fp) = {
            let h = s.world.get::<Health>(e).expect("h");
            (h.fit, h.firepower_kills)
        };
        let tubes = fit.saturating_sub(fp).max(1);
        let round = round_class(&mun);
        let tr = trajectory(st, round, range, &mun);
        let bearing = m.aim.sub(shooter.pos).bearing();
        let reload_s = s.world.get::<Ident>(e).expect("i").platform.mounts.get(mount_i).map(|cm| cm.reload_s).unwrap_or(10.0);
        let mut rng = s.ctx.rng("fires.salvo", mid, m.fired as u64);
        // Errors: fixed per mission, then per round.
        let errs = ifm::delivery_errors(range, met_age_h(s, st), st.p.gun_pnt_sigma_m, &st.p.ip);
        // A GNSS-guided round's accuracy follows its navigation through the
        // jamming along its flight (§8.7) [P15].
        let guided_cep = if guided(&mun) && mun.guidance == bgws_catalogue::Guidance::Gnss {
            crate::domains::drones::pnt::precision_accuracy(s, shooter.pos, m.aim, tr.tof_s, Some(m.side)).cep_m.max(mun.cep_m)
        } else {
            mun.cep_m
        };
        let (disp_r, disp_d) = if guided(&mun) { (guided_cep / 1.1774, guided_cep / 1.1774) } else { (errs.disp_range_m, errs.disp_defl_m) };
        let bias = match m.bias {
            Some(b) => b,
            None if guided(&mun) => (0.0, 0.0),
            None => {
                let k = if m.careful { st.p.careful_bias } else { 1.0 };
                (normal(&mut rng, errs.bias_range_m * k), normal(&mut rng, errs.bias_defl_m * k))
            }
        };
        // The true target, if there is one and it lives; else the aim point.
        let truth = m.target.map(|t| s.ctx.entity(t)).filter(|&t| alive(s, t)).and_then(|t| s.world.get::<Kin>(t).map(|k| k.pos)).unwrap_or(m.aim);
        let (aim_r, aim_d) = unframe(bearing, m.aim.sub(truth));
        let observed = !m.counter_battery
            && !guided(&mun)
            && st.fires_mode == DomainMode::Simulated
            && m.observer.zip(m.target).map(|(o, t)| alive(s, s.ctx.entity(o)) && s.world.get::<Aware>(s.ctx.entity(o)).and_then(|a| a.seen.get(&t).map(|v| (now - v.last).secs() <= 10.0)).unwrap_or(false)).unwrap_or(false);
        // Adjust only before fire for effect has begun.
        let adjusting = observed && (m.fired == 0 || m.adjust.is_some()) && !m.adjust.as_ref().map(|a| a.ready).unwrap_or(false);
        let mut adjust = m.adjust.clone();
        let (impacts, n, next_s): (Vec<Enu>, u32, f64) = if adjusting {
            let a = adjust.get_or_insert_with(|| AdjustFire::new((aim_r + bias.0, aim_d + bias.1)));
            let r = a.step((disp_r, disp_d), st.p.adjust_sigma_m, &st.p.ip, &mut rng);
            (vec![truth.add(frame(bearing, r.impact.0, r.impact.1))], 1, st.p.ip.adjust_cycle_s.max(reload_s))
        } else {
            let rounds_total = if m.rounds_total == 0 { ifm::default_mission_rounds(tubes, &st.p.ip).ceil().max(1.0) as u32 } else { m.rounds_total };
            let left = if m.rounds_total == 0 { rounds_total } else { m.rounds_left };
            let n = left.min(tubes).min(mount_rounds).max(1);
            let centre = match &adjust {
                Some(a) => a.bias,
                None => (aim_r + bias.0, aim_d + bias.1),
            };
            let impacts = (0..n).map(|_| truth.add(frame(bearing, centre.0 + normal(&mut rng, disp_r), centre.1 + normal(&mut rng, disp_d)))).collect();
            let mm = st.fires.missions.get_mut(&mid).expect("m");
            mm.rounds_total = rounds_total;
            mm.rounds_left = left - n.min(left);
            (impacts, n, reload_s)
        };
        // Spend the rounds.
        {
            let mut arms = s.world.get_mut::<Arms>(e).expect("arms");
            if let Some(ms) = arms.mounts.iter_mut().find(|x| x.index == mount_i) {
                ms.rounds = ms.rounds.saturating_sub(n);
            }
        }
        if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
            sig.last_fired = Some(now);
        }
        if let Some(mut a) = s.world.get_mut::<Act>(e) {
            a.last_shot = Some(now);
            a.last_event = now;
        }
        let first = m.first_fired.is_none();
        if first {
            if let Some(mut g) = s.world.get_mut::<Gun>(e) {
                g.fired_here += 1;
            }
        }
        let land_at = now + SimDuration::from_secs(tr.tof_s);
        s.ctx.queue.schedule(land_at, Priority::Physical, Scheduled::RoundsLand { mission: mid });
        {
            let mm = st.fires.missions.get_mut(&mid).expect("m");
            mm.bias = Some(bias);
            mm.adjust = adjust.clone();
            mm.fired += n;
            mm.first_fired.get_or_insert(now);
            mm.next_at = now + SimDuration::from_secs(next_s);
            mm.salvos.push(Salvo { land_at, munition: mun.id.clone(), round, bearing, impacts: impacts.clone(), adjusting });
            mm.state = if !adjusting && mm.rounds_left == 0 { MState::InFlight } else { MState::Firing };
        }
        if first || adjusting {
            let text = format!(
                "{} {} {} {} at {:.1} km for {}",
                name(s, sh),
                if adjusting { "adjusts with" } else { "fires" },
                n,
                mun.id,
                range / 1000.0,
                m.observer.map(|o| name(s, o)).unwrap_or_else(|| "counter-battery".into())
            );
            let data = serde_json::json!({
                "mission": mid - ID_BASE, "munition": mun.id, "rounds": n, "rangeM": range, "tofS": tr.tof_s, "qeMil": tr.qe_rad * 3200.0 / std::f64::consts::PI, "charge": tr.charge + 1,
                "metAgeH": met_age_h(s, st), "biasRangeM": bias.0, "biasDeflM": bias.1, "biasSigma": [errs.bias_range_m, errs.bias_defl_m], "dispersion": [disp_r, disp_d], "adjusting": adjusting, "careful": m.careful,
            });
            let mut b = s.ctx.log.push(now, if adjusting { "fires.adjust" } else { "fires.fire" }).side(m.side).entity(sh).text(text).data(data);
            if let Some(t) = m.target {
                b = b.target(t);
            }
            drop(b);
        }
        cb_observe(s, st, sh, m.side, shooter.pos, n, tr);
    }
}

/// Missions whose last rounds have landed are complete; the gun decides whether to move.
fn complete(s: &mut Sim, st: &mut FiresWorld) {
    let now = s.ctx.now;
    let done: Vec<u64> = st.fires.missions.values().filter(|m| m.state == MState::InFlight && m.salvos.is_empty()).map(|m| m.id).collect();
    for mid in done {
        let m = st.fires.missions.get_mut(&mid).expect("m");
        m.state = MState::Complete;
        let (side, sh, kills, fired, scoot) = (m.side, m.shooter, m.kills, m.fired, m.scoot);
        let Some(sh) = sh else { continue };
        let text = format!("{}'s mission is complete: {} rounds, {} knocked out", name(s, sh), fired, kills);
        s.ctx.log.push(now, "fires.complete").side(side).entity(sh).text(text).data(serde_json::json!({ "mission": mid - ID_BASE, "rounds": fired, "kills": kills, "s": (now - st.fires.missions[&mid].requested_at).secs() }));
        let e = s.ctx.entity(sh);
        if !alive(s, e) {
            continue;
        }
        if scoot {
            displace(s, st, sh, "shoot and scoot");
        } else if st.fires_mode == DomainMode::Simulated && s.world.get::<Arms>(e).map(|a| a.rounds() > 0).unwrap_or(false) {
            let g = s.world.get::<Gun>(e).cloned();
            let located = g.map(|g| g.located || g.fired_here >= st.p.located_after).unwrap_or(false);
            raise(s, sh, "counterBatteryThreat", None, Some(if located { "displaceNow" } else { "stay" }));
        }
    }
}

/// A gun comes out of action; it moves off after its displacement time [P23].
pub(crate) fn displace(s: &mut Sim, st: &mut FiresWorld, gun: EntityId, why: &str) {
    let now = s.ctx.now;
    let e = s.ctx.entity(gun);
    let Some(system) = s.world.get::<Gun>(e).map(|g| g.system) else { return };
    if s.world.get::<Gun>(e).and_then(|g| g.displace_at).is_some() {
        return;
    }
    let t = ifm::displacement_time_s(system, &st.p.ip);
    if let Some(mut g) = s.world.get_mut::<Gun>(e) {
        g.displace_at = Some(now + SimDuration::from_secs(t));
    }
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Hold;
    }
    for m in st.fires.missions.values_mut().filter(|m| m.shooter == Some(gun) && matches!(m.state, MState::Assigned | MState::Laying | MState::Firing)) {
        if m.fired > 0 {
            m.state = MState::InFlight;
            m.rounds_left = 0;
        } else {
            m.state = MState::Requested;
            m.shooter = None;
        }
    }
    st.fires.asking.remove(&gun);
    let side = s.world.get::<Ident>(e).expect("i").side;
    let text = format!("{} comes out of action to displace ({why}); moving in {:.0} s", name(s, gun), t);
    s.ctx.log.push(now, "fires.displace").side(side).entity(gun).text(text).data(serde_json::json!({ "phase": "pack", "why": why, "displaceS": t }));
}

// ── Rounds landing ───────────────────────────────────────────────────────

/// The Carleton posture of an entity against a burst.
fn posture_of(s: &Sim, e: Entity) -> TargetPosture {
    let i = s.world.get::<Ident>(e).expect("i");
    let dug = s.world.get::<super::Dig>(e).map(|d| d.done).unwrap_or(false);
    match i.platform.body.vulnerability {
        Vulnerability::Armoured => TargetPosture::ArmouredVehicle,
        // Light armour (IFVs, SP guns, carriers) stops small fragments only.
        Vulnerability::Light => TargetPosture::SoftVehicle,
        Vulnerability::Personnel => {
            if dug {
                TargetPosture::Entrenched
            } else if s.world.get::<Act>(e).map(|a| a.posture == Posture::Moving).unwrap_or(false) {
                TargetPosture::Standing
            } else {
                TargetPosture::Prone
            }
        }
        _ => TargetPosture::SoftVehicle,
    }
}

/// Rounds of a mission land: lethality and suppression on everything near.
pub fn rounds_land(s: &mut Sim, st: &mut FiresWorld, mid: u64) {
    let now = s.ctx.now;
    let Some(m) = st.fires.missions.get_mut(&mid) else { return };
    let (due, later): (Vec<Salvo>, Vec<Salvo>) = std::mem::take(&mut m.salvos).into_iter().partition(|x| x.land_at <= now);
    m.salvos = later;
    let (side, shooter) = (m.side, m.shooter);
    for (k, salvo) in due.into_iter().enumerate() {
        let kills = burst(s, st, mid, k as u64, side, shooter, &salvo);
        st.fires.missions.get_mut(&mid).expect("m").kills += kills;
    }
}

/// One salvo's bursts against every entity in reach.
fn burst(s: &mut Sim, st: &mut FiresWorld, mid: u64, k: u64, side: Side, shooter: Option<EntityId>, salvo: &Salvo) -> u32 {
    let now = s.ctx.now;
    let ip = st.p.ip.clone();
    let scale = ip.round_scale[salvo.round as usize];
    let standing = Carleton::for_round(salvo.round, TargetPosture::Standing, &ip);
    let reach = 4.0 * standing.rx_m.max(standing.ry_m);
    let felt = st.p.suppress_radius_m * scale;
    let mut kills_total = 0;
    let mut effects = Vec::new();
    let mut found_guns = Vec::new();
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let (tid, tside, platform) = {
            let i = s.world.get::<Ident>(e).expect("i");
            if !matches!(i.domain, bgws_catalogue::Domain::Land | bgws_catalogue::Domain::Node) {
                continue;
            }
            (i.id, i.side, i.platform.clone())
        };
        let k_pos = s.world.get::<Kin>(e).expect("kin");
        if k_pos.agl > 5.0 {
            continue;
        }
        let pos = k_pos.pos;
        let fit = s.world.get::<Health>(e).expect("h").fit;
        let person = platform.body.vulnerability == Vulnerability::Personnel;
        let w = if person { st.p.spacing_person_m } else { st.p.spacing_vehicle_m } * (fit.max(1) as f64).sqrt();
        let near: Vec<Enu> = salvo.impacts.iter().copied().filter(|p| p.sub(pos).horizontal() <= reach + w).collect();
        let blasts = salvo.impacts.iter().filter(|p| p.sub(pos).horizontal() <= felt + w / 2.0).count();
        if tside != side && s.world.get::<super::Gun>(e).is_some() && salvo.impacts.iter().any(|p| p.sub(pos).horizontal() <= st.p.near_gun_m) {
            found_guns.push(tid);
        }
        if near.is_empty() && blasts == 0 {
            continue;
        }
        let posture = posture_of(s, e);
        let c = Carleton::for_round(salvo.round, posture, &ip);
        let stay = if platform.body.vulnerability == Vulnerability::Structure { platform.body.staying_power.max(1.0) } else { 1.0 };
        let mut q = 1.0;
        for p in &near {
            // The burst's offset from the element's centre, in (deflection, range).
            let (r, d) = unframe(salvo.bearing, p.sub(pos));
            q *= 1.0 - (c.mean_p_kill_area(w, w, (d, r), (0.0, 0.0)) / stay).clamp(0.0, 1.0);
        }
        let pk = 1.0 - q;
        let mut rng = s.ctx.rng("fires.burst", mid ^ (k << 32), tid.0 as u64);
        let kills = (0..fit).filter(|_| rng.gen::<f64>() < pk).count() as u32;
        if blasts > 0 {
            let mut crew = s.world.get_mut::<Crew>(e).expect("crew");
            crew.blasts_per_min += blasts as f64;
            if let Some(sh) = shooter {
                crew.last_incoming = Some((now, sh));
            }
        }
        if kills > 0 {
            let destroyed = {
                let mut h = s.world.get_mut::<Health>(e).expect("h");
                h.fit -= kills;
                for _ in 0..kills {
                    h.losses.push(now);
                }
                if h.fit == 0 {
                    h.destroyed = true;
                }
                s.world.get_mut::<Crew>(e).expect("crew").new_casualties += kills as f64 / s.world.get::<Health>(e).expect("h").start as f64;
                s.world.get::<Health>(e).expect("h").destroyed
            };
            kills_total += kills;
            if destroyed {
                let text = format!("{} is destroyed by indirect fire", name(s, tid));
                s.ctx.log.push(now, "unit.destroyed").side(tside).entity(tid).text(text);
            }
        }
        if kills > 0 || pk > 0.01 {
            effects.push(serde_json::json!({ "entity": tid.0, "pk": pk, "kills": kills, "blasts": blasts, "posture": format!("{posture:?}"), "lethalAreaM2": c.lethal_area() }));
        }
    }
    let centre = salvo.impacts.iter().fold(Enu::default(), |a, p| a.add(*p)).scale(1.0 / salvo.impacts.len().max(1) as f64);
    let text = format!("{} rounds of {} land{}: {} knocked out", salvo.impacts.len(), salvo.munition, if salvo.adjusting { " (adjusting)" } else { "" }, kills_total);
    let mut b = s.ctx.log.push(now, "fires.impact").side(side).text(text).data(serde_json::json!({ "mission": mid - ID_BASE, "munition": salvo.munition, "rounds": salvo.impacts.len(), "centreE": centre.e, "centreN": centre.n, "kills": kills_total, "effects": effects, "adjusting": salvo.adjusting }));
    if let Some(sh) = shooter {
        b = b.entity(sh);
    }
    drop(b);
    // Rounds close to a gun: it knows it has been found.
    for g in found_guns {
        let e = s.ctx.entity(g);
        if let Some(mut gun) = s.world.get_mut::<Gun>(e) {
            gun.located = true;
        }
        if st.fires_mode == DomainMode::Simulated && s.world.get::<Gun>(e).map(|x| x.displace_at.is_none()).unwrap_or(false) {
            raise(s, g, "counterBatteryThreat", None, Some("displaceNow"));
        }
    }
    kills_total
}

// ── Counter-battery ──────────────────────────────────────────────────────

/// Enemy weapon-locating radars in whose cover a gun fires track its rounds.
fn cb_observe(s: &mut Sim, st: &mut FiresWorld, gun: EntityId, side: Side, gun_pos: Enu, rounds: u32, _tr: Traj) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        if i.side == side || i.side == Side::Neutral || !is_wlr(&i.platform) || !alive(s, e) {
            continue;
        }
        if s.world.get::<Act>(e).map(|a| a.activity == Activity::Quiet).unwrap_or(false) {
            continue;
        }
        let rid = i.id;
        let rside = i.side;
        let (range_max, for_deg) = i.platform.sensors.iter().find(|x| x.kind == CatSensor::Cbr).map(|x| (x.range_max_m, x.for_deg)).unwrap_or((40_000.0, 360.0));
        let k = s.world.get::<Kin>(e).expect("kin");
        let d = gun_pos.sub(k.pos);
        let range = d.horizontal();
        if range > range_max {
            continue;
        }
        let off = (d.bearing() - k.heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        if for_deg < 360.0 && off.abs() > for_deg.to_radians() / 2.0 {
            continue;
        }
        let mut rng = s.ctx.rng("fires.cb.track", rid.0 as u64, gun.0 as u64);
        let tracked = (0..rounds).filter(|_| rng.gen::<f64>() < st.p.cb_p_track).count() as u32;
        if tracked == 0 {
            continue;
        }
        let tr = st.fires.cb.entry((rid, gun)).or_default();
        tr.rounds += tracked;
        if tr.last_fix.map(|t| (now - t).secs() < st.p.cb_recall_s).unwrap_or(false) {
            continue;
        }
        let n = tr.rounds;
        tr.rounds = 0;
        tr.last_fix = Some(now);
        let loc = ifm::wlr_locate(range, n, &st.p.ip);
        let pos = gun_pos.add(Enu::new(normal(&mut rng, loc.sigma_m), normal(&mut rng, loc.sigma_m), 0.0));
        let key = st.fires.next_fix;
        st.fires.next_fix += 1;
        st.fires.fixes.insert(key, CbFix { radar: rid, side: rside, gun, pos, truth: gun_pos, loc: loc.clone(), range_m: range, rounds: n });
        s.ctx.queue.schedule(now + SimDuration::from_secs(loc.time_to_locate_s), Priority::Physical, Scheduled::Timer { system: "fires.cb".into(), key });
    }
}

/// A radar's fix is ready: its side's picture gets the gun, and its fires answer.
pub fn cb_fix(s: &mut Sim, st: &mut FiresWorld, key: u64) {
    let now = s.ctx.now;
    let Some(fix) = st.fires.fixes.remove(&key) else { return };
    if !alive(s, s.ctx.entity(fix.radar)) {
        return;
    }
    let sigma = fix.loc.sigma_m;
    s.ctx.reports.push((
        now,
        fix.side,
        Detection {
            time: now,
            observer: fix.radar,
            sensor: SensorKind::Radar,
            domain: bgws_know::Domain::Land,
            motion: MotionHint::Static,
            measurement: Measurement::Position { pos: fix.pos, cov: Matrix3::from_diagonal_element(sigma * sigma) },
            class: ClassEvidence { level: IdLevel::Recognise, reported: Some(Hypothesis::Artillery), resolves_decoys: false },
            truth: TruthTag::of(fix.gun),
        },
    ));
    let text = format!("{} tracks {} rounds back to a gun {:.1} km away (±{:.0} m)", name(s, fix.radar), fix.rounds, fix.range_m / 1000.0, fix.loc.cep_m);
    s.ctx.log.push(now, "fires.counterBattery").side(fix.side).entity(fix.radar).target(fix.gun).text(text).data(serde_json::json!({
        "sigmaM": sigma, "cepM": fix.loc.cep_m, "timeToLocateS": fix.loc.time_to_locate_s, "rangeM": fix.range_m, "rounds": fix.rounds, "errorM": fix.pos.sub(fix.truth).horizontal(),
    }));
    if mode(s, ScopeDomain::Fires) == DomainMode::Off {
        return;
    }
    let open = st.fires.missions.values().any(|m| m.side == fix.side && m.target == Some(fix.gun) && !matches!(m.state, MState::Complete | MState::Refused));
    if !open {
        request(s, st, fix.side, Some(fix.radar), Some(fix.gun), fix.pos, 4, true, "counter-battery");
    }
}

// ── Decisions ────────────────────────────────────────────────────────────

/// Facts for a shooter's mission question.
pub fn mission_facts(s: &Sim, w: &FiresWorld, id: EntityId, m: &mut BTreeMap<String, FactValue>) {
    let Some(mid) = w.fires.asking.get(&id) else { return };
    let Some(mi) = w.fires.missions.get(mid) else { return };
    let Some(sh) = shooter_of(s, s.ctx.entity(id)) else { return };
    m.insert("target.rangeKm".into(), FactValue::Num((mi.aim.sub(sh.pos).horizontal() / 100.0).round() / 10.0));
    m.insert("target.inRange".into(), FactValue::Bool(sh.reaches(mi.aim)));
    if let Some(t) = mi.target {
        let identified = s.world.get::<Aware>(s.ctx.entity(mi.observer.unwrap_or(id))).and_then(|a| a.seen.get(&t).map(|v| v.level >= 2)).unwrap_or(false);
        m.insert("target.identified".into(), FactValue::Bool(identified));
    }
}

/// The `fires.*` actions.
pub fn apply(s: &mut Sim, st: &mut FiresWorld, e: Entity, id: EntityId, action: &str) -> bool {
    let current = st.fires.asking.remove(&id).or_else(|| st.fires.missions.values().find(|m| m.shooter == Some(id) && m.state == MState::Assigned).map(|m| m.id));
    let _ = e;
    match action {
        "fires.fireNow" | "fires.fireWhenLaid" | "fires.fireThenScoot" => {
            if let Some(mid) = current {
                accept(s, st, mid, action == "fires.fireWhenLaid", action == "fires.fireThenScoot");
            }
            true
        }
        "fires.refuse" => {
            if let Some(mid) = current {
                let m = st.fires.missions.get_mut(&mid).expect("m");
                m.state = MState::Requested;
                m.shooter = None;
                m.refused_by.insert(id);
                let side = m.side;
                let text = format!("{} refuses the mission", name(s, id));
                s.ctx.log.push(s.ctx.now, "fires.refuse").side(side).entity(id).text(text).data(serde_json::json!({ "mission": mid - ID_BASE }));
            }
            true
        }
        "fires.displaceNow" => {
            if let Some(mid) = current {
                st.fires.asking.insert(id, mid);
            }
            displace(s, st, id, "counter-battery threat");
            true
        }
        "fires.finishThenDisplace" => {
            if let Some(mid) = current {
                st.fires.asking.insert(id, mid);
            }
            let next = st.fires.missions.values_mut().find(|m| m.shooter == Some(id) && matches!(m.state, MState::Assigned | MState::Laying | MState::Firing));
            match next {
                Some(m) => m.scoot = true,
                None => displace(s, st, id, "finished its missions"),
            }
            true
        }
        "fires.stay" => {
            if let Some(mid) = current {
                st.fires.asking.insert(id, mid);
            }
            true
        }
        _ => {
            if let Some(mid) = current {
                st.fires.asking.insert(id, mid);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn frames_round_trip() {
        for b in [0.0, 0.7, 2.0, -1.3] {
            let v = frame(b, 120.0, -35.0);
            let (r, d) = unframe(b, v);
            assert_relative_eq!(r, 120.0, epsilon = 1e-9);
            assert_relative_eq!(d, -35.0, epsilon = 1e-9);
        }
        // Firing north: range is north, deflection east.
        let v = frame(0.0, 10.0, 5.0);
        assert_relative_eq!(v.n, 10.0, epsilon = 1e-9);
        assert_relative_eq!(v.e, 5.0, epsilon = 1e-9);
    }

    #[test]
    fn mission_ids_do_not_clash() {
        assert!(owns(ID_BASE) && owns(ID_BASE + 5));
        assert!(!owns(3) && !owns(super::super::logistics::ID_BASE));
    }
}
