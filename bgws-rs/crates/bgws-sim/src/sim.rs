//! The sans-IO simulation (§3.2): `Sim::new` from a scenario, `advance`
//! until a time or a decision point, `answer`, `command`, views and
//! snapshots. Nothing here touches the network, the disk or a clock.

use crate::clock::{next_stop, Rate};
use crate::components::*;
use crate::ground::Ground;
use crate::hq::{self, HqSituation, HqUnit, Plan};
use crate::params::Params;
use crate::queue::{EventQueue, Priority, Scheduled};
use crate::view::{FrameView, Viewer};
use bevy_ecs::prelude::*;
use bgws_behaviour::{Registry, Vocabulary};
use bgws_catalogue::{Catalogue, Domain, Platform};
use bgws_core::{Answer, Enu, EnuFrame, EntityId, EventLog, Figures, Geodetic, LogEvent, Question, QuestionId, QuestionKind, Seed, Side, SimDuration, SimTime, StreamKey};
use bgws_env::{UtcDateTime, WeatherField, WeatherPreset};
use bgws_know::{SideView, TrackStore};
use bgws_models::people::{Fatigue, Morale, Skill, Suppression};
use bgws_scenario::v4::{ScenarioV4, VictoryCondition, WeatherSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

/// What `advance` returns.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Step {
    /// The clock reached `until`. Nothing is waiting.
    Advanced,
    /// The clock stopped at `at`: these decisions are due. Answer them, then advance again.
    NeedsAnswers { at: SimTime, questions: Vec<Question> },
    /// The game ended.
    Over(Outcome),
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub at: SimTime,
    pub winner: Option<Side>,
    pub reason: String,
    pub strength: BTreeMap<Side, f64>,
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("unknown platform `{platform}` for force `{force}`")]
    UnknownPlatform { force: String, platform: String },
    #[error("bad start time: {0}")]
    StartTime(String),
    #[error("behaviour templates: {0}")]
    Templates(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum AnswerError {
    #[error("no question {0:?} is waiting")]
    Unknown(QuestionId),
    #[error("option `{choice}` is not on offer for question {question:?}")]
    NotOffered { question: QuestionId, choice: String },
}

#[derive(Debug, Error, PartialEq)]
pub enum CommandError {
    #[error("not a valid plan: {0}")]
    BadPlan(String),
}

/// Who sits in an HQ's seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeatKind {
    Planner,
    Person,
    Rules,
}

/// How a run is set up beyond the scenario.
#[derive(Clone, Debug)]
pub struct SimOptions {
    /// Flat open ground instead of the scenario's (calibration vignettes).
    pub flat_ground: bool,
    /// Ask the HQs for plans at the start.
    pub plan_at_start: bool,
    /// A weather field loaded by the host (ERA5), overriding the scenario's.
    pub weather: Option<WeatherField>,
    /// Terrain loaded by the host from the scenario's theatre pack (§14.2).
    pub terrain: Option<bgws_env::Terrain>,
}

impl Default for SimOptions {
    fn default() -> Self {
        SimOptions { flat_ground: false, plan_at_start: true, weather: None, terrain: None }
    }
}

/// A question that is out, and what it applies to.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    pub q: Question,
    pub entity: Option<EntityId>,
    pub about: Option<EntityId>,
    pub answered: bool,
    pub handed_out: bool,
}

/// A trigger raised by a system for a unit this tick.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Trigger {
    pub entity: EntityId,
    pub point: &'static str,
    pub about: Option<EntityId>,
}

