//! `bgws federate`: a simulation waiting in a federation is sent its
//! scenario over HLA (BGWS_Control, as the HLA–Palantir bridge sends it from
//! Foundry), shows its units before it starts, plays when started, pauses,
//! and stops (its units leave the federation). It
//! answers each command as a federate.status event.

use bgws_interop::hla::link::{HlaLink, HlaSettings};
use bgws_interop::hla::mock::MockRti;
use bgws_interop::hla::netn::{self, ControlMsg};
use bgws_interop::hla::proto::callback_request::CallbackRequest as Cb;
use bgws_interop::hla::rti::{Handle, Rti};
use bgws_run::federate::{Federate, FederateOptions};
use bgws_scenario::v4::InteropSpec;
use serde_json::json;
use std::collections::BTreeMap;

fn scenario() -> String {
    json!({
        "format": "bgws.foundry", "version": 1, "name": "Op Wait", "seed": 3, "durationMin": 20,
        "area": {"south": 54.65, "west": 20.40, "north": 54.75, "east": 20.55}, "terrain": "generated",
        "command": {"blue": "rules", "red": "rules"},
        "units": [
            {"id": "b-1", "label": "Blue 1 tanks", "side": "blue", "platform": "generic/mbt", "count": 4, "lat": 54.67, "lon": 20.46},
            {"id": "b-2", "label": "Blue 2 mech", "side": "blue", "platform": "generic/ifv", "count": 4, "lat": 54.67, "lon": 20.47},
            {"id": "r-1", "label": "Red 1 platoon", "side": "red", "platform": "generic/ifv", "count": 3, "lat": 54.73, "lon": 20.47}
        ],
        "objectives": [{"side": "blue", "label": "OBJ GOLD", "lat": 54.73, "lon": 20.47}]
    })
    .to_string()
}

struct Foundry {
    rti: Rti,
    control: Handle,
    event: Handle,
    params: BTreeMap<Handle, &'static str>,
}

impl Foundry {
    fn new(addr: &str) -> Foundry {
        let rti = Rti::connect(addr, Default::default()).unwrap();
        rti.join("bridge", "test", "f", &[]).unwrap();
        let control = rti.interaction_class(netn::CONTROL_CLASS).unwrap();
        rti.publish_interaction(&control).unwrap();
        let event = rti.interaction_class(netn::EVENT_CLASS).unwrap();
        let mut params = BTreeMap::new();
        for p in netn::EVENT_PARAMETERS {
            params.insert(rti.parameter(&event, p).unwrap(), p);
        }
        rti.subscribe_interaction(&event).unwrap();
        let ground = rti.object_class(bgws_interop::hla::rpr::CLASSES[0]).unwrap();
        let marking = rti.attribute(&ground, "Marking").unwrap();
        rti.subscribe_object(&ground, &[marking]).unwrap();
        Foundry { rti, control, event, params }
    }
    fn send(&self, command: &str, request: &str, scenario: &str, options: &str) {
        let c = ControlMsg { federate: String::new(), command: command.into(), request_id: request.into(), scenario: scenario.into(), options: options.into() };
        let v: Vec<(Handle, Vec<u8>)> = netn::control_parameters(&c).into_iter().map(|(k, v)| (self.rti.parameter(&self.control, k).unwrap(), v)).collect();
        self.rti.send_interaction(&self.control, &v, None).unwrap();
    }
    /// (state, requestId, message) of each status, and entities discovered and removed.
    fn read(&self) -> (Vec<(String, String, String)>, usize, usize) {
        let (mut statuses, mut found, mut gone) = (vec![], 0, 0);
        for cb in self.rti.callbacks() {
            match cb {
                Cb::ReceiveInteraction(r) if Handle(r.interaction_class.clone().unwrap().data) == self.event => {
                    let p: Vec<(&str, Vec<u8>)> = r.parameter_values.unwrap().parameter_handle_value.into_iter().map(|x| (self.params[&Handle(x.parameter_handle.unwrap().data)], x.value)).collect();
                    let refs: Vec<(&str, &[u8])> = p.iter().map(|(n, v)| (*n, v.as_slice())).collect();
                    let e = netn::read_event(&refs).unwrap();
                    if e.kind == "federate.status" {
                        let d: serde_json::Value = serde_json::from_str(&e.data).unwrap();
                        statuses.push((d["state"].as_str().unwrap().to_string(), d["requestId"].as_str().unwrap().to_string(), d["message"].as_str().unwrap().to_string()));
                    }
                }
                Cb::DiscoverObjectInstance(_) => found += 1,
                Cb::RemoveObjectInstance(_) => gone += 1,
                _ => {}
            }
        }
        (statuses, found, gone)
    }
}

