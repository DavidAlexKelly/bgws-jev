//! Scenario version 4 (RUST_ENGINE.md §14.3): "the current scenario JSON
//! (forces, scope, world setup, templates, victory, events) with additions
//! for the new physics (emitter settings, weather source, theatre pack)".
//!
//! What changes from version 3:
//! - **One force list with stable ids.** Land units and world entities are
//!   both `Force`s (a `layer` remembers which they were in the TS build);
//!   every position is WGS84 `Geodetic` (lat, lon, height above the
//!   ellipsoid), never a local metre map.
//! - **The theatre:** an origin for local ENU frames, bounds every position
//!   must sit in, and an optional theatre-pack path (§14.2).
//! - **Time and weather:** a UTC start date-time (for the sun and moon,
//!   §7.4) and a weather source — a preset, the synthetic generator, or an
//!   ERA5 file (§14.2).
//! - **Emitter settings:** each side's default EMCON and per-force overrides
//!   (the hard gate of rule 5).
//! - **Deciders:** per side, who leads units and who plans for HQs: the
//!   rules, Jev, Claude, a local model, or a person (§10).
//! - **Victory** gains the TS rules' breakpoint as an explicit condition.

use crate::scope::{Scope, Weather};
use bgws_catalogue::{Echelon, Planner};
use bgws_core::{Geodetic, Seed, Side};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const V4_FORMAT: &str = "bgws.scenario";
pub const V4_VERSION: u32 = 4;

/// Troop quality (forceList `TROOP_QUALITY`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TroopQuality {
    Conscript,
    #[default]
    Regular,
    Veteran,
    Elite,
}

impl TroopQuality {
    /// The 1–8 scale (conscript 1, regular 4, veteran 6, elite 8).
    pub fn rating(self) -> u8 {
        match self {
            TroopQuality::Conscript => 1,
            TroopQuality::Regular => 4,
            TroopQuality::Veteran => 6,
            TroopQuality::Elite => 8,
        }
    }
}

/// Which TS layer a force came from: a land unit on the board, or a world entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Layer {
    #[default]
    Unit,
    Entity,
}

/// Emission control: a hard gate set in orders (W6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Emcon {
    Silent,
    Limited,
    #[default]
    Free,
}

/// Who decides (§10): the rules, Jev, Claude, a local model, or a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Decider {
    #[default]
    Rules,
    Jev,
    Claude,
    Local,
    Person,
}

/// A satellite's place in its orbit at t = 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrbitPlacement {
    #[serde(default)]
    pub phase_deg: f64,
    #[serde(default)]
    pub raan_deg: f64,
}

/// What an entity is doing at the start (world/types.ts `EntityTask`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EntityTask {
    Idle,
    Route {
        waypoints: Vec<Geodetic>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt_m: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        then: Option<Box<EntityTask>>,
    },
    Orbit {
        at: Geodetic,
        radius_m: f64,
        alt_m: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<f64>,
    },
    Strike {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
        aim_at: Geodetic,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt_m: Option<f64>,
    },
    Intercept {
        target_id: String,
    },
    ReturnToBase {
        base_id: String,
    },
    Station {
        at: Geodetic,
        radius_m: f64,
    },
    Landed {
        base_id: String,
        ready_at: f64,
    },
    LoiterSearch {
        at: Geodetic,
        radius_m: f64,
        until: f64,
    },
    Deliver {
        to_unit: String,
        cargo: Value,
        via: Vec<Geodetic>,
    },
    Reload {
        port_id: String,
        ready_at: f64,
    },
}

