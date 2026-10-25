//! Drones and counter-drone (§8.7; research R4, R5.1, R5.2, P8, P9).
//!
//! - **Teams** are land units with drones and batteries in their stores
//!   (`bgws_ops::drones::DroneStores`) and an operator with skill and
//!   fatigue. An ISR team keeps a drone over the nearest known enemy (or
//!   the front); a strike team launches at targets it knows — its own
//!   drones' sightings and the targets its HQ hands it over the nets — a
//!   couple at a time.
//! - **Flights** are records flown at every clock stop (10 Hz while any is
//!   up) with a battery, a fibre spool or an RF link. The link is a link
//!   budget each second (`bgws_ops::drones::assess_link`): the operator's
//!   uplink at the drone and the drone's video at the operator, each
//!   through terrain path loss, against the noise and the J from every
//!   jammer at that end. It breaks with a hazard from its quality; then the
//!   drone's autonomy decides (`link_loss_outcome`).
//! - **Cameras** search with ACQUIRE from the drone's height: ISR sightings
//!   become the team's reports; a strike drone over its aim point searches
//!   until it finds a target or its search time runs out.
//! - **The FPV strike chain:** find (the camera), approach (counter-drone
//!   fire and the link, both played second by second), terminal
//!   (`terminal_hit_probability` from the operator's skill and fatigue,
//!   target motion, wind and link quality, with passive measures), effect
//!   (kill given hit by target class) applied to the target. Hit and kill
//!   rates are measured, not set (P9).
//! - **Counter-drone:** every ground unit near a drone may detect it by its
//!   sensors (`bgws_ops::counter_drone::detect`) and engage it with what it
//!   has: AD guns, small arms, interceptor drones, short-range SAMs. Jammers
//!   act through the link budget. Cages are a passive measure.
//! - **Drone-observed:** a unit under an enemy drone's camera feels it
//!   (stress, §8.10) and is asked what to do when it hears one overhead.

use super::{alive_id, ent, flight_key, pos_of, push_trigger, side_of, LossQuery, QCtx, C4};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bgws_catalogue::{Domain, Platform, SensorKind as CatSensor, Vulnerability};
use bgws_core::{hazard_probability, Enu, EntityId, Side, SimDuration, SimTime, TrackId};
use bgws_env::{transmission, OpticalBand};
use bgws_know::{ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::sensing::acquire::{acquire, AcquireInput, Contrast, Metric, Search, Stage};
use bgws_ops::counter_drone::{defeat, detect, passive_modifiers, DefeatMeans, DetectionInput, Engagement, PassiveMeasure};
use bgws_ops::drones::{assess_link, link_loss_outcome, terminal_hit_probability, ControlLink, DroneSpec, DroneStores, DroneType, LinkAssessment, LinkInputs, LinkLossBehaviour, Operator, StrikeInputs, StrikeParams, StrikeStage, TargetClass};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use bgws_scenario::v4::Emcon;
use nalgebra::Matrix3;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The control link this second.
#[derive(Clone, Debug, Default)]
pub struct LinkRec {
    pub quality: f64,
    pub p_control: f64,
    pub p_video: f64,
    pub up_js_db: f64,
    pub down_js_db: f64,
    pub lost: bool,
    /// Flying on alone (autonomy after link loss, or chosen).
    pub autonomous: bool,
    pub asked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Orbit { at: Enu, radius: f64, agl: f64 },
    /// Fly to the aim point and search there.
    Strike { aim: Enu, agl: f64 },
    /// Diving on a target in the camera.
    Terminal,
    LoiterSearch { at: Enu, radius: f64, until: SimTime },
    Return,
    Hover,
}

/// A drone in the air.
#[derive(Clone, Debug)]
pub struct Flight {
    pub id: u64,
    pub team: EntityId,
    pub side: Side,
    pub kind: DroneType,
    pub spec: DroneSpec,
    pub platform: Arc<Platform>,
    pub pos: Enu,
    pub agl: f64,
    pub heading: f64,
    pub task: Task,
    pub launched: SimTime,
    pub endurance_s: f64,
    pub flown_m: f64,
    pub link: LinkRec,
    pub target: Option<EntityId>,
    pub search_from: Option<SimTime>,
    pub found_at: Option<SimTime>,
    pub battery_asked: bool,
    /// When the sighting that cued this strike was made.
    pub cue: Option<SimTime>,
    /// Finished: an outcome to record, or recovered.
    pub done: bool,
}

/// Where a team thinks a target is.
#[derive(Clone, Debug)]
pub struct TargetInfo {
    pub at: Enu,
    pub seen: SimTime,
    pub value: f64,
}

/// A drone team.
#[derive(Clone, Debug)]
pub struct TeamRec {
    pub entity: EntityId,
    pub side: Side,
    pub archetype: String,
    pub platform: Arc<Platform>,
    pub kind: DroneType,
    pub spec: DroneSpec,
    pub stores: DroneStores,
    pub batteries_start: u32,
    pub operator: Operator,
    pub last_launch: SimTime,
    pub link_losses: u32,
    pub jammed_asked: SimTime,
    pub targets: BTreeMap<u64, TargetInfo>,
    pub handoff: Option<u64>,
    /// Fibre drones it can switch to.
    pub fibre_stock: u32,
    pub out_logged: bool,
}

impl TeamRec {
    pub fn isr(&self) -> bool {
        matches!(self.kind, DroneType::ReconMultirotor | DroneType::FixedWingIsr)
    }
}

/// How one strike sortie ended (for the AAR and the P9 vignette).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrikeRecord {
    pub flight: u64,
    pub team: EntityId,
    pub side: Side,
    pub kind: DroneType,
    pub target: Option<EntityId>,
    pub stage: StrikeStage,
    pub launched: SimTime,
    pub at: SimTime,
    /// Operator to strike point, m.
    pub range_m: f64,
    pub p_hit: f64,
    pub p_kill: f64,
    /// The striking side first saw this target at.
    pub first_seen: Option<SimTime>,
    /// The sighting that cued the strike was made at (detect-to-strike starts here).
    pub cued: Option<SimTime>,
}

#[derive(Default)]
pub struct DroneState {
    pub active: bool,
    pub teams: BTreeMap<EntityId, TeamRec>,
    pub flights: BTreeMap<u64, Flight>,
    pub next_id: u64,
    pub observed: BTreeSet<EntityId>,
    pub overhead_asked: BTreeMap<EntityId, SimTime>,
    pub cuas_tracks: BTreeMap<(EntityId, u64), SimTime>,
    pub records: Vec<StrikeRecord>,
    pub found_asked: BTreeMap<(Side, EntityId), SimTime>,
    pub first_seen: BTreeMap<(Side, EntityId), SimTime>,
    pub hiding: BTreeMap<EntityId, SimTime>,
    pub sorties: u32,
    /// Casualties not yet a whole section.
    pub casualties: BTreeMap<EntityId, f64>,
    pub interceptor_fired: BTreeSet<(EntityId, u64)>,
    pub sp: Option<StrikeParams>,
}

/// The drone type of a catalogue drone record.
pub fn drone_type(p: &Platform) -> DroneType {
    let id = p.id.to_lowercase();
    if id.contains("fibre") {
        DroneType::FpvFibre
    } else if id.contains("fpv") {
        DroneType::FpvRf
    } else if id.contains("loiter") {
        DroneType::LoiteringMunition
    } else if id.contains("bomber") {
        DroneType::Bomber
    } else if id.contains("interceptor") {
        DroneType::Interceptor
    } else if id.contains("owa") {
        DroneType::OwaShahed
    } else if p.mover.kind == bgws_catalogue::MoverKind::FixedWing {
        DroneType::FixedWingIsr
    } else {
        DroneType::ReconMultirotor
    }
}

fn strike_type(k: DroneType) -> bool {
    matches!(k, DroneType::FpvRf | DroneType::FpvFibre | DroneType::LoiteringMunition | DroneType::Bomber)
}

pub fn setup(s: &mut Sim, c4: &mut C4) {
    c4.drones.sp = Some(StrikeParams::from_figures(&s.ctx.figures));
    if super::mode(s, ScopeDomain::Drones) != DomainMode::Simulated {
        return;
    }
    c4.drones.active = true;
    for (id, r) in &c4.roles {
        let Some((arch, count)) = &r.team else { continue };
        let Some(e) = ent(s, *id) else { continue };
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        let Some(dp) = super::platform(s, arch) else { continue };
        let kind = drone_type(&dp);
        let spec = DroneSpec::from_figures(kind, &s.ctx.figures);
        let team_wh = i.platform.stores.battery_wh;
        let batteries = if dp.mover.battery_wh > 0.0 && team_wh > 0.0 { (team_wh / dp.mover.battery_wh).floor() as u32 } else { *count * 2 };
        let stores = DroneStores { drones: [(kind, *count)].into_iter().collect(), batteries };
        let skill = (i.quality as f64 - 1.0) / 5.0;
        c4.drones.teams.insert(
            *id,
            TeamRec {
                entity: *id,
                side: i.side,
                archetype: arch.clone(),
                platform: dp,
                kind,
                spec,
                stores,
                batteries_start: batteries,
                operator: Operator { skill: skill.clamp(0.0, 1.0), fatigue: 0.0 },
                last_launch: SimTime::from_secs(-1e6),
                link_losses: 0,
                jammed_asked: SimTime::from_secs(-1e6),
                targets: BTreeMap::new(),
                handoff: None,
                fibre_stock: if kind == DroneType::FpvRf { count / 2 } else { 0 },
                out_logged: false,
            },
        );
    }
}

fn value_of_hyp(h: Hypothesis) -> f64 {
    match h {
        Hypothesis::Artillery => 4.0,
        Hypothesis::Mbt => 3.0,
        Hypothesis::Ifv | Hypothesis::Apc => 2.5,
        Hypothesis::Truck => 2.0,
        Hypothesis::Infantry => 1.0,
        _ => 1.5,
    }
}

