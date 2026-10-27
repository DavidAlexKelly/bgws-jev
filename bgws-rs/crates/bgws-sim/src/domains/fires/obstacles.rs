//! Engineers and obstacles (§8.9, the TypeScript `landExt.ts`):
//!
//! - The scenario's minefields, ditches and dragon's teeth are rectangles on
//!   the ground, recorded in a `bgws_env::ChangeLayer` with the sides that
//!   know of them. They are not burnt into the terrain's going: the route
//!   planner is shared by both sides, and a planner that avoided an unknown
//!   minefield would leak the truth. Units meet them here instead.
//! - A unit sees an obstacle from close by (mines) or further (ditches) with
//!   line of sight; one that drives into an unknown minefield rolls
//!   encounters by `engineers::encounter_probability` for the ground it
//!   covered, and on a strike halts and asks (`minefieldStruck`). One about to
//!   enter a known obstacle (or a ditch a vehicle cannot cross) stops short
//!   and asks (`obstacleEncountered`).
//! - Breaching: engineers go to the near edge and clear a lane by
//!   `engineers::breach_lane` (about 1.5 h per 6 m lane through a 500 m belt
//!   **[P13]**), slower under observation and stopped while suppressed;
//!   ditches by `reduction_time_s`. When the lane is open everyone waiting
//!   goes through it.
//! - Digging in (`settledInPosition` → `land.digIn`) takes
//!   `engineers::dig_time_s`; a dug-in unit fights hull-down and counts as
//!   entrenched against indirect fire.

use super::{alive, is_engineer, name, raise, Dig, FiresWorld};
use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_catalogue::{Domain, MoverKind};
use bgws_core::{Enu, EntityId, Side, SimTime};
use bgws_env::{Change, ChangeKind, ChangeLayer, ObstacleKind as EnvObstacle};
use bgws_models::engineers::{self as eng, BreachMethod, Minefield, Obstacle as EngObstacle, Position, Soil, Tools};
use bgws_scenario::v4::ObstacleKind;
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Lane {
    /// Position across the obstacle's front, m.
    pub u: f64,
    pub width_m: f64,
    pub by: EntityId,
    pub done_at: SimTime,
    /// Mines left in it (a share of the field's density).
    pub residual: f64,
}

#[derive(Clone, Debug)]
pub struct ObstacleSt {
    pub id: String,
    pub owner: Side,
    pub kind: ObstacleKind,
    pub centre: Enu,
    pub width_m: f64,
    pub depth_m: f64,
    /// Radians clockwise from north: the direction of its depth.
    pub bearing: f64,
    /// Mines per m².
    pub density_m2: f64,
    pub known_to: BTreeSet<Side>,
    pub lanes: Vec<Lane>,
}

