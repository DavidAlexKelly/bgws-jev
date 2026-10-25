//! The joint domains' hook points into the clock (§6.1): air and missiles at
//! the fast rates, drones, EW, comms, fires, logistics and the rest at their
//! own. Each domain lives in `domains/` and is called from here, so the land
//! loop in `systems.rs` stays the same whatever the scope switches on.

use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_behaviour::{Capability, FactValue};
use bgws_core::{EntityId, Side};
use std::collections::{BTreeMap, BTreeSet};

/// After the forces are spawned: domain state, schedules.
pub fn setup(_s: &mut Sim) {}

/// Is anything fast in flight (munitions, interceptors)? Then 20 Hz.
pub fn fast_active(_s: &Sim) -> bool {
    false
}

/// Is anything flying? Then 10 Hz.
pub fn air_active(_s: &Sim) -> bool {
    false
}

pub fn fast(_s: &mut Sim, _dt: f64) {}
pub fn base(_s: &mut Sim, _dt: f64) {}
pub fn slow(_s: &mut Sim, _dt: f64) {}
pub fn minute(_s: &mut Sim, _dt: f64) {}
pub fn script_event(_s: &mut Sim, _e: &bgws_scenario::v4::ScriptedEvent) {}
pub fn scheduled(_s: &mut Sim, _ev: Scheduled) {}

/// World facts for an agent's template walk.
pub fn facts(_s: &Sim, _e: Entity, _m: &mut BTreeMap<String, FactValue>) {}

/// Domain capabilities (emitters, launchers) for the gates.
pub fn capabilities(_s: &Sim, _e: Entity, _c: &mut BTreeSet<Capability>) {}

/// Carry out a world action; false if it is not a world action.
pub fn apply_action(_s: &mut Sim, _e: Entity, _id: EntityId, _action: &str, _params: &BTreeMap<String, serde_json::Value>, _about: Option<EntityId>) -> bool {
    false
}

/// A domain victory condition met.
pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}
