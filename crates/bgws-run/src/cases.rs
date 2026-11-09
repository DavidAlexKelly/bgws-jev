//! Decision cases: hand-written situations with the answers we expect, to
//! test a decider (Jev above all) quickly and repeatably, without playing
//! games.
//!
//! A case is a unit's state (the same named fields the game sends, see
//! `bgws_sim::decide::unit_state`), a behaviour template and a decision
//! point. Its question is built the way the game builds one: the template's
//! options for that point, after the hard gates (ROE, weapons, cohesion)
//! read from the state, with the template's doctrine and leanings. Then:
//!
//! - **acceptable** options (any of them passes), **wrong** ones (a fail),
//!   anything else counts as neither, and **why**, in a sentence;
//! - **variations**: the same case with one or more facts changed, each with
//!   its own expectations, and optionally a **trend**: as the variations go
//!   on, the share the decider puts on some options should not fall;
//! - **invariance**, checked for every case: the same choice with the
//!   options in reverse order, and with the unit renamed.
//!
//! Cases are JSON: the shipped starter set in `data/cases/`, and the lab's
//! own (made from moments) in `<lab>/cases/`. `bases.json` holds starting
//! states (a tank troop, an infantry section…) a case changes with `set`
//! (dotted paths: `enemiesSeen.0.rangeM`). After every change the derived
//! fields are worked out again: `weaponReachM` from the weapons with rounds
//! left, and each enemy's `inOurReach` and `canReachUs` from its range.

use crate::host::DeciderChoice;
use anyhow::{anyhow, Context, Result};
use bgws_behaviour::{build_question, Capability, Cohesion, FactValue, Gates, QuestionContext, QuestionLimits, Registry, Roe, RulesChoice, WeaponRelease};
use bgws_core::{Figures, Question, QuestionId, QuestionKind, Side, SimDuration, SimTime};
use bgws_decide::Decider;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A starting state cases build on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Base {
    pub template: String,
    pub state: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Case {
    pub id: String,
    /// The decision point (D0…D12).
    pub point: String,
    /// In plain words: why the expected answers are right.
    pub why: String,
    /// A base in `bases.json`, changed by `set`; or a whole `state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    /// Dotted path → value, applied to the base's (or the case's) state.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, Value>,
    pub acceptable: Vec<String>,
    #[serde(default)]
    pub wrong: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variations: Vec<Variation>,
    /// Where it came from (a lab moment), if anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variation {
    pub name: String,
    pub steps: Vec<Step>,
    /// As the steps go on, the share on these options should not fall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub label: String,
    pub set: BTreeMap<String, Value>,
    /// Expectations for this step; the case's own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptable: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrong: Option<Vec<String>>,
}

/// One question to ask: a case, or one of its variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub case: String,
    pub point: String,
    /// `base`, `<variation>: <step>`, `reordered`, `renamed`.
    pub variant: String,
    pub question: Question,
    /// Empty for invariance variants: they are judged against the base's answer.
    pub acceptable: Vec<String>,
    pub wrong: Vec<String>,
}

/// The case folders: the shipped set and the lab's.
pub fn case_dirs(lab: &Path) -> Vec<PathBuf> {
    vec![crate::data_dir().join("cases"), lab.join("cases")]
}

/// Read every case (and the bases) from these folders.
pub fn load(dirs: &[PathBuf]) -> Result<(BTreeMap<String, Base>, Vec<Case>)> {
    let mut bases = BTreeMap::new();
    let mut cases = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json")).collect();
        paths.sort();
        for p in paths {
            let text = std::fs::read_to_string(&p)?;
            if p.file_name().and_then(|n| n.to_str()) == Some("bases.json") {
                let b: BTreeMap<String, Base> = serde_json::from_str(&text).with_context(|| format!("{}", p.display()))?;
                bases.extend(b);
            } else {
                // A file holds one case or a list of them.
                let v: Value = serde_json::from_str(&text).with_context(|| format!("{}", p.display()))?;
                let list: Vec<Case> = if v.is_array() { serde_json::from_value(v) } else { serde_json::from_value(v).map(|c| vec![c]) }.with_context(|| format!("{}", p.display()))?;
                cases.extend(list);
            }
        }
    }
    cases.sort_by(|a, b| (a.point.trim_start_matches('D').parse::<u32>().unwrap_or(99), &a.id).cmp(&(b.point.trim_start_matches('D').parse::<u32>().unwrap_or(99), &b.id)));
    Ok((bases, cases))
}

