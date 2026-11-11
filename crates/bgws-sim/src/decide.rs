//! Decision points (§10.1): triggers raised by the systems become questions
//! through the unit's behaviour template, with facts from its own state and
//! hard gates applied before options are offered. Drills and single options
//! apply without asking. Answers take effect after the agent's reaction time,
//! through the action library below.

use crate::components::*;
use crate::hq::Plan;
use crate::sim::{Pending, Sim, Trigger};
use crate::systems::{alive, name, threat_bearing};
use bgws_behaviour::{build_question, needs_asking, Capability, Cohesion, FactValue, Gates, QuestionContext, QuestionLimits, Roe, RulesChoice, WeaponRelease};
use bgws_catalogue::Domain;
use bgws_core::{Answer, AnswerSource, Enu, EntityId, QuestionId, QuestionKind, Side, SimDuration};
use bgws_models::people::{reaction_median_s, sample_reaction_s, Drill, MoraleState};
use std::collections::{BTreeMap, BTreeSet};

/// Priority among decision points when several fire at once (as the TS engine).
const PRIORITY: [&str; 13] = ["D0", "D2", "D3", "D4", "D12", "D5", "D9", "D1", "D6", "D8", "D7", "D10", "D11"];

fn rank(p: &str) -> usize {
    PRIORITY.iter().position(|x| *x == p).unwrap_or(PRIORITY.len())
}

/// Turn this tick's triggers into questions (or immediate drills).
pub fn raise(s: &mut Sim) {
    let mut triggers = std::mem::take(&mut s.ctx.triggers);
    triggers.sort_by(|a, b| a.entity.cmp(&b.entity).then(rank(a.point).cmp(&rank(b.point))).then(a.about.cmp(&b.about)));
    let mut done: BTreeSet<EntityId> = BTreeSet::new();
    for t in triggers {
        if done.contains(&t.entity) {
            continue;
        }
        let e = s.ctx.entity(t.entity);
        // Another simulation decides for its entities.
        if !alive(s, e) || crate::remote::is_remote(s, e) || s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(true) {
            continue;
        }
        if let Some(last) = s.ctx.last_asked.get(&t.entity) {
            if (s.ctx.now - *last).secs() < s.ctx.p.cooldown_s && t.point != "D2" && t.point != "D0" {
                continue;
            }
        }
        // Its leader just hit: it carries on until the second takes over.
        if s.world.get::<Crew>(e).is_some_and(|c| c.leaderless_until.is_some()) && t.point != "D11" {
            continue;
        }
        // A broken unit runs itself.
        let morale = s.world.get::<Crew>(e).map(|c| c.morale.state).unwrap_or(MoraleState::Steady);
        if matches!(morale, MoraleState::Broken | MoraleState::Routed) && t.point != "D11" {
            continue;
        }
        let rule = doctrine_rule(s, e, &t);
        if ask_with_rule(s, &t, rule) {
            done.insert(t.entity);
        }
    }
}

/// Cover a unit counts as being in, not in the open.
const IN_COVER: f64 = 0.3;
/// A shooter fired on it this recently, s: under fire.
const UNDER_FIRE_S: f64 = 30.0;

/// The doctrine's drills at the points where they differ from the
/// template's fallback (which is to carry on):
/// - D3, fired on by an unseen shooter (react to contact): in the open, get
///   into cover from the bearing; in cover, search along it.
/// - D2, fired on by a shooter it has located: return fire; but a vehicle in
///   the open goes hull-down first, and a shooter beyond all its weapons'
///   reach is left by pulling out of its sight.
/// - D6, after a volley: a unit holding ground, under fire from a shooter it
///   has seen, fires and moves to another position (its alternate) before
///   the enemy's fire is adjusted onto it.
/// - D11, rallied: back to its orders if no longer under fire, else hold
///   and recover.
fn doctrine_rule(s: &Sim, e: bevy_ecs::prelude::Entity, t: &Trigger) -> RulesChoice {
    let (Some(k), Some(a), Some(c), Some(i)) = (s.world.get::<Kin>(e), s.world.get::<Act>(e), s.world.get::<Crew>(e), s.world.get::<Ident>(e)) else { return RulesChoice::Fallback };
    let now = s.ctx.now;
    let cover = s.ctx.ground.micro(k.pos.e, k.pos.n).cover;
    let in_cover = cover >= IN_COVER || a.posture == Posture::HullDown;
    let option = |o: &str| RulesChoice::Option(o.into());
    match t.point {
        "D3" => option(if in_cover { "search" } else { "cover" }),
        "D2" => {
            let Some(shooter) = t.about.and_then(|x| s.world.get::<Aware>(e)?.seen.get(&x).map(|v| v.at)) else { return option("returnFire") };
            let range = shooter.sub(k.pos).horizontal();
            let reach = s.world.get::<Arms>(e).map(|ar| ar.mounts.iter().filter(|m| m.rounds > 0 || m.stock.values().any(|n| *n > 0)).filter_map(|m| i.platform.mounts.get(m.index)).flat_map(|m| m.munitions.iter()).filter_map(|m| s.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max)).unwrap_or(0.0);
            let vehicle = !matches!(i.platform.body.vulnerability, bgws_catalogue::Vulnerability::Personnel);
            if range > reach {
                option("pullBack")
            } else if vehicle && !in_cover {
                option("hullDown")
            } else {
                option("returnFire")
            }
        }
        "D6" => {
            let holding = !a.activity.moving() && !a.mission.phases.iter().skip(a.mission.phase).any(|p| p.moving());
            let located_fire = c.last_incoming.is_some_and(|(at, by)| (now - at).secs() <= UNDER_FIRE_S && s.world.get::<Aware>(e).is_some_and(|w| w.seen.contains_key(&by)));
            if holding && located_fire {
                option("fireAndMove")
            } else {
                RulesChoice::Fallback
            }
        }
        "D11" => {
            // Rallied: back to its orders once no longer under fire (holding
            // to recover would drop them for good); under fire, hold.
            let quiet = c.last_incoming.is_none_or(|(at, _)| (now - at).secs() > 2.0 * UNDER_FIRE_S);
            option(if quiet { "resume" } else { "hold" })
        }
        _ => RulesChoice::Fallback,
    }
}