impl EntityTask {
    /// Every force id the task refers to.
    pub fn references(&self) -> Vec<&str> {
        match self {
            EntityTask::Strike { target_id: Some(t), .. } | EntityTask::Intercept { target_id: t } => vec![t.as_str()],
            EntityTask::ReturnToBase { base_id } | EntityTask::Landed { base_id, .. } => vec![base_id.as_str()],
            EntityTask::Deliver { to_unit, .. } => vec![to_unit.as_str()],
            EntityTask::Reload { port_id, .. } => vec![port_id.as_str()],
            EntityTask::Route { then: Some(t), .. } => t.references(),
            _ => vec![],
        }
    }
    /// Every position in the task.
    pub fn positions(&self) -> Vec<Geodetic> {
        match self {
            EntityTask::Route { waypoints, then, .. } => {
                let mut v = waypoints.clone();
                if let Some(t) = then {
                    v.extend(t.positions());
                }
                v
            }
            EntityTask::Orbit { at, .. } | EntityTask::Station { at, .. } | EntityTask::LoiterSearch { at, .. } => vec![*at],
            EntityTask::Strike { aim_at, .. } => vec![*aim_at],
            EntityTask::Deliver { via, .. } => via.clone(),
            _ => vec![],
        }
    }
}

/// One platform or element on the map, with a stable id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Force {
    pub id: String,
    pub label: String,
    pub side: Side,
    /// A catalogue record id or alias.
    pub platform: String,
    /// Platforms in the element.
    pub count: u32,
    #[serde(default)]
    pub quality: TroopQuality,
    /// WGS84; `h` is height above the ellipsoid (negative under water).
    pub position: Geodetic,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<f64>,
    #[serde(default)]
    pub layer: Layer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echelon: Option<Echelon>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub concealed: bool,
    /// A headquarters, and how many subordinates it can activate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_rating: Option<f64>,
    /// A decoy.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dummy: bool,
    /// Based at or embarked on another force (a ship's port, an aircraft's base).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<EntityTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<OrbitPlacement>,
    /// "military" or "commercial" (satellites).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Neutral shipping: its lane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lane: Vec<Geodetic>,
    /// Starting status ("active", "landed"…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Owner of an obstacle or measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObstacleKind {
    Minefield,
    Ditch,
    DragonsTeeth,
    SeaMines,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Obstacle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub owner: Side,
    pub kind: ObstacleKind,
    /// Centre of the rectangle.
    pub at: Geodetic,
    pub width_m: f64,
    pub depth_m: f64,
    #[serde(default)]
    pub bearing_deg: f64,
    /// Mines per metre of front.
    #[serde(default = "half")]
    pub density: f64,
}

fn half() -> f64 {
    0.5
}

/// One HQ in the command tree (W4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hq {
    pub id: String,
    pub side: Side,
    pub echelon: Echelon,
    /// The force it sits in.
    pub post_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub subordinates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<Planner>,
    /// Who plans for it; absent: the side's HQ decider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decider: Option<Decider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AltBand {
    pub min: f64,
    pub max: f64,
}

/// Control measures with real geometry (W4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlMeasure {
    pub id: String,
    /// phaseLine, boundary, objective, nai, tai, fscl, cfl, nfa, rfa, roz, corridor, killBox.
    pub kind: String,
    pub side: Side,
    pub label: String,
    pub points: Vec<Geodetic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_m: Option<AltBand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_until: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub from: f64,
    pub until: f64,
}

/// A prepared cyber access (W9).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CyberAccess {
    pub id: String,
    pub side: Side,
    /// bearer, logistics, airDefence, public.
    pub target: String,
    pub system: String,
    pub readiness: f64,
    pub p_effect: f64,
    pub duration_s: f64,
    pub detection_risk: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
}

