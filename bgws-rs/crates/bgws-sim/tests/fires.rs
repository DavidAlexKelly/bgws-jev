//! The fires, engineers and sustainment domain in real games (RUST_ENGINE.md
//! §8.8 indirect fire, §8.9, §8.10 casualties, §8.11) and the phase 3 and 7
//! "done when" vignettes (§18): breach time [P13], counter-battery survival
//! [P23], culmination when supply is cut [R7], casualty evacuation delays
//! [R7].

use bgws_core::{Answer, AnswerSource, Enu, LogEvent, Seed, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Activity, Act, Health, MoveModeS};
use bgws_sim::domains::fires::Stores;
use bgws_sim::{default_figures, Sim, SimOptions, Step};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn scenario(name: &str) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn kinds(ev: &[LogEvent]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in ev {
        *m.entry(e.kind.clone()).or_insert(0) += 1;
    }
    m
}

const LAT: f64 = 54.2;
const LON: f64 = 18.6;

fn at(e: f64, n: f64) -> Value {
    json!({ "lat": LAT + n / 111_320.0, "lon": LON + e / (111_320.0 * LAT.to_radians().cos()), "h": 0.0 })
}

/// A force: (id, side, platform, count, east m, north m).
type F<'a> = (&'a str, &'a str, &'a str, u32, f64, f64);

/// A small vignette on flat ground built on M2's frame: these forces and
/// obstacles, these domain modes, no HQ planners (the test gives the orders).
fn vignette(forces: &[F], obstacles: Value, domains: &[(&str, &str)], events: Value) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/m2-mech-company-hasty-attack.json", env!("CARGO_MANIFEST_DIR"));
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["theatre"]["ground"] = json!("flat");
    v["theatre"].as_object_mut().unwrap().remove("bounds");
    v["scope"]["planners"] = json!([]);
    v["scope"]["durationMin"] = json!(600.0);
    for (k, m) in domains {
        v["scope"]["domains"][*k] = json!(m);
    }
    v["forces"] = Value::Array(forces.iter().map(|(id, side, p, n, e, nn)| json!({ "id": id, "label": id, "side": side, "platform": p, "count": n, "quality": "regular", "position": at(*e, *nn), "layer": "unit" })).collect());
    v["world"]["obstacles"] = obstacles;
    v["events"] = events;
    ScenarioV4::from_json(&v.to_string()).unwrap()
}

fn sim(sc: &ScenarioV4, seed: u64) -> Sim {
    Sim::with_options(sc, Seed(seed), default_figures(), SimOptions { flat_ground: true, plan_at_start: false, weather: None, terrain: None }).unwrap()
}

fn entity(s: &Sim, name: &str) -> bevy_ecs::prelude::Entity {
    s.ctx.entity(s.ctx.by_name[name])
}

fn order_move(s: &mut Sim, name: &str, route: Vec<Enu>) {
    let e = entity(s, name);
    s.world.get_mut::<Act>(e).unwrap().activity = Activity::Move { route, mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Hold)) };
}

/// Play to `secs`, answering with `pick(agent, trigger)` where it says, else the rules.
fn play(s: &mut Sim, secs: f64, pick: impl Fn(&str, &str) -> Option<&'static str>) -> Vec<LogEvent> {
    let mut ev = Vec::new();
    loop {
        match s.advance(SimTime::from_secs(secs)) {
            Step::Advanced | Step::Over(_) => break,
            Step::NeedsAnswers { questions, .. } => {
                let mut a = s.rules_answers(&questions);
                for (q, ans) in questions.iter().zip(a.iter_mut()) {
                    if let Some(c) = pick(&q.agent, &q.trigger) {
                        if q.has_option(c) {
                            *ans = Answer::simple(q, c, AnswerSource::Rules);
                        }
                    }
                }
                s.answer(a).unwrap();
            }
        }
        ev.extend(s.drain_events());
    }
    ev.extend(s.drain_events());
    ev
}

fn rules(_: &str, _: &str) -> Option<&'static str> {
    None
}

// ── In real games ────────────────────────────────────────────────────────

/// M3: both sides' mortars answer calls for fire, adjust onto what their
/// observers see, fire for effect and displace; the minefield stops the
/// attack until lanes are made.
#[test]
fn battalion_attack_uses_fires_and_breaches() {
    let sc = scenario("m3-battalion-attack");
    let mut s = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let ev = play(&mut s, 3600.0, rules);
    let k = kinds(&ev);
    eprintln!("{k:?}");
    for kind in ["fires.request", "fires.adjust", "fires.impact", "fires.complete", "fires.displace", "land.obstacleFound", "land.obstacle", "land.breach"] {
        assert!(k.get(kind).copied().unwrap_or(0) > 0, "no {kind}");
    }
    // The shooters were asked, and the time of flight came from the ballistics.
    assert!(ev.iter().any(|e| e.kind == "decide.question" && e.data["trigger"] == "fireRequestReceived"));
    let fire = ev.iter().find(|e| e.kind == "fires.fire" || e.kind == "fires.adjust").unwrap();
    let tof = fire.data["tofS"].as_f64().unwrap();
    assert!(tof > 10.0 && tof < 90.0, "mortar time of flight {tof}");
}

