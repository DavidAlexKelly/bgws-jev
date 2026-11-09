//! Game records replay exactly (§12), including what a person did between
//! steps: plans and seat changes come back at the same simulated time and
//! the same point in the answers. A branch replays to a moment and plays on.

use bgws_core::{SimDuration, SimTime, Side};
use bgws_run::{default_figures, load_scenario, router_for, DeciderChoice, Host, HostOptions};
use bgws_scenario::record::GameRecord;
use bgws_sim::{SeatKind, SimOptions, Step};
use serde_json::json;
use std::path::PathBuf;

fn scenario(name: &str) -> bgws_scenario::v4::ScenarioV4 {
    load_scenario(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios").join(format!("{name}.json"))).unwrap()
}

fn opts(flat: bool) -> HostOptions {
    HostOptions { deciders: DeciderChoice::Rules, sim: SimOptions { flat_ground: flat, ..Default::default() }, ..Default::default() }
}

/// Play in small steps as the server does, answering through the router.
fn step_to(h: &mut Host, until: SimTime) {
    while h.sim.now() < until {
        let to = (h.sim.now() + SimDuration::from_secs(7.0)).min(until);
        loop {
            match h.sim.advance(to) {
                Step::Advanced => break,
                Step::Over(_) => return,
                Step::NeedsAnswers { at, questions } => {
                    let a = h.decide(questions).unwrap();
                    h.submit(at, a).unwrap();
                }
            }
        }
    }
}

fn a_plan(h: &Host, side: Side) -> serde_json::Value {
    let unit = h.sim.ctx.scenario.forces.iter().find(|f| f.side == side).map(|f| f.label.clone()).unwrap();
    json!({ "plan": "pull back and hold", "orders": [{ "unit": unit, "task": "withdraw", "intent": "keep the force intact",
        "phases": [{ "label": "back", "do": "move", "to": { "ref": "self", "bearingDeg": 270.0, "distanceM": 1500.0 }, "mode": "tactical" }, { "label": "hold", "do": "hold" }] }] })
}

fn finish(h: &Host) -> GameRecord {
    h.finished_record()
}

#[test]
fn commands_and_seats_replay_exactly() {
    let sc = scenario("skirmish-advance-to-contact-v1");
    let mut h = Host::new(&sc, sc.seed, default_figures(), &opts(false)).unwrap();
    step_to(&mut h, SimTime::from_secs(300.0));
    let plan = a_plan(&h, Side::Blue);
    h.command(Side::Blue, "person", plan).unwrap();
    h.set_seat(Side::Red, SeatKind::Rules);
    step_to(&mut h, SimTime::from_secs(1500.0));
    let rec = finish(&h);
    assert_eq!(rec.commands.len(), 2);

    // Without the commands the game goes differently, so they matter.
    let mut bare = Host::replay(GameRecord { commands: Vec::new(), fingerprint: None, ..rec.clone() }, default_figures()).unwrap();
    let diverged = bare.play_until(SimTime::from_secs(1500.0), |_| {}).is_err() || bare.sim.fingerprint() != rec.fingerprint.unwrap();
    assert!(diverged, "the person's commands changed nothing");

    let mut r = Host::replay(rec.clone(), default_figures()).unwrap();
    r.play_until(SimTime::from_secs(1500.0), |_| {}).unwrap();
    r.verify_replay().unwrap();
    assert_eq!(r.record.commands, rec.commands);
}

#[test]
fn run_setup_and_figure_overrides_replay() {
    let sc = scenario("skirmish-symmetric-control-v1");
    let mut f = default_figures();
    f.set("sim.fire.lay_min_s", 12.0).unwrap();
    let mut h = Host::new(&sc, sc.seed, f, &opts(true)).unwrap();
    h.play_until(SimTime::from_secs(1200.0), |_| {}).unwrap();
    let rec = finish(&h);
    assert!(rec.run.flat_ground);
    assert_eq!(rec.figure_overrides.get("sim.fire.lay_min_s"), Some(&12.0));
    let mut r = Host::replay(rec, default_figures()).unwrap();
    r.play_until(SimTime::from_secs(1200.0), |_| {}).unwrap();
    r.verify_replay().unwrap();
}

#[test]
fn a_branch_with_the_same_deciders_is_the_same_game() {
    let sc = scenario("skirmish-advance-to-contact-v1");
    let mut h = Host::new(&sc, sc.seed, default_figures(), &opts(false)).unwrap();
    step_to(&mut h, SimTime::from_secs(240.0));
    let plan = a_plan(&h, Side::Red);
    h.command(Side::Red, "person", plan).unwrap();
    step_to(&mut h, SimTime::from_secs(1800.0));
    let rec = finish(&h);
    let end = h.sim.now();
    assert!(end.secs() > 260.0, "the game ended at {end}");
    // Between the person's plan and the end of the game.
    let at = SimTime::from_secs(((240.0 + end.secs()) / 2.0).floor());
    let router = router_for(&sc, &default_figures(), &opts(false));
    let mut b = Host::branch(rec.clone(), default_figures(), at, router).unwrap();
    assert!(!b.replaying());
    assert_eq!(b.sim.now(), at);
    assert_eq!(b.record.commands.len(), 1, "the branch's record holds the game up to the branch");
    step_to(&mut b, end);
    assert_eq!(b.sim.fingerprint(), rec.fingerprint.unwrap());
}