impl ObstacleSt {
    /// (u across the front, v into the depth) of a point.
    pub fn local(&self, p: Enu) -> (f64, f64) {
        let d = p.sub(self.centre);
        let b = self.bearing;
        (d.e * b.cos() - d.n * b.sin(), d.e * b.sin() + d.n * b.cos())
    }
    pub fn world(&self, u: f64, v: f64) -> Enu {
        let b = self.bearing;
        self.centre.add(Enu::new(u * b.cos() + v * b.sin(), -u * b.sin() + v * b.cos(), 0.0))
    }
    pub fn inside(&self, p: Enu) -> bool {
        let (u, v) = self.local(p);
        u.abs() <= self.width_m / 2.0 && v.abs() <= self.depth_m / 2.0
    }
    pub fn lane_at(&self, p: Enu, slack: f64) -> Option<&Lane> {
        let (u, _) = self.local(p);
        self.lanes.iter().find(|l| (l.u - u).abs() <= l.width_m / 2.0 + slack)
    }
    fn blocks_vehicles(&self) -> bool {
        matches!(self.kind, ObstacleKind::Ditch | ObstacleKind::DragonsTeeth)
    }
    fn eng_kind(&self) -> Option<EngObstacle> {
        match self.kind {
            ObstacleKind::Ditch => Some(EngObstacle::AntiTankDitch),
            ObstacleKind::DragonsTeeth => Some(EngObstacle::DragonsTeeth),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreachState {
    Moving,
    Working,
}

#[derive(Clone, Debug)]
pub struct Breach {
    pub obstacle: usize,
    pub by: EntityId,
    pub at: Enu,
    pub u: f64,
    pub state: BreachState,
    pub started: SimTime,
    pub done_at: SimTime,
    pub plan: Option<eng::BreachPlan>,
}

/// A unit stopped at an obstacle, and where it was going.
#[derive(Clone, Debug)]
pub struct Halt {
    pub obstacle: usize,
    pub dest: Option<Enu>,
    pub asked_at: SimTime,
    pub waiting: bool,
}

pub struct ObstacleState {
    pub obstacles: Vec<ObstacleSt>,
    pub breaches: Vec<Breach>,
    pub halted: BTreeMap<EntityId, Halt>,
    pub changes: ChangeLayer,
    /// Breach durations, s (the breach vignette).
    pub breach_s: Vec<f64>,
}

impl ObstacleState {
    pub fn new(_p: &super::FParams) -> Self {
        ObstacleState { obstacles: vec![], breaches: vec![], halted: BTreeMap::new(), changes: ChangeLayer::default(), breach_s: vec![] }
    }
}

pub fn setup(s: &mut Sim, st: &mut FiresWorld) {
    for (k, o) in s.ctx.scenario.world.obstacles.iter().enumerate() {
        if o.kind == ObstacleKind::SeaMines {
            continue;
        }
        let centre = s.ctx.frame.geodetic_to_enu(o.at);
        let ob = ObstacleSt {
            id: o.id.clone().unwrap_or_else(|| format!("obstacle-{}", k + 1)),
            owner: o.owner,
            kind: o.kind,
            centre,
            width_m: o.width_m.max(1.0),
            depth_m: o.depth_m.max(1.0),
            bearing: o.bearing_deg.to_radians(),
            density_m2: if o.kind == ObstacleKind::Minefield { o.density.max(0.0) / o.depth_m.max(1.0) } else { 0.0 },
            known_to: [o.owner].into_iter().collect(),
            lanes: vec![],
        };
        let a = ob.world(-ob.width_m / 2.0, 0.0);
        let b = ob.world(ob.width_m / 2.0, 0.0);
        let kind = match o.kind {
            ObstacleKind::Ditch => EnvObstacle::AntiTankDitch,
            ObstacleKind::DragonsTeeth => EnvObstacle::DragonsTeeth,
            _ => EnvObstacle::Minefield,
        };
        st.obstacles.changes.push(Change { id: k as u64, at: SimTime::ZERO, e: centre.e, n: centre.n, kind: ChangeKind::Obstacle { kind, line: vec![(a.e, a.n), (b.e, b.n)], depth_m: ob.depth_m }, known_to: ob.known_to.clone() });
        st.obstacles.obstacles.push(ob);
    }
    for &e in &s.ctx.order.clone() {
        let land = s.world.get::<Ident>(e).map(|i| i.domain == Domain::Land && !super::is_node(&i.platform)).unwrap_or(false);
        if land {
            s.world.entity_mut(e).insert(Dig::default());
        }
    }
}

fn vehicle(s: &Sim, e: Entity) -> bool {
    s.world.get::<Ident>(e).map(|i| i.platform.mover.kind != MoverKind::Foot).unwrap_or(false)
}

/// The final point of a moving unit's route.
fn destination(a: &Activity) -> Option<Enu> {
    match a {
        Activity::Move { route, then, .. } | Activity::Withdraw { route, then } => then.as_deref().and_then(destination).or_else(|| route.last().copied()),
        _ => None,
    }
}

pub fn base(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    digging(s, st, dt);
    if st.obstacles.obstacles.is_empty() {
        return;
    }
    let now = s.ctx.now;
    let breaching: BTreeSet<EntityId> = st.obstacles.breaches.iter().map(|b| b.by).collect();
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let (id, side, eng_or_recce, land) = {
            let i = s.world.get::<Ident>(e).expect("i");
            (i.id, i.side, is_engineer(&i.platform) || i.platform.template.contains("recce"), i.domain == Domain::Land && !super::is_node(&i.platform))
        };
        if !land {
            continue;
        }
        let k = *s.world.get::<Kin>(e).expect("kin");
        // Spotting obstacles in view.
        for oi in 0..st.obstacles.obstacles.len() {
            let o = &st.obstacles.obstacles[oi];
            if o.known_to.contains(&side) {
                continue;
            }
            let near = o.centre.sub(k.pos).horizontal() - o.width_m.max(o.depth_m) / 2.0;
            let (eyes, per_s) = if o.kind == ObstacleKind::Minefield {
                (if eng_or_recce { st.p.see_mines_engineer_m } else { st.p.see_mines_m }, st.p.spot_mines_per_s)
            } else {
                (st.p.see_ditch_m, st.p.spot_ditch_per_s)
            };
            if near > eyes {
                continue;
            }
            let (u, v) = o.local(k.pos);
            let closest = o.world(u.clamp(-o.width_m / 2.0, o.width_m / 2.0), v.clamp(-o.depth_m / 2.0, o.depth_m / 2.0));
            if s.ctx.ground.los(k.pos, 2.0, closest, 0.3).p_visible <= 0.0 {
                continue;
            }
            let mut rng = s.ctx.rng("obstacle.spot", id.0 as u64, oi as u64);
            if rng.gen::<f64>() < per_s * dt {
                found(s, st, oi, side, id, "spots");
            }
        }
        if breaching.contains(&id) {
            continue;
        }
        let act = s.world.get::<Act>(e).expect("a").activity.clone();
        if !act.moving() {
            continue;
        }
        let step = k.vel.horizontal() * dt;
        if step <= 0.0 {
            continue;
        }
        let is_vehicle = vehicle(s, e);
        for oi in 0..st.obstacles.obstacles.len() {
            let o = &st.obstacles.obstacles[oi];
            if o.owner == side {
                continue;
            }
            let here = o.inside(k.pos);
            let lane = o.lane_at(k.pos, st.p.lane_slack_m).cloned();
            if here && o.kind == ObstacleKind::Minefield {
                let density = o.density_m2 * lane.as_ref().map(|l| l.residual).unwrap_or(1.0);
                let p = eng::encounter_probability(density, st.p.ep.vehicle_track_width_m, step) * if is_vehicle { 1.0 } else { st.p.foot_factor };
                let mut rng = s.ctx.rng("obstacle.mine", id.0 as u64, oi as u64);
                if rng.gen::<f64>() < p {
                    mine_strike(s, st, e, id, side, oi, &act, &mut rng);
                    break;
                }
                continue;
            }
            if here || lane.is_some() {
                continue;
            }
            // About to go in: stop short and decide.
            let next = match &act {
                Activity::Move { route, .. } | Activity::Withdraw { route, .. } => route.first().copied(),
                _ => None,
            };
            let Some(next) = next else { continue };
            let dir = next.sub(k.pos);
            if dir.horizontal() < 1e-6 {
                continue;
            }
            let ahead = k.pos.add(dir.scale(st.p.look_ahead_m.min(dir.horizontal() + st.p.look_ahead_m) / dir.horizontal()));
            let blocks = o.blocks_vehicles() && is_vehicle;
            if !o.inside(ahead) || o.lane_at(ahead, st.p.lane_slack_m).is_some() || !(o.known_to.contains(&side) || blocks) {
                continue;
            }
            if let Some(h) = st.obstacles.halted.get(&id) {
                if h.obstacle == oi && (now - h.asked_at).secs() < st.p.ask_again_s {
                    halt(s, e);
                    break;
                }
            }
            if !o.known_to.contains(&side) {
                found(s, st, oi, side, id, "comes up against");
            }
            st.obstacles.halted.insert(id, Halt { obstacle: oi, dest: destination(&act), asked_at: now, waiting: false });
            halt(s, e);
            let text = format!("{} halts at {}", name(s, id), st.obstacles.obstacles[oi].id);
            s.ctx.log.push(now, "land.obstacle").side(side).entity(id).text(text).data(serde_json::json!({ "obstacle": st.obstacles.obstacles[oi].id, "kind": format!("{:?}", st.obstacles.obstacles[oi].kind) }));
            let rule = obstacle_rule(s, st, e, id, side, oi);
            raise(s, id, "obstacleEncountered", None, Some(rule));
            break;
        }
    }
    breaches(s, st, dt);
}

fn found(s: &mut Sim, st: &mut FiresWorld, oi: usize, side: Side, by: EntityId, how: &str) {
    let o = &mut st.obstacles.obstacles[oi];
    o.known_to.insert(side);
    if let Some(c) = st.obstacles.changes.changes.iter_mut().find(|c| c.id == oi as u64) {
        c.known_to.insert(side);
    }
    let (oid, kind) = (o.id.clone(), o.kind);
    let text = format!("{} {how} a {:?} ({oid})", name(s, by), kind);
    s.ctx.log.push(s.ctx.now, "land.obstacleFound").side(side).entity(by).text(text).data(serde_json::json!({ "obstacle": oid, "kind": format!("{kind:?}") }));
}

fn halt(s: &mut Sim, e: Entity) {
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Hold;
    }
    if let Some(mut k) = s.world.get_mut::<Kin>(e) {
        k.vel = Enu::default();
    }
}

#[allow(clippy::too_many_arguments)]
fn mine_strike(s: &mut Sim, st: &mut FiresWorld, e: Entity, id: EntityId, side: Side, oi: usize, act: &Activity, rng: &mut impl Rng) {
    let now = s.ctx.now;
    if !st.obstacles.obstacles[oi].known_to.contains(&side) {
        found(s, st, oi, side, id, "runs onto");
    }
    let lost = rng.gen::<f64>() < st.p.mine_kill;
    let destroyed = {
        let mut h = s.world.get_mut::<Health>(e).expect("h");
        if lost {
            h.fit = h.fit.saturating_sub(1);
            h.losses.push(now);
            if h.fit == 0 {
                h.destroyed = true;
            }
        } else {
            h.mobility_kills += 1;
        }
        h.destroyed
    };
    if lost {
        let start = s.world.get::<Health>(e).expect("h").start as f64;
        let mut c = s.world.get_mut::<Crew>(e).expect("crew");
        c.new_casualties += 1.0 / start;
        c.blasts_per_min += 1.0;
    }
    st.obstacles.halted.insert(id, Halt { obstacle: oi, dest: destination(act), asked_at: now, waiting: false });
    halt(s, e);
    let oid = st.obstacles.obstacles[oi].id.clone();
    let text = format!("{} strikes a mine in {oid}: {}", name(s, id), if lost { "a vehicle lost" } else { "a vehicle immobilised" });
    s.ctx.log.push(now, "land.mine").side(side).entity(id).text(text).data(serde_json::json!({ "obstacle": oid, "lost": lost }));
    if destroyed {
        let text = format!("{} is destroyed", name(s, id));
        s.ctx.log.push(now, "unit.destroyed").side(side).entity(id).text(text);
        return;
    }
    let rule = if !st.obstacles.obstacles[oi].lanes.is_empty() {
        // A residual mine in a cleared lane: push on through it.
        "pushThroughLane"
    } else if nearest_engineer(s, st, side, s.world.get::<Kin>(e).expect("k").pos, Some(id)).is_some() {
        "callEngineers"
    } else {
        "reverseOut"
    };
    raise(s, id, "minefieldStruck", None, Some(rule));
}

/// The rules' choice at an obstacle (TS `registerRules.obstacleEncountered`).
fn obstacle_rule(s: &Sim, st: &FiresWorld, e: Entity, id: EntityId, side: Side, oi: usize) -> &'static str {
    let o = &st.obstacles.obstacles[oi];
    if !o.lanes.is_empty() {
        return "pushThroughLane";
    }
    if s.world.get::<Ident>(e).map(|i| is_engineer(&i.platform)).unwrap_or(false) {
        return "breach";
    }
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    let tank = s.world.get::<Ident>(e).map(|i| can_breach(&i.platform)).unwrap_or(false);
    if nearest_engineer(s, st, side, pos, Some(id)).is_some() {
        "callEngineers"
    } else if tank {
        // No engineers: a tank ploughs its own lane, slowly.
        "breach"
    } else if o.width_m <= st.p.bypass_max_m {
        "bypass"
    } else {
        "wait"
    }
}

