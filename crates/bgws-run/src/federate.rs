//! A BGWS simulation waiting in an HLA federation for its scenario
//! (`bgws federate`): Foundry, or any federate, tells it what to play and
//! when, through `BGWS_Control` (our FOM module), and gives its units orders
//! through NETN-SMC tasking ([`bgws_sim::tasking`]).
//!
//! - **load** builds the scenario (Foundry's form or a full BGWS one, see
//!   [`crate::foundry`]): real ground for its area is built as a theatre pack
//!   the first time (elevation, land cover and roads from public map data, so
//!   the host needs the internet then) and reused after. Its units go out to
//!   the federation at once, where they stand, so the initial state can be
//!   checked before it starts. A load while a game is on ends that game.
//! - **start** / **resume** play it in real time (at **speed**, simulated
//!   seconds per second); **pause** holds the clock; **stop** ends it and
//!   takes its units off. RPR StartResume and StopFreeze do the same as start
//!   and pause.
//!
//! It answers every command, and says where it is every 30 s, as a
//! `BGWS_Event` of kind `federate.status` (data: requestId, state, message,
//! scenario, simTimeS, speed, units), which the bridge streams to Foundry with
//! the game's own events. Each game's record (every input, Foundry's orders
//! included) is kept, so it replays exactly with no federation.

use crate::host::{Host, HostOptions, Link};
use anyhow::{anyhow, Result};
use bgws_core::{LogEvent, Seed, SimTime};
use bgws_interop::hla::link::HlaLink;
use bgws_interop::hla::netn::{self, ControlMsg, EventOut, Party};
use bgws_sim::Sim;
use serde_json::json;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct FederateOptions {
    /// Where theatre packs for Foundry's areas are built and kept.
    pub packs: PathBuf,
    /// Allow generated ground (no other simulation has it: off by default).
    pub allow_generated: bool,
    /// Simulated seconds per wall-clock second, until told otherwise.
    pub speed: f64,
    /// Where each game's record is written when it ends.
    pub records: Option<PathBuf>,
    /// Grid spacing of packs built for Foundry's areas, m.
    pub cell_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    BuildingTerrain,
    Ready,
    Running,
    Paused,
    Finished,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::BuildingTerrain => "building_terrain",
            State::Ready => "ready",
            State::Running => "running",
            State::Paused => "paused",
            State::Finished => "finished",
        }
    }
}

struct Game {
    host: Host,
    until: SimTime,
    name: String,
    units: usize,
}

/// A terrain build under way: the scenario waits for it.
struct Building {
    request: String,
    scenario: bgws_scenario::v4::ScenarioV4,
    seed: Seed,
    progress: mpsc::Receiver<(String, f64)>,
    done: mpsc::Receiver<Result<PathBuf, String>>,
    last_said: Instant,
}

/// The game's link: the federation, shown each second.
struct GameLink<'a> {
    link: &'a mut HlaLink,
    errors: Vec<String>,
}

impl Link for GameLink<'_> {
    fn tick(&mut self, sim: &Sim, events: &[LogEvent]) {
        if let Err(e) = self.link.tick(sim, events) {
            self.errors.push(e.to_string());
        }
    }
    fn inbound(&mut self, sim: &Sim) -> Vec<bgws_sim::remote::ExternalInput> {
        self.link.inbound(sim)
    }
}

pub struct Federate {
    link: HlaLink,
    opts: FederateOptions,
    spec: bgws_scenario::v4::InteropSpec,
    state: State,
    game: Option<Game>,
    building: Option<Building>,
    speed: f64,
    said: u64,
    heartbeat: Instant,
    /// Real-time pacing: the wall clock and simulated time it counts from.
    pace: Option<(Instant, f64)>,
}

/// Set by SIGTERM or Ctrl-C: leave the federation cleanly (a federate that
/// does not resign holds its name, and a restart under it is refused).
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The pack folder for an area (the same area reuses it).
fn pack_dir(packs: &std::path::Path, b: &bgws_scenario::v4::Bounds) -> PathBuf {
    let c = |v: f64| format!("{v:.4}").replace('-', "m");
    packs.join(format!("foundry_{}_{}_{}_{}", c(b.west), c(b.south), c(b.east), c(b.north)))
}

