//! Entities owned by another simulation (docs/design/INTEROP_PLAN.md §3.2).
//!
//! Everything from outside comes in through [`apply`] as an
//! [`ExternalInput`], protocol-free (the DIS or HLA adapter makes it), and
//! only at a tick boundary. The host records every input, so a replay gives
//! them back at the same ticks with no network.
//!
//! A remote entity is moved only by its owner: between its updates it is
//! dead-reckoned here (on our ground, for a land entity). It is a target, so
//! our units see, report and fire at it, but it does not see, decide, fire
//! or feel anything here, and its damage is its owner's: what we do to it
//! goes out as a detonation for the owner to judge, and each tick its health
//! is set back to the state its owner last sent. It counts in no side's
//! strength. It is removed when its owner removes it, or when nothing has
//! come for `TIMEOUT_S`.

use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_core::{Enu, EntityId, Side, SimTime};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// No update for this long and a remote entity is taken off (2.4 DIS heartbeats), s.
pub const TIMEOUT_S: f64 = 12.0;

/// An entity's identity in another simulation (DIS site, application, entity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalKey {
    pub site: u16,
    pub application: u16,
    pub entity: u16,
}

impl std::fmt::Display for ExternalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.site, self.application, self.entity)
    }
}

/// A detonation as judged: (event, target, burst point to the metre).
pub type DetonationKey = ((u16, u16, u16), Option<EntityId>, (i64, i64));

/// What a detonation did where it went off, as the firing simulation saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetonationResult {
    /// It hit the target.
    Impact,
    /// It burst close to the target.
    Proximate,
    /// It hit the ground (or anything else).
    Ground,
}

/// One input from another simulation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "camelCase")]
pub enum ExternalInput {
    /// An entity's state: created the first time, updated after.
    #[serde(rename_all = "camelCase")]
    EntityState {
        key: ExternalKey,
        /// The catalogue platform standing in for it.
        platform: String,
        side: Side,
        label: String,
        /// East, north, height above mean sea level, m.
        pos: Enu,
        vel: Enu,
        /// Radians clockwise from north.
        heading: f64,
        /// 0 none, 1 slight, 2 moderate, 3 destroyed.
        damage: u8,
    },
    /// Its owner took it off.
    EntityRemoved { key: ExternalKey },
    /// A remote entity fired (at one of ours, if `target`).
    #[serde(rename_all = "camelCase")]
    Fired { firer: ExternalKey, target: Option<EntityId>, munition: String },
    /// A round went off; ours to judge if it was at one of ours.
    #[serde(rename_all = "camelCase")]
    Detonation {
        firer: Option<ExternalKey>,
        target: Option<EntityId>,
        /// The catalogue munition standing in for it.
        munition: String,
        at: Enu,
        result: DetonationResult,
        rounds: u32,
        /// The firing simulation's event (site, application, number); with the
        /// target and burst point, a duplicated packet is not judged twice.
        event: (u16, u16, u16),
    },
    /// The exercise was stopped (frozen) or started again.
    Freeze { frozen: bool },
}

/// Marks an entity as another simulation's.
#[derive(Component, Clone, Debug)]
pub struct Remote {
    pub key: ExternalKey,
    pub last_update: SimTime,
    /// The damage its owner last sent (0-3).
    pub damage: u8,
    /// Taken off (removed by its owner, or timed out).
    pub gone: bool,
}

/// Is this entity another simulation's?
pub fn is_remote(s: &Sim, e: Entity) -> bool {
    s.world.get::<Remote>(e).is_some()
}

/// The entity for an external key, if we have it.
pub fn entity_of(s: &Sim, key: ExternalKey) -> Option<EntityId> {
    s.ctx.remote.get(&key).copied()
}

/// Apply one input (at a tick boundary).
pub fn apply(s: &mut Sim, input: ExternalInput) {
    let now = s.ctx.now;
    match input {
        ExternalInput::EntityState { key, platform, side, label, pos, vel, heading, damage } => {
            let e = match s.ctx.remote.get(&key).map(|&id| s.ctx.entity(id)) {
                Some(e) => e,
                None => {
                    let Some(e) = spawn(s, key, &platform, side, &label, pos, heading) else { return };
                    e
                }
            };
            let land = s.world.get::<Ident>(e).is_some_and(|i| i.domain == bgws_catalogue::Domain::Land);
            let pos = if land { Enu::new(pos.e, pos.n, s.ctx.ground.height(pos.e, pos.n)) } else { pos };
            if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                k.pos = pos;
                k.vel = vel;
                k.heading = heading;
            }
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.posture = if vel.horizontal() > 0.2 { Posture::Moving } else { Posture::Halted };
            }
            let was_gone = s.world.get::<Remote>(e).is_some_and(|r| r.gone);
            if let Some(mut r) = s.world.get_mut::<Remote>(e) {
                r.last_update = now;
                r.damage = damage.min(3);
                r.gone = false;
            }
            if was_gone {
                let text = format!("{label} ({key}) is back");
                s.ctx.log.push(now, "interop.back").side(side).entity(s.world.get::<Ident>(e).map(|i| i.id).unwrap_or(EntityId(0))).text(text);
            }
            set_health(s, e);
        }
        ExternalInput::EntityRemoved { key } => {
            if let Some(id) = s.ctx.remote.get(&key).copied() {
                take_off(s, s.ctx.entity(id), "removed by its owner");
            }
        }
        ExternalInput::Fired { firer, target, munition } => {
            let Some(fid) = entity_of(s, firer) else { return };
            let Some(tid) = target.filter(|t| t.index() < s.ctx.order.len()) else { return };
            let te = s.ctx.entity(tid);
            if is_remote(s, te) {
                return;
            }
            crate::systems::fired_upon(s, te, fid);
            let text = format!("{} fires {munition} at {}", crate::systems::name(s, fid), crate::systems::name(s, tid));
            let side = s.world.get::<Ident>(s.ctx.entity(fid)).map(|i| i.side).unwrap_or(Side::Neutral);
            s.ctx.log.push(now, "interop.fire").side(side).entity(fid).target(tid).text(text).data(serde_json::json!({ "munition": munition }));
        }
        ExternalInput::Detonation { firer, target, munition, at, result, rounds, event } => {
            if !s.ctx.detonations_seen.insert((event, target, (at.e.round() as i64, at.n.round() as i64))) {
                return;
            }
            crate::systems::external_detonation(s, firer.and_then(|k| entity_of(s, k)), target, &munition, at, result, rounds, event);
        }
        ExternalInput::Freeze { frozen } => {
            s.ctx.frozen = frozen;
            let text = if frozen { "the exercise is stopped (frozen)" } else { "the exercise starts again" };
            s.ctx.log.push(now, "interop.freeze").text(text).data(serde_json::json!({ "frozen": frozen }));
        }
    }
}

