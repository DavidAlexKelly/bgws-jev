//! The air and air-defence decision points (`core/world` template, W7
//! nodes) and the `air.*` actions that answer them (TS `registerHandlers`
//! in `world/domains/air.ts`):
//!
//! | trigger              | agent        | raised when                                        |
//! |----------------------|--------------|----------------------------------------------------|
//! | `missileWarning`     | aircraft     | a missile is launched at it and its RWR (or eyes) see it |
//! | `targetInWez`        | fighter      | an enemy aircraft track is inside its missiles' envelope |
//! | `bingoFuel`          | aircraft     | fuel at bingo, or a CAP out of missiles            |
//! | `threatRingAhead`    | aircraft     | its route crosses a located AD ring                |
//! | `trackAppears`       | AD unit      | a raid comes into its zone                         |
//! | `radarTargetedByArm` | AD unit      | an anti-radiation missile is seen coming           |
//!
//! Facts and gates come from the agent's own state and its side's picture,
//! never the truth (rule 1): weapon release follows the agent's weapon
//! control status and whether the track is identified; EMCON follows its
//! radar's state.

use super::craft::{self, mount_of};
use super::defence::{go_silent, Policy};
use super::flight::{self, LaunchSpec, Purpose};
use super::radar::AirSensors;
use super::{AdUnit, AirWorld, AskContext, Craft, Task, Wcs};
use crate::components::*;
use crate::sim::{Sim, Trigger};
use crate::systems::{alive, name};
use bevy_ecs::prelude::Entity;
use bgws_behaviour::{Capability, FactValue, Gates, WeaponRelease};
use bgws_catalogue::WeaponKind;
use bgws_core::{EntityId, Enu, Side, SimDuration};
use bgws_know::{IdLevel, TrackView};
use std::collections::{BTreeMap, BTreeSet};

fn view_of(s: &Sim, side: Side, ctx: &AskContext) -> Option<TrackView> {
    let store = s.ctx.tracks.get(&side)?;
    if let Some(t) = ctx.track {
        if let Some(v) = store.view(t, &s.ctx.p.know) {
            return Some(v);
        }
    }
    let target = ctx.target?;
    let audit = store.audit();
    store
        .confirmed(&s.ctx.p.know)
        .into_iter()
        .filter(|t| audit.truth_of(t.id) == Some(target))
        .min_by(|a, b| a.age.cmp(&b.age).then(a.id.cmp(&b.id)))
}

/// Fighters with a hostile aircraft in their envelope: ask whether to shoot.
pub fn air_to_air(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    for id in aw.crafts.clone() {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let c = s.world.get::<Craft>(e).expect("c");
        if !c.airborne()
            || c.wcs == Wcs::Hold
            || s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(true)
        {
            continue;
        }
        let Some((_, mun, _)) = mount_of(s, e, &[WeaponKind::Aam]) else {
            continue;
        };
        let side = s.world.get::<Ident>(e).expect("i").side;
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        let reach = mun.max_range_m * aw.p.wez_share;
        let store = &s.ctx.tracks[&side];
        let audit = store.audit();
        let mut best: Option<(f64, EntityId, bgws_core::TrackId)> = None;
        for t in store.confirmed(&s.ctx.p.know) {
            if t.domain != bgws_know::Domain::Air || t.age.secs() > 10.0 {
                continue;
            }
            if super::defence::apparent_class(&t)
                != bgws_domains::air_defence::ThreatClass::Aircraft
            {
                continue;
            }
            let Some(truth) = audit.truth_of(t.id) else {
                continue;
            };
            if s.world.get::<Craft>(s.ctx.entity(truth)).is_none() {
                continue;
            }
            let r = t.position.sub(pos).norm();
            if r > reach {
                continue;
            }
            if aw
                .asked
                .get(&(id, truth, 0))
                .map(|x| (now - *x).secs() < aw.p.wez_ask_s)
                .unwrap_or(false)
            {
                continue;
            }
            if best.map(|b| r < b.0).unwrap_or(true) {
                best = Some((r, truth, t.id));
            }
        }
        if let Some((_, truth, track)) = best {
            // One of our missiles already on its way at it from this flight: wait.
            let busy = aw.flights.iter().any(|f| {
                s.world
                    .get::<super::InFlight>(s.ctx.entity(*f))
                    .map(|f| f.alive && f.launcher == Some(id) && f.target == Some(truth))
                    .unwrap_or(false)
            });
            if busy {
                continue;
            }
            aw.asked.insert((id, truth, 0), now);
            aw.context.insert(
                id,
                AskContext {
                    track: Some(track),
                    target: Some(truth),
                    site: None,
                    ring_m: 0.0,
                },
            );
            s.ctx.triggers.push(Trigger {
                entity: id,
                point: "targetInWez",
                about: Some(truth),
            });
        }
    }
}

