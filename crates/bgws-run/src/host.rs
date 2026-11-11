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
    let jev = Arc::new(JevClient::from_env(JevConfig { wording: crate::jev_wording(), ..JevConfig::from_figures(figures) }));
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

/// A pack's ground for a scenario: terrain, road lines and rivers in its
/// frame, its sight surface and its terrain appreciation.
pub struct PackGround {
    pub terrain: bgws_env::Terrain,
    pub roads: Vec<bgws_env::RoadLine>,
    pub rivers: Vec<bgws_env::River>,
    pub sight: Option<bgws_env::SightView>,
    pub appreciation: Option<bgws_env::AppreciationView>,
}

/// The scenario's theatre pack, if it names one: terrain assembled over the
/// area its positions cover (with a margin), and the pack's roads there.
/// The host does the disk, the sim stays pure. A pack built around another
/// origin (a box's centre) is re-sampled into the scenario's frame through
/// latitude and longitude, so positions and ground agree. A pack without a
/// terrain appreciation (or with an older one) gets one now, saved with it.
pub fn load_pack(scenario: &ScenarioV4) -> Result<Option<PackGround>> {
    let Some(dir) = &scenario.theatre.pack else { return Ok(None) };
    let path = crate::resolve_pack(dir);
    let mut pack = bgws_env::TheatrePack::open(&path).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?;
    let frame = scenario.frame();
    let pack_frame = bgws_core::EnuFrame::new(pack.meta.frame_origin);
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (_, g) in scenario.positions() {
        let v = pack_frame.geodetic_to_enu(g);
        lo = (lo.0.min(v.e), lo.1.min(v.n));
        hi = (hi.0.max(v.e), hi.1.max(v.n));
    }
    if lo.0 > hi.0 {
        // No positions: the whole pack.
        let s = pack.meta.spec;
        lo = (s.origin_e, s.origin_n);
        hi = (s.origin_e + s.cell * (s.width - 1) as f64, s.origin_n + s.cell * (s.height - 1) as f64);
    }
    // A scenario somewhere else (a pack named but the scenario not moved
    // onto it) has no ground under it: say so, and how to fix it.
    let s = pack.meta.spec;
    let on_pack = |g: bgws_core::Geodetic| {
        let v = pack_frame.geodetic_to_enu(g);
        v.e >= s.origin_e && v.n >= s.origin_n && v.e <= s.origin_e + s.cell * (s.width - 1) as f64 && v.n <= s.origin_n + s.cell * (s.height - 1) as f64
    };
    if !scenario.forces.is_empty() && !scenario.forces.iter().any(|f| on_pack(f.position)) {
        let (lat, lon) = scenario.forces.iter().fold((0.0, 0.0), |a, f| (a.0 + f.position.lat, a.1 + f.position.lon));
        let k = scenario.forces.len() as f64;
        let o = pack.meta.frame_origin;
        return Err(anyhow!(
            "theatre pack {dir} is around {:.3}, {:.3}, but none of the scenario's forces are on it (they are around {:.3}, {:.3}). Move the scenario onto the pack: the editor's \"Move onto the pack\", or the viewer's New game with Ground set to the pack",
            o.lat, o.lon, lat / k, lon / k
        ));
    }
    let m = 5_000.0;
    let aoi = bgws_env::Aoi { min_e: lo.0 - m, min_n: lo.1 - m, max_e: hi.0 + m, max_n: hi.1 + m };
    let mut g = pack_ground(&path, dir, &mut pack, &frame, &aoi)?;
    if g.appreciation.is_none() {
        // One analysis at a time: a game opening while another works it out waits and reads its result.
        static ANALYSING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = ANALYSING.lock().unwrap_or_else(|e| e.into_inner());
        let done = bgws_env::TheatrePack::open(&path)
            .ok()
            .and_then(|p| p.meta.analysis)
            .and_then(|f| bgws_env::Appreciation::read(&path.join(f)).ok())
            .filter(|a| a.version == bgws_env::appreciation::VERSION);
        let a = match done {
            Some(a) => a,
            None => {
                eprintln!("theatre pack {dir}: no terrain analysis yet; working it out once (saved with the pack)");
                let t0 = std::time::Instant::now();
                let a = analyse_pack(&path, &|_, _| {})?;
                eprintln!("theatre pack {dir}: terrain analysis done in {:.0} s", t0.elapsed().as_secs_f64());
                a
            }
        };
        g.appreciation = Some(appreciation_view(std::sync::Arc::new(a), &pack_frame, &frame));
    }
    Ok(Some(g))
}

