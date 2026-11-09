//! Who answers what (RUST_ENGINE.md §3.3, §10.2–10.5): per side and per
//! question kind, a decider (rules, Jev, Claude, a local model, a person or a
//! record); per side, a planner for HQ questions. The router batches one
//! side's questions for one decider into one call, runs every call
//! concurrently, and hands back one answer per question in question order,
//! filling anything missing or invalid with the template's fallback.
//!
//! A [`PersonSeat`] lets a human answer through a tokio channel: the host's UI
//! receives a [`PersonRequest`] and replies to it.

use crate::decider::{reconcile, Decider, Planner};
use crate::hq::HeuristicPlanner;
use crate::rules::RulesDecider;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Question, QuestionId, QuestionKind, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// The kinds of decider a seat can have, for scenario settings and the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Rules,
    Fallback,
    Jev,
    Claude,
    Local,
    Person,
    Recorded,
}

fn kind_index(k: QuestionKind) -> u8 {
    match k {
        QuestionKind::Unit => 0,
        QuestionKind::Entity => 1,
        QuestionKind::Hq => 2,
    }
}

/// The routing table.
#[derive(Clone)]
pub struct Router {
    deciders: BTreeMap<(Side, u8), (Route, Arc<dyn Decider>)>,
    planners: BTreeMap<Side, (Route, Arc<dyn Planner>)>,
    default_decider: Arc<dyn Decider>,
    default_planner: Arc<dyn Planner>,
}

impl Router {
    /// Everything on the rules and the heuristic planner: free, instant,
    /// deterministic (the batch runner's default).
    pub fn rules() -> Router {
        Router { deciders: BTreeMap::new(), planners: BTreeMap::new(), default_decider: Arc::new(RulesDecider), default_planner: Arc::new(HeuristicPlanner::default()) }
    }

    /// Route a side's unit or entity questions to a decider.
    pub fn set_decider(&mut self, side: Side, kind: QuestionKind, route: Route, decider: Arc<dyn Decider>) -> &mut Self {
        self.deciders.insert((side, kind_index(kind)), (route, decider));
        self
    }

    /// Route a side's HQ questions to a planner.
    pub fn set_planner(&mut self, side: Side, route: Route, planner: Arc<dyn Planner>) -> &mut Self {
        self.planners.insert(side, (route, planner));
        self
    }

    /// Which route answers this side's questions of this kind.
    pub fn route(&self, side: Side, kind: QuestionKind) -> Route {
        if kind == QuestionKind::Hq {
            return self.planners.get(&side).map(|(r, _)| *r).unwrap_or(Route::Rules);
        }
        self.deciders.get(&(side, kind_index(kind))).map(|(r, _)| *r).unwrap_or(Route::Rules)
    }

    fn decider(&self, side: Side, kind: QuestionKind) -> Arc<dyn Decider> {
        self.deciders.get(&(side, kind_index(kind))).map(|(_, d)| d.clone()).unwrap_or_else(|| self.default_decider.clone())
    }

