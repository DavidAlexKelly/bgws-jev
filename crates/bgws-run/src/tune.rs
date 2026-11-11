//! Tuning how Jev is asked: a larger model (Claude, through OpenRouter)
//! rewrites the wording Jev is given (`bgws_decide::jev::Wording`), and the
//! decision cases and the engagement suite judge each rewrite.
//!
//! One round:
//! 1. The proposer sees the current wording, the cases Jev gets wrong on the
//!    training set (the situation, its options as worded, Jev's
//!    probabilities, the answers expected and why), the suite's results,
//!    and the rules below, and proposes a few rewrites.
//! 2. Each rewrite is checked before it is tried: every placeholder kept, no
//!    piece too long, option descriptions only for real actions, and nothing
//!    that names a case: no case ids, unit or platform names from the cases,
//!    and no numbers of three digits or more (no ranges), so a rewrite can
//!    only help through general wording.
//! 3. Each one that passes is asked the training cases. It must keep every
//!    consistency check (same answer reordered and renamed) and every trend
//!    the current wording keeps, and score better (passes less wrongs).
//! 4. The best is played on the suite (the scenarios not held out). It is
//!    taken only if it does not do worse there than the current wording,
//!    beyond the noise.
//!
//! After the rounds, the confidence cut-off is tuned on the training
//! answers. Last, the result is checked against the original on what it
//! never saw: the held-out cases and the held-out scenarios. Nothing changes
//! the game until it is accepted (`bgws lab tune --accept <run>`), which
//! writes `data/jev/wording.json`.

use crate::cases::{self, Case, Item, TestRun};
use crate::suite::{self, Entry, SuiteRun};
use anyhow::{anyhow, Context, Result};
use bgws_core::Figures;
use bgws_decide::jev::{JevClient, JevConfig, Wording};
use bgws_decide::Decider;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_PROPOSER: &str = "anthropic/claude-opus-5.5";

#[derive(Clone, Debug)]
pub struct TuneOptions {
    pub rounds: usize,
    pub candidates: usize,
    /// The proposer, an OpenRouter chat model.
    pub proposer: String,
    /// Jev's model (else the configured one).
    pub jev_model: Option<String>,
    /// Judge on the suite too (else on the cases only).
    pub use_suite: bool,
    pub suite_seeds: Vec<u64>,
    /// Share of cases held out from tuning (by a hash of the id).
    pub held_out: f64,
}

impl Default for TuneOptions {
    fn default() -> Self {
        TuneOptions { rounds: 3, candidates: 4, proposer: DEFAULT_PROPOSER.into(), jev_model: None, use_suite: true, suite_seeds: vec![1, 2, 3, 4], held_out: 0.3 }
    }
}

/// How a wording did.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    /// (passes − wrongs) / judged questions.
    pub cases: f64,
    pub pass: usize,
    pub wrong: usize,
    pub judged: usize,
    pub invariant: usize,
    pub invariance_checks: usize,
    pub trends: usize,
    pub trend_checks: usize,
    /// The suite's result Δ + exchange Δ over all pairs, and its standard error (result).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_se: Option<f64>,
}

fn score_of(r: &TestRun) -> Score {
    let judged = r.pass + r.wrong + r.other;
    Score {
        cases: (r.pass as f64 - r.wrong as f64) / judged.max(1) as f64,
        pass: r.pass,
        wrong: r.wrong,
        judged,
        invariant: r.invariant,
        invariance_checks: r.invariance_checks,
        trends: r.trends.iter().filter(|t| t.pass).count(),
        trend_checks: r.trends.len(),
        suite: None,
        suite_se: None,
    }
}

