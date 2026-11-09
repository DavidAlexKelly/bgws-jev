//! Safeguards on templates (RUST_ENGINE.md §10.6, **[ref R8]**): bounded
//! option lists, and an explicit de-escalation or withdrawal option at every
//! decision reached from a contact-like trigger. These warn (the template
//! still loads); the bound is also enforced when a question is built
//! (`question::build_question`).

use crate::registry::Registry;
use crate::types::GraphNode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Triggers that mean contact with an enemy: the land decision points where a
/// unit meets or is engaged by one, and the world triggers of the same kind.
pub const CONTACT_TRIGGERS: &[&str] = &[
    "D1", "D2", "D3", "D5", "sighted", "contact", "underFire", "missileWarning", "vampireInbound", "droneOverhead", "surfaceContact", "sonarContact",
    "counterDetected", "trackAppears", "usvSwarm", "droneAttackOnConvoy", "radarTargetedByArm", "routeThreatened", "operatorLocated",
];

/// Is this trigger contact-like (listed, or named like one)?
pub fn is_contact_trigger(trigger: &str) -> bool {
    let t = trigger.to_ascii_lowercase();
    CONTACT_TRIGGERS.contains(&trigger) || ["contact", "underfire", "sighted", "firedon", "ambush"].iter().any(|k| t.contains(k))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SafeguardIssue {
    /// No option at this decision steps back from the fight.
    NoDeescalation,
    /// More options than the bound; the excess is cut when asked.
    TooManyOptions { count: usize, max: usize },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeguardWarning {
    pub template: String,
    pub trigger: String,
    pub node: String,
    #[serde(flatten)]
    pub issue: SafeguardIssue,
}

/// Every choose node reachable from a trigger, through every branch.
pub fn reachable_choose_nodes(t: &crate::types::ResolvedTemplate, trigger: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = t.triggers.get(trigger).cloned().into_iter().collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        match t.nodes.get(&id) {
            Some(GraphNode::Choose { .. }) => out.push(id),
            Some(GraphNode::Branch { cases, otherwise }) => {
                stack.push(otherwise.clone());
                stack.extend(cases.iter().rev().map(|c| c.next.clone()));
            }
            _ => {}
        }
    }
    out.sort();
    out
}

/// Check every template in the registry, on its own triggers and on the
/// world triggers it answers through core/world (`template_for_trigger`).
/// Options removed by the template do not count; options whose action is
/// unknown to the vocabulary do not count as de-escalation.
pub fn check_safeguards(registry: &Registry, max_options: usize) -> Vec<SafeguardWarning> {
    let mut out = Vec::new();
    let world: Vec<String> = registry.resolved(crate::archetypes::WORLD_DEFAULT_TEMPLATE).map(|w| w.triggers.keys().cloned().collect()).unwrap_or_default();
    for id in registry.templates().keys() {
        let Some(own) = registry.resolved(id) else { continue };
        let triggers: BTreeSet<String> = own.triggers.keys().cloned().chain(world.iter().cloned()).collect();
        for trigger in &triggers {
            let Some(t) = registry.template_for_trigger(&[id.as_str()], trigger) else { continue };
            for node in reachable_choose_nodes(&t, trigger) {
                let Some(GraphNode::Choose { options, .. }) = t.nodes.get(&node) else { continue };
                let live: Vec<_> = options.iter().filter(|o| !t.remove.contains(&o.id)).collect();
                if live.len() > max_options {
                    out.push(SafeguardWarning { template: id.clone(), trigger: trigger.clone(), node: node.clone(), issue: SafeguardIssue::TooManyOptions { count: live.len(), max: max_options } });
                }
                if is_contact_trigger(trigger) && !live.iter().any(|o| registry.vocabulary().action(&o.action).is_some_and(|a| a.traits.deescalates)) {
                    out.push(SafeguardWarning { template: id.clone(), trigger: trigger.clone(), node, issue: SafeguardIssue::NoDeescalation });
                }
            }
        }
    }
    out
}
