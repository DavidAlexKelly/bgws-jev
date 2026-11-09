//! Claude planners (RUST_ENGINE.md §10.3), through the Anthropic Messages API
//! called directly from the host: `POST {base}/v1/messages` with `x-api-key`
//! and `anthropic-version: 2023-06-01`, key from `ANTHROPIC_API_KEY`.
//!
//! Structured output: the request offers one tool, `submit_plan`, whose
//! `input_schema` is the question's `plan_schema`, and asks for exactly one
//! call. The returned input is then checked against the schema
//! (`schema_check`) before it is used. The tool is forced with
//! `tool_choice: {type: "tool"}` only when [`ToolChoice::Forced`] is set:
//! current models (the default `claude-sonnet-5-5` among them) refuse forced
//! tool use with a 400, so the default is `auto` plus an instruction, with
//! `strict: true` when the schema is closed.
//!
//! Requests are queued per HQ, retried with exponential backoff on 408, 409,
//! 429 and 5xx (honouring `retry-after`), and costed from the token usage.
//! Anything that fails (no key, an error, a refusal, no valid plan) falls back
//! to the heuristic planner, and the answer says why. The same tool trick, with
//! an enum of option ids per question, lets Claude answer unit and entity
//! questions too ([`ClaudeDecider`]).

use crate::decider::{Decider, Planner};
use crate::hq::{joint_guidance_schema, joint_prompt, land_prompt, orders_plan_schema, parse_joint_guidance, HeuristicPlanner, HqSituation, PlannerKind};
use crate::http::{default_client, post_json, HttpError};
use crate::jev::bias_words;
use crate::key::ApiKey;
use crate::schema_check::check;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Figures, Question, Side};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
pub const KEY_ENV: &str = "ANTHROPIC_API_KEY";
const PLAN_TOOL: &str = "submit_plan";
const ANSWER_TOOL: &str = "answer_decisions";

/// How the single tool is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToolChoice {
    /// `auto`, with the instruction to call the tool once (current models).
    #[default]
    Auto,
    /// `{type: "tool", name}`: only for models that still accept forced tool use.
    Forced,
}

#[derive(Clone, Debug)]
pub struct ClaudeConfig {
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub retries: u32,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub timeout: Duration,
    /// US dollars per million tokens.
    pub price_input: f64,
    pub price_output: f64,
    pub tool_choice: ToolChoice,
}

impl ClaudeConfig {
    pub fn from_figures(f: &Figures) -> ClaudeConfig {
        ClaudeConfig {
            base_url: ANTHROPIC_BASE_URL.into(),
            model: DEFAULT_MODEL.into(),
            max_tokens: f.get("decide.claude.max_tokens") as u32,
            retries: f.get("decide.claude.retries") as u32,
            backoff_base: Duration::from_secs_f64(f.get("decide.claude.backoff_base")),
            backoff_max: Duration::from_secs_f64(f.get("decide.claude.backoff_max")),
            timeout: Duration::from_secs_f64(f.get("decide.claude.timeout")),
            price_input: f.get("decide.claude.price_input"),
            price_output: f.get("decide.claude.price_output"),
            tool_choice: ToolChoice::Auto,
        }
    }
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        ClaudeConfig::from_figures(&crate::default_figures())
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum ClaudeError {
    #[error("no Anthropic key (set {KEY_ENV})")]
    NoKey,
    #[error("{0}")]
    Http(#[from] HttpError),
    #[error("Claude declined ({0})")]
    Refusal(String),
    #[error("Claude did not call {0}")]
    NoToolCall(String),
    #[error("the plan does not match its schema: {0}")]
    Invalid(String),
}

/// A tool call's input, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub input: Value,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub attempts: u32,
}

/// The Messages API client with retries and costing.
pub struct MessagesClient {
    http: reqwest::Client,
    key: Option<ApiKey>,
    pub cfg: ClaudeConfig,
    spent: Mutex<BTreeMap<Side, f64>>,
}

impl MessagesClient {
    pub fn from_env(cfg: ClaudeConfig) -> MessagesClient {
        MessagesClient::with_http(default_client(), ApiKey::from_env(KEY_ENV), cfg)
    }
    pub fn with_http(http: reqwest::Client, key: Option<ApiKey>, cfg: ClaudeConfig) -> MessagesClient {
        MessagesClient { http, key, cfg, spent: Mutex::new(BTreeMap::new()) }
    }

    /// What this client has spent for a side.
    pub fn spent(&self, side: Side) -> f64 {
        self.spent.lock().map(|m| m.get(&side).copied().unwrap_or(0.0)).unwrap_or(0.0)
    }

    /// The request body: one tool, the prompt, the instruction to call it once.
    pub fn request_body(&self, system: &str, prompt: &str, tool: &str, description: &str, schema: &Value) -> Value {
        let mut t = json!({"name": tool, "description": description, "input_schema": schema});
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            t["strict"] = json!(true);
        }
        let choice = match self.cfg.tool_choice {
            ToolChoice::Auto => json!({"type": "auto"}),
            ToolChoice::Forced => json!({"type": "tool", "name": tool}),
        };
        json!({
            "model": self.cfg.model,
            "max_tokens": self.cfg.max_tokens,
            "system": format!("{system}\n\nAnswer only by calling the {tool} tool, exactly once."),
            "messages": [{"role": "user", "content": prompt}],
            "tools": [t],
            "tool_choice": choice,
        })
    }

