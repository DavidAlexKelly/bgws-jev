//! Dismounts: the infantry an IFV or APC carries.
//!
//! A carrier's troops ride with it as a [`Troops`] record, not a unit: they
//! do not see, fire or move on their own, and a vehicle lost takes its share
//! of them with it. The carrier is asked whether to let them out
//! (`dismountPoint`: here, in cover close by, or stay mounted) when:
//! - an enemy is seen within `sim.dismount.contact_m` (infantry fight close,
//!   and a carrier in a fight at that range is a target);
//! - the carrier is within `sim.dismount.objective_m` of where the last
//!   phase of its move ends (the assault goes in on foot);
//! - the carrier is in a town block (built-up ground is cleared on foot).
//!
//! The doctrine's answer: out here if in cover or the enemy is within 300 m,
//! else in the best cover within 150 m. Told to stay mounted, it is asked
//! again two minutes later. They get out (after the drive to cover) as a
//! new foot unit beside the carrier, with its orders, its rules of
//! engagement and its HQ (so its share of the picture), after
//! `sim.dismount.time_s`; the carrier halts there and supports by fire.
//! They do not get back in (a remount is not modelled).
//!
//! Troops aboard count in their side's strength from the start, so a side's
//! strength does not jump when they get out.

use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_core::{EntityId, SimDuration, SimTime};
use std::sync::Arc;

/// The infantry a carrier holds.
#[derive(Component, Clone, Debug)]
pub struct Troops {
    /// The catalogue archetype of one section.
    pub archetype: String,
    /// Sections carried at the start (a full load).
    pub sections: u32,
    /// Getting out: when they will be out.
    pub leaving: Option<SimTime>,
    /// Out: the foot unit they became, and its strength then.
    pub out: Option<(EntityId, u32)>,
    /// Driving to cover to let them out there.
    pub to_cover: Option<bgws_core::Enu>,
    /// Told to stay mounted: not asked again until then.
    pub not_before: SimTime,
}

impl Troops {
    /// Sections still aboard: a carrier that has lost vehicles has lost the
    /// troops in them.
    pub fn aboard(&self, h: &Health) -> u32 {
        if self.out.is_some() || h.destroyed || h.start == 0 {
            return 0;
        }
        ((self.sections as f64) * h.fit as f64 / h.start as f64).round() as u32
    }

    /// For the side's strength: (fit, start) of the troops this record
    /// stands for. Out, only the sections lost before they got out (the foot
    /// unit counts the rest itself).
    pub fn strength(&self, h: &Health) -> (f64, f64) {
        match self.out {
            Some((_, n)) => (0.0, self.sections.saturating_sub(n) as f64),
            None => (self.aboard(h) as f64, self.sections as f64),
        }
    }
}

/// Give every carrier its troops (its platform's `carried.dismounts`, one
/// load per vehicle in the force).
pub fn setup(s: &mut Sim) {
    for &e in &s.ctx.order.clone() {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        let Some(d) = i.platform.carried.dismounts.clone() else { continue };
        if i.domain != bgws_catalogue::Domain::Land || s.ctx.catalogue.platform(&d.archetype).is_none() || d.sections == 0 {
            continue;
        }
        s.world.entity_mut(e).insert(Troops { archetype: d.archetype, sections: d.sections, leaving: None, out: None, to_cover: None, not_before: SimTime::ZERO });
    }
}

/// Why its troops should get out now, if they should.
fn reason(s: &Sim, e: Entity) -> Option<&'static str> {
    let p = &s.ctx.p.dismount;
    let k = s.world.get::<Kin>(e)?;
    let a = s.world.get::<Act>(e)?;
    if matches!(a.activity, Activity::Withdraw { .. }) {
        return None;
    }
    let near = s.world.get::<Aware>(e)?.seen.values().any(|v| (s.ctx.now - v.last).secs() < 30.0 && v.at.sub(k.pos).horizontal() <= p.contact_m);
    if near {
        return Some("enemy close");
    }
    if let Activity::Move { route, .. } = &a.activity {
        // The end of the mission's last phase (or of a move with no phases).
        let last_phase = a.mission.phase + 1 >= a.mission.phases.len();
        if last_phase && route.last().is_some_and(|end| end.sub(k.pos).horizontal() <= p.objective_m) && !a.mission.purpose.starts_with("hold") {
            return Some("near the objective");
        }
    }
    if s.ctx.ground.block(k.pos.e, k.pos.n).is_some() {
        return Some("in a town");
    }
    None
}

