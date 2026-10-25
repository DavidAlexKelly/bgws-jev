//! The TypeScript scenario file, versions 1–3 (ports realtime/scenario.ts):
//! forces placed as lat/lng, the scope, rules, command, behaviours, and the
//! version-3 world (entities, obstacles, environment, HQs, control measures,
//! cyber accesses, victory, sea, NAIs, events, network). Read only to be
//! converted to version 4 (`convert.rs`).
//!
//! `parse_v3` checks a file field by field and lists every problem in the
//! words the TS used; it upgrades versions 1 and 2 on the way, as the TS did.
//! The one check it cannot port is building the behaviour-template registry
//! (that lives in `bgws-behaviour`); it checks template ids instead.

use crate::scope::{effective_scope, scope_errors, scope_for, ScenarioType, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const V3_FORMAT: &str = "bgws.realtime.scenario";
pub const V3_VERSION: u32 = 3;

/// A TS position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundV3 {
    pub choice: String,
    pub seed: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesV3 {
    pub comms: String,
    pub breakpoint_pct: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Directives {
    pub blue: String,
    pub red: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandV3 {
    pub use_commander: bool,
    pub model: String,
    pub use_jev: bool,
    pub directives: Directives,
}

/// lib/forceBuilder.ts `PlacedElement`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedElement {
    pub id: String,
    pub label: String,
    pub side: String,
    pub platform: String,
    /// Kept as written (an integer stays an integer on a round trip).
    pub platform_count: serde_json::Number,
    pub troop_quality: String,
    pub position: LatLng,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concealed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_dummy: Option<bool>,
}

/// world/create.ts `EntitySpec`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitySpecV3 {
    pub id: String,
    pub owner: String,
    pub archetype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub at: LatLng,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit_phase_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit_raan_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<Vec<LatLng>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// The version-3 world (world/create.ts `WorldSetup` without `platforms`).
/// Sections other than entities are kept as JSON; the converter types them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorldV3 {
    #[serde(default)]
    pub entities: Vec<EntitySpecV3>,
    #[serde(default)]
    pub obstacles: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Map<String, Value>>,
    #[serde(default)]
    pub hqs: Vec<Value>,
    #[serde(default)]
    pub measures: Vec<Value>,
    #[serde(default)]
    pub cyber: Vec<Value>,
    #[serde(default)]
    pub victory: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea: Option<Value>,
    #[serde(default)]
    pub nais: Vec<Value>,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<BTreeMap<String, String>>,
}

/// realtime/scenario.ts `RealtimeScenario`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioV3 {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub saved_at: String,
    pub seed: String,
    pub ground: GroundV3,
    pub scope: Scope,
    pub rules: RulesV3,
    pub command: CommandV3,
    pub forces: Vec<PlacedElement>,
    pub behaviours: BTreeMap<String, String>,
    pub behaviour_templates: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldV3>,
    /// Not in the TS type: force lists' caveats, carried when present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caveats: Vec<String>,
}

fn is_str(v: Option<&Value>) -> bool {
    v.is_some_and(Value::is_string)
}
fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}
fn latlng_ok(v: Option<&Value>) -> bool {
    num(v.and_then(|p| p.get("lat"))).is_some() && num(v.and_then(|p| p.get("lng"))).is_some()
}

/// A version-1 file as version 2: a Skirmish, its time limit as the duration.
fn from_version1(raw: &Map<String, Value>) -> Map<String, Value> {
    let mut out = raw.clone();
    let mut rules = raw.get("rules").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut scope = scope_for(ScenarioType::Skirmish);
    if let Some(t) = rules.remove("timeLimitMin").as_ref().and_then(Value::as_f64) {
        scope.duration_min = t;
    }
    out.insert("version".into(), Value::from(V3_VERSION));
    out.insert("scope".into(), serde_json::to_value(scope).unwrap_or(Value::Null));
    out.insert("rules".into(), Value::Object(rules));
    out
}

/// Template ids a v3 file may name without bundling them: every built-in
/// `core/` template (the TS bundles them with the app).
pub fn is_builtin_template(id: &str) -> bool {
    id.starts_with("core/")
}

