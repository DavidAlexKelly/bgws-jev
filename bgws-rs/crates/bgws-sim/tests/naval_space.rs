//! Naval, space and cyber in real games (§8.6, §8.12; §18 phases 8 and 9):
//! the littoral and joint-strike presets play, the Red Sea magazine-depth
//! vignette sits in the R6.2 band, a satellite pass becomes image detections
//! in the side's picture, a cyber effect window opens and closes, and the
//! same seed gives the same game on one thread or four.

use bgws_core::{LogEvent, Seed, Side, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::domains::naval::NavalState;
use bgws_sim::domains::space::{self, SpaceState};
use bgws_sim::{default_figures, Sim};
use serde_json::{json, Value};

fn raw(name: &str) -> Value {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn scenario(name: &str) -> ScenarioV4 {
    ScenarioV4::from_json(&raw(name).to_string()).unwrap()
}

fn count(events: &[LogEvent], kind: &str) -> usize {
    events.iter().filter(|e| e.kind == kind).count()
}

#[test]
fn m7_littoral_plays_with_a_naval_fight() {
    let sc = scenario("m7-littoral-holding-the-strait");
    let mut sim = Sim::new(&sc, Seed(7), default_figures()).unwrap();
    let out = sim.run_with_rules(SimTime::from_secs(3.0 * 3600.0));
    let ev = sim.drain_events();
    let st = sim.world.resource::<NavalState>();
    eprintln!("m7: {out:?}\n{:?}", st.stats);
    assert!(count(&ev, "naval.radarContact") > 0, "ships and the drone see each other");
    assert!(count(&ev, "naval.salvo") > 0, "the coastal batteries fire");
    assert!(count(&ev, "naval.vampire") > 0, "ships detect what comes at them");
    assert!(count(&ev, "naval.raid") > 0, "ships fight the raids");
    assert!(count(&ev, "naval.intercept") > 0, "and kill some of it");
    assert!(st.stats.engagements > 0 && st.stats.missiles + st.stats.gun_rounds > 0);
    // Magazines deplete.
    let ddg = sim.ctx.entity(sim.ctx.by_name["b-ddg"]);
    let c = sim.world.get::<bgws_sim::domains::naval::ShipCombat>(ddg).unwrap();
    let (left, full) = c.sams();
    assert!(left < full, "the destroyer spent interceptors ({left} of {full})");
}

#[test]
fn m8_joint_strike_runs_space_and_cyber() {
    let sc = scenario("m8-joint-strike");
    let mut sim = Sim::new(&sc, Seed(3), default_figures()).unwrap();
    let out = sim.run_with_rules(SimTime::from_secs(3.0 * 3600.0));
    let ev = sim.drain_events();
    eprintln!("m8: {out:?}; {:?}", sim.world.resource::<SpaceState>().stats);
    assert!(count(&ev, "space.pass") > 0);
    assert!(count(&ev, "cyber.effect") + count(&ev, "cyber.failed") == 1, "the access is executed once");
    // GNSS and SATCOM helpers answer for the other domains.
    let g = space::gnss_geometry(&sim, sc.theatre.origin).expect("a GNSS fix");
    assert!(g.visible >= 4 && g.hdop > 0.3 && g.hdop < 5.0, "{g:?}");
    assert!(space::satcom_capacity_bps(&sim, Side::Blue, true) > 0.0);
}

/// A pass of the SAR satellite over the named areas becomes detections in
/// blue's track store after downlink and analysis.
#[test]
fn a_satellite_pass_produces_image_detections() {
    let sc = scenario("m8-joint-strike");
    let mut sim = Sim::new(&sc, Seed(5), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(2.5 * 3600.0));
    let ev = sim.drain_events();
    let imaged: Vec<&LogEvent> = ev.iter().filter(|e| e.kind == "space.imaged").collect();
    let delivered: Vec<&LogEvent> = ev.iter().filter(|e| e.kind == "space.imagery").collect();
    assert!(!imaged.is_empty(), "the satellite images on its pass");
    assert!(!delivered.is_empty(), "the imagery is delivered");
    let first_taken = imaged[0].t;
    assert!(delivered[0].t > first_taken, "after downlink and analysis");
    let st = sim.world.resource::<SpaceState>();
    assert!(st.stats.detections_delivered > 0, "{:?}", st.stats);
    // In blue's picture, from a satellite sensor.
    let store = &sim.ctx.tracks[&Side::Blue];
    let from_space = store
        .ids()
        .into_iter()
        .filter_map(|t| store.view(t, &sim.ctx.p.know))
        .filter(|v| v.sensors.iter().any(|s| matches!(s, bgws_know::SensorKind::Sar | bgws_know::SensorKind::Visual)))
        .count();
    assert!(from_space > 0, "imagery tracks in blue's store");
}

/// A short cyber effect opens and closes, and the multiplier other domains
/// read follows it.
#[test]
fn a_cyber_effect_window_opens_and_closes() {
    let mut v = raw("m8-joint-strike");
    v["world"]["cyber"][0]["durationS"] = json!(1800.0);
    v["world"]["cyber"][0]["pEffect"] = json!(1.0);
    v["world"]["cyber"][0]["detectionRisk"] = json!(0.0);
    let sc = ScenarioV4::from_json(&v.to_string()).unwrap();
    let mut sim = Sim::new(&sc, Seed(9), default_figures()).unwrap();
    let ad = bgws_ops::cyber::TargetProcess::AdTrackSharing;
    assert_eq!(space::cyber_multiplier(&sim, Side::Red, &ad), 1.0);
    sim.run_with_rules(SimTime::from_secs(600.0));
    let ev = sim.drain_events();
    let open = ev.iter().find(|e| e.kind == "cyber.effect").expect("the effect starts");
    assert_eq!(space::cyber_multiplier(&sim, Side::Red, &ad), 0.0, "red's AD loses track sharing");
    assert_eq!(space::cyber_multiplier(&sim, Side::Blue, &ad), 1.0, "only the victim suffers");
    assert_eq!(space::cyber_multiplier_named(&sim, Side::Red, "airDefence"), 0.0);
    sim.run_with_rules(SimTime::from_secs(3_000.0));
    let ev = sim.drain_events();
    let close = ev.iter().find(|e| e.kind == "cyber.effectEnds").expect("the effect ends");
    assert!((close.t.secs() - open.t.secs() - 1800.0).abs() < 2.0);
    assert_eq!(space::cyber_multiplier(&sim, Side::Red, &ad), 1.0);
}

/// The Red Sea vignette [R6.2]: one-way drones, decoys and anti-ship missiles
/// at a destroyer group. Across seeds the ships fire about 0.58 missiles per
/// engagement (band 0.3–1.0) because guns, CIWS and EW take the cheap threats.
fn red_sea(seed: u64) -> (u32, u32, u32, u32) {
    let mut v = raw("m7-littoral-holding-the-strait");
    let forces = v["forces"].as_array().unwrap().clone();
    let mut keep: Vec<Value> = forces.into_iter().filter(|f| !matches!(f["id"].as_str().unwrap(), "r-hq" | "m-1" | "m-2" | "r-usv-1" | "r-usv-2" | "r-usv-3" | "r-usv-4")).collect();
    // A second destroyer in company.
    let mut ddg2 = keep.iter().find(|f| f["id"] == "b-ddg").unwrap().clone();
    ddg2["id"] = json!("b-ddg-2");
    ddg2["label"] = json!("Blue destroyer 2");
    ddg2["position"]["lon"] = json!(ddg2["position"]["lon"].as_f64().unwrap() + 0.05);
    keep.push(ddg2);
    v["forces"] = Value::Array(keep);
    v["victory"] = json!([]);
    v["world"]["obstacles"] = json!([]);
    let sc = ScenarioV4::from_json(&v.to_string()).unwrap();
    let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(2.0 * 3600.0));
    let st = sim.world.resource::<NavalState>();
    if std::env::var("RAIDS").is_ok() {
        for r in &st.raids {
            eprintln!("{:>6.0} {:?} {:?} n{} shots {:?} kills {:?}", r.at.secs(), r.ship, r.layer, r.threats, r.outcome.shots_by_system.iter().filter(|x| *x.1 > 0).collect::<Vec<_>>(), r.outcome.kills_by_system.iter().filter(|x| *x.1 > 0).collect::<Vec<_>>());
        }
    }
    (st.stats.missiles, st.stats.engagements, st.stats.gun_rounds, st.stats.ship_hits)
}

#[test]
fn red_sea_missiles_per_engagement_in_band() {
    let (mut m, mut e, mut g, mut h) = (0, 0, 0, 0);
    for seed in 1..=4 {
        let r = red_sea(seed);
        eprintln!("seed {seed}: missiles {} engagements {} gun rounds {} hits {}", r.0, r.1, r.2, r.3);
        m += r.0;
        e += r.1;
        g += r.2;
        h += r.3;
    }
    let per = m as f64 / e.max(1) as f64;
    eprintln!("missiles per engagement {per:.2}, gun rounds per engagement {:.2}, hits {h}", g as f64 / e.max(1) as f64);
    assert!(e >= 40, "the group fights the raid ({e} engagements)");
    assert!((0.3..=1.0).contains(&per), "R6.2: about 0.58 missiles per engagement, got {per:.2}");
}

/// Undersea and littoral: a frigate's sonar finds a submarine, a merchant
/// runs into a minefield on its lane, and explosive USVs run in on the
/// frigate and meet its guns.
#[test]
fn sonar_mines_and_usvs() {
    let mut v = raw("m7-littoral-holding-the-strait");
    let forces = v["forces"].as_array().unwrap().clone();
    let mut keep: Vec<Value> = forces.into_iter().filter(|f| matches!(f["id"].as_str().unwrap(), "b-ffg" | "b-port" | "m-1" | "r-usv-1" | "r-usv-2" | "r-usv-3" | "r-usv-4")).collect();
    for (i, f) in keep.iter_mut().enumerate() {
        if f["id"].as_str().unwrap().starts_with("r-usv") {
            f["position"] = json!({ "lat": 54.20 + 0.01 * i as f64, "lon": 18.95, "h": 0.0 });
        }
    }
    keep.push(json!({ "id": "r-ssn", "label": "Red submarine", "side": "red", "platform": "generic/ssn", "count": 1, "position": { "lat": 54.25, "lon": 18.74, "h": -100.0 } }));
    v["forces"] = Value::Array(keep);
    v["victory"] = json!([]);
    v["world"]["obstacles"] = json!([{ "owner": "red", "kind": "seaMines", "at": { "lat": 54.456, "lon": 17.0, "h": 0.0 }, "widthM": 20000.0, "depthM": 10000.0, "bearingDeg": 0.0, "density": 3.0 }]);
    let sc = ScenarioV4::from_json(&v.to_string()).unwrap();
    let mut sim = Sim::new(&sc, Seed(4), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(2.5 * 3600.0));
    let ev = sim.drain_events();
    assert!(count(&ev, "naval.sonarContact") > 0, "sonar finds the submarine");
    assert!(count(&ev, "naval.mine") > 0, "the merchant strikes a mine");
    assert!(count(&ev, "naval.usvKilled") + count(&ev, "naval.usvHit") > 0, "the USVs meet the frigate");
    // Sonar contacts reach blue's picture as sea tracks.
    let store = &sim.ctx.tracks[&Side::Blue];
    assert!(store.audit().tracks.values().any(|t| t.contributions.iter().any(|c| c.sensor == bgws_know::SensorKind::Sonar)), "sonar fixes in blue's track audit");
}

fn fingerprint(name: &str, seed: u64, secs: f64) -> bgws_core::Fingerprint {
    let sc = scenario(name);
    let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(secs));
    sim.fingerprint()
}

#[test]
fn naval_and_space_games_are_deterministic() {
    for name in ["m7-littoral-holding-the-strait", "m8-joint-strike"] {
        let a = fingerprint(name, 21, 3600.0);
        assert_eq!(a, fingerprint(name, 21, 3600.0), "{name}: same seed, same game");
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| fingerprint(name, 21, 3600.0));
        let four = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| fingerprint(name, 21, 3600.0));
        assert_eq!(one, four, "{name}: thread count does not matter");
        assert_eq!(a, one);
    }
}