/// Everything besides the ECS world.
pub struct Ctx {
    pub now: SimTime,
    pub seed: Seed,
    pub figures: Arc<Figures>,
    pub p: Params,
    pub log: EventLog,
    pub queue: EventQueue,
    pub scenario: ScenarioV4,
    pub catalogue: Arc<Catalogue>,
    pub registry: Registry,
    pub vocab: Vocabulary,
    pub ground: Ground,
    pub weather: WeatherField,
    pub start: UtcDateTime,
    pub frame: EnuFrame,
    pub tracks: BTreeMap<Side, TrackStore>,
    pub(crate) pending: BTreeMap<QuestionId, Pending>,
    pub(crate) answers: BTreeMap<QuestionId, Answer>,
    pub(crate) next_q: u64,
    pub(crate) triggers: Vec<Trigger>,
    pub over: Option<Outcome>,
    pub order: Vec<Entity>,
    pub by_name: BTreeMap<String, EntityId>,
    pub seats: BTreeMap<Side, SeatKind>,
    pub(crate) last_asked: BTreeMap<EntityId, SimTime>,
    pub(crate) reports: Vec<(SimTime, Side, bgws_know::Detection)>,
    pub(crate) last_plan: BTreeMap<Side, Plan>,
}

pub struct Sim {
    pub world: World,
    pub ctx: Ctx,
}

impl Ctx {
    pub fn entity(&self, id: EntityId) -> Entity {
        self.order[id.index()]
    }
    pub fn rng(&self, purpose: &'static str, a: u64, b: u64) -> rand_chacha::ChaCha8Rng {
        self.seed.stream(StreamKey::new(purpose, a, b, self.now.0))
    }
    pub fn duration_limit(&self) -> SimTime {
        SimTime::from_secs(self.scenario.scope.duration_min * 60.0)
    }
}

fn weather_preset(w: bgws_scenario::scope::Weather) -> WeatherPreset {
    use bgws_scenario::scope::Weather as W;
    match w {
        W::Clear => WeatherPreset::Clear,
        W::Overcast => WeatherPreset::Overcast,
        W::Rain => WeatherPreset::Rain,
        W::Fog => WeatherPreset::Fog,
        W::Snow => WeatherPreset::Snow,
        W::Storm => WeatherPreset::Storm,
    }
}

impl Sim {
    /// Build the world from a scenario (§3.2).
    pub fn new(scenario: &ScenarioV4, seed: Seed, figures: Figures) -> Result<Sim, LoadError> {
        Self::with_options(scenario, seed, figures, SimOptions::default())
    }

