//! Phase 4 over DIS (docs/design/INTEROP_PLAN.md §7): a real game's radios,
//! messages, radars and jammers, aggregated formations and minefields go out
//! as Transmitter, Signal, Electromagnetic Emission, Aggregate State and
//! Minefield State PDUs, each one parsing; and another simulation's radar
//! and jammer, coming in, reach our electronic warfare.

use bgws_core::{Seed, SimTime};
use bgws_interop::dis::DisPublisher;
use bgws_interop::dr::DrSettings;
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use bgws_sim::remote::{EmitterKind, ExternalInput, ExternalKey, RemoteEmitter};
use bgws_sim::{default_figures, Sim};
use dis_rs::model::PduBody;
use std::collections::BTreeMap;
use std::net::UdpSocket;

fn sim(name: &str) -> Sim {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Sim::new(&ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap(), Seed(1), default_figures()).unwrap()
}

#[test]
fn radios_emitters_formations_and_minefields_go_out() {
    let mut s = sim("m4-drone-line");
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
    let spec = InteropSpec { exercise: 4, site: 5, application: 6, ..Default::default() };
    let mut publisher = DisPublisher::new(&rx.local_addr().unwrap().to_string(), spec, DrSettings::default()).unwrap();
    let mut count: BTreeMap<u8, usize> = BTreeMap::new();
    let mut transmitting = 0;
    let mut signal_kinds = std::collections::BTreeSet::new();
    let mut jammers = 0;
    let mut aggregated = 0;
    let mut minefield_len = vec![];
    let mut buf = [0u8; 8192];
    for t in 1..=240u32 {
        s.run_with_rules(SimTime::from_secs(t as f64));
        let events = s.drain_events();
        publisher.publish(&s, &events).unwrap();
        while let Ok(n) = rx.recv(&mut buf) {
            let b = &buf[..n];
            *count.entry(b[2]).or_default() += 1;
            if b[2] == 37 {
                // Minefield State: written here, so read it field by field.
                assert_eq!(b[3], 8, "minefield family");
                assert_eq!(u16::from_be_bytes([b[8], b[9]]) as usize, n, "length field");
                let points = b[21] as usize; // after the header (12), ID (6), sequence (2), force (1)
                assert_eq!(n, 72 + 8 * points);
                minefield_len.push(n);
                continue;
            }
            let pdus = dis_rs::parse(b).unwrap_or_else(|e| panic!("PDU type {} does not parse: {e:?}", b[2]));
            for p in pdus {
                match p.body {
                    PduBody::Transmitter(x) => transmitting += (x.transmit_state == dis_rs::enumerations::TransmitterTransmitState::OnAndTransmitting) as usize,
                    PduBody::Signal(x) => {
                        let v: serde_json::Value = serde_json::from_slice(&x.data).expect("a signal's data is a JSON record");
                        signal_kinds.insert(v["what"].as_str().unwrap_or_default().split(' ').next().unwrap_or_default().to_string());
                    }
                    PduBody::ElectromagneticEmission(x) => jammers += x.emitter_systems.iter().filter(|e| e.function == dis_rs::enumerations::EmitterSystemFunction::Jammer_42).count(),
                    PduBody::AggregateState(x) => aggregated += (x.aggregate_state == dis_rs::enumerations::AggregateStateAggregateState::Aggregated) as usize,
                    _ => {}
                }
            }
        }
    }
    eprintln!("PDUs by type {count:?}; transmitting {transmitting}; signal kinds {signal_kinds:?}; jammer systems {jammers}; aggregated {aggregated}; minefield PDU sizes {:?}", minefield_len.iter().collect::<std::collections::BTreeSet<_>>());
    for (ty, name) in [(25u8, "Transmitter"), (26, "Signal"), (37, "Minefield State")] {
        assert!(count.get(&ty).copied().unwrap_or(0) > 0, "no {name} PDU");
    }
    assert!(transmitting > 0, "no radio was ever transmitting");
    assert!(!signal_kinds.is_empty());
}

#[test]
fn a_remote_radar_and_jammer_reach_our_electronic_warfare() {
    let mut s = sim("m4-drone-line");
    s.run_with_rules(SimTime::from_secs(2.0));
    let key = ExternalKey { site: 9, application: 9, entity: 1 };
    let at = s.world.get::<bgws_sim::components::Kin>(s.ctx.order[0]).unwrap().pos;
    s.apply_external(ExternalInput::EntityState { key, platform: "generic/ifv".into(), side: bgws_core::Side::Red, label: "remote".into(), pos: at, vel: bgws_core::Enu::default(), heading: 0.0, damage: 0 });
    let radar = RemoteEmitter { what: EmitterKind::Radar, freq_hz: 9.4e9, bw_hz: 1e6, erp_w: 5e4, duty: 1.0 };
    let jammer = RemoteEmitter { what: EmitterKind::Jammer, freq_hz: 4.5e7, bw_hz: 5e6, erp_w: 200.0, duty: 1.0 };
    s.apply_external(ExternalInput::Emitter { key, slot: (1, 0x101), emitter: Some(radar) });
    s.apply_external(ExternalInput::Emitter { key, slot: (1, 0x201), emitter: Some(jammer) });
    let id = bgws_sim::remote::entity_of(&s, key).unwrap();
    s.run_with_rules(SimTime::from_secs(4.0));
    let c4 = s.world.get_resource::<bgws_sim::domains::drones::C4>().unwrap();
    assert!(c4.ew.last_tx.iter().any(|t| t.entity == id && t.what == "radar" && t.freq_hz == 9.4e9), "the remote radar is on the air for our ESM");
    assert!(c4.ew.jammers.iter().any(|j| j.entity == id), "the remote jammer is among our jammers");
    // Off again: gone from both.
    s.apply_external(ExternalInput::Emitter { key, slot: (1, 0x101), emitter: None });
    s.apply_external(ExternalInput::Emitter { key, slot: (1, 0x201), emitter: None });
    s.run_with_rules(SimTime::from_secs(6.0));
    let c4 = s.world.get_resource::<bgws_sim::domains::drones::C4>().unwrap();
    assert!(!c4.ew.last_tx.iter().any(|t| t.entity == id), "nothing on the air");
    assert!(!c4.ew.jammers.iter().any(|j| j.entity == id));
}