fn facts(s: &Sim, e: bevy_ecs::prelude::Entity, about: Option<EntityId>) -> BTreeMap<String, FactValue> {
    let i = s.world.get::<Ident>(e).expect("i");
    let h = s.world.get::<Health>(e).expect("h");
    let c = s.world.get::<Crew>(e).expect("c");
    let a = s.world.get::<Act>(e).expect("a");
    let k = s.world.get::<Kin>(e).expect("k");
    let aw = s.world.get::<Aware>(e).expect("aw");
    let class = match i.platform.body.vulnerability {
        bgws_catalogue::Vulnerability::Armoured => "armoured_vehicle",
        bgws_catalogue::Vulnerability::Personnel => "foot",
        _ => "soft_skin",
    };
    let cohesion = match c.morale.state {
        MoraleState::Steady => "steady",
        MoraleState::Shaken | MoraleState::Pinned => "shaken",
        _ => "broken",
    };
    let enemy = about.and_then(|t| aw.seen.get(&t)).or_else(|| aw.seen.values().min_by(|x, y| x.at.sub(k.pos).horizontal().total_cmp(&y.at.sub(k.pos).horizontal())));
    let mut m = BTreeMap::new();
    m.insert("self.class".into(), FactValue::Text(class.into()));
    m.insert("self.strength".into(), FactValue::Num((h.strength() * 10.0).round()));
    m.insert("self.cohesion".into(), FactValue::Text(cohesion.into()));
    m.insert("self.inCover".into(), FactValue::Bool(s.ctx.ground.micro(k.pos.e, k.pos.n).cover > 0.3 || a.posture == Posture::HullDown));
    let exposure = s.ctx.ground.exposure(k.pos.e, k.pos.n);
    m.insert("self.deadGround".into(), exposure.map(|x| FactValue::Bool(x < 0.1)).unwrap_or(FactValue::Null));
    m.insert("self.onKeyTerrain".into(), FactValue::Bool(s.ctx.ground.key_terrain_near(k.pos.e, k.pos.n, 300.0).iter().any(|t| t.kind == bgws_env::KeyKind::Observation)));
    m.insert("enemy.known".into(), FactValue::Bool(enemy.is_some()));
    m.insert("enemy.rangeM".into(), enemy.map(|v| FactValue::Num(v.at.sub(k.pos).horizontal().round())).unwrap_or(FactValue::Null));
    m.insert("orders.urgent".into(), FactValue::Bool(a.mission.urgent));
    // Its orders still lead somewhere: a move it has not finished, now or in a later step.
    m.insert("orders.moveLeft".into(), FactValue::Bool(heading_for(a).is_some_and(|g| g.sub(k.pos).horizontal() > MOVE_LEFT_M)));
    // World facts default to null for land units; the world layer fills its own.
    for f in bgws_behaviour::facts::world_facts() {
        m.entry(f.name.to_string()).or_insert(FactValue::Null);
    }
    crate::world::facts(s, e, &mut m);
    m
}

fn gates(s: &Sim, e: bevy_ecs::prelude::Entity, about: Option<EntityId>) -> Gates {
    let h = s.world.get::<Health>(e).expect("h");
    let c = s.world.get::<Crew>(e).expect("c");
    let a = s.world.get::<Act>(e).expect("a");
    let arms = s.world.get::<Arms>(e).expect("arms");
    let mut caps = BTreeSet::new();
    if h.can_fire() && arms.rounds() > 0 {
        caps.insert(Capability::Weapon);
    }
    if h.can_move() {
        caps.insert(Capability::Mobility);
    }
    caps.insert(Capability::Radio);
    if s.world.get::<crate::dismount::Troops>(e).is_some_and(|t| t.aboard(h) > 0 && t.leaving.is_none() && t.to_cover.is_none()) {
        caps.insert(Capability::Dismounts);
    }
    crate::world::capabilities(s, e, &mut caps);
    let mut g = Gates {
        roe: match a.roe {
            RoeS::Never => Roe::Never,
            RoeS::IfFiredUpon => Roe::IfFiredUpon,
            RoeS::WithinShortRange => Roe::WithinShortRange,
            RoeS::Always => Roe::Always,
        },
        fired_upon: c.last_incoming.is_some(),
        weapon_release: WeaponRelease::Free,
        target_identified: true,
        emcon: crate::domains::drones::emcon_gate(s, e),
        capabilities: Some(caps),
        cohesion: match c.morale.state {
            MoraleState::Steady => Cohesion::Steady,
            MoraleState::Shaken | MoraleState::Pinned => Cohesion::Shaken,
            _ => Cohesion::Broken,
        },
    };
    crate::domains::air::gates(s, e, about, &mut g);
    g
}

/// The unit's situation in words: only what it has seen, heard and been told.
pub(crate) fn situation(s: &Sim, e: bevy_ecs::prelude::Entity, about: Option<EntityId>) -> String {
    let i = s.world.get::<Ident>(e).expect("i");
    let h = s.world.get::<Health>(e).expect("h");
    let k = s.world.get::<Kin>(e).expect("k");
    let a = s.world.get::<Act>(e).expect("a");
    let c = s.world.get::<Crew>(e).expect("c");
    let aw = s.world.get::<Aware>(e).expect("aw");
    let mut out = format!(
        "{} ({}), {} of {} fit, {}, {}. Suppression {:.0}%. Mission: {}.",
        i.label,
        i.platform.name,
        h.fit,
        h.start,
        a.activity.word(),
        match a.posture {
            Posture::Moving => "on the move",
            Posture::Halted => "halted",
            Posture::Settled => "in position",
            Posture::HullDown => "hull-down in cover",
        },
        c.suppression.level * 100.0,
        if a.mission.purpose.is_empty() { "none" } else { &a.mission.purpose }
    );
    let mut seen: Vec<(&EntityId, &Seen)> = aw.seen.iter().collect();
    seen.sort_by(|x, y| x.1.at.sub(k.pos).horizontal().total_cmp(&y.1.at.sub(k.pos).horizontal()).then(x.0.cmp(y.0)));
    for (id, v) in seen.iter().take(5) {
        let d = v.at.sub(k.pos);
        let what = match v.level {
            0 => "something".to_string(),
            1 => "a vehicle or team".to_string(),
            _ => s.world.get::<Ident>(s.ctx.entity(**id)).map(|x| x.platform.name.clone()).unwrap_or_default(),
        };
        out.push_str(&format!(" Sees {} {:.0} m to the {}{}{}.", what, d.horizontal(), compass(d.bearing()), if v.moving { ", moving" } else { "" }, if Some(**id) == about { " (this decision is about it)" } else { "" }));
    }
    for v in aw.suspects.values() {
        out.push_str(&format!(" Fired on from the {}, shooter not seen.", compass(v.bearing)));
    }
    out
}

/// Where a move leads: the end of the current move, else of the next one in its orders.
/// A friend's report is of a contact this unit sees when it lies this close (or within twice its error), m.
const REPORT_MATCH_M: f64 = 300.0;
/// Reported contacts this far from the unit are listed, m.
const REPORT_REACH_M: f64 = 4_000.0;
/// A report's view of what a contact is counts at this belief or more.
const REPORT_SURE: f64 = 0.6;

/// What a contact is, in words, as its side's picture has it.
fn hypothesis_words(h: bgws_know::Hypothesis) -> &'static str {
    use bgws_know::Hypothesis as H;
    match h {
        H::Mbt => "a tank",
        H::Ifv => "an infantry fighting vehicle",
        H::Apc => "an armoured personnel carrier",
        H::Truck => "a truck",
        H::Artillery => "artillery",
        H::Infantry => "infantry or an anti-tank team",
        H::LandDecoy => "a decoy",
        _ => "something not on the ground",
    }
}

/// What recognising a contact (short of identifying it) tells: its kind.
fn recognised_as(s: &Sim, id: EntityId) -> &'static str {
    match s.world.get::<Ident>(s.ctx.entity(id)).map(|i| i.platform.body.vulnerability) {
        Some(bgws_catalogue::Vulnerability::Personnel) => "a team on foot",
        Some(bgws_catalogue::Vulnerability::Armoured) => "an armoured vehicle",
        Some(_) => "a soft-skinned vehicle",
        None => "a vehicle or team",
    }
}

/// A contact this close to where a unit's orders end is on its objective, m.
const OBJECTIVE_NEAR_M: f64 = 600.0;
/// A tactical move's pace as a share of the vehicle's cross-country speed (for the ETA).
const ETA_PACE: f64 = 0.6;

/// Farther than this from where its orders lead, a unit still has a move to make.
pub(crate) const MOVE_LEFT_M: f64 = 150.0;

