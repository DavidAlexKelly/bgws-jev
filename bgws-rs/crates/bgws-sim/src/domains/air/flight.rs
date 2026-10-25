//! Munitions in flight (§8.1 Missiles and munitions; TS `world/munitions.ts`):
//! each is an entity on a kinematic profile from `bgws_domains::missile`
//! (cruise, sea-skimming, ballistic, one-way drone, rocket, glide), with the
//! catalogue's speed, so defenders' radars see it at the right place and
//! time and the radar horizon shapes the defence's window [R6.2]. No
//! guidance laws or seekers: a strike lands with the munition's CEP about
//! its aim point; an interceptor or air-to-air missile flies to its
//! predicted intercept point and kills with its single-shot Pk if the target
//! is inside its seeker basket when it gets there.
//!
//! One-way attack drones placed in the scenario as forces (Shahed-like) are
//! flown the same way from their position to their strike task's aim point.

use super::{AirWorld, AskContext, Ledger};
use crate::components::*;
use crate::sim::{Sim, Trigger};
use crate::systems::{alive, name};
use bevy_ecs::prelude::*;
use bgws_catalogue::{Munition, MunitionKind, Vulnerability};
use bgws_core::TrackId;
use bgws_core::{EntityId, Enu, Side, SimTime};
use bgws_domains::air_defence::ThreatClass;
use bgws_domains::missile::{Flight, FlightClass, Profile};
use bgws_domains::vec3::V3;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Why it flies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    /// At a point on the surface: bombs, missiles, rockets, one-way drones.
    Strike,
    /// An air-defence interceptor at an air track.
    Intercept,
    /// An air-to-air missile.
    AirToAir,
}

/// What to launch.
#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub side: Side,
    pub launcher: Option<EntityId>,
    pub munition: String,
    /// Launch point (u = altitude).
    pub from: Enu,
    /// Aim or intercept point (u = altitude; for a strike, the ground there).
    pub to: Enu,
    /// The entity it is meant for, if any.
    pub target: Option<EntityId>,
    pub purpose: Purpose,
    /// The track it was fired at (interceptors).
    pub track: Option<TrackId>,
    /// Single-shot kill probability at the target (interceptors, AAMs, strikes).
    pub pk: f64,
}

/// A munition in flight.
#[derive(Component, Clone, Debug)]
pub struct InFlight {
    pub flight: Flight,
    pub side: Side,
    pub munition: String,
    pub label: String,
    /// What it is as a threat to the other side; `None` for interceptors and AAMs.
    pub class: Option<ThreatClass>,
    pub launcher: Option<EntityId>,
    pub target: Option<EntityId>,
    pub track: Option<TrackId>,
    pub purpose: Purpose,
    pub pk: f64,
    pub decoy: bool,
    pub cost: f64,
    pub alive: bool,
    /// Fast enough to need the 20 Hz rate.
    pub fast: bool,
    /// A scenario force (a one-way drone), not a spawned round.
    pub force: bool,
    pub rcs_m2: f64,
    pub cep: f64,
    pub lethal_radius: f64,
    pub structural_hits: f64,
    pub radar_guided: bool,
    pub kind: MunitionKind,
    /// How far from the intercept point a homing interceptor still finds its target.
    pub basket: f64,
}

/// The kinematic profile class for a munition.
pub fn class_of(kind: MunitionKind) -> FlightClass {
    match kind {
        MunitionKind::Ballistic => FlightClass::Ballistic,
        MunitionKind::Owa | MunitionKind::Loitering => FlightClass::OneWayDrone,
        MunitionKind::Rocket | MunitionKind::Shell | MunitionKind::Mortar => FlightClass::Rocket,
        MunitionKind::GlideBomb | MunitionKind::Bomb => FlightClass::GlideBomb,
        MunitionKind::Asm => FlightClass::SeaSkimmer,
        _ => FlightClass::Cruise,
    }
}

