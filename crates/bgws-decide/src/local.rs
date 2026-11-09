//! A local model behind the same `Decider` trait (RUST_ENGINE.md §10.5): an
//! OpenAI-compatible server (llama.cpp or vLLM), `POST {base}/v1/chat/completions`
//! with `response_format: json_schema` constraining the reply to
//! `{"choice": <one of the option ids>}`. Probabilities and confidence come
//! from the token logprobs when the server returns them: the alternatives at
//! the first token of the choice's value are mapped to the options they
//! begin, normalised over the options. Questions are sent concurrently, one
//! request each (vLLM batches them continuously).

use crate::decider::Decider;
use crate::http::{default_client, post_json};
use crate::jev::bias_words;
use crate::key::ApiKey;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Figures, Question, Side};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LocalConfig {
    /// e.g. "http://127.0.0.1:8080".
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
    /// Applied only when logprobs give a confidence.
    pub min_confidence: f64,
    pub top_logprobs: u32,
}

impl LocalConfig {
    pub fn from_figures(f: &Figures, base_url: &str, model: &str) -> LocalConfig {
        LocalConfig {
            base_url: base_url.into(),
            model: model.into(),
            timeout: Duration::from_secs_f64(f.get("decide.local.timeout")),
            min_confidence: f.get("decide.local.min_confidence"),
            top_logprobs: f.get("decide.local.top_logprobs") as u32,
        }
    }
}

pub struct LocalDecider {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    key: Option<ApiKey>,
    cfg: LocalConfig,
}

impl LocalDecider {
    pub fn new(cfg: LocalConfig) -> LocalDecider {
        LocalDecider::with_http(default_client(), None, cfg)
    }
    pub fn with_http(http: reqwest::Client, key: Option<ApiKey>, cfg: LocalConfig) -> LocalDecider {
        LocalDecider { inner: Arc::new(Inner { http, key, cfg }) }
    }
}

/// The request for one question.
pub fn request_body(model: &str, side: Side, q: &Question, top_logprobs: u32) -> Value {
    let ids: Vec<&str> = q.options.iter().map(|o| o.id.as_str()).collect();
    let mut user = format!("You decide for {}, on the {side} side. ", q.agent);
    if !q.doctrine.is_empty() {
        user.push_str(&format!("Its doctrine: {}\n\n", q.doctrine.join(" ")));
    }
    user.push_str(&format!("{}. {} {}\nOptions:\n", q.title, q.question, q.situation));
    for o in &q.options {
        user.push_str(&format!("- {}: {}{}\n", o.id, o.summary, bias_words(o.bias)));
    }
    user.push_str("Reply with JSON: {\"choice\": <option id>}.");
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You decide for one agent in a wargame, from what it knows, within its doctrine."},
            {"role": "user", "content": user}
        ],
        "temperature": 0,
        "logprobs": true,
        "top_logprobs": top_logprobs,
        "response_format": {"type": "json_schema", "json_schema": {"name": "choice", "strict": true, "schema": {
            "type": "object", "properties": {"choice": {"type": "string", "enum": ids}}, "required": ["choice"], "additionalProperties": false}}}
    })
}

/// Option probabilities from the logprobs, if there are any: at the token
/// where the choice's value starts, each alternative token is credited to the
/// options whose id starts with it (split evenly), then normalised.
pub fn probabilities_from_logprobs(choice: &Value, ids: &[String]) -> Option<BTreeMap<String, f64>> {
    let tokens = choice.pointer("/logprobs/content")?.as_array()?;
    let mut text = String::new();
    let mut offsets = Vec::with_capacity(tokens.len());
    for t in tokens {
        offsets.push(text.len());
        text.push_str(t.get("token")?.as_str()?);
    }
    // The value starts after `"choice"`, the colon and the opening quote.
    let k = text.find("\"choice\"")? + 8;
    let start = text[k..].find('"')? + k + 1;
    if start >= text.len() {
        return None;
    }
    let at = (0..tokens.len()).rev().find(|&i| offsets[i] <= start);
    let alts = tokens.get(at?)?.get("top_logprobs")?.as_array()?;
    let mut p: BTreeMap<String, f64> = ids.iter().map(|id| (id.clone(), 0.0)).collect();
    for a in alts {
        let tok = a.get("token").and_then(Value::as_str)?.trim_start_matches(|c: char| c == '"' || c == ':' || c.is_whitespace());
        let lp = a.get("logprob").and_then(Value::as_f64)?;
        if tok.is_empty() {
            continue;
        }
        let matches: Vec<&String> = ids.iter().filter(|id| id.starts_with(tok) || tok.starts_with(id.as_str())).collect();
        for m in &matches {
            *p.get_mut(*m)? += lp.exp() / matches.len() as f64;
        }
    }
    let total: f64 = p.values().sum();
    if total <= 0.0 {
        return None;
    }
    p.values_mut().for_each(|v| *v /= total);
    Some(p)
}

impl Inner {
    async fn one(&self, side: Side, q: Question) -> Answer {
        let url = format!("{}/v1/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let auth = self.key.as_ref().map(|k| format!("Bearer {}", k.expose()));
        let headers: Vec<(&str, &str)> = auth.as_deref().map(|a| vec![("Authorization", a)]).unwrap_or_default();
        let body = request_body(&self.cfg.model, side, &q, self.cfg.top_logprobs);
        let reply = match post_json(&self.http, &url, &headers, &body, self.cfg.timeout).await {
            Ok(r) => r,
            Err(e) => return Answer::fallback(&q, format!("local model unavailable ({e}) — the fallback stands")),
        };
        let first = reply.body.pointer("/choices/0").cloned().unwrap_or(Value::Null);
        let content = first.pointer("/message/content").and_then(Value::as_str).unwrap_or_default();
        let choice = serde_json::from_str::<Value>(content).ok().and_then(|v| v.get("choice").and_then(Value::as_str).map(str::to_owned));
        let Some(choice) = choice.filter(|c| q.has_option(c)) else {
            return Answer::fallback(&q, "the local model gave no valid choice — the fallback stands");
        };
        let ids: Vec<String> = q.options.iter().map(|o| o.id.clone()).collect();
        let probs = probabilities_from_logprobs(&first, &ids);
        let confidence = probs.as_ref().and_then(|p| p.values().copied().reduce(f64::max));
        if let Some(c) = confidence.filter(|c| *c < self.cfg.min_confidence) {
            let mut a = Answer::fallback(&q, format!("the local model was unsure ({}%) — the fallback stands", (c * 100.0).round()));
            a.probabilities = probs;
            a.confidence = Some(c);
            return a;
        }
        Answer { probabilities: probs, confidence, latency_ms: Some(reply.latency_ms), rationale: Some(format!("the local model chose {choice}")), ..Answer::simple(&q, choice, AnswerSource::Local) }
    }
}

#[async_trait]
impl Decider for LocalDecider {
    async fn decide(&self, side: Side, questions: Vec<Question>) -> Vec<Answer> {
        let mut set = tokio::task::JoinSet::new();
        for (i, q) in questions.iter().cloned().enumerate() {
            let inner = self.inner.clone();
            set.spawn(async move { (i, inner.one(side, q).await) });
        }
        let mut out: Vec<Option<Answer>> = vec![None; questions.len()];
        while let Some(r) = set.join_next().await {
            if let Ok((i, a)) = r {
                out[i] = Some(a);
            }
        }
        out.into_iter().zip(&questions).map(|(a, q)| a.unwrap_or_else(|| Answer::fallback(q, "the local model task failed"))).collect()
    }
    fn name(&self) -> &str {
        "local"
    }
}
