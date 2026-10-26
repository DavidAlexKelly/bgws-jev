//! `bgws-server`: plays a scenario on a dedicated thread (§3.3) and serves
//! the map viewer. The browser connects to `/ws`, receives frames, log
//! events and questions for any side a person sits in, and sends play,
//! pause, speed, viewer and answers back. The same messages are what the
//! Tauri shell's channel carries (§16).
//!
//! Scrubbing and "what if" (§12): the game keeps a keyframe of every view
//! every `--keyframe-s` simulated seconds, so the timeline can be dragged
//! back and forth; `branch` restarts the game at a moment by replaying its
//! record there, without calling a decider, and playing on.
//!
//! ```text
//! bgws-server <scenario> [--port 8765] [--person blue] [--deciders rules|jev|claude|local] [--seed N] [--flat]
//!             [--record game.json] [--keyframe-s 30]
//! ```

use anyhow::{anyhow, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use bgws_core::{Answer, AnswerSource, Question, QuestionId, Seed, Side, SimDuration, SimTime};
use bgws_run::{default_figures, load_scenario, DeciderChoice, Host, HostOptions};
use bgws_sim::{SimOptions, Step, Viewer};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

const VIEWER: &str = include_str!("../../../web/viewer/index.html");

#[derive(Parser)]
struct Args {
    scenario: String,
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// A side a person plays: its questions go to the browser.
    #[arg(long)]
    person: Option<Side>,
    #[arg(long, default_value = "scenario")]
    deciders: DeciderChoice,
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long)]
    flat: bool,
    /// Where `save` writes the game record.
    #[arg(long, default_value = "bgws-game.json")]
    record: PathBuf,
    /// Simulated seconds between keyframes for the timeline.
    #[arg(long, default_value_t = 30.0)]
    keyframe_s: f64,
}

/// What the browser sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Inbound {
    Play,
    Pause,
    Speed { speed: f64 },
    Viewer { viewer: Viewer },
    Answer { question: u64, choice: String },
    /// A person's plan for a side (the orders schema), sent over its HQ's nets.
    Command { side: Side, plan: serde_json::Value },
    /// Who sits in a side's HQ seat.
    Seat { side: Side, seat: bgws_sim::SeatKind },
    /// Show the keyframe at or before `t` (simulated seconds); pauses the game.
    Seek { t: f64 },
    /// Back to the live game.
    Live,
    /// Restart the game at `t` (simulated seconds) and play on from there.
    Branch { t: f64 },
    /// Write the game record.
    Save,
    /// What happened to a unit, with the figures behind it (the AAR inspector).
    Why { unit: String },
    /// The after-action review so far.
    Review,
}

/// Views of the game every few simulated seconds, for the timeline. When
/// there are too many, every other one is dropped and the spacing doubles.
struct Keyframes {
    every: SimDuration,
    frames: Vec<(SimTime, [String; 3])>,
}

const VIEWERS: [Viewer; 3] = [Viewer::Umpire, Viewer::Blue, Viewer::Red];
const MAX_KEYFRAMES: usize = 2_000;

impl Keyframes {
    fn take(&mut self, sim: &bgws_sim::Sim) {
        let now = sim.now();
        if self.frames.last().is_some_and(|(t, _)| now < *t + self.every) {
            return;
        }
        self.frames.push((now, VIEWERS.map(|v| serde_json::to_string(&sim.view(v)).unwrap_or_default())));
        if self.frames.len() > MAX_KEYFRAMES {
            self.frames = self.frames.iter().step_by(2).cloned().collect();
            self.every = SimDuration::from_secs(self.every.secs() * 2.0);
        }
    }

    fn at(&self, t: SimTime, viewer: Viewer) -> Option<(SimTime, &str)> {
        let i = VIEWERS.iter().position(|v| *v == viewer)?;
        self.frames.iter().rev().find(|(ft, _)| *ft <= t).or(self.frames.first()).map(|(ft, f)| (*ft, f[i].as_str()))
    }

    fn truncate_after(&mut self, t: SimTime) {
        self.frames.retain(|(ft, _)| *ft <= t);
    }
}

struct GameConfig {
    person: Option<Side>,
    record: PathBuf,
    keyframe_s: f64,
}

#[derive(Clone)]
struct AppState {
    out: broadcast::Sender<String>,
    to_game: mpsc::Sender<Inbound>,
}

