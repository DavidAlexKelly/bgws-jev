//! The decider interface the host calls (RUST_ENGINE.md §3.2, §3.3, §10).
//! A decider sees only `Question`s, which the sim built from what each agent
//! believes (rule 1), and returns `Answer`s; it never sees the world.

use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Question, Side};

/// Answers a side's unit and entity questions at one moment.
///
/// Implementations return one answer per question, in any order; the
/// [`Router`](crate::router::Router) matches them by question id and fills a
/// missing or invalid one with the template's fallback.
#[async_trait]
pub trait Decider: Send + Sync {
    async fn decide(&self, side: Side, questions: Vec<Question>) -> Vec<Answer>;
    fn name(&self) -> &str;
}

/// Answers an HQ's plan request (`QuestionKind::Hq`) with an answer whose
/// `plan` holds the structured orders.
#[async_trait]
pub trait Planner: Send + Sync {
    async fn plan(&self, question: Question) -> Answer;
    fn name(&self) -> &str;
}

/// Put answers in question order; anything missing, or naming an option that
/// was not offered, becomes the fallback with the reason (an HQ answer with a
/// plan is kept whatever its choice).
pub fn reconcile(questions: &[Question], mut answers: Vec<Answer>, who: &str) -> Vec<Answer> {
    questions
        .iter()
        .map(|q| match answers.iter().position(|a| a.question == q.id) {
            Some(i) => {
                let a = answers.swap_remove(i);
                let valid = q.options.is_empty() || q.has_option(&a.choice) || a.plan.is_some();
                if valid {
                    a
                } else {
                    Answer::fallback(q, format!("{who} answered \"{}\", which was not offered", a.choice))
                }
            }
            None => Answer::fallback(q, format!("{who} gave no answer")),
        })
        .collect()
}

/// Is this answer a fallback?
pub fn is_fallback(a: &Answer) -> bool {
    matches!(a.source, AnswerSource::Fallback { .. })
}
