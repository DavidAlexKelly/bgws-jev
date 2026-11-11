//! Real-time play (docs/design/INTEROP_PLAN.md §3.4): decisions do not stop
//! the clock, answers apply when they come back, and a real-time game
//! replays exactly from its record; with a pace, the clock follows the wall.

use bgws_core::{Seed, SimTime};
use bgws_run::{DeciderChoice, Host, HostOptions};
use bgws_scenario::v4::ScenarioV4;
use std::time::Instant;

fn scenario() -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/gen-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn rules() -> HostOptions {
    HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }
}

#[test]
fn a_real_time_game_replays_exactly() {
    let sc = scenario();
    let mut host = Host::new(&sc, Seed(3), bgws_run::default_figures(), &rules()).unwrap();
    let until = SimTime::from_secs(8.0 * 60.0);
    let mut ticks = 0;
    host.play_realtime(until, None, |_| ticks += 1).unwrap();
    let rec = host.finished_record();
    assert!(rec.run.realtime, "the record says it was played in real time");
    assert!(!rec.answers.is_empty(), "questions were asked and answered");
    // One tick a second, and second 0 too when the start asks the HQs for plans.
    assert!((8 * 60..=8 * 60 + 1).contains(&ticks) || rec.outcome.is_some(), "one tick each second ({ticks})");

    let mut replay = Host::replay(rec.clone(), bgws_run::default_figures()).unwrap();
    replay.play_realtime(rec.ends_at(), None, |_| {}).unwrap();
    replay.verify_replay().expect("the replay matches the record");
}

#[test]
fn with_a_pace_the_clock_follows_the_wall() {
    let sc = scenario();
    let mut host = Host::new(&sc, Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let started = Instant::now();
    // 30 simulated seconds at 10x: about 3 s of wall clock.
    host.play_realtime(SimTime::from_secs(30.0), Some(10.0), |_| {}).unwrap();
    let wall = started.elapsed().as_secs_f64();
    assert!((2.7..3.6).contains(&wall), "30 s at 10x took {wall:.2} s");
}
