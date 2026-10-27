//! Claude planners and decider against a local mock of the Messages API:
//! request shape (headers, one tool with the plan schema), success with cost
//! from usage, retries with backoff on 5xx/429, fallback to the heuristic on
//! persistent errors, invalid plans and refusals, the joint-guidance checks,
//! the per-HQ queue, and the enum trick for unit questions.

mod common;
use bgws_core::{AnswerSource, Geodetic, Question, QuestionId, QuestionKind, Side};
use bgws_decide::claude::{ClaudeConfig, ClaudeDecider, ClaudePlanner, MessagesClient, ToolChoice};
use bgws_decide::hq::{orders_plan_schema, CopTrack, Guidance, HqSituation, HqUnit, PlannerKind, UnitRole};
use bgws_decide::{ApiKey, Decider, HeuristicPlanner, Planner};
use common::*;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn cfg(url: &str) -> ClaudeConfig {
    ClaudeConfig { base_url: url.into(), backoff_base: Duration::from_millis(5), backoff_max: Duration::from_millis(40), timeout: Duration::from_millis(800), ..ClaudeConfig::default() }
}
fn planner(url: &str) -> ClaudePlanner {
    ClaudePlanner::new(MessagesClient::with_http(http(), ApiKey::new("sk-ant-test"), cfg(url)), HeuristicPlanner::default())
}

fn unit(id: &str, role: UnitRole, lat: f64, lon: f64) -> HqUnit {
    HqUnit { id: id.into(), label: id.into(), role, strength: 8.0, vehicles_fit: 4, vehicles_total: 4, at: Geodetic::new(lat, lon, 0.0), reach_m: Some(15000.0), alive: true }
}

fn situation(planner: PlannerKind) -> HqSituation {
    HqSituation {
        hq: "B-HQ".into(),
        echelon: "battalion".into(),
        planner,
        subordinates: vec!["B1".into(), "B2".into(), "B3".into(), "B-ART".into(), "B-CP".into()],
        sub_hqs: vec![],
        post: Some(Geodetic::new(50.0, 10.0, 0.0)),
        guidance: Some(Guidance { by: "BDE".into(), aims: vec!["seize the crossing".into()], priorities: vec!["speed".into()], ..Default::default() }),
        why: "the battle rhythm".into(),
        cop: vec![CopTrack { id: "R7".into(), thought: "a tank platoon".into(), domain: "land".into(), seen_ago_s: 300.0, by: "B2".into(), quality: 9, error_m: 400.0, at: Some(Geodetic::new(50.2, 10.0, 0.0)) }],
        units: vec![unit("B1", UnitRole::Manoeuvre, 50.0, 10.0), unit("B2", UnitRole::Manoeuvre, 50.0, 10.01), unit("B3", UnitRole::Manoeuvre, 50.0, 10.02), unit("B-ART", UnitRole::Fires, 49.9, 10.0), unit("B-CP", UnitRole::Post, 50.0, 10.0)],
        objective: Some(Geodetic::new(50.1, 10.0, 0.0)),
        reference_points: [("OBJECTIVE".to_string(), Geodetic::new(50.1, 10.0, 0.0))].into_iter().collect(),
        shareable_means: [("sortiesPerDay".to_string(), 8.0), ("droneHours".to_string(), 24.0)].into_iter().collect(),
    }
}

fn hq_question(id: u64, agent: &str, s: &HqSituation) -> Question {
    Question { kind: QuestionKind::Hq, agent: agent.into(), trigger: "battleRhythm".into(), title: "Plan".into(), question: "Write the plan.".into(), situation: serde_json::to_string(s).unwrap(), options: vec![], fallback: "plan".into(), rules_choice: "plan".into(), ..world_question(id) }
}

fn tool_reply(input: Value) -> Reply {
    Reply::json(200, json!({"id": "msg_1", "type": "message", "role": "assistant", "stop_reason": "tool_use",
        "content": [{"type": "text", "text": "Here is the plan."}, {"type": "tool_use", "id": "toolu_1", "name": "submit_plan", "input": input}],
        "usage": {"input_tokens": 1000, "output_tokens": 500}}))
}

fn good_orders() -> Value {
    json!({"plan": "B1 and B2 take the crossing; B3 in reserve", "orders": [
        {"unit": "B1", "task": "seize the crossing", "intent": "secure it for the brigade", "phases": [
            {"label": "advance", "do": "move", "to": {"ref": "OBJECTIVE", "bearingDeg": 270, "distanceM": 300}, "mode": "tactical"},
            {"label": "hold it", "do": "overwatch"}]},
        {"unit": "X9", "task": "not mine", "intent": "-", "phases": [{"label": "hold", "do": "hold"}]}
    ]})
}

