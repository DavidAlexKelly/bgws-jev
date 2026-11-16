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

/// NETN attributes we publish when the class has them.
pub const IDENTITY_ATTRIBUTES: [&str; 2] = ["UniqueId", "Callsign"];

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
}
