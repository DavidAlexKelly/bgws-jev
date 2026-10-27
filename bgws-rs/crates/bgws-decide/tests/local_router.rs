//! The local model decider, the router, a person's seat, recorded answers and
//! the heuristic planner.

mod common;
use bgws_core::{Answer, AnswerSource, Geodetic, QuestionKind, Side};
use bgws_decide::hq::{heuristic_plan, CopTrack, HeuristicParams, HqSituation, HqUnit, PlannerKind, UnitRole};
use bgws_decide::local::probabilities_from_logprobs;
use bgws_decide::{default_figures, person_seat, Decider, FallbackDecider, JevClient, JevConfig, LocalConfig, LocalDecider, Planner, RecordedDecider, ReplayDivergence, Route, Router, RulesDecider};
use common::*;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn local(url: &str) -> LocalDecider {
    let cfg = LocalConfig { timeout: Duration::from_millis(500), ..LocalConfig::from_figures(&default_figures(), url, "qwen") };
    LocalDecider::with_http(http(), None, cfg)
}

/// Tokens `{"choice":"` then the value's first token with alternatives.
fn completion(choice: &str, alts: &[(&str, f64)]) -> Value {
    let top: Vec<Value> = alts.iter().map(|(t, lp)| json!({"token": t, "logprob": lp})).collect();
    json!({"choices": [{"message": {"role": "assistant", "content": format!("{{\"choice\":\"{choice}\"}}")}, "logprobs": {"content": [
        {"token": "{\"", "logprob": 0.0, "top_logprobs": []},
        {"token": "choice", "logprob": 0.0, "top_logprobs": []},
        {"token": "\":\"", "logprob": 0.0, "top_logprobs": []},
        {"token": choice, "logprob": alts[0].1, "top_logprobs": top},
        {"token": "\"}", "logprob": 0.0, "top_logprobs": []}
    ]}}]})
}

#[tokio::test]
async fn local_model_constrained_to_option_ids_with_logprob_probabilities() {
    let mock = serve(|_, _| Reply::json(200, completion("engage", &[("engage", (0.7f64).ln()), ("pull", (0.2f64).ln()), ("keep", (0.1f64).ln())]))).await;
    let a = local(&mock.url).decide(Side::Red, vec![land_question(1, "R1")]).await.remove(0);
    assert_eq!((a.choice.as_str(), &a.source), ("engage", &AnswerSource::Local));
    let p = a.probabilities.unwrap();
    approx::assert_relative_eq!(p["engage"], 0.7, epsilon = 1e-9);
    approx::assert_relative_eq!(p["pullBack"], 0.2, epsilon = 1e-9);
    approx::assert_relative_eq!(a.confidence.unwrap(), 0.7, epsilon = 1e-9);
    let req = mock.request(0);
    assert_eq!(req.path, "/v1/chat/completions");
    assert_eq!(req.body["response_format"]["json_schema"]["schema"]["properties"]["choice"]["enum"], json!(["keep", "engage", "pullBack"]));
    assert_eq!(req.body["logprobs"], true);
}

#[tokio::test]
async fn local_model_without_logprobs_low_confidence_and_errors() {
    let plain = serve(|_, _| Reply::json(200, json!({"choices": [{"message": {"content": "{\"choice\": \"keep\"}"}}]}))).await;
    let a = local(&plain.url).decide(Side::Red, vec![land_question(1, "R1")]).await.remove(0);
    assert_eq!((a.choice.as_str(), a.probabilities.is_none()), ("keep", true));

    let unsure = serve(|_, _| Reply::json(200, completion("engage", &[("engage", (0.2f64).ln()), ("pull", (0.2f64).ln()), ("keep", (0.2f64).ln()), ("x", 0.0)]))).await;
    let a = local(&unsure.url).decide(Side::Red, vec![land_question(1, "R1")]).await.remove(0);
    // Equal thirds over the options: 0.33 ≥ 0.25 is taken.
    assert_eq!(a.source, AnswerSource::Local);

    let down = serve(|_, _| Reply::json(500, json!({}))).await;
    let answers = local(&down.url).decide(Side::Red, vec![land_question(1, "R1"), land_question(2, "R2")]).await;
    assert!(answers.iter().all(|a| matches!(&a.source, AnswerSource::Fallback { reason } if reason.contains("local model unavailable"))));
    assert_eq!(down.calls(), 2, "one request per question, concurrently");
}

#[test]
fn logprob_reading_maps_prefixes_and_normalises() {
    let ids = vec!["fireNow".to_string(), "fireWhenLaid".to_string(), "refuse".to_string()];
    let c = completion("fireNow", &[("fire", (0.8f64).ln()), ("ref", (0.2f64).ln())]);
    let p = probabilities_from_logprobs(&c["choices"][0], &ids).unwrap();
    approx::assert_relative_eq!(p["fireNow"], 0.4, epsilon = 1e-9);
    approx::assert_relative_eq!(p["refuse"], 0.2, epsilon = 1e-9);
    assert!(probabilities_from_logprobs(&json!({}), &ids).is_none());
}