fn with_suite(mut s: Score, r: &SuiteRun) -> Score {
    if let Some(all) = r.summaries.iter().find(|x| x.scope == "all") {
        s.suite = Some(all.result.mean + all.exchange.mean);
        s.suite_se = Some(all.result.se);
    }
    s
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub round: usize,
    pub rationale: String,
    pub wording: Wording,
    /// Why it was not tried, if it was not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Score>,
    #[serde(default)]
    pub taken: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuneRun {
    pub id: String,
    pub proposer: String,
    pub jev_model: String,
    pub train_cases: Vec<String>,
    pub held_out_cases: Vec<String>,
    pub original: Wording,
    pub tuned: Wording,
    /// Training cases (and suite): the original, then after each round.
    pub train_scores: Vec<Score>,
    pub candidates: Vec<Candidate>,
    /// The confidence cut-off: chosen, its training score, the score at the old one.
    pub threshold: (f64, f64, f64),
    /// What it never saw: the held-out cases (and held-out scenarios), original then tuned.
    pub held_out_original: Score,
    pub held_out_tuned: Score,
    pub cost_usd: f64,
    pub notes: Vec<String>,
}

/// Cases held out from tuning: the share of them whose ids hash lowest
/// (at least one), so the split is fixed and the right size.
pub fn split(cases: &[Case], share: f64) -> (Vec<Case>, Vec<Case>) {
    let mut by: Vec<(u64, &Case)> = cases.iter().map(|c| (bgws_core::rng::fnv1a(c.id.as_bytes()), c)).collect();
    by.sort_by_key(|x| x.0);
    let n = ((cases.len() as f64 * share).round() as usize).clamp(usize::from(cases.len() > 1), cases.len().saturating_sub(1));
    let held: BTreeSet<&str> = by.iter().take(n).map(|x| x.1.id.as_str()).collect();
    (cases.iter().filter(|c| !held.contains(c.id.as_str())).cloned().collect(), cases.iter().filter(|c| held.contains(c.id.as_str())).cloned().collect())
}

/// The words that would name a case: ids, unit and platform names, what was seen.
fn case_words(cases: &[Case], items: &[Item]) -> BTreeSet<String> {
    let mut w: BTreeSet<String> = cases.iter().map(|c| c.id.to_lowercase()).collect();
    for it in items {
        let Some(st) = &it.question.state else { continue };
        for p in ["/unit/name", "/unit/type"] {
            if let Some(s) = st.pointer(p).and_then(Value::as_str) {
                w.insert(s.to_lowercase());
            }
        }
        for e in st.get("enemiesSeen").and_then(Value::as_array).into_iter().flatten() {
            if let Some(s) = e.get("what").and_then(Value::as_str) {
                w.insert(s.to_lowercase());
            }
        }
        for f in st.get("friendsWithin3km").and_then(Value::as_array).into_iter().flatten() {
            if let Some(s) = f.get("unit").and_then(Value::as_str) {
                w.insert(s.to_lowercase());
            }
        }
    }
    // Generic words that happen to be names ("something") are not leaks.
    w.retain(|x| x.len() >= 4 && !matches!(x.as_str(), "something" | "a vehicle or team" | "the unit"));
    w
}

/// Check a rewrite before trying it.
pub fn check(w: &Wording, words: &BTreeSet<String>, actions: &BTreeSet<String>) -> std::result::Result<(), String> {
    let need: [(&str, &str, &[&str]); 6] = [
        ("land", &w.land, &["{agent}", "{situation}", "{question}", "{point}", "{title}", "{doctrine}", "{balance}", "{others}", "{directive}"]),
        ("world", &w.world, &["{agent}", "{situation}", "{question}", "{title}", "{doctrine}", "{directive}", "{side}"]),
        ("doctrine", &w.doctrine, &["{text}"]),
        ("others", &w.others, &["{names}"]),
        ("statePointer", &w.state_pointer, &["{agent}"]),
        ("statePointerProse", &w.state_pointer_prose, &["{situation}", "{agent}"]),
    ];
    for (k, t, ps) in need {
        for p in ps {
            if !t.contains(p) {
                return Err(format!("{k} lost its {p} placeholder"));
            }
        }
    }
    for (k, t) in w.texts() {
        let max = if k.starts_with("optionWords") { 400 } else { 2_000 };
        if t.len() > max {
            return Err(format!("{k} is {} characters (at most {max})", t.len()));
        }
        let lower = t.to_lowercase();
        if let Some(x) = words.iter().find(|x| lower.contains(x.as_str())) {
            return Err(format!("{k} names something from the cases (\"{x}\"): wording must be general"));
        }
        // No ranges or other specific figures: numbers of three digits or more.
        let mut run = 0;
        for c in t.chars() {
            run = if c.is_ascii_digit() { run + 1 } else { 0 };
            if run >= 3 {
                return Err(format!("{k} has a number of three or more digits: no specific ranges or figures"));
            }
        }
    }
    if w.option_words.len() > 80 {
        return Err("too many option descriptions".into());
    }
    if let Some(a) = w.option_words.keys().find(|a| !actions.contains(*a)) {
        return Err(format!("optionWords has an unknown action {a}"));
    }
    Ok(())
}

fn jev(w: &Wording, figures: &Figures, model: &Option<String>) -> (Arc<dyn Decider>, String) {
    let mut cfg = JevConfig { wording: w.clone(), ..JevConfig::from_figures(figures) };
    if let Some(m) = model {
        cfg.model = m.clone();
    }
    let name = cfg.model.clone();
    (Arc::new(JevClient::from_env(cfg)), name)
}

/// The proposer's reply: rewrites, from an OpenRouter chat model.
/// Room for the proposer's reply: several whole wordings, each a few thousand tokens.
const PROPOSER_MAX_TOKENS: u32 = 32_000;

fn propose(proposer: &str, prompt: &str, rt: &tokio::runtime::Runtime) -> Result<(Vec<(String, Wording)>, f64)> {
    let system = "You improve how a fast decision model is asked to make small-unit tactical decisions in a wargame. You rewrite general wording only. Reply with one JSON object and nothing else.";
    let (text, cost) = rt.block_on(bgws_decide::openrouter::chat(proposer, system, prompt, PROPOSER_MAX_TOKENS, std::time::Duration::from_secs(900))).map_err(|e| anyhow!("proposer: {e}"))?;
    // The JSON object in the reply (it may come fenced).
    let (a, b) = (text.find('{').ok_or_else(|| anyhow!("proposer: no JSON in its reply"))?, text.rfind('}').ok_or_else(|| anyhow!("proposer: no JSON in its reply"))?);
    let v: Value = match serde_json::from_str(&text[a..=b]) {
        Ok(v) => v,
        Err(e) => {
            // Kept for a look: usually a reply cut off at the token limit.
            let p = std::env::temp_dir().join("bgws-proposer-unreadable.txt");
            let _ = std::fs::write(&p, &text);
            return Err(anyhow!("proposer: its JSON does not read ({e}; {} characters, kept in {})", text.len(), p.display()));
        }
    };
    let mut out = Vec::new();
    for c in v.get("candidates").and_then(Value::as_array).into_iter().flatten() {
        let rationale = c.get("rationale").and_then(Value::as_str).unwrap_or("").to_string();
        match c.get("wording").map(|w| serde_json::from_value::<Wording>(w.clone())) {
            Some(Ok(w)) => out.push((rationale, w)),
            Some(Err(e)) => out.push((format!("{rationale} [unreadable: {e}]"), Wording { land: String::new(), ..Default::default() })),
            None => {}
        }
    }
    Ok((out, cost))
}

/// What the proposer is shown: the rules, the wording, Jev's failures, the suite.
#[allow(clippy::too_many_arguments)]
fn prompt(current: &Wording, train: &TestRun, items: &[Item], cases_list: &[Case], suite_run: Option<&SuiteRun>, n: usize, actions: &[(String, String, String)], history: &[String]) -> String {
    let mut fails = Vec::new();
    for r in train.items.iter().filter(|r| r.verdict == "wrong" || r.verdict == "other" || r.verdict == "changed") {
        let Some(it) = items.iter().find(|i| i.case == r.case && i.variant == r.variant) else { continue };
        let why = cases_list.iter().find(|c| c.id == r.case).map(|c| c.why.clone()).unwrap_or_default();
        let opts: Vec<Value> = it.question.options.iter().map(|o| json!({ "id": o.id, "action": o.action, "shownAs": current.option_words.get(&o.action).cloned().unwrap_or_else(|| o.summary.clone()), "bias": o.bias })).collect();
        fails.push(json!({
            "decisionPoint": format!("{} {}", it.question.trigger, it.question.title),
            "question": it.question.question,
            "doctrine": it.question.doctrine,
            "variant": r.variant,
            "state": it.question.state,
            "options": opts,
            "jevChose": r.raw_choice,
            "jevProbabilities": r.probabilities,
            "jevConfidence": r.confidence,
            "expectedAcceptable": r.acceptable,
            "expectedWrong": r.wrong,
            "why": why,
        }));
    }
    let suite_text = suite_run.map(|s| s.table()).unwrap_or_else(|| "(not played)".into());
    let acts: Vec<Value> = actions.iter().map(|(n, l, d)| json!({ "action": n, "label": l, "description": d })).collect();
    format!(
        r#"A wargame asks a decision model, Jev, to choose for small land units (troops, sections, teams) at decision points. Jev is given a state (the unit's situation as named fields, under units.<name>) and, per question, "instructions" text and one short description per option. It returns a probability per option and a confidence. Below a confidence cut-off its choice is not taken and the doctrine default stands.

Your job: rewrite the WORDING Jev is given so it chooses better. The wording is the JSON object below. Its text fields are templates with {{placeholders}} the engine fills.

Current wording:
{current}

Fields you may change: land, world, balance, doctrine, others, statePointer, statePointerProse, landNote, worldNote, bias (four strings: strongly preferred, preferred, discouraged, strongly discouraged), optionWords (action name -> how that option is described to Jev, replacing its short label). Do not set minConfidence or pointConfidence (they are tuned separately).

Rules, checked automatically (a rewrite that breaks one is discarded):
- Keep every placeholder in each template: land needs {{directive}} {{agent}} {{balance}} {{doctrine}} {{point}} {{title}} {{situation}} {{question}} {{others}}; world needs {{directive}} {{agent}} {{side}} {{doctrine}} {{title}} {{question}} {{situation}}; doctrine {{text}}; others {{names}}; statePointer {{agent}}; statePointerProse {{situation}} {{agent}}.
- Wording must be general: it is used for every unit in every situation. Never name a unit, a platform, a weapon or a case, and use no numbers of three or more digits (no specific ranges). Teach principles (how to read the state, what the options really mean, what trade-offs matter), not answers.
- Each text at most 2000 characters; each option description at most 400.
- The same choice must come out when the options are listed in another order or the unit is renamed.

How a rewrite is judged: first on hand-written cases (passes less wrongs), keeping every consistency check; then in played games against the doctrine default (the engagement suite), where it must not do worse. The suite matters most: it is what actually happens.

The actions options can stand for (name, short label Jev sees now, longer description):
{acts}

Where Jev goes wrong now on the training cases ({nfail} questions; each with the state as given, the options as shown, Jev's probabilities, the answers expected and why):
{fails}

How Jev does in played games now (Δ is Jev minus the doctrine default, for the side it played):
{suite_text}

Earlier rounds:
{history}

Propose {n} different rewrites, each trying a distinct idea (for example: clearer option descriptions that say what each option costs and risks; guidance in the land template on reading weapon reach and threat; a different balance rule; plainer notes). Reply with exactly:
{{"candidates": [{{"rationale": "one or two sentences: the idea", "wording": {{ ...the whole wording object, every field... }} }}]}}"#,
        current = serde_json::to_string_pretty(current).unwrap_or_default(),
        acts = serde_json::to_string(&acts).unwrap_or_default(),
        nfail = fails.len(),
        fails = serde_json::to_string_pretty(&fails).unwrap_or_default(),
        history = if history.is_empty() { "(none)".to_string() } else { history.join("\n") },
    )
}

/// Run the loop. Writes `<lab>/tune/<id>/` (the run, the tuned wording) and returns the run.
pub fn tune(lab: &Path, opts: &TuneOptions, figures: &Figures, progress: &(dyn Fn(&str) + Sync)) -> Result<TuneRun> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (bases, all_cases) = cases::load(&cases::case_dirs(lab))?;
    let (train_cases, held_cases) = split(&all_cases, opts.held_out);
    let mut b = cases::Builder::new(bases, figures)?;
    let train_items = cases::build_all(&mut b, &train_cases)?;
    let held_items = cases::build_all(&mut b, &held_cases)?;
    let words = case_words(&all_cases, &train_items.iter().chain(&held_items).cloned().collect::<Vec<_>>());
    let vocab = bgws_behaviour::Vocabulary::builtin();
    let actions: Vec<(String, String, String)> = vocab.actions.values().filter(|a| a.name.starts_with("land.")).map(|a| (a.name.clone(), a.label.clone(), a.description.clone())).collect();
    let action_names: BTreeSet<String> = vocab.actions.keys().cloned().collect();
    let entries: Vec<Entry> = suite::load_entries()?;
    let (train_entries, held_entries): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| !e.held_out);
    let mut cost = 0.0;
    let mut notes = Vec::new();
    let eval_cases = |w: &Wording, cs: &[Case], its: &[Item]| -> Result<TestRun> {
        let (d, name) = jev(w, figures, &opts.jev_model);
        cases::run(cs, its, d, &name)
    };
    let eval_suite = |w: &Wording, es: &[Entry]| -> Result<SuiteRun> {
        let (d, name) = jev(w, figures, &opts.jev_model);
        suite::run(es, &opts.suite_seeds, d, &name, figures, 6, &suite::Options::default(), &|_| {})
    };
    let original = crate::jev_wording();
    let jev_model = jev(&original, figures, &opts.jev_model).1;
    progress(&format!("{} training cases ({} questions), {} held out; judging the current wording", train_cases.len(), train_items.len(), held_cases.len()));
    let mut current = original.clone();
    let mut cur_run = eval_cases(&current, &train_cases, &train_items)?;
    cost += cur_run.cost_usd;
    let mut cur_suite = if opts.use_suite && !train_entries.is_empty() {
        let s = eval_suite(&current, &train_entries)?;
        cost += s.cost_usd;
        Some(s)
    } else {
        None
    };
    let mut cur_score = score_of(&cur_run);
    if let Some(s) = &cur_suite {
        cur_score = with_suite(cur_score, s);
    }
    if cur_run.fell_back == cur_run.items.len() {
        return Err(anyhow!("every answer was the fallback: is {} set and openrouter.ai reachable?", bgws_decide::jev::KEY_ENV));
    }
    progress(&format!("current: cases {:+.2} ({} pass, {} wrong of {}), suite {}", cur_score.cases, cur_score.pass, cur_score.wrong, cur_score.judged, cur_score.suite.map(|x| format!("{x:+.2}")).unwrap_or("–".into())));
    let mut train_scores = vec![cur_score.clone()];
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    for round in 1..=opts.rounds {
        progress(&format!("round {round}: asking {} for {} rewrites", opts.proposer, opts.candidates));
        let p = prompt(&current, &cur_run, &train_items, &train_cases, cur_suite.as_ref(), opts.candidates, &actions, &history);
        let (props, c) = match propose(&opts.proposer, &p, &rt) {
            Ok(x) => x,
            Err(e) => {
                notes.push(format!("round {round}: {e}"));
                progress(&format!("round {round}: {e}"));
                continue;
            }
        };
        cost += c;
        let mut best: Option<(usize, Score, TestRun)> = None;
        for (rationale, mut w) in props {
            // Thresholds are not the proposer's to set.
            w.min_confidence = current.min_confidence;
            w.point_confidence = current.point_confidence.clone();
            let mut cand = Candidate { round, rationale: rationale.clone(), wording: w.clone(), rejected: None, score: None, taken: false };
            if let Err(why) = check(&w, &words, &action_names) {
                progress(&format!("  rejected: {why}"));
                cand.rejected = Some(why);
                candidates.push(cand);
                continue;
            }
            let run = eval_cases(&w, &train_cases, &train_items)?;
            cost += run.cost_usd;
            let s = score_of(&run);
            let gates = s.invariant >= cur_score.invariant && s.trends >= cur_score.trends;
            progress(&format!("  cases {:+.2} ({} pass, {} wrong){}: {}", s.cases, s.pass, s.wrong, if gates { "" } else { ", fails a consistency or trend check" }, rationale.chars().take(100).collect::<String>()));
            if !gates {
                cand.rejected = Some("broke a consistency or trend check the current wording keeps".into());
            }
            cand.score = Some(s.clone());
            if gates && s.cases > cur_score.cases + 0.02 && best.as_ref().is_none_or(|b| s.cases > b.1.cases) {
                best = Some((candidates.len(), s, run));
            }
            candidates.push(cand);
        }
        let Some((k, mut s, run)) = best else {
            history.push(format!("round {round}: no rewrite beat the current wording on the cases"));
            train_scores.push(cur_score.clone());
            continue;
        };
        // The suite: not worse beyond the noise.
        if let Some(cs) = &cur_suite {
            let sr = eval_suite(&candidates[k].wording, &train_entries)?;
            cost += sr.cost_usd;
            s = with_suite(s, &sr);
            let (now, was, se) = (s.suite.unwrap_or(0.0), cs.summaries.iter().find(|x| x.scope == "all").map(|a| a.result.mean + a.exchange.mean).unwrap_or(0.0), s.suite_se.unwrap_or(0.0));
            progress(&format!("  best on the cases, in played games: {now:+.2} against {was:+.2} (±{se:.2})"));
            candidates[k].score = Some(s.clone());
            if now < was - se {
                candidates[k].rejected = Some(format!("better on the cases, worse in played games ({now:+.2} against {was:+.2})"));
                history.push(format!("round {round}: \"{}\" scored {:+.2} on the cases but {now:+.2} in games against {was:+.2}: rejected", candidates[k].rationale, s.cases));
                train_scores.push(cur_score.clone());
                continue;
            }
            cur_suite = Some(sr);
        }
        candidates[k].taken = true;
        history.push(format!("round {round}: took \"{}\": cases {:+.2} -> {:+.2}", candidates[k].rationale, cur_score.cases, s.cases));
        current = candidates[k].wording.clone();
        cur_run = run;
        cur_score = s;
        train_scores.push(cur_score.clone());
    }
    // The confidence cut-off, on the training answers of the final wording.
    let fig = figures.get("decide.jev.min_confidence");
    let threshold = cases::best_threshold(&cur_run, current.min_confidence.unwrap_or(fig));
    if threshold.1 > threshold.2 + 1e-9 {
        current.min_confidence = Some(threshold.0);
        progress(&format!("confidence cut-off {:.2} (training score {:+.2}, against {:+.2} at the old one)", threshold.0, threshold.1, threshold.2));
    }
    // What it never saw.
    progress("checking on the held-out cases and scenarios");
    let ho = eval_cases(&original, &held_cases, &held_items)?;
    let ht = eval_cases(&current, &held_cases, &held_items)?;
    cost += ho.cost_usd + ht.cost_usd;
    let (mut held_out_original, mut held_out_tuned) = (score_of(&ho), score_of(&ht));
    if opts.use_suite && !held_entries.is_empty() {
        let so = eval_suite(&original, &held_entries)?;
        let st = eval_suite(&current, &held_entries)?;
        cost += so.cost_usd + st.cost_usd;
        held_out_original = with_suite(held_out_original, &so);
        held_out_tuned = with_suite(held_out_tuned, &st);
    }
    let id = format!("tune-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let run = TuneRun {
        id: id.clone(),
        proposer: opts.proposer.clone(),
        jev_model,
        train_cases: train_cases.iter().map(|c| c.id.clone()).collect(),
        held_out_cases: held_cases.iter().map(|c| c.id.clone()).collect(),
        original,
        tuned: current,
        train_scores,
        candidates,
        threshold,
        held_out_original,
        held_out_tuned,
        cost_usd: cost,
        notes,
    };
    let dir = lab.join("tune").join(&id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("run.json"), serde_json::to_vec_pretty(&run)?)?;
    std::fs::write(dir.join("wording.json"), serde_json::to_vec_pretty(&run.tuned)?)?;
    std::fs::write(dir.join("report.md"), run.report())?;
    Ok(run)
}

