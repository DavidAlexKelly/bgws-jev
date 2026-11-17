//! A scenario as Foundry sends it ("bgws.foundry" version 1), made into a
//! BGWS scenario (v4). Foundry gives the essentials: the area, the units and
//! where they stand, the objectives, who commands each side. The rest (scope,
//! domains, comms, behaviours) comes from a shipped scenario's defaults.
//!
//! ```json
//! {"format": "bgws.foundry", "version": 1, "name": "Op Example", "seed": 1,
//!  "durationMin": 90, "startUtc": "2026-06-01T08:00:00Z", "weather": "clear",
//!  "area": {"south": 54.65, "west": 20.40, "north": 54.75, "east": 20.55},
//!  "terrain": "real",
//!  "command": {"blue": "rules", "red": "rules"},
//!  "units": [{"id": "b-1", "label": "Blue 1 tanks", "side": "blue",
//!             "platform": "generic/mbt", "count": 4, "quality": "regular",
//!             "lat": 54.68, "lon": 20.46, "headingDeg": 0}],
//!  "objectives": [{"side": "blue", "label": "OBJ GOLD", "lat": 54.71,
//!                  "lon": 20.47, "radiusM": 600, "holdMin": 10}],
//!  "breakpointPct": 60}
//! ```
//!
//! A unit whose platform is a command post (`generic/cp-*`) becomes its
//! side's HQ, commanding the side's other units. `command.<side>` is `rules`
//! (BGWS's HQ plans; Foundry's orders still take over a unit) or `foundry`
//! (no HQ: units hold until Foundry orders them). A full BGWS scenario
//! (`"format": "bgws.scenario"`) is taken as it is.

use bgws_scenario::v4::{Bounds, ScenarioV4};
use serde::Deserialize;
use serde_json::{json, Value};

/// The shipped scenario whose defaults fill in what Foundry does not say.
const TEMPLATE: &str = include_str!("../../../data/scenarios/gen-kgd-company-attack.json");