/// M6 (logistics and medical simulated): artillery fires, casualties are
/// triaged and evacuated, and outcomes are logged.
#[test]
#[ignore = "slow (≈10 s): six hours of the sustained offensive"]
fn sustained_offensive_runs_the_chain() {
    let sc = scenario("m6-sustained-offensive");
    let mut s = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let ev = play(&mut s, 6.0 * 3600.0, rules);
    let k = kinds(&ev);
    eprintln!("{k:?}");
    for kind in ["fires.fire", "fires.impact", "medical.casualties", "medical.evacuation"] {
        assert!(k.get(kind).copied().unwrap_or(0) > 0, "no {kind}");
    }
}

// ── Phase 3: breach time [P13] ───────────────────────────────────────────

fn breach_once(seed: u64) -> Option<f64> {
    let sc = vignette(
        &[("b-tanks", "blue", "generic/mbt", 4, 0.0, -1200.0), ("b-eng", "blue", "generic/engineerSection", 2, 300.0, -1600.0), ("r-inf", "red", "generic/infantrySection", 1, 0.0, 20_000.0)],
        json!([{ "owner": "red", "kind": "minefield", "at": at(0.0, 0.0), "widthM": 1500.0, "depthM": 500.0, "bearingDeg": 0.0, "density": 0.5 }]),
        &[("fires", "off"), ("logistics", "off"), ("medical", "off")],
        json!([]),
    );
    let mut s = sim(&sc, seed);
    let goal = s.ctx.frame.geodetic_to_enu(serde_json::from_value(at(0.0, 2000.0)).unwrap());
    order_move(&mut s, "b-tanks", vec![goal]);
    let ev = play(&mut s, 4.0 * 3600.0, rules);
    let e = entity(&s, "b-tanks");
    let through = s.world.get::<bgws_sim::components::Kin>(e).unwrap().pos.n > 300.0;
    let h = s.world.get::<Health>(e).unwrap();
    let done = ev.iter().find(|e| e.kind == "land.breach" && e.data["phase"] == "done").and_then(|e| e.data["seconds"].as_f64());
    let mines = ev.iter().filter(|e| e.kind == "land.mine").count();
    eprintln!("seed {seed}: breach {done:?}, through {through}, mines {mines}, tanks fit {} immobilised {}", h.fit, h.mobility_kills);
    // Through the lane unless the mines left it unable to move.
    if done.is_some() && h.can_move() {
        assert!(through, "seed {seed}: a lane was made but the tanks did not go through");
    }
    done
}

/// The breach vignette: an engineer section clears a 6 m lane through a
/// 500 m belt in about 1.5 h [P13]; the median over seeds is in 0.5–1.5×.
#[test]
fn breach_time_in_band() {
    let mut t: Vec<f64> = (1..=6).filter_map(breach_once).collect();
    assert!(t.len() >= 5, "breaches finished: {t:?}");
    t.sort_by(f64::total_cmp);
    let median = t[t.len() / 2];
    eprintln!("breach seconds {t:?}, median {median}");
    assert!((0.5 * 5400.0..=1.5 * 5400.0).contains(&median), "median breach {median} s");
}

// ── Phase 3: counter-battery survival [P23] ──────────────────────────────

fn cb_forces(medical: bool) -> ScenarioV4 {
    vignette(
        &[
            ("b-gun", "blue", "generic/sph155", 3, 0.0, -15_000.0),
            ("b-recce", "blue", "generic/recce", 2, 0.0, -3000.0),
            ("r-inf-1", "red", "generic/infantrySection", 1, 0.0, 0.0),
            ("r-inf-2", "red", "generic/infantrySection", 1, 250.0, 100.0),
            ("r-inf-3", "red", "generic/infantrySection", 1, -250.0, 100.0),
            ("r-cbr", "red", "generic/cbRadar", 1, 0.0, 5000.0),
            ("r-gun", "red", "generic/sph155", 3, 1000.0, 8000.0),
        ],
        json!([]),
        &[("fires", "simulated"), ("logistics", "off"), ("medical", if medical { "simulated" } else { "off" })],
        json!([]),
    )
}

