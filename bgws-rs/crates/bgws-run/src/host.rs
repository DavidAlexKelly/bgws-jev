//! The host loop (§3.3). The clock waits in simulated time while the
//! deciders answer, so a slow model never changes the game, only how long
//! it takes to play. Answers are recorded, so a game with Jev and Claude
//! replays exactly without calling them again (§12).

use anyhow::{anyhow, bail, Context, Result};
use bgws_core::{Answer, Figures, LogEvent, Question, QuestionKind, Seed, Side, SimTime};
use bgws_decide::{ClaudeConfig, ClaudeDecider, ClaudePlanner, HeuristicPlanner, JevClient, JevConfig, LocalConfig, LocalDecider, Route, Router};
use bgws_scenario::record::{BuildInfo, GameRecord, MachineInfo};
use bgws_scenario::v4::{Decider, ScenarioV4};
use bgws_sim::{Sim, SimOptions, Step};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// Who answers, overriding the scenario's choice for both sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeciderChoice {
    /// As the scenario says (rules where it says nothing).
    Scenario,
    Rules,
    Jev,
    Claude,
    Local,
}

impl std::str::FromStr for DeciderChoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "scenario" => DeciderChoice::Scenario,
            "rules" => DeciderChoice::Rules,
            "jev" => DeciderChoice::Jev,
            "claude" => DeciderChoice::Claude,
            "local" => DeciderChoice::Local,
            _ => return Err(format!("unknown decider `{s}` (scenario, rules, jev, claude, local)")),
        })
    }
}

#[derive(Clone, Debug)]
pub struct HostOptions {
    pub deciders: DeciderChoice,
    /// Base URL of an OpenAI-compatible local server (for `local`).
    pub local_url: String,
    pub local_model: String,
    /// Claude model for planners (overrides the scenario's).
    pub claude_model: Option<String>,
    pub sim: SimOptions,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions { deciders: DeciderChoice::Scenario, local_url: "http://127.0.0.1:8080".into(), local_model: "local".into(), claude_model: None, sim: SimOptions::default() }
    }
}

/// Read a scenario of any supported version (v3 files are converted).
pub fn load_scenario(path: &Path) -> Result<ScenarioV4> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cat = bgws_catalogue::Catalogue::builtin().map_err(|e| anyhow!("catalogue: {e:?}"))?;
    bgws_scenario::validate::load_any(&text, Some(&cat)).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// The routing table for a scenario: each side's units and HQs to the
/// decider the scenario (or the override) names; keys come from the
/// environment and never leave the host (§10.2).
pub fn router_for(scenario: &ScenarioV4, figures: &Figures, opts: &HostOptions) -> Router {
    let mut r = Router::rules();
    let heuristic = HeuristicPlanner::from_figures(figures);
    r.set_planner(Side::Blue, Route::Rules, Arc::new(heuristic.clone()));
    r.set_planner(Side::Red, Route::Rules, Arc::new(heuristic.clone()));
    let jev = Arc::new(JevClient::from_env(JevConfig::from_figures(figures)));
    let mut ccfg = ClaudeConfig::from_figures(figures);
    if let Some(m) = opts.claude_model.clone().or_else(|| (!scenario.command.model.is_empty()).then(|| scenario.command.model.clone())) {
        ccfg.model = m;
    }
    let claude_planner = Arc::new(ClaudePlanner::from_env(ccfg.clone()));
    let claude_decider = Arc::new(ClaudeDecider::new(bgws_decide::MessagesClient::from_env(ccfg)));
    let local = Arc::new(LocalDecider::new(LocalConfig::from_figures(figures, &opts.local_url, &opts.local_model)));
    for side in Side::PLAYERS {
        let sc = scenario.command.side(side);
        let pick = |d: Decider| match opts.deciders {
            DeciderChoice::Scenario => d,
            DeciderChoice::Rules => Decider::Rules,
            DeciderChoice::Jev => Decider::Jev,
            DeciderChoice::Claude => Decider::Claude,
            DeciderChoice::Local => Decider::Local,
        };
        for kind in [QuestionKind::Unit, QuestionKind::Entity] {
            match pick(sc.units) {
                Decider::Jev => {
                    r.set_decider(side, kind, Route::Jev, jev.clone());
                }
                Decider::Claude => {
                    r.set_decider(side, kind, Route::Claude, claude_decider.clone());
                }
                Decider::Local => {
                    r.set_decider(side, kind, Route::Local, local.clone());
                }
                Decider::Rules | Decider::Person => {}
            }
        }
        if matches!(pick(sc.hqs), Decider::Claude | Decider::Jev) || (opts.deciders == DeciderChoice::Claude) {
            r.set_planner(side, Route::Claude, claude_planner.clone());
        }
    }
    r
}

