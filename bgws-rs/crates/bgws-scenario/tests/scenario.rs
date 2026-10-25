//! Acceptance checks for the scenario formats (§14.3): the shipped v4
//! scenarios load and validate against the catalogue; they are exactly what
//! the converter makes of the TS presets; the conversion keeps authored
//! metres; validation catches broken scenarios with typed errors; the game
//! record round-trips through a file; and no loader panics on bad input.

use bgws_catalogue::Catalogue;
use bgws_core::{Answer, AnswerSource, Enu, EnuFrame, QuestionId, Seed, Side, SimTime};
use bgws_scenario::convert::{ts_latlng_of, ts_offset_of};
use bgws_scenario::v3::LatLng;
use bgws_scenario::*;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn catalogue() -> Catalogue {
    Catalogue::builtin().expect("catalogue")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v3")
}

fn fixture(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join(name)).expect("fixture")).expect("json")
}

fn v4_of(name: &str) -> ScenarioV4 {
    let v3 = parse_v3(&fixture(name)).expect("valid v3");
    convert_v3(&v3, Some(&catalogue()), &ConvertOptions::default()).expect("converts")
}

#[test]
fn shipped_scenarios_load_and_validate() {
    let cat = catalogue();
    let all = builtin_scenarios().expect("all parse");
    assert_eq!(all.len(), 14);
    for s in &all {
        let problems = validate(s, &cat);
        assert!(problems.is_empty(), "{}: {problems:?}", s.name);
    }
    let skirmishes = all.iter().filter(|s| s.scope.kind == ScenarioType::Skirmish).count();
    assert!(skirmishes >= 7, "{skirmishes}");
}

#[test]
fn shipped_scenarios_are_what_the_converter_makes() {
    for (file, text) in BUILTIN_SCENARIOS {
        let shipped = ScenarioV4::from_json(text).unwrap();
        let v3_name = if file.starts_with("skirmish-") { file.to_string() } else { format!("M{}.json", &file[1..2]) };
        assert_eq!(v4_of(&v3_name), shipped, "{file}: regenerate with the export_v3 example");
    }
}

#[test]
fn conversion_keeps_authored_metres() {
    let s = v4_of("M7.json");
    let frame = EnuFrame::new(TS_DEFAULT_ORIGIN);
    // presets.ts puts r-asm-1 at 150 km east, 10 km south; the destroyer at the origin; a drone at 500 m.
    let asm = frame.geodetic_to_enu(s.force("r-asm-1").unwrap().position);
    assert!((asm.e - 150_000.0).abs() < 0.5 && (asm.n + 10_000.0).abs() < 0.5, "{asm:?}");
    let ddg = s.force("b-ddg").unwrap().position;
    assert!(ddg.surface_distance(TS_DEFAULT_ORIGIN) < 0.01);
    assert_eq!(s.force("r-owa-1").unwrap().position.h, 500.0);
    assert_eq!(s.force("b-ddg").unwrap().parent.as_deref(), Some("b-port"));
    assert!(matches!(s.force("r-owa-1").unwrap().task, Some(EntityTask::Strike { .. })));
    // Keeping lat/lng instead leaves the v3 positions as they were.
    let v3 = parse_v3(&fixture("M7.json")).unwrap();
    let kept = convert_v3(&v3, None, &ConvertOptions { placement: Placement::KeepLatLon, ..Default::default() }).unwrap();
    let p = v3.forces.iter().find(|f| f.id == "r-asm-1").unwrap().position;
    let k = kept.force("r-asm-1").unwrap().position;
    assert_eq!((k.lat, k.lon), (p.lat, p.lng));
    // At theatre ranges the two placements differ, which is why the choice is recorded.
    assert!(k.surface_distance(s.force("r-asm-1").unwrap().position) > 50.0);
    assert!(s.converted_from.as_ref().unwrap().notes.iter().any(|n| n.contains("ENU")));
}