    pub fn with_options(scenario: &ScenarioV4, seed: Seed, figures: Figures, opts: SimOptions) -> Result<Sim, LoadError> {
        let figures = Arc::new(figures);
        let p = Params::from_figures(&figures);
        let catalogue = Arc::new(Catalogue::builtin_with(&figures).map_err(|e| LoadError::Templates(format!("catalogue: {e:?}")))?);
        let vocab = Vocabulary::builtin();
        let custom: Vec<bgws_behaviour::Template> = scenario.behaviours.templates.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
        let registry = Registry::with_custom(&custom, &vocab, bgws_behaviour::load::LoadMode::Compatible).map_err(|e| LoadError::Templates(e.to_string()))?;
        let frame = scenario.frame();
        let origin = scenario.theatre.origin;
        // Ground covering every position, with a margin.
        let mut half: f64 = 5_000.0;
        for (_, g) in scenario.positions() {
            let v = frame.geodetic_to_enu(g);
            half = half.max(v.e.abs().max(v.n.abs()) + 3_000.0);
        }
        let ground = if let Some(t) = opts.terrain.clone() {
            Ground::from_terrain(t, bgws_core::rng::fnv1a(scenario.theatre.ground_seed.as_bytes()), &figures)
        } else if opts.flat_ground || scenario.theatre.ground == "flat" {
            Ground::flat(origin, half, &figures)
        } else {
            let seed_text = if scenario.theatre.ground_seed.is_empty() { scenario.name.clone() } else { scenario.theatre.ground_seed.clone() };
            Ground::generated(origin, &seed_text, half, &figures)
        };
        let start = UtcDateTime::parse(&scenario.start_utc).map_err(|e| LoadError::StartTime(format!("{e:?}")))?;
        let wspec = bgws_env::GridSpec::new(4, 4, half * 2.0 / 3.0, -half, -half);
        let weather = opts.weather.clone().unwrap_or_else(|| match &scenario.weather {
            WeatherSource::Preset { weather } => WeatherField::preset(wspec, weather_preset(*weather)),
            WeatherSource::Synthetic { seed: s, .. } => {
                let sp = bgws_env::weather::SynthWeatherParams {
                    seed: s.unwrap_or(seed.0),
                    spec: wspec,
                    start,
                    lat: origin.lat,
                    lon: origin.lon,
                    duration_h: (scenario.scope.duration_min / 60.0).max(1.0) + 1.0,
                    step_h: 1.0,
                    fronts: 2,
                };
                bgws_env::weather::synthetic(&sp, &figures)
            }
            WeatherSource::Era5 { .. } => WeatherField::preset(wspec, WeatherPreset::Clear),
        });
        let mut ctx = Ctx {
            now: SimTime::ZERO,
            seed,
            figures: figures.clone(),
            p,
            log: EventLog::new(),
            queue: EventQueue::new(),
            scenario: scenario.clone(),
            catalogue: catalogue.clone(),
            registry,
            vocab,
            ground,
            weather,
            start,
            frame,
            tracks: Side::PLAYERS.iter().map(|s| (*s, TrackStore::new(*s))).collect(),
            pending: BTreeMap::new(),
            answers: BTreeMap::new(),
            next_q: 1,
            triggers: Vec::new(),
            over: None,
            order: Vec::new(),
            by_name: BTreeMap::new(),
            seats: Side::PLAYERS.iter().map(|s| (*s, SeatKind::Planner)).collect(),
            last_asked: BTreeMap::new(),
            reports: Vec::new(),
            last_plan: BTreeMap::new(),
        };
        let mut world = World::new();
        for (i, f) in scenario.forces.iter().enumerate() {
            let platform = catalogue.platform(&f.platform).ok_or_else(|| LoadError::UnknownPlatform { force: f.id.clone(), platform: f.platform.clone() })?.clone();
            let id = EntityId(i as u32);
            let e = spawn_force(&mut world, &mut ctx, id, f, Arc::new(platform));
            ctx.order.push(e);
            ctx.by_name.insert(f.id.clone(), id);
        }
        ctx.log.push(SimTime::ZERO, "sim.start").text(format!("{}: {} forces, seed {}", scenario.name, scenario.forces.len(), seed.0)).data(serde_json::json!({ "scenario": scenario.name, "seed": seed.0, "figures": figures.len() }));
        for (i, ev) in scenario.events.iter().enumerate() {
            ctx.queue.schedule(SimTime::from_secs(ev.at), Priority::Scenario, Scheduled::ScriptEvent { index: i as u32 });
        }
        let mut sim = Sim { world, ctx };
        crate::world::setup(&mut sim);
        if opts.plan_at_start && !scenario.scope.planners.is_empty() {
            for side in Side::PLAYERS {
                if sim.ctx.scenario.forces_of(side).next().is_some() {
                    sim.raise_hq_question(side, "the start of the operation");
                }
            }
        }
        Ok(sim)
    }

    pub fn now(&self) -> SimTime {
        self.ctx.now
    }

    pub fn figures(&self) -> &Figures {
        &self.ctx.figures
    }

    /// Advance the clock to `until`, stopping early at a decision point or the end.
    /// With `scope.time.skipQuiet`, quiet stretches skip the land sensing pass
    /// (see [`Sim::advance_quiet`]); the outcome is the same either way.
    pub fn advance(&mut self, until: SimTime) -> Step {
        let skip = self.ctx.scenario.scope.time.map(|t| t.skip_quiet).unwrap_or(false);
        self.advance_with(until, skip)
    }