/// What the defence sees it as (before any decoy is recognised).
pub fn threat_class(kind: MunitionKind) -> Option<ThreatClass> {
    match kind {
        MunitionKind::Owa | MunitionKind::Loitering => Some(ThreatClass::OneWayDrone),
        MunitionKind::Cruise
        | MunitionKind::Agm
        | MunitionKind::Arm
        | MunitionKind::GlideBomb
        | MunitionKind::Bomb => Some(ThreatClass::Cruise),
        MunitionKind::Asm => Some(ThreatClass::SeaSkimmer),
        MunitionKind::Ballistic => Some(ThreatClass::Ballistic),
        MunitionKind::Rocket => Some(ThreatClass::Rocket),
        _ => None,
    }
}

fn profile_for(s: &Sim, m: &Munition, purpose: Purpose) -> Profile {
    let mut p = Profile::from_figures(class_of(m.kind), &s.ctx.figures);
    if m.speed_ms > 0.0 {
        p.speed = m.speed_ms;
    }
    if m.cep_m > 0.0 {
        p.cep = m.cep_m;
    }
    if purpose != Purpose::Strike {
        // Interceptors fly straight at the intercept point.
        p.cruise_alt = 0.0;
        p.loft = 0.0;
        p.cep = 0.0;
        if p.speed <= 0.0 {
            p.speed = 1_000.0;
        }
    }
    p
}

fn v3(p: Enu) -> V3 {
    V3::new(p.e, p.n, p.u)
}

/// Launch a munition: an entity flying from `from` to `to`. Returns its id.
pub fn launch(s: &mut Sim, aw: &mut AirWorld, spec: LaunchSpec) -> Option<EntityId> {
    let m = s.ctx.catalogue.munition(&spec.munition)?.clone();
    let now = s.ctx.now;
    let profile = profile_for(s, &m, spec.purpose);
    let mut to = spec.to;
    if spec.purpose == Purpose::Strike {
        to.u = s.ctx.ground.height(to.e, to.n);
    }
    let flight = Flight::new(profile, v3(spec.from), v3(to), now.secs());
    let fast = flight.profile.speed > 100.0;
    let id = EntityId(s.ctx.order.len() as u32);
    aw.launches += 1;
    let label = format!("{} #{}", m.name, aw.launches);
    let f = InFlight {
        flight,
        side: spec.side,
        munition: m.id.clone(),
        label,
        class: if spec.purpose == Purpose::Strike {
            threat_class(m.kind)
        } else {
            None
        },
        launcher: spec.launcher,
        target: spec.target,
        track: spec.track,
        purpose: spec.purpose,
        pk: spec.pk,
        decoy: m.decoy,
        cost: m.cost_usd,
        alive: true,
        fast,
        force: false,
        rcs_m2: aw.p.rcs_munition,
        cep: m.cep_m,
        lethal_radius: m.lethal_radius_m,
        structural_hits: m.structural_hits,
        radar_guided: matches!(
            m.guidance,
            bgws_catalogue::Guidance::Radar | bgws_catalogue::Guidance::ActiveRadar
        ),
        kind: m.kind,
        // A homing missile corrects errors that grow with its time of flight.
        basket: aw
            .p
            .basket_m
            .max(aw.p.basket_share * spec.to.sub(spec.from).norm()),
    };
    let heading = to.sub(spec.from).bearing();
    let e = s
        .world
        .spawn((
            Kin {
                pos: spec.from,
                vel: Enu::default(),
                heading,
                agl: spec.from.u - s.ctx.ground.height(spec.from.e, spec.from.n),
            },
            f,
        ))
        .id();
    s.ctx.order.push(e);
    aw.flights.push(id);
    aw.air_live = true;
    aw.fast_live |= fast;
    if spec.purpose == Purpose::Strike {
        aw.ledger.entry(spec.side).or_default().strike_cost += m.cost_usd;
    }
    // Missile warning for an aircraft shot at (`missileWarnings`).
    if let (Some(t), Purpose::Intercept | Purpose::AirToAir) = (spec.target, spec.purpose) {
        let te = s.ctx.entity(t);
        if let Some(i) = s.world.get::<Ident>(te) {
            if super::is_aircraft(&i.platform) {
                let rwr = i.platform.sensors.iter().any(|x| {
                    x.kind == bgws_catalogue::SensorKind::Rwr
                        || x.kind == bgws_catalogue::SensorKind::MissileWarning
                });
                let radar = matches!(
                    m.guidance,
                    bgws_catalogue::Guidance::Radar | bgws_catalogue::Guidance::ActiveRadar
                );
                let mut rng = s.ctx.rng("air.warn", id.0 as u64, t.0 as u64);
                if (rwr && radar) || rng.gen::<f64>() < aw.p.see_ir_missile {
                    aw.context.insert(
                        t,
                        AskContext {
                            track: None,
                            target: spec.launcher,
                            site: None,
                            ring_m: 0.0,
                        },
                    );
                    s.ctx.triggers.push(Trigger {
                        entity: t,
                        point: "missileWarning",
                        about: spec.launcher,
                    });
                }
            }
        }
    }
    Some(id)
}