/// Set a dotted path (`a.b.0.c`) in a JSON value, making objects on the way.
pub fn set_path(v: &mut Value, path: &str, x: Value) -> Result<()> {
    let mut cur = v;
    let parts: Vec<&str> = path.split('.').collect();
    for (k, p) in parts.iter().enumerate() {
        let last = k + 1 == parts.len();
        if let Ok(i) = p.parse::<usize>() {
            let a = cur.as_array_mut().ok_or_else(|| anyhow!("{path}: {p} indexes something that is not a list"))?;
            if i > a.len() {
                return Err(anyhow!("{path}: no item {i}"));
            }
            if i == a.len() {
                a.push(json!({}));
            }
            if last {
                a[i] = x;
                return Ok(());
            }
            cur = &mut a[i];
        } else {
            if !cur.is_object() {
                *cur = json!({});
            }
            let o = cur.as_object_mut().expect("object");
            if last {
                if x.is_null() {
                    o.remove(*p);
                } else {
                    o.insert((*p).to_string(), x);
                }
                return Ok(());
            }
            cur = o.entry((*p).to_string()).or_insert_with(|| json!({}));
        }
    }
    Ok(())
}

/// Work the derived fields out again: weapon reach, and each enemy's reach.
pub fn derive(st: &mut Value) {
    if let Some(ws) = st.get("weapons").and_then(Value::as_array) {
        let reach = ws.iter().filter(|w| w.get("rounds").and_then(Value::as_f64).unwrap_or(1.0) > 0.0).filter_map(|w| w.get("maxRangeM").and_then(Value::as_f64)).fold(0.0, f64::max);
        st["weaponReachM"] = json!(reach);
    }
    let reach = st.get("weaponReachM").and_then(Value::as_f64).unwrap_or(0.0);
    if let Some(es) = st.get_mut("enemiesSeen").and_then(Value::as_array_mut) {
        for e in es {
            let Some(r) = e.get("rangeM").and_then(Value::as_f64) else { continue };
            e["inOurReach"] = json!(reach > 0.0 && r <= reach);
            if let Some(theirs) = e.get("itsReachM").and_then(Value::as_f64) {
                e["canReachUs"] = json!(r <= theirs);
            }
        }
    }
}

/// The facts and gates the template's walk reads, from the state (as the game works them out).
fn facts_and_gates(st: &Value) -> (BTreeMap<String, FactValue>, Gates) {
    let s = |p: &str| st.pointer(p).and_then(Value::as_str).unwrap_or("");
    let n = |p: &str| st.pointer(p).and_then(Value::as_f64);
    let b = |p: &str| st.pointer(p).and_then(Value::as_bool);
    let class = st.pointer("/unit/class").and_then(Value::as_str).unwrap_or("armoured_vehicle").to_string();
    let cohesion = match s("/unit/morale") {
        "shaken" | "pinned" => ("shaken", Cohesion::Shaken),
        "broken" => ("broken", Cohesion::Broken),
        _ => ("steady", Cohesion::Steady),
    };
    let enemies = st.get("enemiesSeen").and_then(Value::as_array).cloned().unwrap_or_default();
    let nearest = enemies.iter().filter_map(|e| e.get("rangeM").and_then(Value::as_f64)).fold(None, |a: Option<f64>, r| Some(a.map_or(r, |x| x.min(r))));
    let mut f = BTreeMap::new();
    f.insert("self.class".into(), FactValue::Text(class));
    f.insert("self.strength".into(), FactValue::Num((n("/unit/strengthPct").unwrap_or(100.0) / 10.0).round()));
    f.insert("self.cohesion".into(), FactValue::Text(cohesion.0.into()));
    f.insert("self.inCover".into(), FactValue::Bool(n("/ground/cover").unwrap_or(0.0) > 0.3 || s("/unit/posture") == "hull-down in cover"));
    f.insert("self.deadGround".into(), b("/ground/deadGround").map(FactValue::Bool).unwrap_or(FactValue::Null));
    f.insert("self.onKeyTerrain".into(), FactValue::Bool(false));
    f.insert("enemy.known".into(), FactValue::Bool(nearest.is_some()));
    f.insert("enemy.rangeM".into(), nearest.map(|r| FactValue::Num(r.round())).unwrap_or(FactValue::Null));
    f.insert("orders.urgent".into(), FactValue::Bool(b("/orders/urgent").unwrap_or(false)));
    for w in bgws_behaviour::facts::world_facts() {
        f.entry(w.name.to_string()).or_insert(FactValue::Null);
    }
    let mut caps = BTreeSet::from([Capability::Radio, Capability::Mobility]);
    if n("/weaponReachM").unwrap_or(0.0) > 0.0 {
        caps.insert(Capability::Weapon);
    }
    let g = Gates {
        roe: match s("/orders/roe") {
            "weapons hold" => Roe::Never,
            "fire only if fired upon" => Roe::IfFiredUpon,
            "fire within short range" => Roe::WithinShortRange,
            _ => Roe::Always,
        },
        fired_upon: b("/firedUpon").unwrap_or(false) || st.get("firedOnByUnseen").and_then(Value::as_array).is_some_and(|a| !a.is_empty()),
        weapon_release: WeaponRelease::Free,
        target_identified: true,
        emcon: Default::default(),
        capabilities: Some(caps),
        cohesion: cohesion.1,
    };
    (f, g)
}