/// Cover worth getting out in.
const IN_COVER: f64 = 0.3;
/// How far a carrier looks for cover to let its troops out in, m.
const COVER_SEARCH_M: f64 = 150.0;
/// Enemy this close: out here, whatever the cover, m.
const OUT_NOW_M: f64 = 300.0;
/// Told to stay mounted: asked again after this, s.
const REASK_S: f64 = 120.0;

/// 1 Hz: carriers whose troops should get out are asked; those told to
/// get out stop (after a drive to cover) and let them out.
pub fn base(s: &mut Sim) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(t) = s.world.get::<Troops>(e).cloned() else { continue };
        let Some(h) = s.world.get::<Health>(e).cloned() else { continue };
        if t.out.is_some() || h.destroyed {
            continue;
        }
        if let Some(at) = t.leaving {
            if at <= now {
                get_out(s, e);
            }
            continue;
        }
        if let Some(c) = t.to_cover {
            // Out when there, or when it stops short.
            let (pos, moving) = {
                let k = s.world.get::<Kin>(e).expect("kin");
                let a = s.world.get::<Act>(e).expect("act");
                (k.pos, a.activity.moving())
            };
            if pos.sub(c).horizontal() <= 25.0 || !moving {
                start_drill(s, e, "in cover");
            }
            continue;
        }
        // One question at a time.
        if t.aboard(&h) == 0 || now < t.not_before || s.world.get::<Act>(e).is_none_or(|a| a.deciding) {
            continue;
        }
        let Some(why) = reason(s, e) else { continue };
        let id = s.world.get::<Ident>(e).expect("ident").id;
        let pos = s.world.get::<Kin>(e).expect("kin").pos;
        let cover = s.ctx.ground.micro(pos.e, pos.n).cover;
        let close = s.world.get::<Aware>(e).map(|a| a.seen.values().any(|v| (now - v.last).secs() < 30.0 && v.at.sub(pos).horizontal() <= OUT_NOW_M)).unwrap_or(false);
        let rule = if cover >= IN_COVER || close { "dismountHere" } else { "dismountInCover" };
        s.world.entity_mut(e).insert(WhyOut(why));
        crate::decide::ask_with_rule(s, &crate::sim::Trigger { entity: id, point: "dismountPoint", about: None }, bgws_behaviour::RulesChoice::Option(rule.into()));
    }
}

/// Why the question was asked (for the log when they get out).
#[derive(Component, Clone, Debug)]
struct WhyOut(&'static str);

/// The carrier halts; its troops are out after the drill's time.
fn start_drill(s: &mut Sim, e: Entity, why: &str) {
    let now = s.ctx.now;
    let Some(h) = s.world.get::<Health>(e).cloned() else { return };
    let at = now + SimDuration::from_secs(s.ctx.p.dismount.time_s);
    let n = {
        let Some(mut tr) = s.world.get_mut::<Troops>(e) else { return };
        tr.leaving = Some(at);
        tr.to_cover = None;
        tr.aboard(&h)
    };
    let (id, side, label, keep_going) = {
        let i = s.world.get::<Ident>(e).expect("ident");
        let a = s.world.get::<Act>(e).expect("act");
        (i.id, i.side, i.label.clone(), a.activity.clone())
    };
    // What it was doing (before any drive to cover) carries on with the troops.
    if s.world.get::<WasDoing>(e).is_none() {
        stash(s, e, keep_going);
    }
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Overwatch;
    }
    let text = format!("{label} halts to dismount its infantry ({why})");
    s.ctx.log.push(now, "unit.dismount").side(side).entity(id).text(text).data(serde_json::json!({ "why": why, "sections": n }));
}

/// The dismount actions (`land.dismountHere`, `land.dismountInCover`,
/// `land.stayMounted`); false for any other, or a unit with no troops.
pub fn apply_action(s: &mut Sim, e: Entity, action: &str) -> bool {
    let now = s.ctx.now;
    if s.world.get::<Troops>(e).is_none() {
        return false;
    }
    let why = s.world.get::<WhyOut>(e).map(|w| w.0).unwrap_or("ordered");
    match action {
        "land.dismountHere" => start_drill(s, e, why),
        "land.dismountInCover" => {
            let pos = s.world.get::<Kin>(e).expect("kin").pos;
            let c = crate::domains::drones::drones::cover_near(s, pos, COVER_SEARCH_M);
            if c.sub(pos).horizontal() < 25.0 {
                start_drill(s, e, why);
            } else {
                let keep_going = s.world.get::<Act>(e).expect("act").activity.clone();
                stash(s, e, keep_going);
                if let Some(mut a) = s.world.get_mut::<Act>(e) {
                    a.activity = Activity::Move { route: vec![c], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) };
                    a.posture = Posture::Moving;
                }
                if let Some(mut tr) = s.world.get_mut::<Troops>(e) {
                    tr.to_cover = Some(c);
                }
            }
        }
        "land.stayMounted" => {
            if let Some(mut tr) = s.world.get_mut::<Troops>(e) {
                tr.not_before = now + SimDuration::from_secs(REASK_S);
            }
        }
        _ => return false,
    }
    true
}