/// One-way drones placed as forces with a strike task start flying.
pub fn setup(s: &mut Sim, aw: &mut AirWorld) {
    if !aw.air_on {
        return;
    }
    let order = s.ctx.order.clone();
    for (idx, &e) in order.iter().enumerate() {
        let Some(i) = s.world.get::<Ident>(e) else {
            continue;
        };
        if !super::is_munition_like(&i.platform) {
            continue;
        }
        // A strike on a ship is the naval module's to fly and defend.
        if s.world.get::<crate::domains::naval::defence::NavalStrike>(e).is_some() {
            continue;
        }
        let platform = i.platform.clone();
        let side = i.side;
        let dummy = i.dummy;
        let Some(force) = s.ctx.scenario.forces.get(idx).cloned() else {
            continue;
        };
        let Some(bgws_scenario::v4::EntityTask::Strike {
            aim_at, target_id, ..
        }) = force.task.clone()
        else {
            continue;
        };
        let k = *s.world.get::<Kin>(e).expect("kin");
        let g = s.ctx.ground.height(k.pos.e, k.pos.n);
        let from = Enu::new(k.pos.e, k.pos.n, g + k.agl.max(50.0));
        let aim = s.ctx.frame.geodetic_to_enu(aim_at);
        let to = Enu::new(aim.e, aim.n, s.ctx.ground.height(aim.e, aim.n));
        let warhead = platform
            .mounts
            .first()
            .and_then(|m| m.munitions.first())
            .and_then(|m| s.ctx.catalogue.munition(m))
            .cloned();
        let mut profile = Profile::from_figures(FlightClass::OneWayDrone, &s.ctx.figures);
        if platform.mover.cruise_kmh > 0.0 {
            profile.speed = platform.mover.cruise_kmh / 3.6;
        }
        profile.cruise_alt = 0.0;
        if let Some(w) = &warhead {
            if w.cep_m > 0.0 {
                profile.cep = w.cep_m;
            }
        }
        let flight = Flight::new(profile, v3(from), v3(to), 0.0);
        let rcs = platform
            .signatures
            .rcs_m2("x", bgws_catalogue::Aspect::Front)
            .max(1e-4);
        let decoy = dummy || platform.signatures.decoy || warhead.is_none();
        let f = InFlight {
            flight,
            side,
            munition: warhead.as_ref().map(|w| w.id.clone()).unwrap_or_default(),
            label: i.label.clone(),
            class: Some(ThreatClass::OneWayDrone),
            launcher: None,
            target: target_id
                .as_ref()
                .and_then(|t| s.ctx.by_name.get(t).copied()),
            track: None,
            purpose: Purpose::Strike,
            pk: warhead.as_ref().map(|w| w.pk).unwrap_or(0.0),
            decoy,
            cost: platform.cost_usd,
            alive: true,
            fast: false,
            force: true,
            rcs_m2: rcs,
            cep: warhead.as_ref().map(|w| w.cep_m).unwrap_or(0.0),
            lethal_radius: warhead.as_ref().map(|w| w.lethal_radius_m).unwrap_or(0.0),
            structural_hits: warhead.as_ref().map(|w| w.structural_hits).unwrap_or(0.0),
            radar_guided: false,
            kind: MunitionKind::Owa,
            basket: 0.0,
        };
        {
            let mut km = s.world.get_mut::<Kin>(e).expect("kin");
            km.pos = from;
            km.heading = to.sub(from).bearing();
        }
        if let Some(mut a) = s.world.get_mut::<Act>(e) {
            a.activity = Activity::Hold;
        }
        s.world.entity_mut(e).insert(f);
        aw.flights.push(EntityId(idx as u32));
    }
}

