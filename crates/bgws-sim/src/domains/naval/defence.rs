//! Salvo combat at sea (§8.6; R6.1, R6.2, R6.3, P28): anti-ship missiles,
//! one-way drones and torpedoes against ships, the ship's layered defence,
//! damage, USVs.
//!
//! Every threat at a ship is a kinematic flight (`bgws_domains::missile`)
//! that homes on its target. When it rises over the ship's radar horizon it
//! joins the ship's raid queue and the combat system asks `vampireInbound`
//! (which layer). The raid is then fought once as a salvo against layers:
//! `bgws_domains::air_defence::engage_raid` with the ship's (and close
//! consorts') systems built from their magazines, which deplete. Each
//! threat's fate (killed by a system at a time, or leaking) plays out on the
//! clock; a leaker that reaches its ship rolls its hit and damages
//! compartments, with fire and flooding fought by damage control over time.

use super::fleet::{alive, ask_or_do, ShipCombat, Vessel, VesselKind, NavTask};
use super::{label, put, take, NavalState};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_catalogue::{Domain, WeaponKind};
use bgws_core::{Enu, EntityId, Side, SimDuration, SimTime};
use bgws_domains::air_defence::{engage_raid, AdSystem, Assignment, Doctrine, KillChain, Layer, RaidOutcome, Threat, ThreatClass};
use bgws_domains::missile::{radar_horizon, Flight, FlightClass, Profile};
use bgws_domains::V3;
use rand::Rng;
use serde::Serialize;
use std::collections::BTreeMap;

/// Where a threat is in its life.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InboundState {
    Flying,
    Detected { at: SimTime },
    Engaged,
    Done,
}

/// What the raid decided for a threat.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Fate {
    Killed { at: f64, by: String },
    Leaks,
}

/// One weapon in flight at a ship (or a one-way drone claimed from the air).
#[derive(Clone, Debug)]
pub struct Inbound {
    pub id: u64,
    /// The air entity it is, if it is one (a claimed one-way drone).
    pub entity: Option<EntityId>,
    pub shooter: Option<EntityId>,
    pub side: Side,
    pub target: EntityId,
    pub class: ThreatClass,
    pub munition: String,
    pub flight: Flight,
    pub cost: f64,
    /// Damage scale (1 = a typical anti-ship missile).
    pub warhead: f64,
    /// Hit chance once its seeker has the ship.
    pub pk: f64,
    pub basket: f64,
    pub state: InboundState,
    pub fate: Option<Fate>,
}

/// Marks an air entity whose flight this domain owns (the air domain skips it).
#[derive(Component, Clone, Debug)]
pub struct NavalStrike {
    pub inbound: u64,
}

/// A torpedo running at a target.
#[derive(Clone, Debug)]
pub struct Torpedo {
    pub id: u64,
    pub shooter: EntityId,
    pub side: Side,
    pub target: EntityId,
    pub pk: f64,
    pub warhead: f64,
}

/// The layer a ship's combat system fights a raid with (vampireInbound).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum LayerChoice {
    Long,
    Medium,
    Short,
    Unmask,
    Decoy,
}

/// One raid fought, for the AAR (rule 10).
#[derive(Clone, Debug, Serialize)]
pub struct RaidRecord {
    pub at: SimTime,
    pub ship: EntityId,
    pub layer: LayerChoice,
    pub threats: usize,
    pub missiles: u32,
    pub engaged: u32,
    pub gun_rounds: u32,
    pub outcome: RaidOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SysKind {
    Long,
    Medium,
    Short,
    Ciws,
    Gun,
    Ew,
}

impl SysKind {
    fn sam(self) -> bool {
        matches!(self, SysKind::Long | SysKind::Medium | SysKind::Short)
    }
}

fn v3(p: Enu) -> V3 {
    V3::new(p.e, p.n, 0.0)
}

fn flight_class(c: ThreatClass) -> FlightClass {
    match c {
        ThreatClass::SeaSkimmer => FlightClass::SeaSkimmer,
        ThreatClass::Ballistic => FlightClass::Ballistic,
        ThreatClass::OneWayDrone | ThreatClass::Decoy => FlightClass::OneWayDrone,
        ThreatClass::Rocket => FlightClass::Rocket,
        _ => FlightClass::Cruise,
    }
}

/// Claim air entities on a strike at a ship: this domain flies them (§8.6).
pub(crate) fn claim_air_threats(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let f = s.ctx.figures.clone();
    let refw = st.p.g("ref_warhead_kg");
    for (idx, &e) in s.ctx.order.clone().iter().enumerate() {
        let (id, side, platform, dummy) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side, i.platform.clone(), i.dummy)
        };
        if !matches!(platform.domain, Domain::Air | Domain::Munition) {
            continue;
        }
        let Some(bgws_scenario::v4::EntityTask::Strike { target_id, aim_at, alt_m }) = s.ctx.scenario.forces.get(idx).and_then(|f| f.task.clone()) else { continue };
        let aim = s.ctx.frame.geodetic_to_enu(aim_at);
        let is_ship = |s: &Sim, t: EntityId| s.world.get::<Ident>(s.ctx.entity(t)).map(|i| matches!(i.domain, Domain::Surface | Domain::Subsurface) && i.side != side).unwrap_or(false);
        let target = target_id.and_then(|t| s.ctx.by_name.get(&t).copied()).filter(|t| is_ship(s, *t)).or_else(|| {
            s.ctx
                .order
                .iter()
                .filter_map(|&o| {
                    let i = s.world.get::<Ident>(o)?;
                    let d = s.world.get::<Kin>(o)?.pos.sub(aim).horizontal();
                    (i.domain == Domain::Surface && i.side != side && d <= 5_000.0).then_some((d, i.id))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|x| x.1)
        });
        let Some(target) = target else { continue };
        let class = if dummy || platform.signatures.decoy {
            ThreatClass::Decoy
        } else if platform.class.to_lowercase().contains("owa") || platform.mover.kind == bgws_catalogue::MoverKind::Missile && platform.mover.cruise_kmh < 400.0 {
            ThreatClass::OneWayDrone
        } else {
            ThreatClass::Cruise
        };
        let mun = platform.mounts.first().and_then(|m| m.munitions.first()).and_then(|m| s.ctx.catalogue.munition(m)).cloned();
        let mut prof = Profile::from_figures(flight_class(class), &f);
        if platform.mover.cruise_kmh > 0.0 {
            prof.speed = platform.mover.cruise_kmh / 3.6;
        }
        if let Some(a) = alt_m {
            prof.cruise_alt = a;
        }
        let from = s.world.get::<Kin>(e).expect("kin").pos;
        let to = s.world.get::<Kin>(s.ctx.entity(target)).expect("kin").pos;
        let n = st.next_key();
        st.inbound.insert(
            n,
            Inbound {
                id: n,
                entity: Some(id),
                shooter: None,
                side,
                target,
                class,
                munition: mun.as_ref().map(|m| m.id.clone()).unwrap_or_else(|| platform.id.clone()),
                flight: Flight::new(prof, v3(from), v3(to), 0.0),
                cost: mun.as_ref().map(|m| m.cost_usd).filter(|c| *c > 0.0).unwrap_or(2.0e4),
                warhead: if class == ThreatClass::Decoy { 0.0 } else { mun.as_ref().map(|m| m.warhead_kg / refw).unwrap_or(0.25) },
                pk: mun.as_ref().map(|m| m.pk).unwrap_or(0.5),
                basket: st.p.g("owa_basket_m"),
                state: InboundState::Flying,
                fate: None,
            },
        );
        s.world.entity_mut(e).insert(NavalStrike { inbound: n });
    }
    put(s, st);
}

