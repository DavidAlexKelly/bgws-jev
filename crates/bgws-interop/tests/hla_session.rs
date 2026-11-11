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
