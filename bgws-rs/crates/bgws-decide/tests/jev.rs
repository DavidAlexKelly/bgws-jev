//! The Jev client against a local mock of the Decisions API: the batch shape
//! (one request per side, one choice question per agent, as
//! `jevWorldDecider.test.ts` and `jevRealtimeDecider` ask), confident answers,
//! low confidence, timeout, server error, no key, and the cache.

mod common;
use bgws_core::{AnswerSource, Side};
use bgws_decide::jev::{bias_words, parse_choices};
use bgws_decide::{ApiKey, Decider, JevClient, JevConfig};
use common::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

fn client(url: &str, key: Option<&str>) -> JevClient {
    let cfg = JevConfig { url: format!("{url}/api/alpha/decisions"), timeout: Duration::from_millis(500), ..JevConfig::default() };
    JevClient::with_http(http(), key.and_then(ApiKey::new), cfg)
}

/// Answer every question with o0 at this confidence.
fn answer_all(confidence: f64) -> impl Fn(usize, &Seen) -> Reply {
    move |_, req| {
        let answers: serde_json::Map<String, Value> = req.body["questions"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| (k.clone(), json!({"type": "choice", "choice": "o0", "probabilities": {"o0": 0.9, "o1": 0.1}, "confidence": confidence})))
            .collect();
        Reply::json(200, json!({"answers": answers, "usage": {"cost": 0.002}}))
    }
}

#[tokio::test]
async fn asks_one_choice_per_question_in_one_request_and_takes_a_confident_answer() {
    let mock = serve(answer_all(0.9)).await;
    let jev = client(&mock.url, Some("sk-or-test"));
    let answers = jev.decide(Side::Blue, vec![world_question(1), world_question(2)]).await;
    assert_eq!(mock.calls(), 1, "one request per side per moment");
    let req = mock.request(0);
    assert_eq!((req.method.as_str(), req.path.as_str()), ("POST", "/api/alpha/decisions"));
    assert_eq!(req.header("authorization"), Some("Bearer sk-or-test"));
    assert_eq!(req.header("x-title"), Some("BGWS"));
    assert_eq!(req.body["model"], "typesafe/jev-1.13");
    assert_eq!(req.body["state"]["you"], "blue");
    let qs = req.body["questions"].as_object().unwrap();
    assert_eq!(qs.keys().collect::<Vec<_>>(), ["q0", "q1"]);
    let q = &qs["q0"];
    assert_eq!(q["type"], "choice");
    assert!(q["instructions"].as_str().unwrap().contains("Mass fires"));
    assert!(q["instructions"].as_str().unwrap().contains("You decide for b1, on the blue side."));
    assert_eq!(q["criteria"]["o0"], "the guns [preferred by its doctrine]");
    assert!(q["criteria"]["o1"].as_str().unwrap().contains("strongly discouraged"));

    assert_eq!(answers.iter().map(|a| (a.question.0, a.choice.as_str())).collect::<Vec<_>>(), [(1, "guns"), (2, "guns")]);
    let a = &answers[0];
    assert_eq!(a.source, AnswerSource::Jev);
    assert_eq!(a.confidence, Some(0.9));
    assert_eq!(a.probabilities.as_ref().unwrap(), &BTreeMap::from([("guns".to_string(), 0.9), ("hold".to_string(), 0.1)]));
    approx::assert_relative_eq!(a.cost_usd.unwrap(), 0.001);
    assert!(a.latency_ms.is_some());
    approx::assert_relative_eq!(jev.spent(Side::Blue), 0.002);
    assert!(!format!("{answers:?}").contains("sk-or-test"), "the key never appears in an answer");
}

#[tokio::test]
async fn land_units_are_asked_as_the_realtime_decider_asks() {
    let mock = serve(answer_all(0.8)).await;
    let jev = client(&mock.url, Some("k"));
    let answers = jev.decide(Side::Red, vec![land_question(1, "R1"), land_question(2, "R2"), world_question(3)]).await;
    let body = mock.request(0).body;
    let u0 = &body["questions"]["u0"];
    let text = u0["instructions"].as_str().unwrap();
    assert!(text.starts_with("You lead R1. The intent comes first"), "{text}");
    assert!(text.contains("Decision point D1, new contact. R1 has run into the contact R3"));
    assert!(text.contains("Also deciding now, so choose to work together: R2."));
    assert!(text.ends_with("The crew has already done its drill (cover, and return fire if it can see the shooter)."));
    // "keep" keeps its own key, as in the TypeScript.
    assert_eq!(u0["criteria"].as_object().unwrap().keys().collect::<Vec<_>>(), ["keep", "o1", "o2"]);
    assert!(body["questions"]["q0"].is_object());
    assert!(body["state"]["note"].as_str().unwrap().starts_with("Only what each unit believes"));
    // o0 is not a key of a land question that starts with keep: no readable answer for it.
    assert!(matches!(answers[0].source, AnswerSource::Fallback { .. }));
    assert_eq!(answers[2].choice, "guns");
}