/// A salvo of anti-ship missiles from a ship, submarine or coastal battery
/// at an aim point (the track's position, led by its velocity). Returns the
/// number fired.
pub(crate) fn launch_salvo(s: &mut Sim, st: &mut NavalState, shooter: EntityId, target: EntityId, aim: Enu, munition: &str, n: u32) -> u32 {
    let Some(mun) = s.ctx.catalogue.munition(munition).cloned() else { return 0 };
    let se = s.ctx.entity(shooter);
    // Take the rounds: a ship's magazine, else the launcher's mounts.
    let have = if let Some(c) = s.world.get::<ShipCombat>(se) {
        c.magazine.get(munition).copied().unwrap_or(0)
    } else {
        s.world.get::<Arms>(se).map(|a| a.mounts.iter().filter(|m| m.munition == munition).map(|m| m.rounds).sum()).unwrap_or(0)
    };
    let n = n.min(have);
    if n == 0 {
        return 0;
    }
    if let Some(mut c) = s.world.get_mut::<ShipCombat>(se) {
        *c.magazine.get_mut(munition).expect("listed") -= n;
    } else if let Some(mut a) = s.world.get_mut::<Arms>(se) {
        let mut left = n;
        for m in a.mounts.iter_mut().filter(|m| m.munition == munition) {
            let t = left.min(m.rounds);
            m.rounds -= t;
            left -= t;
        }
    }
    let class = match mun.flight {
        bgws_catalogue::FlightProfile::SeaSkim => ThreatClass::SeaSkimmer,
        bgws_catalogue::FlightProfile::Ballistic => ThreatClass::Ballistic,
        _ => ThreatClass::Cruise,
    };
    let mut prof = Profile::from_figures(flight_class(class), &s.ctx.figures);
    if mun.speed_ms > 0.0 {
        prof.speed = mun.speed_ms;
    }
    let from = s.world.get::<Kin>(se).expect("kin").pos;
    let side = s.world.get::<Ident>(se).expect("ident").side;
    let now = s.ctx.now.secs();
    let ripple = st.p.g("ripple_s");
    for i in 0..n {
        let id = st.next_key();
        st.inbound.insert(
            id,
            Inbound {
                id,
                entity: None,
                shooter: Some(shooter),
                side,
                target,
                class,
                munition: mun.id.clone(),
                flight: Flight::new(prof.clone(), v3(from), v3(aim), now + ripple * i as f64),
                cost: mun.cost_usd,
                warhead: mun.warhead_kg / st.p.g("ref_warhead_kg"),
                pk: mun.pk,
                basket: st.p.g("seeker_basket_m"),
                state: InboundState::Flying,
                fate: None,
            },
        );
    }
    st.stats.salvos += 1;
    if let Some(mut sig) = s.world.get_mut::<SigState>(se) {
        sig.last_fired = Some(s.ctx.now);
    }
    let range = aim.sub(from).horizontal();
    let text = format!("{} fires a salvo of {n} {} at {} {:.0} km away", label(s, shooter), mun.name, label(s, target), range / 1000.0);
    s.ctx.log.push(s.ctx.now, "naval.salvo").side(side).entity(shooter).target(target).text(text).data(serde_json::json!({ "munition": mun.id, "count": n, "rangeM": range }));
    n
}

