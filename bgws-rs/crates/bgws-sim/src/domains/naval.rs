//! Domain systems: naval. See `crate::world` for the hook points.

use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_behaviour::{Capability, FactValue};
use bgws_core::{EntityId, Side};
use std::collections::{BTreeMap, BTreeSet};

pub fn setup(_s: &mut Sim) {}
pub fn fast_active(_s: &Sim) -> bool {
    false
}
pub fn air_active(_s: &Sim) -> bool {
    false
}
pub fn fast(_s: &mut Sim, _dt: f64) {}
pub fn base(_s: &mut Sim, _dt: f64) {}
pub fn slow(_s: &mut Sim, _dt: f64) {}
pub fn minute(_s: &mut Sim, _dt: f64) {}
pub fn script_event(_s: &mut Sim, _e: &bgws_scenario::v4::ScriptedEvent) {}
/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(_s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    Some(ev)
}
pub fn facts(_s: &Sim, _e: Entity, _m: &mut BTreeMap<String, FactValue>) {}
pub fn capabilities(_s: &Sim, _e: Entity, _c: &mut BTreeSet<Capability>) {}
pub fn apply_action(_s: &mut Sim, _e: Entity, _id: EntityId, _action: &str, _params: &BTreeMap<String, serde_json::Value>, _about: Option<EntityId>) -> bool {
    false
}
pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}
