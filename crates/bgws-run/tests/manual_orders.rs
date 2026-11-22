//! A person's direct orders mid-game (the viewer's Orders panel, the
//! server's `order` message): a unit given "move here" goes there and the
//! task is reported done; halt, rules of engagement and engage act; an order
//! a unit cannot take is refused with the reason; and the game record keeps
//! the orders, so a replay is the same game.

use bgws_core::{Seed, SimTime};
use bgws_run::host::ManualOrder;
use bgws_run::{default_figures, load_scenario, Host, HostOptions};
use bgws_sim::components::{Act, Activity, Kin, RoeS};

fn host() -> Host {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios/gen-company-attack.json");
    let sc = load_scenario(&path).unwrap();
    Host::new(&sc, Seed(3), default_figures(), &HostOptions::default()).unwrap()
}

fn at(h: &Host, unit: &str) -> bgws_core::Geodetic {
    let id = h.sim.ctx.by_name[unit];
    h.sim.ctx.frame.enu_to_geodetic(h.sim.world.get::<Kin>(h.sim.ctx.entity(id)).unwrap().pos)
}

#[test]
fn orders_given_mid_game_are_carried_out_and_replay() {
    let mut h = host();
    h.play_until(SimTime::from_secs(60.0), |_| {}).unwrap();
    // Back west, away from where its HQ sends it.
    let to = at(&h, "b-1").destination(270f64.to_radians(), 900.0);
    let task = h.order("b-1", ManualOrder::Move { lat: to.lat, lon: to.lon, roads: false }).unwrap();
    assert_eq!(task, "manual-1");
    h.order("b-2", ManualOrder::Halt).unwrap();
    h.order("b-3", ManualOrder::Roe { roe: "never".into() }).unwrap();
    // Refused, with the reason.
    assert!(h.order("b-4", ManualOrder::Engage { target: "nobody".into() }).unwrap_err().to_string().contains("no unit"));
    assert!(h.order("nobody", ManualOrder::Halt).is_err());
    assert!(h.order("b-4", ManualOrder::Roe { roe: "sometimes".into() }).is_err());
    let b3 = h.sim.ctx.entity(h.sim.ctx.by_name["b-3"]);
    assert_eq!(h.sim.world.get::<Act>(b3).unwrap().roe, RoeS::Never);
    let b2 = h.sim.ctx.entity(h.sim.ctx.by_name["b-2"]);
    assert!(matches!(h.sim.world.get::<Act>(b2).unwrap().activity, Activity::Hold));

    let mut done = false;
    for t in (70..=900).step_by(10) {
        h.play_until(SimTime::from_secs(t as f64), |_| {}).unwrap();
        let ev = h.sim.world.get_resource::<bgws_sim::tasking::TaskBook>().unwrap().tasks.iter().find(|x| x.task == "manual-1").unwrap().status;
        if ev == bgws_sim::tasking::TaskStatus::Completed {
            done = true;
            break;
        }
    }
    let off = at(&h, "b-1").surface_distance(to);
    eprintln!("b-1 is {off:.0} m from where it was sent");
    assert!(done, "the move completed");
    assert!(off < 100.0);
    h.play_until(SimTime::from_secs(1000.0), |_| {}).unwrap();

    // The record keeps the orders: a replay is the same game.
    let rec = h.finished_record();
    assert_eq!(rec.commands.iter().filter(|c| c.kind == bgws_scenario::record::CommandKind::Order).count(), 3, "the three orders given");
    let until = rec.ends_at();
    let mut r = Host::replay(rec, default_figures()).unwrap();
    r.play_until(until, |_| {}).unwrap();
    r.verify_replay().unwrap();
    assert!(at(&r, "b-1").surface_distance(at(&h, "b-1")) < 1e-6);
}