/// Field by field, what a wording changes.
pub fn diff(a: &Wording, b: &Wording) -> String {
    let mut out = String::new();
    let (ta, tb) = (a.texts(), b.texts());
    let keys: BTreeSet<String> = ta.iter().chain(&tb).map(|(k, _)| k.clone()).collect();
    for k in keys {
        let x = ta.iter().find(|(kk, _)| *kk == k).map(|(_, v)| *v);
        let y = tb.iter().find(|(kk, _)| *kk == k).map(|(_, v)| *v);
        if x != y {
            out.push_str(&format!("{k}:\n  - {}\n  + {}\n", x.unwrap_or("(none)"), y.unwrap_or("(none)")));
        }
    }
    if a.min_confidence != b.min_confidence {
        out.push_str(&format!("minConfidence: {:?} -> {:?}\n", a.min_confidence, b.min_confidence));
    }
    if a.point_confidence != b.point_confidence {
        out.push_str(&format!("pointConfidence: {:?} -> {:?}\n", a.point_confidence, b.point_confidence));
    }
    if out.is_empty() {
        out.push_str("(no change)\n");
    }
    out
}

impl TuneRun {
    pub fn report(&self) -> String {
        let s = |x: &Score| {
            format!(
                "cases {:+.2} ({} pass, {} wrong of {}; consistency {}/{}; trends {}/{}){}",
                x.cases,
                x.pass,
                x.wrong,
                x.judged,
                x.invariant,
                x.invariance_checks,
                x.trends,
                x.trend_checks,
                x.suite.map(|v| format!("; games {v:+.2} ±{:.2}", x.suite_se.unwrap_or(0.0))).unwrap_or_default()
            )
        };
        let mut out = format!("# Jev wording tuning {}\n\nProposer {}, Jev {}. {} training cases, {} held out. Cost ${:.3}.\n\n## Training\n\n", self.id, self.proposer, self.jev_model, self.train_cases.len(), self.held_out_cases.len(), self.cost_usd);
        for (k, x) in self.train_scores.iter().enumerate() {
            out.push_str(&format!("- {}: {}\n", if k == 0 { "start".to_string() } else { format!("after round {k}") }, s(x)));
        }
        out.push_str(&format!("\nConfidence cut-off: {:.2} (training {:+.2}, against {:+.2} at the old one).\n\n## Rewrites tried\n\n", self.threshold.0, self.threshold.1, self.threshold.2));
        for c in &self.candidates {
            out.push_str(&format!(
                "- round {}{}: {}{}{}\n",
                c.round,
                if c.taken { " (taken)" } else { "" },
                c.rationale,
                c.score.as_ref().map(|x| format!(" — {}", s(x))).unwrap_or_default(),
                c.rejected.as_ref().map(|r| format!(" — not taken: {r}")).unwrap_or_default()
            ));
        }
        out.push_str(&format!("\n## What it never saw (held out)\n\n- original: {}\n- tuned: {}\n\n", s(&self.held_out_original), s(&self.held_out_tuned)));
        let better = self.held_out_tuned.cases >= self.held_out_original.cases && self.held_out_tuned.suite.unwrap_or(0.0) >= self.held_out_original.suite.unwrap_or(0.0) - self.held_out_original.suite_se.unwrap_or(0.0);
        out.push_str(if better { "The tuned wording holds up on what it never saw.\n" } else { "The tuned wording does not hold up on what it never saw: do not accept it as it is.\n" });
        out.push_str(&format!("\n## The change\n\n```\n{}```\n\nTo use it: `bgws lab tune --accept {}` (writes data/jev/wording.json).\n", diff(&self.original, &self.tuned), self.id));
        for n in &self.notes {
            out.push_str(&format!("\nNote: {n}\n"));
        }
        out
    }
}

/// Accept a run's wording: write it to `data/jev/wording.json`. Returns the diff.
pub fn accept(lab: &Path, id: &str) -> Result<(PathBuf, String)> {
    if id.contains('/') || id.contains("..") {
        return Err(anyhow!("bad run id {id}"));
    }
    let src = lab.join("tune").join(id).join("wording.json");
    let w: Wording = serde_json::from_slice(&std::fs::read(&src).with_context(|| src.display().to_string())?)?;
    let d = diff(&crate::jev_wording(), &w);
    let dst = crate::data_dir().join("jev").join("wording.json");
    std::fs::create_dir_all(dst.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&dst, serde_json::to_vec_pretty(&w)?)?;
    Ok((dst, d))
}

/// Every tuning run kept, newest first.
pub fn runs(lab: &Path) -> Vec<TuneRun> {
    let mut v: Vec<TuneRun> = std::fs::read_dir(lab.join("tune")).into_iter().flatten().filter_map(|e| e.ok()).filter_map(|e| std::fs::read(e.path().join("run.json")).ok()).filter_map(|b| serde_json::from_slice(&b).ok()).collect();
    v.sort_by(|a, b| b.id.cmp(&a.id));
    v
}