/// A tank (mine plough) can breach for itself.
fn can_breach(p: &bgws_catalogue::Platform) -> bool {
    let t = format!("{} {} {}", p.id, p.class, p.role).to_lowercase();
    t.contains("mbt") || t.contains("tank")
}

fn nearest_engineer(s: &Sim, st: &FiresWorld, side: Side, near: Enu, not: Option<EntityId>) -> Option<EntityId> {
    s.ctx
        .order
        .iter()
        .filter(|&&e| alive(s, e))
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            if i.side != side || !is_engineer(&i.platform) || Some(i.id) == not || st.obstacles.breaches.iter().any(|b| b.by == i.id) {
                return None;
            }
            let d = s.world.get::<Kin>(e)?.pos.sub(near).horizontal();
            (d <= st.p.engineer_reach_m).then_some((d, i.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|x| x.1)
}

/// The engineers go to the near edge opposite `from` and start a breach there.
fn start_breach(s: &mut Sim, st: &mut FiresWorld, by: EntityId, oi: usize, from: Enu) {
    if st.obstacles.breaches.iter().any(|b| b.by == by) {
        return;
    }
    let o = &st.obstacles.obstacles[oi];
    let (u, v) = o.local(from);
    let u = u.clamp(-o.width_m / 2.0 + 10.0, (o.width_m / 2.0 - 10.0).max(-o.width_m / 2.0 + 10.0));
    let sign = if v < 0.0 { -1.0 } else { 1.0 };
    let at = o.world(u, sign * (o.depth_m / 2.0 + 20.0));
    st.obstacles.breaches.push(Breach { obstacle: oi, by, at, u, state: BreachState::Moving, started: s.ctx.now, done_at: SimTime(i64::MAX), plan: None });
    let e = s.ctx.entity(by);
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Move { route: vec![at], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Hold)) };
    }
    st.obstacles.halted.remove(&by);
    let side = s.world.get::<Ident>(e).expect("i").side;
    let text = format!("{} moves up to breach {}", name(s, by), st.obstacles.obstacles[oi].id);
    s.ctx.log.push(s.ctx.now, "land.breach").side(side).entity(by).text(text).data(serde_json::json!({ "phase": "move", "obstacle": st.obstacles.obstacles[oi].id }));
}