#[test]
fn conversion_carries_the_rest() {
    let m2 = v4_of("M2.json");
    // 05:30 local at 18.6° E.
    let h = bgws_scenario::time::local_solar_hour(m2.start_unix_s().unwrap(), m2.theatre.origin.lon);
    assert!((h - 5.5).abs() < 1e-3, "{h}");
    assert_eq!(m2.breakpoint_pct(Side::Blue), Some(50.0));
    assert_eq!(m2.world.obstacles.len(), 1);
    assert_eq!(m2.command.side(Side::Blue).units, Decider::Rules);
    let m3 = v4_of("M3.json");
    assert_eq!(m3.command.hqs.len(), 5);
    assert_eq!(m3.command.hqs.iter().find(|h| h.id == "blue-bn").unwrap().decider, Some(Decider::Claude));
    let m8 = v4_of("M8.json");
    assert_eq!(m8.world.network.get(&Side::Blue), Some(&Network::Datalink));
    assert_eq!(m8.world.cyber.len(), 1);
    assert_eq!(m8.force("b-eo").unwrap().provider.as_deref(), Some("commercial"));
    assert_eq!(m8.force("b-ssn").unwrap().position.h, -100.0);
    assert!(m8.victory.iter().any(|v| matches!(v, VictoryCondition::TargetSet { .. })));
    let me = v4_of("skirmish-meeting-engagement-v1.json");
    assert!(me.force("red-recce").unwrap().concealed);
    assert!(me.force("blue-decoy").unwrap().dummy);
    assert_eq!(me.force("blue-1").unwrap().quality, TroopQuality::Veteran);
    assert!(!me.notes.is_empty());
    assert_eq!(me.seed_text.as_deref(), Some("meeting-engagement-v1"));
    // A file far from the TS origin gets its own origin.
    let mut far = fixture("M2.json");
    for f in far["forces"].as_array_mut().unwrap() {
        f["position"]["lng"] = json!(f["position"]["lng"].as_f64().unwrap() + 60.0);
    }
    let s = load_value(far, None, &ConvertOptions::default()).unwrap();
    assert!((s.theatre.origin.lon - 78.6).abs() < 0.1, "{:?}", s.theatre.origin);
}

#[test]
fn ts_projection_helpers_invert() {
    let p = ts_latlng_of(TS_DEFAULT_ORIGIN, -800.0, 3300.0);
    let (e, n) = ts_offset_of(TS_DEFAULT_ORIGIN, LatLng { lat: p.lat, lng: p.lng });
    assert!((e + 800.0).abs() < 1e-6 && (n - 3300.0).abs() < 1e-6);
}

#[test]
fn validation_catches_broken_scenarios() {
    let cat = catalogue();
    let mut s = v4_of("M3.json");
    let mut dup = s.forces[0].clone();
    dup.platform = "generic/nothing".into();
    s.forces.push(dup);
    let frame = EnuFrame::new(s.theatre.origin);
    s.forces[1].position = frame.enu_to_geodetic(Enu::new(5_000_000.0, 0.0, 0.0));
    s.command.hqs[0].subordinates.push("ghost".into());
    s.behaviours.assign.insert(s.forces[2].id.clone(), "custom/missing".into());
    s.start_utc = "tomorrow".into();
    s.scope.set(ScopeDomain::Land, DomainMode::Off);
    let errs = validate(&s, &cat);
    let has = |f: &dyn Fn(&ScenarioError) -> bool| errs.iter().any(|e| f(e));
    assert!(has(&|e| matches!(e, ScenarioError::DuplicateId { .. })));
    assert!(has(&|e| matches!(e, ScenarioError::UnknownPlatform { platform, .. } if platform == "generic/nothing")));
    assert!(has(&|e| matches!(e, ScenarioError::OutOfTheatre { .. })));
    assert!(has(&|e| matches!(e, ScenarioError::UnknownReference { target, .. } if target == "ghost")));
    assert!(has(&|e| matches!(e, ScenarioError::UnknownTemplate { .. })));
    assert!(has(&|e| matches!(e, ScenarioError::Invalid { field, .. } if field == "startUtc")));
    assert!(has(&|e| matches!(e, ScenarioError::Scope(_))));
}

#[test]
fn v4_round_trips_and_loads_from_disk() {
    let s = v4_of("M6.json");
    let text = s.to_json_pretty();
    assert_eq!(ScenarioV4::from_json(&text).unwrap(), s);
    assert_eq!(load_any(&text, None).unwrap(), s);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("bgws-scenario-test-m6.json");
    s.save(&path).unwrap();
    assert_eq!(load_path(&path, None).unwrap(), s);
    // A v3 file on disk converts on load.
    let v3 = load_path(&fixtures().join("M4.json"), Some(&catalogue())).unwrap();
    assert_eq!(v3, v4_of("M4.json"));
    assert!(matches!(load_path(Path::new("/no/such/file.json"), None), Err(LoadError::Io { .. })));
}