/// Retarget, move, detect, kill and land every threat in flight (base rate).
pub(crate) fn fly_threats(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    let t = now.secs();
    let ids: Vec<u64> = st.inbound.iter().filter(|(_, i)| i.state != InboundState::Done).map(|(k, _)| *k).collect();
    for id in ids {
        let mut th = st.inbound[&id].clone();
        // A claimed air entity shot down by someone else is gone.
        if let Some(eid) = th.entity {
            if !alive(s, s.ctx.entity(eid)) {
                th.state = InboundState::Done;
                st.inbound.insert(id, th);
                continue;
            }
        }
        // Home on the ship: a virtual origin keeps the progress so far.
        let te = s.ctx.entity(th.target);
        // (Not in the last seconds: the terminal dive is committed.)
        if alive(s, te) && t >= th.flight.launched && th.flight.arrival() - t > 3.0 {
            let aim = s.world.get::<Kin>(te).expect("kin").pos;
            let cur = th.flight.position(t);
            let d = V3::new(aim.e - cur.x, aim.n - cur.y, 0.0);
            let dist = d.horizontal();
            if dist > 1.0 {
                let u = d * (1.0 / dist);
                let flown = th.flight.profile.speed * (t - th.flight.launched);
                th.flight.from = V3::new(cur.x - u.x * flown, cur.y - u.y * flown, 0.0);
                th.flight.to = V3::new(aim.e, aim.n, 0.0);
            }
        }
        let p = th.flight.position(t);
        if let Some(eid) = th.entity {
            let e = s.ctx.entity(eid);
            let vel = th.flight.velocity(t);
            if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                k.pos = Enu::new(p.x, p.y, p.z);
                k.agl = p.z;
                k.vel = Enu::new(vel.x, vel.y, vel.z);
                if vel.horizontal() > 0.1 {
                    k.heading = vel.x.atan2(vel.y);
                }
            }
        }
        // Its fate from the raid.
        if let Some(Fate::Killed { at, by }) = th.fate.clone() {
            if t >= at && at <= th.flight.arrival() {
                th.state = InboundState::Done;
                if let Some(eid) = th.entity {
                    destroy(s, eid);
                }
                let what = describe(&th);
                let text = format!("{by} kills {what} aimed at {}", label(s, th.target));
                s.ctx.log.push(now, "naval.intercept").side(th.side.enemy()).target(th.target).text(text).data(serde_json::json!({ "inbound": id, "by": by, "class": th.class }));
                st.inbound.insert(id, th);
                continue;
            }
        }
        if t >= th.flight.arrival() && t >= th.flight.launched {
            st.inbound.insert(id, th);
            impact(s, &mut st, id);
            continue;
        }
        // Over the target's radar horizon: into its raid queue.
        if th.state == InboundState::Flying && t >= th.flight.launched && alive(s, te) {
            if let (Some(v), Some(_)) = (s.world.get::<Vessel>(te).filter(|v| v.kind == VesselKind::Warship), s.world.get::<ShipCombat>(te)) {
                let ship = s.world.get::<Kin>(te).expect("kin").pos;
                let range_max = s.world.get::<Ident>(te).expect("ident").platform.sensors.iter().filter(|x| x.kind == bgws_catalogue::SensorKind::Radar).map(|x| x.range_max_m).fold(0.0, f64::max);
                let d = V3::new(p.x - ship.e, p.y - ship.n, 0.0).horizontal();
                if v.radar_h > 0.0 && !v.silent && d <= range_max && d <= radar_horizon(v.radar_h, p.z.max(1.0)) {
                    th.state = InboundState::Detected { at: now };
                    {
                        let mut c = s.world.get_mut::<ShipCombat>(te).expect("combat");
                        c.pending.push(id);
                        if c.pending_since.is_none() {
                            c.pending_since = Some(now);
                        }
                    }
                    let what = describe(&th);
                    let text = format!("{} detects {what} inbound at {:.0} km", label(s, th.target), d / 1000.0);
                    let side = s.world.get::<Ident>(te).expect("ident").side;
                    s.ctx.log.push(now, "naval.vampire").side(side).entity(th.target).text(text).data(serde_json::json!({ "inbound": id, "class": th.class, "rangeM": d, "altM": p.z }));
                }
            }
        }
        st.inbound.insert(id, th);
    }
    put(s, st);
}

fn describe(th: &Inbound) -> &'static str {
    match th.class {
        ThreatClass::OneWayDrone => "a one-way drone",
        ThreatClass::Decoy => "a one-way drone (a decoy)",
        ThreatClass::SeaSkimmer => "a sea-skimming missile",
        ThreatClass::Ballistic => "a ballistic missile",
        _ => "a cruise missile",
    }
}

fn destroy(s: &mut Sim, id: EntityId) {
    let now = s.ctx.now;
    if let Some(mut h) = s.world.get_mut::<Health>(s.ctx.entity(id)) {
        if !h.destroyed {
            h.destroyed = true;
            for _ in 0..h.fit {
                h.losses.push(now);
            }
            h.fit = 0;
        }
    }
}

