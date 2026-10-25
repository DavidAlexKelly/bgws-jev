//! The closed vocabulary (RUST_ENGINE.md §10.1; DECISION_FLOW_GRAPHS.md §3.1):
//! every action and fact a template may name. A port of
//! `behaviour/vocabulary.ts`: the land vocabulary (decision points D0–D12)
//! plus the world vocabulary, in one list, so a template naming anything
//! else fails validation wherever it is loaded.
//!
//! An engine that implements only some actions restricts the vocabulary
//! (`restrict_actions`); the loader then reports the others in its action
//! compatibility table instead of failing the registry (§20).

use crate::facts::{land_facts, world_facts, FactInfo};
use crate::library::{land_actions, world_actions, ActionInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The facts and actions a template may name.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Vocabulary {
    pub actions: BTreeMap<String, ActionInfo>,
    pub facts: BTreeMap<String, FactInfo>,
}

impl Vocabulary {
    /// Everything this build knows: the land and world actions and facts.
    pub fn builtin() -> Vocabulary {
        let mut v = Vocabulary::default();
        for a in land_actions().into_iter().chain(world_actions()) {
            v.actions.insert(a.name.clone(), a);
        }
        for f in land_facts().into_iter().chain(world_facts()) {
            v.facts.insert(f.name.clone(), f);
        }
        v
    }

    /// Keep only the actions an engine supports (by name).
    pub fn restrict_actions<'a>(mut self, supported: impl IntoIterator<Item = &'a str>) -> Vocabulary {
        let keep: std::collections::BTreeSet<&str> = supported.into_iter().collect();
        self.actions.retain(|name, _| keep.contains(name.as_str()));
        self
    }

    /// Add (or replace) an action, for an engine with verbs beyond the library.
    pub fn with_action(mut self, action: ActionInfo) -> Vocabulary {
        self.actions.insert(action.name.clone(), action);
        self
    }

    /// Add (or replace) a fact.
    pub fn with_fact(mut self, fact: FactInfo) -> Vocabulary {
        self.facts.insert(fact.name.clone(), fact);
        self
    }

    pub fn has_action(&self, name: &str) -> bool {
        self.actions.contains_key(name)
    }

    pub fn has_fact(&self, name: &str) -> bool {
        self.facts.contains_key(name)
    }

    pub fn action(&self, name: &str) -> Option<&ActionInfo> {
        self.actions.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_is_the_union() {
        let v = Vocabulary::builtin();
        assert_eq!(v.actions.len(), 53 + 139);
        assert_eq!(v.facts.len(), 7 + 24);
        assert!(v.has_action("land.assault") && v.has_fact("roe.weaponsFree"));
        let r = v.restrict_actions(["land.assault"]);
        assert_eq!(r.actions.len(), 1);
    }
}
