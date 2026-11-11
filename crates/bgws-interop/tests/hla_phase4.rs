//! Phase 4 over HLA (RPR FOM): a game's radios, messages, emitters,
//! minefields and aggregated formations reach other federates as RPR objects
//! and interactions, and another BGWS federate takes the radios and jammers
//! in as emitters for its electronic warfare, as over DIS.

use bgws_core::{Seed, SimTime};
use bgws_interop::hla::encoding::Dec;
use bgws_interop::hla::link::{HlaLink, HlaSettings};
use bgws_interop::hla::mock::MockRti;
use bgws_interop::hla::proto::callback_request::CallbackRequest as Cb;
use bgws_interop::hla::rpr_extras as rx;
use bgws_interop::hla::rti::{Handle, Rti};
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use bgws_sim::remote::{EmitterKind, ExternalInput};
use bgws_sim::{default_figures, Sim};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

fn sim(name: &str) -> Sim {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Sim::new(&ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap(), Seed(1), default_figures()).unwrap()
}

fn spec(site: u16) -> InteropSpec {
    InteropSpec { site, application: 1, ..Default::default() }
}

/// A plain federate watching the phase 4 classes and the signal interaction.
fn observer(rti: &MockRti) -> (Rti, BTreeMap<Handle, &'static str>, Handle, BTreeMap<Handle, String>) {
    let r = Rti::connect(&rti.address(), Default::default()).unwrap();
    r.join("observer", "test", "phase4", &[]).unwrap();
    let mut classes = BTreeMap::new();
    let mut attrs = BTreeMap::new();
    for c in rx::CLASSES {
        let h = r.object_class(c).unwrap();
        let hs: Vec<Handle> = rx::attributes_for(c)
            .iter()
            .map(|a| {
                let x = r.attribute(&h, a).unwrap();
                attrs.insert(x.clone(), a.to_string());
                x
            })
            .collect();
        r.subscribe_object(&h, &hs).unwrap();
        classes.insert(h, c);
    }
    let signal = r.interaction_class(rx::RAW_BINARY_SIGNAL).unwrap();
    for p in rx::SIGNAL_PARAMETERS {
        attrs.insert(r.parameter(&signal, p).unwrap(), p.to_string());
    }
    r.subscribe_interaction(&signal).unwrap();
    (r, classes, signal, attrs)
}

#[test]
fn radios_emitters_minefields_and_messages_cross_over_hla() {
    let rti = MockRti::start().unwrap();
    let creator = Rti::connect(&rti.address(), Default::default()).unwrap();
    creator.create_federation("phase4", &[]).unwrap();
    let (obs, classes, signal, names) = observer(&rti);
    let mut s1 = sim("m4-drone-line");
    let s2 = sim("m4-drone-line");
    let mut a = HlaLink::join(HlaSettings::new(&rti.address(), "phase4", "a"), spec(1)).unwrap();
    let mut b = HlaLink::join(HlaSettings::new(&rti.address(), "phase4", "b"), spec(2)).unwrap();
    let mut inputs = vec![];
    for t in 1..=150u32 {
        s1.run_with_rules(SimTime::from_secs(t as f64));
        let events = s1.drain_events();
        a.tick(&s1, &events).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        inputs.extend(b.inbound(&s2));
    }
    std::thread::sleep(Duration::from_millis(100));
    inputs.extend(b.inbound(&s2));
    // The other BGWS federate takes them in as emitters.
    let kinds: BTreeSet<String> = inputs.iter().filter_map(|i| if let ExternalInput::Emitter { emitter: Some(e), .. } = i { Some(format!("{:?}", e.what)) } else { None }).collect();
    eprintln!("emitter kinds taken in: {kinds:?}; notes {:?}", b.notes);
    assert!(kinds.contains(&format!("{:?}", EmitterKind::Jammer)), "a jammer came across");
    assert!(kinds.contains(&format!("{:?}", EmitterKind::Radio)), "a transmitting radio came across");
    // The observer saw each class and the messages.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let mut messages = BTreeSet::new();
    for c in obs.callbacks() {
        match c {
            Cb::DiscoverObjectInstance(d) => *seen.entry(classes[&Handle(d.object_class.unwrap().data)]).or_default() += 1,
            Cb::ReceiveInteraction(r) if Handle(r.interaction_class.clone().unwrap().data) == signal => {
                for p in r.parameter_values.unwrap().parameter_handle_value {
                    if names[&Handle(p.parameter_handle.unwrap().data)] == "SignalData" {
                        let mut d = Dec::new(&p.value);
                        let n = d.u32().unwrap() as usize;
                        let v: serde_json::Value = serde_json::from_slice(d.bytes(n).unwrap()).unwrap();
                        messages.insert(v["what"].as_str().unwrap_or_default().split(' ').next().unwrap_or_default().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    eprintln!("objects discovered by class {seen:?}; messages {messages:?}");
    for c in [rx::RADIO_TRANSMITTER, rx::EMITTER_SYSTEM, rx::JAMMER_BEAM, rx::MINEFIELD] {
        assert!(seen.get(c).copied().unwrap_or(0) > 0, "no {c}");
    }
    assert!(!messages.is_empty(), "no message crossed");
}

#[test]
fn aggregated_formations_cross_over_hla() {
    let rti = MockRti::start().unwrap();
    let creator = Rti::connect(&rti.address(), Default::default()).unwrap();
    creator.create_federation("phase4", &[]).unwrap();
    let (obs, classes, _, names) = observer(&rti);
    let mut s = sim("m6-sustained-offensive");
    let mut a = HlaLink::join(HlaSettings::new(&rti.address(), "phase4", "a"), spec(1)).unwrap();
    for t in 1..=40u32 {
        s.run_with_rules(SimTime::from_secs(t as f64));
        let events = s.drain_events();
        a.tick(&s, &events).unwrap();
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut aggregates = BTreeSet::new();
    let mut members = 0;
    let mut state = None;
    for c in obs.callbacks() {
        match c {
            Cb::DiscoverObjectInstance(d) if classes[&Handle(d.object_class.clone().unwrap().data)] == rx::AGGREGATE => {
                aggregates.insert(d.object_instance_name);
            }
            Cb::ReflectAttributeValues(r) => {
                for v in r.attribute_values.unwrap().attribute_handle_value {
                    match names[&Handle(v.attribute_handle.unwrap().data)].as_str() {
                        "EntityIdentifiers" => members = members.max(Dec::new(&v.value).u32().unwrap()),
                        "AggregateState" => state = v.value.first().copied(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    eprintln!("aggregates {aggregates:?}, most members {members}, state {state:?}");
    assert!(aggregates.len() >= 4, "{aggregates:?}");
    assert!(members > 0);
    assert_eq!(state, Some(1), "aggregated");
}
