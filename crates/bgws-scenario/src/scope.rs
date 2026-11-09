//! A scenario's scope (RUST_ENGINE.md §14.3; ports realtime/scope.ts and
//! MULTI_DOMAIN_PLAN.md §5): how long it runs, how big it is, which command
//! levels exist, which domains are simulated, abstracted or off, which
//! planners exist, where detail is aggregated, and the time dimension.
//!
//! A scope is chosen first, from a scenario type (§5.3), then edited at
//! setup. `effective_scope` turns any scope into one the engine can run,
//! with notes on what changed; a scope that needs something the engine
//! cannot do is refused with reasons, never quietly downgraded.

use bgws_catalogue::{Echelon, Planner};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// The domains a scope switches (scope.ts `DOMAINS`), in the TS order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopeDomain {
    Land,
    Drones,
    Fires,
    Ew,
    Air,
    AirDefence,
    Naval,
    Space,
    Cyber,
    Logistics,
    Medical,
}

impl ScopeDomain {
    pub const ALL: [ScopeDomain; 11] = [
        ScopeDomain::Land,
        ScopeDomain::Drones,
        ScopeDomain::Fires,
        ScopeDomain::Ew,
        ScopeDomain::Air,
        ScopeDomain::AirDefence,
        ScopeDomain::Naval,
        ScopeDomain::Space,
        ScopeDomain::Cyber,
        ScopeDomain::Logistics,
        ScopeDomain::Medical,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ScopeDomain::Land => "land",
            ScopeDomain::Drones => "drones",
            ScopeDomain::Fires => "fires",
            ScopeDomain::Ew => "ew",
            ScopeDomain::Air => "air",
            ScopeDomain::AirDefence => "airDefence",
            ScopeDomain::Naval => "naval",
            ScopeDomain::Space => "space",
            ScopeDomain::Cyber => "cyber",
            ScopeDomain::Logistics => "logistics",
            ScopeDomain::Medical => "medical",
        }
    }

    /// The label for the setup screen (scope.ts `DOMAIN_LABEL`).
    pub fn label(self) -> &'static str {
        match self {
            ScopeDomain::Land => "Land",
            ScopeDomain::Drones => "Drones",
            ScopeDomain::Fires => "Fires (artillery)",
            ScopeDomain::Ew => "Electronic warfare",
            ScopeDomain::Air => "Air",
            ScopeDomain::AirDefence => "Air defence",
            ScopeDomain::Naval => "Naval",
            ScopeDomain::Space => "Space",
            ScopeDomain::Cyber => "Cyber",
            ScopeDomain::Logistics => "Logistics",
            ScopeDomain::Medical => "Medical",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DomainMode {
    Simulated,
    Abstracted,
    Off,
}

impl DomainMode {
    pub fn key(self) -> &'static str {
        match self {
            DomainMode::Simulated => "simulated",
            DomainMode::Abstracted => "abstracted",
            DomainMode::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScenarioType {
    Skirmish,
    CombinedArms,
    DroneLine,
    AirCampaign,
    SustainedOffensive,
    Maritime,
    Strike,
    Joint,
    Custom,
}

impl ScenarioType {
    pub const ALL: [ScenarioType; 9] = [
        ScenarioType::Skirmish,
        ScenarioType::CombinedArms,
        ScenarioType::DroneLine,
        ScenarioType::AirCampaign,
        ScenarioType::SustainedOffensive,
        ScenarioType::Maritime,
        ScenarioType::Strike,
        ScenarioType::Joint,
        ScenarioType::Custom,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Weather {
    Clear,
    Overcast,
    Rain,
    Fog,
    Snow,
    Storm,
}

/// Which units Jev leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JevLeads {
    All,
    NearContact,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub width_km: f64,
    pub depth_km: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EchelonRange {
    pub highest: Echelon,
    pub lowest: Echelon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub aggregate_away_from_contact: bool,
    pub jev: JevLeads,
}

/// Start hour, weather, clock speed and quiet skipping (§5.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeTime {
    /// Hour of day at the start, 0–24 (local).
    pub start_hour: f64,
    pub weather: Weather,
    /// Fastest the clock may run, × real time.
    pub max_compression: f64,
    pub skip_quiet: bool,
}

/// scope.ts `DEFAULT_TIME`.
pub const DEFAULT_TIME: ScopeTime = ScopeTime { start_hour: 10.0, weather: Weather::Clear, max_compression: 60.0, skip_quiet: false };

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    #[serde(rename = "type")]
    pub kind: ScenarioType,
    /// The time limit, game minutes.
    pub duration_min: f64,
    pub area: Area,
    pub echelons: EchelonRange,
    pub domains: BTreeMap<ScopeDomain, DomainMode>,
    pub planners: Vec<Planner>,
    pub detail: Detail,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<ScopeTime>,
}

impl Scope {
    /// A domain's mode; a domain missing from the map is off.
    pub fn mode(&self, d: ScopeDomain) -> DomainMode {
        self.domains.get(&d).copied().unwrap_or(DomainMode::Off)
    }
    pub fn set(&mut self, d: ScopeDomain, m: DomainMode) {
        self.domains.insert(d, m);
    }
    /// The scope's time, or the default; a game of days may run the clock
    /// at up to ten minutes a second (scope.ts `timeOf`).
    pub fn time_or_default(&self) -> ScopeTime {
        if let Some(t) = self.time {
            return t;
        }
        let mut t = DEFAULT_TIME;
        if self.duration_min > 24.0 * 60.0 {
            t.max_compression = 600.0;
        }
        t
    }
    /// Does the scope ask for other than the default light and weather?
    pub fn changes_environment(&self) -> bool {
        self.time.is_some_and(|t| t.start_hour != DEFAULT_TIME.start_hour || t.weather != DEFAULT_TIME.weather)
    }
}

/// A scenario type's chooser entry and default scope (scope.ts `SCENARIO_TYPE_INFO`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioTypeInfo {
    pub name: &'static str,
    pub blurb: &'static str,
    pub milestone: &'static str,
    pub victory: &'static str,
}

fn domains(simulated: &[ScopeDomain], abstracted: &[ScopeDomain]) -> BTreeMap<ScopeDomain, DomainMode> {
    ScopeDomain::ALL
        .iter()
        .map(|d| {
            let m = if simulated.contains(d) {
                DomainMode::Simulated
            } else if abstracted.contains(d) {
                DomainMode::Abstracted
            } else {
                DomainMode::Off
            };
            (*d, m)
        })
        .collect()
}

pub fn type_info(t: ScenarioType) -> ScenarioTypeInfo {
    let (name, blurb, milestone, victory) = match t {
        ScenarioType::Skirmish => ("Skirmish", "Platoons and companies, a few kilometres, up to two hours.", "M1", "Breakpoint or objective"),
        ScenarioType::CombinedArms => ("Combined-arms battle", "Company to brigade, with drones, EW and fires simulated.", "M3", "Objectives by time"),
        ScenarioType::DroneLine => ("Drone line", "A positional front: drones, EW, artillery and infantry over days.", "M4", "Objectives by time"),
        ScenarioType::AirCampaign => ("Air campaign", "Flights, air defence and missiles over hundreds of kilometres.", "M5", "Air superiority, or a target set destroyed"),
        ScenarioType::SustainedOffensive => ("Sustained offensive", "Battalion to division over days, with logistics and medical.", "M6", "Objectives by time"),
        ScenarioType::Maritime => ("Maritime / littoral", "Ships, coastal batteries and drones at sea, with neutral shipping.", "M7", "Sea control, or ships and cargo through"),
        ScenarioType::Strike => ("Strike and missile defence", "Missiles, air defence and space across a theatre.", "M8", "Operational aims"),
        ScenarioType::Joint => ("Joint operation", "Every domain, platoon to joint HQ, over weeks.", "M8", "The overall aims"),
        ScenarioType::Custom => ("Custom", "Start from the skirmish and set each part yourself.", "—", "Breakpoint or objective"),
    };
    ScenarioTypeInfo { name, blurb, milestone, victory }
}

/// The scope a scenario type starts with (scope.ts `scopeFor`).
pub fn scope_for(t: ScenarioType) -> Scope {
    use Echelon as E;
    use Planner as P;
    use ScopeDomain::*;
    let (duration, (w, d), (hi, lo), doms, planners, (agg, jev)) = match t {
        ScenarioType::Skirmish => (90.0, (10.0, 10.0), (E::Company, E::Platoon), domains(&[Land], &[Drones, Fires]), vec![P::Tactical], (false, JevLeads::All)),
        ScenarioType::CombinedArms => (
            120.0,
            (30.0, 30.0),
            (E::Brigade, E::Company),
            domains(&[Land, Drones, Ew, Fires, Logistics], &[Air, Space]),
            vec![P::Overall, P::Land],
            (false, JevLeads::All),
        ),
        ScenarioType::DroneLine => (
            24.0 * 60.0,
            (30.0, 15.0),
            (E::Brigade, E::Platoon),
            domains(&[Land, Drones, Ew, Fires, Logistics, Medical], &[Air, Space]),
            vec![P::Overall, P::Land],
            (false, JevLeads::All),
        ),
        ScenarioType::AirCampaign => (
            24.0 * 60.0,
            (300.0, 300.0),
            (E::Component, E::Platoon),
            domains(&[Air, AirDefence, Ew, Space], &[Land, Logistics]),
            vec![P::Overall, P::Air, P::Land],
            (false, JevLeads::NearContact),
        ),
        ScenarioType::SustainedOffensive => (
            3.0 * 24.0 * 60.0,
            (100.0, 100.0),
            (E::Division, E::Battalion),
            domains(&[Land, Drones, Ew, AirDefence, Logistics, Medical], &[Air, Space, Cyber]),
            vec![P::Overall, P::Land, P::Sustainment],
            (true, JevLeads::NearContact),
        ),
        ScenarioType::Maritime => (
            24.0 * 60.0,
            (500.0, 500.0),
            (E::Component, E::Platoon),
            domains(&[Naval, Air, AirDefence, Drones, Ew, Space, Logistics], &[Land]),
            vec![P::Overall, P::Maritime, P::Air],
            (false, JevLeads::NearContact),
        ),
        ScenarioType::Strike => (
            24.0 * 60.0,
            (1000.0, 1000.0),
            (E::Joint, E::Platoon),
            domains(&[Air, AirDefence, Space, Ew, Cyber, Drones], &[Land, Logistics]),
            vec![P::Overall, P::Air, P::SpaceCyber],
            (true, JevLeads::NearContact),
        ),
        ScenarioType::Joint => (
            7.0 * 24.0 * 60.0,
            (400.0, 300.0),
            (E::Joint, E::Platoon),
            domains(&ScopeDomain::ALL, &[]),
            vec![P::Overall, P::Land, P::Air, P::Maritime, P::SpaceCyber, P::Sustainment],
            (true, JevLeads::NearContact),
        ),
        ScenarioType::Custom => (90.0, (10.0, 10.0), (E::Company, E::Platoon), domains(&[Land], &[]), vec![P::Tactical], (false, JevLeads::All)),
    };
    Scope {
        kind: t,
        duration_min: duration,
        area: Area { width_km: w, depth_km: d },
        echelons: EchelonRange { highest: hi, lowest: lo },
        domains: doms,
        planners,
        detail: Detail { aggregate_away_from_contact: agg, jev },
        time: None,
    }
}

/// Which modes each domain has built (scope.ts `BUILT`).
pub fn built_modes(d: ScopeDomain) -> &'static [DomainMode] {
    use DomainMode::*;
    match d {
        ScopeDomain::Land => &[Simulated],
        ScopeDomain::AirDefence | ScopeDomain::Naval | ScopeDomain::Cyber => &[Simulated, Off],
        _ => &[Simulated, Abstracted, Off],
    }
}

/// The milestone that first builds each domain.
pub fn built_in(d: ScopeDomain) -> &'static str {
    match d {
        ScopeDomain::Land => "now",
        ScopeDomain::Drones | ScopeDomain::Ew => "M2",
        ScopeDomain::Fires => "M3",
        ScopeDomain::Medical => "M4",
        ScopeDomain::Air | ScopeDomain::AirDefence | ScopeDomain::Space => "M5",
        ScopeDomain::Logistics => "M6",
        ScopeDomain::Naval => "M7",
        ScopeDomain::Cyber => "M8",
    }
}

