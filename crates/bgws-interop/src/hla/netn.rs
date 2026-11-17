//! What a federation built on NETN (the NATO Education and Training Network
//! FOM, over RPR) wants of an entity beyond RPR, and BGWS's event log as an
//! interaction.
//!
//! - **Identity.** NETN's MIM gives every object a `UniqueId` (a UUID, 16
//!   raw octets) and NETN's `BaseEntity` a `Callsign`. Tools built on NETN
//!   key entities by the UUID, which outlives a federation execution where
//!   an instance handle does not. Ours is a name-based (version 5) UUID of
//!   the entity's instance name, so the same entity in the same game has the
//!   same UUID every run. Both are published only where the federation's FOM
//!   has them.
//! - **Events.** `BGWS_Event`, in our own module
//!   (`crates/bgws-interop/fom/BGWS-Events.xml`), carries each event the
//!   simulation logs: kind, time, who, to whom, where, a line for people and
//!   the data behind it as JSON.

use super::encoding::{Dec, DecodeError, Enc};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;

/// NETN attributes we publish when the class has them.
/// (`SupportedActions` says which control actions it takes: [`TASK_CLASSES`].)
pub const IDENTITY_ATTRIBUTES: [&str; 3] = ["UniqueId", "Callsign", "SupportedActions"];

/// The BGWS event module's text, joined with whenever a FOM is given.
pub const EVENTS_MODULE: &str = include_str!("../../fom/BGWS-Events.xml");
pub const EVENTS_MODULE_NAME: &str = "BGWS-Events.xml";

pub const EVENT_CLASS: &str = "HLAinteractionRoot.BGWS_Event";
pub const EVENT_PARAMETERS: [&str; 16] = ["Kind", "Sequence", "SimTime", "Federate", "Side", "Entity", "EntityUniqueId", "EntityCallsign", "Target", "TargetUniqueId", "TargetCallsign", "Latitude", "Longitude", "Altitude", "Text", "Data"];

/// Data longer than this goes as `{"truncated": true}`.
pub const MAX_DATA: usize = 16 * 1024;

/// The namespace our UUIDs are made in (itself a fixed random UUID).
const NAMESPACE: [u8; 16] = [0x6b, 0x1e, 0x43, 0x0f, 0x8a, 0x52, 0x4d, 0x2c, 0x9b, 0x37, 0x51, 0xc4, 0x0e, 0x7a, 0x2d, 0x96];

/// RFC 9562 version 5 (SHA-1, name-based) UUID of an instance name.
pub fn unique_id(instance_name: &str) -> [u8; 16] {
    let mut h = Sha1::new();
    h.update(NAMESPACE);
    h.update(instance_name.as_bytes());
    let d = h.finalize();
    let mut u = [0u8; 16];
    u.copy_from_slice(&d[..16]);
    u[6] = (u[6] & 0x0f) | 0x50;
    u[8] = (u[8] & 0x3f) | 0x80;
    u
}

/// 8-4-4-4-12 lower-case hex.
pub fn uuid_text(u: &[u8; 16]) -> String {
    let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// One entity named in an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Party {
    /// Object instance name.
    pub name: String,
    pub unique_id: String,
    pub callsign: String,
}

impl Party {
    pub fn of(instance_name: &str, callsign: &str) -> Party {
        if instance_name.is_empty() {
            return Party { callsign: callsign.into(), ..Default::default() };
        }
        Party { name: instance_name.into(), unique_id: uuid_text(&unique_id(instance_name)), callsign: callsign.into() }
    }
}

/// A logged event as it goes out.
#[derive(Clone, Debug, PartialEq)]
pub struct EventOut {
    pub kind: String,
    pub seq: u64,
    pub t: f64,
    pub federate: String,
    pub side: String,
    pub entity: Party,
    pub target: Party,
    /// Latitude, longitude (degrees), height above the ellipsoid (m).
    pub at: Option<[f64; 3]>,
    pub text: String,
    pub data: String,
}