fn breaches(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    let now = s.ctx.now;
    let mut k = 0;
    while k < st.obstacles.breaches.len() {
        let b = st.obstacles.breaches[k].clone();
        let e = s.ctx.entity(b.by);
        let oid = st.obstacles.obstacles[b.obstacle].id.clone();
        let side = s.world.get::<Ident>(e).expect("i").side;
        if !alive(s, e) {
            st.obstacles.breaches.remove(k);
            continue;
        }
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        let moving = s.world.get::<Act>(e).map(|a| a.activity.moving()).unwrap_or(false);
        match b.state {
            BreachState::Moving => {
                if pos.sub(b.at).horizontal() <= 60.0 && !moving {
                    let (plan, time) = breach_time(s, st, e, b.obstacle);
                    let br = &mut st.obstacles.breaches[k];
                    br.state = BreachState::Working;
                    br.started = now;
                    br.done_at = now + bgws_core::SimDuration::from_secs(time);
                    br.plan = plan.clone();
                    let text = format!("{} starts breaching {oid} ({:.0} min)", name(s, b.by), time / 60.0);
                    s.ctx.log.push(now, "land.breach").side(side).entity(b.by).text(text).data(serde_json::json!({ "phase": "start", "obstacle": oid, "plannedS": time, "plan": plan }));
                } else if !moving {
                    // Its orders were changed: send it on again.
                    if let Some(mut a) = s.world.get_mut::<Act>(e) {
                        a.activity = Activity::Move { route: vec![b.at], mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Hold)) };
                    }
                }
            }
            BreachState::Working => {
                if pos.sub(b.at).horizontal() > 80.0 {
                    st.obstacles.breaches.remove(k);
                    let text = format!("{} abandons the breach of {oid}", name(s, b.by));
                    s.ctx.log.push(now, "land.breach").side(side).entity(b.by).text(text).data(serde_json::json!({ "phase": "abandon", "obstacle": oid }));
                    continue;
                }
                let supp = s.world.get::<Crew>(e).map(|c| c.suppression.level).unwrap_or(0.0);
                if supp >= st.p.work_stops_supp {
                    st.obstacles.breaches[k].done_at = b.done_at + bgws_core::SimDuration::from_secs(dt);
                } else if now >= b.done_at {
                    st.obstacles.breaches.remove(k);
                    let residual = b.plan.as_ref().map(|p| p.residual_fraction).unwrap_or(0.0);
                    let o = &mut st.obstacles.obstacles[b.obstacle];
                    o.lanes.push(Lane { u: b.u, width_m: st.p.ep.lane_width_m, by: b.by, done_at: now, residual });
                    o.known_to.insert(side);
                    let secs = (now - b.started).secs();
                    st.obstacles.breach_s.push(secs);
                    let text = format!("{} has cleared a lane through {oid} in {:.0} min", name(s, b.by), secs / 60.0);
                    s.ctx.log.push(now, "land.breach").side(side).entity(b.by).text(text).data(serde_json::json!({ "phase": "done", "obstacle": oid, "seconds": secs }));
                    // Everyone waiting at it goes through.
                    let waiting: Vec<EntityId> = st.obstacles.halted.iter().filter(|(_, h)| h.obstacle == b.obstacle && h.waiting).map(|(id, _)| *id).collect();
                    for w in waiting {
                        let we = s.ctx.entity(w);
                        if alive(s, we) && s.world.get::<Ident>(we).map(|i| i.side == side).unwrap_or(false) {
                            through_lane(s, st, w);
                        }
                    }
                    continue;
                }
            }
        }
        k += 1;
    }
}

