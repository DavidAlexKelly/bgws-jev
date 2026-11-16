//! A federation built on NETN (as the CSE-Prep federation and its
//! HLA-Palantir bridge are): created with a MIM of its own, which gives every
//! object a `UniqueId`, and a FOM whose `BaseEntity` has a `Callsign`. BGWS
//! publishes both for its entities, and its event log as `BGWS_Event`
//! interactions from our own module, which joins with the FOM.

use bgws_core::{Seed, SimTime};
use bgws_interop::hla::link::{HlaLink, HlaSettings};
use bgws_interop::hla::mock::MockRti;
use bgws_interop::hla::netn;
use bgws_interop::hla::proto::callback_request::CallbackRequest as Cb;
use bgws_interop::hla::rti::{FomFile, Handle, Rti};
use bgws_interop::hla::{encoding::Dec, rpr, rpr_extras as rx};
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use bgws_sim::{default_figures, Sim};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// A strict FOM shaped like RPR, holding exactly what BGWS uses, with each
/// branch's attributes on the branch's root (inherited below it) and NETN's
/// `Callsign` on `BaseEntity`.
fn rpr_fom() -> FomFile {
    #[derive(Default)]
    struct Node {
        members: BTreeSet<String>,
        children: BTreeMap<String, Node>,
    }
    fn add<'a>(root: &'a mut Node, path: &str) -> &'a mut Node {
        let mut n = root;
        for seg in path.split('.').skip(1) {
            n = n.children.entry(seg.to_string()).or_default();
        }
        n
    }
    fn xml(name: &str, n: &Node, tag: &str, member: &str, out: &mut String) {
        out.push_str(&format!("<{tag}><name>{name}</name>"));
        for m in &n.members {
            out.push_str(&format!("<{member}><name>{m}</name><dataType>HLAopaqueData</dataType></{member}>"));
        }
        for (k, c) in &n.children {
            xml(k, c, tag, member, out);
        }
        out.push_str(&format!("</{tag}>"));
    }
    let mut objects = Node::default();
    for class in rpr::CLASSES.iter().chain(rx::CLASSES.iter()) {
        add(&mut objects, class);
        let branch: Vec<&str> = class.split('.').take(2).collect();
        let attrs = if class.starts_with(rpr::BASE) { rpr::attributes_for(class) } else { rx::attributes_for(class) };
        add(&mut objects, &branch.join(".")).members.extend(attrs.into_iter().map(String::from));
    }
    add(&mut objects, "HLAobjectRoot.BaseEntity").members.extend(["Callsign".to_string(), "SupportedActions".to_string()]);
    let mut interactions = Node::default();
    for (class, params) in [(rpr::WEAPON_FIRE, &rpr::WEAPON_FIRE_PARAMETERS[..]), (rpr::MUNITION_DETONATION, &rpr::MUNITION_DETONATION_PARAMETERS[..]), (rx::RAW_BINARY_SIGNAL, &rx::SIGNAL_PARAMETERS[..])] {
        add(&mut interactions, class).members.extend(params.iter().map(|p| p.to_string()));
    }
    // NETN-SMC and ETR, as the bridge's merged FOM has them.
    add(&mut interactions, netn::SMC).members.insert("Entity".into());
    add(&mut interactions, netn::TASK).members.insert("TaskId".into());
    for (short, _, _) in netn::TASK_CLASSES {
        let c = add(&mut interactions, &netn::task_class(short));
        if short == "CancelTasks" {
            c.members.insert("Tasks".into());
        } else if short != "StopAtSideOfRoad" {
            c.members.insert("TaskParameters".into());
        }
    }
    add(&mut interactions, netn::SMC_RESPONSE).members.extend(netn::SMC_RESPONSE_PARAMETERS.map(String::from));
    add(&mut interactions, netn::TASK_STATUS).members.extend(netn::TASK_STATUS_PARAMETERS.map(String::from));
    let mut out = String::from(r#"<?xml version="1.0"?><objectModel xmlns="http://standards.ieee.org/IEEE1516-2010"><objects>"#);
    xml("HLAobjectRoot", &objects, "objectClass", "attribute", &mut out);
    out.push_str("</objects><interactions>");
    xml("HLAinteractionRoot", &interactions, "interactionClass", "parameter", &mut out);
    out.push_str("</interactions></objectModel>");
    FomFile { name: "rpr-shaped.xml".into(), content: out.into_bytes() }
}

