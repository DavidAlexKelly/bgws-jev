//! The BGWS RTI (`bgws mock-rti`) beyond what BGWS federates use: a FOM's
//! class hierarchy in discovery, reflection and interactions; names checked
//! against the FOM; next-message time advance; the management objects.

use bgws_interop::hla::encoding::Dec;
use bgws_interop::hla::mock::MockRti;
use bgws_interop::hla::proto::callback_request::CallbackRequest as Cb;
use bgws_interop::hla::rti::{float_time_of, FomFile, Handle, Rti, RtiError};
use bgws_interop::hla::session::SessionSettings;
use std::collections::BTreeMap;
use std::time::Duration;

const W: Duration = Duration::from_secs(5);

fn fom() -> FomFile {
    FomFile::read(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/hierarchy.xml")).unwrap()
}

fn settings() -> SessionSettings {
    SessionSettings { response_timeout: W, ..Default::default() }
}

fn join(rti: &MockRti, name: &str, foms: &[FomFile]) -> Rti {
    let r = Rti::connect(&rti.address(), settings()).unwrap();
    r.create_federation("fx", foms).unwrap();
    r.join(name, "test", "fx", &[]).unwrap();
    r
}

fn reflected(c: &Cb) -> Option<BTreeMap<Handle, Vec<u8>>> {
    let m = match c {
        Cb::ReflectAttributeValues(r) => r.attribute_values.clone(),
        Cb::ReflectAttributeValuesWithTime(r) => r.attribute_values.clone(),
        _ => return None,
    };
    Some(m?.attribute_handle_value.into_iter().map(|a| (Handle(a.attribute_handle.unwrap().data), a.value)).collect())
}

#[test]
fn subscribers_at_a_superclass_get_it_as_that_class() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let (b, c) = (join(&rti, "b", &[]), join(&rti, "c", &[]));
    let gv = a.object_class("GroundVehicle").unwrap();
    let base = b.object_class("BaseEntity").unwrap();
    let phys = c.object_class("HLAobjectRoot.BaseEntity.PhysicalEntity").unwrap();
    let (ty, dmg) = (a.attribute(&gv, "EntityType").unwrap(), a.attribute(&gv, "DamageState").unwrap());
    // Inherited: one handle whichever class it is asked at.
    assert_eq!(b.attribute(&base, "EntityType").unwrap(), ty);
    // The FOM decides what exists.
    assert!(matches!(a.object_class("Tank"), Err(RtiError::Hla { name, .. }) if name == "NameNotFound"));
    assert!(matches!(b.attribute(&base, "DamageState"), Err(RtiError::Hla { name, .. }) if name == "NameNotFound"), "declared below BaseEntity");

    b.subscribe_object(&base, std::slice::from_ref(&ty)).unwrap();
    c.subscribe_object(&phys, &[ty.clone(), dmg.clone()]).unwrap();
    a.publish_object(&gv, &[ty.clone(), dmg.clone()]).unwrap();
    let tank = a.register(&gv).unwrap();
    a.update(&tank, &[(ty.clone(), vec![1]), (dmg.clone(), vec![2])], None).unwrap();

    for (who, class, want) in [(&b, &base, vec![ty.clone()]), (&c, &phys, vec![ty.clone(), dmg.clone()])] {
        let d = who.wait_for(W, |c| matches!(c, Cb::DiscoverObjectInstance(_))).expect("discovered");
        let Cb::DiscoverObjectInstance(d) = d else { unreachable!() };
        assert_eq!(Handle(d.object_class.unwrap().data), *class, "known as the class subscribed to");
        let r = who.wait_for(W, |c| reflected(c).is_some()).map(|c| reflected(&c).unwrap()).expect("reflected");
        assert_eq!(r.keys().cloned().collect::<Vec<_>>(), want, "only the attributes subscribed there");
    }
}

