//! The joint domains' hook points into the clock (§6.1). Each domain lives
//! in `domains/` and is called from here in a fixed order, so the land loop
//! in `systems.rs` stays the same whatever the scope switches on.

use crate::domains::{air, drones, fires, lod, naval, space};
use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_behaviour::{Capability, FactValue};
use bgws_core::{EntityId, Side};
use std::collections::{BTreeMap, BTreeSet};

macro_rules! each {
    ($f:ident ( $($a:expr),* )) => {{
        fires::$f($($a),*);
        air::$f($($a),*);
        drones::$f($($a),*);
        naval::$f($($a),*);
        space::$f($($a),*);
        lod::$f($($a),*);
    }};
}

/// After the forces are spawned: domain state, schedules.
pub fn setup(s: &mut Sim) {
    // Naval first: it claims the strike forces aimed at ships (`NavalStrike`),
    // which the air module then leaves to it.
    fires::setup(s);
    naval::setup(s);
    air::setup(s);
    drones::setup(s);
    space::setup(s);
    lod::setup(s);
}

/// Is anything fast in flight (munitions, interceptors)? Then 20 Hz.
pub fn fast_active(s: &Sim) -> bool {
    fires::fast_active(s) || air::fast_active(s) || drones::fast_active(s) || naval::fast_active(s) || space::fast_active(s) || lod::fast_active(s)
}

/// Is anything flying? Then 10 Hz.
pub fn air_active(s: &Sim) -> bool {
    fires::air_active(s) || air::air_active(s) || drones::air_active(s) || naval::air_active(s) || space::air_active(s) || lod::air_active(s)
}

pub fn fast(s: &mut Sim, dt: f64) {
    each!(fast(s, dt));
}
pub fn base(s: &mut Sim, dt: f64) {
    each!(base(s, dt));
}
pub fn slow(s: &mut Sim, dt: f64) {
    each!(slow(s, dt));
}
pub fn minute(s: &mut Sim, dt: f64) {
    each!(minute(s, dt));
}
pub fn script_event(s: &mut Sim, e: &bgws_scenario::v4::ScriptedEvent) {
    each!(script_event(s, e));
}

pub fn scheduled(s: &mut Sim, ev: Scheduled) {
    let mut ev = Some(ev);
    for f in [fires::scheduled, air::scheduled, drones::scheduled, naval::scheduled, space::scheduled, lod::scheduled] {
        if let Some(e) = ev.take() {
            ev = f(s, e);
        }
    }
}

/// World facts for an agent's template walk.
pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    each!(facts(s, e, m));
}

/// Domain capabilities (emitters, launchers) for the gates.
pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    each!(capabilities(s, e, c));
}

/// Carry out a world action; false if no domain knows it.
pub fn apply_action(s: &mut Sim, e: Entity, id: EntityId, action: &str, params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    fires::apply_action(s, e, id, action, params, about)
        || air::apply_action(s, e, id, action, params, about)
        || drones::apply_action(s, e, id, action, params, about)
        || naval::apply_action(s, e, id, action, params, about)
        || space::apply_action(s, e, id, action, params, about)
        || lod::apply_action(s, e, id, action, params, about)
}

/// A domain victory condition met.
pub fn victory(s: &Sim) -> Option<(Option<Side>, String)> {
    fires::victory(s).or_else(|| air::victory(s)).or_else(|| drones::victory(s)).or_else(|| naval::victory(s)).or_else(|| space::victory(s)).or_else(|| lod::victory(s))
}