/// A threat reaches its aim point: the seeker looks for a ship in its basket.
fn impact(s: &mut Sim, st: &mut NavalState, id: u64) {
    let now = s.ctx.now;
    let Some(mut th) = st.inbound.get(&id).cloned() else { return };
    th.state = InboundState::Done;
    st.inbound.insert(id, th.clone());
    if let Some(eid) = th.entity {
        destroy(s, eid);
    }
    st.stats.leakers += 1;
    let aim = Enu::new(th.flight.to.x, th.flight.to.y, 0.0);
    let in_basket = |s: &Sim, t: EntityId| -> Option<f64> {
        let e = s.ctx.entity(t);
        if !alive(s, e) || s.world.get::<Vessel>(e).map(|v| v.depth > 20.0).unwrap_or(true) {
            return None;
        }
        let d = s.world.get::<Kin>(e)?.pos.sub(aim).horizontal();
        (d <= th.basket).then_some(d)
    };
    let victim = in_basket(s, th.target).map(|_| th.target).or_else(|| {
        s.ctx
            .order
            .iter()
            .filter_map(|&o| {
                let i = s.world.get::<Ident>(o)?;
                if i.side == th.side {
                    return None;
                }
                in_basket(s, i.id).map(|d| (d, i.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|x| x.1)
    });
    let what = describe(&th);
    let Some(victim) = victim else {
        let text = format!("{what} aimed at {} finds no ship and falls into the sea", label(s, th.target));
        s.ctx.log.push(now, "naval.miss").side(th.side).target(th.target).text(text).data(serde_json::json!({ "inbound": id }));
        return;
    };
    if th.class == ThreatClass::Decoy || th.warhead <= 0.0 {
        let text = format!("{what} reaches {} harmlessly", label(s, victim));
        s.ctx.log.push(now, "naval.miss").side(th.side).target(victim).text(text).data(serde_json::json!({ "inbound": id, "decoy": true }));
        return;
    }
    let ve = s.ctx.entity(victim);
    let decoyed = s.world.get::<ShipCombat>(ve).map(|c| c.decoy_until > now).unwrap_or(false);
    let p = th.pk * if decoyed { st.p.g("decoy_factor") } else { 1.0 };
    let mut rng = s.ctx.rng("naval.impact", id, victim.0 as u64);
    if rng.gen::<f64>() >= p {
        let text = format!("{what} misses {}{}", label(s, victim), if decoyed { " (decoyed)" } else { "" });
        s.ctx.log.push(now, "naval.miss").side(th.side).target(victim).text(text).data(serde_json::json!({ "inbound": id, "p": p }));
        return;
    }
    let text = format!("{what} hits {}", label(s, victim));
    s.ctx.log.push(now, "naval.hit").side(th.side).target(victim).text(text).data(serde_json::json!({ "inbound": id, "p": p, "munition": th.munition, "shooter": th.shooter }));
    apply_hit(s, st, victim, th.warhead, th.shooter, &th.munition);
}

/// A hit on a ship: a compartment takes flooding, fire and maybe its
/// systems; a boat or anything without compartments is simply lost.
pub(crate) fn apply_hit(s: &mut Sim, st: &mut NavalState, target: EntityId, scale: f64, by: Option<EntityId>, what: &str) {
    let now = s.ctx.now;
    let e = s.ctx.entity(target);
    if !alive(s, e) {
        return;
    }
    let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
    let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
    st.recent_hits.retain(|(t, _)| (now - *t).secs() < 86_400.0);
    st.recent_hits.push((now, pos));
    st.stats.ship_hits += 1;
    if side == Side::Neutral {
        st.stats.neutral_hits += 1;
    }
    let Some(mut c) = s.world.get::<ShipCombat>(e).cloned() else {
        destroy(s, target);
        let text = format!("{} is destroyed by {what}", label(s, target));
        s.ctx.log.push(now, "naval.sunk").side(side).entity(target).text(text);
        return;
    };
    let prev = c.state.clone();
    let k = (st.p.g("ref_staying") / c.staying).sqrt();
    let mut rng = s.ctx.rng("naval.damage", target.0 as u64, c.hits as u64);
    let n = c.damage.graph.node_count();
    let i = rng.gen_range(0..n.max(1));
    c.damage.hit(i, scale * k, &st.p.damage, &mut rng);
    c.hits += 1;
    c.state = c.damage.state(&st.p.damage);
    let compartment = c.damage.graph.node_weights().nth(i).map(|x| x.name.clone()).unwrap_or_default();
    *s.world.get_mut::<ShipCombat>(e).expect("combat") = c.clone();
    let text = format!("{} is hit by {what} in the {compartment}: flooding {:.0}%, {} fires", label(s, target), c.state.flooding * 100.0, c.state.fires);
    s.ctx.log.push(now, "naval.damage").side(side).entity(target).text(text).data(serde_json::json!({ "by": by, "compartment": compartment, "scale": scale * k, "state": c.state }));
    consequences(s, target, &prev, &c.state);
}

fn consequences(s: &mut Sim, target: EntityId, prev: &bgws_domains::naval::DamageState, now_state: &bgws_domains::naval::DamageState) {
    let now = s.ctx.now;
    let e = s.ctx.entity(target);
    let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
    if now_state.sunk && !prev.sunk {
        destroy(s, target);
        let text = format!("{} sinks (flooding {:.0}%)", label(s, target), now_state.flooding * 100.0);
        s.ctx.log.push(now, "naval.sunk").side(side).entity(target).text(text.clone()).data(serde_json::json!({ "state": now_state }));
        s.ctx.log.push(now, "unit.destroyed").side(side).entity(target).text(text);
        return;
    }
    if let Some(mut h) = s.world.get_mut::<Health>(e) {
        if now_state.mission_kill {
            h.firepower_kills = h.fit;
        }
        if now_state.systems_lost.contains(&bgws_domains::naval::ShipSystem::Propulsion) {
            h.mobility_kills = h.fit;
        }
    }
    if now_state.mission_kill && !prev.mission_kill {
        let text = format!("{} is out of action: lost {:?}", label(s, target), now_state.systems_lost);
        s.ctx.log.push(now, "naval.missionKill").side(side).entity(target).text(text).data(serde_json::json!({ "state": now_state }));
    }
}

/// Fire and flooding over time, damage control to the worst first (minute rate).
pub(crate) fn damage_control(s: &mut Sim, dt: f64) {
    let Some(st) = take(s) else { return };
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let Some(mut c) = s.world.get::<ShipCombat>(e).cloned() else { continue };
        if c.state.fires == 0 && c.state.flooding <= 0.0 {
            continue;
        }
        let id = s.world.get::<Ident>(e).expect("ident").id;
        let prev = c.state.clone();
        let mut rng = s.ctx.rng("naval.dc", id.0 as u64, 0);
        c.damage.step(dt / 60.0, &st.p.damage, &mut rng);
        c.state = c.damage.state(&st.p.damage);
        let next = c.state.clone();
        *s.world.get_mut::<ShipCombat>(e).expect("combat") = c;
        if prev.fires > 0 && next.fires == 0 {
            let side = s.world.get::<Ident>(e).expect("ident").side;
            let text = format!("{}: fires out, flooding {:.0}%", label(s, id), next.flooding * 100.0);
            s.ctx.log.push(s.ctx.now, "naval.damageControl").side(side).entity(id).text(text);
        }
        consequences(s, id, &prev, &next);
    }
    put(s, st);
}

/// The combat system's queue (base rate): ask which layer, or engage by
/// doctrine when the question is not answered in time [R6.2].
pub(crate) fn ship_defence(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(c) = s.world.get::<ShipCombat>(e).cloned() else { continue };
        if c.pending.is_empty() {
            continue;
        }
        let id = s.world.get::<Ident>(e).expect("ident").id;
        let pending: Vec<u64> = c.pending.iter().copied().filter(|p| st.inbound.get(p).map(|i| matches!(i.state, InboundState::Detected { .. })).unwrap_or(false)).collect();
        if pending.is_empty() || !alive(s, e) {
            let mut cm = s.world.get_mut::<ShipCombat>(e).expect("combat");
            cm.pending.clear();
            cm.pending_since = None;
            cm.asked = false;
            continue;
        }
        s.world.get_mut::<ShipCombat>(e).expect("combat").pending = pending.clone();
        if let Some((layer, at)) = c.layer {
            if (now - at).secs() < st.p.g("cic_hold_s") {
                engage(s, &mut st, id, layer);
                continue;
            }
        }
        let (opt, action, layer) = doctrine(s, &st, e, &pending);
        if !c.asked {
            s.world.get_mut::<ShipCombat>(e).expect("combat").asked = true;
            let lead = pending.first().and_then(|p| st.inbound.get(p)).and_then(|i| i.entity.or(i.shooter));
            put(s, st);
            ask_or_do(s, id, "vampireInbound", lead, opt, action);
            st = take(s).expect("state");
        } else if c.pending_since.map(|t| (now - t).secs() >= st.p.g("cic_auto_s")).unwrap_or(false) {
            let side = s.world.get::<Ident>(e).expect("ident").side;
            let text = format!("{}'s combat system engages by doctrine ({opt}) before an order comes", label(s, id));
            s.ctx.log.push(now, "naval.cicAuto").side(side).entity(id).text(text);
            engage(s, &mut st, id, layer);
        }
    }
    put(s, st);
}

/// The doctrine's layer for a raid (the TS rule): cheap threats to the inner
/// layers, a close raid to every layer that bears, otherwise the outermost.
fn doctrine(s: &Sim, st: &NavalState, e: Entity, pending: &[u64]) -> (&'static str, &'static str, LayerChoice) {
    let kinds = ship_kinds(s, e);
    let has = |k: SysKind| kinds.iter().any(|x| x.0 == k);
    let t = s.ctx.now.secs();
    let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
    let cheap = pending.iter().filter_map(|p| st.inbound.get(p)).all(|i| i.cost < st.p.g("cheap_threat_usd"));
    let nearest = pending.iter().filter_map(|p| st.inbound.get(p)).map(|i| {
        let q = i.flight.position(t);
        (q.x - pos.e).hypot(q.y - pos.n)
    }).fold(f64::INFINITY, f64::min);
    let sams = [SysKind::Long, SysKind::Medium, SysKind::Short].iter().filter(|k| has(**k)).count();
    if cheap {
        if has(SysKind::Short) {
            ("short", "sea.engageShort", LayerChoice::Short)
        } else {
            ("medium", "sea.engageMedium", LayerChoice::Medium)
        }
    } else if nearest < st.p.g("close_threat_m") && sams >= 2 {
        ("unmask", "sea.unmask", LayerChoice::Unmask)
    } else if has(SysKind::Long) {
        ("long", "sea.engageLong", LayerChoice::Long)
    } else {
        ("medium", "sea.engageMedium", LayerChoice::Medium)
    }
}

/// A ship's defensive systems: (kind, munition, rounds) from its magazine.
fn ship_kinds(s: &Sim, e: Entity) -> Vec<(SysKind, String, u32)> {
    let Some(i) = s.world.get::<Ident>(e) else { return vec![] };
    let Some(c) = s.world.get::<ShipCombat>(e) else { return vec![] };
    if c.state.mission_kill {
        return vec![];
    }
    let mut out = Vec::new();
    for m in &i.platform.mounts {
        for mu in &m.munitions {
            let Some(x) = s.ctx.catalogue.munition(mu) else { continue };
            let rounds = c.magazine.get(mu).copied().unwrap_or(0);
            if rounds == 0 || out.iter().any(|o: &(SysKind, String, u32)| &o.1 == mu) {
                continue;
            }
            let kind = if x.kind == bgws_catalogue::MunitionKind::Sam {
                if x.max_range_m >= 80_000.0 {
                    SysKind::Long
                } else if x.max_range_m >= 15_000.0 {
                    SysKind::Medium
                } else {
                    SysKind::Short
                }
            } else if m.kind == WeaponKind::Ciws {
                SysKind::Ciws
            } else if matches!(m.kind, WeaponKind::Howitzer | WeaponKind::Cannon | WeaponKind::GunAd) && x.targets.contains(&bgws_catalogue::TargetDomain::Air) {
                SysKind::Gun
            } else {
                continue;
            };
            out.push((kind, mu.clone(), rounds));
        }
    }
    if i.domain == Domain::Surface {
        out.push((SysKind::Ew, "ew".into(), 1000));
    }
    out
}

fn allowed(layer: LayerChoice, kind: SysKind, has_medium: bool, conserve: bool) -> bool {
    if conserve && matches!(kind, SysKind::Long | SysKind::Medium) {
        return false;
    }
    match layer {
        LayerChoice::Long | LayerChoice::Unmask => true,
        LayerChoice::Medium => kind != SysKind::Long || !has_medium,
        LayerChoice::Short => !matches!(kind, SysKind::Long | SysKind::Medium),
        LayerChoice::Decoy => matches!(kind, SysKind::Ciws | SysKind::Gun | SysKind::Ew),
    }
}

/// Fight a ship's raid queue as one salvo against its (and its consorts')
/// layers with `engage_raid`; magazines deplete; each threat's fate is set.
pub(crate) fn engage(s: &mut Sim, st: &mut NavalState, ship: EntityId, layer: LayerChoice) {
    let now = s.ctx.now;
    let t = now.secs();
    let e = s.ctx.entity(ship);
    let Some(c) = s.world.get::<ShipCombat>(e).cloned() else { return };
    let pending: Vec<u64> = c.pending.iter().copied().filter(|p| st.inbound.get(p).map(|i| matches!(i.state, InboundState::Detected { .. })).unwrap_or(false)).collect();
    {
        let mut cm = s.world.get_mut::<ShipCombat>(e).expect("combat");
        cm.pending.clear();
        cm.pending_since = None;
        cm.asked = false;
        cm.layer = Some((layer, now));
        if layer == LayerChoice::Decoy {
            cm.decoy_until = now + SimDuration::from_secs(st.p.g("decoy_s"));
        }
    }
    if pending.is_empty() {
        return;
    }
    let f = s.ctx.figures.clone();
    let side = s.world.get::<Ident>(e).expect("ident").side;
    let pos = s.world.get::<Kin>(e).expect("kin").pos;
    // The ship and its consorts close enough to cover it.
    let mut shooters: Vec<Entity> = vec![e];
    let support = st.p.g("mutual_support_m");
    for &o in &s.ctx.order {
        if o == e || !alive(s, o) {
            continue;
        }
        let ok = s.world.get::<Ident>(o).map(|i| i.side == side).unwrap_or(false)
            && s.world.get::<Vessel>(o).map(|v| v.kind == VesselKind::Warship && !matches!(v.task, NavTask::InPort { .. })).unwrap_or(false)
            && s.world.get::<Kin>(o).map(|k| k.pos.sub(pos).horizontal() <= support).unwrap_or(false);
        if ok {
            shooters.push(o);
        }
    }
    let unmask = layer == LayerChoice::Unmask;
    let ciws_rpb = st.p.g("ciws_rounds_per_burst");
    let gun_rpb = st.p.g("gun5_rounds_per_burst");
    let mut systems: Vec<AdSystem> = Vec::new();
    let mut owners: Vec<(Entity, SysKind, String, f64)> = Vec::new();
    for &o in &shooters {
        let kinds = ship_kinds(s, o);
        let has_medium = kinds.iter().any(|k| k.0 == SysKind::Medium);
        let conserve = s.world.get::<ShipCombat>(o).map(|c| c.conserve).unwrap_or(false);
        let opos = v3(s.world.get::<Kin>(o).expect("kin").pos);
        let oid = s.world.get::<Ident>(o).expect("ident").id;
        let radar_h = s.world.get::<Vessel>(o).map(|v| v.radar_h.max(10.0)).unwrap_or(20.0);
        for (kind, mu, rounds) in kinds {
            if !allowed(layer, kind, has_medium, conserve) {
                continue;
            }
            let name = format!("{}:{}", label(s, oid), mu);
            let mut sys = match kind {
                SysKind::Long => AdSystem::from_figures("lrsam", &name, opos, &f),
                SysKind::Medium => AdSystem::from_figures("mrsam", &name, opos, &f),
                SysKind::Short => AdSystem::from_figures("shorad", &name, opos, &f),
                SysKind::Ciws => AdSystem::from_figures("gun", &name, opos, &f),
                SysKind::Ew => AdSystem::from_figures("ewSoftKill", &name, opos, &f),
                SysKind::Gun => {
                    let cost = s.ctx.catalogue.munition(&mu).map(|m| m.cost_usd).unwrap_or(2_000.0);
                    let mut pk = BTreeMap::new();
                    pk.insert(ThreatClass::OneWayDrone, st.p.g("gun5_pk_owa"));
                    for cl in [ThreatClass::Cruise, ThreatClass::SeaSkimmer] {
                        pk.insert(cl, st.p.g("gun5_pk_missile"));
                    }
                    AdSystem {
                        name: name.clone(),
                        layer: Layer::PointDefence,
                        position: opos,
                        sensor_height: radar_h,
                        min_range: 500.0,
                        max_range: st.p.g("gun5_range_m"),
                        pk,
                        cost: cost * gun_rpb,
                        rounds: 0,
                        cycle: st.p.g("gun5_cycle_s"),
                        interceptor_speed: s.ctx.catalogue.munition(&mu).map(|m| m.speed_ms).unwrap_or(700.0),
                        doctrine: Doctrine::ShootLookShoot,
                    }
                }
            };
            sys.sensor_height = radar_h;
            let per = match kind {
                SysKind::Ciws => ciws_rpb,
                SysKind::Gun => gun_rpb,
                _ => 1.0,
            };
            sys.rounds = if kind == SysKind::Ew { sys.rounds } else { (rounds as f64 / per).floor() as u32 };
            if unmask && kind.sam() {
                sys.cycle *= 0.5;
            }
            if sys.rounds == 0 {
                continue;
            }
            owners.push((o, kind, mu, per));
            systems.push(sys);
        }
    }
    // Missiles first with the standard layered doctrine; then the cheap
    // threats (drones, decoys) with the ship's cost-aware doctrine, which
    // holds a missile while its guns still have a fair chance [R6.2].
    let missile_chain = KillChain::from_figures(&f);
    let mut cheap_chain = missile_chain.clone();
    cheap_chain.defer_pk_ratio = st.p.g("cic_defer_pk_ratio");
    let value = st.p.g("threat_value_usd");
    let cut = st.p.g("cheap_threat_usd");
    let threats: Vec<Threat> = pending
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let th = &st.inbound[p];
            let cur = th.flight.position(t);
            let fl = Flight::new(th.flight.profile.clone(), V3::new(cur.x, cur.y, 0.0), th.flight.to, t);
            Threat { id: i as u32, class: th.class, flight: fl, cost: th.cost, value }
        })
        .collect();
    let start: Vec<u32> = systems.iter().map(|x| x.rounds).collect();
    let mut rng = s.ctx.rng("naval.raid", ship.0 as u64, st.raids.len() as u64);
    let mut slots: Vec<Option<bgws_domains::air_defence::ThreatOutcome>> = vec![None; threats.len()];
    let mut out = RaidOutcome { threats: vec![], shots_by_system: BTreeMap::new(), kills_by_system: BTreeMap::new(), rounds_left: BTreeMap::new(), interceptor_cost: 0.0, attacker_cost_destroyed: 0.0, value_leaked: 0.0 };
    let ew: Vec<usize> = (0..systems.len()).filter(|k| owners[*k].1 == SysKind::Ew).collect();
    let hard: Vec<usize> = (0..systems.len()).filter(|k| owners[*k].1 != SysKind::Ew).collect();
    let every: Vec<usize> = (0..systems.len()).collect();
    let missiles_idx: Vec<usize> = (0..threats.len()).filter(|i| threats[*i].cost >= cut).collect();
    let cheap_idx: Vec<usize> = (0..threats.len()).filter(|i| threats[*i].cost < cut).collect();
    // One stage: these systems against these threats; returns the threats
    // left alive (only killed ones are settled unless `last`).
    let mut stage = |sys_idx: &[usize], th_idx: &[usize], chain: &KillChain, last: bool, systems: &mut Vec<AdSystem>, rng: &mut rand_chacha::ChaCha8Rng| -> Vec<usize> {
        if th_idx.is_empty() {
            return vec![];
        }
        let sub: Vec<Threat> = th_idx.iter().enumerate().map(|(k, i)| Threat { id: k as u32, ..threats[*i].clone() }).collect();
        let mut sys: Vec<AdSystem> = sys_idx.iter().map(|k| systems[*k].clone()).collect();
        let o = engage_raid(&mut sys, &sub, chain, Assignment::Optimal, 1.0, rng);
        for (j, k) in sys_idx.iter().enumerate() {
            systems[*k].rounds = sys[j].rounds;
        }
        let mut left = Vec::new();
        for (k, i) in th_idx.iter().enumerate() {
            let mut x = o.threats[k].clone();
            x.id = *i as u32;
            if x.killed_by.is_some() || last {
                if let Some(prev) = &slots[*i] {
                    x.shots += prev.shots;
                }
                slots[*i] = Some(x);
            } else {
                // Keep its shots so far; it goes on to the next stage.
                slots[*i] = Some(x);
                left.push(*i);
            }
        }
        for (n, v) in o.shots_by_system {
            *out.shots_by_system.entry(n).or_insert(0) += v;
        }
        for (n, v) in o.kills_by_system {
            *out.kills_by_system.entry(n).or_insert(0) += v;
        }
        out.interceptor_cost += o.interceptor_cost;
        out.attacker_cost_destroyed += o.attacker_cost_destroyed;
        out.value_leaked += o.value_leaked;
        left
    };
    stage(&every, &missiles_idx, &missile_chain, true, &mut systems, &mut rng);
    // Soft kill first against the cheap threats, then the guns and missiles
    // at what is left (a free jammer would otherwise hold every gun back).
    let rest = if ew.is_empty() { cheap_idx.clone() } else { stage(&ew, &cheap_idx, &cheap_chain, false, &mut systems, &mut rng) };
    stage(&hard, &rest, &cheap_chain, true, &mut systems, &mut rng);
    out.rounds_left = systems.iter().map(|x| (x.name.clone(), x.rounds)).collect();
    out.threats = slots.into_iter().map(|x| x.expect("every threat is in one group")).collect();
    // Magazines deplete.
    let mut missiles = 0u32;
    let mut gun_rounds = 0u32;
    for (k, sys) in systems.iter().enumerate() {
        let used = start[k] - sys.rounds;
        let (o, kind, mu, per) = &owners[k];
        if *kind == SysKind::Ew || used == 0 {
            continue;
        }
        let rounds = (used as f64 * per).round() as u32;
        if kind.sam() {
            missiles += used;
        } else {
            gun_rounds += rounds;
        }
        if let Some(mut cm) = s.world.get_mut::<ShipCombat>(*o) {
            let m = cm.magazine.entry(mu.clone()).or_insert(0);
            *m = m.saturating_sub(rounds);
        }
    }
    let engaged = out.threats.iter().filter(|x| x.shots > 0).count() as u32;
    for (i, p) in pending.iter().enumerate() {
        let o = &out.threats[i];
        let th = st.inbound.get_mut(p).expect("pending");
        th.state = InboundState::Engaged;
        th.fate = Some(match (&o.killed_by, o.killed_at) {
            (Some(by), Some(at)) => Fate::Killed { at, by: by.clone() },
            _ => Fate::Leaks,
        });
    }
    st.stats.missiles += missiles;
    st.stats.engagements += engaged;
    st.stats.gun_rounds += gun_rounds;
    st.stats.interceptor_cost += out.interceptor_cost;
    for (k, n) in &out.kills_by_system {
        *st.stats.kills_by_system.entry(k.clone()).or_insert(0) += n;
    }
    let killed = out.threats.iter().filter(|x| x.killed_by.is_some()).count();
    let text = format!(
        "{} fights a raid of {} with the {:?} layer: {missiles} missiles, {gun_rounds} gun rounds; {killed} to be killed, {} to leak",
        label(s, ship),
        threats.len(),
        layer,
        threats.len() - killed
    );
    s.ctx.log.push(now, "naval.raid").side(side).entity(ship).text(text).data(serde_json::json!({
        "layer": layer, "threats": threats.len(), "missiles": missiles, "engaged": engaged, "gunRounds": gun_rounds,
        "shots": out.shots_by_system, "kills": out.kills_by_system, "interceptorCost": out.interceptor_cost, "costExchange": out.cost_exchange(),
    }));
    st.raids.push(RaidRecord { at: now, ship, layer, threats: threats.len(), missiles, engaged, gun_rounds, outcome: out });
}