fn text(s: &str) -> Vec<u8> {
    Enc::new().unicode(s).done()
}

fn float(v: f64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// The JSON behind an event, within [`MAX_DATA`].
pub fn data_json(v: &serde_json::Value) -> String {
    if v.is_null() {
        return "{}".into();
    }
    let s = v.to_string();
    if s.len() > MAX_DATA {
        r#"{"truncated":true}"#.into()
    } else {
        s
    }
}

pub fn event_parameters(e: &EventOut) -> Vec<(&'static str, Vec<u8>)> {
    let [lat, lon, h] = e.at.unwrap_or([f64::NAN; 3]);
    vec![
        ("Kind", text(&e.kind)),
        ("Sequence", (e.seq as i64).to_be_bytes().to_vec()),
        ("SimTime", float(e.t)),
        ("Federate", text(&e.federate)),
        ("Side", text(&e.side)),
        ("Entity", text(&e.entity.name)),
        ("EntityUniqueId", text(&e.entity.unique_id)),
        ("EntityCallsign", text(&e.entity.callsign)),
        ("Target", text(&e.target.name)),
        ("TargetUniqueId", text(&e.target.unique_id)),
        ("TargetCallsign", text(&e.target.callsign)),
        ("Latitude", float(lat)),
        ("Longitude", float(lon)),
        ("Altitude", float(h)),
        ("Text", text(&e.text)),
        ("Data", text(&e.data)),
    ]
}

/// Read one back (for tests and for BGWS federates that want each other's).
pub fn read_event(params: &[(&str, &[u8])]) -> Result<EventOut, DecodeError> {
    let get = |k: &str| params.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
    let s = |k: &str| -> Result<String, DecodeError> { get(k).map(|b| Dec::new(b).unicode()).transpose().map(|x| x.unwrap_or_default()) };
    let f = |k: &str| -> Result<f64, DecodeError> { get(k).map(|b| Dec::new(b).f64()).transpose().map(|x| x.unwrap_or(f64::NAN)) };
    let party = |p: &str| -> Result<Party, DecodeError> { Ok(Party { name: s(p)?, unique_id: s(&format!("{p}UniqueId"))?, callsign: s(&format!("{p}Callsign"))? }) };
    let at = [f("Latitude")?, f("Longitude")?, f("Altitude")?];
    Ok(EventOut {
        kind: s("Kind")?,
        seq: get("Sequence").and_then(|b| b.get(..8)).map(|b| i64::from_be_bytes(b.try_into().unwrap()) as u64).unwrap_or(0),
        t: f("SimTime")?,
        federate: s("Federate")?,
        side: s("Side")?,
        entity: party("Entity")?,
        target: party("Target")?,
        at: (!at[0].is_nan()).then_some(at),
        text: s("Text")?,
        data: s("Data")?,
    })
}

// ── Tasking (NETN-SMC 1.0 / NETN-ETR 3.0) ────────────────────────────────────
//
// What the HLA–Palantir bridge sends when Foundry gives an order: a subclass of
// `SMC_EntityControl.Task` naming the entity by its UniqueId, with one
// `TaskParameters` record, or `SMC_EntityControl.CancelTasks`. Every action is
// answered with `SMC_Response` (Action = the interaction's UniqueId), and every
// task's progress with `ETR_TaskStatus`. Layouts are the bridge's
// (WIRE_CONTRACT.md): big-endian, each field aligned from the record's start,
// `LocationStruct` geocentric metres.

pub const SMC: &str = "HLAinteractionRoot.SMC_EntityControl";
pub const TASK: &str = "HLAinteractionRoot.SMC_EntityControl.Task";
pub const SMC_RESPONSE: &str = "HLAinteractionRoot.SMC_Response";
pub const SMC_RESPONSE_PARAMETERS: [&str; 2] = ["Action", "Status"];
pub const TASK_STATUS: &str = "HLAinteractionRoot.ETR_TaskStatus";
pub const TASK_STATUS_PARAMETERS: [&str; 2] = ["Task", "TaskStatus"];

/// The control actions a BGWS entity takes, with NETN's
/// `EntityControlActionEnum` numbers (for `SupportedActions`), and the ones it
/// hears only to refuse them.
pub const TASK_CLASSES: [(&str, i32, bool); 11] = [
    ("MoveToLocation", 28, true),
    ("Patrol", 33, true),
    ("MagicMove", 10, true),
    ("StopAtSideOfRoad", 39, true),
    ("ChangeRulesOfEngagement", 41, true),
    ("DirectFire", 20, true),
    ("IndirectFire", 22, true),
    ("CancelTasks", 0, true),
    ("ChangeSpeed", 36, false),
    ("ChangeHeading", 40, false),
    ("ChangeAltitude", 35, false),
];

/// The full class name of a task class (CancelTasks is not a Task).
pub fn task_class(short: &str) -> String {
    if short == "CancelTasks" {
        format!("{SMC}.{short}")
    } else {
        format!("{TASK}.{short}")
    }
}

/// `SupportedActions`: an HLAvariableArray of HLAinteger32BE.
pub fn supported_actions() -> Vec<u8> {
    let codes: Vec<i32> = TASK_CLASSES.iter().filter(|(n, c, ok)| *ok && *c != 0 && *n != "CancelTasks").map(|(_, c, _)| *c).collect();
    let mut e = Enc::new();
    e.u32(codes.len() as u32);
    for c in codes {
        e.i32(c);
    }
    e.done()
}

/// A decoded task: positions geocentric (x, y, z) metres.
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Move {
        to: [f64; 3],
        path: Vec<[f64; 3]>,
        move_type: i32,
        speed: f32,
    },
    Patrol {
        path: Vec<[f64; 3]>,
        patrol_type: i32,
        duration_ms: i64,
    },
    MagicMove {
        to: [f64; 3],
        heading_deg: f32,
    },
    Stop,
    Roe(u8),
    DirectFire {
        target: [u8; 16],
    },
    IndirectFire {
        area: Vec<[f64; 3]>,
    },
    Cancel {
        tasks: Vec<[u8; 16]>,
    },
    /// Heard, not modelled: ChangeSpeed, ChangeHeading, ChangeAltitude.
    Unsupported(String),
}

