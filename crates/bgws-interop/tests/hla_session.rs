//! The Federate Protocol session layer against the mock RTI
//! (docs/design/INTEROP_PLAN.md phase 3): heartbeats keep a quiet session
//! alive, a dropped connection is resumed with nothing lost or repeated, and
//! call responses go back to the calls that made them under load.

use bgws_interop::hla::mock::MockRti;
use bgws_interop::hla::proto::callback_request::CallbackRequest as Cb;
use bgws_interop::hla::rti::{Handle, Rti};
use bgws_interop::hla::session::SessionSettings;
use std::sync::Arc;
use std::time::Duration;

fn quick() -> SessionSettings {
    SessionSettings { heartbeat: Duration::from_millis(50), timeout: Duration::from_secs(5), reconnect_limit: Duration::from_secs(5), reconnect_delay: Duration::from_millis(20), response_timeout: Duration::from_secs(5) }
}

fn federate(rti: &MockRti, name: &str) -> Rti {
    let r = Rti::connect(&rti.address(), quick()).unwrap();
    r.create_federation("test", &[]).unwrap();
    r.join(name, "bgws", "test", &[]).unwrap();
    r
}

#[test]
fn a_quiet_session_sends_heartbeats_and_stays_up() {
    let rti = MockRti::start().unwrap();
    let a = federate(&rti, "a");
    std::thread::sleep(Duration::from_millis(400));
    assert!(rti.heartbeats() >= 4, "{} heartbeats", rti.heartbeats());
    // Still answering.
    a.object_class("HLAobjectRoot.BaseEntity").unwrap();
    assert_eq!(a.session().resumes(), 0);
}

#[test]
fn a_dropped_connection_resumes_with_nothing_lost_or_repeated() {
    let rti = MockRti::start().unwrap();
    let a = federate(&rti, "a");
    let b = federate(&rti, "b");
    let class = a.object_class("HLAobjectRoot.BaseEntity.PhysicalEntity").unwrap();
    let attr = a.attribute(&class, "Marking").unwrap();
    b.subscribe_object(&class, std::slice::from_ref(&attr)).unwrap();
    a.publish_object(&class, std::slice::from_ref(&attr)).unwrap();
    let inst = a.register(&class).unwrap();
    for i in 0u32..300 {
        if i % 60 == 30 {
            rti.drop_connections();
        }
        if i % 60 == 45 {
            a.session().drop_connection();
            b.session().drop_connection();
        }
        a.update(&inst, &[(attr.clone(), i.to_be_bytes().to_vec())], None).unwrap();
    }
    let mut seen = vec![];
    let mut discovered = 0;
    while seen.len() < 300 {
        let Some((before, last)) = b.wait_until(Duration::from_secs(5), |c| matches!(c, Cb::ReflectAttributeValues(_))) else { break };
        for c in before.into_iter().chain([last]) {
            match c {
                Cb::DiscoverObjectInstance(_) => discovered += 1,
                Cb::ReflectAttributeValues(r) => seen.push(u32::from_be_bytes(r.attribute_values.unwrap().attribute_handle_value[0].value.clone().try_into().unwrap())),
                _ => {}
            }
        }
    }
    assert_eq!(discovered, 1);
    assert_eq!(seen, (0..300).collect::<Vec<_>>(), "every update once, in order");
    assert!(a.session().resumes() >= 5 && b.session().resumes() >= 5, "{} {}", a.session().resumes(), b.session().resumes());
}

