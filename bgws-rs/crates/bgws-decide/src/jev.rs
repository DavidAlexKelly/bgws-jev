//! Jev at the decision points (RUST_ENGINE.md §10.2), a port of
//! `data/jevClient.ts`, `data/jevCache.ts`, `rules/jev.ts` and the prompt
//! shapes of `engine/jevDecider.ts` (`jevRealtimeDecider`, land units) and
//! `world/jevWorldDecider.ts` (world entities).
//!
//! Jev (`typesafe/jev-1.13` on OpenRouter's Decisions API) is a decision
//! model: one request carries a state and a map of named `choice` questions,
//! answered in parallel with a choice, a probability per option and a
//! confidence. All of one side's questions at one moment go in one request.
//! Below the confidence threshold, on a timeout, an error or with no key,
//! each question takes its template fallback and the answer says why. A cache
//! keyed by `Question::cache_key()` answers a repeated situation the same way
//! without paying again; cost and latency go on every answer.

use crate::decider::Decider;
use crate::http::{default_client, post_json, HttpError};
use crate::key::ApiKey;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Figures, Question, QuestionKind, Side};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

/// The pinned model: `-latest` would make yesterday's experiment unrepeatable.
pub const JEV_MODEL: &str = "typesafe/jev-1.13";
/// OpenRouter's Decisions API (alpha).
pub const OPENROUTER_DECISIONS_URL: &str = "https://openrouter.ai/api/alpha/decisions";
/// The environment variable the key is read from.
pub const KEY_ENV: &str = "OPENROUTER_API_KEY";

/// The balance Jev is told at every land decision (`engine/decisions.ts` BALANCE).
pub const BALANCE: &str = "The intent comes first; keep the unit alive and able to carry it out; take a local opportunity only when it doesn't cost the intent.";

const LAND_NOTE: &str = "Only what each unit believes. Enemies it has not found are not listed, and their absence is not evidence that they are not there. A lost contact is where an enemy was last seen, not where it is.";
const WORLD_NOTE: &str = "Each question is raised from what that agent knows, not the truth. Choose within its doctrine.";

/// How an option's doctrine bias reads to Jev (`biasWords`).
pub fn bias_words(bias: i8) -> &'static str {
    match bias {
        b if b >= 2 => " [strongly preferred by its doctrine]",
        b if b > 0 => " [preferred by its doctrine]",
        b if b <= -2 => " [strongly discouraged by its doctrine]",
        b if b < 0 => " [discouraged by its doctrine]",
        _ => "",
    }
}

/// One choice answer, as read from the response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JevChoice {
    pub choice: String,
    pub probabilities: BTreeMap<String, f64>,
    pub confidence: f64,
}

/// Read the choice answers out of a Decisions API body (`parseJevAnswers`,
/// choice questions). Tolerant, because the endpoint is alpha: answers under
/// `answers`, `decisions` or at the top level; an untagged answer; a bare
/// string choice. It never invents an answer to a question not answered, and
/// only accepts a choice that was offered.
pub fn parse_choices(body: &Value, offered: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, JevChoice> {
    let Some(obj) = body.as_object() else { return BTreeMap::new() };
    let container = obj.get("answers").and_then(Value::as_object).or_else(|| obj.get("decisions").and_then(Value::as_object)).unwrap_or(obj);
    let mut out = BTreeMap::new();
    for (qk, keys) in offered {
        let Some(raw) = container.get(qk) else { continue };
        match raw {
            Value::String(s) if keys.contains(s) => {
                out.insert(qk.clone(), JevChoice { choice: s.clone(), probabilities: BTreeMap::new(), confidence: 0.0 });
            }
            Value::Object(r) => {
                let Some(choice) = r.get("choice").and_then(Value::as_str).filter(|c| keys.iter().any(|k| k == c)) else { continue };
                let probabilities = r
                    .get("probabilities")
                    .and_then(Value::as_object)
                    .map(|m| m.iter().filter_map(|(k, v)| v.as_f64().filter(|x| x.is_finite()).map(|x| (k.clone(), x))).collect())
                    .unwrap_or_default();
                let confidence = r.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
                out.insert(qk.clone(), JevChoice { choice: choice.to_owned(), probabilities, confidence });
            }
            _ => {}
        }
    }
    out
}

/// One question as sent: its key, and option key → option id.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyed {
    pub key: String,
    pub options: Vec<(String, String)>,
}