fn uuid(d: &mut Dec) -> Result<[u8; 16], DecodeError> {
    Ok(d.bytes(16)?.try_into().unwrap())
}

fn i64v(d: &mut Dec) -> Result<i64, DecodeError> {
    d.align(8)?;
    Ok(i64::from_be_bytes(d.bytes(8)?.try_into().unwrap()))
}

/// HLAvariableArray of `LocationStruct`: a count, then 8-aligned records.
fn locations(d: &mut Dec) -> Result<Vec<[f64; 3]>, DecodeError> {
    let n = d.u32()? as usize;
    if n > 10_000 {
        return Err(DecodeError(format!("{n} locations")));
    }
    (0..n).map(|_| d.f64x3()).collect()
}

/// A task from its class's last name and its `TaskParameters` (or, for
/// CancelTasks, its `Tasks`).
pub fn decode_task(short: &str, params: Option<&[u8]>) -> Result<Task, DecodeError> {
    let need = || params.ok_or_else(|| DecodeError(format!("{short} without its parameters")));
    match short {
        "MoveToLocation" => {
            let mut d = Dec::new(need()?);
            let to = d.f64x3()?;
            let path = locations(&mut d)?;
            Ok(Task::Move { to, path, move_type: d.i32()?, speed: d.f32()? })
        }
        "Patrol" => {
            let mut d = Dec::new(need()?);
            let n = d.u32()? as usize;
            if n > 10_000 {
                return Err(DecodeError(format!("{n} waypoints")));
            }
            // Waypoint {Location, Speed: float32, SegmentMaxWidth: float32}.
            let mut path = vec![];
            for _ in 0..n {
                path.push(d.f64x3()?);
                d.f32()?;
                d.f32()?;
            }
            let patrol_type = d.i32()?;
            let _move_type = d.i32()?;
            Ok(Task::Patrol { path, patrol_type, duration_ms: i64v(&mut d)? })
        }
        "MagicMove" => {
            let mut d = Dec::new(need()?);
            Ok(Task::MagicMove { to: d.f64x3()?, heading_deg: d.f32()? })
        }
        "StopAtSideOfRoad" => Ok(Task::Stop),
        "ChangeRulesOfEngagement" => Ok(Task::Roe(Dec::new(need()?).octet()?)),
        "DirectFire" => Ok(Task::DirectFire { target: uuid(&mut Dec::new(need()?))? }),
        "IndirectFire" => Ok(Task::IndirectFire { area: locations(&mut Dec::new(need()?))? }),
        "CancelTasks" => {
            let Some(b) = params else { return Ok(Task::Cancel { tasks: vec![] }) };
            let mut d = Dec::new(b);
            let n = d.u32()? as usize;
            Ok(Task::Cancel { tasks: (0..n.min(1000)).map(|_| uuid(&mut d)).collect::<Result<_, _>>()? })
        }
        other => Ok(Task::Unsupported(other.to_string())),
    }
}

