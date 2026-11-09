//! The decision lab: how good are the deciders' choices?
//!
//! - **Moments:** every land unit's decision in a game (a question with more
//!   than one option), kept with the game's record so it can be replayed to.
//!   `collect` plays games and keeps their moments; `import` takes a saved
//!   game's.
//! - **Play-outs:** for each option of a moment, the game is replayed to it,
//!   that option is forced, and the game plays on for a while (the rules
//!   deciding from there, with fresh dice each run). Each run is scored from
//!   the deciding unit's side: the enemy's losses, its own side's and its
//!   own, how much nearer its objective it got, and the game's result if it
//!   ended. The option with the best mean is the measured best; options
//!   within the noise of it count as near-best.
//! - **Labels:** a person's view: the best option, the acceptable ones, a
//!   flag and a note.
//! - **Grading:** a decider (Jev, Claude, a local model, the rules) is asked
//!   each moment's question afresh; the report says how often its choice is
//!   near-best and agrees with the labels, and whether its confidence is
//!   honest, per decision point.
//!
//! Files, under the lab folder (`$BGWS_LAB`, else `./lab`): `games/<id>.json`
//! (game records) and `moments/<id>.json`.

use crate::host::{DeciderChoice, Host, HostOptions};
use anyhow::{anyhow, Context, Result};
use bgws_core::{Answer, AnswerSource, Figures, Question, QuestionKind, Seed, Side, SimTime};
use bgws_decide::Decider;
use bgws_scenario::record::GameRecord;
use bgws_scenario::v4::ScenarioV4;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The lab folder: `$BGWS_LAB`, else `lab` in the working directory.
pub fn lab_dir() -> PathBuf {
    std::env::var_os("BGWS_LAB").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("lab"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub id: String,
    /// The game record it is in (`games/<game>.json`).
    pub game: String,
    pub scenario: String,
    pub at_s: f64,
    pub agent: String,
    pub side: Side,
    pub trigger: String,
    pub title: String,
    /// The question as it was put (with the unit's state).
    pub question: Question,
    /// What was chosen in the game, and by what.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<Chosen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playouts: Option<Playouts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    /// Deciders' answers when graded, by variant name.
    #[serde(default)]
    pub graded: BTreeMap<String, Graded>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chosen {
    pub choice: String,
    pub source: String,
}

/// How a run is scored (from the deciding unit's side).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weights {
    /// Per share of the enemy's vehicles lost.
    pub enemy_loss: f64,
    /// Per share of its own side's vehicles lost.
    pub side_loss: f64,
    /// Per share of its own unit's vehicles lost (on top of the side's).
    pub unit_loss: f64,
    /// Per share of the way to its objective made good (−1 to 1).
    pub progress: f64,
    /// A win (+1) or loss (−1) within the horizon.
    pub outcome: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Weights { enemy_loss: 1.0, side_loss: 1.0, unit_loss: 0.5, progress: 0.25, outcome: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playouts {
    pub runs: usize,
    pub horizon_s: f64,
    pub weights: Weights,
    pub options: Vec<OptionResult>,
    pub best: String,
    /// Options whose mean is within the noise of the best (the best included).
    pub near_best: Vec<String>,
    /// Some option is clearly worse than the best: the moment tells good
    /// choices from bad. Only these count toward a decider's near-best share.
    #[serde(default)]
    pub decisive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionResult {
    pub option: String,
    pub summary: String,
    /// Mean score and its standard error.
    pub mean: f64,
    pub se: f64,
    /// Mean of each part of the score.
    pub enemy_loss: f64,
    pub side_loss: f64,
    pub unit_loss: f64,
    pub progress: f64,
    /// Share of runs in which the game ended.
    pub ended: f64,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    #[serde(default)]
    pub best: Option<String>,
    #[serde(default)]
    pub acceptable: Vec<String>,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Graded {
    pub choice: String,
    #[serde(default)]
    pub probabilities: BTreeMap<String, f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    /// jev, cache, rules, fallback (with why)…
    pub source: String,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

/// The lab on disk.
pub struct Lab {
    pub dir: PathBuf,
}

fn slug(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.chars().take(40).collect::<String>().trim_matches('-').to_string()
}

fn source_word(s: &AnswerSource) -> String {
    match s {
        AnswerSource::Fallback { reason } => format!("fallback: {reason}"),
        other => serde_json::to_value(other).ok().and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned)).unwrap_or_default(),
    }
}

impl Lab {
    pub fn open(dir: &Path) -> Result<Lab> {
        std::fs::create_dir_all(dir.join("games"))?;
        std::fs::create_dir_all(dir.join("moments"))?;
        Ok(Lab { dir: dir.to_path_buf() })
    }

    fn moment_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err(anyhow!("bad moment id {id}"));
        }
        Ok(self.dir.join("moments").join(format!("{id}.json")))
    }

    pub fn moment(&self, id: &str) -> Result<Moment> {
        let p = self.moment_path(id)?;
        serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?).with_context(|| format!("reading {}", p.display()))
    }

    pub fn save(&self, m: &Moment) -> Result<()> {
        let p = self.moment_path(&m.id)?;
        let part = p.with_extension("json.part");
        std::fs::write(&part, serde_json::to_vec_pretty(m)?)?;
        std::fs::rename(part, p)?;
        Ok(())
    }

    /// Every moment, by game and time.
    pub fn moments(&self) -> Result<Vec<Moment>> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(self.dir.join("moments"))? {
            let p = e?.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(m) = serde_json::from_slice::<Moment>(&std::fs::read(&p)?) {
                    v.push(m);
                }
            }
        }
        v.sort_by(|a, b| a.game.cmp(&b.game).then(a.at_s.total_cmp(&b.at_s)).then(a.question.id.cmp(&b.question.id)));
        Ok(v)
    }

    pub fn game(&self, id: &str) -> Result<GameRecord> {
        let p = self.dir.join("games").join(format!("{id}.json"));
        GameRecord::load(&p).map_err(|e| anyhow!("{}: {e}", p.display()))
    }

    /// Keep a played game and its units' decisions. Returns the moments' ids.
    pub fn keep(&self, game: &str, record: &GameRecord, asked: &[Question]) -> Result<Vec<String>> {
        record.save(&self.dir.join("games").join(format!("{game}.json"))).map_err(|e| anyhow!("{e}"))?;
        let mut ids = Vec::new();
        for q in asked.iter().filter(|q| q.kind == QuestionKind::Unit && q.options.len() > 1) {
            let id = format!("{game}-q{}", q.id.0);
            let chosen = record.answers.iter().find(|a| a.answer.question == q.id).map(|a| Chosen { choice: a.answer.choice.clone(), source: source_word(&a.answer.source) });
            // Keep what a person has added if the moment is collected again.
            let old = self.moment(&id).ok();
            let m = Moment {
                id: id.clone(),
                game: game.to_string(),
                scenario: record.scenario.name.clone(),
                at_s: q.at.secs(),
                agent: q.agent.clone(),
                side: q.side,
                trigger: q.trigger.clone(),
                title: q.title.clone(),
                question: q.clone(),
                chosen,
                playouts: old.as_ref().and_then(|o| o.playouts.clone()),
                label: old.as_ref().and_then(|o| o.label.clone()),
                graded: old.map(|o| o.graded).unwrap_or_default(),
            };
            self.save(&m)?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Play a scenario with these deciders and keep its moments.
    pub fn collect(&self, scenario: &ScenarioV4, seed: u64, deciders: DeciderChoice, minutes: f64, figures: Figures) -> Result<Vec<String>> {
        let mut h = Host::new(scenario, Seed(seed), figures, &HostOptions { deciders, ..Default::default() })?;
        h.keep_questions = true;
        h.play_until(SimTime::from_secs(minutes * 60.0), |_| {})?;
        let record = h.finished_record();
        let game = format!("{}-s{seed}-{}", slug(&scenario.name), format!("{deciders:?}").to_lowercase());
        self.keep(&game, &record, &h.asked)
    }

    /// Take a saved game's moments (replayed, so its questions are as they were put).
    pub fn import(&self, record: GameRecord, figures: Figures) -> Result<Vec<String>> {
        let end = record.ended_at.ok_or_else(|| anyhow!("the record has no end time"))?;
        let game = format!("{}-s{}-{}", slug(&record.scenario.name), record.seed.0, record.fingerprint.map(|f| f.to_string()).unwrap_or_default().chars().take(8).collect::<String>());
        let mut h = Host::replay(record.clone(), figures)?;
        h.keep_questions = true;
        h.play_until(end, |_| {})?;
        self.keep(&game, &record, &h.asked)
    }

    /// Play out every option of a moment `runs` times for `horizon_s`.
    pub fn playout(&self, id: &str, runs: usize, horizon_s: f64, weights: Weights, figures: &Figures) -> Result<Moment> {
        let mut m = self.moment(id)?;
        let record = self.game(&m.game)?;
        // Ground loaded once for every run.
        let mut sim = bgws_sim::SimOptions::default();
        if let Some(g) = crate::host::load_pack(&record.scenario)? {
            sim.terrain = Some(g.terrain);
            sim.roads = Some(g.roads);
            sim.rivers = Some(g.rivers);
            sim.sight = g.sight;
            sim.appreciation = g.appreciation;
        }
        let jobs: Vec<(usize, usize)> = (0..m.question.options.len()).flat_map(|o| (0..runs).map(move |k| (o, k))).collect();
        let q = m.question.clone();
        let results: Vec<(usize, Result<Score>)> = jobs.par_iter().map(|&(o, k)| (o, play_once(&record, &q, &q.options[o].id, k as u64, horizon_s, &weights, figures, &sim))).collect();
        let mut options = Vec::new();
        for (o, opt) in q.options.iter().enumerate() {
            let scores: Vec<Score> = results.iter().filter(|r| r.0 == o).map(|r| r.1.as_ref().map(|s| *s).map_err(|e| anyhow!("{e}"))).collect::<Result<_>>()?;
            let n = scores.len().max(1) as f64;
            let values: Vec<f64> = scores.iter().map(|s| s.value).collect();
            let mean = values.iter().sum::<f64>() / n;
            let var = if values.len() > 1 { values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0) } else { 0.0 };
            let avg = |f: fn(&Score) -> f64| scores.iter().map(f).sum::<f64>() / n;
            options.push(OptionResult {
                option: opt.id.clone(),
                summary: opt.summary.clone(),
                mean,
                se: (var / n).sqrt(),
                enemy_loss: avg(|s| s.enemy_loss),
                side_loss: avg(|s| s.side_loss),
                unit_loss: avg(|s| s.unit_loss),
                progress: avg(|s| s.progress),
                ended: avg(|s| if s.ended { 1.0 } else { 0.0 }),
                values,
            });
        }
        let best = options.iter().max_by(|a, b| a.mean.total_cmp(&b.mean)).ok_or_else(|| anyhow!("no options"))?.clone();
        let near_best: Vec<String> = options.iter().filter(|o| best.mean - o.mean <= (best.se.powi(2) + o.se.powi(2)).sqrt().max(1e-9)).map(|o| o.option.clone()).collect();
        let decisive = near_best.len() < options.len();
        m.playouts = Some(Playouts { runs, horizon_s, weights, options, best: best.option, near_best, decisive });
        self.save(&m)?;
        Ok(m)
    }

    /// A person's view of a moment.
    pub fn label(&self, id: &str, label: Label) -> Result<Moment> {
        let mut m = self.moment(id)?;
        for o in label.best.iter().chain(&label.acceptable) {
            if !m.question.has_option(o) {
                return Err(anyhow!("{o} is not an option here"));
            }
        }
        m.label = Some(label);
        self.save(&m)?;
        Ok(m)
    }

    /// Ask a decider every moment's question afresh (one at a time, as it
    /// was put) and keep its answers under `variant`.
    pub fn grade(&self, decider: Arc<dyn Decider>, variant: &str, filter: impl Fn(&Moment) -> bool) -> Result<usize> {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        let mut n = 0;
        for mut m in self.moments()?.into_iter().filter(|m| filter(m)) {
            let a: Answer = rt.block_on(decider.decide(m.side, vec![m.question.clone()])).into_iter().next().ok_or_else(|| anyhow!("no answer"))?;
            m.graded.insert(variant.to_string(), Graded { choice: a.choice, probabilities: a.probabilities.unwrap_or_default(), confidence: a.confidence, source: source_word(&a.source), cost_usd: a.cost_usd });
            self.save(&m)?;
            n += 1;
        }
        Ok(n)
    }

    /// How each decider does, overall and per decision point, against the
    /// play-outs and the labels; with the template's fallback, the rules'
    /// choice and what the game chose as yardsticks.
    pub fn report(&self) -> Result<Report> {
        let moments = self.moments()?;
        let mut rows: BTreeMap<(String, String), Tally> = BTreeMap::new();
        for m in &moments {
            let mut picks: Vec<(String, String, Option<f64>)> = vec![
                ("template fallback".into(), m.question.fallback.clone(), None),
                ("rules".into(), m.question.rules_choice.clone(), None),
            ];
            if let Some(c) = &m.chosen {
                picks.push(("as played".into(), c.choice.clone(), None));
            }
            for (v, g) in &m.graded {
                picks.push((v.clone(), g.choice.clone(), g.confidence.filter(|_| !g.source.starts_with("fallback"))));
            }
            for (who, choice, conf) in picks {
                for point in [m.trigger.clone(), "all".to_string()] {
                    let t = rows.entry((who.clone(), point)).or_default();
                    t.asked += 1;
                    if let Some(p) = m.playouts.as_ref().filter(|p| p.decisive) {
                        t.played += 1;
                        let hit = p.near_best.contains(&choice);
                        t.near_best += hit as usize;
                        t.best += (p.best == choice) as usize;
                        // Regret: how far its option's mean is below the best's.
                        let best = p.options.iter().map(|o| o.mean).fold(f64::NEG_INFINITY, f64::max);
                        t.regret += p.options.iter().find(|o| o.option == choice).map(|o| best - o.mean).unwrap_or(0.0);
                        if let Some(c) = conf {
                            t.calib.push((c, hit));
                        }
                    }
                    if let Some(l) = m.label.as_ref().filter(|l| l.best.is_some() || !l.acceptable.is_empty()) {
                        t.labelled += 1;
                        t.agrees += (l.best.as_deref() == Some(choice.as_str()) || l.acceptable.contains(&choice)) as usize;
                    }
                }
            }
        }
        let rows = rows
            .into_iter()
            .map(|((who, point), t)| {
                let share = |a: usize, b: usize| if b > 0 { Some(a as f64 / b as f64) } else { None };
                // Calibration: mean confidence against hit rate, and the expected calibration error over five bins.
                let (ece, conf) = if t.calib.is_empty() {
                    (None, None)
                } else {
                    let n = t.calib.len() as f64;
                    let mut e = 0.0;
                    for b in 0..5 {
                        let in_bin: Vec<&(f64, bool)> = t.calib.iter().filter(|(c, _)| ((c * 5.0).floor() as usize).min(4) == b).collect();
                        if !in_bin.is_empty() {
                            let k = in_bin.len() as f64;
                            let c = in_bin.iter().map(|x| x.0).sum::<f64>() / k;
                            let h = in_bin.iter().filter(|x| x.1).count() as f64 / k;
                            e += k / n * (c - h).abs();
                        }
                    }
                    (Some(e), Some(t.calib.iter().map(|x| x.0).sum::<f64>() / n))
                };
                ReportRow {
                    decider: who,
                    point,
                    moments: t.asked,
                    played_out: t.played,
                    near_best: share(t.near_best, t.played),
                    best: share(t.best, t.played),
                    mean_regret: if t.played > 0 { Some(t.regret / t.played as f64) } else { None },
                    labelled: t.labelled,
                    agrees_with_labels: share(t.agrees, t.labelled),
                    mean_confidence: conf,
                    calibration_error: ece,
                }
            })
            .collect();
        Ok(Report {
            moments: moments.len(),
            played_out: moments.iter().filter(|m| m.playouts.is_some()).count(),
            decisive: moments.iter().filter(|m| m.playouts.as_ref().is_some_and(|p| p.decisive)).count(),
            labelled: moments.iter().filter(|m| m.label.is_some()).count(),
            flagged: moments.iter().filter(|m| m.label.as_ref().is_some_and(|l| l.flagged)).count(),
            rows,
        })
    }
}

