//! `bgws-server`, as a library (the binary and the desktop shell both run it): plays a scenario on a dedicated thread (§3.3) and serves
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
use axum::extract::{Path as UrlPath, State};
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, post};
use axum::Json;
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

const CLASSIC: &str = include_str!("../../../web/viewer/index.html");
// The viewer (web/app), built by `npm run build` there; the build is committed so cargo alone serves it.
const APP_HTML: &str = include_str!("../../../web/app/dist/index.html");
const APP_JS: &str = include_str!("../../../web/app/dist/app.js");
const APP_CSS: &str = include_str!("../../../web/app/dist/app.css");
const MAP_JS: &str = include_str!("../../../web/app/dist/map.js");
const MAP_CSS: &str = include_str!("../../../web/app/dist/map.css");

#[derive(Parser, Clone, Debug)]
#[command(name = "bgws-server")]
pub struct Args {
    pub scenario: String,
    #[arg(long, default_value_t = 8765)]
    pub port: u16,
    /// A side a person plays: its questions go to the browser.
    #[arg(long)]
    pub person: Option<Side>,
    #[arg(long, default_value = "scenario")]
    pub deciders: DeciderChoice,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub flat: bool,
    /// Where `save` writes the game record.
    #[arg(long, default_value = "bgws-game.json")]
    pub record: PathBuf,
    /// Simulated seconds between keyframes for the timeline.
    #[arg(long, default_value_t = 30.0)]
    pub keyframe_s: f64,
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
    /// Why one unit would see another now (the detection inspector; umpire only).
    Detect { observer: String, target: String },
    /// A unit's latest messages and why each was slow or lost (the link-budget inspector).
    Messages { unit: String },
    /// What a unit sees over the terrain (the viewshed layer; own units in a side view).
    Viewshed { unit: String },
    /// Advance the game this many simulated seconds, then reply `advanced`
    /// (a client that runs the clock itself: the React screens' RustRunner).
    Advance { seconds: f64 },
    /// Check an edited scenario (the editor).
    Check { scenario: serde_json::Value },
    /// Replace the game with an edited scenario, same deciders.
    Load { scenario: serde_json::Value },
    /// A browser has connected: the game, the clock, any questions waiting and the feed so far.
    Hello,
    /// One unit in detail (the inspector): truth for the umpire and own units.
    Inspect { unit: String },
    /// The shipped scenarios, for the "new game" picker.
    Scenarios,
    /// Replace the game with a shipped scenario, same deciders.
    /// With `pack`: moved onto that theatre pack (its forces shifted so the
    /// scenario's origin sits at the pack's centre).
    Open {
        name: String,
        #[serde(default)]
        pack: Option<String>,
    },
    /// Build a theatre pack from real ground for a box (west, south, east, north),
    /// in the background, reporting `terrain` progress.
    Terrain {
        name: String,
        bbox: [f64; 4],
        #[serde(default)]
        cell_m: Option<f64>,
    },
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
    opts: HostOptions,
    record: PathBuf,
    keyframe_s: f64,
}

#[derive(Clone)]
struct AppState {
    out: broadcast::Sender<Out>,
    to_game: mpsc::Sender<Inbound>,
}

