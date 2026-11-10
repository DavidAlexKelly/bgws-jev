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
pub const JEV_MODEL: &str = "typesafe/jev-1.14";
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

/// An option's key in the request: its own id when that is a plain word
/// (`engageNow`, `keep`), which reads better to the model than `o3`;
/// else `o<index>`. Unique within the question.
pub(crate) fn option_key(id: &str, index: usize, taken: &[String]) -> String {
    let plain = !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain && !taken.iter().any(|t| t == id) {
        id.to_string()
    } else {
        format!("o{index}")
    }
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
/// `jevWorldDecider` asks. Option keys are the options' own ids where they
/// are plain words. A question that carries its agent's state as named
/// fields (`Question::state`) puts it in the request's `state`, under
/// `units.<agent>`, and its instructions point there; with `prose`, the
/// situation in words goes in the instructions as well.
pub fn build_request(model: &str, side: Side, questions: &[Question], directive: Option<&str>, max_choices: usize) -> (Value, Vec<Keyed>) {
    build_request_with(model, side, questions, directive, max_choices, false)
}

pub fn build_request_with(model: &str, side: Side, questions: &[Question], directive: Option<&str>, max_choices: usize, prose: bool) -> (Value, Vec<Keyed>) {
    build_request_worded(model, side, questions, directive, max_choices, prose, &Wording::default())
}

/// Fill a wording template: `{name}` → its value.
fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// How a question is worded to Jev: every sentence it is given that is not
/// the situation itself. The defaults are the text the engine has always
/// sent; a tuned wording (`data/jev/wording.json`, from `bgws lab tune`)
/// replaces them. Templates take `{name}` placeholders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Wording {
    /// A land unit's instructions: {directive} {agent} {balance} {doctrine} {point} {title} {situation} {question} {others}.
    pub land: String,
    /// A world entity's: {directive} {agent} {side} {doctrine} {title} {question} {situation}.
    pub world: String,
    /// The rule of thumb every land unit is given.
    pub balance: String,
    /// {text}: the template's doctrine.
    pub doctrine: String,
    /// {names}: the other units deciding at the same moment.
    pub others: String,
    /// Where a unit's state is: {agent}.
    pub state_pointer: String,
    /// The same, after the situation in words (with `prose`): {situation} {agent}.
    pub state_pointer_prose: String,
    pub land_note: String,
    pub world_note: String,
    /// What an option's doctrine leaning reads as: strongly preferred, preferred, discouraged, strongly discouraged.
    pub bias: [String; 4],
    /// Jev's own description of an option, by action (`land.betterPosition`), in place of the library's.
    pub option_words: BTreeMap<String, String>,
    /// The confidence below which Jev's choice is not taken (else the figure's), and per decision point.
    pub min_confidence: Option<f64>,
    pub point_confidence: BTreeMap<String, f64>,
}

impl Default for Wording {
    fn default() -> Self {
        Wording {
            land: "{directive}You lead {agent}. {balance}\n\n{doctrine}Decision point {point}, {title}. {situation} {question} {others}The crew has already done its drill (cover, and return fire if it can see the shooter).".into(),
            world: "{directive}You decide for {agent}, on the {side} side. {doctrine}{title}. {question}{situation}".into(),
            balance: BALANCE.into(),
            doctrine: "Its doctrine: {text}\n\n".into(),
            others: "Also deciding now, so choose to work together: {names}. ".into(),
            state_pointer: "What it knows is under units.{agent} in the state: itself, its weapons and how far they reach, the enemy it has seen (and how far they reach), its orders, its ground and its friends.".into(),
            state_pointer_prose: "{situation} What it knows is under units.{agent} in the state.".into(),
            land_note: LAND_NOTE.into(),
            world_note: WORLD_NOTE.into(),
            bias: [" [strongly preferred by its doctrine]".into(), " [preferred by its doctrine]".into(), " [discouraged by its doctrine]".into(), " [strongly discouraged by its doctrine]".into()],
            option_words: BTreeMap::new(),
            min_confidence: None,
            point_confidence: BTreeMap::new(),
        }
    }
}