fn value_of(p: &Platform, hq: bool) -> f64 {
    if hq {
        return 4.0;
    }
    let r = p.role.to_lowercase();
    if r.contains("artillery") || r.contains("mortar") || r.contains("jam") || p.id.contains("sph") || p.id.contains("mlrs") {
        return 4.0;
    }
    match p.body.vulnerability {
        Vulnerability::Armoured => 3.0,
        Vulnerability::Light => 2.5,
        Vulnerability::Soft => 2.0,
        _ => 1.0,
    }
}

fn target_class(p: &Platform, in_cover: bool) -> TargetClass {
    let r = p.role.to_lowercase();
    if r.contains("artillery") || p.id.contains("sph") || p.id.contains("mortar") {
        return TargetClass::Artillery;
    }
    match p.body.vulnerability {
        Vulnerability::Armoured => TargetClass::Mbt,
        Vulnerability::Light => TargetClass::Ifv,
        Vulnerability::Personnel => {
            if in_cover {
                TargetClass::InfantryCover
            } else {
                TargetClass::InfantryOpen
            }
        }
        _ => TargetClass::Truck,
    }
}

/// An HQ hands a target to a team (arrived over the nets).
pub fn hand_target(c4: &mut C4, team: EntityId, track: TrackId, at: Enu, value: f64, seen: SimTime, now: SimTime) {
    let Some(t) = c4.drones.teams.get_mut(&team) else { return };
    let key = (1u64 << 32) + track.0 as u64;
    t.targets.insert(key, TargetInfo { at, seen, value });
    let _ = now;
}

/// An HQ's COP has a land target: the nearest strike team in reach gets it.
pub fn offer_target(s: &mut Sim, c4: &mut C4, h: usize, entry: TrackId, v: &bgws_know::TrackView) {
    if !c4.drones.active {
        return;
    }
    let now = s.ctx.now;
    let at = v.position;
    let mut best: Option<(EntityId, f64)> = None;
    for (id, t) in &c4.drones.teams {
        if c4.c2.unit_hq.get(id) != Some(&h) || !strike_type(t.kind) || t.stores.drones.values().sum::<u32>() == 0 || !alive_id(s, *id) {
            continue;
        }
        let Some(p) = pos_of(s, *id) else { continue };
        let d = p.sub(at).horizontal();
        if d > t.spec.range_m * 0.9 {
            continue;
        }
        if best.map(|b| d < b.1).unwrap_or(true) {
            best = Some((*id, d));
        }
    }
    let Some((team, _)) = best else { return };
    if c4.c2.hqs[h].handed.get(&(entry, team)).map(|t| (now - *t).secs() < c4.p.handoff_gap_s).unwrap_or(false) {
        return;
    }
    c4.c2.hqs[h].handed.insert((entry, team), now);
    let value = v.class_top.first().map(|(h, _)| value_of_hyp(*h)).unwrap_or(1.5);
    let post = c4.c2.hqs[h].post;
    let payload = super::comms::Payload::Target { track: entry, at, value, seen: v.last_update };
    super::comms::send(now, c4, post, team, bgws_ops::comms::MessageKind::PlanOrder, 1, Some(2_000.0), payload);
}

fn ground_at(s: &Sim, p: Enu) -> f64 {
    s.ctx.ground.height(p.e, p.n)
}