#[tokio::test]
async fn the_router_batches_by_side_and_decider_and_keeps_question_order() {
    let mock = serve(|_, req| {
        let answers: serde_json::Map<String, Value> = req.body["questions"].as_object().unwrap().keys().map(|k| (k.clone(), json!({"choice": "o0", "confidence": 0.9}))).collect();
        Reply::json(200, json!({"answers": answers})).after(Duration::from_millis(100))
    })
    .await;
    let jev: Arc<dyn Decider> = Arc::new(JevClient::with_http(http(), bgws_decide::ApiKey::new("k"), JevConfig { url: mock.url.clone(), ..JevConfig::default() }));
    let mut router = Router::rules();
    router.set_decider(Side::Blue, QuestionKind::Entity, Route::Jev, jev.clone()).set_decider(Side::Red, QuestionKind::Entity, Route::Jev, jev);
    assert_eq!(router.route(Side::Blue, QuestionKind::Entity), Route::Jev);
    assert_eq!(router.route(Side::Blue, QuestionKind::Unit), Route::Rules);
    let mut red = world_question(3);
    red.side = Side::Red;
    let mut drill = world_question(5);
    drill.options.truncate(1);
    let qs = vec![world_question(1), land_question(2, "B1"), red, world_question(4), drill];
    let started = std::time::Instant::now();
    let answers = router.answer(qs).await;
    assert!(started.elapsed() < Duration::from_millis(190), "blue and red asked concurrently: {:?}", started.elapsed());
    assert_eq!(mock.calls(), 2, "one request per side");
    assert_eq!(answers.iter().map(|a| a.question.0).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(answers[0].source, AnswerSource::Jev);
    assert_eq!((answers[1].choice.as_str(), &answers[1].source), ("engage", &AnswerSource::Rules));
    assert_eq!((answers[4].choice.as_str(), answers[4].rationale.as_deref()), ("guns", Some("only option")));
}

struct Liar;
#[async_trait::async_trait]
impl Decider for Liar {
    async fn decide(&self, _: Side, qs: Vec<bgws_core::Question>) -> Vec<Answer> {
        qs.iter().take(1).map(|q| Answer::simple(q, "teleport", AnswerSource::Jev)).collect()
    }
    fn name(&self) -> &str {
        "liar"
    }
}

#[tokio::test]
async fn invalid_or_missing_answers_become_fallbacks() {
    let mut router = Router::rules();
    router.set_decider(Side::Blue, QuestionKind::Entity, Route::Jev, Arc::new(Liar));
    let a = router.answer(vec![world_question(1), world_question(2)]).await;
    assert!(matches!(&a[0].source, AnswerSource::Fallback { reason } if reason.contains("\"teleport\", which was not offered")));
    assert!(matches!(&a[1].source, AnswerSource::Fallback { reason } if reason.contains("liar gave no answer")));
    assert_eq!(a[1].choice, "hold");
    let f = FallbackDecider::default().decide(Side::Blue, vec![world_question(3)]).await;
    assert_eq!(f[0].choice, "hold");
    let r = RulesDecider.decide(Side::Blue, vec![world_question(3)]).await;
    assert_eq!((r[0].choice.as_str(), &r[0].source), ("guns", &AnswerSource::Rules));
}

#[tokio::test]
async fn a_person_answers_through_the_seat() {
    let (seat, mut inbox) = person_seat(4, None);
    tokio::spawn(async move {
        while let Some(req) = inbox.recv().await {
            assert_eq!(req.side, Side::Blue);
            let ids: Vec<_> = req.questions.iter().map(|q| (q.id, "hold".to_string())).collect();
            req.answer(ids);
        }
    });
    let mut router = Router::rules();
    router.set_decider(Side::Blue, QuestionKind::Entity, Route::Person, Arc::new(seat.clone()));
    let a = router.answer(vec![world_question(1)]).await;
    assert_eq!((a[0].choice.as_str(), &a[0].source), ("hold", &AnswerSource::Person));

    // A seat whose person never answers, with a timeout; and an empty seat.
    let (slow, _keep) = person_seat(1, Some(Duration::from_millis(30)));
    let a = slow.decide(Side::Blue, vec![world_question(2)]).await;
    assert!(matches!(&a[0].source, AnswerSource::Fallback { reason } if reason.contains("in time")));
    let (empty, rx) = person_seat(1, None);
    drop(rx);
    let a = empty.decide(Side::Blue, vec![world_question(3)]).await;
    assert!(matches!(&a[0].source, AnswerSource::Fallback { reason } if reason.contains("no one is in the seat")));
}

#[tokio::test]
async fn recorded_answers_replay_and_divergence_is_loud() {
    let rec = vec![Answer { cost_usd: Some(0.01), ..Answer::simple(&world_question(1), "guns", AnswerSource::Jev) }];
    let d = RecordedDecider::new(rec);
    let a = d.decide(Side::Blue, vec![world_question(1)]).await;
    assert_eq!((a[0].choice.as_str(), &a[0].source, a[0].cost_usd), ("guns", &AnswerSource::Recorded, None));
    assert_eq!(d.try_answer(&[world_question(2)]), Err(ReplayDivergence::Missing(bgws_core::QuestionId(2), "b1".into())));
    let mut changed = world_question(1);
    changed.options.remove(0);
    assert!(matches!(d.try_answer(&[changed]), Err(ReplayDivergence::NotOffered(..))));

    let mut router = Router::rules();
    router.set_decider(Side::Blue, QuestionKind::Entity, Route::Recorded, Arc::new(d));
    let handle = tokio::spawn(async move { router.answer(vec![world_question(9), world_question(10)]).await });
    let err = handle.await.unwrap_err();
    assert!(err.is_panic(), "a replay divergence stops the replay");
}

fn hq_situation() -> HqSituation {
    let u = |id: &str, role, lat: f64, lon: f64, reach: Option<f64>| HqUnit { id: id.into(), label: id.into(), role, strength: 8.0, vehicles_fit: 4, vehicles_total: 4, at: Geodetic::new(lat, lon, 0.0), reach_m: reach, alive: true };
    HqSituation {
        hq: "B-HQ".into(),
        echelon: "battalion".into(),
        planner: PlannerKind::Land,
        subordinates: vec![],
        post: Some(Geodetic::new(50.0, 10.0, 0.0)),
        why: "start".into(),
        units: vec![
            u("B1", UnitRole::Manoeuvre, 50.0, 10.0, None),
            u("B2", UnitRole::Manoeuvre, 50.0, 10.01, None),
            u("B3", UnitRole::Manoeuvre, 50.0, 10.02, None),
            u("ART", UnitRole::Fires, 49.8, 10.0, Some(15000.0)),
            u("CP", UnitRole::Post, 49.95, 10.0, None),
            u("ENG", UnitRole::Engineer, 50.0, 10.0, None),
        ],
        objective: Some(Geodetic::new(50.1, 10.0, 0.0)),
        ..Default::default()
    }
}

#[test]
fn the_heuristic_attacks_with_a_reserve_when_strong_enough() {
    let p = HeuristicParams::from_figures(&default_figures());
    let plan = heuristic_plan(&hq_situation(), &p);
    assert_eq!(plan["reserve"], "B3");
    assert!(plan["plan"].as_str().unwrap().contains("take the objective"));
    let orders = plan["orders"].as_array().unwrap();
    let of = |id: &str| orders.iter().find(|o| o["unit"] == id).unwrap().clone();
    assert_eq!(of("B1")["task"], "attack: take the objective");
    // The two lead units 300 m apart across the objective.
    let to = |o: &Value| Geodetic::new(o["phases"][0]["to"]["lat"].as_f64().unwrap(), o["phases"][0]["to"]["lon"].as_f64().unwrap(), 0.0);
    approx::assert_relative_eq!(to(&of("B1")).surface_distance(to(&of("B2"))), 300.0, epsilon = 1.0);
    assert_eq!(of("B3")["task"], "reserve");
    // ART is 33 km out with 15 km reach: it moves up.
    assert_eq!(of("ART")["phases"][0]["do"], "move");
    assert_eq!(of("CP")["phases"][0]["label"], "follow the attack");
    assert_eq!(of("ENG")["phases"][0]["label"], "follow the lead, ready to breach");
    // Every order fits the schema the Claude planner is held to.
    assert!(bgws_decide::schema_check::check(&bgws_decide::hq::orders_plan_schema(), &plan).is_empty());
}

#[test]
fn the_heuristic_holds_when_outnumbered_and_halts_at_culmination() {
    let p = HeuristicParams::from_figures(&default_figures());
    let mut s = hq_situation();
    s.cop = (0..8).map(|i| CopTrack { id: format!("R{i}"), thought: "tanks".into(), domain: "land".into(), seen_ago_s: 60.0, by: "B1".into(), quality: 10, error_m: 50.0, at: Some(Geodetic::new(50.1, 10.0, 0.0)) }).collect();
    let plan = heuristic_plan(&s, &p);
    assert!(plan["plan"].as_str().unwrap().contains("hold where we stand"), "{}", plan["plan"]);
    assert!(plan.get("reserve").is_none());
    let mut s = hq_situation();
    for u in &mut s.units {
        u.vehicles_fit = 3;
    }
    assert!(heuristic_plan(&s, &p)["plan"].as_str().unwrap().contains("culminated at 25% losses"));
    s.objective = None;
    assert_eq!(heuristic_plan(&s, &p)["plan"], "hold: no objective");
}

#[tokio::test]
async fn hq_questions_go_to_the_sides_planner_heuristic_by_default() {
    let mut q = world_question(1);
    q.kind = QuestionKind::Hq;
    q.options.clear();
    q.situation = serde_json::to_string(&hq_situation()).unwrap();
    let a = Router::rules().answer(vec![q.clone()]).await.remove(0);
    assert_eq!(a.source, AnswerSource::Rules);
    assert_eq!(a.plan.unwrap()["reserve"], "B3");
    let h = bgws_decide::HeuristicPlanner::default();
    assert_eq!(h.name(), "heuristic");
}
