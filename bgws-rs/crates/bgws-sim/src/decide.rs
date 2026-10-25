//! Decision points (§10.1): triggers raised by the systems become questions
//! through the unit's behaviour template, with facts from its own state and
//! hard gates applied before options are offered. Drills and single options
//! apply without asking. Answers take effect after the agent's reaction time,
//! through the action library below.

use crate::components::*;
use crate::hq::Plan;
use crate::sim::{Pending, Sim, Trigger};
use crate::systems::{alive, name, threat_bearing};
use bgws_behaviour::{build_question, needs_asking, Capability, Cohesion, Emcon, FactValue, Gates, QuestionContext, QuestionLimits, Roe, RulesChoice, WeaponRelease};
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
        if !alive(s, e) || s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(true) {
            continue;
        }
        if let Some(last) = s.ctx.last_asked.get(&t.entity) {
            if (s.ctx.now - *last).secs() < s.ctx.p.cooldown_s && t.point != "D2" && t.point != "D0" {
                continue;
            }
        }
        // A broken unit runs itself.
        let morale = s.world.get::<Crew>(e).map(|c| c.morale.state).unwrap_or(MoraleState::Steady);
        if matches!(morale, MoraleState::Broken | MoraleState::Routed) && t.point != "D11" {
            continue;
        }
        if ask(s, &t) {
            done.insert(t.entity);
        }
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
    m.insert("enemy.known".into(), FactValue::Bool(enemy.is_some()));
    m.insert("enemy.rangeM".into(), enemy.map(|v| FactValue::Num(v.at.sub(k.pos).horizontal().round())).unwrap_or(FactValue::Null));
    m.insert("orders.urgent".into(), FactValue::Bool(a.mission.urgent));
    // World facts default to null for land units; the world layer fills its own.
    for f in bgws_behaviour::facts::world_facts() {
        m.entry(f.name.to_string()).or_insert(FactValue::Null);
    }
    crate::world::facts(s, e, &mut m);
    m
}

fn gates(s: &Sim, e: bevy_ecs::prelude::Entity) -> Gates {
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
    crate::world::capabilities(s, e, &mut caps);
    Gates {
        roe: match a.roe {
            RoeS::Never => Roe::Never,
            RoeS::IfFiredUpon => Roe::IfFiredUpon,
            RoeS::WithinShortRange => Roe::WithinShortRange,
            RoeS::Always => Roe::Always,
        },
        fired_upon: c.last_incoming.is_some(),
        weapon_release: WeaponRelease::Free,
        target_identified: true,
        emcon: Emcon::Unrestricted,
        capabilities: Some(caps),
        cohesion: match c.morale.state {
            MoraleState::Steady => Cohesion::Steady,
            MoraleState::Shaken | MoraleState::Pinned => Cohesion::Shaken,
            _ => Cohesion::Broken,
        },
    }
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

pub(crate) fn compass(b: f64) -> &'static str {
    const N: [&str; 8] = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
    N[(((b.to_degrees().rem_euclid(360.0) + 22.5) / 45.0) as usize) % 8]
}

/// Raise one question for a trigger. Returns whether anything was raised.
pub(crate) fn ask(s: &mut Sim, t: &Trigger) -> bool {
    let e = s.ctx.entity(t.entity);
    let (agent, side, template, archetype_default, kind) = {
        let i = s.world.get::<Ident>(e).expect("i");
        let kind = if i.domain == Domain::Land && t.point.starts_with('D') { QuestionKind::Unit } else { QuestionKind::Entity };
        (i.name.clone(), i.side, i.template.clone(), i.platform.template.clone(), kind)
    };
    let f = facts(s, e, t.about);
    let g = gates(s, e);
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
        reaction,
        summaries: BTreeMap::new(),
        expansions: BTreeMap::new(),
        rule: RulesChoice::Fallback,
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
            Some(plan) => apply_plan(s, p.q.side, &plan),
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
    { let __text = format!("{}: {}{}", name(s, id), opt.summary, if applied { "" } else { " (nothing to do)" }); s.ctx.log.push(s.ctx.now, "decide.apply").side(p.q.side).entity(id).text(__text).data(serde_json::json!({ "question": qid.0, "action": opt.action })); }
}

