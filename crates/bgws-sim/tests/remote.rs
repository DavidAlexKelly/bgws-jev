//! Entities owned by another simulation (docs/design/INTEROP_PLAN.md §3.2):
//! they join on their first state, stand on our ground, move only by dead
//! reckoning from their owner's states, are fired at but never damaged,
//! moved or decided for here, count in no side's strength, and go when
//! their owner stops sending. Detonations on our entities are ours to judge,
//! once each.

use bgws_core::{Enu, EntityId, Seed, Side, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Health, Kin};
use bgws_sim::remote::{DetonationResult, ExternalInput, ExternalKey};
use bgws_sim::{default_figures, Sim, SimOptions};

fn sim() -> Sim {
    let path = format!("{}/../../data/scenarios/gen-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    let mut sc = ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    sc.victory.clear();
    Sim::with_options(&sc, Seed(1), default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap()
}

const KEY: ExternalKey = ExternalKey { site: 9, application: 9, entity: 1 };

fn state(pos: Enu, vel: Enu, damage: u8) -> ExternalInput {
    ExternalInput::EntityState { key: KEY, platform: "generic/mbt".into(), side: Side::Red, label: "remote tank".into(), pos, vel, heading: 0.0, damage }
}

fn blue_tank(s: &Sim) -> (bevy_ecs::prelude::Entity, EntityId) {
    let id = s.ctx.by_name["b-1"];
    (s.ctx.entity(id), id)
}

#[test]
fn a_remote_entity_joins_moves_by_dead_reckoning_and_times_out() {
    let mut s = sim();
    let strength = s.strength();
    let (_, bid) = blue_tank(&s);
    let at = s.world.get::<Kin>(s.ctx.entity(bid)).unwrap().pos.add(Enu::new(0.0, 1500.0, 0.0));
    s.apply_external(state(Enu::new(at.e, at.n, 999.0), Enu::new(0.0, 5.0, 0.0), 0));
    let rid = bgws_sim::remote::entity_of(&s, KEY).expect("joined");
    let re = s.ctx.entity(rid);
    // On our ground, whatever height it was sent at.
    let k = *s.world.get::<Kin>(re).unwrap();
    assert!((k.pos.u - s.ctx.ground.height(k.pos.e, k.pos.n)).abs() < 1e-6);
    // It counts in no side's strength.
    assert_eq!(s.strength(), strength);
    // Ten seconds on, dead reckoning has carried it 50 m north (no new state).
    s.run_with_rules(SimTime::from_secs(10.0));
    let moved = s.world.get::<Kin>(re).unwrap().pos.n - k.pos.n;
    assert!((moved - 50.0).abs() < 1.0, "dead reckoned {moved:.1} m");
    // Nothing for over 12 s: taken off.
    s.run_with_rules(SimTime::from_secs(14.0));
    assert!(s.world.get::<Health>(re).unwrap().destroyed, "taken off after the time-out");
    // A new state brings it back, and no question was ever asked about it.
    s.apply_external(state(at, Enu::default(), 0));
    assert!(!s.world.get::<Health>(re).unwrap().destroyed);
    let ev = s.drain_events();
    assert!(ev.iter().any(|e| e.kind == "interop.joined") && ev.iter().any(|e| e.kind == "interop.removed") && ev.iter().any(|e| e.kind == "interop.back"));
    assert!(!ev.iter().any(|e| e.kind == "decide.question" && e.entity == Some(rid)), "a remote entity is never asked");
}

#[test]
fn our_fire_on_a_remote_entity_leaves_its_damage_to_its_owner() {
    let mut s = sim();
    let (be, _) = blue_tank(&s);
    let at = s.world.get::<Kin>(be).unwrap().pos.add(Enu::new(0.0, 1200.0, 0.0));
    let mut fired = false;
    for t in 1..=180 {
        // The owner keeps it there, undamaged.
        s.apply_external(state(at, Enu::default(), 0));
        s.run_with_rules(SimTime::from_secs(t as f64));
        let rid = bgws_sim::remote::entity_of(&s, KEY).unwrap();
        let h = s.world.get::<Health>(s.ctx.entity(rid)).unwrap();
        assert!(!h.destroyed && h.fit == h.start, "at {t} s our fire changed a remote entity: {h:?}");
        let ev = s.drain_events();
        fired |= ev.iter().any(|e| e.kind.starts_with("fire.") && e.target == Some(rid) && e.data["remote"] == true);
        assert!(!ev.iter().any(|e| e.kind.starts_with("fire.") && e.entity == Some(rid)), "a remote entity fired here");
    }
    assert!(fired, "our units fired at the remote tank");
}

#[test]
fn a_detonation_on_ours_is_judged_once() {
    let mut s = sim();
    let (be, bid) = blue_tank(&s);
    let at = s.world.get::<Kin>(be).unwrap().pos;
    s.apply_external(state(at.add(Enu::new(0.0, 2000.0, 0.0)), Enu::default(), 0));
    let fit0 = s.world.get::<Health>(be).unwrap().fit;
    let det = |n: u16| ExternalInput::Detonation { firer: Some(KEY), target: Some(bid), munition: "atgmTandem".into(), at, result: DetonationResult::Impact, rounds: 3, event: (9, 9, n) };
    s.apply_external(det(1));
    let after = s.world.get::<Health>(be).unwrap().fit;
    // The same event again changes nothing.
    s.apply_external(det(1));
    assert_eq!(s.world.get::<Health>(be).unwrap().fit, after, "a detonation judged twice");
    // Over many events a tandem ATGM's impacts knock tanks out.
    for n in 2..40 {
        s.apply_external(det(n));
    }
    let end = s.world.get::<Health>(be).unwrap();
    assert!(end.fit < fit0 || end.destroyed, "39 tandem ATGM impacts did nothing");
    let ev = s.drain_events();
    assert_eq!(ev.iter().filter(|e| e.kind == "interop.detonation").count(), 39);
}