    /// Ask for one tool call, retrying what is worth retrying.
    pub async fn call_tool(&self, side: Side, system: &str, prompt: &str, tool: &str, description: &str, schema: &Value) -> Result<ToolCall, ClaudeError> {
        let key = self.key.as_ref().ok_or(ClaudeError::NoKey)?;
        let body = self.request_body(system, prompt, tool, description, schema);
        let url = format!("{}/v1/messages", self.cfg.base_url.trim_end_matches('/'));
        let headers = [("x-api-key", key.expose()), ("anthropic-version", ANTHROPIC_VERSION)];
        let mut attempt = 0;
        let reply = loop {
            attempt += 1;
            match post_json(&self.http, &url, &headers, &body, self.cfg.timeout).await {
                Ok(r) => break r,
                Err(e) if e.retryable() && attempt <= self.cfg.retries => {
                    let backoff = self.cfg.backoff_base.saturating_mul(1 << (attempt - 1).min(16));
                    let wait = match &e {
                        HttpError::Status { retry_after_ms: Some(ms), .. } => Duration::from_millis(*ms).max(backoff),
                        _ => backoff,
                    };
                    tokio::time::sleep(wait.min(self.cfg.backoff_max)).await;
                }
                Err(e) => return Err(e.into()),
            }
        };
        let usage = |k: &str| reply.body.pointer(&format!("/usage/{k}")).and_then(Value::as_u64).unwrap_or(0);
        let (input_tokens, output_tokens) = (usage("input_tokens"), usage("output_tokens"));
        let cost_usd = (input_tokens as f64 * self.cfg.price_input + output_tokens as f64 * self.cfg.price_output) / 1.0e6;
        if let Ok(mut m) = self.spent.lock() {
            *m.entry(side).or_insert(0.0) += cost_usd;
        }
        if reply.body.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
            let cat = reply.body.pointer("/stop_details/category").and_then(Value::as_str).unwrap_or("refusal");
            return Err(ClaudeError::Refusal(cat.into()));
        }
        let input = reply
            .body
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use") && b.get("name").and_then(Value::as_str) == Some(tool))
            .and_then(|b| b.get("input").cloned())
            .ok_or_else(|| ClaudeError::NoToolCall(tool.into()))?;
        Ok(ToolCall { input, input_tokens, output_tokens, cost_usd, latency_ms: reply.latency_ms, attempts: attempt })
    }
}