#[test]
fn a_waiting_federate_is_loaded_started_paused_and_stopped_over_hla() {
    let rti = MockRti::start().unwrap();
    let addr = rti.address();
    let creator = Rti::connect(&addr, Default::default()).unwrap();
    creator.create_federation("f", &[]).unwrap();
    let link = HlaLink::join(HlaSettings::new(&addr, "f", "BGWS-1-1"), InteropSpec { site: 1, application: 1, ..Default::default() }).unwrap();
    let foundry = Foundry::new(&addr);
    let packs = std::env::temp_dir().join(format!("bgws-federate-test-{}", std::process::id()));
    let mut fed = Federate::new(link, InteropSpec { site: 1, application: 1, ..Default::default() }, FederateOptions { packs, allow_generated: true, speed: 1.0, records: None, cell_m: 30.0 });
    let steps = |fed: &mut Federate, n: usize| {
        for _ in 0..n {
            fed.step().unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };

    // Nothing to start yet.
    foundry.send("start", "r0", "", "{}");
    steps(&mut fed, 3);
    // Load: its units appear before it starts.
    foundry.send("load", "r1", &scenario(), "{}");
    steps(&mut fed, 3);
    assert_eq!(fed.state(), "ready");
    let (s1, found, _) = foundry.read();
    eprintln!("{s1:?}, {found} discovered");
    assert!(s1.iter().any(|(st, r, m)| st == "idle" && r == "r0" && m.contains("nothing loaded")));
    assert!(s1.iter().any(|(st, r, _)| st == "ready" && r == "r1"));
    assert!(found >= 2, "units published before the start");
    assert_eq!(fed.sim().unwrap().now().secs(), 0.0);

    // Start, fast.
    foundry.send("start", "r2", "", r#"{"speed": 500}"#);
    steps(&mut fed, 40);
    assert_eq!(fed.state(), "running");
    let t = fed.sim().unwrap().now().secs();
    assert!(t >= 20.0, "it plays ({t} s)");
    // Pause holds the clock.
    foundry.send("pause", "r3", "", "{}");
    steps(&mut fed, 3);
    let held = fed.sim().unwrap().now();
    steps(&mut fed, 10);
    assert_eq!(fed.sim().unwrap().now(), held);
    assert_eq!(fed.state(), "paused");
    // Stop: its units leave.
    foundry.send("stop", "r4", "", "{}");
    steps(&mut fed, 3);
    assert_eq!(fed.state(), "idle");
    let (s2, _, gone) = foundry.read();
    eprintln!("{s2:?}, {gone} removed");
    for (state, r) in [("running", "r2"), ("paused", "r3"), ("idle", "r4")] {
        assert!(s2.iter().any(|(st, rq, _)| st == state && rq == r), "{state} for {r}");
    }
    assert!(gone >= 2, "units taken off");
    // A bad scenario is refused with the reason.
    foundry.send("load", "r5", r#"{"format": "bgws.foundry", "version": 1, "area": {"south": 1, "west": 1, "north": 0, "east": 2}, "units": []}"#, "{}");
    steps(&mut fed, 3);
    let (s3, _, _) = foundry.read();
    assert!(s3.iter().any(|(_, r, m)| r == "r5" && m.contains("refused")), "{s3:?}");
}
