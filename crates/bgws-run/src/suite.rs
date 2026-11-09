//! The engagement suite: does a decider win more and lose less than the
//! doctrine default, in played games?
//!
//! Each entry is a scenario played to a time limit over several seeds. For
//! every scenario and seed, one baseline game is played with the rules
//! deciding for both sides; then, for each side, a game in which the decider
//! under test answers that side's units (the rules still answer the other
//! side, and the HQs plan as they always do). The two games share the seed,
//! so they start identically and differ only by the decider's choices: the
//! difference between them is what the decider adds, with much of the luck
//! cancelled ("paired" games).
//!
//! Measured for the side the decider played, in both games:
//! - **result:** +1 a win, −1 a loss, 0 neither by the time limit;
//! - **own and enemy losses**, as a share of each side's vehicles and teams;
//! - **exchange:** enemy losses less own losses (shares; above 0 is trading well);
//! - **survival:** the mean share of the game its vehicles lived.
//!
//! The report gives each metric's mean difference (decider game minus
//! baseline) and its standard error, per scenario and over the suite.

use crate::host::{DeciderChoice, Host, HostOptions};
use anyhow::{anyhow, Result};
use bgws_core::{Figures, QuestionKind, Seed, Side, SimTime};
use bgws_decide::{Decider, Route};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub scenario: String,
    /// The time limit, minutes (else the scenario's length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<f64>,
    /// What it tests, in a few words.
    #[serde(default)]
    pub note: String,
    /// Kept back from tuning: used only to check a tuned setting.
    #[serde(default)]
    pub held_out: bool,
}

/// `data/suite/suite.json`.
pub fn load_entries() -> Result<Vec<Entry>> {
    let p = crate::data_dir().join("suite").join("suite.json");
    serde_json::from_slice(&std::fs::read(&p).map_err(|e| anyhow!("{}: {e}", p.display()))?).map_err(|e| anyhow!("{}: {e}", p.display()))
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub result: f64,
    pub own_loss: f64,
    pub enemy_loss: f64,
    pub exchange: f64,
    pub survival: f64,
}