#[test]
fn an_interaction_reaches_subscribers_of_a_superclass_as_that_class() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let b = join(&rti, "b", &[]);
    let (fire, launch) = (a.interaction_class("WeaponFire").unwrap(), a.interaction_class("WeaponFire.MissileLaunch").unwrap());
    let (event, seeker) = (a.parameter(&launch, "EventIdentifier").unwrap(), a.parameter(&launch, "SeekerType").unwrap());
    assert_eq!(a.parameter(&fire, "EventIdentifier").unwrap(), event, "inherited");
    b.subscribe_interaction(&fire).unwrap();
    a.publish_interaction(&launch).unwrap();
    a.send_interaction(&launch, &[(event.clone(), vec![7]), (seeker.clone(), vec![9])], None).unwrap();
    let Some(Cb::ReceiveInteraction(r)) = b.wait_for(W, |c| matches!(c, Cb::ReceiveInteraction(_))) else { panic!("not received") };
    assert_eq!(Handle(r.interaction_class.unwrap().data), fire, "received as the class subscribed to");
    let params: Vec<Handle> = r.parameter_values.unwrap().parameter_handle_value.into_iter().map(|p| Handle(p.parameter_handle.unwrap().data)).collect();
    assert_eq!(params, vec![event], "only WeaponFire's parameters");
}

#[test]
fn a_next_message_request_is_granted_at_the_next_message() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let b = join(&rti, "b", &[]);
    for r in [&a, &b] {
        r.enable_time_regulation(1.0, W).unwrap();
        r.enable_time_constrained(W).unwrap();
    }
    let fire = a.interaction_class("WeaponFire").unwrap();
    a.subscribe_interaction(&fire).unwrap();
    b.publish_interaction(&fire).unwrap();
    b.send_interaction(&fire, &[], Some(5.5)).unwrap();
    let grant = |r: &Rti| -> (Vec<Cb>, f64) {
        let (before, g) = r.wait_until(W, |c| matches!(c, Cb::TimeAdvanceGrant(_))).expect("granted");
        let Cb::TimeAdvanceGrant(g) = g else { unreachable!() };
        (before, float_time_of(&g.time.unwrap().data).unwrap())
    };
    a.next_message_request(10.0).unwrap();
    b.next_message_request(10.0).unwrap();
    let (before, t) = grant(&a);
    assert_eq!(t, 5.5, "granted at the message");
    assert!(before.iter().any(|c| matches!(c, Cb::ReceiveInteractionWithTime(_))), "the message comes first");
    a.next_message_request(10.0).unwrap();
    assert_eq!(grant(&a).1, 10.0);
    assert_eq!(grant(&b).1, 10.0);
}

