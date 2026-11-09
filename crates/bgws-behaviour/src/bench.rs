//! The test bench (DECISION_FLOW_GRAPHS.md §5.2, §7 step 8): put a template
//! in a scene and see what it does there: the path through the graph, every
//! option it offers and why the others are missing (removed, condition,
//! unsupported, or a hard gate), and what the rules would choose. Two
//! templates side by side on the same scene show exactly what a change does.
//!
//! A port of `behaviour/bench.ts`, made engine-free: a scene is a trigger, a
//! fact source and the gates, rather than a land game state, so the bench
//! runs the walk the game runs and nothing else. (Whether an action finds
//! something to do here, "nothing here" in the TypeScript bench, is the
//! sim's question and is left to it.)

use crate::facts::Facts;
use crate::gates::Gates;
use crate::question::highest_bias;
use crate::registry::Registry;
use crate::types::{Bias, DropReason, WalkResult};
use crate::walk::WalkError;
use bgws_core::DecisionOption;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Whether an option is on offer, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BenchStatus {
    Offered,
    Dropped(DropReason),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchRow {
    pub option: String,
    pub action: String,
    pub label: String,
    pub status: BenchStatus,
    pub bias: Bias,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    pub template_id: String,
    /// The template, then what it inherits from.
    pub lineage: Vec<String>,
    pub doctrine: Vec<String>,
    pub trigger: String,
    pub question: String,
    /// Graph nodes walked, trigger first.
    pub path: Vec<String>,
    pub rows: Vec<BenchRow>,
    /// The ids on offer, in order.
    pub offered: Vec<String>,
    pub fallback: Option<String>,
    /// What the rules decider would choose (highest bias, then first).
    pub rules_choice: Option<String>,
    /// A drill that runs without a question.
    pub drill: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BenchError {
    #[error("no template \"{0}\"")]
    UnknownTemplate(String),
    #[error(transparent)]
    Walk(#[from] WalkError),
}

/// Run a template on a scene.
pub fn bench(registry: &Registry, template_id: &str, trigger: &str, facts: &impl Facts, gates: &Gates) -> Result<BenchResult, BenchError> {
    if !registry.contains(template_id) {
        return Err(BenchError::UnknownTemplate(template_id.into()));
    }
    let t = registry.template_for_trigger(&[template_id], trigger);
    let walked = match &t {
        Some(t) => registry.walk(t, trigger, facts, gates)?,
        None => WalkResult::None { path: vec![], blocked: None },
    };
    let label = |a: &str| registry.vocabulary().action(a).map(|i| i.label.clone()).unwrap_or_else(|| a.to_owned());
    let mut out = BenchResult {
        template_id: template_id.into(),
        lineage: t.as_ref().map(|t| t.lineage.clone()).unwrap_or_else(|| registry.lineage(template_id)),
        doctrine: t.as_ref().map(|t| t.doctrine.clone()).unwrap_or_default(),
        trigger: trigger.into(),
        question: String::new(),
        path: walked.path().to_vec(),
        rows: Vec::new(),
        offered: Vec::new(),
        fallback: None,
        rules_choice: None,
        drill: None,
    };
    match &walked {
        WalkResult::Choose { question, options, fallback, dropped, .. } => {
            out.question = question.clone();
            for o in options {
                out.rows.push(BenchRow { option: o.id.clone(), action: o.action.clone(), label: label(&o.action), status: BenchStatus::Offered, bias: o.bias });
            }
            for d in dropped {
                out.rows.push(BenchRow { option: d.id.clone(), action: d.action.clone(), label: label(&d.action), status: BenchStatus::Dropped(d.why), bias: 0 });
            }
            out.offered = options.iter().map(|o| o.id.clone()).collect();
            out.fallback = fallback.clone();
            let opts: Vec<DecisionOption> = options.iter().map(|o| DecisionOption { id: o.id.clone(), action: o.action.clone(), summary: String::new(), bias: o.bias, params: Default::default() }).collect();
            out.rules_choice = (!opts.is_empty()).then(|| highest_bias(&opts));
        }
        WalkResult::Do { node_id, .. } => out.drill = Some(node_id.clone()),
        WalkResult::None { .. } => {}
    }
    Ok(out)
}

/// What differs between two bench results on the same scene.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BenchDiff {
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
    /// Option id, bias in A, bias in B.
    pub bias_changed: Vec<(String, Bias, Bias)>,
    pub rules_choice_changed: bool,
}

/// Compare two templates' results on one scene, side by side.
pub fn compare(a: &BenchResult, b: &BenchResult) -> BenchDiff {
    let bias = |r: &BenchResult, id: &str| r.rows.iter().find(|x| x.option == id && x.status == BenchStatus::Offered).map(|x| x.bias);
    BenchDiff {
        only_in_a: a.offered.iter().filter(|id| !b.offered.contains(id)).cloned().collect(),
        only_in_b: b.offered.iter().filter(|id| !a.offered.contains(id)).cloned().collect(),
        bias_changed: a.offered.iter().filter_map(|id| match (bias(a, id), bias(b, id)) {
            (Some(x), Some(y)) if x != y => Some((id.clone(), x, y)),
            _ => None,
        })
        .collect(),
        rules_choice_changed: a.rules_choice != b.rules_choice,
    }
}
