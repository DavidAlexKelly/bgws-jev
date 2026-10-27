//! Patches: how a derived template changes what it inherits (RUST_ENGINE.md
//! §10.1; DECISION_FLOW_GRAPHS.md §4.3, §7 step 6 and step 10).
//!
//! In the TypeScript build a variant's changes are its own fields: `remove`,
//! `bias`, `when`, `doctrine`, and node overrides in its own `nodes` (the
//! Behaviour Manager's simple and graph views write exactly these). A
//! [`Patch`] is one such edit, applied to a variant without touching its
//! parent, and [`describe_changes`] reads a variant back as the editor's
//! "Diff" panel words it.

use crate::types::{Bias, Condition, GraphNode, ResolvedTemplate, Template};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One edit to a variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Patch {
    /// Switch an option off everywhere (it can still be restored).
    Remove { option: String },
    /// Undo a removal made by this template.
    Restore { option: String },
    /// Override (or with `None`, clear) an option's bias.
    SetBias { option: String, bias: Option<Bias> },
    /// Add (or clear) an "only when…" condition on an option.
    SetWhen { option: String, when: Option<Condition> },
    SetDoctrine { text: Option<String> },
    /// Override a choose node's title, question and fallback, copying the
    /// rest from the resolved (inherited) node.
    OverrideNode { node: String, title: String, question: String, fallback: String },
    /// Drop this template's override of a node, going back to the inherited one.
    ClearNodeOverride { node: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PatchError {
    #[error("no choose node \"{0}\" to override")]
    NoChooseNode(String),
    #[error("fallback \"{fallback}\" is not an option of \"{node}\"")]
    BadFallback { node: String, fallback: String },
    #[error("bias must be -2..=2, not {0}")]
    BadBias(i8),
}

/// Apply a patch to `template`. `resolved` is the template as it resolves
/// now (needed to override an inherited node).
pub fn apply_patch(template: &Template, resolved: &ResolvedTemplate, patch: &Patch) -> Result<Template, PatchError> {
    let mut t = template.clone();
    match patch {
        Patch::Remove { option } => {
            let list = t.remove.get_or_insert_with(Vec::new);
            if !list.contains(option) {
                list.push(option.clone());
            }
        }
        Patch::Restore { option } => {
            if let Some(list) = &mut t.remove {
                list.retain(|o| o != option);
                if list.is_empty() {
                    t.remove = None;
                }
            }
        }
        Patch::SetBias { option, bias } => match bias {
            Some(b) if !(-2..=2).contains(b) => return Err(PatchError::BadBias(*b)),
            Some(b) => {
                t.bias.get_or_insert_with(Default::default).insert(option.clone(), *b);
            }
            None => {
                if let Some(m) = &mut t.bias {
                    m.remove(option);
                    if m.is_empty() {
                        t.bias = None;
                    }
                }
            }
        },
        Patch::SetWhen { option, when } => match when {
            Some(c) => {
                t.when.get_or_insert_with(Default::default).insert(option.clone(), c.clone());
            }
            None => {
                if let Some(m) = &mut t.when {
                    m.remove(option);
                    if m.is_empty() {
                        t.when = None;
                    }
                }
            }
        },
        Patch::SetDoctrine { text } => t.doctrine = text.clone().filter(|s| !s.trim().is_empty()),
        Patch::OverrideNode { node, title, question, fallback } => {
            let Some(GraphNode::Choose { options, .. }) = resolved.nodes.get(node) else { return Err(PatchError::NoChooseNode(node.clone())) };
            if !options.iter().any(|o| &o.id == fallback) {
                return Err(PatchError::BadFallback { node: node.clone(), fallback: fallback.clone() });
            }
            let n = GraphNode::Choose { title: title.clone(), question: question.clone(), options: options.clone(), fallback: fallback.clone() };
            t.nodes.get_or_insert_with(Default::default).insert(node.clone(), n);
        }
        Patch::ClearNodeOverride { node } => {
            if let Some(m) = &mut t.nodes {
                m.remove(node);
                if m.is_empty() {
                    t.nodes = None;
                }
            }
        }
    }
    Ok(t)
}

/// A variant's own changes, in words (the Behaviour Manager's diff).
pub fn describe_changes(t: &Template) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = &t.parent {
        out.push(format!("based on {p}"));
    }
    for o in t.remove.iter().flatten() {
        out.push(format!("never offers {o}"));
    }
    for (o, b) in t.bias.iter().flatten() {
        let w = match b {
            2 => "strongly preferred",
            1 => "preferred",
            0 => "neutral",
            -1 => "discouraged",
            _ => "strongly discouraged",
        };
        out.push(format!("{o}: {w}"));
    }
    for o in t.when.iter().flatten().map(|(o, _)| o) {
        out.push(format!("{o}: only when its condition holds"));
    }
    for n in t.nodes.iter().flatten().map(|(n, _)| n) {
        out.push(format!("decision {n} overridden here"));
    }
    if t.doctrine.is_some() {
        out.push("doctrine set".into());
    }
    out
}