#[test]
fn our_emission_pdu_reads_back_as_the_same_emitters() {
    use bgws_interop::extras::EmitterTruth;
    let s = sim("m4-drone-line");
    let spec = InteropSpec { site: 5, application: 6, ..Default::default() };
    let id = bgws_core::EntityId(0);
    let list = [EmitterTruth { id, number: 1, jammer: false, freq_hz: 9.4e9, bw_hz: 1e6, erp_w: 5e4, duty: 1.0 }, EmitterTruth { id, number: 2, jammer: true, freq_hz: 4.5e7, bw_hz: 5e6, erp_w: 200.0, duty: 1.0 }];
    let bytes = bgws_interop::dis::encode(&bgws_interop::dis_extras::emission_pdu(&s, &spec, id, &list, 1, true));
    let p = dis_rs::parse(&bytes).unwrap().remove(0);
    let PduBody::ElectromagneticEmission(ee) = p.body else { panic!("not an EE PDU") };
    let key = ExternalKey { site: 5, application: 6, entity: 1 };
    let mut before = Default::default();
    let inputs = bgws_interop::dis_extras::emission_inputs(&ee, key, &mut before);
    let got: Vec<RemoteEmitter> = inputs.into_iter().filter_map(|i| if let ExternalInput::Emitter { emitter: Some(e), .. } = i { Some(e) } else { None }).collect();
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].what, got[1].what), (EmitterKind::Radar, EmitterKind::Jammer));
    for (g, w) in got.iter().zip(&list) {
        assert!((g.freq_hz - w.freq_hz).abs() / w.freq_hz < 1e-6 && (g.erp_w - w.erp_w).abs() / w.erp_w < 1e-3, "{g:?} vs {w:?}");
    }
    // A later PDU without the jammer turns it off.
    let one = bgws_interop::dis::encode(&bgws_interop::dis_extras::emission_pdu(&s, &spec, id, &list[..1], 2, true));
    let PduBody::ElectromagneticEmission(ee) = dis_rs::parse(&one).unwrap().remove(0).body else { panic!() };
    let off: Vec<_> = bgws_interop::dis_extras::emission_inputs(&ee, key, &mut before).into_iter().filter(|i| matches!(i, ExternalInput::Emitter { emitter: None, .. })).collect();
    assert_eq!(off.len(), 1, "the jammer goes off");
}

#[test]
fn aggregated_formations_go_out_where_their_members_are() {
    let mut s = sim("m6-sustained-offensive");
    let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
    let spec = InteropSpec { exercise: 4, site: 5, application: 6, ..Default::default() };
    let mut publisher = DisPublisher::new(&rx.local_addr().unwrap().to_string(), spec.clone(), DrSettings::default()).unwrap();
    let mut seen: BTreeMap<u16, (usize, [f64; 3])> = BTreeMap::new();
    let mut buf = [0u8; 65536];
    for t in 1..=60u32 {
        s.run_with_rules(SimTime::from_secs(t as f64));
        let events = s.drain_events();
        publisher.publish(&s, &events).unwrap();
        while let Ok(n) = rx.recv(&mut buf) {
            if buf[2] != 33 {
                continue;
            }
            for p in dis_rs::parse(&buf[..n]).expect("an Aggregate State PDU parses") {
                if let PduBody::AggregateState(a) = p.body {
                    let c = a.center_of_mass;
                    seen.insert(a.aggregate_id.entity_id, (a.entities.len(), [c.x_coordinate, c.y_coordinate, c.z_coordinate]));
                }
            }
        }
    }
    assert!(seen.len() >= 4, "{} aggregates published", seen.len());
    let truth = bgws_interop::extras::aggregates(&s, &spec);
    for (number, (members, ecef)) in &seen {
        assert!(*number >= bgws_interop::dis_extras::AGGREGATE_BASE && *members > 0);
        let at = bgws_interop::geo::unplace(&s.ctx.frame, bgws_interop::geoid::Geoid::egm96(), *ecef);
        let t = truth.iter().find(|a| a.number + bgws_interop::dis_extras::AGGREGATE_BASE == *number).expect("a formation");
        assert!(at.sub(t.centre).horizontal() < 200.0 + t.radius_m, "aggregate {number} published {:.0} m from its members' centre", at.sub(t.centre).horizontal());
    }
}