#[test]
fn the_management_objects_show_the_federates() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let class = a.object_class("HLAmanager.HLAfederate").unwrap();
    let (name, regulating) = (a.attribute(&class, "HLAfederateName").unwrap(), a.attribute(&class, "HLAtimeRegulating").unwrap());
    a.subscribe_object(&class, &[name.clone(), regulating.clone()]).unwrap();
    let b = join(&rti, "b", &[]);
    b.enable_time_regulation(1.0, W).unwrap();
    // Collect what comes: each federate's name, and b becoming regulating.
    let mut names = std::collections::BTreeSet::new();
    let mut b_regulating = false;
    let end = std::time::Instant::now() + W;
    while std::time::Instant::now() < end && !(names.len() == 2 && b_regulating) {
        for c in a.callbacks() {
            if let Some(v) = reflected(&c) {
                if let Some(n) = v.get(&name) {
                    names.insert(Dec::new(n).unicode().unwrap());
                }
                if v.get(&regulating).is_some_and(|x| x == &[0, 0, 0, 1]) {
                    b_regulating = true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(names, ["a", "b"].into_iter().map(String::from).collect());
    assert!(b_regulating, "b's HLAtimeRegulating became true");
}

#[test]
fn a_bad_fom_is_refused() {
    let rti = MockRti::start().unwrap();
    let r = Rti::connect(&rti.address(), settings()).unwrap();
    let bad = FomFile { name: "bad.xml".into(), content: b"<objectModel><objects><objectClass><name>Thing</name></objectClass></objects></objectModel>".to_vec() };
    assert!(matches!(r.create_federation("fx", &[bad]), Err(RtiError::Hla { name, .. }) if name == "ErrorReadingFDD"));
}

#[test]
fn unsubscribing_and_unpublishing_stop_the_flow() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let b = join(&rti, "b", &[]);
    let gv = a.object_class("GroundVehicle").unwrap();
    let ty = a.attribute(&gv, "EntityType").unwrap();
    let fire = a.interaction_class("WeaponFire").unwrap();
    // Registering or sending what is not published is refused.
    assert!(matches!(a.register(&gv), Err(RtiError::Hla { name, .. }) if name == "ObjectClassNotPublished"));
    assert!(matches!(a.send_interaction(&fire, &[], None), Err(RtiError::Hla { name, .. }) if name == "InteractionClassNotPublished"));
    a.publish_object(&gv, std::slice::from_ref(&ty)).unwrap();
    a.publish_interaction(&fire).unwrap();
    b.subscribe_object(&gv, std::slice::from_ref(&ty)).unwrap();
    b.subscribe_interaction(&fire).unwrap();
    let tank = a.register(&gv).unwrap();
    a.update(&tank, &[(ty.clone(), vec![1])], None).unwrap();
    a.send_interaction(&fire, &[], None).unwrap();
    assert!(b.wait_for(W, |c| reflected(c).is_some()).is_some());
    assert!(b.wait_for(W, |c| matches!(c, Cb::ReceiveInteraction(_))).is_some());
    // Unsubscribed: nothing more comes.
    b.unsubscribe_object(&gv).unwrap();
    b.unsubscribe_interaction(&fire).unwrap();
    a.update(&tank, &[(ty.clone(), vec![2])], None).unwrap();
    a.send_interaction(&fire, &[], None).unwrap();
    assert!(b.wait_for(Duration::from_millis(300), |c| reflected(c).is_some() || matches!(c, Cb::ReceiveInteraction(_))).is_none());
    // Unpublished: refused again.
    a.unpublish_interaction(&fire).unwrap();
    assert!(a.send_interaction(&fire, &[], None).is_err());
}

#[test]
fn the_management_objects_count_what_a_federate_does() {
    let rti = MockRti::start().unwrap();
    let a = join(&rti, "a", &[fom()]);
    let b = join(&rti, "b", &[]);
    let class = b.object_class("HLAmanager.HLAfederate").unwrap();
    let (name, sent, fdd) = (b.attribute(&class, "HLAfederateName").unwrap(), b.attribute(&class, "HLAupdatesSent").unwrap(), b.attribute(&class, "HLAFOMmoduleDesignatorList").unwrap());
    b.subscribe_object(&class, &[name.clone(), sent.clone(), fdd.clone()]).unwrap();
    let gv = a.object_class("GroundVehicle").unwrap();
    let ty = a.attribute(&gv, "EntityType").unwrap();
    a.publish_object(&gv, std::slice::from_ref(&ty)).unwrap();
    let tank = a.register(&gv).unwrap();
    for i in 0..5u8 {
        a.update(&tank, &[(ty.clone(), vec![i])], None).unwrap();
    }
    // Counts come within the report period (a second).
    std::thread::sleep(Duration::from_millis(1100));
    a.update(&tank, &[(ty.clone(), vec![9])], None).unwrap();
    let mut a_obj = None;
    let mut a_sent = 0;
    let mut modules = vec![];
    let end = std::time::Instant::now() + W;
    while std::time::Instant::now() < end && a_sent < 6 {
        for c in b.callbacks() {
            let (h, v) = match &c {
                Cb::ReflectAttributeValues(r) => (Handle(r.object_instance.clone().unwrap().data), reflected(&c).unwrap()),
                _ => continue,
            };
            if v.get(&name).is_some_and(|n| Dec::new(n).unicode().unwrap() == "a") {
                a_obj = Some(h.clone());
                if let Some(m) = v.get(&fdd) {
                    let mut d = Dec::new(m);
                    modules = (0..d.u32().unwrap()).map(|_| d.unicode().unwrap()).collect();
                }
            }
            if Some(&h) == a_obj.as_ref() {
                if let Some(n) = v.get(&sent) {
                    a_sent = Dec::new(n).i32().unwrap();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(a_sent, 6, "a's HLAupdatesSent");
    assert!(modules.is_empty(), "a joined with no module of its own (it created the federation with one): {modules:?}");
}