/// The situation in words, from the state (for `--prose` and Claude).
pub fn words(st: &Value) -> String {
    let s = |p: &str| st.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |p: &str| st.pointer(p).and_then(Value::as_f64).unwrap_or(0.0);
    let mut out = format!(
        "{} ({}), {} fit, {}, {}. Suppression {:.0}%. Mission: {}.",
        s("/unit/name"),
        s("/unit/type"),
        s("/unit/vehicles"),
        s("/orders/doing"),
        s("/unit/posture"),
        n("/unit/suppressionPct"),
        s("/orders/purpose")
    );
    for e in st.get("enemiesSeen").and_then(Value::as_array).into_iter().flatten() {
        let g = |k: &str| e.get(k).cloned().unwrap_or(Value::Null);
        out.push_str(&format!(
            " Sees {} {:.0} m to the {}{}{}.",
            g("what").as_str().unwrap_or("something"),
            g("rangeM").as_f64().unwrap_or(0.0),
            g("bearing").as_str().unwrap_or("?"),
            if g("moving").as_bool().unwrap_or(false) { ", moving" } else { "" },
            if g("thisDecisionIsAboutIt").as_bool().unwrap_or(false) { " (this decision is about it)" } else { "" }
        ));
    }
    for u in st.get("firedOnByUnseen").and_then(Value::as_array).into_iter().flatten() {
        out.push_str(&format!(" Fired on from the {}, shooter not seen.", u.get("bearing").and_then(Value::as_str).unwrap_or("?")));
    }
    out
}

/// Build a case's question (and its variants') the way the game does.
pub struct Builder {
    registry: Registry,
    limits: QuestionLimits,
    bases: BTreeMap<String, Base>,
    next: u64,
}

impl Builder {
    pub fn new(bases: BTreeMap<String, Base>, figures: &Figures) -> Result<Builder> {
        let registry = Registry::builtin().map_err(|e| anyhow!("behaviour templates: {e}"))?;
        Ok(Builder { registry, limits: QuestionLimits::from_figures(figures), bases, next: 1 })
    }

    /// The case's own state and template, changes applied, derived fields worked out.
    pub fn state_of(&self, c: &Case) -> Result<(String, Value)> {
        let (template, mut st) = match (&c.base, &c.state) {
            (Some(b), _) => {
                let b = self.bases.get(b).ok_or_else(|| anyhow!("{}: no base {b}", c.id))?;
                (b.template.clone(), b.state.clone())
            }
            (None, Some(s)) => (String::new(), s.clone()),
            (None, None) => return Err(anyhow!("{}: give a base or a state", c.id)),
        };
        if let Some(s) = (c.base.is_some()).then_some(()).and(c.state.as_ref()) {
            // A base and a state: the state's top-level fields replace the base's.
            for (k, v) in s.as_object().into_iter().flatten() {
                st[k] = v.clone();
            }
        }
        for (p, v) in &c.set {
            set_path(&mut st, p, v.clone()).with_context(|| c.id.clone())?;
        }
        derive(&mut st);
        let template = c.template.clone().unwrap_or(template);
        if template.is_empty() {
            return Err(anyhow!("{}: no template", c.id));
        }
        Ok((template, st))
    }