/// How a side wins (§5.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VictoryCondition {
    AirSuperiority {
        side: Side,
        at: Geodetic,
        radius_m: f64,
        hold_s: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    TargetSet {
        side: Side,
        targets: Vec<String>,
        share: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    SeaControl {
        side: Side,
        at: Geodetic,
        radius_m: f64,
        hold_s: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Through {
        side: Side,
        ids: Vec<String>,
        at: Geodetic,
        radius_m: f64,
        count: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    All {
        side: Side,
        of: Vec<VictoryCondition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A side breaks when it has lost this share of its combat strength
    /// (the TS `rules.breakpointPct`); `side` absent applies to both.
    Breakpoint {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
        pct: f64,
    },
}

impl VictoryCondition {
    pub fn references(&self) -> Vec<&str> {
        match self {
            VictoryCondition::TargetSet { targets, .. } => targets.iter().map(String::as_str).collect(),
            VictoryCondition::Through { ids, .. } => ids.iter().map(String::as_str).collect(),
            VictoryCondition::All { of, .. } => of.iter().flat_map(|c| c.references()).collect(),
            _ => vec![],
        }
    }
    pub fn positions(&self) -> Vec<Geodetic> {
        match self {
            VictoryCondition::AirSuperiority { at, .. } | VictoryCondition::SeaControl { at, .. } | VictoryCondition::Through { at, .. } => vec![*at],
            VictoryCondition::All { of, .. } => of.iter().flat_map(|c| c.positions()).collect(),
            _ => vec![],
        }
    }
}

/// A scripted event at a set time (W9).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptedEvent {
    /// Seconds after the start.
    pub at: f64,
    /// cutCommercialImagery, cutLeoSatcom, jamSatcom, jamGnss, dazzleEo, asat, restore.
    pub kind: String,
    /// The side that suffers it.
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedArea {
    pub side: Side,
    pub label: String,
    pub at: Geodetic,
    pub priority: f64,
}

/// Water: polygons of sea; with none, all is open ocean (W8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SeaArea {
    pub waters: Vec<Vec<Geodetic>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Network {
    Voice,
    Datalink,
}

/// The world beyond the forces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorldSetup {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obstacles: Vec<Obstacle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<ControlMeasure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cyber: Vec<CyberAccess>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nais: Vec<NamedArea>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea: Option<SeaArea>,
    /// Each side's kill-web network; data link by default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub network: BTreeMap<Side, Network>,
}

/// Overrides of the environment at the start (world/types.ts `Environment`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_km: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_base_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_cover: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precipitation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea_state: Option<f64>,
}

/// Where the weather comes from (§7.3, §14.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WeatherSource {
    /// A fixed preset, with optional overrides.
    Preset { weather: Weather },
    /// The synthetic generator from monthly climatology.
    Synthetic {
        /// A climatology id ("baltic", "red-sea"…).
        climate: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
    },
    /// An ERA5 extract for the date and area (a file path).
    Era5 { path: String },
}

/// The theatre: a local-frame origin, bounds and an optional theatre pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theatre {
    /// The origin of the scenario's ENU frame (and of every converted v3 offset).
    pub origin: Geodetic,
    /// Every position must sit inside, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    /// A theatre pack folder (§14.2), relative to the scenario or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<String>,
    /// Ground when there is no pack: "generated", "raster+relief", "raster" (v3 `ground.choice`).
    #[serde(default = "generated")]
    pub ground: String,
    /// Seed for generated ground.
    #[serde(default)]
    pub ground_seed: String,
}

fn generated() -> String {
    "generated".into()
}