pub(crate) fn heading_for(a: &Act) -> Option<Enu> {
    let end = |x: &Activity| match x {
        Activity::Move { route, .. } | Activity::Withdraw { route, .. } => route.last().copied(),
        _ => None,
    };
    end(&a.activity).or_else(|| a.mission.phases.iter().skip(a.mission.phase).find_map(end))
}

/// The farthest any of a platform's direct weapons reaches, m (0 none).
fn reach_m(s: &Sim, p: &bgws_catalogue::Platform) -> f64 {
    p.mounts.iter().flat_map(|m| m.munitions.iter()).filter_map(|m| s.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max)
}

/// What firing on a target `range` m off would do, by the engine's own sums:
/// the weapon it would fire (its first direct mount with rounds, as direct
/// fire picks), the chance one volley hits a target presenting `pres`
/// (bias averaged out), and that round's penetration at the range, mm, and
/// whether it is a shaped charge (against chemical-energy armour) or not.
fn volley_odds(s: &Sim, e: bevy_ecs::prelude::Entity, range: f64, pres: &bgws_models::direct_fire::Presented) -> Option<(String, f64, f64, bool)> {
    let (i, h, a, c, arms) = (s.world.get::<Ident>(e)?, s.world.get::<Health>(e)?, s.world.get::<Act>(e)?, s.world.get::<Crew>(e)?, s.world.get::<Arms>(e)?);
    // The mount the engine would choose: of those that reach, the most likely to hit.
    let mut best: Option<(String, f64, f64, bool)> = None;
    // Each round a mount holds is a choice (the loaded one and its stock).
    let choices = arms.mounts.iter().filter(|m| !crate::domains::fires::indirect_mount(&i.platform, m.index)).flat_map(|m| {
        std::iter::once(m.clone()).chain(m.stock.iter().map(|(mu, n)| MountState { munition: mu.clone(), rounds: *n, stock: Default::default(), ..m.clone() }))
    });
    for m in choices.filter(|m| m.rounds > 0) {
        if let Some(o) = mount_odds(s, i, h, a, c, &m, range, pres) {
            if best.as_ref().is_none_or(|b| o.1 > b.1) {
                best = Some(o);
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn mount_odds(s: &Sim, i: &Ident, h: &Health, a: &Act, c: &Crew, m: &MountState, range: f64, pres: &bgws_models::direct_fire::Presented) -> Option<(String, f64, f64, bool)> {
    use bgws_models::direct_fire as df;
    let mun = s.ctx.catalogue.munition(&m.munition)?;
    if range > mun.max_range_m || range < mun.min_range_m {
        return None;
    }
    let mount = i.platform.mounts.get(m.index)?;
    let p = &s.ctx.p.direct;
    let fc = df::FireControl::from_quality(mount.fire_control.quality, p);
    let gun = if mun.kind == bgws_catalogue::MunitionKind::SmallArms { p.rifle } else { df::Gun { muzzle_velocity_mps: mun.speed_ms.max(100.0), ..p.mbt_gun } };
    let moving = a.posture == Posture::Moving && !mount.fire_control.stabilised;
    let shot = df::Shot { range_m: range, firer_moving: moving, target_crossing_mps: 0.0, skill_mult: c.skill.error_mult(&s.ctx.p.people), suppression: c.suppression.level };
    let ph = if crate::systems::is_guided_direct(mun) {
        df::guided_ph(crate::systems::operator_guided(mun), c.suppression.level, p) * (1.0 - s.ctx.p.missile_fail_p)
    } else {
        df::ph_expected(pres, &df::error_budget(&gun, &fc, &shot, p))
    };
    let rounds = ((h.fit as f64 * s.ctx.p.rounds_per_volley).round().max(1.0)).min(m.rounds as f64);
    let chemical = matches!(mun.kind, bgws_catalogue::MunitionKind::Heat | bgws_catalogue::MunitionKind::Atgm);
    Some((mount.id.clone(), 1.0 - (1.0 - ph).powf(rounds), mun.penetration_at(range), chemical))
}

/// What a land unit believes, as named fields for a decision model: the
/// same beliefs as [`situation`] (only what reached it), with what the
/// words leave out: its weapons against the ranges, what each enemy it
/// has identified can reach, its orders and where they lead, its ground
/// (cover, dead ground, a town block, key terrain) and its friends.
pub(crate) fn unit_state(s: &Sim, e: bevy_ecs::prelude::Entity, about: Option<EntityId>) -> serde_json::Value {
    use serde_json::json;
    let i = s.world.get::<Ident>(e).expect("i");
    let h = s.world.get::<Health>(e).expect("h");
    let k = s.world.get::<Kin>(e).expect("k");
    let a = s.world.get::<Act>(e).expect("a");
    let c = s.world.get::<Crew>(e).expect("c");
    let aw = s.world.get::<Aware>(e).expect("aw");
    let round = |x: f64| x.round();
    let pct = |x: f64| (x * 100.0).round();
    let posture = match a.posture {
        Posture::Moving => "moving",
        Posture::Halted => "halted",
        Posture::Settled => "in position",
        Posture::HullDown => "hull-down in cover",
    };
    let morale = match c.morale.state {
        MoraleState::Steady => "steady",
        MoraleState::Shaken => "shaken",
        MoraleState::Pinned => "pinned",
        _ => "broken",
    };
    // Its weapons and what they have left.
    let weapons: Vec<serde_json::Value> = s
        .world
        .get::<Arms>(e)
        .map(|ar| {
            ar.mounts
                .iter()
                .filter_map(|m| {
                    let mount = i.platform.mounts.get(m.index)?;
                    let mun = s.ctx.catalogue.munition(&m.munition);
                    let salvo = mount.salvo.max(1);
                    Some(json!({
                        "weapon": mount.id, "munition": m.munition,
                        "maxRangeM": mun.map(|x| round(x.max_range_m)), "minRangeM": mun.map(|x| round(x.min_range_m)).filter(|x| *x > 0.0),
                        "rounds": m.rounds, "volleysLeft": m.rounds / salvo,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let my_reach = s.world.get::<Arms>(e).map(|ar| ar.mounts.iter().filter(|m| m.rounds > 0).filter_map(|m| s.ctx.catalogue.munition(&m.munition)).map(|m| m.max_range_m).fold(0.0, f64::max)).unwrap_or(0.0);
    // Where its orders end: the last point any of its moves leads to.
    let objective = a.mission.phases.iter().rev().find_map(|x| match x {
        Activity::Move { route, .. } | Activity::Withdraw { route, .. } => route.last().copied(),
        _ => None,
    });
    // Friends already firing on each enemy.
    let mut engaged: BTreeMap<EntityId, u32> = BTreeMap::new();
    for &f in &s.ctx.order {
        if f == e || !alive(s, f) || s.world.get::<Ident>(f).is_none_or(|fi| fi.side != i.side) {
            continue;
        }
        if let Some(Activity::Engage { target, .. }) = s.world.get::<Act>(f).map(|x| &x.activity) {
            *engaged.entry(*target).or_default() += 1;
        }
    }
    // What a volley of ours would do, against a vehicle or a team in cover.
    let dp = &s.ctx.p.direct;
    let pct0 = |x: f64| (x * 100.0).round();
    let vehicle = |(length_m, width_m, height_m): (f64, f64, f64), hull_down: bool| {
        let dims = bgws_models::direct_fire::VehicleDims { length_m: length_m.max(3.0), width_m: width_m.max(2.0), hull_height_m: (height_m * 0.6).max(1.0), turret_height_m: (height_m * 0.4).max(0.3) };
        bgws_models::direct_fire::presented_vehicle(&dims, 0.0, if hull_down { bgws_models::direct_fire::VehiclePosture::HullDown } else { bgws_models::direct_fire::VehiclePosture::Exposed }, dp)
    };
    // An unidentified contact: a typical vehicle (a tank's size), or a team dug in.
    let typical = (7.0, 3.5, 2.4);
    let team = bgws_models::direct_fire::presented_person(bgws_models::direct_fire::PersonPosture::Prone, 0.5, dp);
    // Enemies it has seen itself, nearest first.
    let mut seen: Vec<(&EntityId, &Seen)> = aw.seen.iter().collect();
    seen.sort_by(|x, y| x.1.at.sub(k.pos).horizontal().total_cmp(&y.1.at.sub(k.pos).horizontal()).then(x.0.cmp(y.0)));
    let enemies: Vec<serde_json::Value> = seen
        .iter()
        .take(6)
        .map(|(id, v)| {
            let d = v.at.sub(k.pos);
            let range = d.horizontal();
            let known = (v.level >= 2).then(|| s.world.get::<Ident>(s.ctx.entity(**id))).flatten();
            let their_reach = known.map(|x| reach_m(s, &x.platform)).filter(|r| *r > 0.0);
            // Our volley against it: against what it is, when identified; else both ways.
            let odds = match known {
                Some(x) if x.platform.body.vulnerability == bgws_catalogue::Vulnerability::Personnel => volley_odds(s, e, range, &team).map(|(w, p, _, _)| json!({ "weapon": w, "volleyHitPct": pct0(p) })),
                Some(x) => volley_odds(s, e, range, &vehicle((x.platform.body.length_m, x.platform.body.width_m, x.platform.body.height_m), !v.moving)).map(|(w, p, pen, chemical)| {
                    let z = crate::systems::armour_zones(&x.platform);
                    let front = if chemical { z.ce_mm[0] } else { z.ke_mm[0] };
                    json!({ "weapon": w, "volleyHitPct": pct0(p), "penetratesFront": pen > front })
                }),
                None => {
                    let as_vehicle = volley_odds(s, e, range, &vehicle(typical, !v.moving));
                    let as_team = volley_odds(s, e, range, &team);
                    as_vehicle.zip(as_team).map(|((w, pv, _, _), (_, pt, _, _))| json!({ "weapon": w, "volleyHitPctIfVehicle": pct0(pv), "volleyHitPctIfTeamInCover": pct0(pt) }))
                }
            };
            let from_objective = objective.map(|o| v.at.sub(o).horizontal());
            json!({
                "what": match v.level { 0 => "something".to_string(), 1 => recognised_as(s, **id).to_string(), _ => known.map(|x| x.platform.name.clone()).unwrap_or_default() },
                "identified": v.level >= 2,
                "rangeM": round(range), "bearing": compass(d.bearing()),
                "moving": v.moving, "seenAgoS": round((s.ctx.now - v.last).secs()),
                "inOurReach": my_reach > 0.0 && range <= my_reach,
                "itsReachM": their_reach.map(round),
                "canReachUs": their_reach.map(|r| range <= r),
                "thisDecisionIsAboutIt": Some(**id) == about,
                "ifWeFire": odds,
                "onOurObjective": from_objective.map(|d| d <= OBJECTIVE_NEAR_M),
                "fromOurObjectiveM": from_objective.map(round),
                "friendsFiringOnIt": engaged.get(id).copied().filter(|n| *n > 0),
            })
        })
        .collect();
    // What friends have reported (the unit's share of its side's picture):
    // what it makes of the contacts this unit sees but has not identified,
    // and the contacts it cannot see itself.
    let shared = crate::domains::drones::c2::picture_for(s, i.id);
    let near_track = |at: Enu| shared.iter().filter(|t| t.pos.sub(at).horizontal() <= (2.0 * t.error_m).max(REPORT_MATCH_M)).min_by(|a, b| a.pos.sub(at).horizontal().total_cmp(&b.pos.sub(at).horizontal()));
    let reported_as = |at: Enu| {
        near_track(at).and_then(|t| t.class.filter(|c| c.1 >= REPORT_SURE).map(|c| json!({ "what": hypothesis_words(c.0), "surePct": pct(c.1), "ageS": round(t.age_s), "by": t.by.map(|b| name(s, b)) })))
    };
    let enemies: Vec<serde_json::Value> = enemies
        .into_iter()
        .zip(seen.iter().take(6))
        .map(|(mut j, (_, v))| {
            if j.get("identified").and_then(serde_json::Value::as_bool) != Some(true) {
                if let Some(r) = reported_as(v.at) {
                    j["reportedAs"] = r;
                }
            }
            j
        })
        .collect();
    let mut beyond: Vec<(f64, serde_json::Value)> = shared
        .iter()
        .filter(|t| !seen.iter().any(|(_, v)| t.pos.sub(v.at).horizontal() <= (2.0 * t.error_m).max(REPORT_MATCH_M)))
        .filter_map(|t| {
            let d = t.pos.sub(k.pos);
            let r = d.horizontal();
            (r <= REPORT_REACH_M).then(|| {
                (r, json!({ "what": t.class.filter(|c| c.1 >= REPORT_SURE).map(|c| hypothesis_words(c.0)).unwrap_or("unknown"), "rangeM": round(r), "bearing": compass(d.bearing()), "ageS": round(t.age_s), "errorM": round(t.error_m), "by": t.by.map(|b| name(s, b)) }))
            })
        })
        .collect();
    beyond.sort_by(|a, b| a.0.total_cmp(&b.0));
    let reported_by_friends: Vec<serde_json::Value> = beyond.into_iter().take(4).map(|x| x.1).collect();
    let unseen_fire: Vec<serde_json::Value> = aw.suspects.values().map(|v| json!({ "bearing": compass(v.bearing), "agoS": round((s.ctx.now - v.last).secs()) })).collect();
    // Its orders and where they lead.
    let speed_ms = (i.platform.mover.cross_country_kmh.max(1.0) / 3.6) * ETA_PACE;
    let goal = heading_for(a).map(|g| {
        let d = g.sub(k.pos);
        json!({ "distanceM": round(d.horizontal()), "bearing": compass(d.bearing()), "etaMin": round(d.horizontal() / speed_ms / 60.0) })
    });
    let time_left_min = round((s.ctx.duration_limit() - s.ctx.now).secs().max(0.0) / 60.0);
    let orders = json!({
        "purpose": if a.mission.purpose.is_empty() { "none" } else { &a.mission.purpose },
        "doing": a.activity.word(),
        "phase": format!("{} of {}", (a.mission.phase + 1).min(a.mission.phases.len().max(1)), a.mission.phases.len().max(1)),
        "urgent": a.mission.urgent,
        "roe": match a.roe { RoeS::Never => "weapons hold", RoeS::IfFiredUpon => "fire only if fired upon", RoeS::WithinShortRange => "fire within short range", RoeS::Always => "fire at any enemy in reach" },
        "headingFor": goal,
        "gameTimeLeftMin": time_left_min,
    });
    // Its ground.
    let g = &s.ctx.ground;
    let micro = g.micro(k.pos.e, k.pos.n);
    let exposure = g.exposure(k.pos.e, k.pos.n);
    let key: Vec<serde_json::Value> = g
        .key_terrain_near(k.pos.e, k.pos.n, 1_500.0)
        .into_iter()
        .take(3)
        .map(|t| json!({ "kind": t.kind, "distanceM": round((t.e - k.pos.e).hypot(t.n - k.pos.n)), "bearing": compass((t.e - k.pos.e).atan2(t.n - k.pos.n)), "note": t.note }))
        .collect();
    let ground = json!({
        "cover": pct(micro.cover) / 100.0,
        "concealment": pct(micro.concealment) / 100.0,
        "land": format!("{:?}", g.land_cover(k.pos.e, k.pos.n)).to_lowercase(),
        "onRoad": g.road_at(k.pos.e, k.pos.n).is_some(),
        "openToView": exposure.map(|x| pct(x) / 100.0),
        "deadGround": exposure.map(|x| x < 0.1),
        "inTownBlock": g.block(k.pos.e, k.pos.n).map(|b| json!({ "material": b.material, "floors": b.floors, "standingPct": pct(b.standing) })),
        "keyTerrainNear": key,
    });
    // Friends close by, nearest first.
    let mut friends: Vec<(f64, serde_json::Value)> = s
        .ctx
        .order
        .iter()
        .filter(|&&f| f != e && alive(s, f))
        .filter_map(|&f| {
            let (fi, fk, fh) = (s.world.get::<Ident>(f)?, s.world.get::<Kin>(f)?, s.world.get::<Health>(f)?);
            if fi.side != i.side || fi.domain != Domain::Land {
                return None;
            }
            let d = fk.pos.sub(k.pos);
            let r = d.horizontal();
            (r <= 3_000.0).then(|| (r, json!({ "unit": fi.label, "rangeM": round(r), "bearing": compass(d.bearing()), "strengthPct": pct(fh.strength()) })))
        })
        .collect();
    friends.sort_by(|x, y| x.0.total_cmp(&y.0));
    prune(json!({
        "unit": {
            "name": i.label, "type": i.platform.name,
            "class": match i.platform.body.vulnerability {
                bgws_catalogue::Vulnerability::Armoured => "armoured_vehicle",
                bgws_catalogue::Vulnerability::Personnel => "foot",
                _ => "soft_skin",
            },
            "vehicles": format!("{} of {}", h.fit, h.start), "strengthPct": pct(h.strength()),
            "posture": posture, "suppressionPct": pct(c.suppression.level), "morale": morale,
            "lastFiredS": a.last_shot.map(|t| round((s.ctx.now - t).secs())),
            "leaderReplacedInS": c.leaderless_until.map(|t| round((t - s.ctx.now).secs())),
            "infantryAboard": s.world.get::<crate::dismount::Troops>(e).map(|t| t.aboard(h)).filter(|n| *n > 0).map(|n| format!("{n} sections")),
        },
        "weapons": weapons,
        "weaponReachM": round(my_reach),
        "enemiesSeen": enemies,
        "reportedByFriends": reported_by_friends,
        "firedOnByUnseen": unseen_fire,
        "orders": orders,
        "ground": ground,
        "friendsWithin3km": friends.into_iter().take(4).map(|x| x.1).collect::<Vec<_>>(),
    }))
}

/// Drop fields with nothing in them (null), so the model reads only what is known.
fn prune(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(m.into_iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k, prune(x))).collect()),
        serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(prune).collect()),
        x => x,
    }
}

pub(crate) fn compass(b: f64) -> &'static str {
    const N: [&str; 8] = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
    N[(((b.to_degrees().rem_euclid(360.0) + 22.5) / 45.0) as usize) % 8]
}

/// Raise one question for a trigger, with the engine's own rule choice for the rules decider (a
/// world domain's `registerRules`); an option not offered falls back.
pub(crate) fn ask_with_rule(s: &mut Sim, t: &Trigger, rule: RulesChoice) -> bool {
    let e = s.ctx.entity(t.entity);
    let (agent, side, template, archetype_default, kind) = {
        let i = s.world.get::<Ident>(e).expect("i");
        let kind = if i.domain == Domain::Land && t.point.starts_with('D') { QuestionKind::Unit } else { QuestionKind::Entity };
        (i.name.clone(), i.side, i.template.clone(), i.platform.template.clone(), kind)
    };
    let f = facts(s, e, t.about);
    let g = gates(s, e, t.about);
    let candidates = [template.as_str(), archetype_default.as_str()];
    let walked = match s.ctx.registry.decide(&candidates, t.point, &f, &g) {
        Ok(Some(w)) => w,
        Ok(None) => return false,
        Err(err) => {
            s.ctx.log.push(s.ctx.now, "decide.error").entity(t.entity).text(format!("{agent}: {err}"));
            return false;
        }
    };
    let (resolved, walk) = walked;
    let id = QuestionId(s.ctx.next_q);
    let reaction = reaction_time(s, e, t.entity);
    let ctx = QuestionContext {
        id,
        at: s.ctx.now,
        kind,
        agent: agent.clone(),
        side,
        trigger: t.point.to_string(),
        template: resolved.id.clone(),
        situation: situation(s, e, t.about),
        state: (kind == QuestionKind::Unit).then(|| unit_state(s, e, t.about)),
        reaction,
        summaries: BTreeMap::new(),
        expansions: BTreeMap::new(),
        rule,
    };
    let limits = QuestionLimits::from_figures(&s.ctx.figures);
    let Some(q) = build_question(&walk, &ctx, &s.ctx.vocab, limits) else { return false };
    s.ctx.next_q += 1;
    s.ctx.last_asked.insert(t.entity, s.ctx.now);
    s.world.get_mut::<Act>(e).expect("a").deciding = true;
    let ask_it = needs_asking(&q);
    { let __text = format!("{} ({}): {}", name(s, t.entity), t.point, q.title); s.ctx.log.push(s.ctx.now, "decide.question").side(side).entity(t.entity).text(__text).data(serde_json::json!({
        "question": id.0, "trigger": t.point, "options": q.options.iter().map(|o| &o.id).collect::<Vec<_>>(), "fallback": q.fallback, "rules": q.rules_choice, "asked": ask_it,
    })); }
    let pending = Pending { q: q.clone(), entity: Some(t.entity), about: t.about, answered: false, handed_out: false };
    s.ctx.pending.insert(id, pending);
    if !ask_it {
        // A drill or the only option: taken without asking (§10.1).
        let choice = q.options.first().map(|o| o.id.clone()).unwrap_or_else(|| q.fallback.clone());
        let a = Answer::simple(&q, choice, AnswerSource::Rules);
        let _ = s.answer(vec![a]);
    }
    true
}

fn reaction_time(s: &Sim, e: bevy_ecs::prelude::Entity, id: EntityId) -> SimDuration {
    let c = s.world.get::<Crew>(e).expect("c");
    let pp = &s.ctx.p.people;
    let fat = c.fatigue.effects(pp);
    let med = reaction_median_s(Drill::Practised, c.skill.effective(pp), &fat, c.suppression.level, pp);
    let mut rng = s.ctx.rng("reaction", id.0 as u64, 0);
    SimDuration::from_secs(sample_reaction_s(med, pp, &mut rng))
}

/// An answer's time has come: carry it out.
pub fn apply(s: &mut Sim, qid: QuestionId) {
    let Some(p) = s.ctx.pending.remove(&qid) else { return };
    let Some(a) = s.ctx.answers.get(&qid).cloned() else { return };
    if p.q.kind == QuestionKind::Hq {
        let plan = a.plan.clone().and_then(|v| serde_json::from_value::<Plan>(v).ok());
        match plan {
            Some(plan) => {
                // With nets, the plan waits for its planning time and travels as orders.
                if !crate::domains::drones::route_plan(s, p.q.side, &plan, p.q.at) {
                    apply_plan(s, p.q.side, &plan)
                }
            }
            None => {
                s.ctx.log.push(s.ctx.now, "decide.plan").side(p.q.side).text(format!("{} HQ: no usable plan; units keep their orders", p.q.side));
            }
        }
        return;
    }
    let Some(id) = p.entity else { return };
    let e = s.ctx.entity(id);
    if let Some(mut act) = s.world.get_mut::<Act>(e) {
        act.deciding = false;
        act.last_event = s.ctx.now;
    }
    if !alive(s, e) {
        return;
    }
    let Some(opt) = p.q.options.iter().find(|o| o.id == a.choice).cloned() else { return };
    let applied = crate::world::apply_action(s, e, id, &opt.action, &opt.params, p.about) || apply_land_action(s, e, id, &opt.action, p.about);
    { let __text = format!("{}: {}{}", name(s, id), opt.summary, if applied { "" } else { " (nothing to do)" }); s.ctx.log.push(s.ctx.now, "decide.apply").side(p.q.side).entity(id).text(__text).data(serde_json::json!({ "question": qid.0, "action": opt.action, "option": opt.id, "summary": opt.summary, "trigger": p.q.trigger, "title": p.q.title, "options": p.q.options.iter().map(|o| serde_json::json!({ "id": o.id, "summary": o.summary })).collect::<Vec<_>>(), "source": a.source, "probabilities": a.probabilities, "rationale": a.rationale })); }
}

fn point_away(s: &Sim, e: bevy_ecs::prelude::Entity, dist: f64) -> Enu {
    let k = s.world.get::<Kin>(e).expect("k");
    let side = s.world.get::<Ident>(e).expect("i").side;
    let away = threat_bearing(s, e).map(|b| b + std::f64::consts::PI).unwrap_or(if side == Side::Blue { std::f64::consts::PI } else { 0.0 });
    k.pos.add(Enu::new(away.sin(), away.cos(), 0.0).scale(dist))
}

/// The best covered spot within the search radius (cover first, then height).
/// A firing position against the threats it knows of (seen in the last
/// five minutes, or firing from a bearing it has not found): within
/// `sim.exec.reposition_m`, the spot from which the nearest threat sees its
/// turret but not its hull (hull-down to it), or failing that one hidden
/// from all of them; short moves first, and none that close the range for
/// nothing. The spot, and whether it is a hull-down firing position. With no
/// threat known, the best cover nearby (`better_spot`).
fn fire_position(s: &Sim, e: bevy_ecs::prelude::Entity) -> (Enu, bool) {
    let k = *s.world.get::<Kin>(e).expect("k");
    let now = s.ctx.now;
    let i = s.world.get::<Ident>(e).expect("i");
    let person = i.platform.body.vulnerability == bgws_catalogue::Vulnerability::Personnel;
    let (hull, top) = if person { (0.4, 1.1) } else { (1.0, 2.6) };
    let (class, _) = crate::systems::mover_class(i.platform.mover.kind);
    let aw = s.world.get::<Aware>(e).expect("aw");
    let mut threats: Vec<Enu> = aw.seen.values().filter(|v| (now - v.last).secs() < 300.0).map(|v| v.at).collect();
    threats.extend(aw.suspects.values().map(|sp| k.pos.add(Enu::new(sp.bearing.sin() * 1_500.0, sp.bearing.cos() * 1_500.0, 0.0))));
    threats.sort_by(|a, b| a.sub(k.pos).horizontal().total_cmp(&b.sub(k.pos).horizontal()));
    threats.truncate(3);
    if threats.is_empty() {
        return (better_spot(s, e), false);
    }
    let g = &s.ctx.ground;
    let r = s.ctx.p.reposition_m;
    let main = threats[0];
    let d0 = main.sub(k.pos).horizontal();
    let score = |p: Enu| -> (f64, bool) {
        let seen = |t: &Enu, agl: f64| g.los(*t, 2.5, p, agl).p_visible;
        let hull_main = seen(&main, hull);
        let top_main = seen(&main, top);
        let hull_down = hull_main < 0.3 && top_main > 0.5;
        let masked = threats.iter().all(|t| seen(t, top) < 0.3);
        let exposed = threats.iter().map(|t| seen(t, hull)).sum::<f64>() / threats.len() as f64;
        let d = p.sub(k.pos).horizontal();
        let closing = ((d0 - main.sub(p).horizontal()) / r).max(0.0);
        let v = if hull_down { 1.0 } else { 0.0 } + if masked { 0.6 } else { 0.0 } - 0.5 * exposed + 0.3 * g.micro(p.e, p.n).cover - 0.25 * d / r - 0.4 * closing;
        (v, hull_down)
    };
    let mut best = (score(k.pos), k.pos);
    for ring in [0.3, 0.6, 1.0] {
        for step in 0..16 {
            let b = step as f64 * std::f64::consts::TAU / 16.0;
            let p = k.pos.add(Enu::new(b.sin() * r * ring, b.cos() * r * ring, 0.0));
            if !g.contains(p) || g.going(p.e, p.n, class).speed_kmh <= 0.0 {
                continue;
            }
            let p = Enu::new(p.e, p.n, g.height(p.e, p.n));
            let sc = score(p);
            if sc.0 > best.0 .0 + 0.05 {
                best = (sc, p);
            }
        }
    }
    (best.1, best.0 .1)
}

fn better_spot(s: &Sim, e: bevy_ecs::prelude::Entity) -> Enu {
    let k = *s.world.get::<Kin>(e).expect("k");
    let r = s.ctx.p.cover_search_m;
    let here_open = s.ctx.ground.exposure(k.pos.e, k.pos.n).map(|x| s.ctx.ground.appreciation_use[1] * x).unwrap_or(0.0);
    let mut best = (s.ctx.ground.micro(k.pos.e, k.pos.n).cover - here_open, k.pos);
    for ring in 1..=3 {
        for step in 0..12 {
            let b = step as f64 * std::f64::consts::TAU / 12.0;
            let d = r * ring as f64 / 3.0;
            let p = k.pos.add(Enu::new(b.sin() * d, b.cos() * d, 0.0));
            if !s.ctx.ground.contains(p) {
                continue;
            }
            let open = s.ctx.ground.exposure(p.e, p.n).map(|x| s.ctx.ground.appreciation_use[1] * x).unwrap_or(0.0);
            let c = s.ctx.ground.micro(p.e, p.n).cover + 0.002 * (s.ctx.ground.height(p.e, p.n) - k.pos.u).clamp(-20.0, 20.0) - open;
            if c > best.0 + 0.05 {
                best = (c, p);
            }
        }
    }
    best.1
}

fn nearest_friend(s: &Sim, e: bevy_ecs::prelude::Entity) -> Option<(EntityId, Enu)> {
    let me = s.world.get::<Ident>(e)?;
    let pos = s.world.get::<Kin>(e)?.pos;
    s.ctx
        .order
        .iter()
        .filter(|&&o| o != e && alive(s, o))
        .filter_map(|&o| {
            let i = s.world.get::<Ident>(o)?;
            if i.side != me.side || i.domain != Domain::Land {
                return None;
            }
            Some((i.id, s.world.get::<Kin>(o)?.pos))
        })
        .min_by(|a, b| a.1.sub(pos).horizontal().total_cmp(&b.1.sub(pos).horizontal()).then(a.0.cmp(&b.0)))
}

/// A route for a unit to `to` over the ground: by its vehicle, for what the
/// move is for, and around the enemy its side believes in (never the truth).
/// A unit whose job is to find the enemy (recce, a screen, observers) keeps
/// to cover but does not steer away from it.
pub(crate) fn route_for(s: &Sim, e: bevy_ecs::prelude::Entity, to: Enu, mode: MoveModeS) -> Vec<Enu> {
    let seeks = seeks_contact(s, e, None);
    let urgent = is_urgent(s, e, None);
    plan_route(s, e, to, mode, seeks, urgent)
}

/// A mission phase as the activity to start now: a move's route planned
/// afresh from where the unit is (the phase's route was planned from where
/// it stood when the order came, so taken as it is it leads back there).
pub(crate) fn phase_activity(s: &Sim, e: bevy_ecs::prelude::Entity, phase: &Activity) -> Activity {
    match phase {
        Activity::Move { route, mode, then } => match route.last() {
            Some(&to) => Activity::Move { route: route_for(s, e, to, *mode), mode: *mode, then: then.clone() },
            None => phase.clone(),
        },
        Activity::Withdraw { route, then } => match route.last() {
            Some(&to) => Activity::Withdraw { route: plan_route(s, e, to, MoveModeS::Tactical, false, is_urgent(s, e, None)), then: then.clone() },
            None => phase.clone(),
        },
        _ => phase.clone(),
    }
}

/// The unit's current mission phase as the activity to start now.
pub(crate) fn current_phase(s: &Sim, e: bevy_ecs::prelude::Entity) -> Activity {
    let phase = s.world.get::<Act>(e).and_then(|a| a.mission.phases.get(a.mission.phase).cloned());
    phase.map(|p| phase_activity(s, e, &p)).unwrap_or(Activity::Overwatch)
}

/// Is this move time-critical: orders "now", or a weapon getting into range (fire support)?
pub(crate) fn is_urgent(s: &Sim, e: bevy_ecs::prelude::Entity, order: Option<&crate::hq::UnitOrder>) -> bool {
    let fires = |t: &str| {
        let t = t.to_lowercase();
        t.contains("fire support") || t.contains("support by fire") || t.contains("firing position")
    };
    if let Some(o) = order {
        return o.urgency.as_deref() == Some("now") || fires(&o.task);
    }
    s.world.get::<Act>(e).is_some_and(|a| a.mission.urgent || fires(&a.mission.purpose))
}

/// Is this unit out to find the enemy: by its orders (when given), its mission, or its role?
pub(crate) fn seeks_contact(s: &Sim, e: bevy_ecs::prelude::Entity, order: Option<&crate::hq::UnitOrder>) -> bool {
    const WORDS: [&str; 6] = ["recce", "recon", "screen", "observ", "scout", "guard"];
    let has = |t: &str| {
        let t = t.to_lowercase();
        WORDS.iter().any(|w| t.contains(w))
    };
    if let Some(o) = order {
        return o.on_contact.as_deref() == Some("observe") || has(&o.task);
    }
    s.world.get::<Act>(e).is_some_and(|a| has(&a.mission.purpose)) || s.world.get::<Ident>(e).is_some_and(|i| has(&i.platform.role) || has(&i.platform.class))
}

fn plan_route(s: &Sim, e: bevy_ecs::prelude::Entity, to: Enu, mode: MoveModeS, seeks: bool, urgent: bool) -> Vec<Enu> {
    let k = s.world.get::<Kin>(e).expect("k");
    let i = s.world.get::<Ident>(e).expect("i");
    let class = match i.platform.mover.kind {
        bgws_catalogue::MoverKind::Tracked => bgws_env::VehicleClass::Tracked,
        bgws_catalogue::MoverKind::Foot => bgws_env::VehicleClass::Foot,
        _ => bgws_env::VehicleClass::Wheeled,
    };
    let threats = if seeks { Vec::new() } else { believed_threats(s, i.side) };
    let m = &i.platform.mover;
    // Only the obstacles this side knows of: an unknown minefield is met, not avoided.
    let obstacles: Vec<crate::routing::KnownObstacle> = s.ctx.route_obstacles.iter().filter(|(_, known)| known.contains(&i.side)).map(|(o, _)| o.clone()).collect();
    let r = crate::routing::RouteRequest { from: k.pos, to, class, mode, road_kmh: m.road_kmh.max(m.cross_country_kmh).max(1.0), cross_kmh: m.cross_country_kmh.max(1.0), threats: &threats, obstacles: &obstacles, urgent };
    s.ctx.ground.plan(&r, &s.ctx.ground.route_params).waypoints
}

/// Where a side believes the enemy is: its confirmed tracks and its units' own sightings.
pub(crate) fn believed_threats(s: &Sim, side: bgws_core::Side) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = s.ctx.tracks.get(&side).map(|t| t.confirmed(&s.ctx.p.know).iter().map(|t| (t.position.e, t.position.n)).collect()).unwrap_or_default();
    for &o in &s.ctx.order {
        let (Some(oi), Some(aw)) = (s.world.get::<Ident>(o), s.world.get::<Aware>(o)) else { continue };
        if oi.side == side && alive(s, o) {
            v.extend(aw.seen.values().map(|x| (x.at.e, x.at.n)));
        }
    }
    v
}

/// The land action library (`actions/land.ts`), as activities.
pub fn apply_land_action(s: &mut Sim, e: bevy_ecs::prelude::Entity, id: EntityId, action: &str, about: Option<EntityId>) -> bool {
    let target = about.filter(|t| s.world.get::<Aware>(e).map(|a| a.seen.contains_key(t)).unwrap_or(false)).or_else(|| {
        let pos = s.world.get::<Kin>(e)?.pos;
        s.world.get::<Aware>(e)?.seen.iter().min_by(|a, b| a.1.at.sub(pos).horizontal().total_cmp(&b.1.at.sub(pos).horizontal())).map(|x| *x.0)
    });
    let current = s.world.get::<Act>(e).expect("a").activity.clone();
    let mission_activity = current_phase(s, e);
    let withdraw_m = s.ctx.p.withdraw_m;
    let reach = {
        let i = s.world.get::<Ident>(e).expect("i");
        i.platform.mounts.iter().filter_map(|m| m.munitions.first()).filter_map(|m| s.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max)
    };
    let mut chosen_position: Option<Enu> = None;
    let new: Option<Activity> = match action.trim_start_matches("land.") {
        "complyWithOrders" | "resumeOrders" | "resumeOrCarryOn" | "complyWithRequest" => Some(mission_activity),
        "carryOn" | "keepAtIt" | "keepFiring" | "keepHolding" | "keepHoldingHere" | "finishFightFirst" => None,
        "skipToNextPhase" => {
            {
                let mut a = s.world.get_mut::<Act>(e).expect("a");
                if a.mission.phase + 1 < a.mission.phases.len() {
                    a.mission.phase += 1;
                }
            }
            Some(current_phase(s, e))
        }
        "engageNow" | "returnFire" | "fireNow" | "fireFirst" | "shiftFire" | "betterShotOnTarget" | "shiftFireFromLost" | "shiftFireFromTarget" => target.map(|t| Activity::Engage { target: t, volleys: None, then: Some(Box::new(Activity::Overwatch)) }),
        "fireAndMove" => target.map(|t| Activity::Engage { target: t, volleys: Some(2), then: Some(Box::new(Activity::Move { route: vec![better_spot(s, e)], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) })) }),
        "fireAndBack" | "breakOffCovered" => target.map(|t| Activity::Engage { target: t, volleys: Some(1), then: Some(Box::new(Activity::Withdraw { route: vec![point_away(s, e, withdraw_m)], then: Some(Box::new(Activity::Overwatch)) })) }),
        "waitFor" | "waitCloser" => target.map(|t| Activity::Wait { target: t, trigger: WaitTrigger::Range { within_m: (reach * 0.6).max(300.0) }, auto_fire: action.ends_with("waitFor"), met: false }),
        "letPass" | "goQuiet" | "goQuietInCover" | "stopFiring" | "relocateQuietly" => Some(Activity::Quiet),
        "holdHere" | "holdAndRecover" | "cannotComply" => Some(Activity::Hold),
        "observe" | "watchLastSeen" => Some(Activity::Observe),
        "overwatchHere" | "partlyOverwatch" => Some(Activity::Overwatch),
        "betterPosition" | "hullDownThenFire" | "goQuietInCoverMove" => {
            let (spot, firing) = fire_position(s, e);
            chosen_position = firing.then_some(spot);
            Some(Activity::Move { route: vec![spot], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) })
        }
        // Rejoin (as the vocabulary says): resume its orders; with no
        // phases of orders, carry on with what it was doing.
        "rejoin" => s.world.get::<Act>(e).filter(|a| !a.mission.phases.is_empty()).map(|_| mission_activity.clone()),
        "pullBackToFriends" | "joinNearestFriends" => {
            let to = nearest_friend(s, e).map(|f| f.1).unwrap_or_else(|| point_away(s, e, withdraw_m));
            Some(Activity::Withdraw { route: route_for(s, e, to, MoveModeS::Tactical), then: Some(Box::new(Activity::Overwatch)) })
        }
        "outOfSight" | "pullBackFromBearing" | "breakContactFirst" | "slipAway" | "outOfTargetSight" => Some(Activity::Withdraw { route: vec![point_away(s, e, withdraw_m)], then: Some(Box::new(Activity::Overwatch)) }),
        "askFriendForHelp" => {
            if let Some((f, _)) = nearest_friend(s, e) {
                s.ctx.triggers.push(Trigger { entity: f, point: "D9", about: Some(id) });
            }
            None
        }
        "helpFriend" | "moveToSupport" => {
            let to = about.and_then(|f| s.world.get::<Kin>(s.ctx.entity(f)).map(|k| k.pos)).or_else(|| nearest_friend(s, e).map(|f| f.1));
            to.map(|p| Activity::Move { route: route_for(s, e, p, MoveModeS::Tactical), mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) })
        }
        "assault" => target.and_then(|t| s.world.get::<Aware>(e).and_then(|a| a.seen.get(&t).map(|v| v.at))).map(|p| Activity::Move { route: vec![p], mode: MoveModeS::Assault, then: Some(Box::new(Activity::Overwatch)) }),
        "searchBearing" => {
            let b = s.world.get::<Aware>(e).and_then(|a| a.suspects.values().next().map(|v| v.bearing)).unwrap_or(0.0);
            Some(Activity::Search { bearing: b, until: s.ctx.now + SimDuration::from_secs(60.0) })
        }
        "coverFromBearing" => Some(Activity::Move { route: vec![better_spot(s, e)], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Hold)) }),
        "regainContact" => target.and_then(|t| s.world.get::<Aware>(e).and_then(|a| a.seen.get(&t).map(|v| v.at))).map(|p| Activity::Move { route: vec![p], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) }),
        _ => return false,
    };
    if let Some(n) = new {
        if n != current {
            let mut a = s.world.get_mut::<Act>(e).expect("a");
            a.activity = n;
            a.fire_position = chosen_position;
        }
    }
    true
}