/// The affine map from a scenario's frame into a pack's: [a, b, c, d, e, f]
/// with pack = (a·e + b·n + c, d·e + e·n + f).
fn to_pack_affine(pack_frame: &bgws_core::EnuFrame, frame: &bgws_core::EnuFrame) -> [f64; 6] {
    let to_pack = |e: f64, n: f64| {
        let v = pack_frame.geodetic_to_enu(frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0)));
        (v.e, v.n)
    };
    let (o, x, y) = (to_pack(0.0, 0.0), to_pack(1_000.0, 0.0), to_pack(0.0, 1_000.0));
    [(x.0 - o.0) / 1_000.0, (y.0 - o.0) / 1_000.0, o.0, (x.1 - o.1) / 1_000.0, (y.1 - o.1) / 1_000.0, o.1]
}

/// A pack's appreciation seen from a scenario's frame.
fn appreciation_view(a: std::sync::Arc<bgws_env::Appreciation>, pack_frame: &bgws_core::EnuFrame, frame: &bgws_core::EnuFrame) -> bgws_env::AppreciationView {
    let key = a
        .key
        .iter()
        .map(|k| {
            let v = frame.geodetic_to_enu(pack_frame.enu_to_geodetic(bgws_core::Enu::new(k.e, k.n, 0.0)));
            bgws_env::KeyTerrain { e: v.e, n: v.n, ..k.clone() }
        })
        .collect();
    bgws_env::AppreciationView { to_pack: to_pack_affine(pack_frame, frame), key, a }
}

/// A pack's ground over an area, in `frame`.
fn pack_ground(path: &std::path::Path, dir: &str, pack: &mut bgws_env::TheatrePack, frame: &bgws_core::EnuFrame, aoi: &bgws_env::Aoi) -> Result<PackGround> {
    let pack_frame = bgws_core::EnuFrame::new(pack.meta.frame_origin);
    let aoi = *aoi;
    let terrain = pack.assemble(&aoi).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?;
    let same = frame.geodetic_to_enu(pack.meta.frame_origin).horizontal() < 1.0;
    let to_scenario = |e: f64, n: f64| {
        if same {
            (e, n)
        } else {
            let v = frame.geodetic_to_enu(pack_frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0)));
            (v.e, v.n)
        }
    };
    let terrain = if same { terrain } else { reframe(&terrain, &pack_frame, frame) };
    let inside = |p: &[f64; 2]| p[0] >= aoi.min_e && p[0] <= aoi.max_e && p[1] >= aoi.min_n && p[1] <= aoi.max_n;
    let mut roads = Vec::new();
    if let Some(rg) = &pack.meta.road_graph {
        let g = bgws_geodata::roads::RoadGraph::read(&path.join(rg)).map_err(|e| anyhow!("theatre pack {dir} roads: {e}"))?;
        for (k, e) in g.edges.iter().enumerate().filter(|(_, e)| e.geom.iter().any(inside)) {
            roads.push(bgws_env::RoadLine {
                class: e.class,
                bridge: e.bridge,
                tunnel: e.tunnel,
                edge: k as u32,
                from: e.from,
                to: e.to,
                line: e.geom.iter().map(|p| to_scenario(p[0], p[1])).collect(),
            });
        }
    }
    let mut rivers = Vec::new();
    if let Some(rf) = &pack.meta.rivers {
        let all: Vec<bgws_env::River> = std::fs::read(path.join(rf)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for r in all.into_iter().filter(|r| r.line.iter().any(|p| inside(&[p.0, p.1]))) {
            rivers.push(bgws_env::River { line: r.line.iter().map(|p| to_scenario(p.0, p.1)).collect(), ..r });
        }
    }
    // Buildings and woods for sight lines: read in the pack's frame, reached
    // from the scenario's by an affine map (a shift and a slight turn).
    let affine = if same { [1.0, 0.0, 0.0, 0.0, 1.0, 0.0] } else { to_pack_affine(&pack_frame, frame) };
    let sight = match &pack.meta.surface {
        Some(f) => {
            let surface = bgws_env::SightSurface::read(&path.join(f)).map_err(|e| anyhow!("theatre pack {dir} sight surface: {e}"))?;
            Some(bgws_env::SightView::new(std::sync::Arc::new(surface), affine))
        }
        None => None,
    };
    // The terrain appreciation, if it is there and current.
    let appreciation = pack
        .meta
        .analysis
        .as_ref()
        .and_then(|f| bgws_env::Appreciation::read(&path.join(f)).ok())
        .filter(|a| a.version == bgws_env::appreciation::VERSION)
        .map(|a| appreciation_view(std::sync::Arc::new(a), &pack_frame, frame));
    Ok(PackGround { terrain, roads, rivers, sight, appreciation })
}