/// Build one side's request body (`{model, state, questions}`) and the key
/// maps to read the answers back. Land units are keyed `u0, u1…` and asked as
/// `jevRealtimeDecider` asks; world entities and HQs `q0, q1…` as
/// `jevWorldDecider` asks. Option keys are `o<index>` (land "keep" keeps its
/// own key, as the TypeScript does).
pub fn build_request(model: &str, side: Side, questions: &[Question], directive: Option<&str>, max_choices: usize) -> (Value, Vec<Keyed>) {
    let directive = directive.map(str::trim).filter(|d| !d.is_empty()).map(|d| format!("{d}\n\n")).unwrap_or_default();
    let mut qs = Map::new();
    let mut keyed = Vec::new();
    let (mut nu, mut nq) = (0, 0);
    let land: Vec<&Question> = questions.iter().filter(|q| q.kind == QuestionKind::Unit).collect();
    for q in questions {
        let is_land = q.kind == QuestionKind::Unit;
        let key = if is_land {
            nu += 1;
            format!("u{}", nu - 1)
        } else {
            nq += 1;
            format!("q{}", nq - 1)
        };
        let options: Vec<(String, String)> = q.options.iter().take(max_choices).enumerate().map(|(i, o)| (if is_land && o.id == "keep" { "keep".into() } else { format!("o{i}") }, o.id.clone())).collect();
        let mut criteria = Map::new();
        for ((k, _), o) in options.iter().zip(&q.options) {
            criteria.insert(k.clone(), Value::String(format!("{}{}", o.summary, bias_words(o.bias))));
        }
        let doctrine = if q.doctrine.is_empty() { String::new() } else { format!("Its doctrine: {}\n\n", q.doctrine.join(" ")) };
        let instructions = if is_land {
            let others: Vec<&str> = land.iter().filter(|o| o.agent != q.agent).map(|o| o.agent.as_str()).collect();
            let title = q.title.to_lowercase();
            format!(
                "{directive}You lead {}. {BALANCE}\n\n{doctrine}Decision point {}, {title}. {} {} {}The crew has already done its drill (cover, and return fire if it can see the shooter).",
                q.agent,
                q.trigger,
                q.situation,
                q.question,
                if others.is_empty() { String::new() } else { format!("Also deciding now, so choose to work together: {}. ", others.join(", ")) }
            )
        } else {
            let situation = if q.situation.is_empty() { String::new() } else { format!(" {}", q.situation) };
            format!("{directive}You decide for {}, on the {side} side. {doctrine}{}. {}{situation}", q.agent, q.title, q.question)
        };
        qs.insert(key.clone(), json!({ "type": "choice", "instructions": instructions, "criteria": criteria }));
        keyed.push(Keyed { key, options });
    }
    let note = if land.is_empty() { WORLD_NOTE } else { LAND_NOTE };
    let state = json!({ "you": side.as_str(), "note": note });
    (json!({ "model": model, "state": state, "questions": qs }), keyed)
}

/// Settings, from the figures and the host.
#[derive(Clone, Debug)]
pub struct JevConfig {
    /// The Decisions endpoint (configurable for a proxy or a test server).
    pub url: String,
    pub model: String,
    pub timeout: Duration,
    pub min_confidence: f64,
    pub cache_limit: usize,
    pub max_choices: usize,
    /// Doctrine or temperament, put in front of every question.
    pub directive: Option<String>,
}