/// (blue gun lost vehicles, red counter-battery fixes) for one game.
fn cb_once(seed: u64, scoot: bool) -> (u32, usize) {
    let sc = cb_forces(false);
    let mut s = sim(&sc, seed);
    let ev = play(&mut s, 2400.0, |agent, trigger| match (agent, trigger) {
        ("b-gun", "fireRequestReceived") => Some(if scoot { "fireThenScoot" } else { "fireNow" }),
        ("b-gun", "counterBatteryThreat") => Some(if scoot { "displaceNow" } else { "stay" }),
        _ => None,
    });
    let h = s.world.get::<Health>(entity(&s, "b-gun")).unwrap();
    let fixes = ev.iter().filter(|e| e.kind == "fires.counterBattery").count();
    (h.start - h.fit, fixes)
}

/// The counter-battery vignette: a radar backtracks a gun that fires, and
/// its guns answer within minutes; a battery that stays is hit far more
/// often than one that shoots and scoots inside its displacement time [P23].
#[test]
fn counter_battery_survival_in_band() {
    let seeds = 1..=8u64;
    let stay: Vec<(u32, usize)> = seeds.clone().map(|k| cb_once(k, false)).collect();
    let scoot: Vec<(u32, usize)> = seeds.map(|k| cb_once(k, true)).collect();
    let survive = |v: &[(u32, usize)]| v.iter().filter(|x| x.0 == 0).count() as f64 / v.len() as f64;
    eprintln!("stay {stay:?} → {:.2}; scoot {scoot:?} → {:.2}", survive(&stay), survive(&scoot));
    assert!(stay.iter().all(|x| x.1 > 0), "the radar located the gun every time");
    assert!(survive(&scoot) >= 0.75, "shoot and scoot survives [P23]");
    assert!(survive(&stay) <= 0.5, "a battery that stays is found and hit");
}

// ── Phase 7: casualty evacuation delays [R7] ─────────────────────────────

/// Infantry shelled under observation take casualties; their evacuation
/// under fire takes 60–120 min to the collection point [R7], and every
/// outcome is logged.
#[test]
fn evacuation_delays_in_band() {
    let mut ccp = Vec::new();
    let mut outcomes = 0;
    for seed in 1..=4 {
        let sc = cb_forces(true);
        let mut s = sim(&sc, seed);
        let ev = play(&mut s, 4.0 * 3600.0, rules);
        for e in ev.iter().filter(|e| e.kind == "medical.died" || e.kind == "medical.survived") {
            outcomes += 1;
            if let Some(m) = e.data["ccpMin"].as_f64() {
                ccp.push(m);
            }
        }
        assert!(ev.iter().any(|e| e.kind == "medical.casualties"));
    }
    assert!(outcomes > 0 && !ccp.is_empty(), "no outcomes");
    ccp.sort_by(f64::total_cmp);
    let median = ccp[ccp.len() / 2];
    eprintln!("{outcomes} outcomes; minutes to the collection point {ccp:?}");
    assert!((60.0..=120.0).contains(&median), "median {median} min");
}

// ── Phase 7: culmination when supply is cut [R7] ─────────────────────────

fn supply_game(seed: u64, cut: bool) -> (Vec<LogEvent>, f64, bgws_core::Fingerprint) {
    let events = if cut { json!([{ "at": 30.0, "kind": "cutSupply", "side": "blue", "target": "b-depot" }, { "at": 30.0, "kind": "cutSupply", "side": "blue", "target": "b-rail" }]) } else { json!([]) };
    let sc = vignette(
        &[
            ("b-tanks-1", "blue", "generic/mbt", 4, 0.0, 0.0),
            ("b-tanks-2", "blue", "generic/mbt", 4, 600.0, 0.0),
            ("b-depot", "blue", "generic/depot", 1, 0.0, -6000.0),
            ("b-rail", "blue", "generic/railhead", 1, 0.0, -20_000.0),
            ("b-trucks", "blue", "generic/convoyTruck", 4, 200.0, -6000.0),
            ("r-inf", "red", "generic/infantrySection", 1, 0.0, 30_000.0),
        ],
        json!([]),
        &[("fires", "off"), ("logistics", "simulated"), ("medical", "off")],
        events,
    );
    let mut s = sim(&sc, seed);
    for (name, e) in [("b-tanks-1", 0.0), ("b-tanks-2", 600.0)] {
        let ent = entity(&s, name);
        let mut st = s.world.get_mut::<Stores>(ent).unwrap();
        st.fuel_l = 0.35 * st.fuel_max_l;
        let route: Vec<Enu> = (0..40).map(|k| Enu::new(e, if k % 2 == 0 { 4000.0 } else { 0.0 }, 0.0)).collect();
        order_move(&mut s, name, route);
    }
    let ev = play(&mut s, 5.0 * 3600.0, rules);
    let fuel: f64 = ["b-tanks-1", "b-tanks-2"].iter().map(|n| s.world.get::<Stores>(entity(&s, n)).unwrap().fuel_pct()).sum::<f64>() / 2.0;
    (ev, fuel, s.fingerprint())
}