/// How long this unit takes to clear a lane through (or reduce) the obstacle.
fn breach_time(s: &Sim, st: &FiresWorld, e: Entity, oi: usize) -> (Option<eng::BreachPlan>, f64) {
    let i = s.world.get::<Ident>(e).expect("i");
    let engineer = is_engineer(&i.platform);
    let tank = can_breach(&i.platform) && !engineer;
    let o = &st.obstacles.obstacles[oi];
    let id = i.id;
    // Observed: an enemy of the breacher can see it.
    let observed = s.ctx.order.iter().any(|&x| {
        alive(s, x) && s.world.get::<Ident>(x).map(|xi| xi.side == o.owner).unwrap_or(false) && s.world.get::<Aware>(x).map(|a| a.seen.contains_key(&id)).unwrap_or(false)
    });
    let mut rng = s.ctx.rng("obstacle.breach", id.0 as u64, oi as u64);
    let spread = 1.0 + st.p.breach_spread * (2.0 * rng.gen::<f64>() - 1.0);
    let slow = if engineer { 1.0 } else { st.p.non_engineer_slow };
    match o.eng_kind() {
        Some(k) => (None, eng::reduction_time_s(k, engineer, &st.p.ep) * spread * if observed { st.p.ep.observed_factor } else { 1.0 }),
        None => {
            let method = if engineer {
                BreachMethod::LineCharge
            } else if tank {
                BreachMethod::Plough
            } else {
                BreachMethod::Manual
            };
            let plan = eng::breach_lane(method, &Minefield { depth_m: o.depth_m, density_per_m2: o.density_m2, observed }, &st.p.ep);
            let t = plan.time_s * spread * slow;
            (Some(plan), t)
        }
    }
}