/// What ground the scenario wants.
#[derive(Clone, Debug, PartialEq)]
pub enum Terrain {
    /// Real ground for this box: a theatre pack built (or reused) for it.
    Real(Bounds),
    /// Generated ground (only where allowed: not in a federation by default).
    Generated,
    /// As the scenario says (a full BGWS scenario: its pack or ground).
    AsGiven,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Area {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Unit {
    id: String,
    #[serde(default)]
    label: String,
    side: String,
    platform: String,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    quality: Option<String>,
    lat: f64,
    lon: f64,
    #[serde(default)]
    heading_deg: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Objective {
    side: String,
    #[serde(default)]
    label: String,
    lat: f64,
    lon: f64,
    #[serde(default)]
    radius_m: Option<f64>,
    #[serde(default)]
    hold_min: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Foundry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    duration_min: Option<f64>,
    #[serde(default)]
    start_utc: Option<String>,
    #[serde(default)]
    weather: Option<String>,
    area: Area,
    #[serde(default)]
    terrain: Option<String>,
    #[serde(default)]
    command: std::collections::BTreeMap<String, String>,
    units: Vec<Unit>,
    #[serde(default)]
    objectives: Vec<Objective>,
    #[serde(default)]
    breakpoint_pct: Option<f64>,
}

const SIDES: [&str; 3] = ["blue", "red", "neutral"];

/// The echelon a command post commands at.
fn echelon_of(platform: &str) -> Option<&'static str> {
    Some(match platform.strip_prefix("generic/cp-")? {
        "company" => "company",
        "battalion" => "battalion",
        "brigade" => "brigade",
        "division" => "division",
        _ => return None,
    })
}

/// A scenario from Foundry's JSON, and the ground it wants.
pub fn scenario(text: &str) -> Result<(ScenarioV4, Terrain), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the scenario is not JSON: {e}"))?;
    match v.get("format").and_then(Value::as_str) {
        Some("bgws.scenario") => return Ok((ScenarioV4::from_json(text).map_err(|e| format!("the BGWS scenario does not read: {e}"))?, Terrain::AsGiven)),
        Some("bgws.foundry") => {}
        other => return Err(format!("unknown scenario format {other:?} (bgws.foundry or bgws.scenario)")),
    }
    let f: Foundry = serde_json::from_value(v).map_err(|e| format!("the scenario does not read: {e}"))?;
    let a = &f.area;
    if !(a.south < a.north && a.west < a.east && (-90.0..=90.0).contains(&a.south) && (-90.0..=90.0).contains(&a.north)) {
        return Err("the area is not a box (south < north, west < east)".into());
    }
    let (km_ns, km_ew) = ((a.north - a.south) * 111.32, (a.east - a.west) * 111.32 * ((a.south + a.north) / 2.0).to_radians().cos());
    if km_ns > 150.0 || km_ew > 150.0 {
        return Err(format!("the area is {km_ew:.0} by {km_ns:.0} km; at most 150 km a side"));
    }
    if f.units.is_empty() {
        return Err("no units".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for u in &f.units {
        if !ids.insert(u.id.as_str()) {
            return Err(format!("two units are called {}", u.id));
        }
        if !SIDES.contains(&u.side.as_str()) {
            return Err(format!("{}: side {} (blue, red or neutral)", u.id, u.side));
        }
        if !(u.lat >= a.south && u.lat <= a.north && u.lon >= a.west && u.lon <= a.east) {
            return Err(format!("{} stands outside the area", u.id));
        }
    }
    let mut sc: Value = serde_json::from_str(TEMPLATE).expect("the template reads");
    let name = if f.name.trim().is_empty() { "foundry-scenario".to_string() } else { f.name.trim().to_string() };
    let seed = f.seed.unwrap_or(1);
    let centre = json!({"lat": (a.south + a.north) / 2.0, "lon": (a.west + a.east) / 2.0, "h": 0.0});
    let terrain = match f.terrain.as_deref().unwrap_or("real") {
        "real" => Terrain::Real(Bounds { south: a.south, west: a.west, north: a.north, east: a.east }),
        "generated" => Terrain::Generated,
        t => return Err(format!("terrain {t} (real or generated)")),
    };
    sc["name"] = json!(name);
    sc["description"] = json!(format!("{name}: sent from Foundry ({} units)", f.units.len()));
    sc["seed"] = json!(seed);
    sc["seedText"] = json!(name);
    sc["theatre"] = json!({"origin": centre, "bounds": {"south": a.south, "west": a.west, "north": a.north, "east": a.east}, "ground": "generated", "groundSeed": name});
    sc["scope"]["durationMin"] = json!(f.duration_min.unwrap_or(90.0).clamp(1.0, 24.0 * 60.0));
    sc["scope"]["area"] = json!({"widthKm": km_ew, "depthKm": km_ns});
    if let Some(t) = &f.start_utc {
        sc["startUtc"] = json!(t);
    }
    sc["weather"] = json!({"source": "preset", "weather": f.weather.clone().unwrap_or_else(|| "clear".into())});
    let forces: Vec<Value> = f
        .units
        .iter()
        .map(|u| {
            let mut x = json!({
                "id": u.id, "label": if u.label.is_empty() { &u.id } else { &u.label }, "side": u.side, "platform": u.platform,
                "count": u.count.unwrap_or(1).max(1), "quality": u.quality.clone().unwrap_or_else(|| "regular".into()),
                "position": {"lat": u.lat, "lon": u.lon, "h": 0.0}, "layer": "unit",
            });
            if let Some(h) = u.heading_deg {
                x["headingDeg"] = json!(h);
            }
            x
        })
        .collect();
    sc["forces"] = json!(forces);
    // Who commands each side, and its HQ.
    let mut sides = serde_json::Map::new();
    let mut hqs = vec![];
    for side in SIDES {
        let of_side: Vec<&Unit> = f.units.iter().filter(|u| u.side == side).collect();
        if of_side.is_empty() {
            continue;
        }
        let mode = f.command.get(side).map(String::as_str).unwrap_or("rules");
        if mode != "rules" && mode != "foundry" {
            return Err(format!("command.{side}: {mode} (rules or foundry)"));
        }
        sides.insert(side.into(), json!({"units": "rules", "hqs": "rules", "directive": ""}));
        if mode == "foundry" {
            continue;
        }
        if let Some(post) = of_side.iter().find(|u| echelon_of(&u.platform).is_some()) {
            let subordinates: Vec<&str> = of_side.iter().filter(|u| u.id != post.id).map(|u| u.id.as_str()).collect();
            hqs.push(json!({"id": format!("{side}-hq"), "side": side, "echelon": echelon_of(&post.platform), "postId": post.id, "subordinates": subordinates, "planner": "land", "decider": "rules"}));
        }
    }
    sc["command"]["sides"] = Value::Object(sides);
    sc["command"]["hqs"] = json!(hqs);
    let mut victory: Vec<Value> = vec![];
    for o in &f.objectives {
        if !SIDES.contains(&o.side.as_str()) {
            return Err(format!("objective {}: side {}", o.label, o.side));
        }
        victory.push(json!({"kind": "objective", "side": o.side, "at": {"lat": o.lat, "lon": o.lon, "h": 0.0}, "radiusM": o.radius_m.unwrap_or(600.0), "holdS": o.hold_min.unwrap_or(10.0) * 60.0, "label": if o.label.is_empty() { format!("{}'s objective", o.side) } else { o.label.clone() }}));
    }
    victory.push(json!({"kind": "breakpoint", "pct": f.breakpoint_pct.unwrap_or(60.0)}));
    sc["victory"] = json!(victory);
    let text = serde_json::to_string(&sc).expect("json");
    let sc = ScenarioV4::from_json(&text).map_err(|e| format!("the scenario does not make a BGWS scenario: {e}"))?;
    Ok((sc, terrain))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        json!({
            "format": "bgws.foundry", "version": 1, "name": "Op Test", "seed": 7, "durationMin": 30,
            "area": {"south": 54.65, "west": 20.40, "north": 54.75, "east": 20.55},
            "terrain": "generated",
            "command": {"blue": "rules", "red": "foundry"},
            "units": [
                {"id": "b-hq", "label": "Blue HQ", "side": "blue", "platform": "generic/cp-company", "lat": 54.66, "lon": 20.45},
                {"id": "b-1", "label": "Blue 1", "side": "blue", "platform": "generic/mbt", "count": 4, "lat": 54.67, "lon": 20.46},
                {"id": "r-1", "label": "Red 1", "side": "red", "platform": "generic/ifv", "count": 3, "quality": "veteran", "lat": 54.72, "lon": 20.47}
            ],
            "objectives": [{"side": "blue", "label": "OBJ GOLD", "lat": 54.71, "lon": 20.47, "radiusM": 500, "holdMin": 5}]
        })
    }

    #[test]
    fn a_foundry_scenario_makes_a_bgws_scenario() {
        let (sc, terrain) = scenario(&example().to_string()).unwrap();
        assert_eq!(terrain, Terrain::Generated);
        assert_eq!(sc.name, "Op Test");
        assert_eq!(sc.forces.len(), 3);
        // Blue's HQ is its command post; red, commanded from Foundry, has none.
        let v = serde_json::to_value(&sc).unwrap();
        let hqs = v["command"]["hqs"].as_array().unwrap();
        assert_eq!(hqs.len(), 1);
        assert_eq!(hqs[0]["postId"], "b-hq");
        assert_eq!(hqs[0]["subordinates"], json!(["b-1"]));
        assert_eq!(v["victory"][0]["holdS"], 300.0);
        // It plays.
        let mut sim = bgws_sim::Sim::new(&sc, bgws_core::Seed(7), bgws_sim::default_figures()).unwrap();
        sim.run_with_rules(bgws_core::SimTime::from_secs(30.0));
    }

    #[test]
    fn mistakes_are_refused_with_the_reason() {
        let mut e = example();
        e["units"][1]["lat"] = json!(60.0);
        assert!(scenario(&e.to_string()).unwrap_err().contains("outside the area"));
        let mut e = example();
        e["units"][2]["side"] = json!("green");
        assert!(scenario(&e.to_string()).unwrap_err().contains("side green"));
        let mut e = example();
        e["format"] = json!("other");
        assert!(scenario(&e.to_string()).is_err());
        let mut e = example();
        e["units"] = json!([]);
        assert!(scenario(&e.to_string()).unwrap_err().contains("no units"));
        let mut e = example();
        e["terrain"] = json!("real");
        assert!(matches!(scenario(&e.to_string()).unwrap().1, Terrain::Real(_)));
    }

    #[test]
    fn a_full_bgws_scenario_passes_through() {
        let (sc, terrain) = scenario(TEMPLATE).unwrap();
        assert_eq!(terrain, Terrain::AsGiven);
        assert_eq!(sc.name, "gen-kgd-company-attack");
    }
}