    pub fn question(&mut self, template: &str, point: &str, st: &Value) -> Result<Question> {
        let (facts, gates) = facts_and_gates(st);
        let (resolved, walk) = self.registry.decide(&[template, "core/default"], point, &facts, &gates).map_err(|e| anyhow!("{template} {point}: {e}"))?.ok_or_else(|| anyhow!("{template} has no decision point {point}"))?;
        let ctx = QuestionContext {
            id: QuestionId(self.next),
            at: SimTime::ZERO,
            kind: QuestionKind::Unit,
            agent: st.pointer("/unit/name").and_then(Value::as_str).unwrap_or("the unit").to_string(),
            side: Side::Blue,
            trigger: point.to_string(),
            template: resolved.id.clone(),
            situation: words(st),
            state: Some(st.clone()),
            reaction: SimDuration::ZERO,
            summaries: BTreeMap::new(),
            expansions: BTreeMap::new(),
            rule: RulesChoice::Fallback,
        };
        self.next += 1;
        build_question(&walk, &ctx, self.registry.vocabulary(), self.limits).ok_or_else(|| anyhow!("{template} {point}: no options left after the gates"))
    }

    /// A case's items: the case itself, each variation step, and the invariance variants.
    pub fn items(&mut self, c: &Case) -> Result<Vec<Item>> {
        let (template, st) = self.state_of(c)?;
        let q = self.question(&template, &c.point, &st)?;
        let check = |q: &Question, ids: &[String], what: &str| -> Result<()> {
            for id in ids {
                if !q.has_option(id) {
                    return Err(anyhow!("{}: {what} option `{id}` is not offered (offered: {})", c.id, q.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", ")));
                }
            }
            Ok(())
        };
        check(&q, &c.acceptable, "acceptable")?;
        check(&q, &c.wrong, "wrong")?;
        let mut out = vec![Item { case: c.id.clone(), point: c.point.clone(), variant: "base".into(), question: q.clone(), acceptable: c.acceptable.clone(), wrong: c.wrong.clone() }];
        for v in &c.variations {
            for s in &v.steps {
                let mut st2 = st.clone();
                for (p, x) in &s.set {
                    set_path(&mut st2, p, x.clone()).with_context(|| format!("{} {}", c.id, v.name))?;
                }
                derive(&mut st2);
                let q2 = self.question(&template, &c.point, &st2)?;
                let acceptable = s.acceptable.clone().unwrap_or_else(|| c.acceptable.clone());
                let wrong = s.wrong.clone().unwrap_or_else(|| c.wrong.clone());
                check(&q2, &acceptable, "acceptable")?;
                check(&q2, &wrong, "wrong")?;
                out.push(Item { case: c.id.clone(), point: c.point.clone(), variant: format!("{}: {}", v.name, s.label), question: q2, acceptable, wrong });
            }
        }
        // The same question with its options in reverse order, and with the unit renamed.
        let mut rev = q.clone();
        rev.options.reverse();
        rev.id = QuestionId(self.next);
        self.next += 1;
        out.push(Item { case: c.id.clone(), point: c.point.clone(), variant: "reordered".into(), question: rev, acceptable: vec![], wrong: vec![] });
        let mut st3 = st.clone();
        set_path(&mut st3, "unit.name", json!("Unit K"))?;
        let mut ren = self.question(&template, &c.point, &st3)?;
        ren.agent = "Unit K".into();
        out.push(Item { case: c.id.clone(), point: c.point.clone(), variant: "renamed".into(), question: ren, acceptable: vec![], wrong: vec![] });
        Ok(out)
    }
}