/// USVs run in on their targets and detonate alongside [R6.3] (base rate).
pub(crate) fn usv_runs(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let hit_m = st.p.g("usv_hit_m");
    for &e in &s.ctx.order.clone() {
        let Some(v) = s.world.get::<Vessel>(e).cloned() else { continue };
        if v.kind != VesselKind::Usv || !alive(s, e) {
            continue;
        }
        let NavTask::Strike { target, aim } = v.task.clone() else { continue };
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side)
        };
        let pos = s.world.get::<Kin>(e).expect("kin").pos;
        let target = target.filter(|t| alive(s, s.ctx.entity(*t))).or_else(|| {
            // The operator picks the nearest enemy ship in sight of its camera.
            s.ctx
                .order
                .iter()
                .filter_map(|&o| {
                    let i = s.world.get::<Ident>(o)?;
                    let d = s.world.get::<Kin>(o)?.pos.sub(pos).horizontal();
                    (alive(s, o) && i.side != side && i.side != Side::Neutral && i.domain == Domain::Surface && s.world.get::<ShipCombat>(o).is_some() && d < 30_000.0).then_some((d, i.id))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|x| x.1)
        });
        if let Some(mut vm) = s.world.get_mut::<Vessel>(e) {
            vm.task = NavTask::Strike { target, aim };
        }
        match target {
            Some(t) => {
                let tp = s.world.get::<Kin>(s.ctx.entity(t)).expect("kin").pos;
                if tp.sub(pos).horizontal() <= hit_m {
                    destroy(s, id);
                    let text = format!("{} strikes {}", label(s, id), label(s, t));
                    s.ctx.log.push(s.ctx.now, "naval.usvHit").side(side).entity(id).target(t).text(text);
                    let w = st.p.g("usv_warhead");
                    apply_hit(s, &mut st, t, w, Some(id), "an explosive USV");
                }
            }
            None => {
                if aim.sub(pos).horizontal() <= hit_m {
                    destroy(s, id);
                    let text = format!("{} explodes short of any target", label(s, id));
                    s.ctx.log.push(s.ctx.now, "naval.usvMiss").side(side).entity(id).text(text);
                }
            }
        }
    }
    put(s, st);
}