impl Federate {
    pub fn new(link: HlaLink, spec: bgws_scenario::v4::InteropSpec, opts: FederateOptions) -> Federate {
        let speed = opts.speed;
        Federate { link, opts, spec, state: State::Idle, game: None, building: None, speed, said: 0, heartbeat: Instant::now() - Duration::from_secs(60), pace: None }
    }

    /// Say where we are (to the federation, and on stderr).
    fn status(&mut self, request: &str, message: &str) {
        let t = self.game.as_ref().map(|g| g.host.sim.now().secs()).unwrap_or(0.0);
        let data = json!({
            "requestId": request, "state": self.state.word(), "message": message,
            "scenario": self.game.as_ref().map(|g| g.name.clone()).or_else(|| self.building.as_ref().map(|b| b.scenario.name.clone())),
            "simTimeS": t, "speed": self.speed, "units": self.game.as_ref().map(|g| g.units).unwrap_or(0),
            "durationS": self.game.as_ref().map(|g| g.until.secs()),
        });
        eprintln!("[{}] {}{}", self.state.word(), message, if request.is_empty() { String::new() } else { format!(" (request {request})") });
        self.said += 1;
        let out = EventOut {
            kind: "federate.status".into(),
            // Numbered apart from any game's own events.
            seq: (1 << 40) + self.said,
            t,
            federate: self.link.settings.federate.clone(),
            side: String::new(),
            entity: Party::default(),
            target: Party::default(),
            at: None,
            text: format!("{}: {message}", self.state.word()),
            data: netn::data_json(&data),
        };
        if let Err(e) = self.link.send_event(&out) {
            eprintln!("status not sent: {e}");
        }
        self.heartbeat = Instant::now();
    }