impl JevConfig {
    pub fn from_figures(f: &Figures) -> JevConfig {
        JevConfig {
            url: OPENROUTER_DECISIONS_URL.into(),
            model: JEV_MODEL.into(),
            timeout: Duration::from_secs_f64(f.get("decide.jev.timeout")),
            min_confidence: f.get("decide.jev.min_confidence"),
            cache_limit: f.get("decide.jev.cache_limit") as usize,
            max_choices: f.get("decide.jev.max_choices") as usize,
            directive: None,
        }
    }
}

impl Default for JevConfig {
    fn default() -> Self {
        JevConfig::from_figures(&crate::default_figures())
    }
}

/// Remembered answers, bounded, oldest out first (`jevCache.ts`).
#[derive(Debug, Default)]
struct Cache {
    entries: BTreeMap<(String, u64), JevChoiceById>,
    order: VecDeque<(String, u64)>,
}

#[derive(Clone, Debug)]
struct JevChoiceById {
    choice: String,
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
}

/// The Jev client: a [`Decider`] for one or both sides.
pub struct JevClient {
    http: reqwest::Client,
    key: Option<ApiKey>,
    cfg: JevConfig,
    cache: Mutex<Cache>,
    spent: Mutex<BTreeMap<Side, f64>>,
    name: String,
}

impl JevClient {
    /// With the key from `OPENROUTER_API_KEY` (or none: every question falls back).
    pub fn from_env(cfg: JevConfig) -> JevClient {
        JevClient::new(ApiKey::from_env(KEY_ENV), cfg)
    }

    pub fn new(key: Option<ApiKey>, cfg: JevConfig) -> JevClient {
        JevClient::with_http(default_client(), key, cfg)
    }

    /// With a given HTTP client (tests pass one without a proxy).
    pub fn with_http(http: reqwest::Client, key: Option<ApiKey>, cfg: JevConfig) -> JevClient {
        let name = cfg.model.clone();
        JevClient { http, key, cfg, cache: Mutex::new(Cache::default()), spent: Mutex::new(BTreeMap::new()), name }
    }

    /// What this client has spent for a side, as OpenRouter reported it.
    pub fn spent(&self, side: Side) -> f64 {
        self.spent.lock().map(|m| m.get(&side).copied().unwrap_or(0.0)).unwrap_or(0.0)
    }

    /// Entries in the cache.
    pub fn cached(&self) -> usize {
        self.cache.lock().map(|c| c.entries.len()).unwrap_or(0)
    }

    fn cache_get(&self, q: &Question) -> Option<JevChoiceById> {
        self.cache.lock().ok()?.entries.get(&(self.cfg.model.clone(), q.cache_key())).cloned()
    }

    fn cache_put(&self, q: &Question, c: JevChoiceById) {
        let Ok(mut cache) = self.cache.lock() else { return };
        let k = (self.cfg.model.clone(), q.cache_key());
        if cache.entries.insert(k.clone(), c).is_none() {
            cache.order.push_back(k);
        }
        while cache.entries.len() > self.cfg.cache_limit.max(1) {
            match cache.order.pop_front() {
                Some(old) => {
                    cache.entries.remove(&old);
                }
                None => break,
            }
        }
    }

    /// Turn an answer by id into an Answer, applying the threshold.
    fn answer(&self, q: &Question, c: &JevChoiceById, source: AnswerSource, latency_ms: Option<u64>, cost: Option<f64>) -> Answer {
        if c.confidence >= self.cfg.min_confidence && q.has_option(&c.choice) {
            Answer {
                question: q.id,
                choice: c.choice.clone(),
                probabilities: Some(c.probabilities.clone()),
                confidence: Some(c.confidence),
                source,
                plan: None,
                cost_usd: cost,
                latency_ms,
                rationale: Some(format!("Jev chose {} at {}% confidence", c.choice, (c.confidence * 100.0).round())),
            }
        } else {
            let mut a = Answer::fallback(q, format!("Jev was unsure ({} at {}% confidence) — the fallback stands", c.choice, (c.confidence * 100.0).round()));
            a.probabilities = Some(c.probabilities.clone());
            a.confidence = Some(c.confidence);
            a.cost_usd = cost;
            a.latency_ms = latency_ms;
            a
        }
    }
}