/// `SMC_Response` {Action: UUID, Status: HLAboolean}.
pub fn smc_response(action: &[u8; 16], ok: bool) -> Vec<(&'static str, Vec<u8>)> {
    vec![("Action", action.to_vec()), ("Status", (ok as u32).to_be_bytes().to_vec())]
}

/// `ETR_TaskStatus` {Task: UUID, TaskStatus: TaskStatusEnum32}.
pub fn task_status(task: &[u8; 16], status: i32) -> Vec<(&'static str, Vec<u8>)> {
    vec![("Task", task.to_vec()), ("TaskStatus", status.to_be_bytes().to_vec())]
}

/// A UUID's text form back to its 16 octets.
pub fn uuid_from_text(t: &str) -> Option<[u8; 16]> {
    let h: String = t.chars().filter(|c| *c != '-').collect();
    if h.len() != 32 {
        return None;
    }
    let mut u = [0u8; 16];
    for (k, b) in u.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[2 * k..2 * k + 2], 16).ok()?;
    }
    Some(u)
}

// ── Control (BGWS_Control, our module) ──────────────────────────────────────
//
// A BGWS simulation waiting in the federation (`bgws federate`) is told what to
// play and when: load a scenario, start, pause, resume, stop, speed. RPR's
// StartResume and StopFreeze, which other tools send, count as start and pause.

pub const CONTROL_CLASS: &str = "HLAinteractionRoot.BGWS_Control";
pub const CONTROL_PARAMETERS: [&str; 5] = ["Federate", "Command", "RequestId", "Scenario", "Options"];
pub const START_RESUME: &str = "HLAinteractionRoot.StartResume";
pub const STOP_FREEZE: &str = "HLAinteractionRoot.StopFreeze";

/// One control message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlMsg {
    /// The federate addressed; empty for all.
    pub federate: String,
    pub command: String,
    pub request_id: String,
    pub scenario: String,
    /// A JSON object (`{}` when none).
    pub options: String,
}

impl ControlMsg {
    /// Is it for this federate?
    pub fn for_me(&self, me: &str) -> bool {
        self.federate.is_empty() || self.federate == me
    }
    /// `options.speed`, if given.
    pub fn speed(&self) -> Option<f64> {
        serde_json::from_str::<serde_json::Value>(&self.options).ok()?.get("speed")?.as_f64().filter(|s| *s > 0.0)
    }
}

