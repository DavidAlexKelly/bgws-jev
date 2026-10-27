//! From a walk to a [`bgws_core::Question`] (RUST_ENGINE.md §3.2, §10.1,
//! §10.6). The question-building half of `world/questions.ts`'s
//! `raiseQuestion`, generalised for land and world agents alike:
//!
//! - a drill (`do` node) becomes a one-option question, title "<trigger> (drill)";
//! - each offered option becomes a [`DecisionOption`] with words (the
//!   engine's summary if it gives one, else the action's label), its bias and
//!   params, or several when the engine expands it (one per target: `id:key`);
//! - the fallback is the walk's, else its first expansion, else the first option;
//! - the rules' choice is the highest bias then the first, or the template's
//!   rule (its fallback), or the engine's own rule if it names an option;
//! - the option list is bounded (§10.6), always keeping the fallback and the
//!   rules' choice.

use crate::types::{FactValue, WalkResult};
use crate::vocabulary::Vocabulary;
use bgws_core::{DecisionOption, Figures, Question, QuestionId, QuestionKind, SimDuration, SimTime, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How the rules decider picks, when no one else answers.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RulesChoice {
    /// The option with the highest bias; the first on a tie.
    #[default]
    HighestBias,
    /// The template's fallback (the land engine's `ruleFallback` rule).
    Fallback,
    /// The engine's own rule chose this option id (ignored if not offered).
    Option(String),
}

/// One concrete option an action expands to here (one per target, say).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expansion {
    pub key: String,
    pub summary: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// Everything about the agent and the moment a question needs besides the walk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestionContext {
    pub id: QuestionId,
    pub at: SimTime,
    pub kind: QuestionKind,
    pub agent: String,
    pub side: Side,
    pub trigger: String,
    /// The resolved template's id.
    pub template: String,
    /// The agent's beliefs in words (only what reached it).
    pub situation: String,
    /// Drawn by the sim from skill, fatigue and drill (§10.1).
    pub reaction: SimDuration,
    /// Option id → the engine's words for it; else the action's label.
    #[serde(default)]
    pub summaries: BTreeMap<String, String>,
    /// Option id → its concrete expansions; an empty list means "nothing of
    /// that kind here" and drops the option.
    #[serde(default)]
    pub expansions: BTreeMap<String, Vec<Expansion>>,
    #[serde(default)]
    pub rule: RulesChoice,
}

/// The §10.6 bound on options, from the figures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionLimits {
    pub max_options: usize,
}

impl QuestionLimits {
    pub fn from_figures(f: &Figures) -> QuestionLimits {
        QuestionLimits { max_options: f.get("behaviour.safeguards.max_options").round().max(1.0) as usize }
    }
}

fn param_json(p: &BTreeMap<String, FactValue>) -> BTreeMap<String, serde_json::Value> {
    p.iter().map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null))).collect()
}

/// Build the question for a walk. `None` when there is nothing to ask or do
/// (no node for the trigger, a blocked drill, or no option left).
///
/// A question with one option is a drill or the only option: the host takes
/// it without asking anyone (see [`needs_asking`]).
pub fn build_question(walk: &WalkResult, ctx: &QuestionContext, vocab: &Vocabulary, limits: QuestionLimits) -> Option<Question> {
    let words = |id: &str, action: &str| ctx.summaries.get(id).cloned().or_else(|| vocab.action(action).map(|a| a.label.clone())).unwrap_or_else(|| action.to_owned());
    let (node, title, question, doctrine, mut options, walk_fallback) = match walk {
        WalkResult::None { .. } => return None,
        WalkResult::Do { node_id, action, params, .. } => {
            let o = DecisionOption { id: node_id.clone(), action: action.clone(), summary: words(node_id, action), bias: 0, params: param_json(params) };
            (node_id.clone(), format!("{} (drill)", ctx.trigger), String::new(), Vec::new(), vec![o], Some(node_id.clone()))
        }
        WalkResult::Choose { node_id, title, question, doctrine, options, fallback, .. } => {
            let mut out = Vec::new();
            for o in options {
                let base = o.params.as_ref().map(param_json).unwrap_or_default();
                match ctx.expansions.get(&o.id) {
                    None => out.push(DecisionOption { id: o.id.clone(), action: o.action.clone(), summary: words(&o.id, &o.action), bias: o.bias, params: base }),
                    Some(list) => {
                        for x in list {
                            let mut params = base.clone();
                            params.extend(x.params.clone());
                            out.push(DecisionOption { id: format!("{}:{}", o.id, x.key), action: o.action.clone(), summary: x.summary.clone(), bias: o.bias, params });
                        }
                    }
                }
            }
            (node_id.clone(), title.clone(), question.clone(), doctrine.clone(), out, fallback.clone())
        }
    };
    if options.is_empty() {
        return None;
    }
    let fallback = match walk_fallback {
        Some(f) if options.iter().any(|o| o.id == f) => f,
        Some(f) => options.iter().find(|o| o.id.starts_with(&format!("{f}:"))).map_or_else(|| options[0].id.clone(), |o| o.id.clone()),
        None => options[0].id.clone(),
    };
    let rules_choice = match &ctx.rule {
        RulesChoice::Option(id) if options.iter().any(|o| &o.id == id) => id.clone(),
        RulesChoice::Fallback => fallback.clone(),
        _ => highest_bias(&options),
    };
    options = bound(options, limits.max_options, &[&fallback, &rules_choice]);
    Some(Question {
        id: ctx.id,
        at: ctx.at,
        kind: ctx.kind,
        agent: ctx.agent.clone(),
        side: ctx.side,
        trigger: ctx.trigger.clone(),
        template: ctx.template.clone(),
        node,
        title,
        question,
        doctrine,
        situation: ctx.situation.clone(),
        options,
        fallback,
        rules_choice,
        reaction: ctx.reaction,
        plan_schema: None,
    })
}

/// Highest bias, then the first in order.
pub fn highest_bias(options: &[DecisionOption]) -> String {
    let mut best: Option<&DecisionOption> = None;
    for o in options {
        if best.is_none_or(|b| o.bias > b.bias) {
            best = Some(o);
        }
    }
    best.map(|o| o.id.clone()).unwrap_or_default()
}

/// Keep at most `max` options in their order, always keeping the `keep` ids.
fn bound(options: Vec<DecisionOption>, max: usize, keep: &[&String]) -> Vec<DecisionOption> {
    if options.len() <= max {
        return options;
    }
    let must = options.iter().filter(|o| keep.contains(&&o.id)).count();
    let mut room = max.saturating_sub(must);
    options
        .into_iter()
        .filter(|o| {
            if keep.contains(&&o.id) {
                true
            } else if room > 0 {
                room -= 1;
                true
            } else {
                false
            }
        })
        .collect()
}

/// Does this question need a decider? One option (a drill, or the only one
/// left) is taken at once (DECISION_FLOW_GRAPHS.md §3.3).
pub fn needs_asking(q: &Question) -> bool {
    q.options.len() > 1
}
