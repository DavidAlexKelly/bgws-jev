//! A chat completion through OpenRouter (for tools such as the lab's
//! wording tuner, which asks a large model for rewrites). The key stays in
//! this crate: callers get the reply text and its cost.

use crate::http::{default_client, post_json, HttpError};
use crate::key::ApiKey;
use serde_json::{json, Value};
use std::time::Duration;

pub const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// Ask `model` once: a system and a user message. Returns the reply text and its cost (USD, as reported).
pub async fn chat(model: &str, system: &str, user: &str, max_tokens: u32, timeout: Duration) -> Result<(String, f64), String> {
    let key = ApiKey::from_env(crate::jev::KEY_ENV).ok_or_else(|| format!("no OpenRouter key: set {}", crate::jev::KEY_ENV))?;
    let body = json!({
        "model": model,
        "messages": [ { "role": "system", "content": system }, { "role": "user", "content": user } ],
        "max_tokens": max_tokens,
        "usage": { "include": true },
    });
    let auth = format!("Bearer {}", key.expose());
    let reply = post_json(&default_client(), CHAT_URL, &[("Authorization", auth.as_str()), ("X-Title", "BGWS")], &body, timeout).await.map_err(|e| match e {
        HttpError::Status { status, detail, .. } => format!("OpenRouter {status}: {detail}"),
        other => other.to_string(),
    })?;
    let text = reply.body.pointer("/choices/0/message/content").and_then(Value::as_str).ok_or_else(|| format!("no reply text in {}", reply.body))?;
    Ok((text.to_string(), reply.body.pointer("/usage/cost").and_then(Value::as_f64).unwrap_or(0.0)))
}
