//! The scenario generator: its scenarios load and brief each side's mission,
//! prepared defences start dug in, and an objective decides the game.

use bgws_core::{Seed, Side, SimTime};
use bgws_run::scengen::{build, standard, GroundSpec, Kind, Spec};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::{default_figures, Sim};

fn load(spec: &Spec) -> ScenarioV4 {
    ScenarioV4::from_json(&build(spec).unwrap().to_string()).unwrap()
}

/// A standard spec, on flat ground (quick, and the same everywhere).
fn flat(name: &str) -> Spec {
    let mut s = standard().into_iter().find(|s| s.name == name).unwrap();
    s.ground = GroundSpec::Flat;
    s
}

#[test]
fn every_generated_scenario_loads() {
    for spec in standard().into_iter().filter(|s| !matches!(s.ground, GroundSpec::Pack { .. })) {
        let sc = load(&spec);
        assert!(sc.forces.iter().any(|f| f.side == Side::Blue) && sc.forces.iter().any(|f| f.side == Side::Red), "{}", spec.name);
        Sim::new(&sc, Seed(1), default_figures()).unwrap_or_else(|e| panic!("{}: {e:?}", spec.name));
    }
}

#[test]
fn each_side_is_briefed_its_mission() {
    let mission = |name: &str, side| {
        let sim = Sim::new(&load(&flat(name)), Seed(1), default_figures()).unwrap();
        sim.hq_situation(side, "plan").mission
    };
    assert_eq!(mission("gen-company-attack", Side::Blue).as_deref(), Some("attack"));
    assert_eq!(mission("gen-company-attack", Side::Red).as_deref(), Some("defend"));
    // The objective behind blue is a position to fall back to; red's to take.
    assert_eq!(mission("gen-withdrawal", Side::Blue).as_deref(), Some("withdraw"));
    assert_eq!(mission("gen-withdrawal", Side::Red).as_deref(), Some("attack"));
}

#[test]
fn a_prepared_defence_starts_dug_in() {
    let sc = load(&flat("gen-company-attack"));
    let sim = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let state = |id: &str| {
        let e = sim.ctx.entity(sim.ctx.by_name[id]);
        let dug = sim.world.get::<bgws_sim::domains::fires::Dig>(e).is_some_and(|d| d.done);
        let hull_down = sim.world.get::<bgws_sim::components::Act>(e).is_some_and(|a| a.posture == bgws_sim::components::Posture::HullDown);
        (dug, hull_down)
    };
    assert_eq!(state("r-1"), (true, true), "the defending platoon is dug in");
    assert_eq!(state("b-1"), (false, false), "the attacker is not");
}

#[test]
fn an_undefended_objective_is_taken_and_one_not_reached_loses() {
    // The attack with the defence gone: blue takes the objective and holds it.
    let mut spec = flat("gen-company-attack");
    spec.kind = Kind::Attack;
    let mut v = build(&spec).unwrap();
    v["forces"].as_array_mut().unwrap().retain(|f| f["side"] == "blue" || f["id"].as_str().is_some_and(|i| i.ends_with("hq") || i.ends_with("mortar")));
    // What red has left sits well back, out of the fight.
    for f in v["forces"].as_array_mut().unwrap().iter_mut().filter(|f| f["side"] == "red") {
        let lat = f["position"]["lat"].as_f64().unwrap();
        f["position"]["lat"] = serde_json::json!(lat + 0.07);
    }
    let hqs = v["command"]["hqs"].as_array_mut().unwrap();
    for h in hqs.iter_mut().filter(|h| h["side"] == "red") {
        h["subordinates"] = serde_json::json!(["r-mortar"]);
    }
    // Only the objective decides.
    v["victory"].as_array_mut().unwrap().retain(|c| c["kind"] == "objective");
    let sc = ScenarioV4::from_json(&v.to_string()).unwrap();
    let mut sim = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let out = sim.run_with_rules(SimTime::from_secs(90.0 * 60.0)).expect("the game ends");
    assert_eq!(out.winner, Some(Side::Blue), "{}", out.reason);
    assert!(out.reason.contains("objective"), "{}", out.reason);

    // Five minutes is too short to get there: blue fails its objective and loses.
    v["scope"]["durationMin"] = serde_json::json!(5.0);
    let sc = ScenarioV4::from_json(&v.to_string()).unwrap();
    let mut sim = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let out = sim.run_with_rules(SimTime::from_secs(90.0 * 60.0)).expect("the game ends");
    assert_eq!(out.winner, Some(Side::Red), "{}", out.reason);
    assert!(out.reason.contains("did not take its objective"), "{}", out.reason);
}
