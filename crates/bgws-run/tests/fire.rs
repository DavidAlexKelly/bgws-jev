//! Direct fire: a unit fires the weapon most likely to knock its target out
//! (not its first mount), and a guided missile lands when it arrives.

use bgws_core::{Seed, SimTime};
use bgws_run::scengen::{build, standard, GroundSpec};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::{Sim, SimOptions};

fn game(name: &str, seed: u64, minutes: f64) -> Vec<bgws_core::LogEvent> {
    let mut spec = standard().into_iter().find(|s| s.name == name).unwrap();
    spec.ground = GroundSpec::Flat;
    let sc = ScenarioV4::from_json(&build(&spec).unwrap().to_string()).unwrap();
    let mut sim = Sim::with_options(&sc, Seed(seed), bgws_run::default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap();
    bgws_run::experiments::play_with_rules(&mut sim, SimTime::from_secs(minutes * 60.0));
    sim.drain_events()
}

fn num(e: &bgws_core::LogEvent, k: &str) -> f64 {
    e.data.get(k).and_then(serde_json::Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn an_ifv_fires_its_missiles_beyond_its_cannons_reach_and_they_fly() {
    let mut beyond = 0;
    let mut checked = 0;
    for seed in 1..=3 {
        let ev = game("gen-company-attack", seed, 30.0);
        for (i, l) in ev.iter().enumerate().filter(|(_, e)| e.kind == "fire.launch") {
            let range = num(l, "rangeM");
            // The IFV's 30 mm cannon reaches 2 km; only its missile reaches farther.
            if l.text.contains("mech") && range > 2_000.0 {
                beyond += 1;
            }
            // The volley lands (a hit, a kill or a miss on that target) no sooner than its flight time.
            let flight = num(l, "flightS");
            if let Some(land) = ev[i + 1..].iter().find(|e| matches!(e.kind.as_str(), "fire.hit" | "fire.kill" | "fire.miss") && e.entity == l.entity && e.target == l.target && e.data.get("munition") == l.data.get("munition")) {
                let dt = (land.t - l.t).secs();
                assert!(dt + 1.0 >= flight, "landed {dt:.1} s after launch, flight {flight:.1} s");
                checked += 1;
            }
        }
    }
    assert!(beyond > 0, "no IFV fired a missile beyond its cannon's reach");
    assert!(checked > 0, "no missile volley landed");
}