/// Work out a pack's terrain appreciation over the whole of it and save it
/// with the pack: `analysis.bin`, overlays for the viewer (`exposure.png`,
/// `going.png`, over the pack's box) and `keyterrain.json`, and name it in
/// `pack.json`.
pub fn analyse_pack(path: &std::path::Path, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<bgws_env::Appreciation> {
    let dir = path.display().to_string();
    let mut pack = bgws_env::TheatrePack::open(path).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?;
    let frame = bgws_core::EnuFrame::new(pack.meta.frame_origin);
    let s = pack.meta.spec;
    let aoi = bgws_env::Aoi { min_e: s.origin_e, min_n: s.origin_n, max_e: s.origin_e + s.cell * (s.width - 1) as f64, max_n: s.origin_n + s.cell * (s.height - 1) as f64 };
    progress("terrain analysis: loading the pack", 0.0);
    let g = pack_ground(path, &dir, &mut pack, &frame, &aoi)?;
    let figures = crate::default_figures();
    let mut ground = bgws_sim::ground::Ground::from_terrain(g.terrain, 0, &figures).with_roads(&g.roads).with_rivers(&g.rivers);
    if let Some(v) = g.sight {
        ground = ground.with_sight(v);
    }
    let a = bgws_sim::appreciation::analyse(&ground, &bgws_sim::appreciation::AnalyseParams::from_figures(&figures), progress);
    a.write(&path.join("analysis.bin")).map_err(|e| anyhow!("theatre pack {dir}: writing the analysis: {e}"))?;
    write_overlays(path, &pack.meta, &frame, &a)?;
    pack.meta.analysis = Some("analysis.bin".into());
    bgws_env::TheatrePack::write_meta(path, &pack.meta).map_err(|e| anyhow!("theatre pack {dir}: {e:?}"))?;
    Ok(a)
}

/// The viewer's overlays: exposure (dead ground blue, ground open to view red),
/// tracked going (no-go dark, slow-go lighter), on a lon/lat grid over the
/// pack's box, and the key terrain with latitude and longitude.
fn write_overlays(path: &std::path::Path, meta: &bgws_env::PackMeta, frame: &bgws_core::EnuFrame, a: &bgws_env::Appreciation) -> Result<()> {
    let s = meta.spec;
    let corner = |e: f64, n: f64| frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0));
    let [w, so, ea, no] = meta.bbox.unwrap_or_else(|| {
        let (sw, ne) = (corner(s.origin_e, s.origin_n), corner(s.origin_e + s.cell * (s.width - 1) as f64, s.origin_n + s.cell * (s.height - 1) as f64));
        [sw.lon, sw.lat, ne.lon, ne.lat]
    });
    let (pw, ph) = (s.width.min(2_048), ((s.width.min(2_048) as f64) * (s.height as f64 / s.width as f64)).round() as usize);
    let px = |x: usize, y: usize| {
        let (lon, lat) = (w + (ea - w) * (x as f64 + 0.5) / pw as f64, no - (no - so) * (y as f64 + 0.5) / ph as f64);
        let v = frame.geodetic_to_enu(bgws_core::Geodetic::new(lat, lon, 0.0));
        (v.e, v.n)
    };
    let mut exp = vec![[0u8; 4]; pw * ph];
    let mut go = vec![[0u8; 4]; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            let (e, n) = px(x, y);
            if let Some(v) = a.exposure_at(e, n) {
                // Dead ground blue, ground open to view red, clear between.
                exp[y * pw + x] = if v <= 0.1 {
                    [40, 90, 230, (60.0 + 100.0 * (1.0 - v / 0.1)) as u8]
                } else if v >= 0.4 {
                    [230, 70, 50, (40.0 + 110.0 * ((v - 0.4) / 0.4).min(1.0)) as u8]
                } else {
                    [0, 0, 0, 0]
                };
            }
            if let Some((c, _)) = a.mobility_at(0, e, n) {
                go[y * pw + x] = match c {
                    0 => [40, 30, 30, 150],
                    1 => [200, 140, 40, 90],
                    _ => [0, 0, 0, 0],
                };
            }
        }
    }
    // Each file written aside and moved into place, so a reader never sees half of one.
    let swap = |name: &str| std::fs::rename(path.join(format!("{name}.part")), path.join(name));
    bgws_geodata::preview::write_rgba_png(&path.join("exposure.png.part"), pw, ph, &exp)?;
    swap("exposure.png")?;
    bgws_geodata::preview::write_rgba_png(&path.join("going.png.part"), pw, ph, &go)?;
    swap("going.png")?;
    let key: Vec<serde_json::Value> = a
        .key
        .iter()
        .map(|k| {
            let g = frame.enu_to_geodetic(bgws_core::Enu::new(k.e, k.n, 0.0));
            serde_json::json!({ "kind": k.kind, "lat": g.lat, "lon": g.lon, "score": k.score, "classes": k.classes, "note": k.note })
        })
        .collect();
    std::fs::write(path.join("keyterrain.json.part"), serde_json::to_vec_pretty(&serde_json::json!({ "bbox": [w, so, ea, no], "key": key }))?)?;
    swap("keyterrain.json")?;
    Ok(())
}