/// The scenario's theatre pack, if it names one, assembled over the area its
/// positions cover (with a margin): the host does the disk, the sim stays pure.
pub fn load_pack_terrain(scenario: &ScenarioV4) -> Result<Option<bgws_env::Terrain>> {
    let Some(dir) = &scenario.theatre.pack else { return Ok(None) };
    let mut pack = bgws_env::TheatrePack::open(Path::new(dir)).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?;
    let frame = scenario.frame();
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (_, g) in scenario.positions() {
        let v = frame.geodetic_to_enu(g);
        lo = (lo.0.min(v.e), lo.1.min(v.n));
        hi = (hi.0.max(v.e), hi.1.max(v.n));
    }
    let m = 5_000.0;
    let aoi = bgws_env::Aoi { min_e: lo.0 - m, min_n: lo.1 - m, max_e: hi.0 + m, max_n: hi.1 + m };
    Ok(Some(pack.assemble(&aoi).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?))
}

/// What a played game produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayReport {
    pub outcome: Option<bgws_sim::Outcome>,
    pub ended_at: SimTime,
    pub fingerprint: String,
    pub questions: usize,
    pub events: u64,
    pub answers_by_source: std::collections::BTreeMap<String, usize>,
    pub cost_usd: f64,
}

/// A game in progress with its deciders and its record.
pub struct Host {
    pub sim: Sim,
    pub router: Router,
    pub record: GameRecord,
    rt: tokio::runtime::Runtime,
    events_out: Option<std::io::BufWriter<std::fs::File>>,
    /// Answers from a record, for a replay: questions are answered from here only.
    replay: Option<GameRecord>,
    questions: usize,
    by_source: std::collections::BTreeMap<String, usize>,
    cost: f64,
}

impl Host {
    pub fn new(scenario: &ScenarioV4, seed: Seed, figures: Figures, opts: &HostOptions) -> Result<Host> {
        let router = router_for(scenario, &figures, opts);
        let mut sim_opts = opts.sim.clone();
        if sim_opts.terrain.is_none() {
            sim_opts.terrain = load_pack_terrain(scenario)?;
        }
        let sim = Sim::with_options(scenario, seed, figures, sim_opts).map_err(|e| anyhow!("{e}"))?;
        let mut machine = MachineInfo::compiled_for();
        machine.threads = Some(rayon::current_num_threads() as u32);
        let record = GameRecord::new(scenario.clone(), seed, BuildInfo::this(option_env!("BGWS_BUILD_ID").unwrap_or("dev")), machine);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        Ok(Host { sim, router, record, rt, events_out: None, replay: None, questions: 0, by_source: Default::default(), cost: 0.0 })
    }

    /// Replay a recorded game without calling any decider.
    pub fn replay(record: GameRecord, figures: Figures) -> Result<Host> {
        let mut f = figures;
        for (k, v) in &record.figure_overrides {
            f.set(k, *v).map_err(|e| anyhow!("{e}"))?;
        }
        let mut h = Host::new(&record.scenario, record.seed, f, &HostOptions { deciders: DeciderChoice::Rules, ..Default::default() })?;
        h.replay = Some(record);
        Ok(h)
    }

