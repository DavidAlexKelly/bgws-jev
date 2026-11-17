//! Orders from outside: a federation's tasking (NETN SMC, as Foundry sends it
//! through the HLA–Palantir bridge), applied to our units.
//!
//! An order comes in as an [`ExternalInput::Order`](crate::remote::ExternalInput)
//! at a tick boundary, so the host records it and a replay gives it back at the
//! same tick. It becomes what an HQ's order would be (a mission of phases for
//! [`crate::decide`]), or a direct change (teleport, ROE, an engagement, a call
//! for fire). A unit under such orders is under outside command: its HQ's
//! plans pass it by until the orders are cancelled, so a Foundry order is not
//! undone by the next replan. Its own drills still run (it returns fire, takes
//! cover), as they would under any order.
//!
//! Each order is a task the [`TaskBook`] follows: executing until a move
//! arrives (or a patrol is cancelled), the target is destroyed, or the unit
//! is lost, so whoever gave it can be told (NETN `ETR_TaskStatus`).

use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::Resource;
use bgws_core::{EntityId, Enu, Geodetic, Side, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A move counts as done within this of its end point, m.
pub const ARRIVED_M: f64 = 75.0;

/// A unit that stops short of its point this long is sent on again (once),
/// s: its own drills (dismounting, a halt under fire) may have stopped it.
pub const STOPPED_SHORT_S: f64 = 120.0;

/// Tasks kept for reporting once finished.
const KEEP: usize = 500;

/// What an outside order asks of a unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "order", rename_all = "camelCase")]
pub enum ExternalOrder {
    /// Go there. `roads`: keep to roads (a march), else tactically across country.
    #[serde(rename_all = "camelCase")]
    Move { lat: f64, lon: f64, roads: bool },
    /// Go through these points in turn; `repeat` goes round again until cancelled.
    #[serde(rename_all = "camelCase")]
    Route { points: Vec<(f64, f64)>, repeat: bool },
    /// Put it there at once (an exercise controller's move).
    #[serde(rename_all = "camelCase")]
    Teleport { lat: f64, lon: f64, heading_deg: f64 },
    /// Stop and hold where it is.
    Halt,
    /// Drop every order: it holds, and is under its HQ's command again.
    Cancel,
    /// Rules of engagement: never, ifFiredUpon, withinShortRange or always.
    #[serde(rename_all = "camelCase")]
    Roe { roe: String },
    /// Fire on this entity.
    #[serde(rename_all = "camelCase")]
    Engage { target: EntityId },
    /// Call for indirect fire on this point, for the unit's side.
    #[serde(rename_all = "camelCase")]
    FireAt { lat: f64, lon: f64 },
}

impl ExternalOrder {
    pub fn word(&self) -> &'static str {
        match self {
            ExternalOrder::Move { .. } => "move",
            ExternalOrder::Route { .. } => "patrol",
            ExternalOrder::Teleport { .. } => "magic move",
            ExternalOrder::Halt => "halt",
            ExternalOrder::Cancel => "cancel orders",
            ExternalOrder::Roe { .. } => "rules of engagement",
            ExternalOrder::Engage { .. } => "engage",
            ExternalOrder::FireAt { .. } => "fire mission",
        }
    }
}

/// A task's state, as NETN's `TaskStatusEnum32` numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Accepted = 0,
    Refused = 1,
    Cancelled = 2,
    Executing = 3,
    Completed = 4,
    Error = 5,
}

impl TaskStatus {
    pub fn finished(self) -> bool {
        !matches!(self, TaskStatus::Accepted | TaskStatus::Executing)
    }
}

/// One task as followed.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRec {
    /// The giver's id for it (a NETN TaskId, as text).
    pub task: String,
    pub entity: EntityId,
    pub order: ExternalOrder,
    pub status: TaskStatus,
    pub since: SimTime,
    /// Where a move or route ends.
    #[serde(skip)]
    pub dest: Option<Enu>,
    /// Since when it has stood short of its point, and whether it has
    /// already been sent on again.
    #[serde(skip)]
    pub short_since: Option<SimTime>,
    #[serde(skip)]
    pub resent: bool,
}

#[derive(Resource, Default, Clone)]
pub struct TaskBook {
    pub tasks: Vec<TaskRec>,
    /// Units under outside command: their HQ's plans pass them by.
    pub controlled: BTreeSet<EntityId>,
}

pub fn setup(s: &mut Sim) {
    s.world.insert_resource(TaskBook::default());
}

/// Is this unit under outside command (so its HQ's orders pass it by)?
pub fn controlled(s: &Sim, id: EntityId) -> bool {
    s.world.get_resource::<TaskBook>().is_some_and(|b| b.controlled.contains(&id))
}