    /// Advance with quiet skipping (§6.3) whatever the scope says: while
    /// nothing is in contact, in flight or waiting, base ticks before the
    /// earliest possible detection ([`Sim::next_interesting_time`]) run
    /// without the sensing pass, which provably has nothing to do there. It
    /// never changes what happens: the event log is the stepped one.
    pub fn advance_quiet(&mut self, until: SimTime) -> Step {
        self.advance_with(until, true)
    }

    /// Quiet now? Then the next scheduled event or the earliest time a sensor
    /// could detect something (from closing speeds and sensor reach), capped
    /// by `sim.quiet.max_jump_s` and the time limit. `None` when there is
    /// contact, something in flight, or a question or report waiting.
    pub fn next_interesting_time(&self) -> Option<SimTime> {
        crate::domains::lod::quiet::next_interesting_time(self)
    }

    fn advance_with(&mut self, until: SimTime, skip_quiet: bool) -> Step {
        loop {
            if let Some(o) = &self.ctx.over {
                return Step::Over(o.clone());
            }
            let waiting: Vec<Question> = self.ctx.pending.values().filter(|p| !p.answered).map(|p| p.q.clone()).collect();
            if !waiting.is_empty() {
                for p in self.ctx.pending.values_mut() {
                    p.handed_out = true;
                }
                return Step::NeedsAnswers { at: self.ctx.now, questions: waiting };
            }
            if self.ctx.now >= until {
                return Step::Advanced;
            }
            let fast = crate::world::fast_active(self);
            let air = crate::world::air_active(self);
            let mut t = next_stop(self.ctx.now, fast, air).min(until.max(self.ctx.now));
            if t <= self.ctx.now {
                t = next_stop(self.ctx.now, fast, air);
            }
            let sense = !(skip_quiet && Rate::Base.due(t) && crate::domains::lod::quiet::may_skip_sensing(self, t));
            self.tick(t, sense);
        }
    }

    /// One stop of the clock at `t`. `sense` false skips the land sensing
    /// pass (quiet skipping only, when it provably finds nothing).
    fn tick(&mut self, t: SimTime, sense: bool) {
        let dt = (t - self.ctx.now).secs();
        self.ctx.now = t;
        // Scheduled events due now.
        {
            prof_span!("events");
            for (_, ev) in self.ctx.queue.pop_due(t) {
                self.handle(ev);
            }
        }
        {
            prof_span!("world.fast");
            crate::world::fast(self, dt);
        }
        if Rate::Base.due(t) {
            prof_span!("base");
            {
                prof_span!("posture");
                crate::systems::posture(self);
            }
            {
                prof_span!("movement");
                crate::systems::movement(self, 1.0);
            }
            {
                prof_span!("world.base");
                crate::world::base(self, 1.0);
            }
            if sense {
                prof_span!("sensing");
                crate::systems::sensing(self, 1.0);
            }
            {
                prof_span!("reports");
                crate::systems::reports(self);
            }
            {
                prof_span!("direct_fire");
                crate::systems::direct_fire(self);
            }
            {
                prof_span!("people");
                crate::systems::people(self, 1.0);
            }
            {
                prof_span!("autopilot");
                crate::systems::autopilot_events(self);
            }
            {
                prof_span!("decide");
                crate::decide::raise(self);
            }
            {
                prof_span!("victory");
                crate::systems::victory(self);
            }
        }
        if Rate::Slow.due(t) {
            prof_span!("world.slow");
            crate::world::slow(self, 10.0);
        }
        if Rate::Minute.due(t) {
            prof_span!("minute");
            crate::systems::fatigue(self, 60.0);
            crate::world::minute(self, 60.0);
        }
    }

    fn handle(&mut self, ev: Scheduled) {
        match ev {
            Scheduled::ApplyAnswer { question } => crate::decide::apply(self, QuestionId(question)),
            Scheduled::ScriptEvent { index } => {
                if let Some(e) = self.ctx.scenario.events.get(index as usize).cloned() {
                    self.ctx.log.push(self.ctx.now, "scenario.event").side(e.side).text(format!("{} ({})", e.kind, e.side)).data(serde_json::to_value(&e).unwrap_or_default());
                    crate::world::script_event(self, &e);
                }
            }
            other => crate::world::scheduled(self, other),
        }
    }

