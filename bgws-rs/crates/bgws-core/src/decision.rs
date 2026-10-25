//! What crosses the sans-IO boundary (§3.2): the simulation stops at a
//! decision point and hands out questions; the host gets answers from Jev,
//! Claude, a person or the rules, and hands them back.

use crate::ids::Side;
use crate::time::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct QuestionId(pub u64);

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    /// A land unit's decision point (D0–D12 style).
    Unit,
    /// A world entity's decision point (a mortar, a drone, a destroyer).
    Entity,
    /// An HQ's plan request: answered with structured orders.
    Hq,
}

/// One option on offer, already masked by capability, gates and the template.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub id: String,
    pub action: String,
    /// Plain words for Jev and for people.
    pub summary: String,
    /// Doctrine nudge, −2..=2.
    #[serde(default)]
    pub bias: i8,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    pub at: SimTime,
    pub kind: QuestionKind,
    /// The agent's stable scenario id.
    pub agent: String,
    pub side: Side,
    pub trigger: String,
    pub template: String,
    pub node: String,
    pub title: String,
    pub question: String,
    pub doctrine: Vec<String>,
    /// The agent's beliefs, in words: only what reached it (rule 1).
    pub situation: String,
    pub options: Vec<DecisionOption>,
    /// Taken when no decider can answer.
    pub fallback: String,
    /// What the rules decider would pick.
    pub rules_choice: String,
    /// The answer takes effect this long after `at`.
    pub reaction: SimDuration,
    /// For HQ questions: the JSON Schema the plan must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_schema: Option<serde_json::Value>,
}

impl Question {
    pub fn has_option(&self, id: &str) -> bool {
        self.options.iter().any(|o| o.id == id)
    }
    /// A stable cache key: the same situation and options give the same key.
    pub fn cache_key(&self) -> u64 {
        let mut s = String::new();
        s.push_str(&self.template);
        s.push('|');
        s.push_str(&self.node);
        s.push('|');
        s.push_str(&self.situation);
        for o in &self.options {
            s.push('|');
            s.push_str(&o.id);
            s.push_str(&o.bias.to_string());
        }
        crate::rng::fnv1a(s.as_bytes())
    }
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AnswerSource {
    Jev,
    Claude,
    Local,
    Rules,
    Person,
    Cache,
    /// Replayed from a game record without calling anyone.
    Recorded,
    /// The template's fallback, and why.
    Fallback { reason: String },
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub question: QuestionId,
    pub choice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub source: AnswerSource,
    /// HQ answers: the structured orders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Answer {
    pub fn simple(q: &Question, choice: impl Into<String>, source: AnswerSource) -> Self {
        Answer { question: q.id, choice: choice.into(), probabilities: None, confidence: None, source, plan: None, cost_usd: None, latency_ms: None, rationale: None }
    }
    pub fn fallback(q: &Question, reason: impl Into<String>) -> Self {
        Self::simple(q, q.fallback.clone(), AnswerSource::Fallback { reason: reason.into() })
    }
}