/// World facts for an air or AD agent's template walk.
pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    let Some(aw) = super::state(s) else { return };
    let craft = s.world.get::<Craft>(e);
    let unit = s.world.get::<AdUnit>(e);
    if craft.is_none() && unit.is_none() {
        return;
    }
    let Some(i) = s.world.get::<Ident>(e) else {
        return;
    };
    let id = i.id;
    let side = i.side;
    let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
    m.insert(
        "agent.domain".into(),
        FactValue::Text(if craft.is_some() {
            "air".into()
        } else {
            format!("{:?}", i.domain).to_lowercase()
        }),
    );
    m.insert(
        "agent.archetype".into(),
        FactValue::Text(i.platform.id.clone()),
    );
    let start: u32 = i.platform.mounts.iter().map(|mm| mm.magazine).sum::<u32>()
        * s.world.get::<Health>(e).map(|h| h.start).unwrap_or(1);
    let rounds = s.world.get::<Arms>(e).map(|a| a.rounds()).unwrap_or(0);
    m.insert(
        "agent.ammoPct".into(),
        FactValue::Num(if start > 0 {
            (rounds as f64 / start as f64 * 100.0).min(100.0).round()
        } else {
            0.0
        }),
    );
    m.insert("agent.armed".into(), FactValue::Bool(rounds > 0));
    if let Some(c) = craft {
        m.insert(
            "agent.fuelPct".into(),
            FactValue::Num((c.fuel_share() * 100.0).round()),
        );
        m.insert(
            "roe.weaponsFree".into(),
            FactValue::Bool(c.wcs == Wcs::Free),
        );
    }
    if let Some(u) = unit {
        let (ready, mag): (u32, u32) = u.mounts.iter().fold((0, 0), |a, x| {
            (
                a.0 + x.ready_rounds,
                a.1 + x.magazine * s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1),
            )
        });
        m.insert(
            "agent.magazinePct".into(),
            FactValue::Num(if mag > 0 {
                (ready as f64 / mag as f64 * 100.0).round()
            } else {
                0.0
            }),
        );
        m.insert(
            "roe.weaponsFree".into(),
            FactValue::Bool(u.wcs == Wcs::Free),
        );
    }
    if let Some(ctx) = aw.context.get(&id) {
        if let Some(v) = view_of(s, side, ctx) {
            let r = v.position.sub(pos).norm();
            m.insert(
                "target.rangeKm".into(),
                FactValue::Num((r / 100.0).round() / 10.0),
            );
            m.insert(
                "target.identified".into(),
                FactValue::Bool(v.best_level >= IdLevel::Recognise),
            );
            let reach = unit
                .map(|u| u.mounts.iter().map(|x| x.max_range).fold(0.0, f64::max))
                .or_else(|| mount_of(s, e, &[WeaponKind::Aam]).map(|x| x.1.max_range_m))
                .unwrap_or(0.0);
            m.insert("target.inRange".into(), FactValue::Bool(r <= reach));
            let class = super::defence::apparent_class(&v);
            let cost = match class {
                bgws_domains::air_defence::ThreatClass::OneWayDrone
                | bgws_domains::air_defence::ThreatClass::Decoy => 5.0e4,
                bgws_domains::air_defence::ThreatClass::Rocket => 1.5e5,
                bgws_domains::air_defence::ThreatClass::Cruise
                | bgws_domains::air_defence::ThreatClass::SeaSkimmer => 1.5e6,
                bgws_domains::air_defence::ThreatClass::Ballistic => 3.0e6,
                bgws_domains::air_defence::ThreatClass::Aircraft => 5.0e7,
            };
            m.insert("target.costUsd".into(), FactValue::Num(cost));
        }
    }
    let nearest = aw
        .fixes
        .get(&side)
        .map(|f| {
            f.values()
                .map(|x| x.at.sub(pos).horizontal())
                .fold(f64::INFINITY, f64::min)
        })
        .unwrap_or(f64::INFINITY);
    if nearest.is_finite() {
        m.insert(
            "threat.adWithinKm".into(),
            FactValue::Num((nearest / 1000.0).round()),
        );
    }
}