    /// Hand back answers to the questions `advance` returned.
    pub fn answer(&mut self, answers: Vec<Answer>) -> Result<(), AnswerError> {
        for a in &answers {
            let p = self.ctx.pending.get(&a.question).ok_or(AnswerError::Unknown(a.question))?;
            if p.q.kind != QuestionKind::Hq && !p.q.has_option(&a.choice) {
                return Err(AnswerError::NotOffered { question: a.question, choice: a.choice.clone() });
            }
        }
        for a in answers {
            let p = self.ctx.pending.get_mut(&a.question).expect("checked");
            p.answered = true;
            let due = p.q.at + p.q.reaction;
            let due = if due < self.ctx.now { self.ctx.now } else { due };
            let src = serde_json::to_value(&a.source).unwrap_or_default();
            self.ctx.log.push(self.ctx.now, "decide.answer").side(p.q.side).text(format!("{}: {} → {}", p.q.agent, p.q.title, a.choice)).data(serde_json::json!({
                "question": a.question.0, "choice": a.choice, "source": src, "confidence": a.confidence, "probabilities": a.probabilities, "costUsd": a.cost_usd, "rationale": a.rationale,
            }));
            self.ctx.queue.schedule(due, Priority::Decision, Scheduled::ApplyAnswer { question: a.question.0 });
            self.ctx.answers.insert(a.question, a);
        }
        Ok(())
    }

    /// A person or a planner gives a side's orders directly (an HQ's plan).
    pub fn command(&mut self, side: Side, plan: serde_json::Value) -> Result<(), CommandError> {
        let plan: Plan = serde_json::from_value(plan).map_err(|e| CommandError::BadPlan(e.to_string()))?;
        // With nets, a person's orders travel them like anyone else's (§10.4).
        let now = self.ctx.now;
        if !crate::domains::drones::route_plan(self, side, &plan, now) {
            crate::decide::apply_plan(self, side, &plan);
        }
        Ok(())
    }

    pub fn set_seat(&mut self, side: Side, seat: SeatKind) {
        self.ctx.seats.insert(side, seat);
    }

    /// Ask a side's HQ for a plan now (a CCIR fired, the battle rhythm).
    pub fn raise_hq_question(&mut self, side: Side, why: &str) {
        let situation = self.hq_situation(side, why);
        let id = QuestionId(self.ctx.next_q);
        self.ctx.next_q += 1;
        let q = Question {
            id,
            at: self.ctx.now,
            kind: QuestionKind::Hq,
            agent: hq::hq_agent(side),
            side,
            trigger: "plan".into(),
            template: "hq".into(),
            node: "plan".into(),
            title: format!("{} plan", side.as_str()),
            question: format!("Plan the {} side's operation: give each unit its task, intent and phases.", side.as_str()),
            doctrine: vec![self.ctx.scenario.command.side(side).directive.clone()].into_iter().filter(|s| !s.is_empty()).collect(),
            situation: serde_json::to_string(&situation).unwrap_or_default(),
            options: vec![],
            fallback: "heuristic".into(),
            rules_choice: "heuristic".into(),
            reaction: SimDuration::ZERO,
            plan_schema: Some(hq::orders_plan_schema()),
        };
        self.ctx.log.push(self.ctx.now, "decide.question").side(side).text(format!("{} HQ asked for a plan ({why})", side.as_str()));
        self.ctx.pending.insert(id, Pending { q, entity: None, about: None, answered: false, handed_out: false });
    }

