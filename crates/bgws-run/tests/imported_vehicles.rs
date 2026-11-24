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

/// `on_ground`: ground 0 is the scenario's own; another k gives other ground
/// (another seed, other cover where the forces stand); flat ground and theatre
/// packs are left alone.
#[test]
fn on_ground_gives_other_generated_ground() {
    let sc = ScenarioV4::from_json(&std::fs::read_to_string(bgws_run::scenario_file("skirmish-l7-armour-v1")).unwrap()).unwrap();
    assert_eq!(bgws_run::on_ground(&sc, 0).theatre.ground_seed, sc.theatre.ground_seed);
    let g3 = bgws_run::on_ground(&sc, 3);
    assert_eq!(g3.theatre.ground_seed, format!("{}~g3", sc.theatre.ground_seed));
    let cover = |s: &ScenarioV4| {
        let sim = Sim::with_options(s, Seed(1), bgws_run::default_figures(), SimOptions::default()).unwrap();
        (0..40).map(|i| format!("{:?}", sim.ctx.ground.land_cover(-2000.0 + 100.0 * i as f64, 0.0))).collect::<Vec<_>>()
    };
    assert_ne!(cover(&sc), cover(&g3), "a different ground seed gives different ground");
    let mut flat = sc.clone();
    flat.theatre.ground = "flat".into();
    assert_eq!(bgws_run::on_ground(&flat, 3).theatre.ground_seed, flat.theatre.ground_seed);
}

/// `sides_swapped`: the same map under the swapped game (the ground is a
/// function of where a point is), blue starting where red did and red where
/// blue did, facing the other way; swapping twice gives the original.
#[test]
fn sides_swapped_exchanges_places_on_the_same_ground() {
    for name in ["skirmish-symmetric-control-v1", "m2-mech-company-hasty-attack", "gen-company-attack"] {
        let sc = ScenarioV4::from_json(&std::fs::read_to_string(bgws_run::scenario_file(name)).unwrap()).unwrap();
        let sw = bgws_run::sides_swapped(&sc).unwrap();
        let half = bgws_sim::ground_half_extent(&sc).max(bgws_sim::ground_half_extent(&sw));
        let cover = |s: &ScenarioV4| {
            let sim = Sim::with_options(s, Seed(1), bgws_run::default_figures(), SimOptions { ground_half_m: Some(half), ..Default::default() }).unwrap();
            (0..60).map(|i| format!("{:?}/{:.1}", sim.ctx.ground.land_cover(-3000.0 + 100.0 * i as f64, 250.0), sim.ctx.ground.height(-3000.0 + 100.0 * i as f64, 250.0))).collect::<Vec<_>>()
        };
        assert_eq!(cover(&sc), cover(&sw), "{name}: the swapped game is on other ground");
        let centre = |s: &ScenarioV4, side: bgws_core::Side| {
            let v: Vec<(f64, f64)> = s.forces.iter().filter(|f| f.side == side).map(|f| (f.position.lat, f.position.lon)).collect();
            (v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64, v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64)
        };
        let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        assert!(near(centre(&sw, bgws_core::Side::Blue), centre(&sc, bgws_core::Side::Red)), "{name}: blue starts where red did");
        assert!(near(centre(&sw, bgws_core::Side::Red), centre(&sc, bgws_core::Side::Blue)), "{name}: red starts where blue did");
        let back = bgws_run::sides_swapped(&sw).unwrap();
        for (a, b) in back.forces.iter().zip(&sc.forces) {
            assert!(near((a.position.lat, a.position.lon), (b.position.lat, b.position.lon)), "{name}: swapping twice moves {}", a.id);
        }
    }
}
