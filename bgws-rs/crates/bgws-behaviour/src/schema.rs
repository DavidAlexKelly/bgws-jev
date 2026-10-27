//! Validate behaviour templates (RUST_ENGINE.md §10.1; DECISION_FLOW_GRAPHS.md
//! §5.2, §6). A port of `behaviour/schema.ts`. Run over every JSON file at
//! load and in the tests, so a broken template fails CI, never a game.
//!
//! Two levels:
//! - [`validate_template`]: one file on its own: shape, ids, internal
//!   references, and facts/actions against the vocabulary.
//! - [`validate_registry`]: all files together: parents exist, no inheritance
//!   loops, no child removes an option its inherited fallback depends on.
//!
//! Both return every problem found, each a typed [`Problem`] that prints as
//! the TypeScript message did, prefixed with where it is.

use crate::types::{Comparison, GraphNode, Template, BIASES};
use crate::vocabulary::Vocabulary;
use crate::walk::{resolve_template, ResolveError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// What is wrong.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProblemKind {
    NotAnObject,
    MissingId,
    BadVersion,
    BadParent,
    NodesNotObject,
    TriggersNotObject,
    /// A trigger, case or `else` names a node that does not exist.
    NoNode { node: String },
    /// As `NoNode`, after inheritance (registry level).
    NoNodeAfterInheritance { node: String },
    /// A branch's `else` names a node that does not exist.
    ElseNoNode { node: String },
    NodeNotObject,
    MissingTitle,
    MissingQuestion,
    NoOptions,
    OptionNotObject,
    MissingOptionId,
    DuplicateOption { id: String },
    ActionNotString,
    /// The action is not in the vocabulary (§20: reported in the compatibility table).
    UnknownAction { action: String },
    BadBias,
    /// A value in the template-wide `bias` map is not −2..=2.
    BadBiasValue,
    MissingFallback,
    FallbackNotOption { fallback: String },
    NoCases,
    CaseNotObject,
    BadKind,
    ConditionNotObject,
    EmptyList,
    NoFactKey,
    UnknownFact { fact: String },
    BadOp,
    BadValue,
    RemoveNotList,
    WhenNotObject,
    BiasNotObject,
    /// File level: the id does not match the file's path.
    IdPathMismatch { id: String, expected: String },
    DuplicateTemplate { id: String },
    /// The file is not JSON.
    Parse { message: String },
    /// Registry level: parents and loops.
    Resolve { message: String },
    RemovesFallback { fallback: String },
    AllRemoved,
}

/// One problem, with where it is ("core/x.nodes.D1.options[2]").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub at: String,
    #[serde(flatten)]
    pub kind: ProblemKind,
}

impl Problem {
    fn new(at: impl Into<String>, kind: ProblemKind) -> Problem {
        Problem { at: at.into(), kind }
    }
    /// The same problem, with a file path in front.
    pub fn in_file(mut self, path: &str) -> Problem {
        self.at = format!("{path}: {}", self.at);
        self
    }
    pub fn is_unknown_action(&self) -> bool {
        matches!(self.kind, ProblemKind::UnknownAction { .. })
    }
}