/// The engine's limits (scope.ts `ENGINE_CAN.limits`).
pub const DURATION_MIN_MIN: f64 = 5.0;
pub const DURATION_MIN_MAX: f64 = 14.0 * 24.0 * 60.0;
pub const AREA_KM_MAX: f64 = 1500.0;

/// Why a scope cannot be run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "camelCase")]
pub enum ScopeError {
    CannotSimulate { domain: ScopeDomain, planned_for: String },
    CannotBeOff { domain: ScopeDomain },
    Duration { asked: f64, min: f64, max: f64 },
    EchelonUnknown { highest: Echelon, lowest: Echelon },
    LowestAboveHighest { highest: Echelon, lowest: Echelon },
}

fn ech(e: Echelon) -> String {
    serde_json::to_value(e).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::CannotSimulate { domain, planned_for } => write!(f, "{} cannot be simulated yet (planned for {planned_for}).", domain.label()),
            ScopeError::CannotBeOff { domain } => write!(f, "{} cannot be off: the engine has nothing to play without it.", domain.label()),
            ScopeError::Duration { asked, min, max } => write!(f, "A game can run {min}–{max} minutes today (asked for {asked})."),
            ScopeError::EchelonUnknown { highest, lowest } => {
                write!(f, "Only platoon to joint levels exist today (asked for {} → {}).", ech(*lowest), ech(*highest))
            }
            ScopeError::LowestAboveHighest { highest, lowest } => {
                write!(f, "The lowest command level ({}) is above the highest ({}).", ech(*lowest), ech(*highest))
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// The scope the engine will run, what was changed and why it cannot run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScopeCheck {
    pub scope: Scope,
    pub notes: Vec<String>,
    pub errors: Vec<ScopeError>,
}

/// Turn a scope into one the engine can run, saying what changed — or why it cannot (scope.ts `effectiveScope`).
pub fn effective_scope(scope: &Scope) -> ScopeCheck {
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    let mut out = scope.clone();
    for d in ScopeDomain::ALL {
        let want = scope.mode(d);
        if built_modes(d).contains(&want) {
            continue;
        }
        match want {
            DomainMode::Simulated => errors.push(ScopeError::CannotSimulate { domain: d, planned_for: built_in(d).into() }),
            DomainMode::Abstracted => {
                out.set(d, DomainMode::Off);
                notes.push(format!("{}: abstracted support comes with {}; off for now.", d.label(), built_in(d)));
            }
            DomainMode::Off => errors.push(ScopeError::CannotBeOff { domain: d }),
        }
    }
    if scope.duration_min < DURATION_MIN_MIN || scope.duration_min > DURATION_MIN_MAX || !scope.duration_min.is_finite() {
        errors.push(ScopeError::Duration { asked: scope.duration_min, min: DURATION_MIN_MIN, max: DURATION_MIN_MAX });
    }
    if scope.area.width_km > AREA_KM_MAX || scope.area.depth_km > AREA_KM_MAX {
        out.area = Area { width_km: scope.area.width_km.min(AREA_KM_MAX), depth_km: scope.area.depth_km.min(AREA_KM_MAX) };
        notes.push(format!("The board is {AREA_KM_MAX} × {AREA_KM_MAX} km today; the larger area is not used."));
    }
    let (hi, lo) = (scope.echelons.highest, scope.echelons.lowest);
    if !Echelon::SCOPE.contains(&hi) || !Echelon::SCOPE.contains(&lo) {
        errors.push(ScopeError::EchelonUnknown { highest: hi, lowest: lo });
    }
    if lo > hi {
        errors.push(ScopeError::LowestAboveHighest { highest: hi, lowest: lo });
    }
    ScopeCheck { scope: out, notes, errors }
}

/// Can a game of this type be played today?
pub fn playable(t: ScenarioType) -> bool {
    effective_scope(&scope_for(t)).errors.is_empty()
}

/// Check a scope read from a file, field by field (scope.ts `scopeErrors`):
/// every problem, worded as the TS did.
pub fn scope_errors(raw: &Value) -> Vec<String> {
    let mut e = Vec::new();
    let Some(o) = raw.as_object() else { return vec!["\"scope\" must be an object".into()] };
    let is = |v: Option<&Value>, ok: &[&str]| v.and_then(Value::as_str).is_some_and(|s| ok.contains(&s));
    let types = ["skirmish", "combinedArms", "droneLine", "airCampaign", "sustainedOffensive", "maritime", "strike", "joint", "custom"];
    let echelons = ["platoon", "company", "battalion", "brigade", "division", "corps", "component", "joint"];
    let planners = ["overall", "tactical", "land", "air", "maritime", "spaceCyber", "sustainment"];
    let modes = ["simulated", "abstracted", "off"];
    if !is(o.get("type"), &types) {
        e.push(format!("\"scope.type\" must be one of {}", types.join(", ")));
    }
    if !o.get("durationMin").and_then(Value::as_f64).is_some_and(|x| x > 0.0) {
        e.push("\"scope.durationMin\" must be a positive number".into());
    }
    let pos = |v: Option<&Value>| v.and_then(Value::as_f64).is_some_and(|x| x > 0.0);
    let area = o.get("area");
    if !(pos(area.and_then(|a| a.get("widthKm"))) && pos(area.and_then(|a| a.get("depthKm")))) {
        e.push("\"scope.area\" must be { widthKm, depthKm }, both positive".into());
    }
    let ec = o.get("echelons");
    if !(is(ec.and_then(|x| x.get("highest")), &echelons) && is(ec.and_then(|x| x.get("lowest")), &echelons)) {
        e.push(format!("\"scope.echelons.highest/lowest\" must be one of {}", echelons.join(", ")));
    }
    let dm = o.get("domains");
    for d in ScopeDomain::ALL {
        if !is(dm.and_then(|x| x.get(d.key())), &modes) {
            e.push(format!("\"scope.domains.{}\" must be one of {}", d.key(), modes.join(", ")));
        }
    }
    let pl_ok = o.get("planners").and_then(Value::as_array).is_some_and(|a| a.iter().all(|p| is(Some(p), &planners)));
    if !pl_ok {
        e.push(format!("\"scope.planners\" must be a list of {}", planners.join(", ")));
    }
    let det = o.get("detail");
    let det_ok = det.and_then(|d| d.get("aggregateAwayFromContact")).is_some_and(Value::is_boolean) && is(det.and_then(|d| d.get("jev")), &["all", "nearContact"]);
    if !det_ok {
        e.push("\"scope.detail\" must be { aggregateAwayFromContact: true/false, jev: \"all\" | \"nearContact\" }".into());
    }
    if let Some(t) = o.get("time") {
        if !t.get("startHour").and_then(Value::as_f64).is_some_and(|h| (0.0..24.0).contains(&h)) {
            e.push("\"scope.time.startHour\" must be a number from 0 to 24".into());
        }
        if !is(t.get("weather"), &["clear", "overcast", "rain", "fog", "snow", "storm"]) {
            e.push("\"scope.time.weather\" must be one of clear, overcast, rain, fog, snow, storm".into());
        }
        if !t.get("maxCompression").and_then(Value::as_f64).is_some_and(|x| x >= 1.0) {
            e.push("\"scope.time.maxCompression\" must be a number, 1 or more".into());
        }
        if !t.get("skipQuiet").is_some_and(Value::is_boolean) {
            e.push("\"scope.time.skipQuiet\" must be true or false".into());
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_well_formed_and_playable() {
        for t in ScenarioType::ALL {
            let s = scope_for(t);
            assert!(scope_errors(&serde_json::to_value(&s).unwrap()).is_empty(), "{t:?}");
            assert!(playable(t), "{t:?}");
            assert!(!type_info(t).victory.is_empty() && !type_info(t).milestone.is_empty());
        }
    }

    #[test]
    fn a_skirmish_loads_as_the_plan_sets_it() {
        let c = effective_scope(&scope_for(ScenarioType::Skirmish));
        assert!(c.errors.is_empty());
        for d in ScopeDomain::ALL {
            let want = match d {
                ScopeDomain::Land => DomainMode::Simulated,
                ScopeDomain::Drones | ScopeDomain::Fires => DomainMode::Abstracted,
                _ => DomainMode::Off,
            };
            assert_eq!(c.scope.mode(d), want, "{d:?}");
        }
    }

    #[test]
    fn refuses_with_reasons_and_turns_off_missing_abstractions() {
        let mut s = scope_for(ScenarioType::Skirmish);
        s.set(ScopeDomain::Land, DomainMode::Off);
        assert!(effective_scope(&s).errors.iter().any(|e| e.to_string().contains("Land cannot be off")));
        let mut s = scope_for(ScenarioType::Skirmish);
        s.duration_min = 30_000.0;
        assert!(effective_scope(&s).errors.iter().any(|e| e.to_string().contains("5–20160 minutes")));
        let mut s = scope_for(ScenarioType::Skirmish);
        s.set(ScopeDomain::AirDefence, DomainMode::Abstracted);
        let c = effective_scope(&s);
        assert!(c.errors.is_empty());
        assert_eq!(c.scope.mode(ScopeDomain::AirDefence), DomainMode::Off);
        assert!(c.notes.join(" ").contains("Air defence"));
        let mut s = scope_for(ScenarioType::Skirmish);
        s.echelons = EchelonRange { lowest: Echelon::Brigade, highest: Echelon::Company };
        assert!(effective_scope(&s).errors.iter().any(|e| e.to_string().contains("lowest command level")));
    }

    #[test]
    fn the_time_dimension() {
        let s = scope_for(ScenarioType::Skirmish);
        assert_eq!(s.time_or_default(), DEFAULT_TIME);
        assert_eq!(scope_for(ScenarioType::SustainedOffensive).time_or_default().max_compression, 600.0);
        assert!(!s.changes_environment());
        let mut night = s.clone();
        night.time = Some(ScopeTime { start_hour: 2.0, weather: Weather::Fog, max_compression: 60.0, skip_quiet: true });
        assert!(night.changes_environment());
        assert!(scope_errors(&serde_json::to_value(&night).unwrap()).is_empty());
        let mut bad = serde_json::to_value(&night).unwrap();
        bad["time"] = serde_json::json!({"startHour": 30, "weather": "hail", "maxCompression": 0, "skipQuiet": "yes"});
        assert_eq!(scope_errors(&bad).len(), 4);
    }
}