/// What the carrier was doing when it stopped: the troops carry it on.
#[derive(Component, Clone, Debug)]
struct WasDoing(Activity);

fn stash(s: &mut Sim, e: Entity, a: Activity) {
    s.world.entity_mut(e).insert(WasDoing(a));
}

/// The troops get out: a foot unit beside the carrier, with its orders.
fn get_out(s: &mut Sim, e: Entity) {
    let now = s.ctx.now;
    let (Some(t), Some(h)) = (s.world.get::<Troops>(e).cloned(), s.world.get::<Health>(e).cloned()) else { return };
    let n = t.aboard(&h);
    let (cid, side, name, label, heading, pos) = {
        let i = s.world.get::<Ident>(e).expect("ident");
        let k = s.world.get::<Kin>(e).expect("kin");
        (i.id, i.side, i.name.clone(), i.label.clone(), k.heading, k.pos)
    };
    if n == 0 {
        if let Some(mut tr) = s.world.get_mut::<Troops>(e) {
            tr.out = Some((cid, 0));
        }
        return;
    }
    let Some(rec) = s.ctx.catalogue.platform(&t.archetype).cloned() else { return };
    let quality = s.ctx.scenario.forces.iter().find(|f| f.id == name).map(|f| f.quality).unwrap_or_default();
    // A few metres off the carrier's flank, on foot.
    let at = pos.add(bgws_core::Enu::new(heading.cos() * 15.0, -heading.sin() * 15.0, 0.0));
    let force = bgws_scenario::v4::Force {
        id: format!("{name}-dismounts"),
        label: format!("{label} dismounts"),
        side,
        platform: rec.id.clone(),
        count: n,
        quality,
        position: s.ctx.frame.enu_to_geodetic(at),
        heading_deg: Some(heading.to_degrees()),
        layer: bgws_scenario::v4::Layer::Entity,
        echelon: None,
        concealed: false,
        dug_in: false,
        command_rating: None,
        dummy: false,
        parent: Some(name.clone()),
        task: None,
        orbit: None,
        provider: None,
        lane: vec![],
        status: None,
    };
    let id = EntityId(s.ctx.order.len() as u32);
    let ne = crate::sim::spawn_force(&mut s.world, &mut s.ctx, id, &force, Arc::new(rec));
    s.ctx.order.push(ne);
    s.ctx.by_name.insert(force.id.clone(), id);
    // Its orders: what the carrier was doing, under the carrier's mission and rules.
    let was = s.world.get::<WasDoing>(e).map(|w| w.0.clone());
    let (mission, roe) = {
        let a = s.world.get::<Act>(e).expect("act");
        (a.mission.clone(), a.roe)
    };
    if let Some(mut a) = s.world.get_mut::<Act>(ne) {
        a.mission = mission;
        a.roe = roe;
        // On with the carrier's move; if it had stopped, on with the
        // mission's move from here (the assault goes on, on foot).
        let next_move = a.mission.phases.iter().skip(a.mission.phase).find(|p| matches!(p, Activity::Move { .. })).cloned();
        a.activity = match was {
            Some(act @ (Activity::Move { .. } | Activity::Engage { .. })) => act,
            _ => next_move.unwrap_or(Activity::Overwatch),
        };
        a.posture = if a.activity.moving() { Posture::Moving } else { Posture::Halted };
        a.last_event = now;
    }
    // Sightings carry over: they saw what the carrier saw.
    if let Some(aw) = s.world.get::<Aware>(e).cloned() {
        if let Some(mut na) = s.world.get_mut::<Aware>(ne) {
            na.seen = aw.seen;
        }
    }
    crate::domains::drones::c2::adopt(s, cid, id);
    if let Some(mut tr) = s.world.get_mut::<Troops>(e) {
        tr.out = Some((id, n));
        tr.leaving = None;
    }
    s.world.entity_mut(e).remove::<WasDoing>();
    let text = format!("{label}'s infantry are out: {} on foot", force.label);
    s.ctx.log.push(now, "unit.dismounted").side(side).entity(cid).target(id).text(text).data(serde_json::json!({ "sections": n, "unit": force.id }));
}
