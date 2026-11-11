//! A real game published over DIS to a local socket: every PDU parses, every
//! ground vehicle stands on the ground (its ellipsoidal height less the
//! geoid is the sim's ground height), each entity is refreshed within the
//! heartbeat, and dead reckoning sends far fewer states than one per entity
//! per second.

use bgws_core::{Seed, SimTime};
use bgws_interop::dis::DisPublisher;
use bgws_interop::dr::DrSettings;
use bgws_interop::geo::unplace;
use bgws_interop::geoid::Geoid;
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use bgws_sim::{default_figures, Sim};
use std::collections::BTreeMap;
use std::net::UdpSocket;

#[test]
fn a_company_attack_published_over_dis() {
    let path = format!("{}/../../data/scenarios/gen-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    let sc = ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut sim = Sim::new(&sc, Seed(1), default_figures()).unwrap();
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(std::time::Duration::from_millis(200))).unwrap();
    let spec = InteropSpec { exercise: 3, site: 5, application: 6, ..Default::default() };
    let mut publisher = DisPublisher::new(&rx.local_addr().unwrap().to_string(), spec, DrSettings::default()).unwrap();
    let entities = bgws_interop::model::truths(&sim, &publisher.spec).len();
    assert!(entities > 10, "{entities} entities to publish");

    let geoid = Geoid::egm96();
    let mut last_heard: BTreeMap<u16, f64> = BTreeMap::new();
    let mut worst_gap: f64 = 0.0;
    let mut worst_ground: f64 = 0.0;
    let mut received = 0usize;
    let mut fires = std::collections::BTreeSet::new();
    let mut detonations = Vec::new();
    let mut buf = [0u8; 2048];
    // The approach in one go (the first shots come at about 390 s), then
    // 150 s of the fight, published each second.
    let start = 360;
    sim.run_with_rules(SimTime::from_secs(start as f64));
    sim.drain_events();
    let seconds = 150;
    for t in start + 1..=start + seconds {
        sim.run_with_rules(SimTime::from_secs(t as f64));
        let events = sim.drain_events();
        publisher.publish(&sim, &events).unwrap();
        // Drain what arrived this second.
        while let Ok((n, _)) = rx.recv_from(&mut buf) {
            let pdus = dis_rs::parse(&buf[..n]).expect("a valid PDU");
            for p in pdus {
                assert_eq!(p.header.exercise_id, 3);
                let es = match p.body {
                    dis_rs::model::PduBody::EntityState(es) => es,
                    dis_rs::model::PduBody::Fire(f) => {
                        assert_eq!((f.firing_entity_id.simulation_address.site_id, f.event_id.simulation_address.site_id), (5, 5));
                        fires.insert(f.event_id.event_id);
                        continue;
                    }
                    dis_rs::model::PduBody::Detonation(d) => {
                        detonations.push(d.event_id.event_id);
                        continue;
                    }
                    // Phase 4: radios, messages, emitters, aggregates (tests/phase4.rs).
                    dis_rs::model::PduBody::Transmitter(_) | dis_rs::model::PduBody::Signal(_) | dis_rs::model::PduBody::ElectromagneticEmission(_) | dis_rs::model::PduBody::AggregateState(_) => continue,
                    other => panic!("unexpected PDU {other:?}"),
                };
                assert_eq!((es.entity_id.simulation_address.site_id, es.entity_id.simulation_address.application_id), (5, 6));
                received += 1;
                let id = es.entity_id.entity_id;
                if let Some(prev) = last_heard.insert(id, t as f64) {
                    worst_gap = worst_gap.max(t as f64 - prev);
                }
                // A land platform stands on our ground.
                if u8::from(es.entity_type.kind) == 1 && u8::from(es.entity_type.domain) == 1 {
                    let pos = unplace(&sim.ctx.frame, geoid, [es.entity_location.x_coordinate, es.entity_location.y_coordinate, es.entity_location.z_coordinate]);
                    let ground = sim.ctx.ground.height(pos.e, pos.n);
                    worst_ground = worst_ground.max((pos.u - ground).abs());
                }
            }
        }
    }
    assert_eq!(last_heard.len(), entities, "every entity was published");
    // Shots went out as Fire and Detonation PDUs, each detonation with the
    // event of a fire sent before it.
    assert!(!fires.is_empty() && !detonations.is_empty(), "{} fires, {} detonations", fires.len(), detonations.len());
    assert!(detonations.iter().all(|d| fires.contains(d)), "a detonation with no fire before it");
    eprintln!("{} fires, {} detonations", fires.len(), detonations.len());
    // The 5 s heartbeat (plus a second's tick).
    assert!(worst_gap <= 6.0, "an entity went {worst_gap} s without a state");
    // Ground vehicles stand on the ground once the geoid is taken off (the
    // grid's interpolation and the move in the last second aside).
    assert!(worst_ground < 0.5, "a ground vehicle was {worst_ground:.2} m off the ground");
    // Dead reckoning: well under one state per entity per second.
    let per_entity_second = received as f64 / (entities * seconds) as f64;
    assert!(per_entity_second < 0.5, "{received} states for {entities} entities over {seconds} s ({per_entity_second:.2} per entity-second)");
    eprintln!("{received} states for {entities} entities over {seconds} s ({per_entity_second:.2} per entity-second), worst gap {worst_gap} s, worst ground {worst_ground:.3} m");
}