/// Turn an HQ's plan into each unit's mission and first phase.
pub fn apply_plan(s: &mut Sim, side: Side, plan: &Plan) {
    let refs = s.hq_situation(side, "plan").reference_points;
    let mut given = 0;
    for o in &plan.orders {
        if apply_unit_order(s, side, o, &refs) {
            given += 1;
        }
    }
    s.ctx.log.push(s.ctx.now, "decide.plan").side(side).text(format!("{} HQ: {} ({} units tasked)", side, plan.plan, given)).data(serde_json::to_value(plan).unwrap_or_default());
    s.ctx.last_plan.insert(side, plan.clone());
}

/// One unit's orders from a plan: its mission, ROE and first phase.
pub(crate) fn apply_unit_order(s: &mut Sim, side: Side, o: &crate::hq::UnitOrder, refs: &BTreeMap<String, bgws_core::Geodetic>) -> bool {
    let Some(&id) = s.ctx.by_name.get(&o.unit) else { return false };
    let e = s.ctx.entity(id);
    let Some(i) = s.world.get::<Ident>(e) else { return false };
    if i.side != side {
        return false;
    }
    let own = s.ctx.frame.enu_to_geodetic(s.world.get::<Kin>(e).expect("k").pos);
    let seeks = seeks_contact(s, e, Some(o)) || seeks_contact(s, e, None);
    let urgent = is_urgent(s, e, Some(o));
    let phases: Vec<Activity> = o
        .phases
        .iter()
        .map(|ph| {
            let to = ph.to.as_ref().and_then(|p| p.resolve(refs, own)).map(|g| s.ctx.frame.geodetic_to_enu(g));
            let mode = match ph.mode.as_deref() {
                Some("march") => MoveModeS::March,
                Some("assault") => MoveModeS::Assault,
                Some("bound") => MoveModeS::Bound,
                _ => MoveModeS::Tactical,
            };
            match (ph.action.as_str(), to) {
                ("move", Some(p)) => Activity::Move { route: plan_route(s, e, p, mode, seeks, urgent), mode, then: None },
                ("withdraw", Some(p)) => Activity::Withdraw { route: plan_route(s, e, p, MoveModeS::Tactical, false, urgent), then: None },
                ("observe", _) => Activity::Observe,
                ("hold", _) => Activity::Hold,
                _ => Activity::Overwatch,
            }
        })
        .collect();
    let roe = match o.roe.as_deref() {
        Some("never") => RoeS::Never,
        Some("ifFiredUpon") => RoeS::IfFiredUpon,
        Some("withinShortRange") => RoeS::WithinShortRange,
        _ => RoeS::Always,
    };
    let urgent = o.urgency.as_deref() == Some("now");
    let mut a = s.world.get_mut::<Act>(e).expect("a");
    a.mission = Mission { purpose: format!("{}: {}", o.task, o.intent), phases: phases.clone(), phase: 0, urgent };
    a.roe = roe;
    let fighting = matches!(a.activity, Activity::Engage { .. } | Activity::Wait { met: true, .. });
    if !fighting {
        if let Some(first) = phases.first() {
            a.activity = first.clone();
        }
    }
    if fighting {
        s.ctx.triggers.push(Trigger { entity: id, point: "D0", about: None });
    }
    true
}
