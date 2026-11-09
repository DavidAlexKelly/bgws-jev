//! Facts: what a template's conditions may test (RUST_ENGINE.md §10.1;
//! DECISION_FLOW_GRAPHS.md §3.1). A port of `behaviour/facts.ts` and the fact
//! half of `actions/world.ts`.
//!
//! The vocabulary is closed: the engine supplies a value for each fact named
//! here, validation rejects any other name, and the Behaviour Manager offers
//! exactly these. The walker is generic over a [`Facts`] source, so it never
//! sees the world, only what the caller (who reads a `SideView`, never truth)
//! hands it.

use crate::types::FactValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A source of fact values for the agent being decided for.
///
/// `None` means the source does not know the fact at all, which the walker
/// treats as an error (a template naming a fact not supplied fails loudly,
/// never silently false). `Some(FactValue::Null)` means "does not apply", which
/// every ordering comparison treats as false.
pub trait Facts {
    fn fact(&self, name: &str) -> Option<FactValue>;
}

impl Facts for BTreeMap<String, FactValue> {
    fn fact(&self, name: &str) -> Option<FactValue> {
        self.get(name).cloned()
    }
}

impl<T: Facts + ?Sized> Facts for &T {
    fn fact(&self, name: &str) -> Option<FactValue> {
        (**self).fact(name)
    }
}

/// No facts at all: every condition fails with an unknown-fact error. Fine
/// for templates without conditions (all the shipped ones).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFacts;

impl Facts for NoFacts {
    fn fact(&self, _name: &str) -> Option<FactValue> {
        None
    }
}

/// Facts from a closure.
pub struct FnFacts<F: Fn(&str) -> Option<FactValue>>(pub F);

impl<F: Fn(&str) -> Option<FactValue>> Facts for FnFacts<F> {
    fn fact(&self, name: &str) -> Option<FactValue> {
        (self.0)(name)
    }
}

/// Build a fact map from pairs: `fact_map([("self.strength", 0.7.into())])`.
pub fn fact_map<'a>(pairs: impl IntoIterator<Item = (&'a str, FactValue)>) -> BTreeMap<String, FactValue> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// The kind of value a fact takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactKind {
    Number,
    Boolean,
    Text,
}

/// One fact in the vocabulary, as the Behaviour Manager lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactInfo {
    pub name: String,
    pub label: String,
    pub kind: FactKind,
    /// For text facts: the values it can take (empty = open).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

fn info(name: &str, label: &str, kind: FactKind, values: &[&str], unit: Option<&str>) -> FactInfo {
    FactInfo { name: name.into(), label: label.into(), kind, values: values.iter().map(|v| v.to_string()).collect(), unit: unit.map(Into::into) }
}

use FactKind::{Boolean, Number, Text};

/// The land facts (`LAND_FACT_INFO`), in their TypeScript order.
pub fn land_facts() -> Vec<FactInfo> {
    vec![
        info("self.class", "This unit is", Text, &["armoured_vehicle", "soft_skin", "foot"], None),
        info("self.strength", "Its combat strength", Number, &[], None),
        info("self.cohesion", "Its cohesion", Text, &["steady", "shaken", "broken"], None),
        info("self.inCover", "It is in cover", Boolean, &[], None),
        info("enemy.known", "The enemy is known", Boolean, &[], None),
        info("enemy.rangeM", "Range to the enemy", Number, &[], Some("m")),
        info("orders.urgent", "Its new orders are urgent", Boolean, &[], None),
    ]
}

/// The world facts (`WORLD_FACT_INFO`), in their TypeScript order. Every world
/// question supplies all of them; one that does not apply is null.
pub fn world_facts() -> Vec<FactInfo> {
    vec![
        info("agent.domain", "It is a", Text, &["land", "air", "surface", "subsurface", "space", "node"], None),
        info("agent.archetype", "Its archetype", Text, &[], None),
        info("agent.strengthPct", "Its strength left", Number, &[], Some("%")),
        info("agent.ammoPct", "Its ammunition left", Number, &[], Some("%")),
        info("agent.fuelPct", "Its fuel left", Number, &[], Some("%")),
        info("agent.batteryPct", "Its battery left", Number, &[], Some("%")),
        info("agent.magazinePct", "Its magazine left", Number, &[], Some("%")),
        info("agent.onAirS", "Time on the air", Number, &[], Some("s")),
        info("agent.inCover", "It is in cover", Boolean, &[], None),
        info("agent.carriesDismounts", "It carries dismounts", Boolean, &[], None),
        info("agent.hasFibreDrones", "It has fibre drones", Boolean, &[], None),
        info("agent.armed", "It is armed", Boolean, &[], None),
        info("agent.canAutonomous", "It has autonomous terminal guidance", Boolean, &[], None),
        info("link.quality", "Link quality", Number, &[], None),
        info("pnt.quality", "GNSS quality here", Number, &[], None),
        info("target.rangeKm", "Range to the target", Number, &[], Some("km")),
        info("target.inRange", "The target is in range", Boolean, &[], None),
        info("target.identified", "The target is identified", Boolean, &[], None),
        info("target.costUsd", "The target's cost", Number, &[], Some("$")),
        info("threat.adWithinKm", "Nearest known air defence", Number, &[], Some("km")),
        info("threat.located", "Its own position may be located", Boolean, &[], None),
        info("env.night", "It is dark", Boolean, &[], None),
        info("env.badWeather", "The weather is bad", Boolean, &[], None),
        info("roe.weaponsFree", "Weapons free", Boolean, &[], None),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_sources() {
        let m = fact_map([("a", 1.0.into()), ("b", FactValue::Null)]);
        assert_eq!(m.fact("a"), Some(FactValue::Num(1.0)));
        assert_eq!(m.fact("b"), Some(FactValue::Null));
        assert_eq!(m.fact("c"), None);
        assert_eq!(NoFacts.fact("a"), None);
        assert_eq!(FnFacts(|n: &str| (n == "x").then_some(FactValue::Bool(true))).fact("x"), Some(FactValue::Bool(true)));
        assert_eq!(land_facts().len(), 7);
        assert_eq!(world_facts().len(), 24);
    }
}
