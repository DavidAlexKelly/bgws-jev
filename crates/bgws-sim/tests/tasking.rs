//! Orders from outside (a federation's tasking, as Foundry sends it): a move
//! is followed to the end point and reported done; the unit's HQ does not
//! undo it; a teleport, a halt and a cancel act at once; orders for a unit
//! that cannot take them are refused; and the order is an input, so a replay
//! that feeds it at the same tick gives the same game.

use bgws_core::{EntityId, Geodetic, Seed, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Act, Activity, Health, Kin};
use bgws_sim::remote::ExternalInput;
use bgws_sim::tasking::{ExternalOrder, TaskBook, TaskStatus};
use bgws_sim::{default_figures, Sim};

fn sim() -> Sim {
    let path = format!("{}/../../data/scenarios/gen-kgd-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    Sim::new(&ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap(), Seed(1), default_figures()).unwrap()
}

fn id(s: &Sim, name: &str) -> EntityId {
    s.ctx.by_name[name]
}

fn at(s: &Sim, id: EntityId) -> Geodetic {
    s.ctx.frame.enu_to_geodetic(s.world.get::<Kin>(s.ctx.entity(id)).unwrap().pos)
}

fn status(s: &Sim, task: &str) -> TaskStatus {
    s.world.get_resource::<TaskBook>().unwrap().tasks.iter().find(|t| t.task == task).unwrap().status
}

fn order(s: &mut Sim, unit: EntityId, task: &str, order: ExternalOrder) {
    s.apply_external(ExternalInput::Order { entity: unit, task: task.into(), order });
}

#[test]
fn a_move_is_followed_to_its_end_and_the_hq_does_not_undo_it() {
    let mut s = sim();
    let b1 = id(&s, "b-1");
    s.run_with_rules(SimTime::from_secs(5.0));
    // Back west, away from the objective the company HQ will send it to.
    let start = at(&s, b1);
    let to = start.destination(270f64.to_radians(), 1200.0);
    order(&mut s, b1, "t-move", ExternalOrder::Move { lat: to.lat, lon: to.lon, roads: false });
    assert_eq!(status(&s, "t-move"), TaskStatus::Executing);
    let mut done_at = None;
    for t in (10..=1200).step_by(10) {
        s.run_with_rules(SimTime::from_secs(t as f64));
        if status(&s, "t-move") == TaskStatus::Completed {
            done_at = Some(t);
            break;
        }
    }
    let end = at(&s, b1);
    eprintln!("arrived after {done_at:?} s, {:.0} m from the point", end.surface_distance(to));
    assert!(done_at.is_some(), "the move completed");
    assert!(end.surface_distance(to) < 100.0);
    // It holds there under outside command: its HQ's plans pass it by.
    s.run_with_rules(SimTime::from_secs(done_at.unwrap() as f64 + 300.0));
    assert!(at(&s, b1).surface_distance(to) < 150.0, "still where it was sent, {:.0} m off", at(&s, b1).surface_distance(to));
    assert!(bgws_sim::tasking::controlled(&s, b1));
    // Cancelled, it is its HQ's again.
    order(&mut s, b1, "t-cancel", ExternalOrder::Cancel);
    assert_eq!(status(&s, "t-cancel"), TaskStatus::Completed);
    assert!(!bgws_sim::tasking::controlled(&s, b1));
}

#[test]
fn teleport_halt_and_a_newer_order_act_at_once() {
    let mut s = sim();
    let b2 = id(&s, "b-2");
    s.run_with_rules(SimTime::from_secs(5.0));
    let to = at(&s, b2).destination(0f64.to_radians(), 2000.0);
    order(&mut s, b2, "t-far", ExternalOrder::Move { lat: to.lat, lon: to.lon, roads: false });
    s.run_with_rules(SimTime::from_secs(20.0));
    // A newer order supersedes it.
    let jump = at(&s, b2).destination(90f64.to_radians(), 500.0);
    order(&mut s, b2, "t-jump", ExternalOrder::Teleport { lat: jump.lat, lon: jump.lon, heading_deg: 45.0 });
    assert_eq!(status(&s, "t-far"), TaskStatus::Cancelled);
    assert_eq!(status(&s, "t-jump"), TaskStatus::Completed);
    assert!(at(&s, b2).surface_distance(jump) < 1.0);
    let heading = s.world.get::<Kin>(s.ctx.entity(b2)).unwrap().heading.to_degrees();
    assert!((heading - 45.0).abs() < 1e-6);
    order(&mut s, b2, "t-halt", ExternalOrder::Halt);
    assert!(matches!(s.world.get::<Act>(s.ctx.entity(b2)).unwrap().activity, Activity::Hold));
    let here = at(&s, b2);
    s.run_with_rules(SimTime::from_secs(120.0));
    assert!(at(&s, b2).surface_distance(here) < 60.0, "it held");
}

#[test]
fn orders_it_cannot_take_are_refused() {
    let mut s = sim();
    let b3 = id(&s, "b-3");
    s.run_with_rules(SimTime::from_secs(2.0));
    s.world.get_mut::<Health>(s.ctx.entity(b3)).unwrap().destroyed = true;
    order(&mut s, b3, "t-dead", ExternalOrder::Halt);
    assert_eq!(status(&s, "t-dead"), TaskStatus::Refused);
    let b4 = id(&s, "b-4");
    order(&mut s, b4, "t-ghost", ExternalOrder::Engage { target: EntityId(9999) });
    assert_eq!(status(&s, "t-ghost"), TaskStatus::Refused);
}

#[test]
fn an_order_replays_exactly() {
    let run = || {
        let mut s = sim();
        s.run_with_rules(SimTime::from_secs(30.0));
        let b1 = id(&s, "b-1");
        let to = at(&s, b1).destination(200f64.to_radians(), 800.0);
        order(&mut s, b1, "t", ExternalOrder::Move { lat: to.lat, lon: to.lon, roads: false });
        s.run_with_rules(SimTime::from_secs(400.0));
        s.drain_events().iter().map(|e| format!("{} {} {}", e.t.secs(), e.kind, e.text)).collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