/// Guns against small boats (slow rate): a swarm is put to the ship
/// (usvSwarm); once ordered, guns and helicopter work boats in reach; close
/// in, the CIWS fires on its own.
pub(crate) fn usv_guns(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(c) = s.world.get::<ShipCombat>(e).cloned() else { continue };
        let Some(v) = s.world.get::<Vessel>(e).cloned() else { continue };
        if v.kind != VesselKind::Warship || !alive(s, e) {
            continue;
        }
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side)
        };
        let pos = s.world.get::<Kin>(e).expect("kin").pos;
        let boats: Vec<(EntityId, f64)> = c
            .contacts
            .iter()
            .filter(|(_, t)| (now - **t).secs() <= 30.0)
            .filter_map(|(b, _)| {
                let be = s.ctx.entity(*b);
                let i = s.world.get::<Ident>(be)?;
                let bv = s.world.get::<Vessel>(be)?;
                let d = s.world.get::<Kin>(be)?.pos.sub(pos).horizontal();
                (alive(s, be) && i.side != side && bv.kind == VesselKind::Usv && d <= st.p.g("swarm_m")).then_some((*b, d))
            })
            .collect();
        if boats.len() as f64 >= st.p.g("swarm_size") && !st.recently_asked((id, "usvSwarm", None), now, 120.0) {
            st.asked.insert((id, "usvSwarm", None), now);
            let text = format!("{} sees {} explosive boats closing", label(s, id), boats.len());
            s.ctx.log.push(now, "naval.usvSwarm").side(side).entity(id).text(text);
            put(s, st);
            ask_or_do(s, id, "usvSwarm", boats.first().map(|b| b.0), "guns", "sea.gunsAndHelo");
            st = take(s).expect("state");
        }
        let guns = s.world.get::<ShipCombat>(e).map(|c| c.guns_until > now).unwrap_or(false) && !c.state.mission_kill;
        for (b, d) in boats {
            let reach = if guns { st.p.g("gun_usv_range_m") } else { st.p.g("ciws_usv_range_m") };
            if d > reach {
                continue;
            }
            let mut rng = s.ctx.rng("naval.guns", id.0 as u64, b.0 as u64);
            if rng.gen::<f64>() < st.p.g("gun_usv_pk") {
                destroy(s, b);
                let text = format!("{}'s guns sink {} at {:.1} km", label(s, id), label(s, b), d / 1000.0);
                s.ctx.log.push(now, "naval.usvKilled").side(side).entity(id).target(b).text(text);
            }
        }
    }
    put(s, st);
}