    /// End the game on: keep its record, take its units off.
    fn end_game(&mut self, why: &str) {
        if let Some(g) = self.game.take() {
            if let Some(dir) = &self.opts.records {
                let path = dir.join(format!("{}-{}.json", g.name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_"), chrono_stamp()));
                let saved = std::fs::create_dir_all(dir).map_err(|e| e.to_string()).and_then(|_| g.host.finished_record().save(&path).map_err(|e| e.to_string()));
                match saved {
                    Ok(()) => eprintln!("record of {} written to {} ({why})", g.name, path.display()),
                    Err(e) => eprintln!("record of {} not written: {e}", g.name),
                }
            }
        }
        self.link.reset();
    }

    fn load(&mut self, c: &ControlMsg) {
        if self.building.is_some() {
            return self.status(&c.request_id, "busy building terrain for the last scenario; load again when it is ready");
        }
        let (mut sc, terrain) = match crate::foundry::scenario(&c.scenario) {
            Ok(x) => x,
            Err(e) => return self.status(&c.request_id, &format!("scenario refused: {e}")),
        };
        let seed = sc.seed;
        self.end_game("a new scenario was loaded");
        self.state = State::Idle;
        if let Some(s) = c.speed() {
            self.speed = s;
        }
        match terrain {
            crate::foundry::Terrain::Generated if !self.opts.allow_generated => {
                return self.status(&c.request_id, "scenario refused: generated ground, which no other simulation has; send terrain \"real\" (or start bgws federate with --allow-generated-ground)");
            }
            crate::foundry::Terrain::AsGiven if !self.opts.allow_generated => {
                if let Some(why) = sc.interop.clone().unwrap_or_default().ground_refusal(&sc.theatre, false) {
                    return self.status(&c.request_id, &format!("scenario refused: {why}"));
                }
            }
            crate::foundry::Terrain::Real(b) => {
                let dir = pack_dir(&self.opts.packs, &b);
                if dir.join("pack.json").exists() {
                    sc.theatre.pack = Some(dir.to_string_lossy().into_owned());
                } else {
                    return self.build_terrain(c, sc, seed, b, dir);
                }
            }
            _ => {}
        }
        self.start_game(&c.request_id, sc, seed);
    }

    fn build_terrain(&mut self, c: &ControlMsg, sc: bgws_scenario::v4::ScenarioV4, seed: Seed, b: bgws_scenario::v4::Bounds, dir: PathBuf) {
        let (ptx, prx) = mpsc::channel();
        let (dtx, drx) = mpsc::channel();
        let cell_m = self.opts.cell_m;
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::thread::spawn(move || {
            let r = (|| -> Result<PathBuf, String> {
                let bbox = bgws_geodata::mercator::BBox::new(b.west, b.south, b.east, b.north);
                let mut spec = bgws_geodata::build::BuildSpec::new(bbox);
                spec.cell_m = cell_m;
                let tmp = dir.with_extension("building");
                let _ = std::fs::remove_dir_all(&tmp);
                let progress = |stage: &str, f: f64| {
                    let _ = ptx.send((stage.to_string(), f));
                };
                bgws_geodata::pack::build_pack_with(&tmp, &name, &spec, 3.0, &progress, &mut |_| Ok(())).map_err(|e| e.to_string())?;
                crate::host::analyse_pack(&tmp, &progress).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
                Ok(dir)
            })();
            let _ = dtx.send(r);
        });
        self.state = State::BuildingTerrain;
        self.building = Some(Building { request: c.request_id.clone(), scenario: sc, seed, progress: prx, done: drx, last_said: Instant::now() });
        self.status(&c.request_id, "building real terrain for the area (the first time takes a few minutes)");
    }

    /// A terrain build's progress, and its end.
    fn check_build(&mut self) {
        let Some(b) = self.building.as_mut() else { return };
        let mut latest = None;
        while let Ok(p) = b.progress.try_recv() {
            latest = Some(p);
        }
        let say = latest.filter(|_| b.last_said.elapsed() > Duration::from_secs(10)).map(|(stage, f)| format!("building terrain: {stage} {:.0}%", f * 100.0));
        if say.is_some() {
            b.last_said = Instant::now();
        }
        let request = b.request.clone();
        let done = b.done.try_recv().ok();
        if let Some(m) = say {
            self.status(&request, &m);
        }
        if let Some(r) = done {
            let b = self.building.take().expect("building");
            match r {
                Ok(dir) => {
                    let mut sc = b.scenario;
                    sc.theatre.pack = Some(dir.to_string_lossy().into_owned());
                    self.start_game(&request, sc, b.seed);
                }
                Err(e) => {
                    self.state = State::Idle;
                    self.status(&request, &format!("terrain build failed: {e}"));
                }
            }
        }
    }

    /// The scenario built: its units go out where they stand; it waits for start.
    fn start_game(&mut self, request: &str, mut sc: bgws_scenario::v4::ScenarioV4, seed: Seed) {
        // Our DIS identity is the federate's, whatever the scenario says.
        sc.interop = Some(self.spec.clone());
        let host = match Host::new(&sc, seed, crate::default_figures(), &HostOptions::default()) {
            Ok(h) => h,
            Err(e) => {
                self.state = State::Idle;
                return self.status(request, &format!("scenario refused: {e}"));
            }
        };
        let units = host.sim.ctx.order.len();
        let until = SimTime::from_secs(sc.scope.duration_min * 60.0);
        self.game = Some(Game { host, until, name: sc.name.clone(), units });
        // Its initial state, before it starts.
        if let Some(g) = self.game.as_mut() {
            if let Err(e) = self.link.tick(&g.host.sim, &[]) {
                eprintln!("publishing the initial state: {e}");
            }
        }
        self.state = State::Ready;
        self.status(request, &format!("loaded {} ({units} units); send start", sc.name));
    }

    fn command(&mut self, c: ControlMsg) {
        match c.command.as_str() {
            "load" => self.load(&c),
            "start" | "resume" => match self.state {
                State::Ready | State::Paused => {
                    if let Some(s) = c.speed() {
                        self.speed = s;
                    }
                    self.state = State::Running;
                    self.pace = None;
                    self.status(&c.request_id, &format!("running at {}x", self.speed));
                }
                State::Running => self.status(&c.request_id, "already running"),
                _ => self.status(&c.request_id, "nothing loaded to start; load a scenario first"),
            },
            "pause" => {
                if self.state == State::Running {
                    self.state = State::Paused;
                    self.status(&c.request_id, "paused");
                } else {
                    self.status(&c.request_id, "not running");
                }
            }
            "stop" => {
                self.end_game("stopped");
                self.state = State::Idle;
                self.status(&c.request_id, "stopped; its units are off the federation");
            }
            "speed" => match c.speed() {
                Some(s) => {
                    self.speed = s;
                    self.pace = None;
                    self.status(&c.request_id, &format!("speed {s}x"));
                }
                None => self.status(&c.request_id, "speed needs options.speed > 0"),
            },
            other => self.status(&c.request_id, &format!("unknown command {other}")),
        }
    }

    /// One turn of the loop: commands, a terrain build, a second of play.
    pub fn step(&mut self) -> Result<()> {
        for c in self.link.control() {
            self.command(c);
        }
        self.check_build();
        if self.state == State::Running {
            let speed = self.speed;
            let g = self.game.as_mut().ok_or_else(|| anyhow!("running with no game"))?;
            let now = g.host.sim.now();
            let next = SimTime::from_secs(now.secs().floor() + 1.0).min(g.until);
            let (wall0, sim0) = *self.pace.get_or_insert((Instant::now(), now.secs()));
            let mut gl = GameLink { link: &mut self.link, errors: vec![] };
            // Paced here, a second at a time, so speed and pause act between seconds.
            let over = g.host.play_realtime(next, None, &mut gl)?;
            let target = wall0 + Duration::from_secs_f64(((g.host.sim.now().secs() - sim0) / speed).max(0.0));
            let wait = target.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            if let Some(e) = gl.errors.first() {
                eprintln!("HLA: {e}");
            }
            for n in self.link.notes.drain(..) {
                eprintln!("HLA: {n}");
            }
            let g = self.game.as_ref().expect("game");
            if over.is_some() || g.host.sim.now() >= g.until {
                self.state = State::Finished;
                let verdict = over.map(|o| serde_json::to_value(o).unwrap_or_default()).unwrap_or(json!("time up"));
                self.status("", &format!("finished: {verdict}"));
            }
        } else {
            // Ready or paused: orders are taken in (and answered) at once,
            // without the clock moving, so units can be given orders before
            // the start or during a pause.
            if matches!(self.state, State::Ready | State::Paused) {
                if let Some(g) = self.game.as_mut() {
                    let now = g.host.sim.now();
                    let mut gl = GameLink { link: &mut self.link, errors: vec![] };
                    g.host.play_realtime(now, None, &mut gl)?;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let every = if self.state == State::Finished || self.state == State::Idle { 120 } else { 30 };
        if self.heartbeat.elapsed() > Duration::from_secs(every) {
            self.status("", "heartbeat");
        }
        Ok(())
    }

    /// Wait and play until killed.
    pub fn serve(&mut self) -> Result<()> {
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
            libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        }
        self.status("", "waiting for a scenario");
        while !STOP.load(std::sync::atomic::Ordering::SeqCst) {
            self.step()?;
        }
        self.end_game("shutting down");
        self.state = State::Idle;
        self.status("", "leaving the federation");
        // Resigning is what matters; destroying fails while others are joined.
        let _ = self.link.leave();
        Ok(())
    }

    pub fn state(&self) -> &'static str {
        self.state.word()
    }
    pub fn sim(&self) -> Option<&Sim> {
        self.game.as_ref().map(|g| &g.host.sim)
    }
}

/// UTC now as yyyymmddThhmmss (record file names).
fn chrono_stamp() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (s / 86400, s % 86400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}