fn scenario_path(s: &str) -> PathBuf {
    let p = PathBuf::from(s);
    if p.exists() {
        return p;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios").join(format!("{}.json", s.trim_end_matches(".json")))
}

/// The link-budget inspector's answer: each message, how it went, hop by hop.
fn messages_text(sim: &bgws_sim::Sim, msgs: &[bgws_sim::domains::drones::comms::MessageRecord]) -> String {
    if msgs.is_empty() {
        return "No messages yet (or this scenario does not model the nets).".into();
    }
    let name = |id: bgws_core::EntityId| sim.world.get::<bgws_sim::components::Ident>(sim.ctx.entity(id)).map(|i| i.label.clone()).unwrap_or_else(|| format!("#{}", id.0));
    let node = |n: u32| name(bgws_core::EntityId(n));
    let mut t = String::new();
    for m in msgs {
        let took = (m.done - m.sent).secs();
        let how = if m.delivered { format!("delivered in {took:.0} s over {}", m.bearer.as_deref().unwrap_or("?")) } else { format!("LOST after {took:.0} s: {}", m.reason.as_deref().unwrap_or("?")) };
        t.push_str(&format!("{}  {} → {}  {}: {how}\n", m.sent, name(m.from), name(m.to), m.what));
        for f in &m.fallbacks {
            t.push_str(&format!("    fell back from {:?} (PACE level {}) at {}: {:?}\n", f.bearer, f.pace_level, f.at, f.failure));
        }
        for h in &m.hops {
            t.push_str(&format!(
                "    {} → {} on {:?}: queued {:.1} s, on air {:.1} s, {} attempt{}, SINR {:.1} dB, P(success) {:.2}{}\n",
                node(h.from.0), node(h.to.0), h.bearer, h.queue_wait_s, h.airtime_s, h.attempts, if h.attempts == 1 { "" } else { "s" }, h.sinr_db, h.p_success, if h.success { "" } else { ", failed" },
            ));
        }
    }
    t
}

/// The detection inspector's answer, in words, then every factor.
fn detection_text(x: &serde_json::Value) -> String {
    let f = |v: &serde_json::Value| v.as_f64().unwrap_or(f64::NAN);
    let los = &x["lineOfSight"];
    let mut t = format!(
        "{} → {} at {:.0} m: P(detect within {:.0} s) = {:.2}\n\nLine of sight: terrain {} (clearance {:.1} m); clear of canopy {:.2}; overall {:.2}\nLight: {:.1} lux{}; visibility {:.0} m\nTarget: critical size {:.1} m, contrast {:.2}, ΔT {:.1} K, concealment {:.2}{}\nObserver: {}cued, search time ×{:.2}\n\nSensors:\n",
        x["observer"].as_str().unwrap_or("?"), x["target"].as_str().unwrap_or("?"), f(&x["rangeM"]), f(&x["windowS"]), f(&x["pDetectInWindow"]),
        if los["terrainVisible"].as_bool().unwrap_or(false) { "clear" } else { "masked" }, f(&los["clearanceM"]), f(&los["pClearOfCanopy"]), f(&los["pVisible"]),
        f(&x["light"]["lux"]), if x["light"]["night"].as_bool().unwrap_or(false) { " (night)" } else { "" }, f(&x["visibilityM"]),
        f(&x["target_signature"]["criticalDimM"]), f(&x["target_signature"]["contrast"]), f(&x["target_signature"]["deltaTK"]), f(&x["target_signature"]["concealment"]),
        if x["target_signature"]["moving"].as_bool().unwrap_or(false) { ", moving" } else { "" },
        if x["observerState"]["cued"].as_bool().unwrap_or(false) { "" } else { "not " }, f(&x["observerState"]["tauMultiplier"]),
    );
    for sn in x["sensors"].as_array().into_iter().flatten() {
        let a = &sn["acquire"];
        t.push_str(&format!(
            "- {} (to {:.0} m{}): {:.1} cycles against N50 {:.1}, P∞ {:.2}, τ {:.1} s → {:.4}/s{}\n",
            sn["sensor"].as_str().unwrap_or("?"), f(&sn["maxRangeM"]), if sn["inRange"].as_bool().unwrap_or(false) { "" } else { ", out of range" },
            f(&a["cycles"]), f(&a["n50"]), f(&a["p_inf"]), f(&a["tau_s"]), f(&a["rate_per_s"]),
            if sn["nightDegraded"].as_bool().unwrap_or(false) { ", degraded at night" } else { "" },
        ));
    }
    t.push_str("\nEvery factor:\n");
    t.push_str(&serde_json::to_string_pretty(x).unwrap_or_default());
    t
}

/// What goes to the browsers: JSON messages, or binary position updates.
#[derive(Clone, Debug)]
enum Out {
    Text(String),
    Bin(Vec<u8>),
}

/// How often a full frame goes out while the units on the map stay the same.
const FULL_FRAME_EVERY: Duration = Duration::from_secs(1);

/// Positions since the last full frame, as typed arrays (§16): little-endian
/// `u8 1, f64 t, u32 n`, then per entity `u32 id, f64 lat, f64 lon, f32 alt,
/// f32 heading, u8 status` (0 ok, 1 suppressed, 2 pinned, 3 broken, 4 destroyed).
fn positions(t: f64, f: &bgws_sim::FrameView) -> Vec<u8> {
    let mut b = Vec::with_capacity(13 + 29 * f.entities.len());
    b.push(1u8);
    b.extend_from_slice(&t.to_le_bytes());
    b.extend_from_slice(&(f.entities.len() as u32).to_le_bytes());
    for e in &f.entities {
        b.extend_from_slice(&e.id.to_le_bytes());
        b.extend_from_slice(&e.lat.to_le_bytes());
        b.extend_from_slice(&e.lon.to_le_bytes());
        b.extend_from_slice(&(e.alt as f32).to_le_bytes());
        b.extend_from_slice(&(e.heading_deg as f32).to_le_bytes());
        b.push(match e.status.as_str() {
            "suppressed" => 1,
            "pinned" => 2,
            "broken" => 3,
            "destroyed" => 4,
            _ => 0,
        });
    }
    b
}

/// Does this viewer see this event? The umpire sees all; a side sees its own and unsided ones.
fn visible(viewer: Viewer, e: &bgws_core::LogEvent) -> bool {
    viewer.side().is_none_or(|s| e.side.is_none_or(|es| es == s))
}

/// Who decides for each side, and whether its key is there (keys never leave the host).
fn deciders_json(sc: &bgws_scenario::ScenarioV4, opts: &HostOptions, person: Option<Side>) -> serde_json::Value {
    let pick = |d: bgws_scenario::Decider| -> String {
        let d = match opts.deciders {
            DeciderChoice::Scenario => d,
            DeciderChoice::Rules => bgws_scenario::Decider::Rules,
            DeciderChoice::Jev => bgws_scenario::Decider::Jev,
            DeciderChoice::Claude => bgws_scenario::Decider::Claude,
            DeciderChoice::Local => bgws_scenario::Decider::Local,
        };
        format!("{d:?}").to_lowercase()
    };
    let key = |d: &str| match d {
        "claude" => std::env::var_os("ANTHROPIC_API_KEY").is_some(),
        "jev" => std::env::var_os("OPENROUTER_API_KEY").is_some(),
        _ => true,
    };
    let mut out = serde_json::Map::new();
    for side in Side::PLAYERS {
        let c = sc.command.side(side);
        let (units, hqs) = (pick(c.units), pick(c.hqs));
        out.insert(side.as_str().into(), json!({ "units": units, "hqs": hqs, "keyMissing": !key(&units) || !key(&hqs), "person": person == Some(side), "directive": c.directive }));
    }
    serde_json::Value::Object(out)
}

/// The game as the viewer's header and "new game" panel show it.
fn game_json(host: &Host, opts: &HostOptions, person: Option<Side>) -> serde_json::Value {
    let sc = &host.record.scenario;
    let count = |side: Side| sc.forces_of(side).count();
    json!({
        "kind": "game", "name": sc.name, "description": sc.description, "notes": sc.notes,
        "scope": sc.scope, "startUtc": sc.start_utc, "victory": sc.victory,
        "origin": { "lat": sc.theatre.origin.lat, "lon": sc.theatre.origin.lon },
        "forces": { "blue": count(Side::Blue), "red": count(Side::Red) },
        "deciders": deciders_json(sc, opts, person), "flat": host.record.run.flat_ground,
        "limitS": host.sim.ctx.duration_limit().secs(),
        "pack": sc.theatre.pack.as_deref().and_then(pack_json),
    })
}

/// A pack in brief, with the viewer's URLs for its basemap and hillshade
/// (served under /packs/ when it lives in the packs folder).
fn pack_json(name: &str) -> Option<serde_json::Value> {
    let dir = bgws_run::resolve_pack(name);
    let meta: bgws_env::PackMeta = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).ok()?).ok()?;
    let served = dir.parent().and_then(|p| p.canonicalize().ok()) == bgws_run::packs_dir().canonicalize().ok();
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let url = |f: &Option<String>| f.as_ref().filter(|_| served).map(|f| format!("/packs/{folder}/{f}"));
    let roads_km = meta.sources.as_ref().and_then(|s| s["roads"]["km"].as_f64());
    // Towns, for the viewer's place names (the basemap carries no fonts).
    let frame = bgws_core::EnuFrame::new(meta.frame_origin);
    let towns: Vec<serde_json::Value> = meta
        .towns
        .as_ref()
        .and_then(|f| std::fs::read(dir.join(f)).ok())
        .and_then(|b| serde_json::from_slice::<Vec<bgws_env::Town>>(&b).ok())
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let g = frame.enu_to_geodetic(bgws_core::Enu::new(t.e, t.n, 0.0));
            json!({ "name": t.name, "lat": g.lat, "lon": g.lon, "radiusM": t.radius_m })
        })
        .collect();
    Some(json!({
        "name": folder, "bbox": meta.bbox, "cellM": meta.cell, "basemap": url(&meta.basemap), "dem": url(&meta.dem),
        "preview": url(&Some("preview.png".to_string())), "roadsKm": roads_km, "towns": towns,
        "sizeKm": [meta.spec.cell * (meta.spec.width - 1) as f64 / 1000.0, meta.spec.cell * (meta.spec.height - 1) as f64 / 1000.0],
    }))
}