/// Units on the move burn fuel; with the supply line standing, convoys from
/// the depot (topped up by rail) keep them going; cut it and they run dry
/// and the side culminates.
#[test]
fn culmination_when_supply_is_cut() {
    let (fed, fed_fuel, _) = supply_game(1, false);
    let (cut, cut_fuel, _) = supply_game(1, true);
    let kf = kinds(&fed);
    let kc = kinds(&cut);
    eprintln!("fed {kf:?} fuel {fed_fuel:.2}\ncut {kc:?} fuel {cut_fuel:.2}");
    assert!(kf.get("logistics.resupplied").copied().unwrap_or(0) > 0, "convoys delivered");
    assert!(kf.get("logistics.convoy").copied().unwrap_or(0) > 0, "convoys ran");
    assert!(!kf.contains_key("logistics.culmination"), "the supplied side did not culminate");
    assert!(kc.get("logistics.nodeStruck").copied().unwrap_or(0) > 0, "the cut is logged as interdiction");
    assert!(kc.get("logistics.culmination").copied().unwrap_or(0) > 0, "the cut side culminated");
    assert!(kc.get("logistics.outOfFuel").copied().unwrap_or(0) > 0, "it ran dry");
    assert!(fed_fuel > cut_fuel + 0.1);
}

#[test]
#[ignore = "exploration"]
fn explore_vignettes() {
    let sc = cb_forces(false);
    let mut s = sim(&sc, 1);
    let ev = play(&mut s, 2400.0, |a, t| match (a, t) {
        ("b-gun", "fireRequestReceived") => Some("fireNow"),
        ("b-gun", "counterBatteryThreat") => Some("stay"),
        _ => None,
    });
    for e in ev.iter().filter(|e| e.kind.starts_with("fires.") && (e.side == Some(bgws_core::Side::Red) || e.kind != "fires.impact")).take(40) {
        eprintln!("  {} {} {} {}", e.t, e.kind, e.text, if e.kind == "fires.counterBattery" || e.kind == "fires.impact" { e.data.to_string() } else { String::new() });
    }
    let sc = vignette(
        &[("b-tanks", "blue", "generic/mbt", 4, 0.0, -1200.0), ("b-eng", "blue", "generic/engineerSection", 2, 300.0, -1600.0), ("r-inf", "red", "generic/infantrySection", 1, 0.0, 20_000.0)],
        json!([{ "owner": "red", "kind": "minefield", "at": at(0.0, 0.0), "widthM": 1500.0, "depthM": 500.0, "bearingDeg": 0.0, "density": 0.5 }]),
        &[("fires", "off"), ("logistics", "off"), ("medical", "off")],
        json!([]),
    );
    let mut s = sim(&sc, 1);
    let goal = s.ctx.frame.geodetic_to_enu(serde_json::from_value(at(0.0, 2000.0)).unwrap());
    order_move(&mut s, "b-tanks", vec![goal]);
    let ev = play(&mut s, 4.0 * 3600.0, rules);
    for e in ev.iter().filter(|e| !e.kind.starts_with("sense") && e.kind != "people.morale").take(60) {
        eprintln!("  {} {} {} {}", e.t, e.kind, e.text, e.data);
    }
    eprintln!("tanks at {:?}", s.world.get::<bgws_sim::components::Kin>(entity(&s, "b-tanks")).unwrap().pos);
}

// ── Determinism (§12) ────────────────────────────────────────────────────

fn fingerprint(sc: &ScenarioV4, seed: u64, secs: f64) -> bgws_core::Fingerprint {
    let mut s = Sim::new(sc, Seed(seed), default_figures()).unwrap();
    s.run_with_rules(SimTime::from_secs(secs));
    s.fingerprint()
}

#[test]
fn fires_games_are_deterministic() {
    let m3 = scenario("m3-battalion-attack");
    assert_eq!(fingerprint(&m3, 5, 1800.0), fingerprint(&m3, 5, 1800.0));
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| fingerprint(&m3, 6, 1800.0));
    let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| fingerprint(&m3, 6, 1800.0));
    assert_eq!(one, many);
    // Logistics and medical simulated: the counter-battery vignette with medical, and the supply game.
    let cb = cb_forces(true);
    let run = |seed| {
        let mut s = sim(&cb, seed);
        play(&mut s, 1800.0, rules);
        s.fingerprint()
    };
    assert_eq!(run(3), run(3));
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| supply_game(2, true).2);
    let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| supply_game(2, true).2);
    assert_eq!(one, many);
}