fn scenario_path(s: &str) -> PathBuf {
    let p = PathBuf::from(s);
    if p.exists() {
        return p;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios").join(format!("{}.json", s.trim_end_matches(".json")))
}

/// Does this viewer see this event? The umpire sees all; a side sees its own and unsided ones.
fn visible(viewer: Viewer, e: &bgws_core::LogEvent) -> bool {
    viewer.side().is_none_or(|s| e.side.is_none_or(|es| es == s))
}

/// The game thread: the sim never waits on the network or the UI in wall
/// time; it waits in simulated time for answers (§3.2).
fn game(mut host: Host, cfg: GameConfig, inbox: mpsc::Receiver<Inbound>, out: broadcast::Sender<String>) {
    let person = cfg.person;
    let mut keys = Keyframes { every: SimDuration::from_secs(cfg.keyframe_s), frames: Vec::new() };
    let mut scrub: Option<SimTime> = None;
    // Every event so far, for the inspectors (`why`, `review`).
    let mut log: Vec<bgws_core::LogEvent> = Vec::new();
    let mut playing = false;
    let mut speed = 10.0;
    let mut viewer = Viewer::Umpire;
    let mut waiting: BTreeMap<QuestionId, Question> = BTreeMap::new();
    let mut held: Vec<Answer> = Vec::new();
    let mut held_at = SimTime::ZERO;
    let tick = Duration::from_millis(100);
    let send = |out: &broadcast::Sender<String>, v: serde_json::Value| {
        let _ = out.send(v.to_string());
    };
    send(&out, json!({ "kind": "status", "text": "ready: press Play" }));
    loop {
        let started = Instant::now();
        while let Ok(m) = inbox.try_recv() {
            match m {
                Inbound::Play => {
                    playing = true;
                    scrub = None;
                }
                Inbound::Pause => playing = false,
                Inbound::Speed { speed: s } => speed = s.clamp(0.1, 10_000.0),
                Inbound::Viewer { viewer: v } => viewer = v,
                Inbound::Command { side, plan } => {
                    if let Err(e) = host.command(side, "person", plan) {
                        send(&out, json!({ "kind": "status", "text": format!("plan rejected: {e}") }));
                    }
                }
                Inbound::Seat { side, seat } => host.set_seat(side, seat),
                Inbound::Seek { t } => {
                    playing = false;
                    scrub = Some(SimTime::from_secs(t.max(0.0)));
                }
                Inbound::Live => scrub = None,
                // A side's viewer reads only what its feed shows: no truth through the inspectors.
                Inbound::Why { unit } => {
                    let seen: Vec<_> = log.iter().filter(|e| visible(viewer, e)).cloned().collect();
                    send(&out, json!({ "kind": "report", "title": format!("Why: {unit}"), "text": bgws_run::aar::why(&seen, &unit) }));
                }
                Inbound::Review => {
                    let seen: Vec<_> = log.iter().filter(|e| visible(viewer, e)).cloned().collect();
                    send(&out, json!({ "kind": "report", "title": "After-action review", "text": bgws_run::aar::review(&seen) }));
                }
                Inbound::Save => {
                    let text = match host.finished_record().save(&cfg.record) {
                        Ok(()) => format!("saved {}", cfg.record.display()),
                        Err(e) => format!("save failed: {e}"),
                    };
                    send(&out, json!({ "kind": "status", "text": text }));
                }
                Inbound::Branch { t } => {
                    let at = SimTime::from_secs(t.max(0.0)).min(host.sim.now());
                    send(&out, json!({ "kind": "status", "text": format!("branching at {at}…") }));
                    match Host::branch(host.record.clone(), default_figures(), at, host.router.clone()) {
                        Ok(mut h) => {
                            let _ = h.sim.drain_events();
                            host = h;
                            playing = false;
                            scrub = None;
                            waiting.clear();
                            held.clear();
                            keys.truncate_after(at);
                            log.retain(|e| e.t <= at);
                            send(&out, json!({ "kind": "branched", "t": at.secs() }));
                            send(&out, json!({ "kind": "status", "text": format!("branched at {at}: press Play") }));
                        }
                        Err(e) => send(&out, json!({ "kind": "status", "text": format!("branch failed: {e}") })),
                    }
                }
                Inbound::Answer { question, choice } => {
                    if let Some(q) = waiting.remove(&QuestionId(question)) {
                        if q.has_option(&choice) {
                            held.push(Answer::simple(&q, choice, AnswerSource::Person));
                        } else {
                            waiting.insert(q.id, q);
                        }
                    }
                }
            }
        }
        if waiting.is_empty() && !held.is_empty() {
            if let Err(e) = host.submit(held_at, std::mem::take(&mut held)) {
                send(&out, json!({ "kind": "status", "text": format!("answer rejected: {e}") }));
            }
        }
        if playing && waiting.is_empty() {
            let until = host.sim.now() + bgws_core::SimDuration::from_secs(speed * tick.as_secs_f64());
            loop {
                match host.sim.advance(until) {
                    Step::Advanced => break,
                    Step::Over(o) => {
                        playing = false;
                        send(&out, json!({ "kind": "status", "text": format!("over: {} ({})", o.winner.map(|w| format!("{w} wins")).unwrap_or_else(|| "draw".into()), o.reason) }));
                        break;
                    }
                    Step::NeedsAnswers { at, questions } => {
                        let (mine, theirs): (Vec<Question>, Vec<Question>) = questions.into_iter().partition(|q| Some(q.side) == person && q.options.len() > 1);
                        let mut answers = match host.decide(theirs) {
                            Ok(a) => a,
                            Err(e) => {
                                send(&out, json!({ "kind": "status", "text": format!("decider error: {e}") }));
                                playing = false;
                                break;
                            }
                        };
                        if mine.is_empty() {
                            let _ = host.submit(at, answers);
                            continue;
                        }
                        held.append(&mut answers);
                        held_at = at;
                        send(&out, json!({ "kind": "questions", "questions": mine }));
                        waiting.extend(mine.into_iter().map(|q| (q.id, q)));
                        break;
                    }
                }
            }
        }
        keys.take(&host.sim);
        let drained = host.sim.drain_events();
        log.extend(drained.iter().cloned());
        let events: Vec<serde_json::Value> = drained.into_iter().filter(|e| visible(viewer, e)).map(|e| json!({ "clock": format!("{}", e.t), "kind": e.kind, "text": e.text })).collect();
        if !events.is_empty() {
            send(&out, json!({ "kind": "events", "events": events }));
        }
        let live = host.sim.now().secs();
        match scrub {
            Some(t) => {
                if let Some((ft, f)) = keys.at(t, viewer) {
                    let _ = out.send(format!(r#"{{"kind":"frame","scrub":true,"t":{},"live":{live},"frame":{f}}}"#, ft.secs()));
                }
            }
            None => send(&out, json!({ "kind": "frame", "t": live, "live": live, "frame": host.sim.view(viewer) })),
        }
        if waiting.is_empty() {
            send(&out, json!({ "kind": "questions", "questions": [] }));
        }
        if let Some(rest) = tick.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

async fn ws(upgrade: WebSocketUpgrade, State(st): State<AppState>) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| client(socket, st))
}

async fn client(socket: WebSocket, st: AppState) {
    let (mut tx, mut rx) = socket.split();
    let mut feed = st.out.subscribe();
    let to_game = st.to_game.clone();
    let reader = tokio::spawn(async move {
        while let Some(Ok(Message::Text(t))) = rx.next().await {
            if let Ok(m) = serde_json::from_str::<Inbound>(&t) {
                if to_game.send(m).is_err() {
                    break;
                }
            }
        }
    });
    while let Ok(msg) = feed.recv().await {
        if tx.send(Message::Text(msg)).await.is_err() {
            break;
        }
    }
    reader.abort();
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = Args::parse();
    let sc = load_scenario(&scenario_path(&a.scenario))?;
    let seed = Seed(a.seed.unwrap_or(sc.seed.0));
    let opts = HostOptions { deciders: a.deciders, sim: SimOptions { flat_ground: a.flat, ..Default::default() }, ..Default::default() };
    // The host owns its own runtime for model calls, so build it off this one.
    let host = std::thread::spawn(move || Host::new(&sc, seed, default_figures(), &opts)).join().map_err(|_| anyhow!("host setup panicked"))??;
    let (out, _) = broadcast::channel(256);
    let (to_game, inbox) = mpsc::channel();
    let game_out = out.clone();
    let cfg = GameConfig { person: a.person, record: a.record, keyframe_s: a.keyframe_s };
    std::thread::spawn(move || game(host, cfg, inbox, game_out));
    let app = Router::new().route("/", get(|| async { Html(VIEWER) })).route("/ws", get(ws)).with_state(AppState { out, to_game });
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], a.port));
    eprintln!("bgws-server: open http://{addr}/");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