impl fmt::Display for ProblemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ProblemKind::*;
        let ops: Vec<&str> = Comparison::ALL.iter().map(|c| c.as_str()).collect();
        match self {
            NotAnObject => f.write_str("template must be a JSON object"),
            MissingId => f.write_str("\"id\" is required"),
            BadVersion => f.write_str("\"version\" must be a number"),
            BadParent => f.write_str("\"parent\" must be a template id"),
            NodesNotObject => f.write_str("\"nodes\" must be an object"),
            TriggersNotObject => f.write_str("\"triggers\" must be an object"),
            NoNode { node } => write!(f, "no node \"{node}\""),
            NoNodeAfterInheritance { node } => write!(f, "no node \"{node}\" (after inheritance)"),
            NodeNotObject | OptionNotObject | CaseNotObject => f.write_str("must be an object"),
            MissingTitle => f.write_str("\"title\" is required"),
            MissingQuestion => f.write_str("\"question\" is required"),
            NoOptions => f.write_str("needs at least one option"),
            MissingOptionId => f.write_str("\"id\" is required"),
            DuplicateOption { id } => write!(f, "duplicate option id \"{id}\""),
            ActionNotString => f.write_str("\"action\" must be a non-empty string"),
            UnknownAction { action } => write!(f, "unknown action \"{action}\""),
            BadBias => f.write_str("\"bias\" must be -2, -1, 0, 1 or 2"),
            BadBiasValue => f.write_str("must be -2, -1, 0, 1 or 2"),
            ElseNoNode { node } => write!(f, "\"else\" → no node \"{node}\""),
            MissingFallback => f.write_str("\"fallback\" is required"),
            FallbackNotOption { fallback } => write!(f, "fallback \"{fallback}\" is not one of its options"),
            NoCases => f.write_str("needs at least one case"),
            BadKind => f.write_str("\"kind\" must be \"choose\", \"branch\" or \"do\""),
            ConditionNotObject => f.write_str("condition must be an object"),
            EmptyList => f.write_str("\"all\"/\"any\" needs a non-empty list"),
            NoFactKey => f.write_str("needs \"fact\", \"all\", \"any\" or \"not\""),
            UnknownFact { fact } => write!(f, "unknown fact \"{fact}\""),
            BadOp => write!(f, "\"op\" must be one of {}", ops.join(" ")),
            BadValue => f.write_str("\"value\" must be a number, string, boolean or null"),
            RemoveNotList => f.write_str("\"remove\" must be a list of option ids"),
            WhenNotObject => f.write_str("\"when\" must be an object of option id → condition"),
            BiasNotObject => f.write_str("\"bias\" must be an object"),
            IdPathMismatch { id, expected } => write!(f, "\"id\" is \"{id}\" but the file's path says \"{expected}\""),
            DuplicateTemplate { id } => write!(f, "duplicate template id \"{id}\""),
            Parse { message } => write!(f, "not valid JSON: {message}"),
            Resolve { message } => f.write_str(message),
            RemovesFallback { fallback } => write!(f, "removes \"{fallback}\", which is this decision's fallback — keep it, or override the node with another fallback"),
            AllRemoved => f.write_str("every option is removed"),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.kind)
    }
}

fn is_str(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::String(s)) if !s.is_empty())
}
fn str_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "null".into(),
        None => "undefined".into(),
        Some(other) => other.to_string(),
    }
}
fn is_bias(v: &Value) -> bool {
    v.as_f64().is_some_and(|b| b.fract() == 0.0 && BIASES.contains(&(b as i8)) && (-2.0..=2.0).contains(&b))
}

fn check_condition(c: &Value, at: &str, vocab: Option<&Vocabulary>, out: &mut Vec<Problem>) {
    let Some(obj) = c.as_object() else { return out.push(Problem::new(at, ProblemKind::ConditionNotObject)) };
    if obj.contains_key("all") || obj.contains_key("any") {
        // `c.all ?? c.any`, as the TypeScript does.
        let list = obj.get("all").filter(|v| !v.is_null()).or_else(|| obj.get("any"));
        let name = if obj.contains_key("all") { "all" } else { "any" };
        match list.and_then(Value::as_array) {
            Some(items) if !items.is_empty() => {
                for (i, x) in items.iter().enumerate() {
                    check_condition(x, &format!("{at}.{name}[{i}]"), vocab, out);
                }
            }
            _ => out.push(Problem::new(at, ProblemKind::EmptyList)),
        }
        return;
    }
    if let Some(not) = obj.get("not") {
        return check_condition(not, &format!("{at}.not"), vocab, out);
    }
    if !is_str(obj.get("fact")) {
        out.push(Problem::new(at, ProblemKind::NoFactKey));
    } else if let Some(v) = vocab {
        let fact = str_of(obj.get("fact"));
        if !v.has_fact(&fact) {
            out.push(Problem::new(at, ProblemKind::UnknownFact { fact }));
        }
    }
    if !obj.get("op").and_then(Value::as_str).is_some_and(|op| Comparison::parse(op).is_some()) {
        out.push(Problem::new(at, ProblemKind::BadOp));
    }
    if !matches!(obj.get("value"), Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))) {
        out.push(Problem::new(at, ProblemKind::BadValue));
    }
}