/// Domain capabilities: a radar that is on.
pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    if let Some(sens) = s.world.get::<AirSensors>(e) {
        if !sens.radars.is_empty() && super::defence::radiating(s, e) {
            c.insert(Capability::Radar);
            c.insert(Capability::Emitter);
        }
    }
}

/// Weapon release from the agent's weapon control status; EMCON from its radar.
pub fn gates(s: &Sim, e: Entity, _about: Option<EntityId>, g: &mut Gates) {
    let wcs = s
        .world
        .get::<Craft>(e)
        .map(|c| c.wcs)
        .or_else(|| s.world.get::<AdUnit>(e).map(|u| u.wcs));
    let Some(wcs) = wcs else { return };
    g.weapon_release = match wcs {
        Wcs::Free => WeaponRelease::Free,
        Wcs::Tight => WeaponRelease::Tight,
        Wcs::Hold => WeaponRelease::Hold,
    };
    if let (Some(aw), Some(i)) = (super::state(s), s.world.get::<Ident>(e)) {
        if let Some(ctx) = aw.context.get(&i.id) {
            g.target_identified = view_of(s, i.side, ctx)
                .map(|v| v.best_level >= IdLevel::Recognise)
                .unwrap_or(false);
        }
    }
    if let Some(sens) = s.world.get::<AirSensors>(e) {
        if !sens.radars.is_empty() && !super::defence::radiating(s, e) {
            g.emcon = bgws_behaviour::Emcon::Restricted;
        }
    }
}

fn log(s: &mut Sim, id: EntityId, kind: &str, text: String) {
    let side = s
        .world
        .get::<Ident>(s.ctx.entity(id))
        .map(|i| i.side)
        .unwrap_or(Side::Neutral);
    s.ctx
        .log
        .push(s.ctx.now, kind.to_string())
        .side(side)
        .entity(id)
        .text(text);
}