fn spawn(s: &mut Sim, key: ExternalKey, platform: &str, side: Side, label: &str, pos: Enu, heading: f64) -> Option<Entity> {
    let now = s.ctx.now;
    let Some(rec) = s.ctx.catalogue.platform(platform).cloned() else {
        let text = format!("{key}: no catalogue platform {platform}; ignored");
        s.ctx.log.push(now, "interop.unknown").text(text);
        return None;
    };
    let force = bgws_scenario::v4::Force {
        id: format!("remote-{key}"),
        label: if label.is_empty() { format!("remote {key}") } else { label.to_string() },
        side,
        platform: rec.id.clone(),
        count: rec.body.count.max(1),
        quality: Default::default(),
        position: s.ctx.frame.enu_to_geodetic(Enu::new(pos.e, pos.n, 0.0)),
        heading_deg: Some(heading.to_degrees()),
        layer: bgws_scenario::v4::Layer::Entity,
        echelon: None,
        concealed: false,
        dug_in: false,
        command_rating: None,
        dummy: false,
        parent: None,
        task: None,
        orbit: None,
        provider: None,
        lane: vec![],
        status: None,
    };
    let id = EntityId(s.ctx.order.len() as u32);
    let e = crate::sim::spawn_force(&mut s.world, &mut s.ctx, id, &force, Arc::new(rec));
    s.ctx.order.push(e);
    s.ctx.by_name.insert(force.id.clone(), id);
    s.ctx.remote.insert(key, id);
    s.world.entity_mut(e).insert(Remote { key, last_update: now, damage: 0, gone: false });
    if let Some(mut a) = s.world.get_mut::<Act>(e) {
        // Its owner decides what it does; here it only stands for it.
        a.activity = Activity::Hold;
        a.roe = RoeS::Never;
    }
    let text = format!("{} ({key}, {platform}) joins from another simulation", force.label);
    s.ctx.log.push(now, "interop.joined").side(side).entity(id).text(text).data(serde_json::json!({ "key": key.to_string(), "platform": platform }));
    Some(e)
}

/// Health as its owner last said (the owner judges damage to it).
fn set_health(s: &mut Sim, e: Entity) {
    let Some((damage, gone)) = s.world.get::<Remote>(e).map(|r| (r.damage, r.gone)) else { return };
    if let Some(mut h) = s.world.get_mut::<Health>(e) {
        h.destroyed = gone || damage >= 3;
        h.fit = match damage {
            _ if h.destroyed => 0,
            2 => h.start.div_ceil(2).max(1),
            1 => h.start.saturating_sub(1).max(1),
            _ => h.start,
        };
        h.mobility_kills = 0;
        h.firepower_kills = 0;
    }
}

fn take_off(s: &mut Sim, e: Entity, why: &str) {
    let now = s.ctx.now;
    if let Some(mut r) = s.world.get_mut::<Remote>(e) {
        if r.gone {
            return;
        }
        r.gone = true;
    }
    set_health(s, e);
    let (id, side) = s.world.get::<Ident>(e).map(|i| (i.id, i.side)).unwrap_or((EntityId(0), Side::Neutral));
    let text = format!("{} is taken off ({why})", crate::systems::name(s, id));
    s.ctx.log.push(now, "interop.removed").side(side).entity(id).text(text);
}

/// 1 Hz: dead-reckon remote entities between updates (a land entity on our
/// ground), hold their health to their owner's, and take off any that went
/// quiet.
pub fn base(s: &mut Sim, dt: f64) {
    let now = s.ctx.now;
    let ids: Vec<EntityId> = s.ctx.remote.values().copied().collect();
    for id in ids {
        let e = s.ctx.entity(id);
        let Some(r) = s.world.get::<Remote>(e).cloned() else { continue };
        if r.gone {
            set_health(s, e);
            continue;
        }
        if (now - r.last_update).secs() > TIMEOUT_S {
            take_off(s, e, "no update");
            continue;
        }
        let land = s.world.get::<Ident>(e).is_some_and(|i| i.domain == bgws_catalogue::Domain::Land);
        let pos = s.world.get::<Kin>(e).map(|k| k.pos.add(k.vel.scale(dt)));
        if let Some(p) = pos {
            let p = if land { Enu::new(p.e, p.n, s.ctx.ground.height(p.e, p.n)) } else { p };
            if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                k.pos = p;
            }
        }
        set_health(s, e);
    }
}