/// Read and check a TS scenario file (scenario.ts `parseScenario`). Every
/// problem is listed; the scenario is returned only when there are none.
pub fn parse_v3(raw: &Value) -> Result<ScenarioV3, Vec<String>> {
    let Some(obj) = raw.as_object() else { return Err(vec!["Not a scenario file (expected a JSON object).".into()]) };
    if obj.get("format").and_then(Value::as_str) != Some(V3_FORMAT) {
        let f = obj.get("format").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_else(|| "undefined".into());
        return Err(vec![format!("Not a real-time scenario file (format is \"{f}\").")]);
    }
    let mut obj = obj.clone();
    match obj.get("version").and_then(Value::as_u64) {
        Some(1) => obj = from_version1(&obj),
        Some(2) => {
            obj.insert("version".into(), Value::from(V3_VERSION));
        }
        Some(3) => {}
        _ => {
            let v = obj.get("version").map(|v| v.to_string()).unwrap_or_else(|| "undefined".into());
            return Err(vec![format!("Scenario version {v} is not supported (this reader takes versions 1 to {V3_VERSION}).")]);
        }
    }
    let mut errors = Vec::new();
    let mut need = |ok: bool, msg: &str| {
        if !ok {
            errors.push(msg.to_owned());
        }
    };
    need(is_str(obj.get("name")), "\"name\" must be text");
    need(is_str(obj.get("seed")), "\"seed\" must be text");
    let ground = obj.get("ground");
    need(
        ground.and_then(|g| g.get("choice")).and_then(Value::as_str).is_some_and(|c| ["generated", "raster+relief", "raster"].contains(&c)),
        "\"ground.choice\" must be one of generated, raster+relief, raster",
    );
    need(is_str(ground.and_then(|g| g.get("seed"))), "\"ground.seed\" must be text");
    let rules = obj.get("rules");
    need(rules.and_then(|r| r.get("comms")).and_then(Value::as_str).is_some_and(|c| c == "radio" || c == "perfect"), "\"rules.comms\" must be \"radio\" or \"perfect\"");
    need(num(rules.and_then(|r| r.get("breakpointPct"))).is_some_and(|b| (10.0..=100.0).contains(&b)), "\"rules.breakpointPct\" must be 10–100");
    let command = obj.get("command");
    let cb = |k: &str| command.and_then(|c| c.get(k)).is_some_and(Value::is_boolean);
    need(cb("useCommander") && cb("useJev"), "\"command.useCommander\" and \"command.useJev\" must be true/false");
    need(is_str(command.and_then(|c| c.get("model"))), "\"command.model\" must be text");
    let dirs = command.and_then(|c| c.get("directives"));
    need(is_str(dirs.and_then(|d| d.get("blue"))) && is_str(dirs.and_then(|d| d.get("red"))), "\"command.directives.blue/red\" must be text");
    let scope_raw = obj.get("scope").cloned().unwrap_or(Value::Null);
    let scope_problems = scope_errors(&scope_raw);
    let scope_ok = scope_problems.is_empty();
    errors.extend(scope_problems);
    if scope_ok {
        match serde_json::from_value::<Scope>(scope_raw) {
            Ok(s) => errors.extend(effective_scope(&s).errors.into_iter().map(|e| format!("scope: {e}"))),
            Err(e) => errors.push(format!("scope: {e}")),
        }
    }
    let forces = obj.get("forces").and_then(Value::as_array);
    if forces.is_none() {
        errors.push("\"forces\" must be a list".into());
    }
    let mut ids = BTreeSet::new();
    for (i, f) in forces.into_iter().flatten().enumerate() {
        let at = format!("forces[{i}]");
        let Some(f) = f.as_object() else {
            errors.push(format!("{at}: must be an object"));
            continue;
        };
        match f.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => {
                if !ids.insert(id.to_owned()) {
                    errors.push(format!("{at}: duplicate unit id \"{id}\""));
                }
            }
            _ => errors.push(format!("{at}: \"id\" is required")),
        }
        if !f.get("side").and_then(Value::as_str).is_some_and(|s| s == "blue" || s == "red") {
            errors.push(format!("{at}: \"side\" must be \"blue\" or \"red\""));
        }
        if !f.get("platform").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
            errors.push(format!("{at}: \"platform\" is required"));
        }
        if !is_str(f.get("label")) {
            errors.push(format!("{at}: \"label\" must be text"));
        }
        if !num(f.get("platformCount")).is_some_and(|c| c > 0.0) {
            errors.push(format!("{at}: \"platformCount\" must be a positive number"));
        }
        if !is_str(f.get("troopQuality")) {
            errors.push(format!("{at}: \"troopQuality\" must be text"));
        }
        let p = f.get("position");
        let ok = num(p.and_then(|p| p.get("lat"))).is_some_and(|l| l.abs() <= 90.0) && num(p.and_then(|p| p.get("lng"))).is_some_and(|l| l.abs() <= 180.0);
        if !ok {
            errors.push(format!("{at}: \"position\" must be {{ lat, lng }}"));
        }
    }
    let behaviours = obj.get("behaviours").and_then(Value::as_object);
    if behaviours.is_none() {
        errors.push("\"behaviours\" must be an object of unit id → template id".into());
    }
    let templates = obj.get("behaviourTemplates").and_then(Value::as_array);
    if templates.is_none() {
        errors.push("\"behaviourTemplates\" must be a list".into());
    }
    let mut known: BTreeSet<String> = BTreeSet::new();
    for t in templates.into_iter().flatten() {
        match t.get("id").and_then(Value::as_str) {
            Some(id) => {
                known.insert(id.to_owned());
                if let Some(parent) = t.get("parent").and_then(Value::as_str) {
                    if !is_builtin_template(parent) && !templates.into_iter().flatten().any(|o| o.get("id").and_then(Value::as_str) == Some(parent)) {
                        errors.push(format!("Behaviour templates in the file are invalid: \"{id}\" extends unknown \"{parent}\""));
                    }
                }
            }
            None => errors.push("Behaviour templates in the file are invalid: a template has no id".into()),
        }
    }
    for (unit, tid) in behaviours.into_iter().flatten() {
        if !ids.contains(unit) {
            errors.push(format!("behaviours: \"{unit}\" is not one of the forces"));
        }
        let ok = tid.as_str().is_some_and(|t| is_builtin_template(t) || known.contains(t));
        if !ok {
            errors.push(format!("behaviours.{unit}: unknown template \"{}\"", tid.as_str().unwrap_or("?")));
        }
    }
    if let Some(w) = obj.get("world") {
        errors.extend(world_errors(w, &ids));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    serde_json::from_value::<ScenarioV3>(Value::Object(obj)).map_err(|e| vec![format!("Not a valid scenario: {e}")])
}