/// Every pack in the packs folder.
fn packs_json() -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = std::fs::read_dir(bgws_run::packs_dir())
        .map(|d| d.flatten().filter(|e| e.path().join("pack.json").exists()).filter_map(|e| pack_json(&e.file_name().to_string_lossy())).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

/// A scenario moved onto a pack: every position shifted by the step that
/// takes the middle of its forces to the pack's centre (their layout kept),
/// checked to fit.
fn onto_pack(mut sc: bgws_scenario::ScenarioV4, pack: &str) -> Result<bgws_scenario::ScenarioV4, String> {
    let dir = bgws_run::resolve_pack(pack);
    let meta: bgws_env::PackMeta = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).map_err(|e| format!("pack {pack}: {e}"))?).map_err(|e| format!("pack {pack}: {e}"))?;
    let [w, s, e, n] = meta.bbox.ok_or_else(|| format!("pack {pack} has no box (built by `bgws pack`, not `bgws terrain`)"))?;
    let (mut lat0, mut lat1, mut lon0, mut lon1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for f in &sc.forces {
        lat0 = lat0.min(f.position.lat);
        lat1 = lat1.max(f.position.lat);
        lon0 = lon0.min(f.position.lon);
        lon1 = lon1.max(f.position.lon);
    }
    let (mid_lat, mid_lon) = if sc.forces.is_empty() { (sc.theatre.origin.lat, sc.theatre.origin.lon) } else { ((lat0 + lat1) / 2.0, (lon0 + lon1) / 2.0) };
    // Everything moves together: forces, their tasks and lanes, obstacles,
    // measures, areas and victory conditions.
    sc.translate((s + n) / 2.0 - mid_lat, (w + e) / 2.0 - mid_lon);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack.to_string());
    let outside = sc.forces.iter().filter(|f| f.position.lat < s || f.position.lat > n || f.position.lon < w || f.position.lon > e).count();
    if outside > 0 {
        return Err(format!("{outside} of the scenario's forces fall outside pack {pack}: build a bigger box"));
    }
    Ok(sc)
}

