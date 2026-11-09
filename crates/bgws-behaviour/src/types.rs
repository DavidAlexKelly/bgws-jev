//! The shape of a behaviour template (RUST_ENGINE.md §10.1; DECISION_FLOW_GRAPHS.md §3, §6):
//! a unit type's decision graph, stored as a JSON file under `data/templates`.
//! A port of `realtime/behaviour/types.ts`, with the same JSON.
//!
//! Templates are DATA ONLY. They name facts and actions; they never contain
//! code. The engine evaluates the facts against what the agent believes and
//! walks the graph; a decider is asked only at a `choose` node, and only about
//! options that survived the template, the hard gates and their conditions.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A comparison against one named fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
}

impl Comparison {
    /// Every comparison, in the order the TypeScript build lists them.
    pub const ALL: [Comparison; 6] = [Comparison::Eq, Comparison::Ne, Comparison::Gt, Comparison::Ge, Comparison::Lt, Comparison::Le];

    pub fn as_str(self) -> &'static str {
        match self {
            Comparison::Eq => "==",
            Comparison::Ne => "!=",
            Comparison::Gt => ">",
            Comparison::Ge => ">=",
            Comparison::Lt => "<",
            Comparison::Le => "<=",
        }
    }

    pub fn parse(s: &str) -> Option<Comparison> {
        Comparison::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A fact's value: a number, a string, a boolean, or null (does not apply).
/// Whole numbers serialise without a fraction (`0`, not `0.0`), as JSON from
/// the TypeScript build does.
#[derive(Clone, Debug, PartialEq, Deserialize, Default)]
#[serde(untagged)]
pub enum FactValue {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Text(String),
}

impl Serialize for FactValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            FactValue::Null => s.serialize_unit(),
            FactValue::Bool(b) => s.serialize_bool(*b),
            FactValue::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => s.serialize_i64(*n as i64),
            FactValue::Num(n) => s.serialize_f64(*n),
            FactValue::Text(t) => s.serialize_str(t),
        }
    }
}

impl FactValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FactValue::Num(n) => Some(*n),
            _ => None,
        }
    }
}

impl From<f64> for FactValue {
    fn from(v: f64) -> Self {
        FactValue::Num(v)
    }
}
impl From<bool> for FactValue {
    fn from(v: bool) -> Self {
        FactValue::Bool(v)
    }
}
impl From<&str> for FactValue {
    fn from(v: &str) -> Self {
        FactValue::Text(v.to_owned())
    }
}
impl From<String> for FactValue {
    fn from(v: String) -> Self {
        FactValue::Text(v)
    }
}
impl<T: Into<FactValue>> From<Option<T>> for FactValue {
    fn from(v: Option<T>) -> Self {
        v.map_or(FactValue::Null, Into::into)
    }
}

/// A condition: a small expression tree over the closed fact vocabulary.
/// The variants are tried in the TypeScript evaluator's order: `all`, `any`,
/// `not`, then a single comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },
    Fact { fact: String, op: Comparison, value: FactValue },
}

impl Condition {
    /// A single comparison, for building conditions in code.
    pub fn fact(fact: &str, op: Comparison, value: impl Into<FactValue>) -> Condition {
        Condition::Fact { fact: fact.to_owned(), op, value: value.into() }
    }
}

/// Preference nudge a decider is told about, −2..=2. Never removes an option
/// (`remove` does that).
pub type Bias = i8;

/// Every bias a template may set.
pub const BIASES: [Bias; 5] = [-2, -1, 0, 1, 2];

/// Static parameters handed to an action.
pub type Params = BTreeMap<String, FactValue>;

/// One option at a `choose` node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionSpec {
    /// Unique within its node. What a decider answers with, and what traces record.
    pub id: String,
    /// An action in the action library, referred to by name.
    pub action: String,
    /// Offered only when this holds. Absent = always offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bias: Option<Bias>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
}

/// A `branch` case: the first whose condition holds is followed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub when: Condition,
    pub next: String,
}

/// A node of the graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GraphNode {
    /// Ask a decider to pick one option. `fallback` is used when it cannot answer.
    Choose { title: String, question: String, options: Vec<OptionSpec>, fallback: String },
    /// Deterministic split: the first case whose condition holds; otherwise `else`.
    Branch {
        cases: Vec<Case>,
        #[serde(rename = "else")]
        otherwise: String,
    },
    /// A fixed response: no question asked (a drill).
    Do {
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<Params>,
    },
}

/// A behaviour template, as stored on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// "core/armour", "custom/tank-stand-fast". Matches the file's path under templates/.
    pub id: String,
    pub version: u32,
    /// Display name, for the Behaviour Manager. Ignored by the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The unit archetype this is the default for, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    /// Inherit triggers and nodes from another template; own entries override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Plain-language doctrine, passed to the decider alongside every question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doctrine: Option<String>,
    /// Event kind → the node a decision starts at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<BTreeMap<String, GraphNode>>,
    /// Option ids this template removes from every choose node (inherited ones too).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<Vec<String>>,
    /// Option id → bias override, applied across every choose node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bias: Option<BTreeMap<String, Bias>>,
    /// Option id → extra condition, across every choose node: the option is
    /// offered only if its own `when` AND this hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<BTreeMap<String, Condition>>,
}

