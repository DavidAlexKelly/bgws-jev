//! The host loop (§3.3). The clock waits in simulated time while the
//! deciders answer, so a slow model never changes the game, only how long
//! it takes to play. Answers are recorded, so a game with Jev and Claude
//! replays exactly without calling them again (§12).

use anyhow::{anyhow, bail, Context, Result};
use bgws_core::{Answer, Figures, LogEvent, Question, QuestionKind, Seed, Side, SimTime};
use bgws_decide::{ClaudeConfig, ClaudeDecider, ClaudePlanner, HeuristicPlanner, JevClient, JevConfig, LocalConfig, LocalDecider, Route, Router};
use bgws_scenario::record::{BuildInfo, CommandKind, GameRecord, MachineInfo, RecordedCommand, RunSetup};
use bgws_scenario::v4::{Decider, ScenarioV4};
use bgws_sim::{SeatKind, Sim, SimOptions, Step};
use serde::{Deserialize, Serialize};
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
    events_out: Option<Box<dyn crate::eventlog::EventSink>>,
    /// Answers from a record, for a replay: questions are answered from here only.
    replay: Option<GameRecord>,
    /// The next recorded command a replay applies.
    next_command: usize,
    questions: usize,
    by_source: std::collections::BTreeMap<String, usize>,
    cost: f64,
}

impl Host {
    pub fn new(scenario: &ScenarioV4, seed: Seed, figures: Figures, opts: &HostOptions) -> Result<Host> {
        let router = router_for(scenario, &figures, opts);
        let defaults = crate::default_figures();
        let overrides = figures.iter().filter(|f| defaults.figure(&f.key).is_some_and(|d| d.value != f.value)).map(|f| (f.key.clone(), f.value)).collect();
        let mut sim_opts = opts.sim.clone();
        if sim_opts.terrain.is_none() {
            sim_opts.terrain = load_pack_terrain(scenario)?;
        }
        let sim = Sim::with_options(scenario, seed, figures, sim_opts).map_err(|e| anyhow!("{e}"))?;
        let mut machine = MachineInfo::compiled_for();
        machine.threads = Some(rayon::current_num_threads() as u32);
        let mut record = GameRecord::new(scenario.clone(), seed, BuildInfo::this(option_env!("BGWS_BUILD_ID").unwrap_or("dev")), machine);
        record.figure_overrides = overrides;
        record.run = RunSetup { flat_ground: opts.sim.flat_ground, plan_at_start: opts.sim.plan_at_start };
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        Ok(Host { sim, router, record, rt, events_out: None, replay: None, next_command: 0, questions: 0, by_source: Default::default(), cost: 0.0 })
    }

    /// Replay a recorded game without calling any decider.
    pub fn replay(record: GameRecord, figures: Figures) -> Result<Host> {
        let mut f = figures;
        for (k, v) in &record.figure_overrides {
            f.set(k, *v).map_err(|e| anyhow!("{e}"))?;
        }
        let sim = SimOptions { flat_ground: record.run.flat_ground, plan_at_start: record.run.plan_at_start, ..Default::default() };
        let mut h = Host::new(&record.scenario, record.seed, f, &HostOptions { deciders: DeciderChoice::Rules, sim, ..Default::default() })?;
        h.replay = Some(record);
        Ok(h)
    }

    /// Restart a recorded game at `at` ("what if", §12): replay the record
    /// to that moment without calling a decider, then hand the questions to
    /// `router`. The new host's record holds the game up to the branch.
    pub fn branch(record: GameRecord, figures: Figures, at: SimTime, router: Router) -> Result<Host> {
        let mut h = Host::replay(GameRecord { ended_at: None, fingerprint: None, outcome: None, ..record }, figures)?;
        h.play_until(at, |_| {})?;
        h.router = router;
        h.stop_replay();
        Ok(h)
    }

    /// Leave replay mode: from now on the router answers (a "what if" branch).
    pub fn stop_replay(&mut self) {
        self.replay = None;
        self.next_command = 0;
    }

    /// Is this host replaying a record?
    pub fn replaying(&self) -> bool {
        self.replay.is_some()
    }

    /// A plan for a side (a person's, over its HQ's nets): applied now and recorded.
    pub fn command(&mut self, side: Side, seat: &str, plan: serde_json::Value) -> Result<()> {
        self.sim.command(side, plan.clone()).map_err(|e| anyhow!("{e}"))?;
        self.record.push_command(self.sim.now(), seat, Some(side), plan);
        Ok(())
    }

    /// Who sits in a side's HQ seat, recorded so a replay makes the same change.
    pub fn set_seat(&mut self, side: Side, seat: SeatKind) {
        self.sim.set_seat(side, seat);
        self.record.push_seat(self.sim.now(), side, serde_json::to_value(seat).unwrap_or_default());
    }

    /// Replaying: apply the recorded commands that are due, at the same
    /// simulated time and the same point in the answers as when given.
    fn apply_recorded(&mut self) -> Result<()> {
        let Some(rec) = &self.replay else { return Ok(()) };
        let (now, answered) = (self.sim.now(), self.record.answers.len());
        let due: Vec<RecordedCommand> = rec.commands.iter().skip(self.next_command).take_while(|c| c.at <= now && c.after_answers <= answered).cloned().collect();
        self.next_command += due.len();
        for c in due {
            let Some(side) = c.side else { bail!("a recorded command has no side") };
            match c.kind {
                CommandKind::Plan => self.command(side, &c.seat, c.orders).map_err(|e| anyhow!("replay diverged: a recorded plan was rejected: {e}"))?,
                CommandKind::Seat => self.set_seat(side, serde_json::from_value(c.orders)?),
            }
        }
        Ok(())
    }

    /// When the next recorded command is due, if it is after now.
    fn next_command_at(&self) -> Option<SimTime> {
        let c = self.replay.as_ref()?.commands.get(self.next_command)?;
        (c.at > self.sim.now()).then_some(c.at)
    }

    pub fn write_events_to(&mut self, path: &Path) -> Result<()> {
        self.events_out = Some(crate::eventlog::open_sink(path)?);
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
            self.apply_recorded()?;
            let stop = self.next_command_at().filter(|t| *t < until).unwrap_or(until);
            let step = self.sim.advance(stop);
            self.flush_events()?;
            match step {
                Step::Advanced => {
                    self.apply_recorded()?;
                    on_step(&self.sim);
                    if stop < until {
                        continue;
                    }
                    return Ok(None);
                }
                Step::Over(o) => {
                    self.record.outcome = Some(serde_json::to_value(&o)?);
                    self.record.fingerprint = Some(self.sim.fingerprint());
                    on_step(&self.sim);
                    return Ok(Some(o));
                }
                Step::NeedsAnswers { at, questions } => {
                    self.apply_recorded()?;
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
            w.write(&events)?;
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

    /// The record as it stands, closed at the current moment: the place to
    /// stop a replay and the fingerprint to check it against.
    pub fn finished_record(&self) -> GameRecord {
        let mut r = self.record.clone();
        r.ended_at = Some(self.sim.now());
        r.fingerprint = Some(self.sim.fingerprint());
        r
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