#[tokio::test]
async fn falls_back_when_unsure() {
    let mock = serve(answer_all(0.1)).await;
    let [a] = <[_; 1]>::try_from(client(&mock.url, Some("k")).decide(Side::Blue, vec![world_question(3)]).await).unwrap();
    assert_eq!(a.choice, "hold");
    let AnswerSource::Fallback { reason } = &a.source else { panic!("{a:?}") };
    assert!(reason.contains("unsure"), "{reason}");
    assert_eq!(a.confidence, Some(0.1), "what Jev said is kept for the log");
}

#[tokio::test]
async fn falls_back_on_timeout_with_the_reason() {
    let mock = serve(|_, _| Reply::json(200, json!({})).after(Duration::from_secs(3))).await;
    let a = client(&mock.url, Some("k")).decide(Side::Blue, vec![world_question(4)]).await.remove(0);
    let AnswerSource::Fallback { reason } = &a.source else { panic!() };
    assert!(reason.contains("no answer within 500 ms"), "{reason}");
    assert_eq!(a.choice, "hold");
}

#[tokio::test]
async fn falls_back_on_a_server_error_and_on_no_key() {
    let mock = serve(|_, _| Reply::json(500, json!({"error": {"message": "upstream down"}}))).await;
    let a = client(&mock.url, Some("k")).decide(Side::Blue, vec![world_question(5)]).await.remove(0);
    let AnswerSource::Fallback { reason } = &a.source else { panic!() };
    assert!(reason.contains("OpenRouter 500: upstream down"), "{reason}");

    let a = client(&mock.url, None).decide(Side::Blue, vec![world_question(6)]).await.remove(0);
    let AnswerSource::Fallback { reason } = &a.source else { panic!() };
    assert!(reason.contains("no OpenRouter key"));
    assert_eq!(mock.calls(), 1, "no key: nothing is sent");

    let unreadable = serve(|_, _| Reply::json(200, json!({"answers": {}}))).await;
    let a = client(&unreadable.url, Some("k")).decide(Side::Blue, vec![world_question(7)]).await.remove(0);
    assert!(matches!(&a.source, AnswerSource::Fallback { reason } if reason.contains("no readable answers")));
}

#[tokio::test]
async fn the_cache_answers_a_repeated_situation_without_asking() {
    let mock = serve(answer_all(0.9)).await;
    let jev = client(&mock.url, Some("k"));
    let first = jev.decide(Side::Blue, vec![world_question(1)]).await.remove(0);
    // Same situation and options, new question id: from the cache, at no cost.
    let again = jev.decide(Side::Blue, vec![world_question(9)]).await.remove(0);
    assert_eq!(mock.calls(), 1);
    assert_eq!(again.source, AnswerSource::Cache);
    assert_eq!((again.question.0, again.choice.as_str(), again.cost_usd, again.latency_ms), (9, first.choice.as_str(), None, Some(0)));
    // A changed situation is asked.
    let mut q = world_question(10);
    q.situation = "Different now.".into();
    jev.decide(Side::Blue, vec![q, world_question(11)]).await;
    assert_eq!(mock.calls(), 2);
    assert_eq!(mock.request(1).body["questions"].as_object().unwrap().len(), 1, "only the miss is sent");
    assert_eq!(jev.cached(), 2);
}

#[tokio::test]
async fn both_sides_run_concurrently() {
    let mock = serve(|n, req| answer_all(0.9)(n, req).after(Duration::from_millis(200))).await;
    let jev = std::sync::Arc::new(client(&mock.url, Some("k")));
    let (a, b) = (jev.clone(), jev.clone());
    let started = std::time::Instant::now();
    let (x, y) = tokio::join!(a.decide(Side::Blue, vec![world_question(1)]), b.decide(Side::Red, vec![land_question(2, "R1")]));
    assert!(started.elapsed() < Duration::from_millis(380), "{:?}", started.elapsed());
    assert_eq!((x.len(), y.len()), (1, 1));
    assert_eq!(mock.in_flight_max.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn parsing_is_tolerant_but_never_invents() {
    let offered = BTreeMap::from([("q0".to_string(), vec!["o0".to_string(), "o1".to_string()]), ("q1".to_string(), vec!["o0".to_string()])]);
    let top_level = json!({"q0": {"choice": "o1", "confidence": 0.5}, "q1": "o0"});
    let p = parse_choices(&top_level, &offered);
    assert_eq!(p["q0"].choice, "o1");
    assert_eq!(p["q1"].confidence, 0.0);
    let not_offered = json!({"decisions": {"q0": {"choice": "o7", "confidence": 0.9}}});
    assert!(parse_choices(&not_offered, &offered).is_empty());
    assert_eq!(bias_words(0), "");
    assert_eq!(bias_words(-1), " [discouraged by its doctrine]");
}