impl Wording {
    fn bias(&self, b: i8) -> &str {
        match b {
            b if b >= 2 => &self.bias[0],
            b if b > 0 => &self.bias[1],
            b if b <= -2 => &self.bias[3],
            b if b < 0 => &self.bias[2],
            _ => "",
        }
    }

    /// The confidence a choice at this decision point needs.
    pub fn threshold(&self, point: &str, figure: f64) -> f64 {
        self.point_confidence.get(point).copied().or(self.min_confidence).unwrap_or(figure)
    }

    /// Every piece of text in it, for checks (placeholders, leaks, length).
    pub fn texts(&self) -> Vec<(String, &str)> {
        let mut v: Vec<(String, &str)> = vec![
            ("land".into(), self.land.as_str()),
            ("world".into(), self.world.as_str()),
            ("balance".into(), self.balance.as_str()),
            ("doctrine".into(), self.doctrine.as_str()),
            ("others".into(), self.others.as_str()),
            ("statePointer".into(), self.state_pointer.as_str()),
            ("statePointerProse".into(), self.state_pointer_prose.as_str()),
            ("landNote".into(), self.land_note.as_str()),
            ("worldNote".into(), self.world_note.as_str()),
        ];
        for (k, b) in self.bias.iter().enumerate() {
            v.push((format!("bias[{k}]"), b.as_str()));
        }
        for (k, w) in &self.option_words {
            v.push((format!("optionWords.{k}"), w.as_str()));
        }
        v
    }
}