#[derive(Default)]
struct Tally {
    asked: usize,
    played: usize,
    near_best: usize,
    best: usize,
    regret: f64,
    labelled: usize,
    agrees: usize,
    calib: Vec<(f64, bool)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub moments: usize,
    pub played_out: usize,
    /// Played out, and some option clearly worse than the best.
    pub decisive: usize,
    pub labelled: usize,
    pub flagged: usize,
    pub rows: Vec<ReportRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRow {
    pub decider: String,
    /// A decision point (D1…) or `all`.
    pub point: String,
    pub moments: usize,
    /// Decisive played-out moments it is scored on.
    pub played_out: usize,
    /// Share of decisive played-out moments where its choice was near-best.
    pub near_best: Option<f64>,
    /// Share where it was the single best.
    pub best: Option<f64>,
    /// Mean score lost against the best option.
    pub mean_regret: Option<f64>,
    pub labelled: usize,
    pub agrees_with_labels: Option<f64>,
    pub mean_confidence: Option<f64>,
    /// Expected calibration error of its confidence against being near-best (0 is honest).
    pub calibration_error: Option<f64>,
}

impl Report {
    /// The overall rows as a plain table.
    pub fn table(&self) -> String {
        let pct = |x: Option<f64>| x.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "–".into());
        let mut out = format!("{} moments: {} played out ({} decisive: some option clearly worse), {} labelled, {} flagged\n\n", self.moments, self.played_out, self.decisive, self.labelled, self.flagged);
        out.push_str("| decider | moments | near-best | best | regret | agrees with you | confidence | calibration error |\n|---|---|---|---|---|---|---|---|\n");
        for r in self.rows.iter().filter(|r| r.point == "all") {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.decider,
                r.moments,
                pct(r.near_best),
                pct(r.best),
                r.mean_regret.map(|v| format!("{v:.3}")).unwrap_or_else(|| "–".into()),
                pct(r.agrees_with_labels),
                pct(r.mean_confidence),
                r.calibration_error.map(|v| format!("{v:.2}")).unwrap_or_else(|| "–".into()),
            ));
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
struct Score {
    value: f64,
    enemy_loss: f64,
    side_loss: f64,
    unit_loss: f64,
    progress: f64,
    ended: bool,
}

/// Replay to the moment, force the option, reseed (run `k`), play on, score.
#[allow(clippy::too_many_arguments)]
fn play_once(record: &GameRecord, q: &Question, option: &str, k: u64, horizon_s: f64, w: &Weights, figures: &Figures, sim: &bgws_sim::SimOptions) -> Result<Score> {
    let mut h = Host::replay_with(GameRecord { ended_at: None, fingerprint: None, outcome: None, ..record.clone() }, figures.clone(), sim.clone())?;
    h.force_answer(q.id, option, Some(k));
    // To the moment: the question comes up at its time.
    let mut t = q.at;
    while !h.forced() {
        t = SimTime::from_secs(t.secs() + 1.0);
        if h.play_until(t, |_| {})?.is_some() || t.secs() > q.at.secs() + 600.0 {
            return Err(anyhow!("replay never reached question {} ({} {} at {:.0} s)", q.id.0, q.agent, q.trigger, q.at.secs()));
        }
    }
    let s = &h.sim;
    let (side, enemy) = (q.side, q.side.enemy());
    let unit = |s: &bgws_sim::Sim| s.entity_named(&q.agent).and_then(|e| s.world.get::<bgws_sim::components::Health>(e).map(|h| (h.fit, h.start)));
    let pos = |s: &bgws_sim::Sim| s.entity_named(&q.agent).and_then(|e| s.world.get::<bgws_sim::components::Kin>(e).map(|k| k.pos));
    let (side0, enemy0, unit0, goal, at0) = (s.side_fit(side), s.side_fit(enemy), unit(s), s.goal_of(&q.agent), pos(s));
    let over = h.play_until(SimTime::from_secs(h.sim.now().secs() + horizon_s), |_| {})?;
    let s = &h.sim;
    let frac = |a: u32, b: u32| if a > 0 { (a.saturating_sub(b)) as f64 / a as f64 } else { 0.0 };
    let enemy_loss = frac(enemy0, s.side_fit(enemy));
    let side_loss = frac(side0, s.side_fit(side));
    let unit_loss = match (unit0, unit(s)) {
        (Some((f0, _)), Some((f1, _))) => frac(f0, f1),
        (Some(_), None) => 1.0,
        _ => 0.0,
    };
    let progress = match (goal, at0, pos(s)) {
        (Some(g), Some(a), Some(b)) => {
            let d0 = g.sub(a).horizontal();
            ((d0 - g.sub(b).horizontal()) / d0.max(500.0)).clamp(-1.0, 1.0)
        }
        _ => 0.0,
    };
    let outcome = match over.as_ref().and_then(|o| o.winner) {
        Some(x) if x == side => 1.0,
        Some(_) => -1.0,
        None => 0.0,
    };
    let value = w.enemy_loss * enemy_loss - w.side_loss * side_loss - w.unit_loss * unit_loss + w.progress * progress + w.outcome * outcome;
    Ok(Score { value, enemy_loss, side_loss, unit_loss, progress, ended: over.is_some() })
}

/// A decider to grade, and the name its answers are kept under: the kind,
/// the model, and `+prose` when Jev is also given the words.
pub fn decider_for(choice: DeciderChoice, figures: &Figures, model: Option<String>, prose: bool, local: (&str, &str)) -> Result<(Arc<dyn Decider>, String)> {
    use bgws_decide::{ClaudeConfig, ClaudeDecider, JevClient, JevConfig, LocalConfig, LocalDecider, MessagesClient, RulesDecider};
    Ok(match choice {
        DeciderChoice::Jev => {
            let mut cfg = JevConfig::from_figures(figures);
            if let Some(m) = model {
                cfg.model = m;
            }
            cfg.prose = prose;
            let name = format!("{}{}", cfg.model, if prose { "+prose" } else { "" });
            (Arc::new(JevClient::from_env(cfg)), name)
        }
        DeciderChoice::Claude => {
            let mut cfg = ClaudeConfig::from_figures(figures);
            if let Some(m) = model {
                cfg.model = m;
            }
            let name = cfg.model.clone();
            (Arc::new(ClaudeDecider::new(MessagesClient::from_env(cfg))), name)
        }
        DeciderChoice::Local => {
            let cfg = LocalConfig::from_figures(figures, local.0, local.1);
            (Arc::new(LocalDecider::new(cfg)), format!("local:{}", local.1))
        }
        DeciderChoice::Rules | DeciderChoice::Scenario => (Arc::new(RulesDecider), "rules (asked)".into()),
    })
}