/// A lat/lon box, degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl Bounds {
    pub fn contains(&self, g: Geodetic) -> bool {
        g.lat >= self.south && g.lat <= self.north && g.lon >= self.west && g.lon <= self.east
    }
    /// The smallest box holding these points.
    pub fn around(points: impl IntoIterator<Item = Geodetic>) -> Option<Bounds> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut b = Bounds { south: first.lat, west: first.lon, north: first.lat, east: first.lon };
        for p in it {
            b.south = b.south.min(p.lat);
            b.north = b.north.max(p.lat);
            b.west = b.west.min(p.lon);
            b.east = b.east.max(p.lon);
        }
        Some(b)
    }
    pub fn union(self, o: Bounds) -> Bounds {
        Bounds { south: self.south.min(o.south), west: self.west.min(o.west), north: self.north.max(o.north), east: self.east.max(o.east) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmitterSettings {
    /// Each side's EMCON at the start.
    #[serde(default)]
    pub default: BTreeMap<Side, Emcon>,
    /// Per-force overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub forces: BTreeMap<String, Emcon>,
}

impl EmitterSettings {
    pub fn emcon_of(&self, force: &Force) -> Emcon {
        self.forces.get(&force.id).copied().or_else(|| self.default.get(&force.side).copied()).unwrap_or_default()
    }
}

/// Who decides for a side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SideCommand {
    /// Units' decision points.
    pub units: Decider,
    /// HQ plans.
    pub hqs: Decider,
    /// Commander's directive, in words.
    #[serde(default)]
    pub directive: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub sides: BTreeMap<Side, SideCommand>,
    /// The Claude model for planners.
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hqs: Vec<Hq>,
}

impl Command {
    pub fn side(&self, s: Side) -> SideCommand {
        self.sides.get(&s).cloned().unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CommsRule {
    #[default]
    Radio,
    Perfect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Behaviours {
    /// Force id → template id. Forces not listed use their record's template.
    #[serde(default)]
    pub assign: BTreeMap<String, String>,
    /// The user templates `assign` uses, with their parents (opaque JSON, `bgws-behaviour` reads them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Value>,
}

/// Where a converted scenario came from, and the assumptions made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub from_format: String,
    pub from_version: u32,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioV4 {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub saved_at: String,
    pub seed: Seed,
    /// The seed as written, when it was text (v3 seeds are strings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_text: Option<String>,
    pub theatre: Theatre,
    pub scope: Scope,
    /// Start date-time, UTC, RFC 3339 ("2026-06-01T03:16:00Z"): the sun and moon follow from it.
    pub start_utc: String,
    pub weather: WeatherSource,
    #[serde(default)]
    pub environment: EnvironmentSpec,
    #[serde(default)]
    pub comms: CommsRule,
    pub forces: Vec<Force>,
    #[serde(default)]
    pub world: WorldSetup,
    #[serde(default)]
    pub command: Command,
    #[serde(default)]
    pub behaviours: Behaviours,
    #[serde(default)]
    pub emitters: EmitterSettings,
    #[serde(default)]
    pub victory: Vec<VictoryCondition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ScriptedEvent>,
    /// What a reader should know before trusting a result (force-list caveats).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub converted_from: Option<Provenance>,
}

impl ScenarioV4 {
    pub fn force(&self, id: &str) -> Option<&Force> {
        self.forces.iter().find(|f| f.id == id)
    }
    pub fn forces_of(&self, side: Side) -> impl Iterator<Item = &Force> {
        self.forces.iter().filter(move |f| f.side == side)
    }
    /// The ENU frame at the theatre origin.
    pub fn frame(&self) -> bgws_core::EnuFrame {
        bgws_core::EnuFrame::new(self.theatre.origin)
    }
    /// The start as Unix seconds (UTC).
    pub fn start_unix_s(&self) -> Result<i64, crate::time::TimeError> {
        crate::time::parse_utc(&self.start_utc)
    }
    /// The breakpoint for a side, if the victory conditions give one.
    pub fn breakpoint_pct(&self, side: Side) -> Option<f64> {
        self.victory.iter().find_map(|v| match v {
            VictoryCondition::Breakpoint { side: s, pct } if s.is_none() || *s == Some(side) => Some(*pct),
            _ => None,
        })
    }
    /// Every position in the scenario, with what it belongs to.
    pub fn positions(&self) -> Vec<(String, Geodetic)> {
        let mut v = Vec::new();
        for f in &self.forces {
            v.push((f.id.clone(), f.position));
            if let Some(t) = &f.task {
                v.extend(t.positions().into_iter().map(|p| (format!("{}.task", f.id), p)));
            }
            v.extend(f.lane.iter().map(|p| (format!("{}.lane", f.id), *p)));
        }
        for (i, o) in self.world.obstacles.iter().enumerate() {
            v.push((o.id.clone().unwrap_or_else(|| format!("obstacles[{i}]")), o.at));
        }
        for m in &self.world.measures {
            v.extend(m.points.iter().map(|p| (m.id.clone(), *p)));
        }
        for (i, n) in self.world.nais.iter().enumerate() {
            v.push((format!("nais[{i}]"), n.at));
        }
        for (i, c) in self.victory.iter().enumerate() {
            v.extend(c.positions().into_iter().map(|p| (format!("victory[{i}]"), p)));
        }
        v
    }
}