/// The part of NETN's MIM that matters here: `UniqueId` on every object.
fn netn_mim() -> FomFile {
    let x = r#"<?xml version="1.0"?><objectModel xmlns="http://standards.ieee.org/IEEE1516-2010"><objects><objectClass><name>HLAobjectRoot</name><attribute><name>HLAprivilegeToDeleteObject</name></attribute><attribute><name>UniqueId</name><dataType>UUID</dataType></attribute></objectClass></objects><interactions><interactionClass><name>HLAinteractionRoot</name><parameter><name>UniqueId</name><dataType>UUID</dataType></parameter></interactionClass></interactions></objectModel>"#;
    FomFile { name: "NETN-MIM-mini.xml".into(), content: x.as_bytes().to_vec() }
}

fn sim(name: &str) -> Sim {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Sim::new(&ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap(), Seed(1), default_figures()).unwrap()
}

#[test]
fn netn_identity_and_the_event_log_reach_another_federate() {
    let rti = MockRti::start().unwrap();
    let mut settings = HlaSettings::new(&rti.address(), "netn", "BGWS-1-1");
    settings.foms = vec![rpr_fom()];
    settings.mim = Some(netn_mim());
    let mut link = HlaLink::join(settings, InteropSpec { site: 1, application: 1, ..Default::default() }).unwrap();
    assert!(link.notes.is_empty(), "{:?}", link.notes);

    // A reader like the bridge: entities with their UUID and callsign, and
    // the event interaction (from the module BGWS joined with).
    let obs = Rti::connect(&rti.address(), Default::default()).unwrap();
    obs.join("reader", "test", "netn", &[]).unwrap();
    let ground = obs.object_class(rpr::CLASSES[0]).unwrap();
    let mut attr_names = BTreeMap::new();
    let wanted: Vec<Handle> = ["UniqueId", "Callsign", "Marking"]
        .iter()
        .map(|a| {
            let h = obs.attribute(&ground, a).unwrap();
            attr_names.insert(h.clone(), *a);
            h
        })
        .collect();
    obs.subscribe_object(&ground, &wanted).unwrap();
    let event = obs.interaction_class(netn::EVENT_CLASS).unwrap();
    let mut param_names = BTreeMap::new();
    for p in netn::EVENT_PARAMETERS {
        param_names.insert(obs.parameter(&event, p).unwrap(), p);
    }
    obs.subscribe_interaction(&event).unwrap();

    let mut s = sim("gen-kgd-company-attack");
    for t in 1..=90u32 {
        s.run_with_rules(SimTime::from_secs(t as f64));
        let events = s.drain_events();
        link.tick(&s, &events).unwrap();
    }
    std::thread::sleep(Duration::from_millis(200));

    let mut names = BTreeMap::new();
    let mut identity: BTreeMap<String, (Vec<u8>, String)> = BTreeMap::new();
    let mut received = vec![];
    for c in obs.callbacks() {
        match c {
            Cb::DiscoverObjectInstance(d) => {
                names.insert(d.object_instance.unwrap().data, d.object_instance_name);
            }
            Cb::ReflectAttributeValues(r) => {
                let name = names[&r.object_instance.unwrap().data].clone();
                let e = identity.entry(name).or_default();
                for v in r.attribute_values.unwrap().attribute_handle_value {
                    match attr_names[&Handle(v.attribute_handle.unwrap().data)] {
                        "UniqueId" => e.0 = v.value,
                        "Callsign" => e.1 = Dec::new(&v.value).unicode().unwrap(),
                        _ => {}
                    }
                }
            }
            Cb::ReceiveInteraction(r) => {
                let params: Vec<(&str, Vec<u8>)> = r.parameter_values.unwrap().parameter_handle_value.into_iter().map(|p| (param_names[&Handle(p.parameter_handle.unwrap().data)], p.value)).collect();
                let refs: Vec<(&str, &[u8])> = params.iter().map(|(n, v)| (*n, v.as_slice())).collect();
                received.push(netn::read_event(&refs).unwrap());
            }
            _ => {}
        }
    }
    eprintln!("{} ground vehicles with identity; {} events, kinds {:?}", identity.len(), received.len(), received.iter().map(|e| e.kind.as_str()).collect::<BTreeSet<_>>());

    // Every vehicle has its UUID (version 5, of its instance name) and its name.
    assert!(!identity.is_empty());
    for (name, (uuid, callsign)) in &identity {
        assert_eq!(uuid.as_slice(), netn::unique_id(name).as_slice(), "{name}");
        assert!(!callsign.is_empty() && !callsign.starts_with("BGWS-"), "{name}: callsign {callsign:?}");
    }
    // The event log came across in order, and its entities are the same
    // entities: the UUID in an event is the UUID on the object.
    assert!(received.len() > 10, "{} events", received.len());
    assert!(received.windows(2).all(|w| w[0].seq < w[1].seq), "events in log order");
    assert!(received.iter().all(|e| e.federate == "BGWS-1-1"));
    let named: Vec<_> = received.iter().filter(|e| identity.contains_key(&e.entity.name)).collect();
    assert!(!named.is_empty(), "an event names a vehicle");
    for e in named {
        assert_eq!(e.entity.unique_id, netn::uuid_text(&netn::unique_id(&e.entity.name)));
        assert_eq!(e.entity.callsign, identity[&e.entity.name].1);
        let [lat, lon, _] = e.at.expect("a position with the entity");
        assert!((54.0..56.0).contains(&lat) && (19.0..22.0).contains(&lon), "{lat} {lon} is in Kaliningrad");
    }
}