/// A torpedo into the water: the target hears it (counterDetected for a
/// submarine; a ship turns away).
pub(crate) fn launch_torpedo(s: &mut Sim, st: &mut NavalState, shooter: EntityId, target: EntityId, munition: &str) -> bool {
    let Some(mun) = s.ctx.catalogue.munition(munition).cloned() else { return false };
    let se = s.ctx.entity(shooter);
    let te = s.ctx.entity(target);
    {
        let Some(mut c) = s.world.get_mut::<ShipCombat>(se) else { return false };
        let Some(r) = c.magazine.get_mut(munition) else { return false };
        if *r == 0 {
            return false;
        }
        *r -= 1;
    }
    let range = s.world.get::<Kin>(te).expect("kin").pos.sub(s.world.get::<Kin>(se).expect("kin").pos).horizontal();
    let run = range / mun.speed_ms.max(5.0);
    let id = st.next_key();
    let side = s.world.get::<Ident>(se).expect("ident").side;
    st.torpedoes.insert(id, Torpedo { id, shooter, side, target, pk: mun.pk, warhead: mun.warhead_kg / st.p.g("ref_warhead_kg") * st.p.g("torpedo_under_keel") });
    s.ctx.queue.schedule(s.ctx.now + SimDuration::from_secs(run), Priority::Physical, Scheduled::Timer { system: "naval.torpedo".into(), key: id });
    let text = format!("{} fires a torpedo at {} ({:.1} km, {:.0} s run)", label(s, shooter), label(s, target), range / 1000.0, run);
    s.ctx.log.push(s.ctx.now, "naval.torpedo").side(side).entity(shooter).target(target).text(text);
    let sprint = st.p.g("sprint_s");
    if let Some(mut v) = s.world.get_mut::<Vessel>(te) {
        v.evading_until = s.ctx.now + SimDuration::from_secs(sprint.min(run));
    }
    if s.world.get::<Ident>(te).map(|i| i.domain == Domain::Subsurface).unwrap_or(false) {
        st.deferred.push((target, "counterDetected", Some(shooter), "evade", "sea.evade"));
    }
    true
}