/// Move every munition along its flight; resolve the ones that arrive.
pub fn step(s: &mut Sim, aw: &mut AirWorld) {
    let t = s.ctx.now.secs();
    for id in aw.flights.clone() {
        let e = s.ctx.entity(id);
        let Some(f) = s.world.get::<InFlight>(e) else {
            continue;
        };
        if !f.alive {
            continue;
        }
        let pos = f.flight.position(t);
        let vel = f.flight.velocity(t);
        let arrived = t >= f.flight.arrival();
        let g = s.ctx.ground.height(pos.x, pos.y);
        {
            let mut k = s.world.get_mut::<Kin>(e).expect("kin");
            k.pos = Enu::new(pos.x, pos.y, pos.z);
            k.vel = Enu::new(vel.x, vel.y, vel.z);
            if vel.horizontal() > 1e-6 {
                k.heading = vel.heading();
            }
            k.agl = pos.z - g;
        }
        if arrived {
            resolve(s, aw, id);
        }
    }
}

/// The munition got there.
fn resolve(s: &mut Sim, aw: &mut AirWorld, id: EntityId) {
    let e = s.ctx.entity(id);
    let f = s.world.get::<InFlight>(e).expect("f").clone();
    s.world.get_mut::<InFlight>(e).expect("f").alive = false;
    match f.purpose {
        Purpose::Strike => impact(s, aw, id, &f),
        Purpose::Intercept | Purpose::AirToAir => intercept_end(s, aw, id, &f),
    }
}

/// Is this air target (aircraft or munition) still flying?
pub(crate) fn air_target_alive(s: &Sim, t: EntityId) -> bool {
    let e = s.ctx.entity(t);
    if let Some(f) = s.world.get::<InFlight>(e) {
        return f.alive;
    }
    if let Some(c) = s.world.get::<super::Craft>(e) {
        return c.airborne() && alive(s, e);
    }
    false
}

fn intercept_end(s: &mut Sim, aw: &mut AirWorld, id: EntityId, f: &InFlight) {
    let now = s.ctx.now;
    let point = Enu::new(f.flight.to.x, f.flight.to.y, f.flight.to.z);
    let shooter = f.launcher.unwrap_or(id);
    // The seeker takes what is in its basket: the intended target if it is
    // still there, else the nearest enemy air object (a decoy, a wingman).
    let candidates: Vec<EntityId> = aw
        .crafts
        .iter()
        .chain(aw.flights.iter())
        .copied()
        .filter(|c| Some(*c) != Some(id))
        .collect();
    let mut found: Option<(f64, EntityId)> = None;
    for c in candidates {
        if !air_target_alive(s, c) {
            continue;
        }
        let ce = s.ctx.entity(c);
        let enemy = s
            .world
            .get::<InFlight>(ce)
            .map(|x| x.side != f.side && x.class.is_some())
            .or_else(|| s.world.get::<Ident>(ce).map(|i| i.side != f.side))
            .unwrap_or(false);
        if !enemy && Some(c) != f.target {
            continue;
        }
        let Some(p) = s.world.get::<Kin>(ce).map(|k| k.pos) else {
            continue;
        };
        let d = p.sub(point).norm() - if Some(c) == f.target { 1.0 } else { 0.0 };
        // It homes on its own target over the whole basket; anything else
        // must stray into the seeker's close field.
        let reach = if Some(c) == f.target {
            f.basket
        } else {
            aw.p.basket_m
        };
        if d <= reach
            && found
                .map(|b| d < b.0 || (d == b.0 && c < b.1))
                .unwrap_or(true)
        {
            found = Some((d, c));
        }
    }
    let Some(t) = found.map(|x| x.1).or(f.target) else {
        let text = format!("{} finds nothing at the intercept point", f.label);
        s.ctx
            .log
            .push(now, "ad.intercept")
            .side(f.side)
            .entity(shooter)
            .text(text)
            .data(serde_json::json!({ "kill": false }));
        return;
    };
    let te = s.ctx.entity(t);
    let in_basket = found.is_some();
    let flying = air_target_alive(s, t);
    // Modifiers at the end-game: a defending aircraft, a low target for a radar seeker.
    let mut pk = f.pk;
    let mut why = Vec::new();
    if let Some(c) = s.world.get::<super::Craft>(te) {
        if c.defending_until >= now {
            pk *= aw.p.defend_factor;
            why.push("the target defended");
        }
    }
    if f.radar_guided
        && s.world
            .get::<Kin>(te)
            .map(|k| k.agl < aw.p.low_agl)
            .unwrap_or(false)
    {
        pk *= aw.p.low_factor;
        why.push("the target was low");
    }
    let mut rng = s.ctx.rng("air.endgame", id.0 as u64, t.0 as u64);
    let roll: f64 = rng.gen();
    let kill = flying && in_basket && roll < pk;
    let side = f.side;
    let text = if !flying {
        format!(
            "{} finds nothing: {} is already down",
            f.label,
            name_any(s, t)
        )
    } else if !in_basket {
        format!(
            "{} misses {}: not in the seeker basket",
            f.label,
            name_any(s, t)
        )
    } else if kill {
        format!("{} kills {}", f.label, name_any(s, t))
    } else {
        format!("{} misses {} (Pk {:.2})", f.label, name_any(s, t), pk)
    };
    let kind = if f.purpose == Purpose::AirToAir {
        "air.missile"
    } else {
        "ad.intercept"
    };
    s.ctx.log.push(now, kind).side(side).entity(shooter).target(t).text(text).data(serde_json::json!({ "munition": f.munition, "pk": pk, "roll": roll, "inBasket": in_basket, "kill": kill, "modifiers": why }));
    if kill {
        kill_air(s, aw, t, shooter, side);
    }
}

