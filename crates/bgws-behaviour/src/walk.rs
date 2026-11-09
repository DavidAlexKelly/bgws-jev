//! Resolve a template against its parents, evaluate conditions, and walk the
//! graph from a trigger to the decision a decider is asked (or the drill that
//! runs). A port of `behaviour/walk.ts` (RUST_ENGINE.md §10.1;
//! DECISION_FLOW_GRAPHS.md §3.3), with the §10.1 hard gates added in
//! [`walk_gated`].
//!
//! Pure and deterministic: no randomness, no clock, no I/O. Facts come from
//! the caller's [`Facts`] source, so the same template, facts, gates and
//! trigger always give the same result, and replays stay exact.

use crate::facts::Facts;
use crate::gates::Gates;
use crate::types::{Comparison, Condition, DropReason, Dropped, FactValue, GraphNode, OfferedOption, ResolvedTemplate, Template, WalkResult};
use crate::vocabulary::Vocabulary;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// A runaway graph (a branch cycle) stops here rather than hanging the tick.
/// A safety limit of the walker, not a model figure.
pub const MAX_WALK_STEPS: usize = 32;

/// Why a template could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ResolveError {
    #[error("Template inheritance loop at \"{at}\" (from \"{from}\")")]
    Loop { at: String, from: String },
    #[error("Unknown template \"{id}\"{}", parent_of.as_ref().map(|p| format!(" (parent in \"{p}\")")).unwrap_or_default())]
    Unknown { id: String, parent_of: Option<String> },
}

/// Why a walk could not finish.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WalkError {
    #[error("Unknown fact \"{0}\"")]
    UnknownFact(String),
    #[error("Template \"{template}\": no node \"{node}\"")]
    MissingNode { template: String, node: String },
    #[error("Template \"{template}\": walk from trigger \"{trigger}\" exceeded {MAX_WALK_STEPS} steps (cycle?): {tail}")]
    Cycle { template: String, trigger: String, tail: String },
}