impl Metrics {
    fn minus(self, o: Metrics) -> Metrics {
        Metrics { result: self.result - o.result, own_loss: self.own_loss - o.own_loss, enemy_loss: self.enemy_loss - o.enemy_loss, exchange: self.exchange - o.exchange, survival: self.survival - o.survival }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub scenario: String,
    pub seed: u64,
    pub side: Side,
    pub decider: Metrics,
    pub baseline: Metrics,
    pub diff: Metrics,
    /// The decider's game: questions its side's units were asked, and how they were answered (jev, fallback, cache…).
    pub answered_by: BTreeMap<String, usize>,
    pub cost_usd: f64,
    pub ended_s: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub mean: f64,
    pub se: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// A scenario, or `all`.
    pub scope: String,
    pub pairs: usize,
    /// Mean of each metric in the decider's games and in the baselines.
    pub decider: Metrics,
    pub baseline: Metrics,
    /// Mean difference (decider − baseline) and its standard error.
    pub result: Stat,
    pub own_loss: Stat,
    pub enemy_loss: Stat,
    pub exchange: Stat,
    pub survival: Stat,
    /// Pairs the decider did better in, worse in (by result, then exchange).
    pub better: usize,
    pub worse: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiteRun {
    pub decider: String,
    pub seeds: Vec<u64>,
    pub pairs: Vec<Pair>,
    pub summaries: Vec<Summary>,
    pub cost_usd: f64,
    pub wall_s: f64,
    /// Share of the decider's questions answered by it (not its fallback); none for the rules.
    pub answered_share: Option<f64>,
    /// Games kept in the lab (the decider's worst), for their decisions to be played out.
    #[serde(default)]
    pub kept: Vec<String>,
}

/// What to do besides measuring.
#[derive(Clone, Default)]
pub struct Options {
    /// Keep the decider's worst games in this lab (their moments, to play out).
    pub keep_worst: Option<(PathBuf, usize)>,
}

/// Play a game to `minutes` with `router` and measure it for `side`.
type Played = (Metrics, BTreeMap<String, usize>, f64, f64, bgws_scenario::record::GameRecord);

fn play(sc: &bgws_scenario::v4::ScenarioV4, seed: u64, minutes: f64, figures: &Figures, side: Side, decider: Option<&Arc<dyn Decider>>) -> Result<Played> {
    let mut h = Host::new(sc, Seed(seed), figures.clone(), &HostOptions { deciders: DeciderChoice::Rules, ..Default::default() })?;
    if let Some(d) = decider {
        h.router.set_decider(side, QuestionKind::Unit, Route::Jev, d.clone());
        h.router.set_decider(side, QuestionKind::Entity, Route::Jev, d.clone());
    }
    let start = |h: &Host, s: Side| h.sim.side_fit(s);
    let (own0, enemy0) = (start(&h, side), start(&h, side.enemy()));
    // Every vehicle of the side, for survival.
    let vehicles0: u32 = own0;
    let limit = SimTime::from_secs(minutes * 60.0);
    let over = h.play_until(limit, |_| {})?;
    let end = h.sim.now().secs().max(1.0);
    let frac = |a: u32, b: u32| if a > 0 { a.saturating_sub(b) as f64 / a as f64 } else { 0.0 };
    let own_loss = frac(own0, h.sim.side_fit(side));
    let enemy_loss = frac(enemy0, h.sim.side_fit(side.enemy()));
    // Survival: each vehicle lived to its loss, or to the end.
    let mut lived = 0.0;
    let mut lost = 0u32;
    for &e in &h.sim.ctx.order {
        let (Some(i), Some(hh)) = (h.sim.world.get::<bgws_sim::components::Ident>(e), h.sim.world.get::<bgws_sim::components::Health>(e)) else { continue };
        if i.side != side {
            continue;
        }
        for t in &hh.losses {
            lived += t.secs() / end;
            lost += 1;
        }
    }
    let survival = if vehicles0 > 0 { (lived + vehicles0.saturating_sub(lost) as f64) / vehicles0 as f64 } else { 1.0 };
    let result = match over.as_ref().and_then(|o| o.winner) {
        Some(w) if w == side => 1.0,
        Some(_) => -1.0,
        None => 0.0,
    };
    let rep = h.report();
    Ok((Metrics { result, own_loss, enemy_loss, exchange: enemy_loss - own_loss, survival }, rep.answers_by_source, rep.cost_usd, end, h.finished_record()))
}

fn stat(v: &[f64]) -> Stat {
    let n = v.len().max(1) as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = if v.len() > 1 { v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0) } else { 0.0 };
    Stat { mean, se: (var / n).sqrt() }
}

fn summarise(scope: &str, ps: &[&Pair]) -> Summary {
    let col = |f: fn(&Metrics) -> f64, which: fn(&Pair) -> Metrics| ps.iter().map(|p| f(&which(p))).collect::<Vec<f64>>();
    let mean_of = |which: fn(&Pair) -> Metrics| {
        let n = ps.len().max(1) as f64;
        let s = |f: fn(&Metrics) -> f64| ps.iter().map(|p| f(&which(p))).sum::<f64>() / n;
        Metrics { result: s(|m| m.result), own_loss: s(|m| m.own_loss), enemy_loss: s(|m| m.enemy_loss), exchange: s(|m| m.exchange), survival: s(|m| m.survival) }
    };
    let d = |p: &Pair| p.diff;
    let better = ps.iter().filter(|p| p.diff.result > 0.0 || (p.diff.result == 0.0 && p.diff.exchange > 0.01)).count();
    let worse = ps.iter().filter(|p| p.diff.result < 0.0 || (p.diff.result == 0.0 && p.diff.exchange < -0.01)).count();
    Summary {
        scope: scope.to_string(),
        pairs: ps.len(),
        decider: mean_of(|p| p.decider),
        baseline: mean_of(|p| p.baseline),
        result: stat(&col(|m| m.result, d)),
        own_loss: stat(&col(|m| m.own_loss, d)),
        enemy_loss: stat(&col(|m| m.enemy_loss, d)),
        exchange: stat(&col(|m| m.exchange, d)),
        survival: stat(&col(|m| m.survival, d)),
        better,
        worse,
    }
}

/// A baseline's measures, by scenario index, seed and side.
type BaseKey = (usize, u64, Side);

/// Run the suite: every entry, every seed, the decider on each side, paired
/// with the rules baseline. `parallel` games at a time.
#[allow(clippy::too_many_arguments)]
pub fn run(entries: &[Entry], seeds: &[u64], decider: Arc<dyn Decider>, name: &str, figures: &Figures, parallel: usize, opts: &Options, progress: &(dyn Fn(&str) + Sync)) -> Result<SuiteRun> {
    let t0 = std::time::Instant::now();
    let scenarios: Vec<(Entry, bgws_scenario::v4::ScenarioV4)> = entries
        .iter()
        .map(|e| crate::load_scenario(&crate::scenario_file(&e.scenario)).map(|s| (e.clone(), s)))
        .collect::<Result<_>>()?;
    let jobs: Vec<(usize, u64, Side)> = (0..scenarios.len()).flat_map(|i| seeds.iter().flat_map(move |&k| [Side::Blue, Side::Red].map(|s| (i, k, s)))).collect();
    // Baselines: one rules game per scenario and seed, measured for each side.
    let base: BTreeMap<BaseKey, Metrics> = (0..scenarios.len())
        .into_par_iter()
        .flat_map(|i| seeds.par_iter().map(move |&k| (i, k)))
        .map(|(i, k)| -> Result<Vec<(BaseKey, Metrics)>> {
            let (e, sc) = &scenarios[i];
            let m = e.minutes.unwrap_or(sc.scope.duration_min);
            [Side::Blue, Side::Red].into_iter().map(|s| play(sc, k, m, figures, s, None).map(|r| ((i, k, s), r.0))).collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(parallel.max(1)).build()?;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let played: Vec<(Pair, bgws_scenario::record::GameRecord)> = pool.install(|| {
        jobs.par_iter()
            .map(|&(i, k, side)| -> Result<(Pair, bgws_scenario::record::GameRecord)> {
                let (e, sc) = &scenarios[i];
                let m = e.minutes.unwrap_or(sc.scope.duration_min);
                let (dm, by, cost, ended, record) = play(sc, k, m, figures, side, Some(&decider))?;
                let b = base[&(i, k, side)];
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress(&format!("[{n}/{}] {} seed {k} {side}: result {:+.0} (baseline {:+.0}), exchange {:+.2} ({:+.2})", jobs.len(), e.scenario, dm.result, b.result, dm.exchange, b.exchange));
                Ok((Pair { scenario: e.scenario.clone(), seed: k, side, decider: dm, baseline: b, diff: dm.minus(b), answered_by: by, cost_usd: cost, ended_s: ended }, record))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    // The worst games, kept in the lab: their decisions are moments to play out.
    let mut kept = Vec::new();
    if let Some((dir, n)) = &opts.keep_worst {
        let lab = crate::lab::Lab::open(dir)?;
        let mut order: Vec<&(Pair, bgws_scenario::record::GameRecord)> = played.iter().filter(|(p, _)| p.diff.result < 0.0 || p.diff.exchange < -0.01).collect();
        order.sort_by(|a, b| (a.0.diff.result, a.0.diff.exchange).partial_cmp(&(b.0.diff.result, b.0.diff.exchange)).unwrap_or(std::cmp::Ordering::Equal));
        for (p, rec) in order.into_iter().take(*n) {
            let game = format!("suite-{}-s{}-{}-{}", p.scenario, p.seed, p.side, name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
            let mut h = Host::replay(rec.clone(), figures.clone())?;
            h.keep_questions = true;
            h.play_until(rec.ended_at.unwrap_or(SimTime::ZERO), |_| {})?;
            // Only the decider's side: those are its decisions.
            let asked: Vec<bgws_core::Question> = h.asked.into_iter().filter(|q| q.side == p.side).collect();
            lab.keep(&game, rec, &asked)?;
            kept.push(game);
        }
    }
    let pairs: Vec<Pair> = played.into_iter().map(|x| x.0).collect();
    let mut summaries = Vec::new();
    for (e, _) in &scenarios {
        let ps: Vec<&Pair> = pairs.iter().filter(|p| p.scenario == e.scenario).collect();
        summaries.push(summarise(&e.scenario, &ps));
    }
    summaries.push(summarise("all", &pairs.iter().collect::<Vec<_>>()));
    let asked: usize = pairs.iter().flat_map(|p| p.answered_by.iter()).filter(|(k, _)| !matches!(k.as_str(), "rules" | "person")).map(|(_, v)| v).sum();
    let by_decider: usize = pairs.iter().flat_map(|p| p.answered_by.iter()).filter(|(k, _)| matches!(k.as_str(), "jev" | "claude" | "local" | "cache")).map(|(_, v)| v).sum();
    Ok(SuiteRun {
        decider: name.to_string(),
        seeds: seeds.to_vec(),
        cost_usd: pairs.iter().map(|p| p.cost_usd).sum(),
        wall_s: t0.elapsed().as_secs_f64(),
        answered_share: (asked > 0).then(|| by_decider as f64 / asked as f64),
        kept,
        pairs,
        summaries,
    })
}

impl SuiteRun {
    pub fn table(&self) -> String {
        let s = |x: &Stat| format!("{:+.2}±{:.2}", x.mean, x.se);
        let mut out = format!(
            "{}: {} paired games over seeds {:?}, {:.0} s, ${:.4}{}.\n\n| scenario | pairs | better / worse | result Δ | exchange Δ | own loss Δ | enemy loss Δ | survival Δ |\n|---|---|---|---|---|---|---|---|\n",
            self.decider,
            self.pairs.len(),
            self.seeds,
            self.wall_s,
            self.cost_usd,
            self.answered_share.map(|a| format!("; {:.0}% of its questions answered by it (the rest its fallback)", a * 100.0)).unwrap_or_default()
        );
        for m in &self.summaries {
            out.push_str(&format!("| {} | {} | {} / {} | {} | {} | {} | {} | {} |\n", m.scope, m.pairs, m.better, m.worse, s(&m.result), s(&m.exchange), s(&m.own_loss), s(&m.enemy_loss), s(&m.survival)));
        }
        if !self.kept.is_empty() {
            out.push_str(&format!("\nIts worst games are in the lab, to play out their decisions: {}\n", self.kept.join(", ")));
        }
        out.push_str("\nΔ is the decider's game minus the rules baseline on the same seed, for the side it played: result (+1 win, −1 loss), exchange (enemy share lost − own share lost), losses (shares), survival (mean share of the game its vehicles lived). ± is one standard error.\n");
        out
    }
}

pub fn save(lab: &Path, r: &SuiteRun) -> Result<PathBuf> {
    let dir = lab.join("suite");
    std::fs::create_dir_all(&dir)?;
    let file: String = r.decider.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    let p = dir.join(format!("{file}.json"));
    std::fs::write(&p, serde_json::to_vec_pretty(r)?)?;
    Ok(p)
}

pub fn runs(lab: &Path) -> Vec<SuiteRun> {
    let mut v: Vec<SuiteRun> = std::fs::read_dir(lab.join("suite")).into_iter().flatten().filter_map(|e| e.ok()).filter_map(|e| std::fs::read(e.path()).ok()).filter_map(|b| serde_json::from_slice(&b).ok()).collect();
    v.sort_by(|a, b| a.decider.cmp(&b.decider));
    v
}