/// The torpedo reaches its target: a hit unless it evaded.
pub(crate) fn torpedo_arrives(s: &mut Sim, key: u64) {
    let Some(mut st) = take(s) else { return };
    if let Some(tp) = st.torpedoes.remove(&key) {
        let te = s.ctx.entity(tp.target);
        if alive(s, te) {
            let evading = s.world.get::<Vessel>(te).map(|v| v.evading_until > s.ctx.now).unwrap_or(false);
            let p = tp.pk * if evading { st.p.g("evade_factor") } else { 1.0 };
            let mut rng = s.ctx.rng("naval.torpedo", key, 0);
            if rng.gen::<f64>() < p {
                let text = format!("{}'s torpedo hits {}", label(s, tp.shooter), label(s, tp.target));
                s.ctx.log.push(s.ctx.now, "naval.hit").side(tp.side).entity(tp.shooter).target(tp.target).text(text);
                apply_hit(s, &mut st, tp.target, tp.warhead, Some(tp.shooter), "a torpedo");
            } else {
                let text = format!("{}'s torpedo misses {}{}", label(s, tp.shooter), label(s, tp.target), if evading { " (evaded)" } else { "" });
                s.ctx.log.push(s.ctx.now, "naval.miss").side(tp.side).entity(tp.shooter).target(tp.target).text(text);
            }
        }
    }
    put(s, st);
}
