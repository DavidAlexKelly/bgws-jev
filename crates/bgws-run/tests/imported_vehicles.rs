//! The vehicles imported from the asset CSVs (`imported.json` with the
//! weapon fits of `weapon-fits.json`) play: every fitted one is put in a
//! meeting engagement, half a side each, and the game runs with them firing
//! their own rounds.

use bgws_core::{Seed, SimTime};
use bgws_run::scengen::{build, standard, GroundSpec};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::{Sim, SimOptions};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn fitted() -> Vec<String> {
    let fits: Value = serde_json::from_str(include_str!("../../../data/catalogue/weapon-fits.json")).unwrap();
    fits["fits"].as_object().unwrap().keys().cloned().collect()
}

/// The meeting engagement with its units replaced by `platforms`, alternately
/// blue and red, in a line across each side's start.
fn meeting_with(platforms: &[String]) -> ScenarioV4 {
    let mut spec = standard().into_iter().find(|s| s.name == "gen-meeting-engagement").unwrap();
    spec.ground = GroundSpec::Flat;
    let mut v = build(&spec).unwrap();
    let forces = v["forces"].as_array().unwrap().clone();
    let template = |id: &str| forces.iter().find(|f| f["id"] == id).unwrap().clone();
    let (b1, r1) = (template("b-1"), template("r-1"));
    let mut out: Vec<Value> = forces.iter().filter(|f| matches!(f["id"].as_str(), Some("b-hq" | "r-hq"))).cloned().collect();
    let (mut blue, mut red) = (Vec::new(), Vec::new());
    for (i, p) in platforms.iter().enumerate() {
        let (base, side_list, side) = if i % 2 == 0 { (&b1, &mut blue, "b") } else { (&r1, &mut red, "r") };
        let k = side_list.len();
        let mut f = base.clone();
        let id = format!("{side}-l7-{k}");
        f["id"] = json!(id);
        f["label"] = json!(format!("{side} {p}"));
        f["platform"] = json!(p);
        f["count"] = json!(1);
        let lon = base["position"]["lon"].as_f64().unwrap() + (k as f64 - 27.0) * 0.0009;
        f["position"]["lon"] = json!(lon);
        side_list.push(id);
        out.push(f);
    }
    v["forces"] = Value::Array(out);
    for h in v["command"]["hqs"].as_array_mut().unwrap() {
        h["subordinates"] = json!(if h["side"] == "blue" { &blue } else { &red });
    }
    v["victory"] = json!([]);
    ScenarioV4::from_json(&v.to_string()).unwrap()
}

#[test]
fn every_fitted_imported_vehicle_plays_and_fires_its_own_rounds() {
    let platforms = fitted();
    assert!(platforms.len() >= 100, "{}", platforms.len());
    let sc = meeting_with(&platforms);
    let mut sim = Sim::with_options(&sc, Seed(1), bgws_run::default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap();
    bgws_run::experiments::play_with_rules(&mut sim, SimTime::from_secs(20.0 * 60.0));
    let events = sim.drain_events();
    let fired: BTreeSet<String> = events
        .iter()
        .filter(|e| matches!(e.kind.as_str(), "fire.hit" | "fire.kill" | "fire.miss" | "fire.launch"))
        .filter_map(|e| e.data.get("munition").and_then(Value::as_str).map(str::to_string))
        .collect();
    let own: Vec<&String> = fired.iter().filter(|m| m.starts_with("l7/")).collect();
    eprintln!("imported rounds fired: {own:?}");
    assert!(own.len() >= 5, "few of the imported rounds were fired: {fired:?}");
    // Something was knocked out: the rounds do damage, not just fly.
    assert!(events.iter().any(|e| e.kind == "fire.kill"), "no kills");
}

/// The shipped scenarios built on imported vehicles load and play: both sides
/// fire, with their own rounds.
#[test]
fn the_imported_vehicle_scenarios_play() {
    for name in ["skirmish-l7-armour-v1", "skirmish-l7-mech-v1", "skirmish-l7-ifv-v1"] {
        let path = bgws_run::scenario_file(name);
        let sc = ScenarioV4::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut sim = Sim::with_options(&sc, Seed(1), bgws_run::default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap();
        bgws_run::experiments::play_with_rules(&mut sim, SimTime::from_secs(20.0 * 60.0));
        let events = sim.drain_events();
        let shots: Vec<&bgws_core::LogEvent> = events.iter().filter(|e| matches!(e.kind.as_str(), "fire.hit" | "fire.kill" | "fire.miss" | "fire.launch")).collect();
        let sides: BTreeSet<String> = shots.iter().map(|e| format!("{:?}", e.side)).collect();
        assert!(sides.len() >= 2, "{name}: only {sides:?} fired");
        assert!(shots.iter().any(|e| e.data.get("munition").and_then(Value::as_str).is_some_and(|m| m.starts_with("l7/"))), "{name}: no imported round fired");
    }
}
