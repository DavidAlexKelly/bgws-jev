//! The air domain in real games (§8.1, §8.2, §8.8, §18 phase 6): the m5
//! and m8 presets play with aircraft, munitions, radars and air defence;
//! a Shahed raid against a layered defence gives an intercept rate in the
//! [P10] band; sorties per aircraft per day sit in the [P30] band; runs are
//! deterministic by seed and thread count.

use bgws_core::{LogEvent, Seed, Side, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::domains::air;
use bgws_sim::{default_figures, Sim};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn scenario_json(name: &str) -> Value {
    let path = format!(
        "{}/../../data/scenarios/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn scenario(name: &str) -> ScenarioV4 {
    ScenarioV4::from_json(&scenario_json(name).to_string()).unwrap()
}

fn kinds(ev: &[LogEvent]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in ev {
        *m.entry(e.kind.clone()).or_insert(0) += 1;
    }
    m
}

fn triggers(ev: &[LogEvent]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in ev.iter().filter(|e| e.kind == "decide.question") {
        if let Some(t) = e.data.get("trigger").and_then(|t| t.as_str()) {
            *m.entry(t.to_string()).or_insert(0) += 1;
        }
    }
    m
}

/// A force record.
#[allow(clippy::too_many_arguments)]
fn force(
    id: &str,
    side: &str,
    platform: &str,
    count: u32,
    lat: f64,
    lon: f64,
    h: f64,
    task: Option<Value>,
) -> Value {
    let mut f = json!({ "id": id, "label": id, "side": side, "platform": platform, "count": count, "quality": "regular", "position": { "lat": lat, "lon": lon, "h": h }, "layer": "entity" });
    if let Some(t) = task {
        f["task"] = t;
    }
    f
}

/// A vignette on flat ground around (54.2, 18.6), built from the m5 shell.
fn vignette(name: &str, forces: Vec<Value>, minutes: f64) -> ScenarioV4 {
    let mut v = scenario_json("m5-air-over-the-battle");
    v["name"] = json!(name);
    v["forces"] = Value::Array(forces);
    v["victory"] = json!([]);
    v["theatre"]["ground"] = json!("flat");
    v["theatre"].as_object_mut().unwrap().remove("bounds");
    v["scope"]["durationMin"] = json!(minutes);
    v["scope"]["planners"] = json!([]);
    ScenarioV4::from_json(&v.to_string()).unwrap()
}

/// A layered defence of a depot against a Shahed raid: 36 armed drones and
/// 24 decoys arriving together from 125–127 km [P10].
fn shahed_raid() -> ScenarioV4 {
    let (lat0, lon0) = (54.2, 18.6);
    let mut f = vec![
        force("b-depot", "blue", "generic/depot", 1, lat0, lon0, 0.0, None),
        force(
            "b-sam",
            "blue",
            "generic/samMedium",
            2,
            lat0 - 0.02,
            lon0,
            0.0,
            None,
        ),
        force(
            "r-hq",
            "red",
            "generic/cp-company",
            2,
            lat0 + 1.6,
            lon0,
            0.0,
            None,
        ),
    ];
    f.push(force(
        "b-shorad",
        "blue",
        "generic/shorad",
        2,
        lat0 + 0.04,
        lon0,
        0.0,
        None,
    ));
    f.push(force(
        "b-gun",
        "blue",
        "generic/counterDrone",
        2,
        lat0 + 0.015,
        lon0 + 0.01,
        0.0,
        None,
    ));
    f.push(force(
        "b-idrone",
        "blue",
        "generic/interceptorTeam",
        1,
        lat0 + 0.03,
        lon0 - 0.02,
        0.0,
        None,
    ));
    // The raid arrives together, on an arc 125–131 km out, from the north.
    for i in 0..60 {
        let decoy = i % 5 == 1 || i % 5 == 3;
        let bearing = (-30.0 + 60.0 * i as f64 / 59.0f64).to_radians();
        let r_deg = (125.0 + ((i * 7) % 5) as f64 * 0.5) / 111.0;
        let lat = lat0 + r_deg * bearing.cos();
        let lon = lon0 + r_deg * bearing.sin() / lat0.to_radians().cos();
        let task = json!({ "kind": "strike", "aimAt": { "lat": lat0 + 0.002 * ((i % 3) as f64 - 1.0), "lon": lon0, "h": 0.0 }, "altM": 1500.0 });
        f.push(force(
            &format!("r-owa-{i}"),
            "red",
            if decoy {
                "generic/owaDecoy"
            } else {
                "generic/owaDrone"
            },
            1,
            lat,
            lon,
            1500.0,
            Some(task),
        ));
    }
    vignette("shahed-raid", f, 75.0)
}

#[test]
fn m5_air_over_the_battle_plays() {
    let sc = scenario("m5-air-over-the-battle");
    let mut sim = Sim::new(&sc, Seed(5), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(2.0 * 3600.0));
    let ev = sim.drain_events();
    let k = kinds(&ev);
    let t = triggers(&ev);
    eprintln!("m5: {k:#?}\ntriggers {t:?}");
    for kind in [
        "air.takeoff",
        "air.land",
        "air.shoot",
        "ad.engage",
        "ad.intercept",
        "munition.impact",
        "air.esmFix",
        "air.release",
    ] {
        assert!(k.get(kind).copied().unwrap_or(0) > 0, "no {kind}");
    }
    assert!(
        ev.iter()
            .any(|e| e.kind == "ad.intercept" && e.data["kill"] == json!(true)),
        "no interceptor ever killed anything"
    );
    for trig in ["trackAppears", "targetInWez", "missileWarning"] {
        assert!(t.get(trig).copied().unwrap_or(0) > 0, "no {trig} question");
    }
    let blue = air::defence::ledger(&sim, Side::Blue);
    assert!(blue.shots > 0 && blue.interceptor_cost > 0.0, "{blue:?}");
    // The air-superiority helper answers for the front.
    let front = bgws_core::Geodetic::new(54.47, 18.6, 0.0);
    let a = air::air_superiority(&sim, front, 25_000.0);
    eprintln!("air superiority over the front: {a:?}");
}

#[test]
fn m8_joint_strike_flies_sead() {
    let sc = scenario("m8-joint-strike");
    let mut sim = Sim::new(&sc, Seed(8), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(2.0 * 3600.0));
    let ev = sim.drain_events();
    let k = kinds(&ev);
    eprintln!("m8: {k:#?}\ntriggers {:?}", triggers(&ev));
    assert!(k.get("air.takeoff").copied().unwrap_or(0) > 0);
    assert!(
        ev.iter()
            .any(|e| e.kind == "air.tasked" && e.text.contains("SEAD")),
        "no SEAD tasked"
    );
    assert!(
        ev.iter()
            .any(|e| e.kind == "air.release" && e.data["sead"] == json!(true)),
        "no ARM released"
    );
    assert!(k.get("munition.impact").copied().unwrap_or(0) > 0);
    assert!(
        k.get("ad.engage").copied().unwrap_or(0) > 0,
        "red AD never fired"
    );
}

/// [P10]: about 85% of a Shahed raid intercepted by a layered defence;
/// decoys draw fire; the cost exchange is logged.
#[test]
fn shahed_raid_intercept_rate_in_band() {
    let sc = shahed_raid();
    let mut rates = Vec::new();
    let mut decoy_shots = 0;
    for seed in 1..=4u64 {
        let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(75.0 * 60.0));
        let ev = sim.drain_events();
        let l = air::defence::ledger(&sim, Side::Blue);
        let real = 36.0;
        let rate = (l.kills - l.decoys_killed) as f64 / real;
        eprintln!(
            "seed {seed}: {l:?} rate {rate:.2} cx {:.2}",
            l.cost_exchange()
        );
        assert!(ev.iter().any(|e| e.kind == "ad.costExchange"));
        rates.push(rate);
        decoy_shots += l.decoy_shots;
    }
    let mean = rates.iter().sum::<f64>() / rates.len() as f64;
    assert!(
        (0.75..=0.95).contains(&mean),
        "intercept rate {mean:.2} ({rates:?})"
    );
    assert!(decoy_shots > 0, "decoys should draw fire");
}

/// [P30]: fighters fly 2–3 sorties a day in a surge.
#[test]
fn sortie_rate_in_band() {
    let f = vec![
        force(
            "b-base",
            "blue",
            "generic/airbase",
            1,
            54.2,
            18.6,
            0.0,
            None,
        ),
        force(
            "r-hq",
            "red",
            "generic/cp-company",
            2,
            55.4,
            18.6,
            0.0,
            None,
        ),
        // Something of the enemy's that flies, so the base keeps a CAP up.
        force("r-owa", "red", "generic/owaDrone", 1, 55.4, 18.7, 0.0, None),
    ];
    let sc = vignette("sorties", f, 24.0 * 60.0);
    let mut sim = Sim::new(&sc, Seed(30), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(24.0 * 3600.0));
    let ev = sim.drain_events();
    let flights = ev
        .iter()
        .filter(|e| e.kind == "air.base")
        .map(|e| e.data["flights"].as_u64().unwrap())
        .sum::<u64>() as f64;
    let sorties = ev.iter().filter(|e| e.kind == "air.takeoff").count() as f64;
    let per = sorties / flights;
    eprintln!("{sorties} sorties by {flights} flights in a day: {per:.2}");
    assert!(
        (1.8..=3.2).contains(&per),
        "sorties per aircraft per day {per:.2}"
    );
}

#[test]
fn same_seed_same_air_game() {
    let sc = shahed_raid();
    let run = |seed| {
        let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(50.0 * 60.0));
        sim.fingerprint()
    };
    assert_eq!(run(3), run(3));
    assert_ne!(run(3), run(4));
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| run(9));
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| run(9));
    assert_eq!(one, many);
}

#[test]
fn m5_is_deterministic_across_threads() {
    let sc = scenario("m5-air-over-the-battle");
    let run = || {
        let mut sim = Sim::new(&sc, Seed(11), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(1800.0));
        sim.fingerprint()
    };
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(run);
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(run);
    assert_eq!(one, many);
}