#[async_trait]
impl Decider for JevClient {
    async fn decide(&self, side: Side, questions: Vec<Question>) -> Vec<Answer> {
        let mut out: Vec<Option<Answer>> = vec![None; questions.len()];
        // Cache hits answer at once, at no cost.
        let mut ask: Vec<usize> = Vec::new();
        for (i, q) in questions.iter().enumerate() {
            match self.cache_get(q) {
                Some(c) => out[i] = Some(self.answer(q, &c, AnswerSource::Cache, Some(0), None)),
                None => ask.push(i),
            }
        }
        if !ask.is_empty() {
            let batch: Vec<Question> = ask.iter().map(|&i| questions[i].clone()).collect();
            let answers = self.ask(side, &batch).await;
            for (i, a) in ask.into_iter().zip(answers) {
                out[i] = Some(a);
            }
        }
        out.into_iter().zip(&questions).map(|(a, q)| a.unwrap_or_else(|| Answer::fallback(q, "Jev gave no answer"))).collect()
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl JevClient {
    /// One request for these questions.
    async fn ask(&self, side: Side, batch: &[Question]) -> Vec<Answer> {
        let Some(key) = &self.key else {
            return batch.iter().map(|q| Answer::fallback(q, format!("Jev unavailable (no OpenRouter key: set {KEY_ENV}) — the fallback stands"))).collect();
        };
        let (body, keyed) = build_request(&self.cfg.model, side, batch, self.cfg.directive.as_deref(), self.cfg.max_choices);
        let auth = format!("Bearer {}", key.expose());
        let headers = [("Authorization", auth.as_str()), ("X-Title", "BGWS")];
        let reply = match post_json(&self.http, &self.cfg.url, &headers, &body, self.cfg.timeout).await {
            Ok(r) => r,
            Err(e) => {
                let why = match &e {
                    HttpError::Status { status, detail, .. } => format!("OpenRouter {status}: {detail}"),
                    other => other.to_string(),
                };
                return batch.iter().map(|q| Answer::fallback(q, format!("Jev unavailable ({why}) — the fallback stands"))).collect();
            }
        };
        let offered: BTreeMap<String, Vec<String>> = keyed.iter().map(|k| (k.key.clone(), k.options.iter().map(|(ok, _)| ok.clone()).collect())).collect();
        let parsed = parse_choices(&reply.body, &offered);
        if parsed.is_empty() {
            // Every question unanswered is schema drift, not a decision.
            return batch.iter().map(|q| Answer::fallback(q, "Jev returned no readable answers — the fallback stands")).collect();
        }
        let cost = reply.body.pointer("/usage/cost").and_then(Value::as_f64);
        if let (Some(c), Ok(mut m)) = (cost, self.spent.lock()) {
            *m.entry(side).or_insert(0.0) += c;
        }
        let share = cost.map(|c| c / batch.len() as f64);
        batch
            .iter()
            .zip(&keyed)
            .map(|(q, k)| match parsed.get(&k.key) {
                Some(c) => {
                    let id_of = |key: &str| k.options.iter().find(|(ok, _)| ok == key).map(|(_, id)| id.clone());
                    let by_id = JevChoiceById {
                        choice: id_of(&c.choice).unwrap_or_default(),
                        probabilities: k.options.iter().map(|(ok, id)| (id.clone(), c.probabilities.get(ok).copied().unwrap_or(0.0))).collect(),
                        confidence: c.confidence,
                    };
                    self.cache_put(q, by_id.clone());
                    self.answer(q, &by_id, AnswerSource::Jev, Some(reply.latency_ms), share)
                }
                None => {
                    let mut a = Answer::fallback(q, "Jev gave no answer for this question in its reply — the fallback stands");
                    a.latency_ms = Some(reply.latency_ms);
                    a.cost_usd = share;
                    a
                }
            })
            .collect()
    }
}