/// Log events as the browser reads them, with names for ids.
fn events_json(sim: &bgws_sim::Sim, events: &[bgws_core::LogEvent]) -> Vec<serde_json::Value> {
    if events.is_empty() {
        return Vec::new();
    }
    let names: BTreeMap<bgws_core::EntityId, &str> = sim.ctx.by_name.iter().map(|(n, id)| (*id, n.as_str())).collect();
    let name = |id: Option<bgws_core::EntityId>| id.and_then(|i| names.get(&i).copied());
    events.iter().map(|e| json!({ "clock": format!("{}", e.t), "t": e.t.secs(), "kind": e.kind, "text": e.text, "side": e.side, "entity": name(e.entity), "target": name(e.target), "data": e.data })).collect()
}

/// The shipped scenarios, in brief.
fn scenarios_json() -> serde_json::Value {
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(scenarios_dir()) {
        let mut paths: Vec<_> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        for p in paths {
            let Ok(v) = std::fs::read_to_string(&p).map_err(|_| ()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|_| ())) else { continue };
            let forces = v["forces"].as_array().map(|f| f.iter().filter(|x| x["layer"].as_str().unwrap_or("unit") == "unit").count()).unwrap_or(0);
            out.push(json!({ "file": p.file_stem().and_then(|s| s.to_str()), "name": v["name"], "description": v["description"], "type": v["scope"]["type"], "durationMin": v["scope"]["durationMin"], "units": forces }));
        }
    }
    json!({ "kind": "scenarios", "scenarios": out, "packs": packs_json() })
}

/// Build a theatre pack on its own thread, reporting progress to every browser.
fn spawn_terrain_build(name: String, bbox: [f64; 4], cell_m: Option<f64>, out: broadcast::Sender<Out>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let send = |v: serde_json::Value| drop(out.send(Out::Text(v.to_string())));
        let safe = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe {
            return send(json!({ "kind": "terrain", "name": name, "state": "failed", "text": "name: letters, digits, - and _ only" }));
        }
        let b = bgws_geodata::mercator::BBox::new(bbox[0], bbox[1], bbox[2], bbox[3]);
        if let Err(e) = bgws_geodata::mercator::BBox::parse(&format!("{},{},{},{}", b.west, b.south, b.east, b.north)) {
            return send(json!({ "kind": "terrain", "name": name, "state": "failed", "text": e }));
        }
        let mut spec = bgws_geodata::build::BuildSpec::new(b);
        if let Some(c) = cell_m {
            spec.cell_m = c.clamp(10.0, 200.0);
        }
        let last = std::sync::Mutex::new((String::new(), -1i32));
        let progress = |stage: &str, f: f64| {
            let pct = (f * 100.0).floor() as i32;
            if let Ok(mut l) = last.lock() {
                if l.0 != stage || pct >= l.1 + 5 {
                    *l = (stage.to_string(), pct);
                    send(json!({ "kind": "terrain", "name": name, "state": "running", "stage": stage, "fraction": f }));
                }
            }
        };
        let dir = bgws_run::packs_dir().join(&name);
        match bgws_geodata::pack::build_pack(&dir, &name, &spec, 3.0, &progress) {
            Ok(_) => {
                send(json!({ "kind": "terrain", "name": name, "state": "done", "pack": pack_json(&name) }));
                send(scenarios_json());
            }
            Err(e) => send(json!({ "kind": "terrain", "name": name, "state": "failed", "text": e.to_string() })),
        }
    })
}