/// Claude in an HQ's seat: the overall planner's JointGuidance, or a land or
/// tactical planner's mission orders for the HQ's own units. Requests for one
/// HQ are queued; different HQs plan concurrently.
pub struct ClaudePlanner {
    client: MessagesClient,
    fallback: HeuristicPlanner,
    queues: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ClaudePlanner {
    pub fn new(client: MessagesClient, fallback: HeuristicPlanner) -> ClaudePlanner {
        ClaudePlanner { client, fallback, queues: Mutex::new(BTreeMap::new()) }
    }
    pub fn from_env(cfg: ClaudeConfig) -> ClaudePlanner {
        ClaudePlanner::new(MessagesClient::from_env(cfg), HeuristicPlanner::default())
    }
    pub fn client(&self) -> &MessagesClient {
        &self.client
    }

    fn queue(&self, hq: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut q = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        q.entry(hq.to_owned()).or_default().clone()
    }

    async fn try_plan(&self, q: &Question) -> Result<(Value, ToolCall, Vec<String>), ClaudeError> {
        let s = HqSituation::of(q);
        let kind = s.as_ref().map(|s| s.planner).unwrap_or_default();
        let side = q.side.as_str();
        let (prompt, default_schema) = match (&s, kind) {
            (Some(s), PlannerKind::Overall) => (joint_prompt(s, side), joint_guidance_schema()),
            (Some(s), _) => (land_prompt(s, side), orders_plan_schema()),
            (None, _) => (format!("{}\n\n{}\n\n{}", q.title, q.question, q.situation), orders_plan_schema()),
        };
        let schema = q.plan_schema.clone().unwrap_or(default_schema);
        let system = "You are a military staff planner in a wargame. Plan only from what has reached your HQ; it is not the truth.";
        let call = self.client.call_tool(q.side, system, &prompt, PLAN_TOOL, "Submit the plan, matching the schema exactly.", &schema).await?;
        let problems = check(&schema, &call.input);
        if !problems.is_empty() {
            return Err(ClaudeError::Invalid(problems.join("; ")));
        }
        let mut warnings = Vec::new();
        let plan = match (&s, kind) {
            (Some(s), PlannerKind::Overall) => {
                let (g, w, requests) = parse_joint_guidance(&call.input, s);
                warnings.extend(w);
                warnings.extend(requests);
                json!({"by": "claude", "plan": if g.aims.is_empty() { "joint guidance".to_string() } else { g.aims.join("; ") }, "orders": [], "guidance": g, "warnings": warnings})
            }
            (Some(s), _) => {
                // Only the HQ's own units' orders are kept.
                let mine: Vec<&String> = s.subordinates.iter().chain(s.units.iter().map(|u| &u.id)).collect();
                let all = call.input.get("orders").and_then(Value::as_array).cloned().unwrap_or_default();
                let (kept, dropped): (Vec<Value>, Vec<Value>) = all.into_iter().partition(|o| o.get("unit").and_then(Value::as_str).is_some_and(|u| mine.iter().any(|m| *m == u)));
                for o in &dropped {
                    warnings.push(format!("orders for {} ignored: not this HQ's unit", o.get("unit").unwrap_or(&Value::Null)));
                }
                if kept.is_empty() {
                    return Err(ClaudeError::Invalid("no orders for this HQ's units".into()));
                }
                let mut p = call.input.clone();
                p["by"] = json!("claude");
                p["orders"] = Value::Array(kept);
                p["warnings"] = json!(warnings);
                p
            }
            (None, _) => {
                let mut p = call.input.clone();
                p["by"] = json!("claude");
                p
            }
        };
        Ok((plan, call, warnings))
    }
}

#[async_trait]
impl Planner for ClaudePlanner {
    async fn plan(&self, q: Question) -> Answer {
        let queue = self.queue(&q.agent);
        let _turn = queue.lock().await;
        match self.try_plan(&q).await {
            Ok((plan, call, warnings)) => Answer {
                question: q.id,
                choice: q.fallback.clone(),
                probabilities: None,
                confidence: None,
                source: AnswerSource::Claude,
                rationale: Some(format!("{}{}", plan["plan"].as_str().unwrap_or("plan"), if warnings.is_empty() { String::new() } else { format!(" (warnings: {})", warnings.join("; ")) })),
                plan: Some(plan),
                cost_usd: Some(call.cost_usd),
                latency_ms: Some(call.latency_ms),
            },
            Err(e) => self.fallback.answer(&q, AnswerSource::Fallback { reason: format!("Claude could not plan ({e}): the heuristic planned") }),
        }
    }
    fn name(&self) -> &str {
        "claude"
    }
}

/// Claude as a unit or entity decider: one call per side and moment, one tool
/// whose input has a property per question, each an enum of its option ids.
pub struct ClaudeDecider {
    client: MessagesClient,
}

impl ClaudeDecider {
    pub fn new(client: MessagesClient) -> ClaudeDecider {
        ClaudeDecider { client }
    }

    /// The tool schema: `{q0: enum[ids], q1: …}`, all required, closed.
    pub fn schema(questions: &[Question]) -> Value {
        let mut props = Map::new();
        for (i, q) in questions.iter().enumerate() {
            props.insert(format!("q{i}"), json!({"type": "string", "enum": q.options.iter().map(|o| o.id.clone()).collect::<Vec<_>>()}));
        }
        json!({"type": "object", "properties": props, "required": (0..questions.len()).map(|i| format!("q{i}")).collect::<Vec<_>>(), "additionalProperties": false})
    }

    fn prompt(side: Side, questions: &[Question]) -> String {
        let mut s = format!("You decide for agents on the {side} side. Each question is raised from what that agent knows, not the truth. Choose within its doctrine.\n");
        for (i, q) in questions.iter().enumerate() {
            s.push_str(&format!("\nq{i}: {} — {}. {} {}\n", q.agent, q.title, q.question, q.situation));
            if !q.doctrine.is_empty() {
                s.push_str(&format!("Its doctrine: {}\n", q.doctrine.join(" ")));
            }
            for o in &q.options {
                s.push_str(&format!("- {}: {}{}\n", o.id, o.summary, bias_words(o.bias)));
            }
        }
        s
    }
}

#[async_trait]
impl Decider for ClaudeDecider {
    async fn decide(&self, side: Side, questions: Vec<Question>) -> Vec<Answer> {
        if questions.is_empty() {
            return vec![];
        }
        let schema = Self::schema(&questions);
        let system = "You lead units in a wargame at their decision points.";
        match self.client.call_tool(side, system, &Self::prompt(side, &questions), ANSWER_TOOL, "Answer every decision with one of its option ids.", &schema).await {
            Ok(call) => {
                let share = call.cost_usd / questions.len() as f64;
                questions
                    .iter()
                    .enumerate()
                    .map(|(i, q)| match call.input.get(format!("q{i}")).and_then(Value::as_str).filter(|c| q.has_option(c)) {
                        Some(c) => Answer { cost_usd: Some(share), latency_ms: Some(call.latency_ms), rationale: Some(format!("Claude chose {c}")), ..Answer::simple(q, c, AnswerSource::Claude) },
                        None => Answer { cost_usd: Some(share), ..Answer::fallback(q, "Claude gave no valid answer — the fallback stands") },
                    })
                    .collect()
            }
            Err(e) => questions.iter().map(|q| Answer::fallback(q, format!("Claude unavailable ({e}) — the fallback stands"))).collect(),
        }
    }
    fn name(&self) -> &str {
        "claude"
    }
}
