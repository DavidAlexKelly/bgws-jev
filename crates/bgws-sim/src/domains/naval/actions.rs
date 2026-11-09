//! The `sea.*` world actions (the TS `actions/naval` handlers) that the
//! warship and submarine templates' options carry out.

use super::defence::{engage, launch_salvo, launch_torpedo, LayerChoice};
use super::fleet::{alive, home_for, NavTask, ShipCombat, Vessel, VesselKind};
use super::sense::{strikers, track_of};
use super::{label, put, take};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_core::{Enu, EntityId, SimDuration};
use std::collections::BTreeMap;

/// Where the side believes a contact is (its track), else nowhere.
fn believed(s: &Sim, e: Entity, about: Option<EntityId>) -> Option<(EntityId, Enu, Enu)> {
    let side = s.world.get::<Ident>(e)?.side;
    let t = about?;
    let v = track_of(s, side, t)?;
    Some((t, v.position, v.velocity))
}

fn away(from: Enu, me: Enu, dist: f64) -> Enu {
    let d = me.sub(from);
    let h = d.horizontal().max(1.0);
    me.add(Enu::new(d.e / h, d.n / h, 0.0).scale(dist))
}

fn set_task(s: &mut Sim, e: Entity, t: NavTask) {
    if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
        v.task = t;
    }
}

fn going_home(s: &Sim, e: Entity) -> bool {
    s.world.get::<Vessel>(e).map(|v| v.task.going_home()).unwrap_or(false)
}