fn check_action(action: Option<&Value>, at: &str, vocab: Option<&Vocabulary>, out: &mut Vec<Problem>) {
    if !is_str(action) {
        out.push(Problem::new(at, ProblemKind::ActionNotString));
    } else if let Some(v) = vocab {
        let name = str_of(action);
        if !v.has_action(&name) {
            out.push(Problem::new(at, ProblemKind::UnknownAction { action: name }));
        }
    }
}

/// Check one template on its own. Returns every problem found (empty = valid).
/// Without a vocabulary, facts and actions are not checked.
pub fn validate_template(raw: &Value, vocab: Option<&Vocabulary>) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(t) = raw.as_object() else { return vec![Problem::new("(no id)", ProblemKind::NotAnObject)] };
    let id = if is_str(t.get("id")) { str_of(t.get("id")) } else { "(no id)".into() };
    let at = |w: &str| format!("{id}{w}");

    if !is_str(t.get("id")) {
        out.push(Problem::new(id.clone(), ProblemKind::MissingId));
    }
    if !t.get("version").is_some_and(|v| v.as_u64().is_some()) {
        out.push(Problem::new(id.clone(), ProblemKind::BadVersion));
    }
    if t.contains_key("parent") && !is_str(t.get("parent")) {
        out.push(Problem::new(id.clone(), ProblemKind::BadParent));
    }

    let empty = Map::new();
    let nodes_v = t.get("nodes").filter(|v| !v.is_null());
    let nodes = match nodes_v {
        None => Some(&empty),
        Some(v) => v.as_object(),
    };
    if nodes.is_none() {
        out.push(Problem::new(id.clone(), ProblemKind::NodesNotObject));
    }
    let node_ids: BTreeSet<&String> = nodes.map(|n| n.keys().collect()).unwrap_or_default();
    // A child may point at nodes it inherits, so unresolved references are
    // only errors for root templates; validate_registry re-checks resolved ones.
    let own_refs_only = !t.contains_key("parent");
    let ref_ok = |r: Option<&Value>| is_str(r) && (!own_refs_only || node_ids.contains(&str_of(r)));

    if let Some(trig) = t.get("triggers") {
        match trig.as_object() {
            None => out.push(Problem::new(id.clone(), ProblemKind::TriggersNotObject)),
            Some(map) => {
                for (evt, target) in map {
                    if !ref_ok(Some(target)) {
                        out.push(Problem::new(at(&format!(".triggers.{evt}")), ProblemKind::NoNode { node: str_of(Some(target)) }));
                    }
                }
            }
        }
    }

    for (nid, node) in nodes.into_iter().flatten() {
        let w = at(&format!(".nodes.{nid}"));
        let Some(n) = node.as_object() else {
            out.push(Problem::new(w, ProblemKind::NodeNotObject));
            continue;
        };
        match n.get("kind").and_then(Value::as_str) {
            Some("choose") => {
                if !is_str(n.get("title")) {
                    out.push(Problem::new(w.clone(), ProblemKind::MissingTitle));
                }
                if !is_str(n.get("question")) {
                    out.push(Problem::new(w.clone(), ProblemKind::MissingQuestion));
                }
                let options = n.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
                if options.is_empty() {
                    out.push(Problem::new(w.clone(), ProblemKind::NoOptions));
                }
                let mut seen = BTreeSet::new();
                for (i, o) in options.iter().enumerate() {
                    let ow = format!("{w}.options[{i}]");
                    let Some(o) = o.as_object() else {
                        out.push(Problem::new(ow, ProblemKind::OptionNotObject));
                        continue;
                    };
                    if !is_str(o.get("id")) {
                        out.push(Problem::new(ow.clone(), ProblemKind::MissingOptionId));
                    } else {
                        let oid = str_of(o.get("id"));
                        if !seen.insert(oid.clone()) {
                            out.push(Problem::new(ow.clone(), ProblemKind::DuplicateOption { id: oid }));
                        }
                    }
                    check_action(o.get("action"), &ow, vocab, &mut out);
                    if let Some(c) = o.get("when") {
                        check_condition(c, &format!("{ow}.when"), vocab, &mut out);
                    }
                    if o.get("bias").is_some_and(|b| !is_bias(b)) {
                        out.push(Problem::new(ow.clone(), ProblemKind::BadBias));
                    }
                }
                if !is_str(n.get("fallback")) {
                    out.push(Problem::new(w.clone(), ProblemKind::MissingFallback));
                } else {
                    let fb = str_of(n.get("fallback"));
                    if !seen.contains(&fb) {
                        out.push(Problem::new(w.clone(), ProblemKind::FallbackNotOption { fallback: fb }));
                    }
                }
            }
            Some("branch") => {
                let cases = n.get("cases").and_then(Value::as_array).cloned().unwrap_or_default();
                if cases.is_empty() {
                    out.push(Problem::new(w.clone(), ProblemKind::NoCases));
                }
                for (i, c) in cases.iter().enumerate() {
                    let cw = format!("{w}.cases[{i}]");
                    let Some(c) = c.as_object() else {
                        out.push(Problem::new(cw, ProblemKind::CaseNotObject));
                        continue;
                    };
                    check_condition(c.get("when").unwrap_or(&Value::Null), &format!("{cw}.when"), vocab, &mut out);
                    if !ref_ok(c.get("next")) {
                        out.push(Problem::new(cw, ProblemKind::NoNode { node: str_of(c.get("next")) }));
                    }
                }
                if !ref_ok(n.get("else")) {
                    out.push(Problem::new(w.clone(), ProblemKind::ElseNoNode { node: str_of(n.get("else")) }));
                }
            }
            Some("do") => check_action(n.get("action"), &w, vocab, &mut out),
            _ => out.push(Problem::new(w, ProblemKind::BadKind)),
        }
    }

    if let Some(r) = t.get("remove") {
        if !r.as_array().is_some_and(|l| l.iter().all(|x| is_str(Some(x)))) {
            out.push(Problem::new(id.clone(), ProblemKind::RemoveNotList));
        }
    }
    if let Some(wh) = t.get("when") {
        match wh.as_object() {
            None => out.push(Problem::new(id.clone(), ProblemKind::WhenNotObject)),
            Some(map) => {
                for (oid, c) in map {
                    check_condition(c, &at(&format!(".when.{oid}")), vocab, &mut out);
                }
            }
        }
    }
    if let Some(b) = t.get("bias") {
        match b.as_object() {
            None => out.push(Problem::new(id.clone(), ProblemKind::BiasNotObject)),
            Some(map) => {
                for (oid, v) in map {
                    if !is_bias(v) {
                        out.push(Problem::new(at(&format!(".bias.{oid}")), ProblemKind::BadBiasValue));
                    }
                }
            }
        }
    }
    out
}