/// Fire an air-to-air missile at the target of the question.
fn shoot(s: &mut Sim, aw: &mut AirWorld, id: EntityId, e: Entity) -> bool {
    let Some(ctx) = aw.context.get(&id).cloned() else {
        return false;
    };
    let Some(target) = ctx.target else {
        return false;
    };
    let side = s.world.get::<Ident>(e).expect("i").side;
    let Some(v) = view_of(s, side, &ctx) else {
        return false;
    };
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    let range = v.position.sub(pos).norm();
    // The short-range missile inside its envelope, the long-range one beyond it.
    let platform = s.world.get::<Ident>(e).expect("i").platform.clone();
    let arms = s.world.get::<Arms>(e).expect("arms").clone();
    let mut best: Option<(usize, bgws_catalogue::Munition)> = None;
    for m in &arms.mounts {
        let Some(cm) = platform.mounts.get(m.index) else {
            continue;
        };
        if cm.kind != WeaponKind::Aam || m.rounds == 0 {
            continue;
        }
        let Some(mun) = s.ctx.catalogue.munition(&m.munition).cloned() else {
            continue;
        };
        if range > mun.max_range_m || range < mun.min_range_m {
            continue;
        }
        if best
            .as_ref()
            .map(|b| mun.max_range_m < b.1.max_range_m)
            .unwrap_or(true)
        {
            best = Some((m.index, mun));
        }
    }
    let Some((mount, mun)) = best else {
        return false;
    };
    let speed = mun.speed_ms.max(300.0);
    let sol = bgws_domains::air::intercept(
        bgws_domains::V3::new(pos.e, pos.n, pos.u),
        speed,
        bgws_domains::V3::new(v.position.e, v.position.n, v.position.u),
        bgws_domains::V3::new(v.velocity.e, v.velocity.n, v.velocity.u),
    );
    let tgo = if sol.time_to_go.is_finite() {
        sol.time_to_go
    } else {
        range / speed
    };
    let aim = v.position.add(v.velocity.scale(tgo));
    // Energy: the Pk falls off towards maximum range.
    let pk = mun.pk * (1.0 - aw.p.energy_falloff * (range / mun.max_range_m).powi(2)).max(0.0);
    flight::launch(
        s,
        aw,
        LaunchSpec {
            side,
            launcher: Some(id),
            munition: mun.id.clone(),
            from: pos,
            to: aim,
            target: Some(target),
            purpose: Purpose::AirToAir,
            track: Some(v.id),
            pk,
        },
    );
    if let Some(m) = s
        .world
        .get_mut::<Arms>(e)
        .expect("arms")
        .mounts
        .iter_mut()
        .find(|m| m.index == mount)
    {
        m.rounds -= 1;
    }
    {
        let l = aw.ledger.entry(side).or_default();
        l.shots += 1;
        l.interceptor_cost += mun.cost_usd;
    }
    if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
        sig.last_fired = Some(s.ctx.now);
    }
    let text = format!(
        "{} fires {} at {} from {:.0} km (Pk {:.2})",
        name(s, id),
        mun.name,
        super::flight::name_any(s, target),
        range / 1000.0,
        pk
    );
    let now = s.ctx.now;
    s.ctx
        .log
        .push(now, "air.shoot")
        .side(side)
        .entity(id)
        .target(target)
        .text(text)
        .data(serde_json::json!({ "munition": mun.id, "rangeM": range, "pk": pk }));
    true
}