    /// Answer every question. One option (a drill, or the only one left) is
    /// taken at once without asking (DECISION_FLOW_GRAPHS.md §3.3). A task
    /// that panics (a replay divergence) propagates its panic.
    pub async fn answer(&self, questions: Vec<Question>) -> Vec<Answer> {
        let mut set = tokio::task::JoinSet::new();
        let mut immediate: Vec<Answer> = Vec::new();
        // Group by side and decider instance: one call per decider per side.
        let mut groups: Vec<(Side, Arc<dyn Decider>, Vec<Question>)> = Vec::new();
        for q in &questions {
            if q.kind == QuestionKind::Hq {
                let planner = self.planners.get(&q.side).map(|(_, p)| p.clone()).unwrap_or_else(|| self.default_planner.clone());
                let q = q.clone();
                set.spawn(async move { vec![planner.plan(q).await] });
                continue;
            }
            if q.options.len() == 1 {
                immediate.push(Answer { rationale: Some("only option".into()), ..Answer::simple(q, q.options[0].id.clone(), AnswerSource::Rules) });
                continue;
            }
            let d = self.decider(q.side, q.kind);
            match groups.iter_mut().find(|(s, g, _)| *s == q.side && Arc::ptr_eq(g, &d)) {
                Some((_, _, qs)) => qs.push(q.clone()),
                None => groups.push((q.side, d, vec![q.clone()])),
            }
        }
        for (side, d, qs) in groups {
            set.spawn(async move {
                let name = d.name().to_owned();
                let answers = d.decide(side, qs.clone()).await;
                reconcile(&qs, answers, &name)
            });
        }
        let mut all = immediate;
        while let Some(r) = set.join_next().await {
            match r {
                Ok(a) => all.extend(a),
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                Err(_) => {}
            }
        }
        reconcile(&questions, all, "router")
    }
}

/// A question set waiting for a person, with the way to reply.
#[derive(Debug)]
pub struct PersonRequest {
    pub side: Side,
    pub questions: Vec<Question>,
    reply: oneshot::Sender<Vec<Answer>>,
}

impl PersonRequest {
    /// Reply with a choice per question (and, for an HQ, use [`PersonRequest::reply`] with a plan).
    pub fn answer(self, choices: impl IntoIterator<Item = (QuestionId, String)>) {
        let answers = choices
            .into_iter()
            .filter_map(|(id, c)| self.questions.iter().find(|q| q.id == id).map(|q| Answer { rationale: Some("a person chose".into()), ..Answer::simple(q, c, AnswerSource::Person) }))
            .collect();
        let _ = self.reply.send(answers);
    }
    /// Reply with full answers (the source is set to Person).
    pub fn reply(self, answers: Vec<Answer>) {
        let _ = self.reply.send(answers.into_iter().map(|a| Answer { source: AnswerSource::Person, ..a }).collect());
    }
}

/// A human in a seat: questions go out on a channel, answers come back.
#[derive(Clone, Debug)]
pub struct PersonSeat {
    tx: mpsc::Sender<PersonRequest>,
    /// How long to wait; `None` = as long as it takes (the clock waits).
    pub timeout: Option<Duration>,
}

/// A seat and the receiving end for the UI.
pub fn person_seat(buffer: usize, timeout: Option<Duration>) -> (PersonSeat, mpsc::Receiver<PersonRequest>) {
    let (tx, rx) = mpsc::channel(buffer.max(1));
    (PersonSeat { tx, timeout }, rx)
}

impl PersonSeat {
    async fn ask(&self, side: Side, questions: Vec<Question>) -> Vec<Answer> {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(PersonRequest { side, questions: questions.clone(), reply }).await.is_err() {
            return questions.iter().map(|q| Answer::fallback(q, "no one is in the seat")).collect();
        }
        let got = match self.timeout {
            Some(t) => tokio::time::timeout(t, rx).await.map_err(|_| "the person did not answer in time"),
            None => Ok(rx.await),
        };
        match got {
            Ok(Ok(a)) => reconcile(&questions, a, "the person"),
            Ok(Err(_)) => questions.iter().map(|q| Answer::fallback(q, "the person left the seat")).collect(),
            Err(why) => questions.iter().map(|q| Answer::fallback(q, why)).collect(),
        }
    }
}

#[async_trait]
impl Decider for PersonSeat {
    async fn decide(&self, side: Side, questions: Vec<Question>) -> Vec<Answer> {
        self.ask(side, questions).await
    }
    fn name(&self) -> &str {
        "person"
    }
}

#[async_trait]
impl Planner for PersonSeat {
    async fn plan(&self, question: Question) -> Answer {
        let side = question.side;
        self.ask(side, vec![question]).await.remove(0)
    }
    fn name(&self) -> &str {
        "person"
    }
}