/// Foundry's orders as the HLA–Palantir bridge sends them (NETN-SMC): a move
/// for a BGWS unit is answered (SMC_Response, ETR_TaskStatus accepted), the
/// unit goes there, and the task is reported executing then completed; an
/// order BGWS does not model is refused.
#[test]
fn a_unit_takes_an_order_from_the_federation_and_reports_it_done() {
    let rti = MockRti::start().unwrap();
    let mut settings = HlaSettings::new(&rti.address(), "netn", "BGWS-1-1");
    settings.foms = vec![rpr_fom()];
    settings.mim = Some(netn_mim());
    let mut link = HlaLink::join(settings, InteropSpec { site: 1, application: 1, ..Default::default() }).unwrap();

    // The tasker: publishes the task classes, listens for the answers.
    let tasker = Rti::connect(&rti.address(), Default::default()).unwrap();
    tasker.join("bridge", "test", "netn", &[]).unwrap();
    let class = |short: &str| {
        let h = tasker.interaction_class(&netn::task_class(short)).unwrap();
        tasker.publish_interaction(&h).unwrap();
        h
    };
    let (mv, speed) = (class("MoveToLocation"), class("ChangeSpeed"));
    let response = tasker.interaction_class(netn::SMC_RESPONSE).unwrap();
    let status = tasker.interaction_class(netn::TASK_STATUS).unwrap();
    tasker.subscribe_interaction(&response).unwrap();
    tasker.subscribe_interaction(&status).unwrap();
    let mut names: BTreeMap<Handle, &str> = BTreeMap::new();
    for (c, ps) in [(&response, &netn::SMC_RESPONSE_PARAMETERS[..]), (&status, &netn::TASK_STATUS_PARAMETERS[..])] {
        for p in ps {
            names.insert(tasker.parameter(c, p).unwrap(), p);
        }
    }

    let mut s = sim("gen-kgd-company-attack");
    let mut inputs = vec![];
    let tick = |s: &mut Sim, link: &mut HlaLink, t: u32, inputs: &mut Vec<_>| {
        for i in link.inbound(s) {
            inputs.push(i.clone());
            s.apply_external(i);
        }
        s.run_with_rules(SimTime::from_secs(t as f64));
        let events = s.drain_events();
        link.tick(s, &events).unwrap();
    };
    for t in 1..=3 {
        tick(&mut s, &mut link, t, &mut inputs);
    }
    // b-1, 1 km west, by the UUID the tracks stream shows Foundry.
    let b1 = s.ctx.by_name["b-1"];
    let start = s.ctx.frame.enu_to_geodetic(s.world.get::<bgws_sim::components::Kin>(s.ctx.entity(b1)).unwrap().pos);
    let to = start.destination(270f64.to_radians(), 1000.0);
    let ecef = to.to_ecef();
    let uuid = netn::unique_id(&rpr::instance_name(1, 1, 1 + b1.index() as u16));
    let task = [7u8; 16];
    let send = |class: &Handle, params: &[(&str, Vec<u8>)]| {
        let values: Vec<(Handle, Vec<u8>)> = params.iter().map(|(n, v)| (tasker.parameter(class, n).unwrap(), v.clone())).collect();
        tasker.send_interaction(class, &values, None).unwrap();
    };
    let mut tp = bgws_interop::hla::encoding::Enc::new();
    tp.f64x3([ecef.x, ecef.y, ecef.z]).u32(0).i32(0).f32(0.0);
    send(&mv, &[("Entity", uuid.to_vec()), ("TaskId", task.to_vec()), ("UniqueId", task.to_vec()), ("TaskParameters", tp.done())]);
    send(&speed, &[("Entity", uuid.to_vec()), ("TaskId", [9u8; 16].to_vec()), ("UniqueId", [9u8; 16].to_vec()), ("TaskParameters", 5.0f32.to_be_bytes().to_vec())]);
    std::thread::sleep(Duration::from_millis(100));
    for t in 4..=400 {
        tick(&mut s, &mut link, t, &mut inputs);
    }
    std::thread::sleep(Duration::from_millis(200));

    let mut answers = vec![];
    let mut statuses: BTreeMap<[u8; 16], Vec<i32>> = BTreeMap::new();
    for c in tasker.callbacks() {
        if let Cb::ReceiveInteraction(r) = c {
            let p: BTreeMap<&str, Vec<u8>> = r.parameter_values.unwrap().parameter_handle_value.into_iter().map(|p| (names[&Handle(p.parameter_handle.unwrap().data)], p.value)).collect();
            if let Some(a) = p.get("Action") {
                answers.push((a[0], p["Status"][3]));
            } else {
                statuses.entry(p["Task"].clone().try_into().unwrap()).or_default().push(i32::from_be_bytes(p["TaskStatus"].clone().try_into().unwrap()));
            }
        }
    }
    eprintln!("answers {answers:?}; statuses {statuses:?}; notes {:?}", link.notes);
    assert!(answers.contains(&(7, 1)), "the move is accepted");
    assert!(answers.contains(&(9, 0)), "the speed change is refused");
    assert_eq!(statuses[&task], vec![0, 3, 4], "accepted, executing, completed");
    assert_eq!(statuses[&[9u8; 16]], vec![1], "refused");
    // The order went in as an input (so it is recorded and replays) and was carried out.
    assert!(inputs.iter().any(|i| matches!(i, bgws_sim::remote::ExternalInput::Order { .. })));
    let end = s.ctx.frame.enu_to_geodetic(s.world.get::<bgws_sim::components::Kin>(s.ctx.entity(b1)).unwrap().pos);
    assert!(end.surface_distance(to) < 100.0, "{:.0} m off", end.surface_distance(to));
}