#[tokio::test]
async fn a_land_plan_through_the_tool_with_its_cost() {
    let mock = serve(|_, _| tool_reply(good_orders())).await;
    let p = planner(&mock.url);
    let a = p.plan(hq_question(1, "B-HQ", &situation(PlannerKind::Land))).await;
    assert_eq!(a.source, AnswerSource::Claude, "{a:?}");
    let plan = a.plan.clone().unwrap();
    assert_eq!(plan["by"], "claude");
    assert_eq!(plan["orders"].as_array().unwrap().len(), 1, "orders for other HQs' units are dropped");
    assert!(plan["warnings"][0].as_str().unwrap().contains("X9"));
    approx::assert_relative_eq!(a.cost_usd.unwrap(), (1000.0 * 2.0 + 500.0 * 10.0) / 1.0e6);
    approx::assert_relative_eq!(p.client().spent(Side::Blue), 0.007);

    let req = mock.request(0);
    assert_eq!(req.path, "/v1/messages");
    assert_eq!(req.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(req.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(req.body["model"], "claude-sonnet-5-5");
    assert_eq!(req.body["tools"][0]["name"], "submit_plan");
    assert_eq!(req.body["tools"][0]["input_schema"], orders_plan_schema());
    assert_eq!(req.body["tool_choice"], json!({"type": "auto"}));
    assert!(req.body["system"].as_str().unwrap().contains("exactly once"));
    let prompt = req.body["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("You are the battalion HQ B-HQ"));
    assert!(prompt.contains("Guidance from BDE: seize the crossing"));
    assert!(prompt.contains("R7: a tank platoon (land), seen 5 min ago by B2, quality 9/15, may be 400 m off"));
    assert!(!format!("{a:?}").contains("sk-ant-test"));
}

#[tokio::test]
async fn the_question_schema_is_used_and_forced_choice_is_optional() {
    let schema = json!({"type": "object", "required": ["plan"], "properties": {"plan": {"type": "string"}}, "additionalProperties": false});
    let mock = serve(|_, _| tool_reply(json!({"plan": "hold"}))).await;
    let client = MessagesClient::with_http(http(), ApiKey::new("k"), ClaudeConfig { tool_choice: ToolChoice::Forced, ..cfg(&mock.url) });
    let p = ClaudePlanner::new(client, HeuristicPlanner::default());
    let mut q = hq_question(1, "B-HQ", &situation(PlannerKind::Land));
    q.situation = "Plain words, not a situation.".into();
    q.plan_schema = Some(schema.clone());
    let a = p.plan(q).await;
    assert_eq!(a.source, AnswerSource::Claude);
    let req = mock.request(0);
    assert_eq!(req.body["tools"][0]["input_schema"], schema);
    assert_eq!(req.body["tools"][0]["strict"], true, "closed schema: strict");
    assert_eq!(req.body["tool_choice"], json!({"type": "tool", "name": "submit_plan"}));
}

#[tokio::test]
async fn retries_with_backoff_on_5xx_and_429() {
    let mock = serve(|n, _| match n {
        0 => Reply::json(500, json!({"error": {"message": "boom"}})),
        1 => Reply::json(529, json!({"error": {"message": "overloaded"}})),
        2 => Reply::json(429, json!({"error": {"message": "slow down"}})).header("retry-after", "0.01"),
        _ => tool_reply(good_orders()),
    })
    .await;
    let client = MessagesClient::with_http(http(), ApiKey::new("k"), cfg(&mock.url));
    let call = client.call_tool(Side::Blue, "s", "p", "submit_plan", "d", &orders_plan_schema()).await.unwrap();
    assert_eq!(call.attempts, 4);
    assert_eq!(mock.calls(), 4);
}

#[tokio::test]
async fn persistent_errors_fall_back_to_the_heuristic_with_the_reason() {
    let mock = serve(|_, _| Reply::json(503, json!({"error": {"message": "unavailable"}}))).await;
    let a = planner(&mock.url).plan(hq_question(1, "B-HQ", &situation(PlannerKind::Land))).await;
    assert_eq!(mock.calls(), 4, "1 try + 3 retries");
    let AnswerSource::Fallback { reason } = &a.source else { panic!("{a:?}") };
    assert!(reason.contains("HTTP 503: unavailable") && reason.contains("the heuristic planned"), "{reason}");
    assert_eq!(a.plan.as_ref().unwrap()["by"], "heuristic");

    // A 400 is not retried.
    let bad = serve(|_, _| Reply::json(400, json!({"error": {"message": "tool_choice not supported"}}))).await;
    let a = planner(&bad.url).plan(hq_question(2, "B-HQ", &situation(PlannerKind::Land))).await;
    assert_eq!(bad.calls(), 1);
    assert!(matches!(a.source, AnswerSource::Fallback { .. }));

    // No key: nothing is sent.
    let p = ClaudePlanner::new(MessagesClient::with_http(http(), None, cfg(&bad.url)), HeuristicPlanner::default());
    let a = p.plan(hq_question(3, "B-HQ", &situation(PlannerKind::Land))).await;
    assert!(matches!(&a.source, AnswerSource::Fallback { reason } if reason.contains("no Anthropic key")));
    assert_eq!(bad.calls(), 1);
}

#[tokio::test]
async fn invalid_plans_and_refusals_fall_back() {
    let invalid = serve(|_, _| tool_reply(json!({"orders": [{"unit": "B1"}]}))).await;
    let a = planner(&invalid.url).plan(hq_question(1, "B-HQ", &situation(PlannerKind::Land))).await;
    let AnswerSource::Fallback { reason } = &a.source else { panic!() };
    assert!(reason.contains("missing \"plan\""), "{reason}");

    let refused = serve(|_, _| Reply::json(200, json!({"stop_reason": "refusal", "stop_details": {"category": "cyber"}, "content": [], "usage": {"input_tokens": 10, "output_tokens": 0}}))).await;
    let a = planner(&refused.url).plan(hq_question(2, "B-HQ", &situation(PlannerKind::Land))).await;
    assert!(matches!(&a.source, AnswerSource::Fallback { reason } if reason.contains("declined (cyber)")));

    let text_only = serve(|_, _| Reply::json(200, json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "{}"}], "usage": {}}))).await;
    let a = planner(&text_only.url).plan(hq_question(3, "B-HQ", &situation(PlannerKind::Land))).await;
    assert!(matches!(&a.source, AnswerSource::Fallback { reason } if reason.contains("did not call submit_plan")));
}