/// A terrain grid in another ENU frame, sampled through latitude and longitude.
fn reframe(t: &bgws_env::Terrain, from: &bgws_core::EnuFrame, to: &bgws_core::EnuFrame) -> bgws_env::Terrain {
    let s = t.spec;
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (i, j) in [(0, 0), (s.width - 1, 0), (0, s.height - 1), (s.width - 1, s.height - 1)] {
        let (e, n) = s.pos(i, j);
        let v = to.geodetic_to_enu(from.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0)));
        lo = (lo.0.min(v.e), lo.1.min(v.n));
        hi = (hi.0.max(v.e), hi.1.max(v.n));
    }
    let spec = bgws_env::GridSpec::new(((hi.0 - lo.0) / s.cell).ceil() as usize + 1, ((hi.1 - lo.1) / s.cell).ceil() as usize + 1, s.cell, lo.0, lo.1);
    let mut out = bgws_env::Terrain::flat(to.origin, spec, 0.0);
    for j in 0..spec.height {
        for i in 0..spec.width {
            let (e, n) = spec.pos(i, j);
            let v = from.geodetic_to_enu(to.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0)));
            let k = spec.idx(i, j);
            out.elevation.data[k] = t.height(v.e, v.n) as f32;
            let (si, sj) = s.nearest_clamped(v.e, v.n);
            let sk = s.idx(si, sj);
            out.land_cover.data[k] = t.land_cover.data[sk];
            out.soil.data[k] = t.soil.data[sk];
            out.wetness.data[k] = t.wetness.data[sk];
            out.obstacle_gap.data[k] = t.obstacle_gap.data[sk];
        }
    }
    out
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
/// What a real-time game is connected to (docs/design/INTEROP_PLAN.md):
/// `tick` after every second with that second's events (to publish them),
/// `inbound` before each second for what other simulations sent.
pub trait Link {
    fn tick(&mut self, sim: &Sim, events: &[LogEvent]);
    fn inbound(&mut self, _sim: &Sim) -> Vec<bgws_sim::remote::ExternalInput> {
        Vec::new()
    }
}

