use bgws_core::{Seed, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::{default_figures, Sim, Viewer};

fn scenario(name: &str) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_skirmish_plays_to_a_result() {
    let sc = scenario("skirmish-advance-to-contact-v1");
    let mut sim = Sim::new(&sc, Seed(7), default_figures()).unwrap();
    let out = sim.run_with_rules(SimTime::from_secs(3.0 * 3600.0));
    let events = sim.drain_events();
    let kinds = |k: &str| events.iter().filter(|e| e.kind == k).count();
    eprintln!("outcome {out:?}; detects {}, fires {}, kills {}, questions {}", kinds("sense.detect"), kinds("fire.miss") + kinds("fire.hit") + kinds("fire.kill"), kinds("fire.kill"), kinds("decide.question"));
    assert!(kinds("sense.detect") > 0, "nobody saw anything");
    assert!(kinds("fire.kill") > 0, "nobody was hit");
    assert!(out.is_some(), "the game should end");
    let f = sim.view(Viewer::Umpire);
    assert_eq!(f.entities.len(), sc.forces.len());
    let blue = sim.view(Viewer::Blue);
    assert!(blue.entities.iter().all(|e| e.side == bgws_core::Side::Blue));
}

#[test]
fn same_seed_same_game() {
    let sc = scenario("skirmish-meeting-engagement-v1");
    let run = |seed| {
        let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(1800.0));
        sim.fingerprint()
    };
    assert_eq!(run(3), run(3));
    assert_ne!(run(3), run(4));
}

#[test]
fn single_thread_matches_many() {
    let sc = scenario("skirmish-meeting-engagement-v1");
    let run = || {
        let mut sim = Sim::new(&sc, Seed(11), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(1200.0));
        sim.fingerprint()
    };
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(run);
    let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(run);
    assert_eq!(one, many);
}
