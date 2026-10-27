//! Replay without calling anyone (RUST_ENGINE.md §12: "model answers are
//! recorded in the game record, so a game with Jev and Claude replays
//! exactly"). A question the record does not hold, or a recorded choice that
//! is no longer on offer, is a replay divergence: it fails loudly.

use crate::decider::{Decider, Planner};
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Question, QuestionId, Side};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReplayDivergence {
    #[error("replay diverged: question {0:?} ({1}) is not in the record")]
    Missing(QuestionId, String),
    #[error("replay diverged: question {0:?} recorded \"{1}\", which is no longer offered")]
    NotOffered(QuestionId, String),
}

/// Answers from a game record, keyed by question id.
#[derive(Clone, Debug, Default)]
pub struct RecordedDecider {
    answers: BTreeMap<QuestionId, Answer>,
}

impl RecordedDecider {
    pub fn new(record: impl IntoIterator<Item = Answer>) -> RecordedDecider {
        RecordedDecider { answers: record.into_iter().map(|a| (a.question, a)).collect() }
    }

    pub fn len(&self) -> usize {
        self.answers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }

    /// The recorded answers, or the first divergence.
    pub fn try_answer(&self, questions: &[Question]) -> Result<Vec<Answer>, ReplayDivergence> {
        questions
            .iter()
            .map(|q| {
                let a = self.answers.get(&q.id).ok_or_else(|| ReplayDivergence::Missing(q.id, q.agent.clone()))?;
                if !q.options.is_empty() && a.plan.is_none() && !q.has_option(&a.choice) {
                    return Err(ReplayDivergence::NotOffered(q.id, a.choice.clone()));
                }
                Ok(Answer { source: AnswerSource::Recorded, cost_usd: None, latency_ms: Some(0), ..a.clone() })
            })
            .collect()
    }
}

#[async_trait]
impl Decider for RecordedDecider {
    /// Panics on a divergence: a replay that no longer matches its record must
    /// stop, not quietly play on with other answers. Use [`RecordedDecider::try_answer`]
    /// to handle it instead.
    async fn decide(&self, _side: Side, questions: Vec<Question>) -> Vec<Answer> {
        match self.try_answer(&questions) {
            Ok(a) => a,
            Err(e) => panic!("{e}"),
        }
    }
    fn name(&self) -> &str {
        "recorded"
    }
}

#[async_trait]
impl Planner for RecordedDecider {
    async fn plan(&self, question: Question) -> Answer {
        match self.try_answer(std::slice::from_ref(&question)) {
            Ok(mut a) => a.remove(0),
            Err(e) => panic!("{e}"),
        }
    }
    fn name(&self) -> &str {
        "recorded"
    }
}