/// A pack's file, with byte ranges (MapLibre reads PMTiles by range).
async fn pack_file(UrlPath((pack, file)): UrlPath<(String, String)>, headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    let ok = |s: &str| !s.is_empty() && !s.contains("..") && !s.starts_with('/') && !s.contains('\\');
    if !ok(&pack) || !ok(&file) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = bgws_run::packs_dir().join(&pack).join(&file);
    let Ok(mut f) = std::fs::File::open(&path) else { return StatusCode::NOT_FOUND.into_response() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let kind = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("bytes=")).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        let a: u64 = a.parse().ok()?;
        let b: u64 = if b.is_empty() { len.saturating_sub(1) } else { b.parse().ok()? };
        (a <= b && a < len).then_some((a, b.min(len - 1)))
    });
    // Only the bytes asked for: a map reads a basemap a tile at a time.
    let read = |f: &mut std::fs::File, a: u64, n: u64| -> std::io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        f.seek(SeekFrom::Start(a))?;
        let mut buf = vec![0; n as usize];
        f.read_exact(&mut buf)?;
        Ok(buf)
    };
    match range {
        Some((a, b)) => match read(&mut f, a, b - a + 1) {
            Ok(bytes) => (
                StatusCode::PARTIAL_CONTENT,
                [(header::CONTENT_TYPE, kind.to_string()), (header::CONTENT_RANGE, format!("bytes {a}-{b}/{len}")), (header::ACCEPT_RANGES, "bytes".to_string())],
                bytes,
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => match read(&mut f, 0, len) {
            Ok(bytes) => ([(header::CONTENT_TYPE, kind.to_string()), (header::ACCEPT_RANGES, "bytes".to_string())], bytes).into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    }
}

/// The world overview's zooms: the proxy reports this as the archive's
/// deepest, so maps over-zoom it and detail comes only from packs.
const WORLD_MAX_ZOOM: u8 = 9;

/// The Protomaps planet the world overview reads, pinned in the cache folder
/// (`world.url`) so its cached ranges stay valid from day to day.
static WORLD: std::sync::Mutex<Option<std::sync::Arc<dyn bgws_geodata::source::RangeSource>>> = std::sync::Mutex::new(None);

fn world_source(fresh: bool) -> Result<std::sync::Arc<dyn bgws_geodata::source::RangeSource>, String> {
    let mut w = WORLD.lock().map_err(|e| e.to_string())?;
    if let (Some(src), false) = (w.as_ref(), fresh) {
        return Ok(src.clone());
    }
    let cache = bgws_geodata::build::default_cache();
    let pinned = cache.as_ref().map(|c| c.join("world.url"));
    let saved = pinned.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let url = match (std::env::var("BGWS_WORLD").ok(), saved, fresh) {
        (Some(u), _, _) => u,
        (None, Some(u), false) => u,
        (None, saved, _) => match bgws_geodata::build::latest_planet() {
            Ok(u) => {
                if let Some(p) = &pinned {
                    let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
                    let _ = std::fs::write(p, &u);
                }
                u
            }
            // Offline: the build cached last.
            Err(e) => saved.ok_or_else(|| format!("world map: {e}"))?,
        },
    };
    let src: std::sync::Arc<dyn bgws_geodata::source::RangeSource> = std::sync::Arc::from(bgws_geodata::source::open_source(&url, cache.as_deref()).map_err(|e| e.to_string())?);
    *w = Some(src.clone());
    Ok(src)
}

/// The world overview for the maps: byte ranges of the Protomaps planet,
/// fetched once and kept in the cache (`$BGWS_CACHE`), with the header's
/// deepest zoom capped at `WORLD_MAX_ZOOM`.
async fn world_file(headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("bytes=")).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        let a: u64 = a.parse().ok()?;
        let b: u64 = b.parse().ok()?;
        (a <= b && b - a < 64 << 20).then_some((a, b))
    });
    let Some((a, b)) = range else { return (StatusCode::RANGE_NOT_SATISFIABLE, "byte ranges only").into_response() };
    let read = tokio::task::spawn_blocking(move || {
        let first = world_source(false).and_then(|s| s.read(a, b - a + 1).map_err(|e| e.to_string()));
        // A pinned build that has gone: find the latest and try once more.
        first.or_else(|_| world_source(true).and_then(|s| s.read(a, b - a + 1).map_err(|e| e.to_string())))
    })
    .await;
    match read {
        Ok(Ok(mut bytes)) => {
            // The header (byte 101 is its deepest zoom).
            if a == 0 && bytes.len() > 101 && bytes.starts_with(b"PMTiles") {
                bytes[101] = bytes[101].min(WORLD_MAX_ZOOM);
            }
            (
                StatusCode::PARTIAL_CONTENT,
                [(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_RANGE, format!("bytes {a}-{}/*", a + bytes.len() as u64 - 1)), (header::ACCEPT_RANGES, "bytes".to_string()), (header::CACHE_CONTROL, "max-age=86400".to_string())],
                bytes,
            )
                .into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// The game thread: the sim never waits on the network or the UI in wall
/// time; it waits in simulated time for answers (§3.2).
fn game(mut host: Host, cfg: GameConfig, inbox: mpsc::Receiver<Inbound>, out: broadcast::Sender<Out>) {
    let person = cfg.person;
    let mut keys = Keyframes { every: SimDuration::from_secs(cfg.keyframe_s), frames: Vec::new() };
    let mut scrub: Option<SimTime> = None;
    let mut last_full: Option<(Instant, Viewer, Vec<u32>)> = None;
    // A client-driven step in progress, and whether it finished this tick.
    let mut step_to: Option<SimTime> = None;
    // Every event so far, for the inspectors (`why`, `review`).
    let mut log: Vec<bgws_core::LogEvent> = Vec::new();
    let mut playing = false;
    let mut speed = 10.0;
    let mut viewer = Viewer::Umpire;
    // What the browsers were last told of the clock.
    let mut told = (false, speed, viewer);
    // A terrain build in the background, if one is running.
    let mut building: Option<std::thread::JoinHandle<()>> = None;
    let mut waiting: BTreeMap<QuestionId, Question> = BTreeMap::new();
    let mut held: Vec<Answer> = Vec::new();
    let mut held_at = SimTime::ZERO;
    let tick = Duration::from_millis(100);
    let send = |out: &broadcast::Sender<Out>, v: serde_json::Value| {
        let _ = out.send(Out::Text(v.to_string()));
    };
    send(&out, json!({ "kind": "status", "text": "ready: press Play" }));
    loop {
        let started = Instant::now();
        while let Ok(m) = inbox.try_recv() {
            match m {
                Inbound::Advance { seconds } => {
                    step_to = Some(step_to.unwrap_or(host.sim.now()) + SimDuration::from_secs(seconds.max(0.0)));
                    scrub = None;
                }
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
                Inbound::Detect { observer, target } => {
                    let text = if viewer != Viewer::Umpire {
                        "The detection inspector reads truth: switch to the umpire view.".to_string()
                    } else {
                        match (host.sim.ctx.by_name.get(&observer), host.sim.ctx.by_name.get(&target)) {
                            (Some(o), Some(t)) => host.sim.explain_detection(*o, *t, 60.0).map(|x| detection_text(&x)).unwrap_or_else(|| "One of them is not on the board (destroyed, aggregated or not a land unit).".into()),
                            _ => "Unknown unit.".into(),
                        }
                    };
                    send(&out, json!({ "kind": "report", "title": format!("Can {observer} see {target}?"), "text": text }));
                }
                Inbound::Messages { unit } => {
                    let text = match host.sim.ctx.by_name.get(&unit).copied() {
                        None => "Unknown unit.".to_string(),
                        Some(id) => {
                            let side = host.sim.world.get::<bgws_sim::components::Ident>(host.sim.ctx.entity(id)).map(|i| i.side);
                            if viewer.side().is_some_and(|v| Some(v) != side) {
                                "Not one of your units.".to_string()
                            } else {
                                messages_text(&host.sim, &bgws_sim::domains::drones::comms::messages_of(&host.sim, id, 12))
                            }
                        }
                    };
                    send(&out, json!({ "kind": "report", "title": format!("Messages: {unit}"), "text": text }));
                }
                Inbound::Viewshed { unit } => {
                    let id = host.sim.ctx.by_name.get(&unit).copied();
                    let side = id.and_then(|id| host.sim.world.get::<bgws_sim::components::Ident>(host.sim.ctx.entity(id)).map(|i| i.side));
                    match id {
                        Some(id) if viewer.side().is_none_or(|v| Some(v) == side) => match host.sim.viewshed(id, 320) {
                            Some(mut v) => {
                                v["kind"] = json!("viewshed");
                                v["unit"] = json!(unit);
                                v["side"] = json!(side);
                                send(&out, v);
                            }
                            None => send(&out, json!({ "kind": "status", "text": "no viewshed: the unit is not on the ground" })),
                        },
                        _ => send(&out, json!({ "kind": "status", "text": "viewshed: not one of your units" })),
                    }
                }
                Inbound::Check { scenario } => {
                    let errors = bgws_run::editor::check(scenario).err().unwrap_or_default();
                    send(&out, json!({ "kind": "checked", "errors": errors, "loaded": false }));
                }
                m @ (Inbound::Load { .. } | Inbound::Open { .. }) => {
                    let from_editor = matches!(m, Inbound::Load { .. });
                    let made = match m {
                        Inbound::Load { scenario } => bgws_run::editor::check(scenario).map_err(|e| e.join("; ")),
                        Inbound::Open { name, pack } => load_scenario(&scenario_path(&name)).map_err(|e| e.to_string()).and_then(|sc| match pack {
                            Some(p) => onto_pack(sc, &p),
                            None => Ok(sc),
                        }),
                        _ => unreachable!(),
                    }
                    .and_then(|sc| Host::new(&sc, sc.seed, default_figures(), &cfg.opts).map_err(|e| e.to_string()));
                    match made {
                        Ok(h) => {
                            host = h;
                            playing = false;
                            scrub = None;
                            waiting.clear();
                            held.clear();
                            keys.frames.clear();
                            last_full = None;
                            log.clear();
                            send(&out, json!({ "kind": "branched", "t": 0.0, "new": true }));
                            send(&out, game_json(&host, &cfg.opts, person));
                            send(&out, json!({ "kind": "status", "text": format!("loaded {}: press Play", host.record.scenario.name) }));
                            if from_editor {
                                send(&out, json!({ "kind": "checked", "errors": [], "loaded": true }));
                            }
                        }
                        Err(e) if from_editor => send(&out, json!({ "kind": "checked", "errors": [e], "loaded": false })),
                        Err(e) => send(&out, json!({ "kind": "status", "text": format!("could not open: {e}") })),
                    }
                }
                Inbound::Hello => {
                    send(&out, game_json(&host, &cfg.opts, person));
                    send(&out, json!({ "kind": "state", "playing": playing, "speed": speed, "viewer": viewer }));
                    let seen: Vec<_> = log.iter().filter(|e| visible(viewer, e)).cloned().collect();
                    let recent = &seen[seen.len().saturating_sub(400)..];
                    send(&out, json!({ "kind": "events", "events": events_json(&host.sim, recent), "replay": true }));
                    if !waiting.is_empty() {
                        send(&out, json!({ "kind": "questions", "questions": waiting.values().collect::<Vec<_>>() }));
                    }
                    last_full = None;
                }
                Inbound::Inspect { unit } => match host.sim.ctx.by_name.get(&unit).and_then(|id| bgws_sim::view::unit_detail(&host.sim, *id, viewer)) {
                    Some(mut d) => {
                        // Its latest decisions and what it heard, from the feed this viewer may read.
                        let id = host.sim.ctx.by_name[&unit];
                        let mine = |e: &&bgws_core::LogEvent| e.entity == Some(id) && visible(viewer, e);
                        let decisions: Vec<_> = log.iter().rev().filter(mine).filter(|e| e.kind == "decide.apply").take(6).map(|e| json!({ "t": e.t.secs(), "text": e.text, "data": e.data })).collect();
                        let recent: Vec<_> = log.iter().rev().filter(mine).filter(|e| !e.kind.starts_with("decide.")).take(8).map(|e| json!({ "t": e.t.secs(), "kind": e.kind, "text": e.text })).collect();
                        d["decisions"] = json!(decisions);
                        d["recent"] = json!(recent);
                        d["kind"] = json!("unit");
                        send(&out, d);
                    }
                    None => send(&out, json!({ "kind": "unit", "name": unit, "gone": true })),
                },
                Inbound::Scenarios => send(&out, scenarios_json()),
                Inbound::Terrain { name, bbox, cell_m } => {
                    if building.as_ref().is_some_and(|h: &std::thread::JoinHandle<()>| !h.is_finished()) {
                        send(&out, json!({ "kind": "terrain", "name": name, "state": "busy", "text": "a terrain build is already running" }));
                    } else {
                        building = Some(spawn_terrain_build(name, bbox, cell_m, out.clone()));
                    }
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
                            last_full = None;
                            log.retain(|e| e.t <= at);
                            send(&out, json!({ "kind": "branched", "t": at.secs() }));
                            send(&out, json!({ "kind": "status", "text": format!("branched at {at}: press Play") }));
                        }
                        Err(e) => send(&out, json!({ "kind": "status", "text": format!("branch failed: {e}") })),
                    }
                }
                Inbound::Answer { question, choice } => {
                    if let Some(q) = waiting.remove(&QuestionId(question)) {
                        // An HQ question has no options: any answer means "no new plan, keep the orders given".
                        if q.has_option(&choice) || q.kind == bgws_core::QuestionKind::Hq {
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
        let mut step_done = false;
        if (playing || step_to.is_some()) && waiting.is_empty() {
            let until = step_to.unwrap_or_else(|| host.sim.now() + bgws_core::SimDuration::from_secs(speed * tick.as_secs_f64()));
            loop {
                match host.sim.advance(until) {
                    Step::Advanced => break,
                    Step::Over(o) => {
                        playing = false;
                        if step_to.take().is_some() {
                            step_done = true;
                        }
                        send(&out, json!({ "kind": "status", "text": format!("over: {} ({})", o.winner.map(|w| format!("{w} wins")).unwrap_or_else(|| "draw".into()), o.reason) }));
                        break;
                    }
                    Step::NeedsAnswers { at, questions } => {
                        // A person's questions, and the HQ questions of any side whose seat a person holds.
                        let seats = host.sim.ctx.seats.clone();
                        let (mine, theirs): (Vec<Question>, Vec<Question>) = questions
                            .into_iter()
                            .partition(|q| (Some(q.side) == person && q.options.len() > 1) || (q.kind == bgws_core::QuestionKind::Hq && seats.get(&q.side) == Some(&bgws_sim::SeatKind::Person)));
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
        if step_to.is_some_and(|t| host.sim.now() >= t) {
            step_to = None;
            step_done = true;
        }
        keys.take(&host.sim);
        let drained = host.sim.drain_events();
        log.extend(drained.iter().cloned());
        let shown: Vec<bgws_core::LogEvent> = drained.into_iter().filter(|e| visible(viewer, e)).collect();
        let events = events_json(&host.sim, &shown);
        if !events.is_empty() {
            send(&out, json!({ "kind": "events", "events": events }));
        }
        let live = host.sim.now().secs();
        match scrub {
            Some(t) => {
                if let Some((ft, f)) = keys.at(t, viewer) {
                    let _ = out.send(Out::Text(format!(r#"{{"kind":"frame","scrub":true,"t":{},"live":{live},"frame":{f}}}"#, ft.secs())));
                    last_full = None;
                }
            }
            None => {
                // A full frame when the units on the map change, the viewer
                // changes, or a second has passed; else positions only.
                let f = host.sim.view(viewer);
                let ids: Vec<u32> = f.entities.iter().map(|e| e.id).collect();
                let fresh = last_full.as_ref().is_some_and(|(at, v, prev)| at.elapsed() < FULL_FRAME_EVERY && *v == viewer && *prev == ids);
                if fresh {
                    let _ = out.send(Out::Bin(positions(live, &f)));
                } else {
                    send(&out, json!({ "kind": "frame", "t": live, "live": live, "frame": f }));
                    last_full = Some((Instant::now(), viewer, ids));
                }
            }
        }
        if waiting.is_empty() {
            send(&out, json!({ "kind": "questions", "questions": [] }));
        }
        if told != (playing, speed, viewer) {
            told = (playing, speed, viewer);
            send(&out, json!({ "kind": "state", "playing": playing, "speed": speed, "viewer": viewer }));
        }
        if step_done {
            send(&out, json!({ "kind": "advanced", "t": host.sim.now().secs(), "over": host.sim.outcome() }));
        }
        if let Some(rest) = tick.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

// ── The Behaviour Manager's library (custom templates as files) ───────────

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios")
}

async fn editor_page() -> axum::response::Response {
    match bgws_run::editor::html(&scenarios_dir()) {
        Ok(h) => Html(h).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn behaviour_page() -> axum::response::Response {
    match bgws_run::behaviour::library_html(&bgws_run::behaviour::library_dir()) {
        Ok(h) => Html(h).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn behaviour_data() -> axum::response::Response {
    match bgws_run::behaviour::registry_with(&bgws_run::behaviour::library_dir()) {
        Ok(r) => Json(bgws_run::behaviour::data(&r)).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn save_template(Json(t): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let errors = match serde_json::from_value::<bgws_behaviour::Template>(t) {
        Ok(t) => bgws_run::behaviour::save(&bgws_run::behaviour::library_dir(), &t).err().unwrap_or_default(),
        Err(e) => vec![format!("not a template: {e}")],
    };
    Json(json!({ "ok": errors.is_empty(), "errors": errors }))
}

async fn delete_template(UrlPath(name): UrlPath<String>) -> Json<serde_json::Value> {
    match bgws_run::behaviour::delete(&bgws_run::behaviour::library_dir(), &format!("custom/{name}")) {
        Ok(()) => Json(json!({ "ok": true, "errors": [] })),
        Err(e) => Json(json!({ "ok": false, "errors": [e.to_string()] })),
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
        let m = match msg {
            Out::Text(t) => Message::Text(t),
            Out::Bin(b) => Message::Binary(b),
        };
        if tx.send(m).await.is_err() {
            break;
        }
    }
    reader.abort();
}

/// Play `a.scenario` and serve the viewer, the editor and the Behaviour
/// Manager until the process ends. `ready` gets the address once listening
/// (port 0 picks a free one, as the desktop shell does).
pub async fn serve(a: Args, ready: impl FnOnce(std::net::SocketAddr)) -> Result<()> {
    let sc = load_scenario(&scenario_path(&a.scenario))?;
    let seed = Seed(a.seed.unwrap_or(sc.seed.0));
    let opts = HostOptions { deciders: a.deciders, sim: SimOptions { flat_ground: a.flat, ..Default::default() }, ..Default::default() };
    // The host owns its own runtime for model calls, so build it off this one.
    let host_opts = opts.clone();
    let host = std::thread::spawn(move || Host::new(&sc, seed, default_figures(), &host_opts)).join().map_err(|_| anyhow!("host setup panicked"))??;
    let (out, _) = broadcast::channel(256);
    let (to_game, inbox) = mpsc::channel();
    let game_out = out.clone();
    let cfg = GameConfig { person: a.person, opts: opts.clone(), record: a.record, keyframe_s: a.keyframe_s };
    std::thread::spawn(move || game(host, cfg, inbox, game_out));
    // Fail early if the custom library does not load.
    bgws_run::behaviour::library_html(&bgws_run::behaviour::library_dir())?;
    // Fail early if the editor's data does not load; it is rebuilt per request (new templates appear).
    bgws_run::editor::html(&scenarios_dir())?;
    let app = Router::new()
        .route("/", get(|| async { ([(axum::http::header::CACHE_CONTROL, "no-cache")], Html(APP_HTML)) }))
        .route("/app.js", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], APP_JS) }))
        .route("/app.css", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], APP_CSS) }))
        .route("/classic", get(|| async { Html(CLASSIC) }))
        .route("/packs/:pack/*file", get(pack_file))
        .route("/map.js", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], MAP_JS) }))
        .route("/map.css", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], MAP_CSS) }))
        .route("/world.pmtiles", get(world_file))
        .route("/behaviour", get(behaviour_page))
        .route("/api/behaviour", get(behaviour_data))
        .route("/api/templates", post(save_template))
        .route("/api/templates/:name", delete(delete_template))
        .route("/editor", get(editor_page))
        .route("/ws", get(ws))
        .with_state(AppState { out, to_game });
    let listener = tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], a.port))).await?;
    ready(listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