#[test]
fn game_record_round_trips() {
    let s = v4_of("skirmish-symmetric-control-v1.json");
    let mut r = GameRecord::new(s.clone(), s.seed, BuildInfo::this("test"), MachineInfo::compiled_for());
    r.figure_overrides.insert("catalogue.derive.fuel_reserve".into(), 0.2);
    r.push_command(SimTime::from_secs_i(30), "blue-hq", Some(Side::Blue), json!({"task": "attack", "objective": [54.21, 18.6]}));
    let ans = Answer {
        question: QuestionId(7),
        choice: "hold".into(),
        probabilities: Some([("hold".to_string(), 0.7), ("advance".to_string(), 0.3)].into_iter().collect()),
        confidence: Some(0.7),
        source: AnswerSource::Jev,
        plan: None,
        cost_usd: Some(0.001),
        latency_ms: Some(800),
        rationale: Some("enemy in overwatch".into()),
    };
    r.push_answer(SimTime::from_secs_i(45), ans.clone());
    r.push_snapshot(RecordedSnapshot::bytes(SimTime::from_secs_i(60), &[0, 1, 2, 250, 255]));
    r.push_snapshot(RecordedSnapshot::json(SimTime::from_secs_i(120), json!({"tick": 120})));
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bgws-record-test.json");
    r.save(&path).unwrap();
    let back = GameRecord::load(&path).unwrap();
    assert_eq!(back, r);
    let replayed = back.replay_answer(QuestionId(7)).unwrap();
    assert_eq!(replayed.choice, "hold");
    assert_eq!(replayed.source, AnswerSource::Recorded);
    assert!(back.replay_answer(QuestionId(8)).is_none());
    assert_eq!(back.snapshot_before(SimTime::from_secs_i(100)).unwrap().to_bytes().unwrap(), vec![0, 1, 2, 250, 255]);
    assert_eq!(back.commands_between(SimTime::ZERO, SimTime::from_secs_i(31)).count(), 1);
    assert_eq!(back.seed, Seed(s.seed.0));
    assert!(matches!(GameRecord::from_json("{}"), Err(RecordError::Format(_))));
    assert!(matches!(GameRecord::from_json("nope"), Err(RecordError::Json(_))));
}

/// Replace the value at a random path of a JSON tree with a random value.
fn mutate(v: &mut Value, path: &[usize], with: Value) {
    let Some((&first, rest)) = path.split_first() else {
        *v = with;
        return;
    };
    match v {
        Value::Object(m) if !m.is_empty() => {
            let k = m.keys().nth(first % m.len()).cloned().unwrap();
            if rest.is_empty() {
                m.insert(k, with);
            } else {
                mutate(m.get_mut(&k).unwrap(), rest, with);
            }
        }
        Value::Array(a) if !a.is_empty() => {
            let i = first % a.len();
            if rest.is_empty() {
                a[i] = with;
            } else {
                mutate(&mut a[i], rest, with);
            }
        }
        other => *other = with,
    }
}

fn arb_json() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(|x| serde_json::Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)),
        ".{0,12}".prop_map(Value::from),
        Just(json!({"lat": 1e9, "lng": -1e9})),
        Just(json!([])),
        Just(json!({})),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, .. ProptestConfig::default() })]

    #[test]
    fn loaders_never_panic_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let text = String::from_utf8_lossy(&bytes);
        let _ = load_any(&text, None);
        let _ = GameRecord::from_json(&text);
        let _ = ScenarioV4::from_json(&text);
    }

    #[test]
    fn loaders_never_panic_on_mutated_files(which in 0usize..4, path in proptest::collection::vec(any::<usize>(), 1..6), with in arb_json()) {
        let cat = catalogue();
        let mut v = match which {
            0 => fixture("M7.json"),
            1 => fixture("M3.json"),
            2 => serde_json::from_str(BUILTIN_SCENARIOS[4].1).unwrap(),
            _ => serde_json::from_str(BUILTIN_SCENARIOS[11].1).unwrap(),
        };
        mutate(&mut v, &path, with);
        if let Ok(s) = load_value(v, Some(&cat), &ConvertOptions::default()) {
            let _ = validate(&s, &cat);
            let _ = s.to_json_pretty();
        }
    }
}
