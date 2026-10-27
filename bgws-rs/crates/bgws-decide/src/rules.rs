//! The rules decider (RUST_ENGINE.md §10.2: "the batch runner uses the rules
//! decider by default so overnight runs cost nothing") and the plain fallback
//! decider. Both answer at once, deterministically, and cost nothing.

use crate::decider::Decider;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Question, Side};

/// Picks each question's `rules_choice` (highest bias then first, or the
/// template's or engine's rule, worked out when the question was built).
#[derive(Clone, Copy, Debug, Default)]
pub struct RulesDecider;

#[async_trait]
impl Decider for RulesDecider {
    async fn decide(&self, _side: Side, questions: Vec<Question>) -> Vec<Answer> {
        questions.iter().map(rules_answer).collect()
    }
    fn name(&self) -> &str {
        "rules"
    }
}

/// The rules' answer to one question.
pub fn rules_answer(q: &Question) -> Answer {
    let choice = if q.has_option(&q.rules_choice) || q.options.is_empty() { q.rules_choice.clone() } else { q.fallback.clone() };
    Answer { rationale: Some("the rules chose".into()), ..Answer::simple(q, choice, AnswerSource::Rules) }
}

/// Takes the template's fallback for every question, saying why.
#[derive(Clone, Debug)]
pub struct FallbackDecider {
    pub reason: String,
}

impl Default for FallbackDecider {
    fn default() -> Self {
        FallbackDecider { reason: "no decider for this seat: the template's fallback".into() }
    }
}

#[async_trait]
impl Decider for FallbackDecider {
    async fn decide(&self, _side: Side, questions: Vec<Question>) -> Vec<Answer> {
        questions.iter().map(|q| Answer::fallback(q, self.reason.clone())).collect()
    }
    fn name(&self) -> &str {
        "fallback"
    }
}