/// A link that only watches each second (a progress line, a test).
pub struct OnTick<F: FnMut(&Sim)>(pub F);

impl<F: FnMut(&Sim)> Link for OnTick<F> {
    fn tick(&mut self, sim: &Sim, _events: &[LogEvent]) {
        (self.0)(sim)
    }
}

/// A game in an HLA federation: publishes each second and, time-managed,
/// waits there for the federation's grant.
pub struct HlaLinkAdapter<F: FnMut(&Sim)> {
    pub link: bgws_interop::hla::link::HlaLink,
    pub on_tick: F,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

impl<F: FnMut(&Sim)> HlaLinkAdapter<F> {
    pub fn new(link: bgws_interop::hla::link::HlaLink, on_tick: F) -> Self {
        HlaLinkAdapter { link, on_tick, errors: Vec::new(), notes: Vec::new() }
    }
}

impl<F: FnMut(&Sim)> Link for HlaLinkAdapter<F> {
    fn tick(&mut self, sim: &Sim, events: &[LogEvent]) {
        (self.on_tick)(sim);
        if let Err(e) = self.link.tick(sim, events) {
            self.errors.push(e.to_string());
        }
    }
    fn inbound(&mut self, sim: &Sim) -> Vec<bgws_sim::remote::ExternalInput> {
        let inputs = self.link.inbound(sim);
        self.notes.append(&mut self.link.notes);
        inputs
    }
}

/// A game on DIS: publishes each second, takes in what others send, and
/// logs the stand-ins chosen for their types.
pub struct DisLinkAdapter<F: FnMut(&Sim)> {
    pub link: bgws_interop::link::DisLink,
    pub on_tick: F,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

impl<F: FnMut(&Sim)> DisLinkAdapter<F> {
    pub fn new(link: bgws_interop::link::DisLink, on_tick: F) -> Self {
        DisLinkAdapter { link, on_tick, errors: Vec::new(), notes: Vec::new() }
    }
}

impl<F: FnMut(&Sim)> Link for DisLinkAdapter<F> {
    fn tick(&mut self, sim: &Sim, events: &[LogEvent]) {
        (self.on_tick)(sim);
        if let Err(e) = self.link.tick(sim, events) {
            self.errors.push(e.to_string());
        }
    }
    fn inbound(&mut self, sim: &Sim) -> Vec<bgws_sim::remote::ExternalInput> {
        let inputs = self.link.inbound(sim);
        self.notes.extend(self.link.notes());
        inputs
    }
}

pub struct Host {
    /// Events kept for a link while playing in lock step.
    collected: Option<Vec<LogEvent>>,
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
    /// Keep every question handed out (`asked`), for the decision lab.
    pub keep_questions: bool,
    pub asked: Vec<Question>,
    /// A play-out: replaying, this question gets this answer instead of the
    /// recorded one; from there the rules decide and the dice are reseeded.
    force: Option<Forced>,
    /// Real-time play, kept across calls (so a game can be played a second
    /// at a time): answers being worked on, recorded answers waiting for
    /// their time, the next recorded input, the last second shown.
    rt_inflight: Vec<tokio::task::JoinHandle<Vec<Answer>>>,
    rt_recorded: Vec<(SimTime, Answer)>,
    rt_next_external: usize,
    rt_ticked: Option<SimTime>,
}

#[derive(Clone, Debug)]
struct Forced {
    question: bgws_core::QuestionId,
    choice: String,
    reseed: Option<u64>,
    done: bool,
}

impl Host {
    pub fn new(scenario: &ScenarioV4, seed: Seed, figures: Figures, opts: &HostOptions) -> Result<Host> {
        let router = router_for(scenario, &figures, opts);
        let defaults = crate::default_figures();
        let overrides = figures.iter().filter(|f| defaults.figure(&f.key).is_some_and(|d| d.value != f.value)).map(|f| (f.key.clone(), f.value)).collect();
        let mut sim_opts = opts.sim.clone();
        if sim_opts.terrain.is_none() {
            if let Some(g) = load_pack(scenario)? {
                sim_opts.terrain = Some(g.terrain);
                sim_opts.roads = Some(g.roads);
                sim_opts.rivers = Some(g.rivers);
                sim_opts.sight = g.sight;
                sim_opts.appreciation = g.appreciation;
            }
        }
        let sim = Sim::with_options(scenario, seed, figures, sim_opts).map_err(|e| anyhow!("{e}"))?;
        let mut machine = MachineInfo::compiled_for();
        machine.threads = Some(rayon::current_num_threads() as u32);
        let mut record = GameRecord::new(scenario.clone(), seed, BuildInfo::this(option_env!("BGWS_BUILD_ID").unwrap_or("dev")), machine);
        record.figure_overrides = overrides;
        record.run = RunSetup { flat_ground: opts.sim.flat_ground, plan_at_start: opts.sim.plan_at_start, realtime: false, lockstep: false };
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        Ok(Host { collected: None, sim, router, record, rt, events_out: None, replay: None, next_command: 0, questions: 0, by_source: Default::default(), cost: 0.0, keep_questions: false, asked: Vec::new(), force: None, rt_inflight: Vec::new(), rt_recorded: Vec::new(), rt_next_external: 0, rt_ticked: None })
    }