/// Carry out an `air.*` action. True if it was this domain's.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    s: &mut Sim,
    aw: &mut AirWorld,
    e: Entity,
    id: EntityId,
    action: &str,
    _params: &BTreeMap<String, serde_json::Value>,
    _about: Option<EntityId>,
) -> bool {
    let now = s.ctx.now;
    let ctx = aw.context.get(&id).cloned();
    match action {
        "air.defend" | "air.abort" => {
            if let Some(mut c) = s.world.get_mut::<Craft>(e) {
                c.defending_until = now + SimDuration::from_secs(aw.p.defend_s);
            }
            log(
                s,
                id,
                "air.defend",
                format!("{} defends: notch, chaff and flares", name(s, id)),
            );
            if action == "air.abort" {
                craft::rtb(s, id, "mission aborted");
            }
        }
        "air.pressAttack" | "air.holdFire" | "air.stayUp" => {}
        "air.shootNow" => {
            shoot(s, aw, id, e);
        }
        "air.closeForPk" => {
            let side = s
                .world
                .get::<Ident>(e)
                .map(|i| i.side)
                .unwrap_or(Side::Neutral);
            let to = ctx
                .as_ref()
                .and_then(|c| view_of(s, side, c))
                .map(|v| Enu::new(v.position.e, v.position.n, 0.0));
            if let (Some(to), Some(mut c)) = (to, s.world.get_mut::<Craft>(e)) {
                c.via = Some(to);
            }
        }
        "air.rtb" => craft::rtb(s, id, "ordered home"),
        "air.tanker" => {
            let side = s
                .world
                .get::<Ident>(e)
                .map(|i| i.side)
                .unwrap_or(Side::Neutral);
            let tanker = aw.crafts.iter().copied().find(|t| {
                let te = s.ctx.entity(*t);
                s.world
                    .get::<Ident>(te)
                    .map(|i| i.side == side && i.platform.id.contains("tanker"))
                    .unwrap_or(false)
                    && s.world
                        .get::<Craft>(te)
                        .map(|c| c.airborne())
                        .unwrap_or(false)
            });
            match tanker {
                Some(t) => {
                    if let Some(mut c) = s.world.get_mut::<Craft>(e) {
                        c.st.fuel = c.fuel_cap * 0.9;
                        c.bingo_asked = false;
                    }
                    let text = format!("{} refuels from {}", name(s, id), name(s, t));
                    log(s, id, "air.tanker", text);
                }
                None => craft::rtb(s, id, "no tanker airborne"),
            }
        }
        "air.onePass" => {
            if let Some(mut c) = s.world.get_mut::<Craft>(e) {
                c.until = Some(now + SimDuration::from_secs(600.0));
            }
        }
        "air.goAround" => {
            let Some(ctx) = ctx else { return true };
            let (Some(site), Some(mut c)) = (ctx.site, s.world.get_mut::<Craft>(e)) else {
                return true;
            };
            let here = Enu::new(c.st.pos.x, c.st.pos.y, 0.0);
            let margin = aw.p.detour_margin;
            match &mut c.task {
                Task::Orbit { at, radius, .. } | Task::Station { at, radius, .. } => {
                    // A station inside the ring moves out of it.
                    let d = at.sub(site);
                    if d.horizontal() < ctx.ring_m * margin {
                        let dir = if d.horizontal() > 1.0 {
                            d.scale(1.0 / d.horizontal())
                        } else {
                            Enu::new(0.0, -1.0, 0.0)
                        };
                        *at = site.add(dir.scale(ctx.ring_m * margin + *radius));
                    }
                }
                Task::Strike { aim, .. } if aim.sub(site).horizontal() < ctx.ring_m * margin => {
                    // The target sits inside the ring: no way round, so under
                    // it — low, below the site's radar horizon.
                    c.low = true;
                }
                _ => {
                    let b = here.sub(site).bearing() + std::f64::consts::FRAC_PI_2;
                    c.via =
                        Some(site.add(Enu::new(b.sin(), b.cos(), 0.0).scale(ctx.ring_m * margin)));
                }
            }
        }
        "air.goLow" => {
            if let Some(mut c) = s.world.get_mut::<Craft>(e) {
                c.low = true;
            }
        }
        "air.requestSead" => {
            if let Some(mut c) = s.world.get_mut::<Craft>(e) {
                c.until = c.until.map(|u| u.max(now + SimDuration::from_secs(600.0)));
            }
            aw.next_tasking = now;
        }
        "air.engageCheapest" | "air.engageSurest" | "air.holdTight" => {
            if let Some(mut u) = s.world.get_mut::<AdUnit>(e) {
                match action {
                    "air.engageCheapest" => {
                        u.policy = Policy::Cheapest;
                        u.wcs = Wcs::Free;
                    }
                    "air.engageSurest" => {
                        u.policy = Policy::Surest;
                        u.wcs = Wcs::Free;
                    }
                    _ => u.wcs = Wcs::Tight,
                }
            }
        }
        "air.goSilentLetPass" => go_silent(s, aw, id, "letting the raid pass"),
        "air.shutDown" => {
            let by = ctx
                .and_then(|c| c.target)
                .map(|t| name(s, t))
                .unwrap_or_else(|| "an attacker".into());
            go_silent(s, aw, id, &format!("an anti-radiation missile from {by}"));
        }
        "air.relocateRadar" => {
            go_silent(s, aw, id, "relocating");
            let k = *s.world.get::<Kin>(e).expect("k");
            let mut rng = s.ctx.rng("ad.relocate", id.0 as u64, 0);
            let b: f64 = rand::Rng::gen::<f64>(&mut rng) * std::f64::consts::TAU;
            let to = k
                .pos
                .add(Enu::new(b.sin(), b.cos(), 0.0).scale(aw.p.relocate_m));
            if s.world
                .get::<Ident>(e)
                .map(|i| i.domain == bgws_catalogue::Domain::Land)
                .unwrap_or(false)
            {
                let route = crate::decide::route_for(s, e, to);
                s.world.get_mut::<Act>(e).expect("a").activity = Activity::Move {
                    route,
                    mode: MoveModeS::March,
                    then: Some(Box::new(Activity::Hold)),
                };
            }
        }
        "air.acceptNineLine" | "air.acceptWithDelay" | "air.declineNineLine" => {
            log(
                s,
                id,
                "air.nineLine",
                format!("{}: {}", name(s, id), action.trim_start_matches("air.")),
            );
        }
        _ => return false,
    }
    true
}