/// Check all templates together: parents, inheritance loops, removed
/// fallbacks, references after inheritance.
pub fn validate_registry(registry: &BTreeMap<String, Template>) -> Vec<Problem> {
    let mut out = Vec::new();
    for id in registry.keys() {
        let resolved = match resolve_template(id, registry) {
            Ok(r) => r,
            Err(e @ (ResolveError::Loop { .. } | ResolveError::Unknown { .. })) => {
                out.push(Problem::new(id.clone(), ProblemKind::Resolve { message: e.to_string() }));
                continue;
            }
        };
        for (evt, target) in &resolved.triggers {
            if !resolved.nodes.contains_key(target) {
                out.push(Problem::new(format!("{id}.triggers.{evt}"), ProblemKind::NoNodeAfterInheritance { node: target.clone() }));
            }
        }
        for (nid, node) in &resolved.nodes {
            let at = format!("{id}.nodes.{nid}");
            match node {
                GraphNode::Branch { cases, otherwise } => {
                    for next in cases.iter().map(|c| &c.next).chain(std::iter::once(otherwise)) {
                        if !resolved.nodes.contains_key(next) {
                            out.push(Problem::new(at.clone(), ProblemKind::NoNodeAfterInheritance { node: next.clone() }));
                        }
                    }
                }
                GraphNode::Choose { options, fallback, .. } => {
                    if resolved.remove.contains(fallback) {
                        out.push(Problem::new(at.clone(), ProblemKind::RemovesFallback { fallback: fallback.clone() }));
                    }
                    if options.iter().all(|o| resolved.remove.contains(&o.id)) {
                        out.push(Problem::new(at.clone(), ProblemKind::AllRemoved));
                    }
                }
                GraphNode::Do { .. } => {}
            }
        }
    }
    out
}