fn name(s: &Sim, id: EntityId) -> String {
    crate::systems::name(s, id)
}

fn to_enu(s: &Sim, lat: f64, lon: f64) -> Enu {
    let p = s.ctx.frame.geodetic_to_enu(Geodetic::new(lat, lon, 0.0));
    Enu::new(p.e, p.n, s.ctx.ground.height(p.e, p.n))
}

fn record(s: &mut Sim, rec: TaskRec) {
    let Some(mut b) = s.world.get_resource_mut::<TaskBook>() else { return };
    b.tasks.push(rec);
    if b.tasks.len() > KEEP {
        // Drop the oldest finished ones first.
        let drop = b.tasks.len() - KEEP;
        let mut gone = 0;
        b.tasks.retain(|t| {
            if gone < drop && t.status.finished() {
                gone += 1;
                false
            } else {
                true
            }
        });
    }
}

/// Close a unit's running tasks as superseded (a newer order took over).
fn supersede(s: &mut Sim, id: EntityId, why: &str) {
    let now = s.ctx.now;
    let mut closed = vec![];
    if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
        for t in b.tasks.iter_mut().filter(|t| t.entity == id && !t.status.finished()) {
            t.status = TaskStatus::Cancelled;
            t.since = now;
            closed.push(t.task.clone());
        }
    }
    for task in closed {
        let text = format!("{}: task {task} cancelled ({why})", name(s, id));
        s.ctx.log.push(now, "task.cancelled").entity(id).text(text).data(serde_json::json!({ "task": task, "why": why }));
    }
}

/// Why this unit cannot take outside orders, if it cannot.
pub fn refusal(s: &Sim, id: EntityId) -> Option<&'static str> {
    let Some(&e) = s.ctx.order.get(id.index()) else { return Some("no such unit") };
    if crate::remote::is_remote(s, e) {
        return Some("it belongs to another simulation");
    }
    if s.world.get::<Health>(e).is_none_or(|h| h.destroyed) {
        return Some("it is destroyed");
    }
    if s.world.get::<Act>(e).is_none() {
        return Some("it takes no orders (an installation)");
    }
    None
}

/// Apply one outside order to one of our units (at a tick boundary).
pub fn apply(s: &mut Sim, entity: EntityId, task: String, order: ExternalOrder) {
    let now = s.ctx.now;
    let side = s.ctx.order.get(entity.index()).and_then(|&e| s.world.get::<Ident>(e)).map(|i| i.side).unwrap_or(Side::Neutral);
    if let Some(why) = refusal(s, entity) {
        let text = format!("{}: {} refused ({why})", name(s, entity), order.word());
        s.ctx.log.push(now, "task.refused").side(side).entity(entity).text(text).data(serde_json::json!({ "task": task, "order": order, "why": why }));
        record(s, TaskRec { task, entity, order, status: TaskStatus::Refused, since: now, dest: None, short_since: None, resent: false });
        return;
    }
    let e = s.ctx.entity(entity);
    let mut dest = None;
    let mut status = TaskStatus::Executing;
    match &order {
        ExternalOrder::Move { lat, lon, roads } => {
            supersede(s, entity, "a new order");
            let mode = if *roads { "march" } else { "tactical" };
            give_phases(s, entity, side, "move", &[(*lat, *lon)], mode);
            dest = Some(to_enu(s, *lat, *lon));
        }
        ExternalOrder::Route { points, .. } => {
            supersede(s, entity, "a new order");
            give_phases(s, entity, side, "patrol", points, "tactical");
            dest = points.last().map(|&(la, lo)| to_enu(s, la, lo));
        }
        ExternalOrder::Teleport { lat, lon, heading_deg } => {
            supersede(s, entity, "moved by the controller");
            let to = to_enu(s, *lat, *lon);
            if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                k.pos = Enu::new(to.e, to.n, to.u + k.agl);
                k.vel = Enu::new(0.0, 0.0, 0.0);
                k.heading = heading_deg.to_radians();
            }
            // Its column would otherwise stretch back to where it was.
            if let Some(mut t) = s.world.get_mut::<crate::formation::Trail>(e) {
                *t = Default::default();
            }
            hold(s, e);
            status = TaskStatus::Completed;
        }
        ExternalOrder::Halt => {
            supersede(s, entity, "halted");
            hold(s, e);
            status = TaskStatus::Completed;
        }
        ExternalOrder::Cancel => {
            supersede(s, entity, "orders cancelled");
            hold(s, e);
            if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
                b.controlled.remove(&entity);
            }
            status = TaskStatus::Completed;
        }
        ExternalOrder::Roe { roe } => {
            let r = match roe.as_str() {
                "never" => RoeS::Never,
                "ifFiredUpon" => RoeS::IfFiredUpon,
                "withinShortRange" => RoeS::WithinShortRange,
                _ => RoeS::Always,
            };
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.roe = r;
            }
            status = TaskStatus::Completed;
        }
        ExternalOrder::Engage { target } => {
            let ok = s.ctx.order.get(target.index()).and_then(|&t| s.world.get::<Health>(t)).is_some_and(|h| !h.destroyed);
            if !ok {
                let text = format!("{}: engage refused (no such target, or destroyed)", name(s, entity));
                s.ctx.log.push(now, "task.refused").side(side).entity(entity).text(text).data(serde_json::json!({ "task": task, "order": order }));
                record(s, TaskRec { task, entity, order, status: TaskStatus::Refused, since: now, dest: None, short_since: None, resent: false });
                return;
            }
            supersede(s, entity, "a new order");
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Engage { target: *target, volleys: None, then: Some(Box::new(Activity::Overwatch)) };
                if a.roe == RoeS::Never {
                    a.roe = RoeS::Always;
                }
            }
            control(s, entity);
        }
        ExternalOrder::FireAt { lat, lon } => {
            let aim = to_enu(s, *lat, *lon);
            let id = s.world.remove_resource::<crate::domains::fires::FiresWorld>().map(|mut st| {
                let id = crate::domains::fires::indirect::request(s, &mut st, side, Some(entity), None, aim, 2, false, "ordered from outside");
                s.world.insert_resource(st);
                id
            });
            if id.is_none() {
                let text = format!("{}: fire mission refused (no fires in this game)", name(s, entity));
                s.ctx.log.push(now, "task.refused").side(side).entity(entity).text(text).data(serde_json::json!({ "task": task, "order": order }));
                record(s, TaskRec { task, entity, order, status: TaskStatus::Refused, since: now, dest: None, short_since: None, resent: false });
                return;
            }
            status = TaskStatus::Completed;
        }
    }
    let text = format!("{}: {} ordered from outside", name(s, entity), order.word());
    s.ctx.log.push(now, "task.accepted").side(side).entity(entity).text(text).data(serde_json::json!({ "task": task, "order": order }));
    if status == TaskStatus::Completed {
        let text = format!("{}: {} done", name(s, entity), order.word());
        s.ctx.log.push(now, "task.completed").side(side).entity(entity).text(text).data(serde_json::json!({ "task": task }));
    }
    record(s, TaskRec { task, entity, order, status, since: now, dest, short_since: None, resent: false });
}