    /// Replay a recorded game without calling any decider.
    pub fn replay(record: GameRecord, figures: Figures) -> Result<Host> {
        Host::replay_with(record, figures, SimOptions::default())
    }

    /// Replay with ground already loaded (`sim`'s terrain, roads, sight…), so
    /// many replays of one game on a pack load it once.
    pub fn replay_with(record: GameRecord, figures: Figures, sim: SimOptions) -> Result<Host> {
        let mut f = figures;
        for (k, v) in &record.figure_overrides {
            f.set(k, *v).map_err(|e| anyhow!("{e}"))?;
        }
        let sim = SimOptions { flat_ground: record.run.flat_ground, plan_at_start: record.run.plan_at_start, ..sim };
        let mut h = Host::new(&record.scenario, record.seed, f, &HostOptions { deciders: DeciderChoice::Rules, sim, ..Default::default() })?;
        h.replay = Some(record);
        Ok(h)
    }

    /// Make this replay a play-out: when `question` comes up it is answered
    /// `choice` instead of what the record says; from then on the rules
    /// answer everything, and with `reseed` the dice are drawn from a new
    /// seed (so play-outs of one choice differ, as games do).
    pub fn force_answer(&mut self, question: bgws_core::QuestionId, choice: &str, reseed: Option<u64>) {
        self.force = Some(Forced { question, choice: choice.to_string(), reseed, done: false });
    }