/// Through the nearest lane, then on to where it was going.
fn through_lane(s: &mut Sim, st: &mut FiresWorld, id: EntityId) {
    let Some(h) = st.obstacles.halted.remove(&id) else { return };
    let o = &st.obstacles.obstacles[h.obstacle];
    let e = s.ctx.entity(id);
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    let Some(lane) = o.lanes.iter().min_by(|a, b| (a.u - o.local(pos).0).abs().total_cmp(&(b.u - o.local(pos).0).abs())) else { return };
    let (_, v) = o.local(pos);
    let sign = if v < 0.0 { -1.0 } else { 1.0 };
    let entry = o.world(lane.u, sign * (o.depth_m / 2.0 + 30.0));
    let exit = o.world(lane.u, -sign * (o.depth_m / 2.0 + 30.0));
    let then = h.dest.map(|d| Box::new(Activity::Move { route: vec![d], mode: MoveModeS::Tactical, then: None }));
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.activity = Activity::Move { route: vec![entry, exit], mode: MoveModeS::Tactical, then: then.or(Some(Box::new(Activity::Overwatch))) };
    }
}

/// The obstacle a unit is stopped at (or the nearest one within 500 m).
fn obstacle_for(s: &Sim, st: &FiresWorld, id: EntityId) -> Option<usize> {
    if let Some(h) = st.obstacles.halted.get(&id) {
        return Some(h.obstacle);
    }
    let pos = s.world.get::<Kin>(s.ctx.entity(id))?.pos;
    st.obstacles
        .obstacles
        .iter()
        .enumerate()
        .map(|(k, o)| (o.centre.sub(pos).horizontal() - o.width_m.max(o.depth_m) / 2.0, k))
        .filter(|(d, _)| *d <= 500.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1)
}