fn control(s: &mut Sim, id: EntityId) {
    if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
        b.controlled.insert(id);
    }
}

/// Clear its mission and hold where it is (it stays under outside command).
fn hold(s: &mut Sim, e: bevy_ecs::prelude::Entity) {
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        a.mission = Mission { purpose: "outside order: hold".into(), phases: vec![Activity::Hold], phase: 0, urgent: false };
        a.activity = Activity::Hold;
        a.bound = None;
    }
    if let Some(id) = s.world.get::<Ident>(e).map(|i| i.id) {
        control(s, id);
    }
}

/// A mission of move phases, as an HQ's order would be, then hold.
fn give_phases(s: &mut Sim, id: EntityId, side: Side, task: &str, points: &[(f64, f64)], mode: &str) {
    let unit = s.ctx.order.get(id.index()).and_then(|&e| s.world.get::<Ident>(e)).map(|i| i.name.clone()).unwrap_or_default();
    let mut phases: Vec<crate::hq::Phase> = points
        .iter()
        .enumerate()
        .map(|(k, &(lat, lon))| crate::hq::Phase { label: format!("{task} {}", k + 1), action: "move".into(), to: Some(crate::hq::OrderPoint { lat: Some(lat), lon: Some(lon), ..Default::default() }), mode: Some(mode.into()) })
        .collect();
    phases.push(crate::hq::Phase { label: "hold".into(), action: "hold".into(), to: None, mode: None });
    let order = crate::hq::UnitOrder { unit, task: format!("outside order: {task}"), intent: "as ordered".into(), urgency: Some("now".into()), roe: None, on_contact: None, phases };
    // Keep its rules of engagement: an order to move is not an order about firing.
    let roe = s.ctx.order.get(id.index()).and_then(|&e| s.world.get::<Act>(e)).map(|a| a.roe);
    crate::decide::apply_unit_order_forced(s, side, &order, &BTreeMap::new());
    if let (Some(r), Some(&e)) = (roe, s.ctx.order.get(id.index())) {
        if let Some(mut a) = s.world.get_mut::<Act>(e) {
            a.roe = r;
        }
    }
    control(s, id);
}