/// What a decider said for one item, and the verdict.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub case: String,
    pub point: String,
    pub variant: String,
    pub choice: String,
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub source: String,
    /// pass, wrong, other; for invariance variants same or changed.
    pub verdict: String,
    pub acceptable: Vec<String>,
    pub wrong: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendResult {
    pub case: String,
    pub variation: String,
    pub toward: Vec<String>,
    /// The share on `toward` at each step (1/0 from the choice when there are no probabilities).
    pub series: Vec<f64>,
    pub pass: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointSummary {
    pub point: String,
    pub items: usize,
    pub pass: usize,
    pub wrong: usize,
    pub other: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRun {
    pub decider: String,
    pub cases: usize,
    pub items: Vec<ItemResult>,
    pub trends: Vec<TrendResult>,
    pub by_point: Vec<PointSummary>,
    pub pass: usize,
    pub wrong: usize,
    pub other: usize,
    /// Invariance variants that kept the base's choice, of those asked.
    pub invariant: usize,
    pub invariance_checks: usize,
    pub mean_confidence_pass: Option<f64>,
    pub mean_confidence_wrong: Option<f64>,
    pub cost_usd: f64,
    /// Answers that were the template's fallback because the decider could not answer (no key, an error, unsure).
    #[serde(default)]
    pub fell_back: usize,
}

fn verdict(choice: &str, acceptable: &[String], wrong: &[String]) -> &'static str {
    if acceptable.iter().any(|a| a == choice) {
        "pass"
    } else if wrong.iter().any(|w| w == choice) {
        "wrong"
    } else {
        "other"
    }
}

/// Ask a decider every item, one question per request (a few at a time),
/// and judge the answers. `name` is what the run is kept under.
pub fn run(cases: &[Case], items: &[Item], decider: Arc<dyn Decider>, name: &str) -> Result<TestRun> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let answers: Vec<bgws_core::Answer> = rt.block_on(async {
        let mut out = Vec::with_capacity(items.len());
        for chunk in items.chunks(8) {
            let mut set = tokio::task::JoinSet::new();
            for (k, it) in chunk.iter().enumerate() {
                let d = decider.clone();
                let q = it.question.clone();
                set.spawn(async move { (k, d.decide(Side::Blue, vec![q.clone()]).await.into_iter().next().unwrap_or_else(|| bgws_core::Answer::fallback(&q, "no answer"))) });
            }
            let mut got: Vec<(usize, bgws_core::Answer)> = Vec::new();
            while let Some(r) = set.join_next().await {
                got.push(r.map_err(|e| anyhow!("{e}"))?);
            }
            got.sort_by_key(|x| x.0);
            out.extend(got.into_iter().map(|x| x.1));
        }
        Ok::<_, anyhow::Error>(out)
    })?;
    Ok(judge(cases, items, &answers, name))
}

/// The yardstick that needs no asking: each question's template fallback.
pub fn fallback_answers(items: &[Item]) -> Vec<bgws_core::Answer> {
    items.iter().map(|it| bgws_core::Answer::simple(&it.question, it.question.fallback.clone(), bgws_core::AnswerSource::Rules)).collect()
}