/// A name for any entity, munitions included.
pub(crate) fn name_any(s: &Sim, id: EntityId) -> String {
    let e = s.ctx.entity(id);
    if let Some(f) = s.world.get::<InFlight>(e) {
        if !f.force {
            return f.label.clone();
        }
    }
    name(s, id)
}

/// An air target is destroyed (one aircraft of a flight, or a munition).
pub(crate) fn kill_air(s: &mut Sim, aw: &mut AirWorld, t: EntityId, by: EntityId, side: Side) {
    let now = s.ctx.now;
    let te = s.ctx.entity(t);
    let mut value = 0.0;
    let mut decoy = false;
    let hit_flight = s.world.get_mut::<InFlight>(te).map(|mut f| {
        f.alive = false;
        (f.cost, f.decoy, f.force)
    });
    if let Some((cost, d, force)) = hit_flight {
        value = cost;
        decoy = d;
        if force {
            if let Some(mut h) = s.world.get_mut::<Health>(te) {
                h.losses.push(now);
                h.fit = 0;
                h.destroyed = true;
            }
        }
    } else if s.world.get::<super::Craft>(te).is_some() {
        let cost = s
            .world
            .get::<Ident>(te)
            .map(|i| i.platform.cost_usd)
            .unwrap_or(0.0);
        value = cost;
        let destroyed = {
            let mut h = s.world.get_mut::<Health>(te).expect("h");
            if h.fit > 0 {
                h.fit -= 1;
                h.losses.push(now);
            }
            if h.fit == 0 {
                h.destroyed = true;
            }
            h.destroyed
        };
        if destroyed {
            s.world.get_mut::<super::Craft>(te).expect("c").status = super::CraftStatus::Lost;
        }
        if let Some(mut c) = s.world.get_mut::<Crew>(te) {
            c.new_casualties += 0.5;
        }
        let text = format!(
            "{} is shot down by {}{}",
            name(s, t),
            name_any(s, by),
            if destroyed {
                ""
            } else {
                " (one aircraft of the flight)"
            }
        );
        let enemy = s
            .world
            .get::<Ident>(te)
            .map(|i| i.side)
            .unwrap_or(side.enemy());
        s.ctx
            .log
            .push(now, "air.shotDown")
            .side(enemy)
            .entity(t)
            .target(by)
            .text(text)
            .data(serde_json::json!({ "costUsd": cost, "flightDestroyed": destroyed }));
        if destroyed {
            let t2 = format!("{} is destroyed", name(s, t));
            s.ctx
                .log
                .push(now, "unit.destroyed")
                .side(enemy)
                .entity(t)
                .text(t2);
        }
    }
    let l: &mut Ledger = aw.ledger.entry(side).or_default();
    l.kills += 1;
    l.value_killed += value;
    if decoy {
        l.decoys_killed += 1;
    }
}