    /// Whether a forced answer has been given yet.
    pub fn forced(&self) -> bool {
        self.force.as_ref().is_some_and(|f| f.done)
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

    /// Play to `until` in real time (docs/design/INTEROP_PLAN.md §3.4): the
    /// clock does not stop for decisions. Questions go to the deciders in the
    /// background; a unit with a question out carries on, and each answer is
    /// given at the first second boundary after it comes back (it applies no
    /// sooner than the crew's reaction time after the question). With `pace`
    /// (sim seconds per wall second), each second waits for the wall clock;
    /// without it the game runs as fast as it can (a replay, a test).
    ///
    /// `link` sees the sim and its events after every second (to publish
    /// them), and before each second hands over what other simulations sent;
    /// each input is applied and recorded at that second boundary. While the
    /// exercise is stopped (a DIS Stop/Freeze), a paced game holds its clock.
    ///
    /// Replaying a real-time record, the recorded answers and inputs are
    /// given at their recorded times, with no network: the replay is exact.
    pub fn play_realtime(&mut self, until: SimTime, pace: Option<f64>, link: &mut dyn Link) -> Result<Option<bgws_sim::Outcome>> {
        self.record.run.realtime = true;
        let (mut wall0, mut sim0) = (std::time::Instant::now(), self.sim.now());
        let mut inflight = std::mem::take(&mut self.rt_inflight);
        let mut recorded = std::mem::take(&mut self.rt_recorded);
        let mut next_external = self.rt_next_external;
        let mut events: Vec<LogEvent> = Vec::new();
        // The last whole second shown to `link` (a second ends either
        // advanced or with questions to hand out; each is shown once).
        let mut ticked: Option<SimTime> = self.rt_ticked;
        let tick = |sim: &Sim, ticked: &mut Option<SimTime>, events: &mut Vec<LogEvent>, link: &mut dyn Link| {
            let now = sim.now();
            if (now.secs().fract() == 0.0 || now >= until) && *ticked != Some(now) {
                *ticked = Some(now);
                link.tick(sim, events);
                events.clear();
            }
        };
        loop {
            self.apply_recorded()?;
            let now = self.sim.now();
            // Inputs from other simulations, at this boundary.
            if let Some(rec) = &self.replay {
                let due: Vec<_> = rec.external.iter().skip(next_external).take_while(|x| x.at <= now).cloned().collect();
                next_external += due.len();
                for x in due {
                    let input: bgws_sim::remote::ExternalInput = serde_json::from_value(x.input.clone()).map_err(|e| anyhow!("a recorded input does not read: {e}"))?;
                    self.record.push_external(now, x.input);
                    self.sim.apply_external(input);
                }
            } else {
                for input in link.inbound(&self.sim) {
                    self.record.push_external(now, serde_json::to_value(&input)?);
                    self.sim.apply_external(input);
                }
            }
            // Stopped: hold the clock (live and paced only), listening.
            if self.sim.ctx.frozen && self.replay.is_none() && pace.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(100));
                (wall0, sim0) = (std::time::Instant::now(), self.sim.now());
                continue;
            }
            // Answers back since the last stop are given now.
            if self.replay.is_some() {
                let (ready, later): (Vec<_>, Vec<_>) = std::mem::take(&mut recorded).into_iter().partition(|(at, _)| *at <= now);
                recorded = later;
                if !ready.is_empty() {
                    self.submit(now, ready.into_iter().map(|(_, a)| a).collect())?;
                }
            } else {
                let (done, running): (Vec<_>, Vec<_>) = std::mem::take(&mut inflight).into_iter().partition(|h| h.is_finished());
                inflight = running;
                for h in done {
                    let answers = self.rt.block_on(h).map_err(|e| anyhow!("a decider failed: {e}"))?;
                    self.submit(now, answers)?;
                }
            }
            let next = SimTime::from_secs(now.secs().floor() + 1.0).min(until);
            let stop = self.next_command_at().filter(|t| *t < next).unwrap_or(next);
            let step = self.sim.advance_nonblocking(stop);
            events.extend(self.flush_events()?);
            match step {
                Step::Over(o) => {
                    self.record.outcome = Some(serde_json::to_value(&o)?);
                    self.record.fingerprint = Some(self.sim.fingerprint());
                    tick(&self.sim, &mut ticked, &mut events, link);
                    (self.rt_inflight, self.rt_recorded, self.rt_next_external, self.rt_ticked) = (inflight, recorded, next_external, ticked);
                    return Ok(Some(o));
                }
                Step::NeedsAnswers { questions, .. } => {
                    tick(&self.sim, &mut ticked, &mut events, link);
                    self.questions += questions.len();
                    if self.keep_questions {
                        self.asked.extend(questions.iter().cloned());
                    }
                    if let Some(rec) = &self.replay {
                        // Recorded answers wait for their time; one never given stays unanswered.
                        for q in &questions {
                            if let Some(a) = rec.answers.iter().find(|a| a.answer.question == q.id) {
                                recorded.push((a.at, a.answer.clone()));
                            }
                        }
                    } else {
                        let router = self.router.clone();
                        inflight.push(self.rt.spawn(async move { router.answer(questions).await }));
                    }
                }
                Step::Advanced => {
                    let now = self.sim.now();
                    tick(&self.sim, &mut ticked, &mut events, link);
                    if now >= until {
                        (self.rt_inflight, self.rt_recorded, self.rt_next_external, self.rt_ticked) = (inflight, recorded, next_external, ticked);
                        return Ok(None);
                    }
                    if let Some(scale) = pace.filter(|s| *s > 0.0) {
                        let target = wall0 + std::time::Duration::from_secs_f64((now.secs() - sim0.secs()) / scale);
                        let wait = target.saturating_duration_since(std::time::Instant::now());
                        if !wait.is_zero() {
                            std::thread::sleep(wait);
                        }
                    }
                }
            }
        }
    }