#[tokio::test]
async fn joint_guidance_is_checked_and_overages_become_requests() {
    let mut s = situation(PlannerKind::Overall);
    s.subordinates = vec!["LCC".into(), "ACC".into()];
    let mock = serve(|_, _| {
        tool_reply(json!({"aims": ["hold the line"], "mainEffort": "LCC", "priorities": ["air defence"], "apportionment": {"sortiesPerDay": 20, "droneHours": 10, "satellites": 3}, "roe": {"weaponsFree": false, "escalationLimit": "noCrossBorder"}}))
    })
    .await;
    let a = planner(&mock.url).plan(hq_question(1, "JHQ", &s)).await;
    assert_eq!(a.source, AnswerSource::Claude, "{a:?}");
    let plan = a.plan.unwrap();
    let g = &plan["guidance"];
    assert_eq!(g["apportionment"], json!({"droneHours": 10.0, "sortiesPerDay": 8.0}));
    assert_eq!(g["mainEffort"], "LCC");
    assert_eq!(g["roe"], json!({"weaponsFree": false, "escalationLimit": "noCrossBorder"}));
    let warnings: Vec<&str> = plan["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(warnings.iter().any(|w| w.contains("apportionment \"satellites\" ignored")));
    assert!(warnings.iter().any(|w| w.contains("sortiesPerDay: asked for 20, only 8 exist")));
    let prompt = mock.request(0).body["messages"][0]["content"].as_str().unwrap().to_owned();
    assert!(prompt.contains("overall (joint) commander for blue") && prompt.contains("sortiesPerDay 8"));
}

#[tokio::test]
async fn requests_are_queued_per_hq_and_concurrent_across_hqs() {
    let mock = serve(|_, _| tool_reply(good_orders()).after(Duration::from_millis(150))).await;
    let p = Arc::new(planner(&mock.url));
    let s = situation(PlannerKind::Land);
    let (a, b) = (p.clone(), p.clone());
    let (q1, q2) = (hq_question(1, "B-HQ", &s), hq_question(2, "B-HQ", &s));
    tokio::join!(a.plan(q1), b.plan(q2));
    assert_eq!(mock.in_flight_max.load(Ordering::SeqCst), 1, "one HQ: one request at a time");
    let mock2 = serve(|_, _| tool_reply(good_orders()).after(Duration::from_millis(150))).await;
    let p = Arc::new(planner(&mock2.url));
    let (a, b) = (p.clone(), p.clone());
    tokio::join!(a.plan(hq_question(1, "B-HQ", &s)), b.plan(hq_question(2, "C-HQ", &s)));
    assert_eq!(mock2.in_flight_max.load(Ordering::SeqCst), 2, "two HQs plan concurrently");
}

#[tokio::test]
async fn claude_answers_unit_questions_through_an_enum_of_option_ids() {
    let mock = serve(|_, req| {
        let enum0 = req.body["tools"][0]["input_schema"]["properties"]["q0"]["enum"].clone();
        assert_eq!(enum0, json!(["keep", "engage", "pullBack"]));
        Reply::json(200, json!({"stop_reason": "tool_use", "content": [{"type": "tool_use", "name": "answer_decisions", "input": {"q0": "pullBack", "q1": "teleport"}}], "usage": {"input_tokens": 100, "output_tokens": 10}}))
    })
    .await;
    let d = ClaudeDecider::new(MessagesClient::with_http(http(), ApiKey::new("k"), cfg(&mock.url)));
    let answers = d.decide(Side::Blue, vec![land_question(1, "B1"), land_question(2, "B2")]).await;
    assert_eq!((answers[0].choice.as_str(), &answers[0].source), ("pullBack", &AnswerSource::Claude));
    assert!(matches!(answers[1].source, AnswerSource::Fallback { .. }));
    assert_eq!(answers[1].question, QuestionId(2));
    let schema = &mock.request(0).body["tools"][0]["input_schema"];
    assert_eq!(schema["required"], json!(["q0", "q1"]));
    assert_eq!(mock.request(0).body["tools"][0]["strict"], true);
}