fn point_away(s: &Sim, e: bevy_ecs::prelude::Entity, dist: f64) -> Enu {
    let k = s.world.get::<Kin>(e).expect("k");
    let side = s.world.get::<Ident>(e).expect("i").side;
    let away = threat_bearing(s, e).map(|b| b + std::f64::consts::PI).unwrap_or(if side == Side::Blue { std::f64::consts::PI } else { 0.0 });
    k.pos.add(Enu::new(away.sin(), away.cos(), 0.0).scale(dist))
}

/// The best covered spot within the search radius (cover first, then height).
fn better_spot(s: &Sim, e: bevy_ecs::prelude::Entity) -> Enu {
    let k = *s.world.get::<Kin>(e).expect("k");
    let r = s.ctx.p.cover_search_m;
    let mut best = (s.ctx.ground.micro(k.pos.e, k.pos.n).cover, k.pos);
    for ring in 1..=3 {
        for step in 0..12 {
            let b = step as f64 * std::f64::consts::TAU / 12.0;
            let d = r * ring as f64 / 3.0;
            let p = k.pos.add(Enu::new(b.sin() * d, b.cos() * d, 0.0));
            if !s.ctx.ground.contains(p) {
                continue;
            }
            let c = s.ctx.ground.micro(p.e, p.n).cover + 0.002 * (s.ctx.ground.height(p.e, p.n) - k.pos.u).clamp(-20.0, 20.0);
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

/// A route for a unit to `to` over the ground, by its vehicle class.
pub(crate) fn route_for(s: &Sim, e: bevy_ecs::prelude::Entity, to: Enu) -> Vec<Enu> {
    let k = s.world.get::<Kin>(e).expect("k");
    let class = match s.world.get::<Ident>(e).expect("i").platform.mover.kind {
        bgws_catalogue::MoverKind::Tracked => bgws_env::VehicleClass::Tracked,
        bgws_catalogue::MoverKind::Foot => bgws_env::VehicleClass::Foot,
        _ => bgws_env::VehicleClass::Wheeled,
    };
    s.ctx.ground.route(k.pos, to, class)
}

/// The land action library (`actions/land.ts`), as activities.
pub fn apply_land_action(s: &mut Sim, e: bevy_ecs::prelude::Entity, id: EntityId, action: &str, about: Option<EntityId>) -> bool {
    let target = about.filter(|t| s.world.get::<Aware>(e).map(|a| a.seen.contains_key(t)).unwrap_or(false)).or_else(|| {
        let pos = s.world.get::<Kin>(e)?.pos;
        s.world.get::<Aware>(e)?.seen.iter().min_by(|a, b| a.1.at.sub(pos).horizontal().total_cmp(&b.1.at.sub(pos).horizontal())).map(|x| *x.0)
    });
    let current = s.world.get::<Act>(e).expect("a").activity.clone();
    let mission_activity = {
        let a = s.world.get::<Act>(e).expect("a");
        a.mission.phases.get(a.mission.phase).cloned().unwrap_or(Activity::Overwatch)
    };
    let withdraw_m = s.ctx.p.withdraw_m;
    let reach = {
        let i = s.world.get::<Ident>(e).expect("i");
        i.platform.mounts.iter().filter_map(|m| m.munitions.first()).filter_map(|m| s.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max)
    };
    let new: Option<Activity> = match action.trim_start_matches("land.") {
        "complyWithOrders" | "resumeOrders" | "resumeOrCarryOn" | "complyWithRequest" => Some(mission_activity),
        "carryOn" | "keepAtIt" | "keepFiring" | "keepHolding" | "keepHoldingHere" | "finishFightFirst" => None,
        "skipToNextPhase" => {
            let mut a = s.world.get_mut::<Act>(e).expect("a");
            if a.mission.phase + 1 < a.mission.phases.len() {
                a.mission.phase += 1;
            }
            Some(a.mission.phases.get(a.mission.phase).cloned().unwrap_or(Activity::Overwatch))
        }
        "engageNow" | "returnFire" | "fireNow" | "fireFirst" | "shiftFire" | "betterShotOnTarget" | "shiftFireFromLost" | "shiftFireFromTarget" => target.map(|t| Activity::Engage { target: t, volleys: None, then: Some(Box::new(Activity::Overwatch)) }),
        "fireAndMove" => target.map(|t| Activity::Engage { target: t, volleys: Some(2), then: Some(Box::new(Activity::Move { route: vec![better_spot(s, e)], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) })) }),
        "fireAndBack" | "breakOffCovered" => target.map(|t| Activity::Engage { target: t, volleys: Some(1), then: Some(Box::new(Activity::Withdraw { route: vec![point_away(s, e, withdraw_m)], then: Some(Box::new(Activity::Overwatch)) })) }),
        "waitFor" | "waitCloser" => target.map(|t| Activity::Wait { target: t, trigger: WaitTrigger::Range { within_m: (reach * 0.6).max(300.0) }, auto_fire: action.ends_with("waitFor"), met: false }),
        "letPass" | "goQuiet" | "goQuietInCover" | "stopFiring" | "relocateQuietly" => Some(Activity::Quiet),
        "holdHere" | "holdAndRecover" | "cannotComply" => Some(Activity::Hold),
        "observe" | "watchLastSeen" => Some(Activity::Observe),
        "overwatchHere" | "partlyOverwatch" => Some(Activity::Overwatch),
        "betterPosition" | "hullDownThenFire" | "goQuietInCoverMove" => Some(Activity::Move { route: vec![better_spot(s, e)], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) }),
        "pullBackToFriends" | "joinNearestFriends" | "rejoin" => {
            let to = nearest_friend(s, e).map(|f| f.1).unwrap_or_else(|| point_away(s, e, withdraw_m));
            Some(Activity::Withdraw { route: route_for(s, e, to), then: Some(Box::new(Activity::Overwatch)) })
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
            to.map(|p| Activity::Move { route: route_for(s, e, p), mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) })
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
            s.world.get_mut::<Act>(e).expect("a").activity = n;
        }
    }
    true
}