/// 1 Hz: follow running tasks to their end.
pub fn update(s: &mut Sim) {
    let now = s.ctx.now;
    let Some(running) = s.world.get_resource::<TaskBook>().map(|b| b.tasks.iter().enumerate().filter(|(_, t)| !t.status.finished()).map(|(k, t)| (k, t.clone())).collect::<Vec<_>>()) else {
        return;
    };
    for (k, t) in running {
        let Some(&e) = s.ctx.order.get(t.entity.index()) else { continue };
        let lost = s.world.get::<Health>(e).is_none_or(|h| h.destroyed);
        let (next, why) = if lost {
            (Some(TaskStatus::Error), "the unit is lost")
        } else {
            match &t.order {
                ExternalOrder::Move { .. } | ExternalOrder::Route { .. } => {
                    let at = s.world.get::<Kin>(e).map(|k| k.pos);
                    let moving = s.world.get::<Act>(e).is_some_and(|a| a.activity.moving());
                    let off = match (at, t.dest) {
                        (Some(p), Some(d)) => ((p.e - d.e).powi(2) + (p.n - d.n).powi(2)).sqrt(),
                        _ => 0.0,
                    };
                    let there = off <= ARRIVED_M;
                    if !there && !moving {
                        // Stopped short: its own drills may have halted it.
                        let since = t.short_since.unwrap_or(now);
                        let set = |s: &mut Sim, since: Option<SimTime>, resent: bool| {
                            if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
                                if let Some(x) = b.tasks.get_mut(k) {
                                    x.short_since = since;
                                    x.resent = resent;
                                }
                            }
                        };
                        if (now - since).secs() < STOPPED_SHORT_S {
                            set(s, Some(since), t.resent);
                            continue;
                        }
                        if !t.resent {
                            let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
                            let points: Vec<(f64, f64)> = match &t.order {
                                ExternalOrder::Move { lat, lon, .. } => vec![(*lat, *lon)],
                                ExternalOrder::Route { points, .. } => points.clone(),
                                _ => vec![],
                            };
                            let mode = if matches!(t.order, ExternalOrder::Move { roads: true, .. }) { "march" } else { "tactical" };
                            give_phases(s, t.entity, side, t.order.word(), &points, mode);
                            set(s, None, true);
                            let text = format!("{} stopped {off:.0} m short; sent on again", name(s, t.entity));
                            s.ctx.log.push(now, "task.resumed").entity(t.entity).text(text).data(serde_json::json!({ "task": t.task, "shortM": off }));
                            continue;
                        }
                        let why = format!("it stopped {off:.0} m short of the point");
                        if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
                            if let Some(x) = b.tasks.get_mut(k) {
                                x.status = TaskStatus::Error;
                                x.since = now;
                            }
                        }
                        let text = format!("{}: {} failed ({why})", name(s, t.entity), t.order.word());
                        s.ctx.log.push(now, "task.failed").entity(t.entity).text(text).data(serde_json::json!({ "task": t.task, "why": why }));
                        continue;
                    }
                    if !there || moving {
                        if t.short_since.is_some() {
                            if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
                                if let Some(x) = b.tasks.get_mut(k) {
                                    x.short_since = None;
                                }
                            }
                        }
                        (None, "")
                    } else if there && !moving {
                        if let ExternalOrder::Route { points, repeat: true } = &t.order {
                            // Round again.
                            let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(Side::Neutral);
                            give_phases(s, t.entity, side, "patrol", points, "tactical");
                            (None, "")
                        } else {
                            (Some(TaskStatus::Completed), "arrived")
                        }
                    } else {
                        (None, "")
                    }
                }
                ExternalOrder::Engage { target } => {
                    let dead = s.ctx.order.get(target.index()).and_then(|&x| s.world.get::<Health>(x)).is_none_or(|h| h.destroyed);
                    let engaging = s.world.get::<Act>(e).is_some_and(|a| matches!(a.activity, Activity::Engage { target: tt, .. } if tt == *target));
                    if dead {
                        (Some(TaskStatus::Completed), "the target is destroyed")
                    } else if !engaging {
                        (Some(TaskStatus::Completed), "it has stopped engaging (out of sight or reach)")
                    } else {
                        (None, "")
                    }
                }
                _ => (Some(TaskStatus::Completed), "done"),
            }
        };
        let Some(next) = next else { continue };
        if let Some(mut b) = s.world.get_resource_mut::<TaskBook>() {
            if let Some(x) = b.tasks.get_mut(k) {
                x.status = next;
                x.since = now;
            }
        }
        let kind = if next == TaskStatus::Error { "task.failed" } else { "task.completed" };
        let text = format!("{}: {} {} ({why})", name(s, t.entity), t.order.word(), if next == TaskStatus::Error { "failed" } else { "done" });
        s.ctx.log.push(now, kind).entity(t.entity).text(text).data(serde_json::json!({ "task": t.task, "why": why }));
    }
}