pub fn judge(cases: &[Case], items: &[Item], answers: &[bgws_core::Answer], name: &str) -> TestRun {
    let source = |a: &bgws_core::Answer| match &a.source {
        bgws_core::AnswerSource::Fallback { reason } => format!("fallback: {reason}"),
        other => serde_json::to_value(other).ok().and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned)).unwrap_or_default(),
    };
    let mut results: Vec<ItemResult> = Vec::new();
    for (it, a) in items.iter().zip(answers) {
        let v = if it.variant == "reordered" || it.variant == "renamed" {
            let base = results.iter().find(|r| r.case == it.case && r.variant == "base").map(|r| r.choice.clone()).unwrap_or_default();
            if base == a.choice { "same" } else { "changed" }
        } else {
            verdict(&a.choice, &it.acceptable, &it.wrong)
        };
        results.push(ItemResult {
            case: it.case.clone(),
            point: it.point.clone(),
            variant: it.variant.clone(),
            choice: a.choice.clone(),
            probabilities: a.probabilities.clone().unwrap_or_default(),
            confidence: a.confidence,
            source: source(a),
            verdict: v.into(),
            acceptable: it.acceptable.clone(),
            wrong: it.wrong.clone(),
        });
    }
    // Trends.
    let mut trends = Vec::new();
    for c in cases {
        for v in c.variations.iter().filter(|v| v.trend.is_some()) {
            let toward = v.trend.clone().unwrap_or_default();
            let series: Vec<f64> = v
                .steps
                .iter()
                .filter_map(|s| results.iter().find(|r| r.case == c.id && r.variant == format!("{}: {}", v.name, s.label)))
                .map(|r| if r.probabilities.is_empty() { f64::from(u8::from(toward.contains(&r.choice))) } else { toward.iter().filter_map(|t| r.probabilities.get(t)).sum() })
                .collect();
            // Never a fall of more than 0.15 from one step to the next, and no lower at the end than at the start.
            let pass = series.windows(2).all(|w| w[1] >= w[0] - 0.15) && series.last() >= series.first();
            trends.push(TrendResult { case: c.id.clone(), variation: v.name.clone(), toward, series, pass });
        }
    }
    let judged: Vec<&ItemResult> = results.iter().filter(|r| r.variant != "reordered" && r.variant != "renamed").collect();
    let count = |v: &str, of: &[&ItemResult]| of.iter().filter(|r| r.verdict == v).count();
    let mut points: Vec<String> = judged.iter().map(|r| r.point.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    points.sort_by_key(|p| p.trim_start_matches('D').parse::<u32>().unwrap_or(99));
    let by_point = points
        .iter()
        .map(|p| {
            let of: Vec<&ItemResult> = judged.iter().copied().filter(|r| &r.point == p).collect();
            PointSummary { point: p.clone(), items: of.len(), pass: count("pass", &of), wrong: count("wrong", &of), other: count("other", &of) }
        })
        .collect();
    let mean = |v: &str| {
        let c: Vec<f64> = judged.iter().filter(|r| r.verdict == v).filter_map(|r| r.confidence).collect();
        (!c.is_empty()).then(|| c.iter().sum::<f64>() / c.len() as f64)
    };
    let inv: Vec<&ItemResult> = results.iter().filter(|r| r.variant == "reordered" || r.variant == "renamed").collect();
    TestRun {
        decider: name.to_string(),
        cases: cases.len(),
        pass: count("pass", &judged),
        wrong: count("wrong", &judged),
        other: count("other", &judged),
        invariant: inv.iter().filter(|r| r.verdict == "same").count(),
        invariance_checks: inv.len(),
        mean_confidence_pass: mean("pass"),
        mean_confidence_wrong: mean("wrong"),
        cost_usd: answers.iter().filter_map(|a| a.cost_usd).sum(),
        fell_back: answers.iter().filter(|a| matches!(a.source, bgws_core::AnswerSource::Fallback { .. })).count(),
        items: results,
        trends,
        by_point,
    }
}

impl TestRun {
    pub fn table(&self) -> String {
        let n = self.pass + self.wrong + self.other;
        let pct = |a: usize, b: usize| if b > 0 { format!("{:.0}%", 100.0 * a as f64 / b as f64) } else { "–".into() };
        let mut out = format!(
            "{}: {} cases, {} questions judged: {} pass ({}), {} wrong ({}), {} neither. Invariance {}/{}. Trends {}/{}.{}{}\n\n| point | questions | pass | wrong | neither |\n|---|---|---|---|---|\n",
            self.decider,
            self.cases,
            n,
            self.pass,
            pct(self.pass, n),
            self.wrong,
            pct(self.wrong, n),
            self.other,
            self.invariant,
            self.invariance_checks,
            self.trends.iter().filter(|t| t.pass).count(),
            self.trends.len(),
            self.mean_confidence_pass.map(|c| format!(" Confidence when right {:.0}%", c * 100.0)).unwrap_or_default(),
            self.mean_confidence_wrong.map(|c| format!(", when wrong {:.0}%.", c * 100.0)).unwrap_or_default(),
        );
        if self.fell_back > 0 {
            let why = self.items.iter().find(|r| r.source.starts_with("fallback")).map(|r| r.source.clone()).unwrap_or_default();
            out = format!("WARNING: {} of {} answers were the template's fallback, not the decider's ({why}).\n\n{out}", self.fell_back, self.items.len());
        }
        for p in &self.by_point {
            out.push_str(&format!("| {} | {} | {} | {} | {} |\n", p.point, p.items, p.pass, p.wrong, p.other));
        }
        let fails: Vec<&ItemResult> = self.items.iter().filter(|r| r.verdict == "wrong" || r.verdict == "changed").collect();
        if !fails.is_empty() {
            out.push_str("\nWrong or inconsistent:\n");
            for r in fails {
                out.push_str(&format!("- {} [{}] chose {} ({}){}\n", r.case, r.variant, r.choice, r.source, if r.acceptable.is_empty() { String::new() } else { format!("; acceptable: {}", r.acceptable.join(", ")) }));
            }
        }
        for t in self.trends.iter().filter(|t| !t.pass) {
            out.push_str(&format!("- trend {} [{}] toward {}: {:?}\n", t.case, t.variation, t.toward.join("/"), t.series.iter().map(|x| (x * 100.0).round() / 100.0).collect::<Vec<_>>()));
        }
        out
    }
}