    /// Advance to `until`, answering every question on the way. `on_step`
    /// sees the sim after each stop (for a UI or progress).
    /// Play in lock step with a time-managed federation (HLA): the clock
    /// waits for deciders as in [`Host::play_until`]; at each whole second
    /// the link is shown the second's events (it sends them and waits for
    /// the federation to grant the next second) and what it took in applies
    /// at that boundary. Inputs are recorded, so a replay needs no
    /// federation; with every federate in lock step, the same seeds give the
    /// same game.
    pub fn play_lockstep(&mut self, until: SimTime, link: &mut dyn Link) -> Result<Option<bgws_sim::Outcome>> {
        self.record.run.lockstep = true;
        let mut next_external = self.rt_next_external;
        loop {
            let now = self.sim.now();
            if let Some(rec) = &self.replay {
                let due: Vec<_> = rec.external.iter().skip(next_external).take_while(|x| x.at <= now).cloned().collect();
                next_external += due.len();
                for x in due {
                    let input: bgws_sim::remote::ExternalInput = serde_json::from_value(x.input.clone()).map_err(|e| anyhow!("a recorded input does not read: {e}"))?;
                    self.record.push_external(now, x.input);
                    self.sim.apply_external(input);
                }
            } else {
                for input in link.inbound(&self.sim) {
                    self.record.push_external(now, serde_json::to_value(&input)?);
                    self.sim.apply_external(input);
                }
            }
            if now >= until {
                self.rt_next_external = next_external;
                return Ok(None);
            }
            let next = SimTime::from_secs(now.secs().floor() + 1.0).min(until);
            self.collected = Some(Vec::new());
            let r = self.play_until(next, |_| {});
            let events = self.collected.take().unwrap_or_default();
            let r = r?;
            link.tick(&self.sim, &events);
            if r.is_some() {
                self.rt_next_external = next_external;
                return Ok(r);
            }
        }
    }

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
                    // A record saved while questions were out (before Play, or while a
                    // person was deciding) ends here: those were never answered.
                    if let Some(rec) = &self.replay {
                        let unanswered = questions.iter().any(|q| !rec.answers.iter().any(|a| a.answer.question == q.id));
                        if unanswered && rec.ended_at.is_some_and(|e| at >= e) {
                            return Ok(None);
                        }
                    }
                    self.questions += questions.len();
                    if self.keep_questions {
                        self.asked.extend(questions.iter().cloned());
                    }
                    let forced = self.force.as_ref().filter(|f| !f.done).and_then(|f| questions.iter().position(|q| q.id == f.question));
                    let mut answers = self.answer(questions.clone())?;
                    if let (Some(i), Some(f)) = (forced, self.force.as_mut()) {
                        if !questions[i].has_option(&f.choice) {
                            bail!("{} is not an option of question {}", f.choice, f.question.0);
                        }
                        // The answer given, kept as it was when it is the one forced.
                        if answers[i].choice != f.choice {
                            answers[i] = Answer::simple(&questions[i], f.choice.clone(), bgws_core::AnswerSource::Rules);
                            answers[i].rationale = Some("forced for a play-out".into());
                        }
                        f.done = true;
                        let reseed = f.reseed;
                        // From here on the game is new: the rules decide (a replay host's
                        // router is the rules', set up as the game was), and fresh dice.
                        self.stop_replay();
                        if let Some(k) = reseed {
                            self.sim.ctx.seed = Seed(bgws_core::rng::fnv1a(format!("{}:{k}", self.sim.ctx.seed.0).as_bytes()));
                        }
                    }
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

    fn flush_events(&mut self) -> Result<Vec<LogEvent>> {
        let events: Vec<LogEvent> = self.sim.drain_events();
        if let Some(w) = &mut self.events_out {
            w.write(&events)?;
        }
        if let Some(c) = &mut self.collected {
            c.extend(events.iter().cloned());
        }
        Ok(events)
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