    /// Leave replay mode: from now on the router answers (a "what if" branch).
    pub fn stop_replay(&mut self) {
        self.replay = None;
    }

    pub fn write_events_to(&mut self, path: &Path) -> Result<()> {
        self.events_out = Some(std::io::BufWriter::new(std::fs::File::create(path)?));
        Ok(())
    }

    /// Answer questions through the router (or the replay record).
    pub fn decide(&mut self, questions: Vec<Question>) -> Result<Vec<Answer>> {
        self.questions += questions.len();
        self.answer(questions)
    }

    /// Hand answers to the sim and the record.
    pub fn submit(&mut self, at: SimTime, answers: Vec<Answer>) -> Result<()> {
        for a in &answers {
            let key = serde_json::to_value(&a.source).ok().and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned)).unwrap_or_default();
            *self.by_source.entry(key).or_default() += 1;
            self.cost += a.cost_usd.unwrap_or(0.0);
            self.record.push_answer(at, a.clone());
        }
        self.sim.answer(answers).map_err(|e| anyhow!("{e}"))
    }

    fn answer(&mut self, questions: Vec<Question>) -> Result<Vec<Answer>> {
        if let Some(rec) = &self.replay {
            return questions
                .iter()
                .map(|q| rec.answers.iter().find(|a| a.answer.question == q.id).map(|a| a.answer.clone()).ok_or_else(|| anyhow!("replay diverged: question {} ({} {}) is not in the record", q.id.0, q.agent, q.trigger)))
                .collect();
        }
        let router = self.router.clone();
        Ok(self.rt.block_on(router.answer(questions)))
    }

    /// Advance to `until`, answering every question on the way. `on_step`
    /// sees the sim after each stop (for a UI or progress).
    pub fn play_until(&mut self, until: SimTime, mut on_step: impl FnMut(&Sim)) -> Result<Option<bgws_sim::Outcome>> {
        loop {
            let step = self.sim.advance(until);
            self.flush_events()?;
            match step {
                Step::Advanced => {
                    on_step(&self.sim);
                    return Ok(None);
                }
                Step::Over(o) => {
                    self.record.outcome = Some(serde_json::to_value(&o)?);
                    self.record.fingerprint = Some(self.sim.fingerprint());
                    on_step(&self.sim);
                    return Ok(Some(o));
                }
                Step::NeedsAnswers { at, questions } => {
                    self.questions += questions.len();
                    let answers = self.answer(questions)?;
                    for a in &answers {
                        let key = serde_json::to_value(&a.source).ok().and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned)).unwrap_or_default();
                        *self.by_source.entry(key).or_default() += 1;
                        self.cost += a.cost_usd.unwrap_or(0.0);
                        self.record.push_answer(at, a.clone());
                    }
                    self.sim.answer(answers).map_err(|e| anyhow!("{e}"))?;
                    on_step(&self.sim);
                }
            }
        }
    }

    fn flush_events(&mut self) -> Result<()> {
        let events: Vec<LogEvent> = self.sim.drain_events();
        if let Some(w) = &mut self.events_out {
            for e in &events {
                serde_json::to_writer(&mut *w, e)?;
                w.write_all(b"\n")?;
            }
            w.flush()?;
        }
        Ok(())
    }

    pub fn report(&self) -> PlayReport {
        PlayReport {
            outcome: self.sim.outcome().cloned(),
            ended_at: self.sim.now(),
            fingerprint: self.sim.fingerprint().to_string(),
            questions: self.questions,
            events: self.sim.ctx.log.count(),
            answers_by_source: self.by_source.clone(),
            cost_usd: self.cost,
        }
    }

    /// Check a replay ended where its record did.
    pub fn verify_replay(&self) -> Result<()> {
        let Some(rec) = &self.replay else { bail!("not a replay") };
        match rec.fingerprint {
            Some(f) if f == self.sim.fingerprint() => Ok(()),
            Some(f) => bail!("replay diverged: recorded fingerprint {f}, replayed {}", self.sim.fingerprint()),
            None => Ok(()),
        }
    }
}