/// [`build_request_with`] in a given wording.
pub fn build_request_worded(model: &str, side: Side, questions: &[Question], directive: Option<&str>, max_choices: usize, prose: bool, w: &Wording) -> (Value, Vec<Keyed>) {
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
        let mut options: Vec<(String, String)> = Vec::new();
        for (i, o) in q.options.iter().take(max_choices).enumerate() {
            let taken: Vec<String> = options.iter().map(|(k, _)| k.clone()).collect();
            options.push((option_key(&o.id, i, &taken), o.id.clone()));
        }
        let mut criteria = Map::new();
        for ((k, _), o) in options.iter().zip(&q.options) {
            let words = w.option_words.get(&o.action).map(String::as_str).unwrap_or(&o.summary);
            criteria.insert(k.clone(), Value::String(format!("{}{}", words, w.bias(o.bias))));
        }
        let doctrine = if q.doctrine.is_empty() { String::new() } else { fill(&w.doctrine, &[("text", &q.doctrine.join(" "))]) };
        let instructions = if is_land {
            let others: Vec<&str> = land.iter().filter(|o| o.agent != q.agent).map(|o| o.agent.as_str()).collect();
            let title = q.title.to_lowercase();
            let situation = match &q.state {
                Some(_) if prose => fill(&w.state_pointer_prose, &[("situation", &q.situation), ("agent", &q.agent)]),
                Some(_) => fill(&w.state_pointer, &[("agent", &q.agent)]),
                None => q.situation.clone(),
            };
            let others = if others.is_empty() { String::new() } else { fill(&w.others, &[("names", &others.join(", "))]) };
            fill(
                &w.land,
                &[("directive", &directive), ("agent", &q.agent), ("balance", &w.balance), ("doctrine", &doctrine), ("point", &q.trigger), ("title", &title), ("situation", &situation), ("question", &q.question), ("others", &others)],
            )
        } else {
            let situation = if q.situation.is_empty() { String::new() } else { format!(" {}", q.situation) };
            fill(&w.world, &[("directive", &directive), ("agent", &q.agent), ("side", side.as_str()), ("doctrine", &doctrine), ("title", &q.title), ("question", &q.question), ("situation", &situation)])
        };
        qs.insert(key.clone(), json!({ "type": "choice", "instructions": instructions, "criteria": criteria }));
        keyed.push(Keyed { key, options });
    }
    let note = if land.is_empty() { &w.world_note } else { &w.land_note };
    let mut state = json!({ "you": side.as_str(), "note": note });
    let units: Map<String, Value> = questions.iter().filter_map(|q| q.state.as_ref().map(|s| (q.agent.clone(), s.clone()))).collect();
    if !units.is_empty() {
        state["units"] = Value::Object(units);
    }
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
    /// Also give a unit's situation in words when its state goes as named fields.
    pub prose: bool,
    /// How questions are worded (and the confidence cut-offs a tuned wording sets).
    pub wording: Wording,
    /// How Jev is asked (`BGWS_JEV_STYLE`): see [`crate::jev_ask::Style`].
    pub style: crate::jev_ask::Style,
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
            prose: false,
            wording: Wording::default(),
            style: crate::jev_ask::Style::from_env(),
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
    vocab: bgws_behaviour::Vocabulary,
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
        let name = if cfg.style == crate::jev_ask::Style::Legacy { name } else { format!("{name}+{}", cfg.style.word()) };
        JevClient { vocab: bgws_behaviour::Vocabulary::builtin(), http, key, cfg, cache: Mutex::new(Cache::default()), spent: Mutex::new(BTreeMap::new()), name }
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

    /// Turn an answer by id into an Answer, applying the threshold (the
    /// legacy style only: confidence measures how peaked the answer is, not
    /// how likely it is to be right).
    fn answer(&self, q: &Question, c: &JevChoiceById, source: AnswerSource, latency_ms: Option<u64>, cost: Option<f64>) -> Answer {
        let cut = if self.cfg.style == crate::jev_ask::Style::Legacy { self.cfg.wording.threshold(&q.trigger, self.cfg.min_confidence) } else { 0.0 };
        if c.confidence >= cut && q.has_option(&c.choice) {
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
        // Land units one per request (plain, judge); the rest together.
        let (single, together): (Vec<usize>, Vec<usize>) = if self.cfg.style == crate::jev_ask::Style::Legacy {
            (Vec::new(), ask)
        } else {
            ask.into_iter().partition(|&i| questions[i].kind == QuestionKind::Unit && questions[i].state.is_some())
        };
        if !single.is_empty() {
            let qs: Vec<Question> = single.iter().map(|&i| questions[i].clone()).collect();
            let answers = if self.cfg.style == crate::jev_ask::Style::Judge { self.judge(&qs).await } else { self.ask_each(side, &qs).await };
            for (i, a) in single.into_iter().zip(answers) {
                out[i] = Some(a);
            }
        }
        if !together.is_empty() {
            let batch: Vec<Question> = together.iter().map(|&i| questions[i].clone()).collect();
            let answers = self.ask(side, &batch).await;
            for (i, a) in together.into_iter().zip(answers) {
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
        let (body, keyed) = build_request_worded(&self.cfg.model, side, batch, self.cfg.directive.as_deref(), self.cfg.max_choices, self.cfg.prose, &self.cfg.wording);
        let auth = format!("Bearer {}", key.expose());
        let headers = [("Authorization", auth.as_str()), ("X-Title", "BGWS")];
        let reply = post_json(&self.http, &self.cfg.url, &headers, &body, self.cfg.timeout).await;
        self.read(side, batch, &keyed, reply)
    }

    /// Each land unit's question in its own request, all at once.
    async fn ask_each(&self, side: Side, qs: &[Question]) -> Vec<Answer> {
        let Some(key) = &self.key else {
            return qs.iter().map(|q| Answer::fallback(q, format!("Jev unavailable (no OpenRouter key: set {KEY_ENV}) — the fallback stands"))).collect();
        };
        let mut set = tokio::task::JoinSet::new();
        let mut keyed: Vec<Keyed> = Vec::new();
        for (i, q) in qs.iter().enumerate() {
            let (body, k) = crate::jev_ask::plain_request(&self.cfg.model, q, self.cfg.max_choices, &self.vocab);
            keyed.push(k);
            let (http, url, auth, timeout) = (self.http.clone(), self.cfg.url.clone(), format!("Bearer {}", key.expose()), self.cfg.timeout);
            set.spawn(async move {
                let headers = [("Authorization", auth.as_str()), ("X-Title", "BGWS")];
                (i, post_json(&http, &url, &headers, &body, timeout).await)
            });
        }
        let mut replies: Vec<Option<Result<crate::http::Reply, HttpError>>> = (0..qs.len()).map(|_| None).collect();
        while let Some(done) = set.join_next().await {
            if let Ok((i, r)) = done {
                replies[i] = Some(r);
            }
        }
        qs.iter()
            .zip(keyed)
            .zip(replies)
            .map(|((q, k), r)| match r {
                Some(r) => self.read(side, std::slice::from_ref(q), std::slice::from_ref(&k), r).pop().unwrap_or_else(|| Answer::fallback(q, "Jev gave no answer")),
                None => Answer::fallback(q, "Jev's request did not finish — the fallback stands"),
            })
            .collect()
    }

    /// The judge style: each unit's decision from its point's table, with
    /// Jev asked only for the judgments a row turns on (all at once); a
    /// point without a table takes the doctrine default.
    async fn judge(&self, qs: &[Question]) -> Vec<Answer> {
        use crate::jev_judge::{decide, judge_request, judgments, parse_nouls, read, Facts};
        let facts: Vec<Facts> = qs.iter().map(|q| q.state.as_ref().map(Facts::from_state).unwrap_or_default()).collect();
        let mut set = tokio::task::JoinSet::new();
        for (i, (q, f)) in qs.iter().zip(&facts).enumerate() {
            let js = judgments(&q.trigger, f);
            let Some(key) = self.key.as_ref().filter(|_| !js.is_empty()) else { continue };
            let body = judge_request(&self.cfg.model, q, &js);
            let (http, url, auth, timeout) = (self.http.clone(), self.cfg.url.clone(), format!("Bearer {}", key.expose()), self.cfg.timeout);
            set.spawn(async move {
                let headers = [("Authorization", auth.as_str()), ("X-Title", "BGWS")];
                (i, post_json(&http, &url, &headers, &body, timeout).await)
            });
        }
        let mut replies: Vec<Option<crate::http::Reply>> = (0..qs.len()).map(|_| None).collect();
        while let Some(done) = set.join_next().await {
            if let Ok((i, Ok(r))) = done {
                replies[i] = Some(r);
            }
        }
        qs.iter()
            .zip(&facts)
            .zip(replies)
            .map(|((q, f), reply)| {
                let nouls = reply.as_ref().map(|r| parse_nouls(&r.body)).unwrap_or_default();
                let cost = reply.as_ref().and_then(|r| r.body.pointer("/usage/cost").and_then(Value::as_f64));
                if let (Some(c), Ok(mut m)) = (cost, self.spent.lock()) {
                    *m.entry(q.side).or_insert(0.0) += c;
                }
                let judged = |k: &str| read(nouls.get(k).copied());
                let said: Vec<String> = nouls.iter().map(|(k, p)| format!("{k} {:.0}%", p * 100.0)).collect();
                let (choice, why) = match decide(&q.trigger, f, &judged) {
                    Some((prefer, row)) => match prefer.iter().find(|id| q.has_option(id)) {
                        Some(id) => (id.to_string(), format!("table {}: {row}", q.trigger)),
                        None => (q.rules_choice.clone(), format!("table {} ({row}) offers none of its options here: the doctrine's choice", q.trigger)),
                    },
                    None => (q.rules_choice.clone(), "no table at this point: the doctrine's choice".to_string()),
                };
                let choice = if q.has_option(&choice) { choice } else { q.fallback.clone() };
                Answer {
                    question: q.id,
                    choice,
                    probabilities: None,
                    confidence: None,
                    source: if reply.is_some() { AnswerSource::Jev } else { AnswerSource::Rules },
                    plan: None,
                    cost_usd: cost,
                    latency_ms: reply.as_ref().map(|r| r.latency_ms),
                    rationale: Some(if said.is_empty() { why } else { format!("{why} (Jev judged: {})", said.join(", ")) }),
                }
            })
            .collect()
    }

    /// Read one reply's answers for its questions.
    fn read(&self, side: Side, batch: &[Question], keyed: &[Keyed], reply: Result<crate::http::Reply, HttpError>) -> Vec<Answer> {
        let reply = match reply {
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
            .zip(keyed)
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