    /// The HQ's picture: own units as reported, and the side's tracks. Never truth.
    pub fn hq_situation(&self, side: Side, why: &str) -> HqSituation {
        let mut units = Vec::new();
        let mut subs = Vec::new();
        let mut centroid = Enu::default();
        let mut n = 0.0;
        for &e in &self.ctx.order {
            let (Some(i), Some(k), Some(h)) = (self.world.get::<Ident>(e), self.world.get::<Kin>(e), self.world.get::<Health>(e)) else { continue };
            if i.side != side || i.domain != Domain::Land {
                continue;
            }
            subs.push(i.name.clone());
            let reach = i.platform.mounts.iter().filter_map(|m| m.munitions.first()).filter_map(|m| self.ctx.catalogue.munition(m)).map(|m| m.max_range_m).fold(0.0, f64::max);
            let role = match i.platform.template.as_str() {
                _ if i.hq => "post",
                "core/artillery" | "core/mortar" => "fires",
                "core/engineer" => "engineer",
                "core/convoy" | "core/node" | "core/sensor" => "support",
                _ => "manoeuvre",
            };
            // With nets, the HQ knows its units as they last reported (§9.2).
            let rep = crate::domains::drones::hq_unit_report(self, side, i.id);
            units.push(HqUnit {
                id: i.name.clone(),
                label: i.label.clone(),
                role: role.into(),
                strength: rep.as_ref().map(|r| r.strength).unwrap_or(h.strength()),
                vehicles_fit: rep.as_ref().map(|r| r.fit).unwrap_or(h.fit),
                vehicles_total: h.start,
                at: self.ctx.frame.enu_to_geodetic(rep.as_ref().map(|r| r.at).unwrap_or(k.pos)),
                reach_m: if reach > 0.0 { Some(reach) } else { None },
                alive: rep.as_ref().map(|r| r.alive).unwrap_or(!h.destroyed),
            });
            centroid = centroid.add(k.pos);
            n += 1.0;
        }
        // The top HQ's COP when the side has HQs; else the side's tracks.
        let cop = crate::domains::drones::hq_cop_tracks(self, side).unwrap_or_else(|| self.ctx.tracks[&side]
            .confirmed(&self.ctx.p.know)
            .into_iter()
            .map(|t| hq::CopTrack {
                id: format!("{}", t.id),
                thought: t.class_top.first().map(|(h, p)| format!("{h:?} ({:.0}%)", p * 100.0)).unwrap_or_else(|| "unknown".into()),
                domain: format!("{:?}", t.domain).to_lowercase(),
                seen_ago_s: (self.ctx.now - t.last_update).secs(),
                by: t.sources.first().and_then(|s| self.ctx.scenario.forces.get(s.index())).map(|f| f.id.clone()).unwrap_or_default(),
                quality: t.quality,
                error_m: t.ellipse.semi_major,
                at: Some(self.ctx.frame.enu_to_geodetic(t.position)),
            })
            .collect::<Vec<_>>());
        // The objective: the enemy's start centroid as briefed (the scenario's intelligence).
        let enemy: Vec<Geodetic> = self.ctx.scenario.forces_of(side.enemy()).map(|f| f.position).collect();
        let objective = if enemy.is_empty() {
            None
        } else {
            Some(Geodetic::new(enemy.iter().map(|g| g.lat).sum::<f64>() / enemy.len() as f64, enemy.iter().map(|g| g.lon).sum::<f64>() / enemy.len() as f64, 0.0))
        };
        let mut refs = BTreeMap::new();
        if let Some(o) = objective {
            refs.insert("OBJECTIVE".to_string(), o);
        }
        if n > 0.0 {
            refs.insert("START".to_string(), self.ctx.frame.enu_to_geodetic(centroid.scale(1.0 / n)));
        }
        HqSituation {
            hq: hq::hq_agent(side),
            echelon: format!("{:?}", self.ctx.scenario.scope.echelons.highest).to_lowercase(),
            planner: if self.ctx.scenario.scope.planners.iter().any(|p| format!("{p:?}").to_lowercase() == "overall") { "overall".into() } else { "land".into() },
            subordinates: subs,
            sub_hqs: vec![],
            post: crate::domains::drones::hq_post(self, side).and_then(|p| self.world.get::<Kin>(self.ctx.entity(p))).map(|k| self.ctx.frame.enu_to_geodetic(k.pos)),
            why: why.into(),
            cop,
            units,
            objective,
            reference_points: refs,
            shareable_means: BTreeMap::new(),
        }
    }