/// Every clock stop: the drones fly.
pub fn fly(s: &mut Sim, dt: f64) {
    let Some(mut c4) = super::take(s) else { return };
    if c4.drones.flights.is_empty() || dt <= 0.0 {
        super::put(s, c4);
        return;
    }
    let climb = c4.p.climb_mps;
    let orbit_r = c4.p.orbit_radius_m;
    let ids: Vec<u64> = c4.drones.flights.keys().copied().collect();
    let mut lost: Vec<(u64, &'static str)> = Vec::new();
    let mut home: Vec<u64> = Vec::new();
    for fid in ids {
        let team_pos = c4.drones.flights.get(&fid).and_then(|f| pos_of(s, f.team));
        let target_pos = c4.drones.flights.get(&fid).and_then(|f| f.target).and_then(|t| pos_of(s, t));
        let f = c4.drones.flights.get_mut(&fid).expect("flight");
        if f.done {
            continue;
        }
        f.endurance_s -= dt;
        if f.endurance_s <= 0.0 {
            lost.push((fid, "battery flat"));
            continue;
        }
        let (goal, goal_agl) = match &f.task {
            Task::Orbit { at, radius, agl } => {
                let d = f.pos.sub(*at);
                if d.horizontal() > radius * 1.2 {
                    (*at, *agl)
                } else {
                    let b = d.bearing() + 0.5;
                    (at.add(Enu::new(b.sin() * radius, b.cos() * radius, 0.0)), *agl)
                }
            }
            Task::LoiterSearch { at, radius, .. } => {
                let d = f.pos.sub(*at);
                if d.horizontal() > radius * 1.2 {
                    (*at, 300.0)
                } else {
                    let b = d.bearing() + 0.3;
                    (at.add(Enu::new(b.sin() * radius, b.cos() * radius, 0.0)), 300.0)
                }
            }
            Task::Strike { aim, agl } => {
                let d = f.pos.sub(*aim);
                if d.horizontal() > orbit_r {
                    (*aim, *agl)
                } else {
                    let b = d.bearing() + 0.6;
                    (aim.add(Enu::new(b.sin() * orbit_r * 0.7, b.cos() * orbit_r * 0.7, 0.0)), *agl)
                }
            }
            Task::Terminal => match target_pos {
                Some(p) => {
                    let d = p.sub(f.pos).horizontal();
                    (p, if d > 150.0 { 60.0 } else { (d * 0.4).max(1.0) })
                }
                None => (f.pos, f.agl),
            },
            Task::Return => match team_pos {
                Some(p) => (p, 20.0),
                None => {
                    lost.push((fid, "nowhere to land"));
                    continue;
                }
            },
            Task::Hover => (f.pos, f.agl),
        };
        let d = Enu::new(goal.e - f.pos.e, goal.n - f.pos.n, 0.0);
        let dist = d.horizontal();
        let step = (f.spec.speed_mps * dt).min(dist);
        if dist > 1e-6 {
            f.pos = Enu::new(f.pos.e + d.e / dist * step, f.pos.n + d.n / dist * step, 0.0);
            f.heading = d.bearing();
        }
        f.flown_m += step;
        let dz = (goal_agl - f.agl).clamp(-climb * dt * 3.0, climb * dt);
        f.agl = (f.agl + dz).max(1.0);
        if f.task == Task::Return && dist <= 30.0 {
            home.push(fid);
        }
        if let ControlLink::Fibre { spool_m } = f.spec.link {
            if let Some(tp) = team_pos {
                if f.pos.sub(tp).horizontal() > spool_m {
                    lost.push((fid, "fibre spool run out"));
                }
            }
        }
    }
    let heights: Vec<(u64, f64)> = c4.drones.flights.iter().map(|(k, f)| (*k, ground_at(s, f.pos))).collect();
    for (k, g) in heights {
        if let Some(f) = c4.drones.flights.get_mut(&k) {
            f.pos.u = g + f.agl;
        }
    }
    for (fid, why) in lost {
        end_flight(s, &mut c4, fid, StrikeStage::LinkLost, why, false);
    }
    for fid in home {
        recover(s, &mut c4, fid);
    }
    c4.drones.flights.retain(|_, f| !f.done);
    super::put(s, c4);
}

fn recover(s: &mut Sim, c4: &mut C4, fid: u64) {
    let Some(f) = c4.drones.flights.get_mut(&fid) else { return };
    f.done = true;
    let (team, kind) = (f.team, f.kind);
    if let Some(t) = c4.drones.teams.get_mut(&team) {
        *t.stores.drones.entry(kind).or_insert(0) += 1;
    }
    let text = format!("{}'s drone back", crate::systems::name(s, team));
    s.ctx.log.push(s.ctx.now, "drones.recovered").side(f_side(c4, team)).entity(team).text(text).data(serde_json::json!({ "flight": fid }));
}

fn f_side(c4: &C4, team: EntityId) -> Side {
    c4.drones.teams.get(&team).map(|t| t.side).unwrap_or(Side::Neutral)
}

/// A flight ends without reaching a terminal attack.
fn end_flight(s: &mut Sim, c4: &mut C4, fid: u64, stage: StrikeStage, why: &str, link: bool) {
    let now = s.ctx.now;
    let Some(f) = c4.drones.flights.get_mut(&fid) else { return };
    if f.done {
        return;
    }
    f.done = true;
    let f = f.clone();
    if link {
        if let Some(t) = c4.drones.teams.get_mut(&f.team) {
            t.link_losses += 1;
        }
    }
    let striking = strike_type(f.kind) && matches!(f.task, Task::Strike { .. } | Task::Terminal | Task::LoiterSearch { .. } | Task::Hover);
    if striking {
        let range = pos_of(s, f.team).map(|p| p.sub(f.pos).horizontal()).unwrap_or(0.0);
        let first = f.target.and_then(|t| c4.drones.first_seen.get(&(f.side, t)).copied());
        c4.drones.records.push(StrikeRecord { flight: fid, team: f.team, side: f.side, kind: f.kind, target: f.target, stage, launched: f.launched, at: now, range_m: range, p_hit: 0.0, p_kill: 0.0, first_seen: first, cued: f.cue });
    }
    let text = format!("{}'s {} lost: {}", crate::systems::name(s, f.team), f.platform.name, why);
    s.ctx.log.push(now, "drones.lost").side(f.side).entity(f.team).text(text).data(serde_json::json!({ "flight": fid, "why": why, "stage": stage, "e": f.pos.e, "n": f.pos.n }));
    // Losing drones to the link: the team asks what to do about the jamming.
    if link {
        let team = f.team;
        let (losses, asked) = c4.drones.teams.get(&team).map(|t| (t.link_losses, t.jammed_asked)).unwrap_or((0, now));
        if losses >= c4.p.jammed_losses && (now - asked).secs() > 600.0 && alive_id(s, team) {
            if let Some(t) = c4.drones.teams.get_mut(&team) {
                t.jammed_asked = now;
                t.link_losses = 0;
            }
            push_trigger(s, c4, team, "jammedHeavily", None, QCtx { flight: Some(fid), ..Default::default() });
        }
    }
}

/// 1 Hz: launches, links, cameras, strikes, counter-drone, overhead.
pub fn base(s: &mut Sim, _dt: f64) {
    let Some(mut c4) = super::take(s) else { return };
    if !c4.drones.active {
        super::put(s, c4);
        return;
    }
    let now = s.ctx.now;
    first_seen(s, &mut c4);
    let cycle = (c4.p.team_cycle_s.max(1.0) * 1e6) as i64;
    if now.0 % cycle == 0 {
        teams(s, &mut c4);
    }
    links(s, &mut c4);
    cameras(s, &mut c4);
    terminal(s, &mut c4);
    counter_drone(s, &mut c4);
    overhead(s, &mut c4);
    c4.drones.flights.retain(|_, f| !f.done);
    super::put(s, c4);
}

/// When each side first saw each enemy (own sightings), for detect-to-strike.
fn first_seen(s: &Sim, c4: &mut C4) {
    for &e in &s.ctx.order {
        let (Some(i), Some(a)) = (s.world.get::<Ident>(e), s.world.get::<Aware>(e)) else { continue };
        for (t, v) in &a.seen {
            c4.drones.first_seen.entry((i.side, *t)).or_insert(v.first);
        }
    }
}

fn wind_mps(s: &Sim) -> f64 {
    s.ctx.scenario.environment.wind_ms.unwrap_or_else(|| {
        let w = s.ctx.weather.sample(0.0, 0.0, s.ctx.now).wind_surface;
        w.0.hypot(w.1)
    })
}

fn nearest_briefed(s: &Sim, side: Side, me: Enu) -> Option<Enu> {
    s.ctx.scenario.forces_of(side.enemy()).map(|f| s.ctx.frame.geodetic_to_enu(f.position)).min_by(|a, b| a.sub(me).horizontal().total_cmp(&b.sub(me).horizontal()))
}

fn enemy_centroid(s: &Sim, side: Side) -> Option<Enu> {
    let v: Vec<Enu> = s.ctx.scenario.forces_of(side.enemy()).map(|f| s.ctx.frame.geodetic_to_enu(f.position)).collect();
    (!v.is_empty()).then(|| v.iter().fold(Enu::default(), |a, b| a.add(*b)).scale(1.0 / v.len() as f64))
}

/// Teams decide whether to launch.
fn teams(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let wind = wind_mps(s);
    let team_ids: Vec<EntityId> = c4.drones.teams.keys().copied().collect();
    for id in team_ids {
        let Some(e) = ent(s, id) else { continue };
        if !crate::systems::alive(s, e) {
            continue;
        }
        let steady = s.world.get::<Crew>(e).map(|c| matches!(c.morale.state, bgws_models::people::MoraleState::Steady | bgws_models::people::MoraleState::Shaken)).unwrap_or(false);
        let moving = s.world.get::<Act>(e).map(|a| a.activity.moving()).unwrap_or(false);
        if !steady || moving {
            continue;
        }
        let me = pos_of(s, id).unwrap_or_default();
        let t = c4.drones.teams[&id].clone();
        let rf = t.spec.jammable();
        if rf && c4.emcon_of(id) == Emcon::Silent {
            continue;
        }
        let multirotor = t.platform.mover.kind == bgws_catalogue::MoverKind::Multirotor;
        if multirotor && wind > c4.p.max_wind_mps {
            continue;
        }
        if t.stores.drones.get(&t.kind).copied().unwrap_or(0) == 0 {
            if !t.out_logged {
                c4.drones.teams.get_mut(&id).expect("team").out_logged = true;
                let text = format!("{} has no drones left", crate::systems::name(s, id));
                s.ctx.log.push(now, "drones.empty").side(t.side).entity(id).text(text);
            }
            continue;
        }
        let aloft: Vec<&Flight> = c4.drones.flights.values().filter(|f| f.team == id && !f.done).collect();
        let reach = match t.spec.link {
            ControlLink::Fibre { spool_m } => spool_m.min(t.spec.range_m),
            _ => t.spec.range_m,
        } * 0.9;
        // What the team knows, fresh, in reach.
        let mut known: Vec<(u64, TargetInfo)> = t.targets.iter().filter(|(_, x)| (now - x.seen).secs() <= c4.p.target_age_s && x.at.sub(me).horizontal() <= reach).map(|(k, v)| (*k, v.clone())).collect();
        if let Some(a) = s.world.get::<Aware>(e) {
            for (tid, v) in &a.seen {
                if (now - v.last).secs() <= c4.p.target_age_s && v.at.sub(me).horizontal() <= reach {
                    let val = ent(s, *tid).and_then(|x| s.world.get::<Ident>(x)).map(|i| value_of(&i.platform, i.hq)).unwrap_or(1.0);
                    known.push((tid.0 as u64, TargetInfo { at: v.at, seen: v.last, value: val }));
                }
            }
        }
        if !c4.comms.active {
            for v in s.ctx.tracks[&t.side].confirmed(&s.ctx.p.know) {
                if v.domain == bgws_know::Domain::Land && (now - v.last_update).secs() <= c4.p.target_age_s && v.position.sub(me).horizontal() <= reach {
                    known.push(((1u64 << 32) + v.id.0 as u64, TargetInfo { at: v.position, seen: v.last_update, value: v.class_top.first().map(|(h, _)| value_of_hyp(*h)).unwrap_or(1.5) }));
                }
            }
        }
        if t.isr() {
            if !aloft.is_empty() {
                continue;
            }
            let near = known.iter().min_by(|a, b| a.1.at.sub(me).horizontal().total_cmp(&b.1.at.sub(me).horizontal()).then(a.0.cmp(&b.0)));
            let watch = match near {
                Some((_, x)) if x.at.sub(me).horizontal() < reach * 0.8 => x.at,
                // No target: watch the nearest enemy position the briefing gave, or push toward it.
                _ => match nearest_briefed(s, t.side, me) {
                    Some(c) => {
                        let d = c.sub(me);
                        let n = d.horizontal().max(1.0);
                        me.add(d.scale(n.min(reach * 0.8).max(c4.p.isr_push_m.min(n)) / n))
                    }
                    None => me,
                },
            };
            let task = Task::Orbit { at: watch, radius: c4.p.orbit_radius_m, agl: c4.p.orbit_agl_m };
            launch(s, c4, id, task, "watch", None);
            continue;
        }
        if aloft.len() >= c4.p.max_aloft || (now - t.last_launch).secs() < c4.p.launch_gap_s {
            continue;
        }
        let taken: Vec<Enu> = c4.drones.flights.values().filter(|f| f.side == t.side).filter_map(|f| if let Task::Strike { aim, .. } = f.task { Some(aim) } else { None }).collect();
        known.retain(|(_, x)| !taken.iter().any(|a| a.sub(x.at).horizontal() < 150.0));
        known.sort_by(|a, b| {
            let ha = Some(a.0) == t.handoff;
            let hb = Some(b.0) == t.handoff;
            hb.cmp(&ha).then(b.1.value.total_cmp(&a.1.value)).then(a.1.at.sub(me).horizontal().total_cmp(&b.1.at.sub(me).horizontal())).then(a.0.cmp(&b.0))
        });
        if let Some((k, x)) = known.first().cloned() {
            if t.handoff == Some(k) {
                c4.drones.teams.get_mut(&id).expect("team").handoff = None;
            }
            launch(s, c4, id, Task::Strike { aim: x.at, agl: c4.p.cruise_agl_m }, "strike", Some(x.seen));
        } else if t.kind == DroneType::LoiteringMunition {
            if let Some(c) = enemy_centroid(s, t.side) {
                let d = c.sub(me);
                let n = d.horizontal().max(1.0);
                let at = me.add(d.scale((reach * 0.7).min(c4.p.loiter_push_m) / n));
                let until = now + SimDuration::from_secs(t.spec.endurance_s * 0.6);
                launch(s, c4, id, Task::LoiterSearch { at, radius: 1500.0, until }, "hunt", None);
            }
        }
    }
}

fn launch(s: &mut Sim, c4: &mut C4, team: EntityId, task: Task, why: &str, cue: Option<SimTime>) {
    let now = s.ctx.now;
    let t = c4.drones.teams.get_mut(&team).expect("team");
    if let Err(err) = t.stores.launch(&t.spec.clone(), &s.ctx.figures) {
        if !t.out_logged {
            t.out_logged = true;
            let text = format!("{} cannot launch: {err}", crate::systems::name(s, team));
            s.ctx.log.push(now, "drones.empty").side(t.side).entity(team).text(text);
        }
        return;
    }
    t.last_launch = now;
    let id = c4.drones.next_id;
    c4.drones.next_id += 1;
    c4.drones.sorties += 1;
    let me = pos_of(s, team).unwrap_or_default();
    let f = Flight {
        id,
        team,
        side: t.side,
        kind: t.kind,
        spec: t.spec.clone(),
        platform: t.platform.clone(),
        pos: Enu::new(me.e, me.n, me.u + 20.0),
        agl: 20.0,
        heading: 0.0,
        task: task.clone(),
        launched: now,
        endurance_s: t.spec.endurance_s,
        flown_m: 0.0,
        link: LinkRec { quality: 1.0, p_control: 1.0, p_video: 1.0, ..Default::default() },
        target: None,
        search_from: None,
        found_at: None,
        battery_asked: false,
        cue,
        done: false,
    };
    let aim = match &task {
        Task::Strike { aim, .. } | Task::Orbit { at: aim, .. } | Task::LoiterSearch { at: aim, .. } => *aim,
        _ => me,
    };
    let text = format!("{} launches a {} ({why}, {:.1} km)", crate::systems::name(s, team), f.platform.name, aim.sub(me).horizontal() / 1000.0);
    s.ctx.log.push(now, "drones.launch").side(f.side).entity(team).text(text).data(serde_json::json!({ "flight": id, "kind": f.kind, "why": why, "rangeM": aim.sub(me).horizontal() }));
    c4.drones.flights.insert(id, f);
}

/// The link budget at both ends of a drone's control link, for a drone at
/// `drone` (agl `agl`) flown by `team`: the uplink at the drone and the
/// video at the operator, through terrain, against noise and jamming.
pub fn link_at(s: &Sim, team: EntityId, drone: Enu, agl: f64) -> Option<LinkAssessment> {
    let c4 = s.world.get_resource::<C4>()?;
    let t = c4.drones.teams.get(&team)?;
    let me = pos_of(s, team)?;
    let side = side_of(s, team)?;
    let (f, op_erp, bw) = c4.roles.get(&team)?.drone_link.unwrap_or((c4.p.drone_link_hz, 1.0, c4.p.jam_width_l));
    let drone_erp = t.platform.emitters.iter().find(|e| e.kind == bgws_catalogue::EmitterKind::DroneLink).map(|e| e.erp_w).unwrap_or(1.0);
    let agl_op = c4.p.gcs_mast_m;
    let pl = |a: Enu, ha: f64, b: Enu, hb: f64| bgws_prop::terrain_path_loss(&s.ctx.ground.terrain, bgws_prop::Terminal { e: a.e, n: a.n, agl: ha }, bgws_prop::Terminal { e: b.e, n: b.n, agl: hb }, f, &c4.radio).total_db;
    let l = pl(me, agl_op, drone, agl);
    let jam_at = |p: Enu, h: f64| {
        let mut v = Vec::new();
        for j in &c4.ew.jammers {
            if !j.jam.on || !j.jam.bands.iter().any(|b| b.covers(f, bw)) {
                continue;
            }
            let Some(jp) = pos_of(s, j.entity) else { continue };
            let mut x = j.jam.at_receiver(f, bw, pl(jp, j.agl, p, h), 0.0, 0.0).j_dbw;
            if j.side == side {
                x -= c4.p.own_deconfliction_db;
            }
            v.push(x);
        }
        bgws_ops::ew::combined_jamming_dbw(v)
    };
    let noise = noise_dbw(bw, c4.p.noise_figure_db);
    let i = LinkInputs { uplink_signal_dbw: db(op_erp) + c4.p.gcs_gain_dbi - l, uplink_jam_dbw: jam_at(drone, agl) - c4.p.control_gain_db, downlink_signal_dbw: db(drone_erp) + c4.p.gcs_gain_dbi - l, downlink_jam_dbw: jam_at(me, agl_op), noise_dbw: noise };
    Some(assess_link(&t.spec, &c4.drones.sp.as_ref()?.link, &i))
}

fn db(w: f64) -> f64 {
    10.0 * w.max(1e-12).log10()
}

fn noise_dbw(bw: f64, nf: f64) -> f64 {
    10.0 * (bgws_core::units::BOLTZMANN * bgws_core::units::STANDARD_TEMPERATURE_K * bw.max(1.0)).log10() + nf
}

/// Every RF drone's link budget this second; links that break.
fn links(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let sp = c4.drones.sp.clone().expect("strike params");
    let rf: Vec<u64> = c4.drones.flights.iter().filter(|(_, f)| !f.done && f.spec.jammable() && !f.link.lost && !f.link.autonomous).map(|(k, _)| *k).collect();
    let mut qs = Vec::new();
    let mut victims = Vec::new();
    let mut meta = Vec::new();
    for fid in &rf {
        let f = &c4.drones.flights[fid];
        let Some(me) = pos_of(s, f.team) else { continue };
        if !alive_id(s, f.team) {
            continue;
        }
        let (freq, op_erp, bw) = c4.roles.get(&f.team).and_then(|r| r.drone_link).unwrap_or((c4.p.drone_link_hz, 1.0, c4.p.jam_width_l));
        let drone_erp = f.platform.emitters.iter().find(|e| e.kind == bgws_catalogue::EmitterKind::DroneLink).map(|e| e.erp_w).unwrap_or(1.0);
        let agl_op = c4.p.gcs_mast_m;
        qs.push(LossQuery { a: f.team.0 as u64, b: flight_key(*fid), f_hz: freq, pa: me, agl_a: agl_op, pb: f.pos, agl_b: f.agl });
        victims.push(super::ew::Victim { key: flight_key(*fid), pos: f.pos, agl: f.agl, side: f.side, freq_hz: freq, bw_hz: bw, gnss: false, rx_gain_dbi: 0.0 });
        victims.push(super::ew::Victim { key: f.team.0 as u64, pos: me, agl: agl_op, side: f.side, freq_hz: freq, bw_hz: bw, gnss: false, rx_gain_dbi: 0.0 });
        meta.push((*fid, op_erp, drone_erp, bw));
    }
    let mut lost_ops: Vec<u64> = rf.iter().copied().filter(|fid| !alive_id(s, c4.drones.flights[fid].team)).collect();
    if !qs.is_empty() {
        let losses = c4.losses.resolve(&s.ctx.ground.terrain, &c4.radio, c4.p.move_tolerance_m, now, &qs);
        let js = super::ew::jamming_dbw(s, c4, &victims);
        for (k, (fid, op_erp, drone_erp, bw)) in meta.into_iter().enumerate() {
            let l = losses[k];
            let i = LinkInputs { uplink_signal_dbw: db(op_erp) + c4.p.gcs_gain_dbi - l, uplink_jam_dbw: js[2 * k] - c4.p.control_gain_db, downlink_signal_dbw: db(drone_erp) + c4.p.gcs_gain_dbi - l, downlink_jam_dbw: js[2 * k + 1], noise_dbw: noise_dbw(bw, c4.p.noise_figure_db) };
            let f = c4.drones.flights.get_mut(&fid).expect("flight");
            let a = assess_link(&f.spec, &sp.link, &i);
            f.link.quality = a.quality;
            f.link.p_control = a.p_control;
            f.link.p_video = a.p_video;
            f.link.up_js_db = a.uplink_js_db;
            f.link.down_js_db = a.downlink_js_db;
        }
    }
    // Breaks, by a hazard from the quality.
    for fid in rf {
        let Some(f) = c4.drones.flights.get(&fid) else { continue };
        if f.done {
            continue;
        }
        let q = f.link.quality.clamp(0.0, 1.0);
        let p_hold = q.powf(1.0 / sp.link.coherence_s.max(1e-3));
        let mut rng = s.ctx.rng("drones.link", fid, 0);
        let breaks = rng.gen::<f64>() >= p_hold;
        if !breaks && q < c4.p.link_poor && !f.link.asked && alive_id(s, f.team) {
            let team = f.team;
            c4.drones.flights.get_mut(&fid).expect("flight").link.asked = true;
            push_trigger(s, c4, team, "linkDegraded", None, QCtx { flight: Some(fid), ..Default::default() });
        }
        if breaks {
            lost_ops.push(fid);
        }
    }
    lost_ops.sort();
    lost_ops.dedup();
    for fid in lost_ops {
        link_lost(s, c4, fid, &sp);
    }
    // Fibre snags, and batteries running low.
    let ids: Vec<u64> = c4.drones.flights.keys().copied().collect();
    for fid in ids {
        let f = &c4.drones.flights[&fid];
        if f.done {
            continue;
        }
        if matches!(f.spec.link, ControlLink::Fibre { .. }) {
            let km = f.spec.speed_mps / 1000.0;
            let p = hazard_probability(sp.link.snag_per_km, km);
            if s.ctx.rng("drones.snag", fid, 0).gen::<f64>() < p {
                end_flight(s, c4, fid, StrikeStage::LinkLost, "fibre snagged", true);
                continue;
            }
        }
        let f = &c4.drones.flights[&fid];
        if !f.battery_asked && matches!(f.kind, DroneType::ReconMultirotor | DroneType::LoiteringMunition | DroneType::FixedWingIsr) && f.endurance_s / f.spec.endurance_s.max(1.0) < c4.p.battery_low && alive_id(s, f.team) {
            let team = f.team;
            c4.drones.flights.get_mut(&fid).expect("flight").battery_asked = true;
            push_trigger(s, c4, team, "batteryLow", None, QCtx { flight: Some(fid), ..Default::default() });
        }
    }
}

fn link_lost(s: &mut Sim, c4: &mut C4, fid: u64, sp: &StrikeParams) {
    let Some(f) = c4.drones.flights.get(&fid).cloned() else { return };
    if f.done {
        return;
    }
    let tspeed = f.target.and_then(|t| ent(s, t)).and_then(|e| s.world.get::<Kin>(e)).map(|k| k.vel.horizontal()).unwrap_or(0.0);
    let out = link_loss_outcome(&f.spec, tspeed, sp);
    let now = s.ctx.now;
    let text = format!("{}'s {} loses its link ({:?})", crate::systems::name(s, f.team), f.platform.name, out.behaviour);
    s.ctx.log.push(now, "drones.link_lost").side(f.side).entity(f.team).text(text).data(serde_json::json!({ "flight": fid, "quality": f.link.quality, "upJsDb": f.link.up_js_db, "downJsDb": f.link.down_js_db, "behaviour": out.behaviour }));
    let fm = c4.drones.flights.get_mut(&fid).expect("flight");
    fm.link.lost = true;
    match out.behaviour {
        LinkLossBehaviour::ReturnHome => {
            fm.task = Task::Return;
            if let Some(t) = c4.drones.teams.get_mut(&f.team) {
                t.link_losses += 1;
            }
        }
        LinkLossBehaviour::Hover => end_flight(s, c4, fid, StrikeStage::LinkLost, "link lost", true),
        LinkLossBehaviour::ContinueToAimPoint | LinkLossBehaviour::AiTerminal => {
            fm.link.autonomous = true;
            if !matches!(fm.task, Task::Strike { .. } | Task::Terminal | Task::LoiterSearch { .. }) {
                end_flight(s, c4, fid, StrikeStage::LinkLost, "link lost", true);
            }
        }
    }
}

/// A target's look to a camera: (critical dim, visual contrast, ΔT, concealment, moving).
fn signature(s: &Sim, c4: &C4, e: bevy_ecs::prelude::Entity) -> Option<(f64, f64, f64, f64, bool)> {
    let i = s.world.get::<Ident>(e)?;
    let k = s.world.get::<Kin>(e)?;
    let a = s.world.get::<Act>(e)?;
    let sig = s.world.get::<SigState>(e)?;
    let h = s.world.get::<Health>(e)?;
    let p = &i.platform;
    let v = &p.signatures.visual.contrast;
    let contrast = match a.posture {
        Posture::Moving => v.moving,
        Posture::Halted => v.halted,
        Posture::Settled => v.halted.min(v.camouflaged.max(v.hull_down)),
        Posture::HullDown => v.hull_down,
    };
    let now = s.ctx.now;
    let dk = &p.signatures.ir.delta_k;
    let fired = sig.last_fired.map(|t| (now - t).secs() < 120.0).unwrap_or(false);
    let moved = sig.last_moved.map(|t| (now - t).secs() < 600.0).unwrap_or(false);
    let dt = if fired { dk.fired } else if moved { dk.moved } else if sig.engine_on { dk.engine_on } else { dk.cold };
    let micro = s.ctx.ground.micro(k.pos.e, k.pos.n);
    // From above, posture hides less than from the ground.
    let w = match a.posture {
        Posture::Moving => 0.2,
        Posture::Halted => 0.4,
        _ => 0.7,
    };
    let hide = if c4.drones.hiding.get(&i.id).map(|t| *t > now).unwrap_or(false) { c4.p.hide_concealment } else { 0.0 };
    let conc = (micro.concealment * w + hide).clamp(0.0, 0.95);
    let crit = p.signatures.visual.critical_dim_m * (h.fit.max(1) as f64).powf(0.15);
    Some((crit, contrast, dt, conc, a.posture == Posture::Moving))
}

/// Probability per second that a drone's cameras detect a target, and the sensor.
fn camera_p(s: &Sim, c4: &C4, f: &Flight, e: bevy_ecs::prelude::Entity, tpos: Enu) -> Option<(f64, SensorKind, f64)> {
    let ground = ground_at(s, f.pos);
    let slant = (tpos.sub(Enu::new(f.pos.e, f.pos.n, ground + f.agl))).norm();
    let max_r = f.platform.sensors.iter().filter(|x| x.kind.is_imaging_eo()).map(|x| x.range_max_m).fold(0.0, f64::max);
    if slant > max_r {
        return None;
    }
    let los = s.ctx.ground.los(Enu::new(f.pos.e, f.pos.n, ground), f.agl, tpos, 1.5);
    if los.p_visible <= 0.0 {
        return None;
    }
    let (crit, contrast, dt, conc, _) = signature(s, c4, e)?;
    let w = s.ctx.weather.sample(f.pos.e, f.pos.n, s.ctx.now);
    let p = &s.ctx.p;
    let mut best: Option<(f64, SensorKind)> = None;
    for sen in f.platform.sensors.iter().filter(|x| x.kind.is_imaging_eo() && slant <= x.range_max_m) {
        let o = sen.optics.as_ref();
        let res = o.map(|o| o.resolution_cyc_per_mrad).unwrap_or(1.0);
        let (c, sk) = if sen.kind == CatSensor::Thermal {
            (Contrast::Thermal { delta_t_k: dt * (1.0 - 0.7 * conc), transmission: transmission(OpticalBand::Lwir, slant, &w, &p.atmos) }, SensorKind::Thermal)
        } else {
            (Contrast::Visual { inherent: contrast * (1.0 - conc), transmission: transmission(OpticalBand::Visible, slant, &w, &p.atmos) }, SensorKind::Visual)
        };
        let mut ap = p.acquire.clone();
        if let Some(o) = o {
            ap.n50 = [o.n50.detect, o.n50.recognise, o.n50.identify];
        }
        let r = acquire(&AcquireInput { critical_dim_m: crit, range_m: slant, cycles_per_mrad: res, metric: Metric::Johnson, atmos_mtf: 1.0, contrast: c, stage: Stage::Detect, search: Search { fov_deg: p.search_fov_deg, cued: false, observer_tau_mult: 1.0 } }, &ap);
        if best.map(|b| r.rate_per_s > b.0).unwrap_or(true) {
            best = Some((r.rate_per_s, sk));
        }
    }
    let (rate, sk) = best?;
    Some((los.p_visible * hazard_probability(rate, 1.0), sk, slant))
}

fn hypothesis(p: &Platform) -> Hypothesis {
    let c = p.class.to_lowercase();
    let r = p.role.to_lowercase();
    if c.contains("mbt") {
        Hypothesis::Mbt
    } else if c.contains("ifv") {
        Hypothesis::Ifv
    } else if c.contains("apc") {
        Hypothesis::Apc
    } else if p.mover.kind == bgws_catalogue::MoverKind::Foot {
        Hypothesis::Infantry
    } else if r.contains("artillery") || r.contains("mortar") {
        Hypothesis::Artillery
    } else {
        Hypothesis::Truck
    }
}

/// Cameras search: ISR drones report, strike drones find their targets.
fn cameras(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let sp = c4.drones.sp.clone().expect("strike params");
    let fids: Vec<u64> = c4.drones.flights.keys().copied().collect();
    // Enemy land entities, once.
    let land: Vec<(EntityId, bevy_ecs::prelude::Entity, Side, Enu)> = s
        .ctx
        .order
        .iter()
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            (i.domain == Domain::Land && crate::systems::alive(s, e) && !i.dummy).then(|| (i.id, e, i.side, s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default()))
        })
        .collect();
    for fid in fids {
        let f = c4.drones.flights[&fid].clone();
        if f.done || (f.link.lost && !f.link.autonomous) || f.task == Task::Terminal || f.task == Task::Return {
            continue;
        }
        let blind = f.link.autonomous && f.spec.on_link_loss == LinkLossBehaviour::ContinueToAimPoint;
        let search_at = match &f.task {
            Task::Strike { aim, .. } => Some(*aim),
            Task::LoiterSearch { at, .. } => Some(*at),
            _ => None,
        };
        let cam_r = f.platform.sensors.iter().filter(|x| x.kind.is_imaging_eo()).map(|x| x.range_max_m).fold(0.0, f64::max);
        if let Some(aim) = search_at {
            if f.pos.sub(aim).horizontal() > cam_r.max(600.0) {
                continue;
            }
            if f.search_from.is_none() {
                c4.drones.flights.get_mut(&fid).expect("f").search_from = Some(now);
            }
        }
        let mut found: Vec<(EntityId, f64, Enu, SensorKind, f64)> = Vec::new();
        for (tid, e, side, tpos) in &land {
            if *side == f.side || *side == Side::Neutral || tpos.sub(f.pos).horizontal() > cam_r {
                continue;
            }
            if let Some(aim) = search_at {
                if tpos.sub(aim).horizontal() > 600.0 {
                    continue;
                }
            }
            let Some((p, sk, slant)) = camera_p(s, c4, &f, *e, *tpos) else { continue };
            let mut rng = s.ctx.rng("drones.camera", fid, tid.0 as u64);
            if rng.gen::<f64>() < p {
                let val = s.world.get::<Ident>(*e).map(|i| value_of(&i.platform, i.hq)).unwrap_or(1.0);
                found.push((*tid, val, *tpos, sk, slant));
            }
        }
        for (tid, _, _, _, _) in &found {
            c4.drones.first_seen.entry((f.side, *tid)).or_insert(now);
        }
        match &f.task {
            Task::Orbit { .. } => {
                for (tid, val, tpos, sk, slant) in found {
                    report_sighting(s, c4, &f, tid, val, tpos, sk, slant);
                }
            }
            Task::Strike { .. } | Task::LoiterSearch { .. } => {
                let pick = found.iter().filter(|x| f.kind != DroneType::LoiteringMunition || ent(s, x.0).and_then(|e| s.world.get::<Ident>(e)).map(|i| i.platform.mover.kind != bgws_catalogue::MoverKind::Foot).unwrap_or(false)).max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).copied();
                if let (Some((tid, _, tpos, _, _)), false) = (pick, blind) {
                    let fm = c4.drones.flights.get_mut(&fid).expect("f");
                    fm.target = Some(tid);
                    fm.found_at = Some(now);
                    fm.task = Task::Terminal;
                    let text = format!("{}'s {} finds {} and dives", crate::systems::name(s, f.team), f.platform.name, crate::systems::name(s, tid));
                    s.ctx.log.push(now, "drones.found").side(f.side).entity(f.team).target(tid).text(text).data(serde_json::json!({ "flight": fid, "searchS": f.search_from.map(|t| (now - t).secs()), "e": tpos.e, "n": tpos.n }));
                } else if blind {
                    // Flying on to the last aim point: it dives on the aim point.
                    if let Some(aim) = search_at {
                        if f.pos.sub(aim).horizontal() < 50.0 {
                            resolve(s, c4, fid, None, &sp);
                        }
                    }
                } else if f.search_from.map(|t| (now - t).secs() > sp.search_s).unwrap_or(false) && matches!(f.task, Task::Strike { .. }) {
                    end_flight(s, c4, fid, StrikeStage::NotFound, "found nothing to strike", false);
                } else if let Task::LoiterSearch { until, .. } = f.task {
                    if now > until {
                        c4.drones.flights.get_mut(&fid).expect("f").task = Task::Return;
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn report_sighting(s: &mut Sim, c4: &mut C4, f: &Flight, tid: EntityId, val: f64, tpos: Enu, sk: SensorKind, slant: f64) {
    let now = s.ctx.now;
    let sigma = c4.p.find_sigma_m * (1.0 + slant / 2000.0);
    let mut rng = s.ctx.rng("drones.fix", f.id, tid.0 as u64);
    let noise = Enu::new(rng.gen::<f64>() * 2.0 - 1.0, rng.gen::<f64>() * 2.0 - 1.0, 0.0).scale(sigma * 1.2);
    let (hyp, moving) = ent(s, tid).and_then(|e| Some((hypothesis(&s.world.get::<Ident>(e)?.platform), s.world.get::<Act>(e)?.posture == Posture::Moving))).unwrap_or((Hypothesis::Truck, false));
    let det = Detection {
        time: now,
        observer: f.team,
        sensor: sk,
        domain: bgws_know::Domain::Land,
        motion: if moving { MotionHint::Moving } else { MotionHint::Static },
        measurement: Measurement::Position { pos: tpos.add(noise), cov: Matrix3::from_diagonal_element(sigma * sigma) },
        class: ClassEvidence { level: if slant < 1200.0 { IdLevel::Recognise } else { IdLevel::Detect }, reported: (slant < 1200.0).then_some(hyp), resolves_decoys: slant < 600.0 },
        truth: TruthTag::of(tid),
    };
    super::comms::queue_detection(s, c4, f.team, f.side, det);
    if let Some(t) = c4.drones.teams.get_mut(&f.team) {
        t.targets.insert(tid.0 as u64, TargetInfo { at: tpos.add(noise), seen: now, value: val });
    }
    let key = (f.side, tid);
    let asked = c4.drones.found_asked.get(&key).copied();
    if asked.map(|t| (now - t).secs() >= c4.p.found_ask_s).unwrap_or(true) {
        c4.drones.found_asked.insert(key, now);
        if asked.is_none() {
            let text = format!("{}'s drone sees {} at {:.1} km", crate::systems::name(s, f.team), crate::systems::name(s, tid), slant / 1000.0);
            s.ctx.log.push(now, "drones.sighting").side(f.side).entity(f.team).target(tid).text(text).data(serde_json::json!({ "flight": f.id, "slantM": slant }));
        }
        push_trigger(s, c4, f.team, "targetFound", Some(tid), QCtx { flight: Some(f.id), target: Some(tid), at: Some(tpos), ..Default::default() });
    }
}

/// Dives that have reached their target.
fn terminal(s: &mut Sim, c4: &mut C4) {
    let sp = c4.drones.sp.clone().expect("strike params");
    let fids: Vec<u64> = c4.drones.flights.iter().filter(|(_, f)| !f.done && f.task == Task::Terminal).map(|(k, _)| *k).collect();
    for fid in fids {
        let f = c4.drones.flights[&fid].clone();
        let Some(t) = f.target else { continue };
        if !alive_id(s, t) {
            // Gone already: search again where it was.
            let at = pos_of(s, t).unwrap_or(f.pos);
            let fm = c4.drones.flights.get_mut(&fid).expect("f");
            fm.target = None;
            fm.task = Task::Strike { aim: at, agl: 60.0 };
            continue;
        }
        let tp = pos_of(s, t).unwrap_or(f.pos);
        if tp.sub(f.pos).horizontal() <= c4.p.strike_reach_m.max(f.spec.speed_mps * 0.6) {
            resolve(s, c4, fid, Some(t), &sp);
        }
    }
}

/// The terminal attack and its effect.
fn resolve(s: &mut Sim, c4: &mut C4, fid: u64, target: Option<EntityId>, sp: &StrikeParams) {
    let now = s.ctx.now;
    let f = c4.drones.flights[&fid].clone();
    let te = target.and_then(|t| ent(s, t));
    let (class, speed, conc, cage, in_cover) = match te {
        Some(e) => {
            let k = s.world.get::<Kin>(e).map(|k| (k.pos, k.vel.horizontal())).unwrap_or_default();
            let cover = s.ctx.ground.micro(k.0.e, k.0.n).cover > 0.3 || s.world.get::<Act>(e).map(|a| a.posture == Posture::HullDown).unwrap_or(false);
            let i = s.world.get::<Ident>(e).expect("ident");
            let conc = signature(s, c4, e).map(|x| x.3).unwrap_or(0.0);
            let cage = c4.roles.get(&i.id).map(|r| r.cage).unwrap_or(false);
            (target_class(&i.platform, cover), k.1, conc, cage, cover)
        }
        None => (TargetClass::InfantryOpen, 0.0, 0.0, false, false),
    };
    let mods = passive_modifiers(if cage { &[PassiveMeasure::Cage] } else { &[] }, &s.ctx.figures);
    let op = c4.drones.teams.get(&f.team).map(|t| t.operator).unwrap_or(Operator { skill: 0.5, fatigue: 0.0 });
    let inputs = StrikeInputs {
        operator: op,
        target: class,
        target_speed_mps: speed,
        concealment: conc,
        wind_mps: wind_mps(s),
        link_quality: if f.spec.jammable() { f.link.quality } else { 1.0 },
        distance_m: pos_of(s, f.team).map(|p| p.sub(f.pos).horizontal()).unwrap_or(0.0),
        approach_s: 0.0,
        counter_drone_rate: 0.0,
        hit_modifier: mods.hit_factor,
        kill_modifier: mods.kill_factor,
    };
    let p_hit = if f.link.autonomous {
        let calm = StrikeInputs { link_quality: 1.0, operator: Operator { skill: 1.0, fatigue: 0.0 }, ..inputs.clone() };
        terminal_hit_probability(sp, &calm) * link_loss_outcome(&f.spec, speed, sp).hit_factor
    } else {
        terminal_hit_probability(sp, &inputs)
    };
    let p_kill = (sp.kill_given_hit.get(&class).copied().unwrap_or(0.0) * mods.kill_factor).clamp(0.0, 1.0);
    let mut rng = s.ctx.rng("drones.strike", fid, target.map(|t| t.0 as u64).unwrap_or(u64::MAX));
    let hit = target.is_some() && rng.gen::<f64>() < p_hit;
    let kill = hit && rng.gen::<f64>() < p_kill;
    let stage = if kill {
        StrikeStage::Destroyed
    } else if hit {
        StrikeStage::HitNoKill
    } else if f.link.autonomous && f.link.lost {
        StrikeStage::LinkLost
    } else {
        StrikeStage::Missed
    };
    c4.drones.flights.get_mut(&fid).expect("f").done = true;
    let range = pos_of(s, f.team).map(|p| p.sub(f.pos).horizontal()).unwrap_or(0.0);
    let first = target.and_then(|t| c4.drones.first_seen.get(&(f.side, t)).copied());
    c4.drones.records.push(StrikeRecord { flight: fid, team: f.team, side: f.side, kind: f.kind, target, stage, launched: f.launched, at: now, range_m: range, p_hit, p_kill, first_seen: first, cued: f.cue });
    let mut destroyed = false;
    if let (Some(t), Some(e)) = (target, te) {
        destroyed = effect(s, c4, f.team, t, e, &class, stage, &mut rng);
    }
    let text = format!(
        "{}'s {} strikes {}: {}",
        crate::systems::name(s, f.team),
        f.platform.name,
        target.map(|t| crate::systems::name(s, t)).unwrap_or_else(|| "the aim point".into()),
        match stage {
            StrikeStage::Destroyed => "knocked out",
            StrikeStage::HitNoKill => "hit, not destroyed",
            _ => "missed",
        }
    );
    s.ctx.log.push(now, "drones.strike").side(f.side).entity(f.team).target(target.unwrap_or(f.team)).text(text).data(serde_json::json!({
        "flight": fid, "kind": f.kind, "stage": stage, "pHit": p_hit, "pKill": p_kill, "class": class, "link": f.link.quality, "autonomous": f.link.autonomous,
        "skill": op.effective_skill(sp), "cage": cage, "inCover": in_cover, "rangeM": range, "flightS": (now - f.launched).secs(), "firstSeenS": first.map(|t| t.secs()),
    }));
    if destroyed {
        if let Some(t) = target {
            let text = format!("{} is destroyed", crate::systems::name(s, t));
            s.ctx.log.push(now, "unit.destroyed").side(f.side.enemy()).entity(t).text(text);
        }
    }
}

/// A hit's effect on the target; true if it is now destroyed.
#[allow(clippy::too_many_arguments)]
fn effect(s: &mut Sim, c4: &mut C4, team: EntityId, t: EntityId, e: bevy_ecs::prelude::Entity, class: &TargetClass, stage: StrikeStage, rng: &mut impl Rng) -> bool {
    let now = s.ctx.now;
    let mut destroyed = false;
    let personnel = matches!(class, TargetClass::InfantryOpen | TargetClass::InfantryCover);
    match stage {
        StrikeStage::Destroyed if personnel => {
            // A casualty; a section's worth of casualties is a section lost.
            let (people, start) = s.world.get::<Ident>(e).map(|i| (i.platform.people().max(1) as f64, s.world.get::<Health>(e).map(|h| h.start).unwrap_or(1) as f64)).unwrap_or((1.0, 1.0));
            let per = (people / start).max(1.0);
            let acc = c4.drones.casualties.entry(t).or_insert(0.0);
            *acc += 1.0;
            let whole = *acc >= per;
            if whole {
                *acc -= per;
            }
            if let Some(mut c) = s.world.get_mut::<Crew>(e) {
                c.new_casualties += 1.0 / people;
            }
            if whole {
                if let Some(mut h) = s.world.get_mut::<Health>(e) {
                    if h.fit > 0 {
                        h.fit -= 1;
                        h.losses.push(now);
                    }
                    if h.fit == 0 {
                        h.destroyed = true;
                        destroyed = true;
                    }
                }
            }
        }
        StrikeStage::Destroyed => {
            if let Some(mut h) = s.world.get_mut::<Health>(e) {
                if h.fit > 0 {
                    h.fit -= 1;
                    h.losses.push(now);
                }
                if h.fit == 0 {
                    h.destroyed = true;
                    destroyed = true;
                }
                let frac = 1.0 / h.start as f64;
                if let Some(mut c) = s.world.get_mut::<Crew>(e) {
                    c.new_casualties += frac;
                }
            }
        }
        StrikeStage::HitNoKill if !personnel => {
            let mobility = rng.gen::<f64>() < 0.5;
            if let Some(mut h) = s.world.get_mut::<Health>(e) {
                if mobility && h.mobility_kills < h.fit {
                    h.mobility_kills += 1;
                } else if h.firepower_kills < h.fit {
                    h.firepower_kills += 1;
                }
            }
        }
        _ => {}
    }
    if let Some(mut c) = s.world.get_mut::<Crew>(e) {
        c.blasts_per_min += if matches!(stage, StrikeStage::Missed) { 3.0 } else { 6.0 };
        c.last_incoming = Some((now, team));
    }
    destroyed
}

/// Ground units detect and engage the drones near them.
fn counter_drone(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let f = s.ctx.figures.clone();
    let vis = {
        let w = s.ctx.weather.sample(0.0, 0.0, now);
        (w.visibility_m / 10_000.0).clamp(0.05, 1.0)
    };
    let units: Vec<(EntityId, bevy_ecs::prelude::Entity, Side, Enu)> = s
        .ctx
        .order
        .iter()
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            (i.domain == Domain::Land && crate::systems::alive(s, e)).then(|| (i.id, e, i.side, s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default()))
        })
        .collect();
    let fids: Vec<u64> = c4.drones.flights.keys().copied().collect();
    let hold = SimDuration::from_secs(c4.p.cuas_track_s);
    for fid in fids {
        for (uid, e, side, upos) in &units {
            let Some(fl) = c4.drones.flights.get(&fid) else { break };
            if fl.done {
                break;
            }
            if *side == fl.side || *side == Side::Neutral {
                continue;
            }
            let horiz = fl.pos.sub(*upos).horizontal();
            if horiz > c4.p.cuas_range_m {
                continue;
            }
            let range = (horiz * horiz + (fl.pos.u - upos.u).powi(2)).sqrt();
            let Some(role) = c4.roles.get(uid).cloned() else { continue };
            let key = (*uid, fid);
            let tracked = c4.drones.cuas_tracks.get(&key).map(|t| now - *t <= hold).unwrap_or(false);
            if !tracked {
                let rcs = fl.platform.signatures.rcs.values().next().map(|r| r.front.max(r.side)).unwrap_or(0.01).max(0.001);
                let acoustic = if fl.platform.mover.kind == bgws_catalogue::MoverKind::FixedWing { c4.p.fixed_wing_acoustic } else { c4.p.isr_acoustic };
                let input = DetectionInput { range_m: range, rf_emitting: fl.spec.jammable() && !fl.link.lost, rcs_m2: rcs, acoustic_level: acoustic, visibility: vis };
                let det = detect(&role.cuas_sensors, &input, 1.0, &f);
                if s.ctx.rng("cuas.detect", uid.0 as u64, fid).gen::<f64>() >= det.p_detect {
                    continue;
                }
            }
            c4.drones.cuas_tracks.insert(key, now);
            // Engage with what it has, cheapest first.
            let eng = Engagement { drone: fl.spec.clone(), range_m: range, exposure_s: 1.0, target_speed_mps: fl.spec.speed_mps };
            let fixed_wing = fl.platform.mover.kind == bgws_catalogue::MoverKind::FixedWing;
            let can_fire = s.world.get::<Health>(*e).map(|h| h.can_fire()).unwrap_or(false) && s.world.get::<Act>(*e).map(|a| a.roe != RoeS::Never && a.activity != Activity::Quiet).unwrap_or(true);
            if !can_fire {
                continue;
            }
            let mut tries: Vec<(&'static str, f64, Option<usize>)> = Vec::new();
            if let Some(m) = role.ad_gun {
                tries.push(("AD gun", defeat(DefeatMeans::Gun, &eng, &f).p_defeat, Some(m)));
            } else if role.small_arms {
                tries.push(("small arms", defeat(DefeatMeans::Gun, &eng, &f).p_defeat * c4.p.small_arms_factor, None));
            }
            if let Some(m) = role.interceptor {
                if !c4.drones.interceptor_fired.contains(&key) && range <= f.get("drones.interceptor.range_m") {
                    let speed = f.get("drones.interceptor.speed_mps");
                    tries.push(("interceptor drone", defeat(DefeatMeans::InterceptorDrone { speed_mps: speed }, &eng, &f).p_defeat, Some(m)));
                }
            }
            if let Some(m) = role.sam {
                if fixed_wing {
                    let pk = s.world.get::<Arms>(*e).and_then(|a| a.mounts.iter().find(|x| x.index == m).map(|x| x.munition.clone())).and_then(|mu| s.ctx.catalogue.munition(&mu).map(|x| (x.pk, x.max_range_m))).unwrap_or((0.0, 0.0));
                    if range <= pk.1 {
                        tries.push(("SAM", pk.0, Some(m)));
                    }
                }
            }
            for (what, p, mount) in tries {
                if p <= 0.0 {
                    continue;
                }
                if let Some(mi) = mount {
                    let mut ok = false;
                    if let Some(mut a) = s.world.get_mut::<Arms>(*e) {
                        if let Some(ms) = a.mounts.iter_mut().find(|x| x.index == mi && x.rounds > 0 && x.ready_at <= now) {
                            let use_n = if what == "AD gun" { 10.min(ms.rounds) } else { 1 };
                            ms.rounds -= use_n;
                            if what != "AD gun" {
                                ms.ready_at = now + SimDuration::from_secs(10.0);
                            }
                            ok = true;
                        }
                    }
                    if !ok {
                        continue;
                    }
                    if what == "interceptor drone" {
                        c4.drones.interceptor_fired.insert(key);
                    }
                }
                let mut rng = s.ctx.rng("cuas.defeat", uid.0 as u64, fid ^ (what.len() as u64) << 48);
                if rng.gen::<f64>() < p {
                    let text = format!("{} brings down {}'s {} ({what}, {:.0} m)", crate::systems::name(s, *uid), crate::systems::name(s, fl.team), fl.platform.name, range);
                    s.ctx.log.push(now, "drones.shot_down").side(*side).entity(*uid).target(fl.team).text(text).data(serde_json::json!({ "flight": fid, "means": what, "p": p, "rangeM": range }));
                    end_flight(s, c4, fid, StrikeStage::ShotDown, what, false);
                    break;
                }
            }
        }
    }
    c4.drones.cuas_tracks.retain(|_, t| now - *t <= hold);
}

/// Drones overhead: units under a camera feel it; units that notice one are asked.
fn overhead(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let mut observed = BTreeSet::new();
    let units: Vec<(EntityId, Side, Enu)> = s.ctx.order.iter().filter_map(|&e| {
        let i = s.world.get::<Ident>(e)?;
        (i.domain == Domain::Land && crate::systems::alive(s, e)).then(|| (i.id, i.side, s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default()))
    }).collect();
    let flights: Vec<(u64, Side, Enu, f64)> = c4.drones.flights.values().filter(|f| !f.done).map(|f| (f.id, f.side, f.pos, f.agl)).collect();
    for (uid, side, p) in units {
        let mut over: Option<u64> = None;
        for (fid, fs, fp, agl) in &flights {
            if *fs == side {
                continue;
            }
            let d = fp.sub(p).horizontal();
            if d <= c4.p.observed_m && *agl < 1000.0 {
                observed.insert(uid);
            }
            if d <= c4.p.overhead_m && c4.drones.cuas_tracks.contains_key(&(uid, *fid)) {
                over = Some(*fid);
            }
        }
        if let Some(fid) = over {
            if c4.drones.overhead_asked.get(&uid).map(|t| (now - *t).secs() < c4.p.overhead_ask_s).unwrap_or(false) {
                continue;
            }
            c4.drones.overhead_asked.insert(uid, now);
            let fp = c4.drones.flights.get(&fid).map(|f| f.pos);
            push_trigger(s, c4, uid, "droneOverhead", None, QCtx { flight: Some(fid), at: fp, ..Default::default() });
        }
    }
    c4.drones.observed = observed;
}

/// Is an enemy drone watching this unit (drone-observed stress, §8.10)?
pub fn drone_observed(s: &Sim, id: EntityId) -> bool {
    s.world.get_resource::<C4>().map(|c| c.drones.observed.contains(&id)).unwrap_or(false)
}

/// Each minute: operators tire while flying and rest otherwise.
pub fn minute(s: &mut Sim, dt: f64) {
    let Some(mut c4) = super::take(s) else { return };
    if let Some(sp) = c4.drones.sp.clone() {
        let flying: BTreeSet<EntityId> = c4.drones.flights.values().map(|f| f.team).collect();
        for (id, t) in c4.drones.teams.iter_mut() {
            if flying.contains(id) {
                t.operator.work(dt / 3600.0, &sp);
            } else {
                t.operator.rest(dt / 3600.0, &sp);
            }
        }
    }
    super::put(s, c4);
}

/// The strike sorties so far (P9, kill-zone depth, detect-to-strike).
pub fn records(s: &Sim) -> Vec<StrikeRecord> {
    s.world.get_resource::<C4>().map(|c| c.drones.records.clone()).unwrap_or_default()
}

/// Drones in the air now: (flight id, team, side, kind, position, height above ground, heading).
pub fn airborne(s: &Sim) -> Vec<(u64, EntityId, Side, DroneType, Enu, f64, f64)> {
    s.world
        .get_resource::<C4>()
        .map(|c| c.drones.flights.values().filter(|f| !f.done).map(|f| (f.id, f.team, f.side, f.kind, f.pos, f.agl, f.heading)).collect())
        .unwrap_or_default()
}

/// Sorties launched.
pub fn sorties(s: &Sim) -> u32 {
    s.world.get_resource::<C4>().map(|c| c.drones.sorties).unwrap_or(0)
}

/// A team moves off (`drones.relocate`, or after its flight).
pub fn relocate(s: &mut Sim, id: EntityId) {
    let Some(e) = ent(s, id) else { return };
    if !crate::systems::alive(s, e) {
        return;
    }
    let dist = s.world.get_resource::<C4>().map(|c| c.p.relocate_m).unwrap_or(1500.0);
    let to = super::ew::point_away(s, id, dist);
    let route = crate::decide::route_for(s, e, to);
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Move { route, mode: MoveModeS::March, then: Some(Box::new(Activity::Hold)) };
    }
    if let Some(mut c4) = super::take(s) {
        c4.on_air.insert(id, 0.0);
        super::put(s, c4);
    }
}

fn cover_near(s: &Sim, p: Enu, r: f64) -> Enu {
    let mut best = (s.ctx.ground.micro(p.e, p.n).cover, p);
    for ring in 1..=3 {
        for k in 0..12 {
            let b = k as f64 * std::f64::consts::TAU / 12.0;
            let d = r * ring as f64 / 3.0;
            let q = p.add(Enu::new(b.sin() * d, b.cos() * d, 0.0));
            if !s.ctx.ground.contains(q) {
                continue;
            }
            let c = s.ctx.ground.micro(q.e, q.n).cover;
            if c > best.0 + 0.05 {
                best = (c, q);
            }
        }
    }
    best.1
}

// ── Actions (drones.*) ──────────────────────────────────────────────────

pub fn apply_action(s: &mut Sim, e: bevy_ecs::prelude::Entity, id: EntityId, action: &str, _params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    let Some(mut c4) = super::take(s) else { return false };
    let now = s.ctx.now;
    let q = c4.qctx.get(&id).cloned().unwrap_or_default();
    let side = side_of(s, id).unwrap_or(Side::Neutral);
    let me = pos_of(s, id).unwrap_or_default();
    let mut move_to: Option<(Enu, MoveModeS)> = None;
    let flight = q.flight.filter(|f| c4.drones.flights.contains_key(f));
    let handled = match action {
        "reportAndWatch" => {
            if let (Some(fid), Some(at)) = (flight, q.at) {
                if let Some(f) = c4.drones.flights.get_mut(&fid) {
                    if let Task::Orbit { at: a, .. } = &mut f.task {
                        *a = at;
                    }
                }
            }
            true
        }
        "callFire" => {
            if let Some(at) = q.at {
                if let Some(g) = super::ew::nearest_guns(s, &c4, side, at) {
                    push_trigger(s, &mut c4, g, "fireRequestReceived", q.target.or(about), QCtx { target: q.target.or(about), at: Some(at), ..Default::default() });
                    let text = format!("{} calls fire on a drone sighting", crate::systems::name(s, id));
                    s.ctx.log.push(now, "drones.call_fire").side(side).entity(id).target(g).text(text);
                }
            }
            true
        }
        "handOff" => {
            if let Some(at) = q.at {
                let team = c4.drones.teams.iter().filter(|(t, x)| x.side == side && strike_type(x.kind) && **t != id && alive_id(s, **t) && x.stores.drones.values().sum::<u32>() > 0).filter_map(|(t, x)| Some((*t, pos_of(s, *t)?.sub(at).horizontal(), x.spec.range_m))).filter(|x| x.1 <= x.2 * 0.9).min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0))).map(|x| x.0);
                if let Some(team) = team {
                    let key = q.target.map(|t| t.0 as u64).unwrap_or(u64::MAX);
                    let value = q.target.and_then(|t| ent(s, t)).and_then(|x| s.world.get::<Ident>(x)).map(|i| value_of(&i.platform, i.hq)).unwrap_or(2.0);
                    let t = c4.drones.teams.get_mut(&team).expect("team");
                    t.targets.insert(key, TargetInfo { at, seen: now, value });
                    t.handoff = Some(key);
                    t.last_launch = SimTime::from_secs(-1e6);
                    let text = format!("{} hands its target to {}", crate::systems::name(s, id), crate::systems::name(s, team));
                    s.ctx.log.push(now, "drones.handoff").side(side).entity(id).target(team).text(text);
                }
            }
            true
        }
        "strike" | "ditchOnTarget" | "onePass" => {
            if let Some(fid) = flight {
                let at = q.at.or_else(|| c4.drones.flights.get(&fid).map(|f| f.pos)).unwrap_or(me);
                if let Some(f) = c4.drones.flights.get_mut(&fid) {
                    if f.spec.carries_warhead {
                        f.task = Task::Strike { aim: at, agl: 60.0 };
                        f.search_from = None;
                    }
                }
            }
            true
        }
        "pushOn" | "stay" | "holdForBetter" => true,
        "climbForLink" => {
            if let Some(f) = flight.and_then(|fid| c4.drones.flights.get_mut(&fid)) {
                match &mut f.task {
                    Task::Orbit { agl, .. } | Task::Strike { agl, .. } => *agl = (*agl + 200.0).min(800.0),
                    _ => {}
                }
                f.link.asked = false;
            }
            true
        }
        "returnHome" => {
            if let Some(f) = flight.and_then(|fid| c4.drones.flights.get_mut(&fid)) {
                f.task = Task::Return;
            }
            true
        }
        "goAutonomous" => {
            if let Some(f) = flight.and_then(|fid| c4.drones.flights.get_mut(&fid)) {
                if matches!(f.spec.on_link_loss, LinkLossBehaviour::AiTerminal | LinkLossBehaviour::ContinueToAimPoint) {
                    f.link.autonomous = true;
                }
            }
            true
        }
        "switchToFibre" => {
            let fibre = super::platform(s, "generic/fpvFibre");
            if let (Some(t), Some(fp)) = (c4.drones.teams.get_mut(&id), fibre) {
                if t.fibre_stock > 0 {
                    t.kind = DroneType::FpvFibre;
                    t.spec = DroneSpec::from_figures(DroneType::FpvFibre, &s.ctx.figures);
                    t.platform = fp;
                    t.stores.drones = [(DroneType::FpvFibre, t.fibre_stock)].into_iter().collect();
                    t.fibre_stock = 0;
                    t.out_logged = false;
                    let text = format!("{} switches to fibre-optic drones", crate::systems::name(s, id));
                    s.ctx.log.push(now, "drones.fibre").side(side).entity(id).text(text);
                }
            }
            true
        }
        "relocate" | "scatter" | "moveToCover" => {
            c4.on_air.insert(id, 0.0);
            let to = match action {
                "relocate" => super::ew::point_away(s, id, c4.p.relocate_m),
                "scatter" => {
                    let b = s.ctx.rng("drones.scatter", id.0 as u64, 0).gen::<f64>() * std::f64::consts::TAU;
                    me.add(Enu::new(b.sin() * 200.0, b.cos() * 200.0, 0.0))
                }
                _ => cover_near(s, me, 400.0),
            };
            move_to = Some((to, if action == "relocate" { MoveModeS::March } else { MoveModeS::Assault }));
            true
        }
        "callFireOnJammer" => {
            let fix = c4.ew.fixes.iter().filter(|(k, v)| k.0 == side && v.what == "jammer").min_by(|a, b| a.1.semi_major_m.total_cmp(&b.1.semi_major_m).then(a.0.cmp(b.0))).map(|(k, v)| (k.1, v.at));
            if let Some((em, at)) = fix {
                if let Some(g) = super::ew::nearest_guns(s, &c4, side, at) {
                    push_trigger(s, &mut c4, g, "fireRequestReceived", Some(em), QCtx { target: Some(em), at: Some(at), ..Default::default() });
                }
            }
            true
        }
        "wait" => {
            c4.emcon.insert(id, Emcon::Silent);
            s.ctx.queue.schedule(now + SimDuration::from_secs(c4.p.wait_s), Priority::Process, Scheduled::Timer { system: "c4.unsilence".into(), key: id.0 as u64 });
            true
        }
        "finishThenRelocate" => {
            s.ctx.queue.schedule(now + SimDuration::from_secs(600.0), Priority::Process, Scheduled::Timer { system: "c4.relocate".into(), key: id.0 as u64 });
            true
        }
        "freezeAndHide" => {
            c4.drones.hiding.insert(id, now + SimDuration::from_secs(c4.p.hide_s));
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                if a.activity.moving() {
                    a.activity = Activity::Hold;
                }
            }
            true
        }
        "shootItDown" => {
            if let Some(f) = flight.and_then(|fid| c4.drones.flights.get(&fid)).cloned() {
                let range = f.pos.sub(me).norm();
                let eng = Engagement { drone: f.spec.clone(), range_m: range, exposure_s: 5.0, target_speed_mps: f.spec.speed_mps };
                let role = c4.roles.get(&id).cloned().unwrap_or_default();
                let p = defeat(DefeatMeans::Gun, &eng, &s.ctx.figures).p_defeat * if role.ad_gun.is_some() { 1.0 } else { c4.p.small_arms_factor };
                if s.ctx.rng("drones.shoot", id.0 as u64, f.id).gen::<f64>() < p {
                    let text = format!("{} shoots down {}'s {}", crate::systems::name(s, id), crate::systems::name(s, f.team), f.platform.name);
                    s.ctx.log.push(now, "drones.shot_down").side(side).entity(id).target(f.team).text(text).data(serde_json::json!({ "flight": f.id, "means": "small arms", "p": p }));
                    end_flight(s, &mut c4, f.id, StrikeStage::ShotDown, "shot down", false);
                }
            }
            true
        }
        "callCounterDrone" => {
            let near = c4.roles.iter().filter(|(t, r)| (r.interceptor.is_some() || r.ad_gun.is_some()) && **t != id && side_of(s, **t) == Some(side) && alive_id(s, **t)).filter_map(|(t, _)| Some((*t, pos_of(s, *t)?.sub(me).horizontal()))).filter(|x| x.1 <= s.ctx.figures.get("drones.interceptor.range_m")).min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0))).map(|x| x.0);
            if let (Some(team), Some(fid)) = (near, flight) {
                if let Some(at) = c4.drones.flights.get(&fid).map(|f| f.pos) {
                    c4.drones.cuas_tracks.insert((team, fid), now);
                    push_trigger(s, &mut c4, team, "interceptTasked", None, QCtx { flight: Some(fid), at: Some(at), ..Default::default() });
                }
            }
            true
        }
        "engageTarget" => {
            let fid = flight.or_else(|| c4.drones.cuas_tracks.keys().filter(|(u, _)| *u == id).map(|(_, f)| *f).find(|f| c4.drones.flights.contains_key(f)));
            if let Some(fid) = fid {
                c4.drones.cuas_tracks.insert((id, fid), now);
                c4.drones.interceptor_fired.remove(&(id, fid));
            }
            true
        }
        _ => false,
    };
    super::put(s, c4);
    if let Some((to, mode)) = move_to {
        let route = crate::decide::route_for(s, e, to);
        if let Some(mut a) = s.world.get_mut::<Act>(e) {
            a.activity = Activity::Move { route, mode, then: Some(Box::new(Activity::Hold)) };
        }
    }
    handled
}