/// scenario.ts `worldErrors`. Archetypes are checked against the catalogue
/// later, in v4 validation.
fn world_errors(raw: &Value, unit_ids: &BTreeSet<String>) -> Vec<String> {
    let mut e = Vec::new();
    let Some(w) = raw.as_object() else { return vec!["\"world\" must be an object".into()] };
    match w.get("entities") {
        None => {}
        Some(Value::Array(list)) => {
            let mut ids = BTreeSet::new();
            for (i, x) in list.iter().enumerate() {
                let at = format!("world.entities[{i}]");
                let Some(x) = x.as_object() else {
                    e.push(format!("{at}: must be an object"));
                    continue;
                };
                match x.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => {
                        if ids.contains(id) || unit_ids.contains(id) {
                            e.push(format!("{at}: duplicate id \"{id}\""));
                        } else {
                            ids.insert(id.to_owned());
                        }
                    }
                    _ => e.push(format!("{at}: \"id\" is required")),
                }
                if !x.get("owner").and_then(Value::as_str).is_some_and(|o| ["blue", "red", "neutral"].contains(&o)) {
                    e.push(format!("{at}: \"owner\" must be blue, red or neutral"));
                }
                if !is_str(x.get("archetype")) {
                    e.push(format!("{at}: unknown archetype \"{}\"", x.get("archetype").map(|v| v.to_string()).unwrap_or_default()));
                }
                if !latlng_ok(x.get("at")) {
                    e.push(format!("{at}: \"at\" must be {{ lat, lng }}"));
                }
            }
        }
        Some(_) => e.push("\"world.entities\" must be a list".into()),
    }
    match w.get("obstacles") {
        None => {}
        Some(Value::Array(list)) => {
            for (i, o) in list.iter().enumerate() {
                let at = format!("world.obstacles[{i}]");
                let Some(o) = o.as_object() else {
                    e.push(format!("{at}: must be an object"));
                    continue;
                };
                if !o.get("kind").and_then(Value::as_str).is_some_and(|k| ["minefield", "ditch", "dragonsTeeth", "seaMines"].contains(&k)) {
                    e.push(format!("{at}: \"kind\" must be minefield, ditch, dragonsTeeth or seaMines"));
                }
                if !o.get("owner").and_then(Value::as_str).is_some_and(|s| s == "blue" || s == "red") {
                    e.push(format!("{at}: \"owner\" must be blue or red"));
                }
                if !(num(o.get("widthM")).is_some_and(|x| x > 0.0) && num(o.get("depthM")).is_some_and(|x| x > 0.0)) {
                    e.push(format!("{at}: \"widthM\" and \"depthM\" must be positive"));
                }
            }
        }
        Some(_) => e.push("\"world.obstacles\" must be a list".into()),
    }
    if w.get("environment").is_some_and(|v| !v.is_object()) {
        e.push("\"world.environment\" must be an object".into());
    }
    if let Some(sea) = w.get("sea") {
        let ok = sea.get("waters").and_then(Value::as_array).is_some_and(|ws| ws.iter().all(|poly| poly.as_array().is_some_and(|p| p.len() >= 3 && p.iter().all(|q| latlng_ok(Some(q))))));
        if !ok {
            e.push("\"world.sea.waters\" must be a list of polygons of at least three { lat, lng } points".into());
        }
    }
    if let Some(ev) = w.get("events") {
        let kinds = ["cutCommercialImagery", "cutLeoSatcom", "jamSatcom", "jamGnss", "dazzleEo", "asat", "restore"];
        let ok = ev.as_array().is_some_and(|l| {
            l.iter().all(|x| num(x.get("at")).is_some() && x.get("kind").and_then(Value::as_str).is_some_and(|k| kinds.contains(&k)) && x.get("side").and_then(Value::as_str).is_some_and(|s| s == "blue" || s == "red"))
        });
        if !ok {
            e.push(format!("\"world.events\" must be a list of {{ at, kind ({}), side }}", kinds.join(", ")));
        }
    }
    if let Some(n) = w.get("network") {
        let ok = n.as_object().is_some_and(|m| m.iter().all(|(k, v)| (k == "blue" || k == "red") && v.as_str().is_some_and(|s| s == "voice" || s == "datalink")));
        if !ok {
            e.push("\"world.network\" must give blue and red each \"voice\" or \"datalink\"".into());
        }
    }
    if let Some(v) = w.get("victory") {
        let kinds = ["airSuperiority", "targetSet", "seaControl", "through", "all"];
        let ok = v.as_array().is_some_and(|l| {
            l.iter().all(|x| x.get("kind").and_then(Value::as_str).is_some_and(|k| kinds.contains(&k)) && x.get("side").and_then(Value::as_str).is_some_and(|s| s == "blue" || s == "red"))
        });
        if !ok {
            e.push(format!("\"world.victory\" must be a list of conditions ({}) each with a side", kinds.join(", ")));
        }
    }
    e
}