pub(crate) fn apply(s: &mut Sim, e: Entity, id: EntityId, action: &str, _params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    let now = s.ctx.now;
    let Some(mut st) = take(s) else { return false };
    let side = s.world.get::<Ident>(e).map(|i| i.side).unwrap_or(bgws_core::Side::Neutral);
    let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
    let sprint = SimDuration::from_secs(st.p.g("sprint_s"));
    let handled = match action {
        "sea.engageLong" | "sea.engageMedium" | "sea.engageShort" | "sea.unmask" | "sea.decoyManoeuvre" => {
            let layer = match action {
                "sea.engageLong" => LayerChoice::Long,
                "sea.engageMedium" => LayerChoice::Medium,
                "sea.engageShort" => LayerChoice::Short,
                "sea.unmask" => LayerChoice::Unmask,
                _ => LayerChoice::Decoy,
            };
            if layer == LayerChoice::Decoy {
                if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
                    v.sprint_until = now + sprint;
                }
                let text = format!("{} fires decoys and turns hard", label(s, id));
                s.ctx.log.push(now, "naval.decoy").side(side).entity(id).text(text);
            }
            if s.world.get::<ShipCombat>(e).is_some() {
                engage(s, &mut st, id, layer);
            }
            true
        }
        "sea.leaveToReload" => {
            if let Some((to, port)) = home_for(s, e) {
                let cur = s.world.get::<Vessel>(e).map(|v| v.task.clone());
                if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
                    if !v.task.going_home() {
                        v.resume = cur;
                    }
                    v.task = if port { NavTask::ToPort { port: to } } else { NavTask::Alongside { with: to, started: false } };
                }
                let d = s.world.get::<Kin>(s.ctx.entity(to)).map(|k| k.pos.sub(pos).horizontal()).unwrap_or(0.0);
                let text = format!("{} leaves station for {} to reload, {:.0} km away", label(s, id), label(s, to), d / 1000.0);
                s.ctx.log.push(now, "naval.leaveStation").side(side).entity(id).target(to).text(text);
            }
            true
        }
        "sea.conserve" => {
            if let Some(mut c) = s.world.get_mut::<ShipCombat>(e) {
                c.conserve = true;
            }
            let text = format!("{} holds its remaining missiles for self-defence", label(s, id));
            s.ctx.log.push(now, "naval.conserve").side(side).entity(id).text(text);
            true
        }
        "sea.requestEscort" => {
            let escort = s
                .ctx
                .order
                .iter()
                .copied()
                .filter(|&o| o != e && alive(s, o))
                .filter(|&o| s.world.get::<Ident>(o).map(|i| i.side == side).unwrap_or(false) && s.world.get::<Vessel>(o).map(|v| v.kind == VesselKind::Warship && !v.task.going_home()).unwrap_or(false))
                .min_by(|a, b| {
                    let d = |x: Entity| s.world.get::<Kin>(x).map(|k| k.pos.sub(pos).horizontal()).unwrap_or(f64::INFINITY);
                    d(*a).total_cmp(&d(*b))
                });
            if let Some(o) = escort {
                set_task(s, o, NavTask::Station { at: pos, radius: 5_000.0 });
                let oid = s.world.get::<Ident>(o).expect("ident").id;
                let text = format!("{} closes to escort {}", label(s, oid), label(s, id));
                s.ctx.log.push(now, "naval.escort").side(side).entity(oid).target(id).text(text);
            }
            true
        }
        "sea.engageBeyondHorizon" => {
            if let Some((t, at, vel)) = believed(s, e, about) {
                if let Some((_, from, mu, rounds, _)) = strikers(s, side).into_iter().find(|x| x.0 == id) {
                    let mun = s.ctx.catalogue.munition(&mu).cloned();
                    let salvo_m = s.world.get::<Ident>(e).and_then(|i| i.platform.mounts.iter().find(|m| m.munitions.contains(&mu)).map(|m| m.salvo)).unwrap_or(2).max(1);
                    let warship = s.world.get::<Vessel>(s.ctx.entity(t)).map(|v| v.kind == VesselKind::Warship).unwrap_or(false);
                    let n = rounds.min(salvo_m * if warship { 2 } else { 1 });
                    // Lead the track by the time of flight.
                    let tof = at.sub(from).horizontal() / mun.map(|m| m.speed_ms.max(50.0)).unwrap_or(280.0);
                    let aim = at.add(Enu::new(vel.e, vel.n, 0.0).scale(tof));
                    launch_salvo(s, &mut st, id, t, aim, &mu, n);
                }
            }
            true
        }
        "sea.closeToIdentify" | "sea.shadow" | "sea.avoid" | "sea.track" => {
            if !going_home(s, e) {
                if let Some((_, at, _)) = believed(s, e, about) {
                    let task = match action {
                        "sea.closeToIdentify" => NavTask::Route { waypoints: vec![away(pos, at, -st.p.g("identify_m") * 0.8)], then: None },
                        "sea.shadow" => NavTask::Station { at: away(pos, at, -st.p.g("shadow_m")), radius: 5_000.0 },
                        "sea.avoid" => NavTask::Route { waypoints: vec![away(at, pos, st.p.g("avoid_m"))], then: None },
                        _ => NavTask::Station { at, radius: st.p.g("shadow_m") / 4.0 },
                    };
                    set_task(s, e, task);
                }
            }
            true
        }
        "sea.attack" => {
            let t = about;
            let mount = s.world.get::<Ident>(e).and_then(|i| i.platform.mounts.iter().find(|m| m.kind == bgws_catalogue::WeaponKind::Torpedo).and_then(|m| m.munitions.first().cloned()));
            if let (Some(t), Some(mu)) = (t, mount) {
                let te = s.ctx.entity(t);
                if alive(s, te) {
                    let range = s.ctx.catalogue.munition(&mu).map(|m| m.max_range_m).unwrap_or(10_000.0);
                    let at = believed(s, e, Some(t)).map(|b| b.1).or_else(|| s.world.get::<Kin>(te).map(|k| k.pos)).unwrap_or(pos);
                    if at.sub(pos).horizontal() <= range * 0.8 {
                        launch_torpedo(s, &mut st, id, t, &mu);
                    } else if !going_home(s, e) {
                        set_task(s, e, NavTask::Strike { target: Some(t), aim: at });
                    }
                }
            }
            true
        }
        "sea.evade" => {
            let from = about.and_then(|a| s.world.get::<Kin>(s.ctx.entity(a)).map(|k| k.pos)).unwrap_or(pos.add(Enu::new(0.0, -1.0, 0.0)));
            let to = away(from, pos, st.p.g("evade_m"));
            if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
                v.deep = v.kind == VesselKind::Submarine;
                v.sprint_until = now + sprint;
                v.evading_until = now + sprint;
                v.task = NavTask::Route { waypoints: vec![to], then: None };
            }
            s.ctx.queue.schedule(now + sprint, Priority::Process, Scheduled::Timer { system: "naval.drift".into(), key: id.0 as u64 });
            let text = format!("{} evades: deep, fast, away", label(s, id));
            s.ctx.log.push(now, "naval.evade").side(side).entity(id).text(text);
            true
        }
        "sea.goSilent" => {
            let silent = SimDuration::from_secs(st.p.g("silent_s"));
            if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
                v.silent = true;
                v.silent_until = now + silent;
            }
            let text = format!("{} goes silent", label(s, id));
            s.ctx.log.push(now, "naval.silent").side(side).entity(id).text(text);
            true
        }
        "sea.gunsAndHelo" => {
            let g = SimDuration::from_secs(st.p.g("guns_s"));
            if let Some(mut c) = s.world.get_mut::<ShipCombat>(e) {
                c.guns_until = now + g;
            }
            true
        }
        "sea.withdraw" => {
            let home = home_for(s, e);
            let heading = s.world.get::<Kin>(e).map(|k| k.heading).unwrap_or(0.0) + std::f64::consts::PI;
            if let Some(mut v) = s.world.get_mut::<Vessel>(e) {
                v.sprint_until = now + sprint;
                v.task = match home {
                    Some((p, true)) => NavTask::ToPort { port: p },
                    _ => NavTask::Route { waypoints: vec![pos.add(Enu::new(heading.sin(), heading.cos(), 0.0).scale(30_000.0))], then: None },
                };
            }
            true
        }
        _ => false,
    };
    put(s, st);
    handled
}