    /// A frame for the UI: truth for the umpire, else what a side believes.
    pub fn view(&self, viewer: Viewer) -> FrameView {
        crate::view::frame(self, viewer)
    }

    /// What a side's deciders may read (rule 1).
    pub fn side_view(&self, side: Side) -> SideView {
        crate::view::side_view(self, side)
    }

    pub fn drain_events(&mut self) -> Vec<LogEvent> {
        self.ctx.log.drain()
    }

    pub fn fingerprint(&self) -> bgws_core::Fingerprint {
        self.ctx.log.fingerprint()
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.ctx.over.as_ref()
    }

    /// Each side's surviving land strength, 0–1.
    pub fn strength(&self) -> BTreeMap<Side, f64> {
        let mut fit: BTreeMap<Side, (f64, f64)> = BTreeMap::new();
        for &e in &self.ctx.order {
            let (Some(i), Some(h)) = (self.world.get::<Ident>(e), self.world.get::<Health>(e)) else { continue };
            if i.dummy || i.side == Side::Neutral || !crate::domains::air::counts_for_strength(&i.platform) {
                continue;
            }
            let s = fit.entry(i.side).or_insert((0.0, 0.0));
            s.0 += if h.destroyed { 0.0 } else { h.fit as f64 };
            s.1 += h.start as f64;
        }
        fit.into_iter().map(|(s, (a, b))| (s, if b > 0.0 { a / b } else { 0.0 })).collect()
    }

    /// The scenario's victory conditions met now, if any.
    pub fn victory_conditions(&self) -> &[VictoryCondition] {
        &self.ctx.scenario.victory
    }

    /// A save point: the state that matters for a replay restart (§12).
    pub fn snapshot(&self) -> Snapshot {
        crate::view::snapshot(self)
    }
}