/// Turn an HQ's plan into each unit's mission and first phase.
pub fn apply_plan(s: &mut Sim, side: Side, plan: &Plan) {
    let refs = s.hq_situation(side, "plan").reference_points;
    let mut given = 0;
    for o in &plan.orders {
        let Some(&id) = s.ctx.by_name.get(&o.unit) else { continue };
        let e = s.ctx.entity(id);
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        if i.side != side {
            continue;
        }
        let own = s.ctx.frame.enu_to_geodetic(s.world.get::<Kin>(e).expect("k").pos);
        let phases: Vec<Activity> = o
            .phases
            .iter()
            .map(|ph| {
                let to = ph.to.as_ref().and_then(|p| p.resolve(&refs, own)).map(|g| s.ctx.frame.geodetic_to_enu(g));
                let mode = match ph.mode.as_deref() {
                    Some("march") => MoveModeS::March,
                    Some("assault") => MoveModeS::Assault,
                    Some("bound") => MoveModeS::Stealth,
                    _ => MoveModeS::Tactical,
                };
                match (ph.action.as_str(), to) {
                    ("move", Some(p)) => Activity::Move { route: route_for(s, e, p), mode, then: None },
                    ("withdraw", Some(p)) => Activity::Withdraw { route: route_for(s, e, p), then: None },
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
        given += 1;
    }
    s.ctx.log.push(s.ctx.now, "decide.plan").side(side).text(format!("{} HQ: {} ({} units tasked)", side, plan.plan, given)).data(serde_json::to_value(plan).unwrap_or_default());
    s.ctx.last_plan.insert(side, plan.clone());
}