/// The obstacle actions (`land.breach`, `land.bypass`, `land.callEngineers`, …) and `land.digIn`.
pub fn apply(s: &mut Sim, st: &mut FiresWorld, e: Entity, id: EntityId, action: &str) -> bool {
    let now = s.ctx.now;
    if action == "land.digIn" {
        start_dig(s, st, e);
        return true;
    }
    let obstacle_action = matches!(action, "land.breach" | "land.bypass" | "land.callEngineers" | "land.waitAtObstacle" | "land.pushThroughLane" | "land.reverseOut");
    if !obstacle_action {
        return false;
    }
    let Some(oi) = obstacle_for(s, st, id) else { return true };
    let side = s.world.get::<Ident>(e).expect("i").side;
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    st.obstacles.halted.entry(id).or_insert(Halt { obstacle: oi, dest: None, asked_at: now, waiting: false });
    let breacher = s.world.get::<Ident>(e).map(|i| is_engineer(&i.platform) || can_breach(&i.platform)).unwrap_or(false);
    match action {
        "land.breach" if breacher => start_breach(s, st, id, oi, pos),
        "land.breach" | "land.callEngineers" => {
            let eng = nearest_engineer(s, st, side, pos, Some(id));
            if let Some(eng) = eng {
                start_breach(s, st, eng, oi, pos);
            }
            if let Some(h) = st.obstacles.halted.get_mut(&id) {
                h.waiting = true;
            }
            halt(s, e);
            let text = format!("{} calls the engineers ({}) to {}", name(s, id), eng.map(|x| name(s, x)).unwrap_or_else(|| "none free".into()), st.obstacles.obstacles[oi].id);
            let mut b = s.ctx.log.push(now, "land.engineers").side(side).entity(id).text(text);
            if let Some(x) = eng {
                b = b.target(x);
            }
            drop(b);
        }
        "land.bypass" => {
            let o = st.obstacles.obstacles[oi].clone();
            if o.width_m > st.p.bypass_max_m {
                if let Some(h) = st.obstacles.halted.get_mut(&id) {
                    h.waiting = true;
                }
                return true;
            }
            let (u, v) = o.local(pos);
            let lr = if u >= 0.0 { 1.0 } else { -1.0 };
            let sign = if v < 0.0 { -1.0 } else { 1.0 };
            let corner = o.world(lr * (o.width_m / 2.0 + 150.0), sign * (o.depth_m / 2.0 + 50.0));
            let beyond = o.world(lr * (o.width_m / 2.0 + 150.0), -sign * (o.depth_m / 2.0 + 100.0));
            let dest = st.obstacles.halted.remove(&id).and_then(|h| h.dest);
            let then = dest.map(|d| Box::new(Activity::Move { route: vec![d], mode: MoveModeS::Tactical, then: None }));
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Move { route: vec![corner, beyond], mode: MoveModeS::Tactical, then: then.or(Some(Box::new(Activity::Overwatch))) };
            }
        }
        "land.waitAtObstacle" => {
            if let Some(h) = st.obstacles.halted.get_mut(&id) {
                h.waiting = true;
            }
            halt(s, e);
        }
        "land.pushThroughLane" => {
            if st.obstacles.obstacles[oi].lanes.is_empty() {
                if let Some(h) = st.obstacles.halted.get_mut(&id) {
                    h.waiting = true;
                }
                halt(s, e);
            } else {
                through_lane(s, st, id);
            }
        }
        "land.reverseOut" => {
            let heading = s.world.get::<Kin>(e).expect("k").heading + std::f64::consts::PI;
            let back = pos.add(Enu::new(heading.sin(), heading.cos(), 0.0).scale(120.0));
            if let Some(h) = st.obstacles.halted.get_mut(&id) {
                h.waiting = true;
            }
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Move { route: vec![back], mode: MoveModeS::March, then: Some(Box::new(Activity::Hold)) };
            }
        }
        _ => {}
    }
    true
}

// ── Digging in ───────────────────────────────────────────────────────────