impl Sim {
    /// Answers from the rules alone: each question's rules choice, and for an
    /// HQ the simplest plan (every unit advances on the objective, tactical).
    /// The batch default and the offline fallback; the decider crate's
    /// heuristic planner is the fuller version.
    pub fn rules_answers(&self, questions: &[Question]) -> Vec<Answer> {
        questions
            .iter()
            .map(|q| {
                if q.kind == QuestionKind::Hq {
                    let sit: Option<HqSituation> = serde_json::from_str(&q.situation).ok();
                    let orders: Vec<serde_json::Value> = sit
                        .map(|s| {
                            s.units
                                .iter()
                                .filter(|u| u.alive)
                                .map(|u| {
                                    let attack = u.role == "manoeuvre" && q.side == Side::Blue;
                                    serde_json::json!({
                                        "unit": u.id, "task": if attack { "attack" } else { "defend" }, "intent": if attack { "close with and destroy the enemy on the objective" } else { "hold ground and destroy the enemy as it comes" },
                                        "phases": if attack { serde_json::json!([{ "label": "advance", "do": "move", "to": { "ref": "OBJECTIVE" }, "mode": "tactical" }]) } else { serde_json::json!([{ "label": "hold", "do": "overwatch" }]) }
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut a = Answer::simple(q, "heuristic", bgws_core::AnswerSource::Rules);
                    a.plan = Some(serde_json::json!({ "plan": "rules: blue advances on the objective, red holds", "orders": orders }));
                    a
                } else {
                    Answer::simple(q, q.rules_choice.clone(), bgws_core::AnswerSource::Rules)
                }
            })
            .collect()
    }

    /// Play to the end (or `limit`) answering with the rules: the batch loop.
    pub fn run_with_rules(&mut self, limit: SimTime) -> Option<Outcome> {
        loop {
            match self.advance(limit) {
                Step::Advanced => return None,
                Step::Over(o) => return Some(o),
                Step::NeedsAnswers { questions, .. } => {
                    let a = self.rules_answers(&questions);
                    self.answer(a).expect("rules answers are valid");
                }
            }
        }
    }
}

/// Saved state (positions, health, activities) for scrubbing and save games.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub at: SimTime,
    pub fingerprint: bgws_core::Fingerprint,
    pub entities: Vec<serde_json::Value>,
}

/// Spawn one scenario force.
pub(crate) fn spawn_force(world: &mut World, ctx: &mut Ctx, id: EntityId, f: &bgws_scenario::v4::Force, platform: Arc<Platform>) -> Entity {
    let pos = ctx.frame.geodetic_to_enu(f.position);
    let ground_h = ctx.ground.height(pos.e, pos.n);
    let domain = platform.domain;
    let agl = match domain {
        Domain::Air | Domain::Space | Domain::Munition => (f.position.h - ground_h).max(0.0),
        _ => 0.0,
    };
    let pos = Enu::new(pos.e, pos.n, if matches!(domain, Domain::Land | Domain::Node | Domain::Cyber) { ground_h } else { pos.u });
    let template = ctx.scenario.behaviours.assign.get(&f.id).cloned().unwrap_or_else(|| platform.template.clone());
    let quality = f.quality.rating();
    let count = if f.count > 0 { f.count } else { platform.body.count.max(1) };
    let skill = Skill { base: (quality as f64 - 1.0) / 5.0, experience: 0.0 };
    let arms = Arms {
        mounts: platform
            .mounts
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let munition = m.munitions.first()?.clone();
                let stored = platform.stores.ammo.get(&munition).copied().map(|x| x as u32);
                let count = if f.count > 0 { f.count } else { platform.body.count.max(1) };
                Some(MountState { index: i, munition, rounds: stored.unwrap_or(m.magazine).max(m.magazine) * count, ready_at: SimTime::from_secs(m.ready_s) })
            })
            .collect(),
        engagements: BTreeMap::new(),
    };
    let heading = f.heading_deg.unwrap_or(match f.side {
        Side::Blue => 0.0,
        Side::Red => 180.0,
        Side::Neutral => 90.0,
    });
    let hq = f.command_rating.is_some() || platform.template == "core/hq";
    let e = world
        .spawn((
            Ident { id, name: f.id.clone(), label: f.label.clone(), side: f.side, platform: platform.clone(), domain, template, quality, dummy: f.dummy || platform.signatures.decoy, hq },
            Kin { pos, vel: Enu::default(), heading: heading.to_radians(), agl },
            Health::new(count),
            Crew {
                suppression: Suppression::default(),
                morale: Morale::new(0.5 + 0.1 * (quality as f64 - 3.0)),
                fatigue: Fatigue::default(),
                skill,
                near_misses_per_min: 0.0,
                blasts_per_min: 0.0,
                new_casualties: 0.0,
                last_incoming: None,
                success: false,
            },
            arms,
            Act {
                activity: Activity::Overwatch,
                roe: RoeS::Always,
                posture: Posture::Settled,
                still_since: SimTime::from_secs(-3600.0),
                mission: Mission { purpose: "hold".into(), phases: vec![], phase: 0, urgent: false },
                deciding: false,
                last_shot: None,
                last_event: SimTime::ZERO,
                volleys: 0,
            },
            Aware::default(),
            SigState { engine_on: domain != Domain::Land || platform.mover.kind != bgws_catalogue::MoverKind::Foot, ..Default::default() },
            Lod::Entity,
        ))
        .id();
    if f.concealed {
        if let Some(mut a) = world.get_mut::<Act>(e) {
            a.posture = Posture::HullDown;
            a.activity = Activity::Quiet;
        }
    }
    e
}