/// Merge a template with its ancestors. Own triggers and nodes override the
/// parent's of the same name; removals accumulate down the chain; bias and
/// `when` overrides are merged root first, so the child wins.
pub fn resolve_template(id: &str, registry: &BTreeMap<String, Template>) -> Result<ResolvedTemplate, ResolveError> {
    let mut chain: Vec<&Template> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut next = Some(id.to_owned());
    while let Some(cur) = next {
        if !seen.insert(cur.clone()) {
            return Err(ResolveError::Loop { at: cur, from: id.into() });
        }
        let t = registry.get(&cur).ok_or_else(|| ResolveError::Unknown { parent_of: (cur != id).then(|| id.to_owned()), id: cur.clone() })?;
        chain.push(t);
        next = t.parent.clone();
    }
    let mut r = ResolvedTemplate {
        id: id.into(),
        lineage: chain.iter().map(|t| t.id.clone()).collect(),
        doctrine: Vec::new(),
        triggers: BTreeMap::new(),
        nodes: BTreeMap::new(),
        remove: BTreeSet::new(),
        bias: BTreeMap::new(),
        when: BTreeMap::new(),
    };
    // Root first, so each descendant overrides what it inherits.
    for t in chain.iter().rev() {
        if let Some(d) = t.doctrine.as_ref().filter(|d| !d.is_empty()) {
            r.doctrine.push(d.clone());
        }
        r.triggers.extend(t.triggers.iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
        r.nodes.extend(t.nodes.iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
        r.remove.extend(t.remove.iter().flatten().cloned());
        r.bias.extend(t.bias.iter().flatten().map(|(k, v)| (k.clone(), *v)));
        r.when.extend(t.when.iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
    }
    Ok(r)
}

fn compare(actual: &FactValue, op: Comparison, expected: &FactValue) -> bool {
    match op {
        Comparison::Eq => return actual == expected,
        Comparison::Ne => return actual != expected,
        _ => {}
    }
    // Ordering comparisons only make sense between numbers; anything else
    // (including a fact that does not apply, null) is false rather than coerced.
    let (FactValue::Num(a), FactValue::Num(b)) = (actual, expected) else { return false };
    match op {
        Comparison::Gt => a > b,
        Comparison::Ge => a >= b,
        Comparison::Lt => a < b,
        Comparison::Le => a <= b,
        Comparison::Eq | Comparison::Ne => unreachable!("handled above"),
    }
}

/// Evaluate a condition. A fact the source does not know is an error, not false.
pub fn evaluate(cond: &Condition, facts: &impl Facts) -> Result<bool, WalkError> {
    match cond {
        Condition::All { all } => {
            for c in all {
                if !evaluate(c, facts)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Condition::Any { any } => {
            for c in any {
                if evaluate(c, facts)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Condition::Not { not } => Ok(!evaluate(not, facts)?),
        Condition::Fact { fact, op, value } => {
            let actual = facts.fact(fact).ok_or_else(|| WalkError::UnknownFact(fact.clone()))?;
            Ok(compare(&actual, *op, value))
        }
    }
}

/// The hard gates and the engine's vocabulary, for a gated walk.
#[derive(Clone, Copy, Debug)]
pub struct Masks<'a> {
    pub gates: &'a Gates,
    /// Actions outside it are dropped as `unsupported`; its traits feed the gates.
    pub vocabulary: &'a Vocabulary,
}

/// Walk from the node `trigger` points at to the first `choose` or `do` node,
/// exactly as the TypeScript walker does (no gates). Returns `none` when the
/// template has no node for this trigger, which lets the caller fall back to
/// the default template.
pub fn walk(template: &ResolvedTemplate, trigger: &str, facts: &impl Facts) -> Result<WalkResult, WalkError> {
    walk_inner(template, trigger, facts, None)
}

/// Walk with the hard gates (§10.1) applied before options are offered. Each
/// option is checked in order: removed by the template → not supported by the
/// engine → closed by a gate → its condition fails. A drill that a gate
/// closes yields `none` with the reason in `blocked`.
pub fn walk_gated(template: &ResolvedTemplate, trigger: &str, facts: &impl Facts, masks: Masks<'_>) -> Result<WalkResult, WalkError> {
    walk_inner(template, trigger, facts, Some(masks))
}

/// Is this action allowed by the engine and the gates?
fn mask(action: &str, masks: Option<Masks<'_>>) -> Option<DropReason> {
    let m = masks?;
    let Some(info) = m.vocabulary.action(action) else { return Some(DropReason::Unsupported) };
    m.gates.check(&info.traits).err().map(DropReason::Gate)
}

fn walk_inner(template: &ResolvedTemplate, trigger: &str, facts: &impl Facts, masks: Option<Masks<'_>>) -> Result<WalkResult, WalkError> {
    let mut path: Vec<String> = Vec::new();
    let Some(start) = template.triggers.get(trigger) else { return Ok(WalkResult::None { path, blocked: None }) };
    let mut node_id = start.clone();
    for _ in 0..MAX_WALK_STEPS {
        let node = template.nodes.get(&node_id).ok_or_else(|| WalkError::MissingNode { template: template.id.clone(), node: node_id.clone() })?;
        path.push(node_id.clone());
        match node {
            GraphNode::Branch { cases, otherwise } => {
                let mut hit = None;
                for c in cases {
                    if evaluate(&c.when, facts)? {
                        hit = Some(c.next.clone());
                        break;
                    }
                }
                node_id = hit.unwrap_or_else(|| otherwise.clone());
            }
            GraphNode::Do { action, params } => {
                if let Some(why) = mask(action, masks) {
                    return Ok(WalkResult::None { path, blocked: Some(why) });
                }
                return Ok(WalkResult::Do { node_id, action: action.clone(), params: params.clone().unwrap_or_default(), path });
            }
            GraphNode::Choose { title, question, options, fallback } => {
                let mut dropped = Vec::new();
                let mut offered = Vec::new();
                for o in options {
                    let why = if template.remove.contains(&o.id) {
                        Some(DropReason::Removed)
                    } else if let Some(why) = mask(&o.action, masks) {
                        Some(why)
                    } else {
                        let own = match &o.when {
                            Some(c) => evaluate(c, facts)?,
                            None => true,
                        };
                        // `when` conjunction: the option's own condition AND the template's for it.
                        let extra = match (own, template.when.get(&o.id)) {
                            (true, Some(c)) => evaluate(c, facts)?,
                            (own, _) => own,
                        };
                        (!extra).then_some(DropReason::Condition)
                    };
                    match why {
                        Some(why) => dropped.push(Dropped { id: o.id.clone(), action: o.action.clone(), why }),
                        None => offered.push(OfferedOption {
                            id: o.id.clone(),
                            action: o.action.clone(),
                            when: o.when.clone(),
                            bias: template.bias.get(&o.id).copied().or(o.bias).unwrap_or(0),
                            params: o.params.clone(),
                        }),
                    }
                }
                let fallback = if offered.iter().any(|o| &o.id == fallback) { Some(fallback.clone()) } else { offered.first().map(|o| o.id.clone()) };
                return Ok(WalkResult::Choose {
                    node_id,
                    title: title.clone(),
                    question: question.clone(),
                    doctrine: template.doctrine.clone(),
                    options: offered,
                    fallback,
                    dropped,
                    path,
                });
            }
        }
    }
    let tail = path[path.len().saturating_sub(6)..].join(" → ");
    Err(WalkError::Cycle { template: template.id.clone(), trigger: trigger.into(), tail })
}