/// Every item of these cases, or the first error (a case that does not build).
pub fn build_all(b: &mut Builder, cases: &[Case]) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    for c in cases {
        out.extend(b.items(c)?);
    }
    Ok(out)
}

/// Where test runs are kept: `<lab>/tests/<name>.json`.
pub fn save_run(lab: &Path, run: &TestRun) -> Result<PathBuf> {
    let dir = lab.join("tests");
    std::fs::create_dir_all(&dir)?;
    let file: String = run.decider.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    let p = dir.join(format!("{file}.json"));
    std::fs::write(&p, serde_json::to_vec_pretty(run)?)?;
    Ok(p)
}

pub fn runs(lab: &Path) -> Vec<TestRun> {
    let mut v: Vec<TestRun> = std::fs::read_dir(lab.join("tests")).into_iter().flatten().filter_map(|e| e.ok()).filter_map(|e| std::fs::read(e.path()).ok()).filter_map(|b| serde_json::from_slice(&b).ok()).collect();
    v.sort_by(|a, b| a.decider.cmp(&b.decider));
    v
}

/// A case from a lab moment: its state and options as put, the acceptable
/// options from your label (else the play-outs' near-best), the wrong ones
/// from the play-outs (clearly worse than the best).
pub fn case_from_moment(m: &crate::lab::Moment, why: &str) -> Result<Case> {
    let st = m.question.state.clone().ok_or_else(|| anyhow!("{}: the moment has no state (collected before units sent fields)", m.id))?;
    let label = m.label.clone().unwrap_or_default();
    let mut acceptable: Vec<String> = label.best.iter().cloned().chain(label.acceptable.iter().cloned()).collect();
    let p = m.playouts.as_ref().filter(|p| p.decisive);
    if acceptable.is_empty() {
        acceptable = p.map(|p| p.near_best.clone()).unwrap_or_default();
    }
    if acceptable.is_empty() {
        return Err(match &m.playouts {
            Some(p) if !p.decisive => anyhow!("{}: its play-outs are not decisive (every option within the noise of the best), so they say nothing about right and wrong; label it yourself if you have a view", m.id),
            _ => anyhow!("{}: label it (best or acceptable) or play it out first", m.id),
        });
    }
    let wrong = p.map(|p| p.options.iter().map(|o| o.option.clone()).filter(|o| !p.near_best.contains(o) && !acceptable.contains(o)).collect()).unwrap_or_default();
    // The template as resolved for the unit (custom ones fall back to the default's options).
    let template = if m.question.template.is_empty() { "core/default".into() } else { m.question.template.clone() };
    Ok(Case {
        id: format!("moment-{}", m.id),
        point: m.trigger.clone(),
        why: if why.trim().is_empty() { label.note.clone() } else { why.to_string() },
        base: None,
        template: Some(template),
        state: Some(st),
        set: BTreeMap::new(),
        acceptable,
        wrong,
        variations: vec![],
        source: Some(m.id.clone()),
    })
}

pub fn save_case(lab: &Path, c: &Case) -> Result<PathBuf> {
    let dir = lab.join("cases");
    std::fs::create_dir_all(&dir)?;
    let p = dir.join(format!("{}.json", c.id));
    std::fs::write(&p, serde_json::to_vec_pretty(c)?)?;
    Ok(p)
}

/// The decider to test, by choice (as the lab grades).
pub fn decider(choice: DeciderChoice, figures: &Figures, model: Option<String>, prose: bool) -> Result<(Arc<dyn Decider>, String)> {
    crate::lab::decider_for(choice, figures, model, prose, ("http://127.0.0.1:8080", "local"))
}