impl Template {
    /// A bare template with an id and a version, for building variants in code.
    pub fn new(id: &str, version: u32) -> Template {
        Template { id: id.to_owned(), version, name: None, archetype: None, parent: None, doctrine: None, triggers: None, nodes: None, remove: None, bias: None, when: None }
    }

    /// A variant of `parent`.
    pub fn variant(id: &str, parent: &str) -> Template {
        Template { parent: Some(parent.to_owned()), ..Template::new(id, 1) }
    }
}

/// A template with its parent chain merged in. What the walker runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTemplate {
    pub id: String,
    /// This template first, then each ancestor.
    pub lineage: Vec<String>,
    /// Root first.
    pub doctrine: Vec<String>,
    pub triggers: BTreeMap<String, String>,
    pub nodes: BTreeMap<String, GraphNode>,
    pub remove: BTreeSet<String>,
    pub bias: BTreeMap<String, Bias>,
    pub when: BTreeMap<String, Condition>,
}

/// A hard gate's name, as recorded in a drop ("gate:roe").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GateName {
    Roe,
    WeaponRelease,
    Emcon,
    Capability,
    Cohesion,
}

impl GateName {
    pub fn as_str(self) -> &'static str {
        match self {
            GateName::Roe => "roe",
            GateName::WeaponRelease => "weaponRelease",
            GateName::Emcon => "emcon",
            GateName::Capability => "capability",
            GateName::Cohesion => "cohesion",
        }
    }
    pub fn parse(s: &str) -> Option<GateName> {
        [GateName::Roe, GateName::WeaponRelease, GateName::Emcon, GateName::Capability, GateName::Cohesion].into_iter().find(|g| g.as_str() == s)
    }
}

impl fmt::Display for GateName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an option at a choose node is not on offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The template (or one it inherits from) removed it.
    Removed,
    /// Its condition (or the template's `when` for it) failed.
    Condition,
    /// The engine has no such action (the compatibility table lists it).
    Unsupported,
    /// A hard gate (§10.1) removed it: ROE, weapon release, EMCON, capability, cohesion.
    Gate(GateName),
}

impl fmt::Display for DropReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DropReason::Removed => f.write_str("removed"),
            DropReason::Condition => f.write_str("condition"),
            DropReason::Unsupported => f.write_str("unsupported"),
            DropReason::Gate(g) => write!(f, "gate:{g}"),
        }
    }
}

impl Serialize for DropReason {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DropReason {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "removed" => Ok(DropReason::Removed),
            "condition" => Ok(DropReason::Condition),
            "unsupported" => Ok(DropReason::Unsupported),
            other => other
                .strip_prefix("gate:")
                .and_then(GateName::parse)
                .map(DropReason::Gate)
                .ok_or_else(|| de::Error::custom(format!("unknown drop reason `{other}`"))),
        }
    }
}

/// An option that survived, with its effective bias.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfferedOption {
    pub id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    pub bias: Bias,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
}

/// An option that did not survive, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dropped {
    pub id: String,
    pub action: String,
    pub why: DropReason,
}

/// The result of a walk: a question to ask, a drill to run, or nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum WalkResult {
    Choose {
        node_id: String,
        title: String,
        question: String,
        doctrine: Vec<String>,
        /// Options that survived, in order.
        options: Vec<OfferedOption>,
        /// The node's fallback if it survived; otherwise the first surviving option.
        fallback: Option<String>,
        /// The node's other options, and why each is not on offer.
        dropped: Vec<Dropped>,
        /// Every node visited, trigger node first.
        path: Vec<String>,
    },
    Do { node_id: String, action: String, params: Params, path: Vec<String> },
    None {
        path: Vec<String>,
        /// Set when a drill was reached but cannot run: the gate or "unsupported".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked: Option<DropReason>,
    },
}

impl WalkResult {
    pub fn path(&self) -> &[String] {
        match self {
            WalkResult::Choose { path, .. } | WalkResult::Do { path, .. } | WalkResult::None { path, .. } => path,
        }
    }
    /// The ids on offer at a choose node (empty otherwise).
    pub fn option_ids(&self) -> Vec<&str> {
        match self {
            WalkResult::Choose { options, .. } => options.iter().map(|o| o.id.as_str()).collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_and_nodes_round_trip_the_ts_json() {
        let c: Condition = serde_json::from_str(r#"{"all":[{"fact":"ammo.main","op":">","value":0},{"not":{"fact":"x","op":"==","value":null}}]}"#).unwrap();
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"all":[{"fact":"ammo.main","op":">","value":0},{"not":{"fact":"x","op":"==","value":null}}]}"#);
        let n: GraphNode = serde_json::from_str(r#"{"kind":"branch","cases":[{"when":{"fact":"s","op":"==","value":true},"next":"a"}],"else":"b"}"#).unwrap();
        assert!(matches!(n, GraphNode::Branch { ref otherwise, .. } if otherwise == "b"));
        let r: DropReason = serde_json::from_str("\"gate:emcon\"").unwrap();
        assert_eq!(r, DropReason::Gate(GateName::Emcon));
        assert_eq!(serde_json::to_string(&DropReason::Removed).unwrap(), "\"removed\"");
    }
}