fn start_dig(s: &mut Sim, st: &mut FiresWorld, e: Entity) {
    let i = s.world.get::<Ident>(e).expect("i");
    let (side, foot) = (i.side, i.platform.mover.kind == MoverKind::Foot);
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    let engineer_near = s.ctx.order.iter().any(|&x| {
        alive(s, x) && s.world.get::<Ident>(x).map(|xi| xi.side == side && is_engineer(&xi.platform)).unwrap_or(false) && s.world.get::<Kin>(x).map(|k| k.pos.sub(pos).horizontal() <= 1_500.0).unwrap_or(false)
    });
    let crew = s.world.get::<Ident>(e).expect("i").platform.body.crew.max(1);
    let need = if foot {
        // Two-man positions dug side by side.
        eng::dig_time_s(Position::Foxhole, Soil::Loam, Tools::Hand, 2, &st.p.ep)
    } else if engineer_near {
        eng::dig_time_s(Position::VehicleHullDown, Soil::Loam, Tools::Excavator, 1, &st.p.ep)
    } else {
        eng::dig_time_s(Position::VehicleHullDown, Soil::Loam, Tools::Hand, crew, &st.p.ep)
    };
    if let Some(mut d) = s.world.get_mut::<Dig>(e) {
        if !d.done {
            d.active = true;
            d.need_s = need;
        }
    }
}

fn digging(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        let Some(d) = s.world.get::<Dig>(e).cloned() else { continue };
        if !alive(s, e) {
            continue;
        }
        let a = s.world.get::<Act>(e).expect("a");
        let (moving, still_s, deciding) = (a.activity.moving(), (now - a.still_since).secs(), a.deciding);
        if moving {
            if d.active || d.done || d.worked_s > 0.0 {
                *s.world.get_mut::<Dig>(e).expect("dig") = Dig { asked: d.asked, ..Dig::default() };
            }
            continue;
        }
        if d.done {
            // Dug in: it fights hull-down from its position.
            s.world.get_mut::<Act>(e).expect("a").posture = Posture::HullDown;
            continue;
        }
        if d.active {
            let supp = s.world.get::<Crew>(e).map(|c| c.suppression.level).unwrap_or(0.0);
            let mut dg = s.world.get_mut::<Dig>(e).expect("dig");
            if supp < st.p.work_stops_supp {
                dg.worked_s += dt;
            }
            if dg.worked_s >= dg.need_s {
                dg.done = true;
                dg.active = false;
                let (id, side) = {
                    let i = s.world.get::<Ident>(e).expect("i");
                    (i.id, i.side)
                };
                let text = format!("{} is dug in", name(s, id));
                s.ctx.log.push(now, "land.dugIn").side(side).entity(id).text(text).data(serde_json::json!({ "seconds": d.need_s }));
            }
            continue;
        }
        let steady = s.world.get::<Crew>(e).map(|c| c.morale.state == bgws_models::people::MoraleState::Steady).unwrap_or(false);
        if !steady || deciding || still_s < st.p.settle_to_dig_s || d.asked.map(|t| (now - t).secs() < st.p.dig_ask_s).unwrap_or(false) {
            continue;
        }
        let (id, carrier) = {
            let i = s.world.get::<Ident>(e).expect("i");
            (i.id, i.platform.carried.cargo_t > 0.0 || i.hq)
        };
        if carrier {
            continue;
        }
        s.world.get_mut::<Dig>(e).expect("dig").asked = Some(now);
        raise(s, id, "settledInPosition", None, Some("digIn"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn ob(bearing_deg: f64) -> ObstacleSt {
        ObstacleSt { id: "m".into(), owner: Side::Red, kind: ObstacleKind::Minefield, centre: Enu::new(100.0, 200.0, 0.0), width_m: 1000.0, depth_m: 200.0, bearing: bearing_deg.to_radians(), density_m2: 0.002, known_to: BTreeSet::new(), lanes: vec![] }
    }

    #[test]
    fn local_frame_round_trips() {
        for b in [0.0, 30.0, 135.0, 270.0] {
            let o = ob(b);
            let p = o.world(123.0, -45.0);
            let (u, v) = o.local(p);
            assert_relative_eq!(u, 123.0, epsilon = 1e-9);
            assert_relative_eq!(v, -45.0, epsilon = 1e-9);
        }
        // Bearing 0: depth runs north, the front east–west.
        let o = ob(0.0);
        assert!(o.inside(Enu::new(500.0, 250.0, 0.0)));
        assert!(!o.inside(Enu::new(100.0, 350.0, 0.0)));
    }

    #[test]
    fn lanes_are_found_across_the_front() {
        let mut o = ob(0.0);
        o.lanes.push(Lane { u: 50.0, width_m: 6.0, by: EntityId(0), done_at: SimTime::ZERO, residual: 0.1 });
        assert!(o.lane_at(o.world(52.0, 0.0), 1.0).is_some());
        assert!(o.lane_at(o.world(60.0, 0.0), 1.0).is_none());
    }
}