pub fn control_parameters(c: &ControlMsg) -> Vec<(&'static str, Vec<u8>)> {
    vec![("Federate", text(&c.federate)), ("Command", text(&c.command)), ("RequestId", text(&c.request_id)), ("Scenario", text(&c.scenario)), ("Options", text(if c.options.is_empty() { "{}" } else { &c.options }))]
}

pub fn read_control(params: &BTreeMap<String, Vec<u8>>) -> Result<ControlMsg, DecodeError> {
    let s = |k: &str| -> Result<String, DecodeError> { params.get(k).map(|b| Dec::new(b).unicode()).transpose().map(|x| x.unwrap_or_default()) };
    Ok(ControlMsg { federate: s("Federate")?, command: s("Command")?.trim().to_lowercase(), request_id: s("RequestId")?, scenario: s("Scenario")?, options: s("Options")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_is_version_5_and_stable() {
        let a = unique_id("BGWS-1-1-7");
        assert_eq!(a, unique_id("BGWS-1-1-7"));
        assert_ne!(a, unique_id("BGWS-1-1-8"));
        assert_eq!(a[6] >> 4, 5);
        assert_eq!(a[8] >> 6, 0b10);
        let t = uuid_text(&a);
        assert_eq!(t.len(), 36);
        assert_eq!(&t[14..15], "5");
    }

    #[test]
    fn event_round_trip() {
        let e = EventOut {
            kind: "fire.hit".into(),
            seq: 42,
            t: 125.0,
            federate: "BGWS-1-1".into(),
            side: "blue".into(),
            entity: Party::of("BGWS-1-1-3", "1 Pl Tank 2"),
            target: Party::of("", ""),
            at: Some([54.68, 20.47, 72.5]),
            text: "a hit".into(),
            data: data_json(&serde_json::json!({"p": 0.4})),
        };
        let params = event_parameters(&e);
        assert_eq!(params.len(), EVENT_PARAMETERS.len());
        for ((n, _), m) in params.iter().zip(EVENT_PARAMETERS) {
            assert_eq!(*n, m);
        }
        // HLAinteger64BE and HLAfloat64BE: 8 octets, big-endian.
        assert_eq!(params[1].1, vec![0, 0, 0, 0, 0, 0, 0, 42]);
        assert_eq!(params[2].1, 125.0f64.to_be_bytes().to_vec());
        let refs: Vec<(&str, &[u8])> = params.iter().map(|(n, v)| (*n, v.as_slice())).collect();
        assert_eq!(read_event(&refs).unwrap(), e);
    }

    #[test]
    fn big_data_is_truncated() {
        let big = serde_json::json!({ "x": "y".repeat(MAX_DATA) });
        assert_eq!(data_json(&big), r#"{"truncated":true}"#);
        assert_eq!(data_json(&serde_json::Value::Null), "{}");
    }

    #[test]
    fn module_parses_and_names_match() {
        let m = super::super::fom::Module::parse(EVENTS_MODULE_NAME, EVENTS_MODULE).unwrap();
        let (path, params) = m.interactions.iter().find(|(p, _)| p.last().map(String::as_str) == Some("BGWS_Event")).expect("BGWS_Event");
        assert_eq!(path.join("."), EVENT_CLASS);
        assert_eq!(params.iter().map(String::as_str).collect::<Vec<_>>(), EVENT_PARAMETERS.to_vec());
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().filter(|w| *w != "|").map(|w| u8::from_str_radix(w, 16).unwrap()).collect()
    }

    // WIRE_CONTRACT.md's golden vectors: latitude 60.476, longitude 25.116,
    // 500 m, as ECEF.
    const LOC: &str = "41 45 C4 F6 65 A4 DA 41 41 34 68 ED C5 36 C1 ED 41 55 15 B1 8C FD 3D 16";

    fn near(a: [f64; 3]) {
        let want = [2853356.7941, 1337581.7704, 5527238.2030];
        for k in 0..3 {
            assert!((a[k] - want[k]).abs() < 1e-3, "{a:?}");
        }
    }

    #[test]
    fn magic_move_golden_vector_b() {
        let b = hex(&format!("{LOC} 43 87 00 00"));
        let Task::MagicMove { to, heading_deg } = decode_task("MagicMove", Some(&b)).unwrap() else { panic!() };
        near(to);
        assert_eq!(heading_deg, 270.0);
    }

    #[test]
    fn move_to_location_golden_vector_c_36_octets() {
        // Empty Path is the count alone (Pitch's encoder, which the bridge uses).
        let b = hex(&format!("{LOC} 00 00 00 00 | 00 00 00 00 | 41 F0 00 00"));
        assert_eq!(b.len(), 36);
        let Task::Move { to, path, move_type, speed } = decode_task("MoveToLocation", Some(&b)).unwrap() else { panic!() };
        near(to);
        assert!(path.is_empty());
        assert_eq!((move_type, speed), (0, 30.0));
    }

    #[test]
    fn a_path_pads_after_its_count_golden_vector_d() {
        // A 3-element LocationStructArray: count, 4 octets padding, 3 x 24.
        let mut b = hex("00 00 00 03 00 00 00 00");
        for _ in 0..3 {
            b.extend(hex(LOC));
        }
        assert_eq!(b.len(), 80);
        let Task::IndirectFire { area } = decode_task("IndirectFire", Some(&b)).unwrap() else { panic!() };
        assert_eq!(area.len(), 3);
        near(area[2]);
    }

    #[test]
    fn patrol_waypoints_and_the_rest() {
        let mut b = hex("00 00 00 02 00 00 00 00");
        for _ in 0..2 {
            b.extend(hex(LOC));
            b.extend(hex("41 20 00 00 00 00 00 00"));
        }
        b.extend(hex("00 00 00 01 00 00 00 03"));
        b.extend(3_600_000i64.to_be_bytes());
        b.extend(60_000i64.to_be_bytes());
        let Task::Patrol { path, patrol_type, duration_ms } = decode_task("Patrol", Some(&b)).unwrap() else { panic!() };
        assert_eq!((path.len(), patrol_type, duration_ms), (2, 1, 3_600_000));
        near(path[1]);
    }

    #[test]
    fn short_parameters_are_an_error_not_a_guess() {
        assert!(decode_task("MagicMove", Some(&hex(LOC))).is_err());
        assert!(decode_task("MoveToLocation", None).is_err());
        assert_eq!(decode_task("ChangeSpeed", Some(&[0, 0, 0, 0])).unwrap(), Task::Unsupported("ChangeSpeed".into()));
    }

    #[test]
    fn answers_and_supported_actions() {
        let u = unique_id("BGWS-1-1-1");
        assert_eq!(uuid_from_text(&uuid_text(&u)), Some(u));
        assert_eq!(smc_response(&u, true)[1].1, vec![0, 0, 0, 1]);
        assert_eq!(task_status(&u, 3)[1].1, vec![0, 0, 0, 3]);
        let sa = supported_actions();
        assert_eq!(&sa[..4], &[0, 0, 0, 7]);
        assert_eq!(&sa[4..8], &28i32.to_be_bytes());
    }

    #[test]
    fn control_round_trip() {
        let c = ControlMsg { federate: "BGWS-1-1".into(), command: "load".into(), request_id: "r1".into(), scenario: "{\"format\":\"bgws.foundry\"}".into(), options: "{\"speed\": 4}".into() };
        let m: BTreeMap<String, Vec<u8>> = control_parameters(&c).into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let back = read_control(&m).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.speed(), Some(4.0));
        assert!(back.for_me("BGWS-1-1") && !back.for_me("BGWS-2-1"));
        assert!(ControlMsg::default().for_me("anyone"));
    }
}