/// scenario.ts `scenarioFileName`.
pub fn scenario_file_name(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
            dash = false;
        } else if !dash {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    format!("{}.bgws-scenario.json", if slug.is_empty() { "scenario" } else { slug })
}

/// The user templates these behaviours need, with their custom ancestors
/// (scenario.ts `referencedTemplates`), sorted by id.
pub fn referenced_templates(behaviours: &BTreeMap<String, String>, available: &[Value]) -> Vec<Value> {
    let by_id: BTreeMap<&str, &Value> = available.iter().filter_map(|t| Some((t.get("id")?.as_str()?, t))).collect();
    let mut needed: BTreeMap<String, Value> = BTreeMap::new();
    for id in behaviours.values() {
        let mut next = Some(id.clone());
        while let Some(n) = next {
            if needed.contains_key(&n) {
                break;
            }
            let Some(t) = by_id.get(n.as_str()) else { break };
            needed.insert(n.clone(), (*t).clone());
            next = t.get("parent").and_then(Value::as_str).map(str::to_owned);
        }
    }
    needed.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn sample() -> Value {
        json!({
            "format": V3_FORMAT, "version": 3, "name": "Meeting engagement", "savedAt": "2026-10-21T12:00:00.000Z", "seed": "1",
            "ground": {"choice": "raster+relief", "seed": "baltic-v1"},
            "scope": serde_json::to_value({ let mut s = scope_for(ScenarioType::Skirmish); s.duration_min = 75.0; s }).unwrap(),
            "rules": {"comms": "radio", "breakpointPct": 50},
            "command": {"useCommander": true, "model": "any", "useJev": true, "directives": {"blue": "", "red": ""}},
            "forces": [
                {"id": "blue-1", "label": "1 Troop", "side": "blue", "platform": "var_11_default", "platformCount": 4, "troopQuality": "regular", "position": {"lat": 54.2, "lng": 18.6}},
                {"id": "red-1", "label": "Red 1", "side": "red", "platform": "var_22_1", "platformCount": 3, "troopQuality": "regular", "position": {"lat": 54.214, "lng": 18.6}}
            ],
            "behaviours": {"blue-1": "custom/stand-fast"},
            "behaviourTemplates": [{"id": "custom/stand-fast", "version": 1, "parent": "core/default", "remove": ["pullBack"]}]
        })
    }

    #[test]
    fn reads_back_what_was_saved() {
        let s = parse_v3(&sample()).expect("valid");
        assert_eq!(s.forces.len(), 2);
        assert_eq!(serde_json::to_value(&s).unwrap()["forces"], sample()["forces"]);
    }

    #[test]
    fn refuses_other_files_and_versions() {
        assert!(parse_v3(&json!([])).unwrap_err()[0].contains("JSON object"));
        assert!(parse_v3(&json!({"format": "something"})).unwrap_err()[0].contains("Not a real-time scenario"));
        let mut v = sample();
        v["version"] = json!(4);
        assert!(parse_v3(&v).unwrap_err()[0].contains("version 4 is not supported"));
    }

    #[test]
    fn lists_every_problem_at_once() {
        let mut bad = sample();
        bad["rules"] = json!({"comms": "smoke signals", "breakpointPct": 5});
        let f0 = bad["forces"][0].clone();
        let mut green = f0.clone();
        green["side"] = json!("green");
        green["position"] = json!({"lat": 999, "lng": 0});
        bad["forces"] = json!([green, f0]);
        bad["behaviours"] = json!({"ghost": "core/default", "blue-1": "custom/missing"});
        bad["behaviourTemplates"] = json!([]);
        let text = parse_v3(&bad).unwrap_err().join("\n");
        for want in ["rules.comms", "breakpointPct", "\"side\"", "\"position\"", "duplicate unit id", "\"ghost\" is not one of the forces", "unknown template \"custom/missing\""] {
            assert!(text.contains(want), "{want}\n{text}");
        }
    }

    #[test]
    fn reads_a_version_1_file_as_a_skirmish() {
        let mut v1 = sample();
        v1["version"] = json!(1);
        v1.as_object_mut().unwrap().remove("scope");
        v1["rules"] = json!({"comms": "radio", "breakpointPct": 50, "timeLimitMin": 45});
        let s = parse_v3(&v1).expect("v1 reads");
        assert_eq!(s.version, 3);
        let mut want = scope_for(ScenarioType::Skirmish);
        want.duration_min = 45.0;
        assert_eq!(s.scope, want);
        assert_eq!(s.rules, RulesV3 { comms: "radio".into(), breakpoint_pct: 50.0 });
    }

    #[test]
    fn checks_the_scope() {
        let mut v = sample();
        v["scope"]["durationMin"] = json!(-1);
        v["scope"]["domains"]["land"] = json!("maybe");
        let t = parse_v3(&v).unwrap_err().join("\n");
        assert!(t.contains("durationMin") && t.contains("scope.domains.land"));
        let mut v = sample();
        v["scope"]["domains"]["land"] = json!("off");
        assert!(parse_v3(&v).unwrap_err().join("\n").contains("scope: Land cannot be off"));
    }

    #[test]
    fn helpers() {
        assert_eq!(scenario_file_name("Meeting engagement: Narva!"), "meeting-engagement-narva.bgws-scenario.json");
        assert_eq!(scenario_file_name("  "), "scenario.bgws-scenario.json");
        let child = json!({"id": "custom/child", "version": 1, "parent": "custom/stand-fast"});
        let sf = json!({"id": "custom/stand-fast", "version": 1, "parent": "core/default"});
        let unused = json!({"id": "custom/unused", "version": 1, "parent": "core/default"});
        let mut b = BTreeMap::new();
        b.insert("a".to_string(), "custom/child".to_string());
        b.insert("b".to_string(), "core/armour".to_string());
        let ids: Vec<String> = referenced_templates(&b, &[sf, child, unused]).iter().map(|t| t["id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(ids, vec!["custom/child", "custom/stand-fast"]);
    }
}