/// A strike arrives: the impact point from the CEP, then the damage there.
fn impact(s: &mut Sim, aw: &mut AirWorld, id: EntityId, f: &InFlight) {
    let now = s.ctx.now;
    let e = s.ctx.entity(id);
    if f.force {
        // The one-way drone itself is expended.
        if let Some(mut h) = s.world.get_mut::<Health>(e) {
            h.fit = 0;
            h.destroyed = true;
        }
    }
    if f.class.is_some() && !f.decoy {
        aw.ledger.entry(f.side.enemy()).or_default().leakers += 1;
    }
    if f.decoy || f.lethal_radius <= 0.0 && f.structural_hits <= 0.0 {
        let text = format!(
            "{} comes down harmlessly{}",
            f.label,
            if f.decoy { " (a decoy)" } else { "" }
        );
        s.ctx
            .log
            .push(now, "munition.spent")
            .side(f.side)
            .entity(id)
            .text(text);
        return;
    }
    // An anti-radiation missile whose radar went quiet loses its guidance.
    let mut cep_mult = 1.0;
    if f.kind == MunitionKind::Arm {
        if let Some(t) = f.target {
            if !super::defence::radiating(s, s.ctx.entity(t)) {
                cep_mult = aw.p.arm_silent_cep_mult;
            }
        }
    }
    let mut flight = f.flight.clone();
    flight.profile.cep *= cep_mult;
    // An ARM homes on the radar while it radiates.
    if f.kind == MunitionKind::Arm && cep_mult == 1.0 {
        if let Some(p) = f
            .target
            .and_then(|t| s.world.get::<Kin>(s.ctx.entity(t)).map(|k| k.pos))
        {
            flight.to = V3::new(p.e, p.n, p.u);
        }
    }
    let mut rng = s.ctx.rng("air.impact", id.0 as u64, 0);
    let (point, miss) = flight.sample_impact(0.0, &mut rng);
    let at = Enu::new(point.x, point.y, s.ctx.ground.height(point.x, point.y));
    let hits = damage_at(s, aw, at, f, id);
    let text = format!(
        "{} lands {:.0} m from its aim point{}",
        f.label,
        miss,
        if hits.is_empty() {
            String::new()
        } else {
            format!(
                ": {}",
                hits.iter()
                    .map(|h| h.1.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    s.ctx.log.push(now, "munition.impact").side(f.side).entity(f.launcher.unwrap_or(id)).text(text).data(serde_json::json!({ "munition": f.munition, "cepM": flight.profile.cep, "missM": miss, "hits": hits.iter().map(|h| h.0.0).collect::<Vec<_>>() }));
    for (t, _) in &hits {
        let te = s.ctx.entity(*t);
        if let Some(i) = s.world.get::<Ident>(te) {
            if i.domain == bgws_catalogue::Domain::Node {
                s.ctx.triggers.push(Trigger {
                    entity: *t,
                    point: "nodeStruck",
                    about: f.launcher,
                });
            }
        }
    }
}

/// Damage to everything near an impact point: structural hits on
/// structures and ships, kill chances by platform for the rest.
fn damage_at(
    s: &mut Sim,
    aw: &mut AirWorld,
    at: Enu,
    f: &InFlight,
    id: EntityId,
) -> Vec<(EntityId, String)> {
    let now = s.ctx.now;
    let lethal = f.lethal_radius.max(1.0);
    let foot = aw.p.footprint_m;
    let mut out = Vec::new();
    let order = s.ctx.order.clone();
    for (idx, &e) in order.iter().enumerate() {
        let tid = EntityId(idx as u32);
        let (Some(i), Some(k)) = (
            s.world.get::<Ident>(e).cloned(),
            s.world.get::<Kin>(e).copied(),
        ) else {
            continue;
        };
        if !alive(s, e) || s.world.get::<InFlight>(e).is_some() {
            continue;
        }
        if s.world
            .get::<super::Craft>(e)
            .map(|c| c.airborne())
            .unwrap_or(false)
        {
            continue;
        }
        let d = Enu::new(k.pos.e - at.e, k.pos.n - at.n, 0.0).horizontal();
        let body = &i.platform.body;
        let side = i.side;
        let structural = matches!(
            body.vulnerability,
            Vulnerability::Structure | Vulnerability::Ship
        );
        if structural {
            let half = body.length_m.max(body.width_m) / 2.0;
            if d > lethal + half {
                continue;
            }
            let hits = f.structural_hits.max(0.05);
            let total = {
                let x = aw.structural.entry(tid).or_insert(0.0);
                *x += hits;
                *x
            };
            let sp = body.staying_power.max(0.1);
            let destroyed = total >= sp;
            if destroyed {
                let mut h = s.world.get_mut::<Health>(e).expect("h");
                for _ in 0..h.fit {
                    h.losses.push(now);
                }
                h.fit = 0;
                h.destroyed = true;
            }
            aw.ledger.entry(f.side).or_default().value_destroyed +=
                if destroyed { i.platform.cost_usd } else { 0.0 };
            let text = format!(
                "{} takes {:.1} hits ({:.1} of {:.1}){}",
                name(s, tid),
                hits,
                total,
                sp,
                if destroyed { ", out of action" } else { "" }
            );
            s.ctx
                .log
                .push(now, "munition.structural")
                .side(side)
                .entity(tid)
                .target(id)
                .text(text.clone())
                .data(serde_json::json!({ "hits": hits, "total": total, "stayingPower": sp }));
            out.push((tid, text));
            continue;
        }
        if d > lethal + foot {
            continue;
        }
        let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(0);
        let share = (lethal * lethal / (foot * foot)).min(1.0)
            * (-(((d - lethal).max(0.0)) / foot).powi(2)).exp();
        let aimed = f.target == Some(tid) && d <= lethal;
        let mut rng = s.ctx.rng("air.damage", id.0 as u64, tid.0 as u64);
        let mut kills = 0;
        for k in 0..fit {
            let p = if k == 0 && aimed {
                f.pk.max(share)
            } else {
                f.pk.max(0.3) * share
            };
            if rng.gen::<f64>() < p {
                kills += 1;
            }
        }
        if kills == 0 {
            if let Some(mut c) = s.world.get_mut::<Crew>(e) {
                c.blasts_per_min += 6.0;
            }
            continue;
        }
        let (destroyed, cost) = {
            let mut h = s.world.get_mut::<Health>(e).expect("h");
            let k = kills.min(h.fit);
            h.fit -= k;
            for _ in 0..k {
                h.losses.push(now);
            }
            if h.fit == 0 {
                h.destroyed = true;
            }
            (h.destroyed, i.platform.cost_usd * k as f64)
        };
        if let Some(mut c) = s.world.get_mut::<Crew>(e) {
            c.new_casualties += kills as f64 / fit.max(1) as f64;
            c.blasts_per_min += 12.0;
        }
        if destroyed {
            if let Some(mut c) = s.world.get_mut::<super::Craft>(e) {
                c.status = super::CraftStatus::Lost;
            }
        }
        aw.ledger.entry(f.side).or_default().value_destroyed += cost;
        let text = format!(
            "{} loses {} of {}{}",
            name(s, tid),
            kills,
            fit,
            if destroyed { " and is destroyed" } else { "" }
        );
        out.push((tid, text.clone()));
        if destroyed {
            let t2 = format!("{} is destroyed", name(s, tid));
            s.ctx
                .log
                .push(now, "unit.destroyed")
                .side(side)
                .entity(tid)
                .text(t2);
        }
    }
    out
}

/// Launch from another domain module without holding the air state.
pub fn launch_munition(s: &mut Sim, spec: LaunchSpec) -> Option<EntityId> {
    super::with(s, |s, aw| launch(s, aw, spec))
}

/// When a munition will arrive (for callers scheduling around it).
pub fn arrival_of(s: &Sim, id: EntityId) -> Option<SimTime> {
    s.world
        .get::<InFlight>(s.ctx.entity(id))
        .map(|f| SimTime::from_secs(f.flight.arrival()))
}