#[test]
fn call_responses_match_their_calls_under_load() {
    let rti = MockRti::start().unwrap();
    let a = Arc::new(federate(&rti, "a"));
    let class = a.object_class("HLAobjectRoot.BaseEntity").unwrap();
    let names: Vec<String> = (0..40).map(|i| format!("attribute{i}")).collect();
    let expected: Vec<Handle> = names.iter().map(|n| a.attribute(&class, n).unwrap()).collect();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let (a, class, names, expected) = (a.clone(), class.clone(), names.clone(), expected.clone());
            std::thread::spawn(move || {
                for k in 0..250 {
                    let i = (k * 7 + t * 13) % names.len();
                    assert_eq!(a.attribute(&class, &names[i]).unwrap(), expected[i], "thread {t}, call {k}");
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert!(rti.calls() >= 2000);
}

/// What Pitch's Chat sample uses beyond BGWS's own calls: short class names,
/// directed interactions and requests for attribute values.
#[test]
fn the_mock_serves_what_the_chat_sample_uses() {
    use bgws_interop::hla::proto::{self as p, call_request::CallRequest as Rq, call_response::CallResponse as Rs};
    use bgws_interop::hla::session::Session;
    use prost::Message;
    let rti = MockRti::start().unwrap();
    let (a, b) = (federate(&rti, "a"), federate(&rti, "b"));
    // `Participant` and `HLAobjectRoot.Participant` are one class.
    assert_eq!(a.object_class("Participant").unwrap(), a.object_class("HLAobjectRoot.Participant").unwrap());
    assert_eq!(a.interaction_class("Communication").unwrap(), b.interaction_class("HLAinteractionRoot.Communication").unwrap());
    let class = a.object_class("Participant").unwrap();
    let name = a.attribute(&class, "Name").unwrap();
    let comm = a.interaction_class("Communication").unwrap();
    let raw = |s: &Session, rq: Rq| -> Rs { p::CallResponse::decode(s.call(&p::CallRequest { call_request: Some(rq) }.encode_to_vec()).unwrap().as_slice()).unwrap().call_response.unwrap() };
    let set = Some(p::InteractionClassHandleSet { interaction_class_handle: vec![p::InteractionClassHandle { data: comm.0.clone() }] });
    for (s, sub) in [(a.session(), false), (b.session(), true)] {
        let oc = Some(p::ObjectClassHandle { data: class.0.clone() });
        let rq = if sub {
            Rq::SubscribeObjectClassDirectedInteractionsRequest(p::SubscribeObjectClassDirectedInteractionsRequest { object_class: oc, interaction_classes: set.clone() })
        } else {
            Rq::PublishObjectClassDirectedInteractionsRequest(p::PublishObjectClassDirectedInteractionsRequest { object_class: oc, interaction_classes: set.clone() })
        };
        assert!(!matches!(raw(s, rq), Rs::ExceptionData(_)));
    }
    b.subscribe_object(&class, std::slice::from_ref(&name)).unwrap();
    a.publish_object(&class, std::slice::from_ref(&name)).unwrap();
    assert!(a.reserve_name("alice", Duration::from_secs(5)).unwrap());
    let alice = a.register_with_name(&class, "alice").unwrap();
    // B asks for alice's name: A is asked to provide it.
    let rq = Rq::RequestInstanceAttributeValueUpdateRequest(p::RequestInstanceAttributeValueUpdateRequest {
        object_instance: Some(p::ObjectInstanceHandle { data: alice.0.clone() }),
        attributes: Some(p::AttributeHandleSet { attribute_handle: vec![p::AttributeHandle { data: name.0.clone() }] }),
        user_supplied_tag: vec![],
    });
    assert!(!matches!(raw(b.session(), rq), Rs::ExceptionData(_)));
    assert!(a.wait_for(Duration::from_secs(5), |c| matches!(c, Cb::ProvideAttributeValueUpdate(_))).is_some(), "the owner is asked to update");
    // A directed interaction to alice reaches B, subscribed at Participant.
    let rq = Rq::SendDirectedInteractionRequest(p::SendDirectedInteractionRequest {
        interaction_class: Some(p::InteractionClassHandle { data: comm.0.clone() }),
        object_instance: Some(p::ObjectInstanceHandle { data: alice.0.clone() }),
        parameter_values: None,
        user_supplied_tag: vec![],
    });
    assert!(!matches!(raw(a.session(), rq), Rs::ExceptionData(_)));
    assert!(b.wait_for(Duration::from_secs(5), |c| matches!(c, Cb::ReceiveDirectedInteraction(_))).is_some(), "the directed interaction arrives");
}
