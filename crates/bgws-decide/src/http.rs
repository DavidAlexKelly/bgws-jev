//! One JSON POST with a deadline: the only place this crate touches the
//! network. Errors carry the status and a short body excerpt, never headers,
//! so a key can never appear in one.

use serde_json::Value;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum HttpError {
    #[error("no answer within {0} ms")]
    Timeout(u64),
    #[error("could not reach {host}: {why}")]
    Connect { host: String, why: String },
    #[error("HTTP {status}: {detail}")]
    Status { status: u16, detail: String, retry_after_ms: Option<u64> },
    #[error("unreadable reply: {0}")]
    Body(String),
}

impl HttpError {
    /// Worth retrying: rate limits, overload, server errors, timeouts, connection failures.
    pub fn retryable(&self) -> bool {
        match self {
            HttpError::Timeout(_) | HttpError::Connect { .. } => true,
            HttpError::Status { status, .. } => matches!(status, 408 | 409 | 429) || *status >= 500,
            HttpError::Body(_) => false,
        }
    }
}

/// A reply that parsed as JSON, and how long it took.
#[derive(Clone, Debug)]
pub struct Reply {
    pub body: Value,
    pub latency_ms: u64,
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_owned()
}

/// POST `body` to `url` with these headers; a non-2xx status is an error with
/// the API's own message where it gives one.
pub async fn post_json(client: &reqwest::Client, url: &str, headers: &[(&str, &str)], body: &Value, timeout: Duration) -> Result<Reply, HttpError> {
    let started = Instant::now();
    let mut req = client.post(url).json(body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let fut = async {
        let resp = req.send().await.map_err(|e| if e.is_timeout() { HttpError::Timeout(timeout.as_millis() as u64) } else { HttpError::Connect { host: host_of(url), why: e.without_url().to_string() } })?;
        let status = resp.status().as_u16();
        let retry_after_ms = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<f64>().ok()).map(|s| (s * 1000.0) as u64);
        let text = resp.text().await.map_err(|e| HttpError::Body(e.without_url().to_string()))?;
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if !(200..300).contains(&status) {
            let detail = parsed
                .as_ref()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| text.chars().take(300).collect());
            return Err(HttpError::Status { status, detail, retry_after_ms });
        }
        parsed.ok_or_else(|| HttpError::Body(text.chars().take(300).collect()))
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(body)) => Ok(Reply { body, latency_ms: started.elapsed().as_millis() as u64 }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(HttpError::Timeout(timeout.as_millis() as u64)),
    }
}

/// A client for these calls: rustls, no global state.
pub fn default_client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap_or_default()
}
