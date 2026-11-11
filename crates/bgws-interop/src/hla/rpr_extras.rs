//! The RPR FOM's objects for phase 4 (docs/design/INTEROP_PLAN.md §7): the
//! same radios, messages, emitters, aggregates and minefields as the DIS
//! side ([`crate::dis_extras`]), as RPR objects and an interaction.
//!
//! | ours | RPR |
//! |---|---|
//! | a radio | `EmbeddedSystem.RadioTransmitter` |
//! | a message | `RadioSignal.RawBinaryRadioSignal` (interaction) |
//! | a radar or jammer | `EmbeddedSystem.EmitterSystem` with one `EmitterBeam.RadarBeam` or `.JammerBeam` |
//! | an aggregated formation | `BaseEntity.AggregateEntity` |
//! | a minefield | `Minefield` |
//!
//! The class, attribute and parameter names and the datatypes follow the
//! RPR FOM 2.0 modules (Communication, Synthetic Environment, Aggregate,
//! Minefield) as we read them; like the entity classes, check them against
//! the partner's FOM on first use. Codes (emitter function, beam function,
//! transmit state) are the DIS ones, from dis-rs's SISO-REF-010.

use super::encoding::{DResult, Dec, DecodeError, Enc};
use super::rpr::{entity_identifier, entity_identifier_of, entity_type, octet, spatial, unicode, world_location, Spatial};
use crate::extras::{AggregateTruth, EmitterTruth, MessageTruth, MinefieldTruth, RadioTruth};
use dis_rs::enumerations::{ElectromagneticEmissionBeamFunction, EmitterSystemFunction};
use std::collections::BTreeMap;

pub const RADIO_TRANSMITTER: &str = "HLAobjectRoot.EmbeddedSystem.RadioTransmitter";
pub const EMITTER_SYSTEM: &str = "HLAobjectRoot.EmbeddedSystem.EmitterSystem";
pub const RADAR_BEAM: &str = "HLAobjectRoot.EmitterBeam.RadarBeam";
pub const JAMMER_BEAM: &str = "HLAobjectRoot.EmitterBeam.JammerBeam";
pub const AGGREGATE: &str = "HLAobjectRoot.BaseEntity.AggregateEntity";
pub const MINEFIELD: &str = "HLAobjectRoot.Minefield";
pub const RAW_BINARY_SIGNAL: &str = "HLAinteractionRoot.RadioSignal.RawBinaryRadioSignal";

pub const CLASSES: [&str; 6] = [RADIO_TRANSMITTER, EMITTER_SYSTEM, RADAR_BEAM, JAMMER_BEAM, AGGREGATE, MINEFIELD];

pub fn attributes_for(class: &str) -> Vec<&'static str> {
    match class {
        RADIO_TRANSMITTER => vec!["EntityIdentifier", "HostObjectIdentifier", "RadioIndex", "RadioSystemType", "TransmitterOperationalStatus", "Frequency", "FrequencyBandwidth", "TransmittedPower", "WorldLocation"],
        EMITTER_SYSTEM => vec!["EntityIdentifier", "HostObjectIdentifier", "EmitterIndex", "EmitterFunctionCode", "EmitterType"],
        RADAR_BEAM | JAMMER_BEAM => vec!["EmitterSystemIdentifier", "BeamIdentifier", "BeamFunctionCode", "EmissionFrequency", "FrequencyRange", "EffectiveRadiatedPower", "PulseRepetitionFrequency", "PulseWidth"],
        AGGREGATE => vec!["EntityIdentifier", "EntityType", "Spatial", "ForceIdentifier", "AggregateMarking", "AggregateState", "Dimensions", "EntityIdentifiers", "Formation"],
        MINEFIELD => vec!["MinefieldIdentifier", "ForceIdentifier", "MinefieldType", "MinefieldLocation", "MinefieldOrientation", "PerimeterPointCoordinates", "ActiveStatus", "Lane", "ProtocolMode"],
        _ => vec![],
    }
}

pub const SIGNAL_PARAMETERS: [&str; 7] = ["HostRadioIndex", "DataRate", "SignalDataLength", "SignalData", "TacticalDataLinkType", "TDLMessageCount", "UserProtocolID"];

/// Instance names: ours, under our entity's.
pub fn radio_name(host: &str, number: u16) -> String {
    format!("{host}-radio-{number}")
}
pub fn emitter_name(host: &str, number: u8) -> String {
    format!("{host}-emitter-{number}")
}
pub fn beam_name(host: &str, number: u8) -> String {
    format!("{host}-emitter-{number}-beam-1")
}
pub fn aggregate_name(site: u16, application: u16, number: u16) -> String {
    format!("BGWS-{site}-{application}-aggregate-{number}")
}
pub fn minefield_name(site: u16, application: u16, number: u16) -> String {
    format!("BGWS-{site}-{application}-minefield-{number}")
}

fn f32v(v: f32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn u16v(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn u32v(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn jammer_function() -> u8 {
    u8::from(EmitterSystemFunction::Jammer_42)
}
pub fn radar_function() -> u8 {
    u8::from(EmitterSystemFunction::Multifunction_1)
}
pub fn jamming_beam() -> u8 {
    u8::from(ElectromagneticEmissionBeamFunction::Jamming)
}
pub fn search_beam() -> u8 {
    u8::from(ElectromagneticEmissionBeamFunction::Search)
}

/// A radio's attributes. `host` is its entity's identifier and instance
/// name, `ecef` where it is, `country` its side's.
pub fn radio_attributes(r: &RadioTruth, host: ([u16; 3], &str), ecef: [f64; 3], country: u16) -> Vec<(&'static str, Vec<u8>)> {
    use bgws_ops::comms::BearerKind as B;
    let category = match r.bearer {
        B::Vhf | B::Uhf | B::Hf | B::SatcomGeo => 3,
        _ => 2,
    };
    vec![
        ("EntityIdentifier", entity_identifier(host.0)),
        ("HostObjectIdentifier", unicode(host.1)),
        ("RadioIndex", u16v(r.number)),
        // RadioTypeStruct: kind, domain, country, category, nomenclature version, nomenclature.
        ("RadioSystemType", Enc::new().octet(7).octet(1).u16(country).octet(category).octet(0).u16(0).done()),
        // TransmitterOperationalStatusEnum8: 0 off, 1 on (not transmitting), 2 transmitting.
        ("TransmitterOperationalStatus", octet(r.state as u8)),
        // UnsignedInteger64, Hz.
        ("Frequency", (r.freq_hz.round().max(0.0) as u64).to_be_bytes().to_vec()),
        ("FrequencyBandwidth", f32v(r.bw_hz as f32)),
        ("TransmittedPower", f32v(crate::dis_extras::dbm(r.power_w))),
        ("WorldLocation", world_location(ecef)),
    ]
}

/// A message as a RawBinaryRadioSignal: a short JSON record, as on DIS.
pub fn signal_parameters(m: &MessageTruth, host_radio: &str, from: u16, to: u16) -> Vec<(&'static str, Vec<u8>)> {
    let data = serde_json::json!({ "what": m.what, "from": from, "to": to, "delivered": m.delivered, "bearer": m.bearer, "sentS": m.sent.secs() }).to_string().into_bytes();
    vec![
        ("HostRadioIndex", unicode(host_radio)),
        ("DataRate", u32v(0)),
        ("SignalDataLength", u16v((data.len() * 8).min(u16::MAX as usize) as u16)),
        ("SignalData", Enc::new().u32(data.len() as u32).bytes(&data).done()),
        ("TacticalDataLinkType", u16v(0)),
        ("TDLMessageCount", u16v(1)),
        ("UserProtocolID", u32v(0)),
    ]
}

pub fn emitter_system_attributes(e: &EmitterTruth, host: ([u16; 3], &str)) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("EntityIdentifier", entity_identifier(host.0)),
        ("HostObjectIdentifier", unicode(host.1)),
        ("EmitterIndex", octet(e.number)),
        ("EmitterFunctionCode", octet(if e.jammer { jammer_function() } else { radar_function() })),
        ("EmitterType", u16v(0)),
    ]
}

pub fn beam_attributes(e: &EmitterTruth, system: &str) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("EmitterSystemIdentifier", unicode(system)),
        ("BeamIdentifier", octet(1)),
        ("BeamFunctionCode", octet(if e.jammer { jamming_beam() } else { search_beam() })),
        ("EmissionFrequency", f32v(e.freq_hz as f32)),
        ("FrequencyRange", f32v(e.bw_hz as f32)),
        ("EffectiveRadiatedPower", f32v(crate::dis_extras::dbm(e.erp_w))),
        ("PulseRepetitionFrequency", f32v(0.0)),
        ("PulseWidth", f32v(0.0)),
    ]
}

/// An aggregate's attributes; `members` are its entities' instance names.
#[allow(clippy::too_many_arguments)]
pub fn aggregate_attributes(a: &AggregateTruth, id: [u16; 3], force: u8, country: u16, ecef: [f64; 3], orientation: [f32; 3], velocity: [f32; 3], members: &[String], aggregated: bool) -> Vec<(&'static str, Vec<u8>)> {
    let marking: Vec<u8> = a.name.bytes().filter(u8::is_ascii).take(31).chain(std::iter::repeat(0)).take(31).collect();
    let mut ids = Enc::new();
    ids.u32(members.len() as u32);
    for m in members {
        ids.unicode(m);
    }
    let moving = velocity.iter().any(|v| v.abs() > 0.2);
    vec![
        ("EntityIdentifier", entity_identifier(id)),
        // Aggregate type: kind 11 (aggregate in RPR's EntityKind), land, the side's country.
        ("EntityType", entity_type([11, 1, country, 0, 0, 0, 0])),
        ("Spatial", spatial(&Spatial { algorithm: if moving { 2 } else { 1 }, location: ecef, frozen: false, orientation, velocity })),
        ("ForceIdentifier", octet(force)),
        ("AggregateMarking", Enc::new().octet(1).bytes(&marking).done()),
        // AggregateStateEnum8: 1 aggregated, 2 disaggregated.
        ("AggregateState", octet(if aggregated { 1 } else { 2 })),
        ("Dimensions", Enc::new().f32((2.0 * a.radius_m) as f32).f32((2.0 * a.radius_m) as f32).f32(0.0).done()),
        ("EntityIdentifiers", ids.done()),
        ("Formation", Enc::new().i32(0).done()),
    ]
}

/// A minefield's attributes: its four corners, in its own frame (x along
/// its depth, y across its front).
pub fn minefield_attributes(m: &MinefieldTruth, id: [u16; 3], force: u8, ecef: [f64; 3], orientation: [f32; 3]) -> Vec<(&'static str, Vec<u8>)> {
    let (hw, hd) = ((m.width_m / 2.0) as f32, (m.depth_m / 2.0) as f32);
    let mut points = Enc::new();
    points.u32(4);
    for (x, y) in [(-hd, -hw), (hd, -hw), (hd, hw), (-hd, hw)] {
        points.f32(x).f32(y);
    }
    vec![
        ("MinefieldIdentifier", entity_identifier(id)),
        ("ForceIdentifier", octet(force)),
        ("MinefieldType", entity_type([0; 7])),
        ("MinefieldLocation", world_location(ecef)),
        ("MinefieldOrientation", Enc::new().f32(orientation[0]).f32(orientation[1]).f32(orientation[2]).done()),
        ("PerimeterPointCoordinates", points.done()),
        ("ActiveStatus", octet(1)),
        ("Lane", octet((m.lanes > 0) as u8)),
        ("ProtocolMode", octet(0)),
    ]
}

// ---- reading another federate's ----

fn be<const N: usize>(b: &[u8]) -> DResult<[u8; N]> {
    b.get(..N).and_then(|s| s.try_into().ok()).ok_or_else(|| DecodeError(format!("{} octets, wanted {N}", b.len())))
}

/// A remote radio as read so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadioIn {
    pub host: Option<[u16; 3]>,
    pub index: Option<u16>,
    pub status: Option<u8>,
    pub freq_hz: Option<f64>,
    pub bw_hz: Option<f64>,
    pub power_dbm: Option<f32>,
}

impl RadioIn {
    pub fn merge(&mut self, v: &BTreeMap<String, Vec<u8>>) -> DResult<()> {
        for (k, b) in v {
            match k.as_str() {
                "EntityIdentifier" => self.host = Some(entity_identifier_of(b)?),
                "RadioIndex" => self.index = Some(u16::from_be_bytes(be(b)?)),
                "TransmitterOperationalStatus" => self.status = b.first().copied(),
                "Frequency" => self.freq_hz = Some(u64::from_be_bytes(be(b)?) as f64),
                "FrequencyBandwidth" => self.bw_hz = Some(f32::from_be_bytes(be(b)?) as f64),
                "TransmittedPower" => self.power_dbm = Some(f32::from_be_bytes(be(b)?)),
                _ => {}
            }
        }
        Ok(())
    }
}

/// A remote emitter system as read so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemIn {
    pub host: Option<[u16; 3]>,
    pub index: Option<u8>,
    pub function: Option<u8>,
}

impl SystemIn {
    pub fn merge(&mut self, v: &BTreeMap<String, Vec<u8>>) -> DResult<()> {
        for (k, b) in v {
            match k.as_str() {
                "EntityIdentifier" => self.host = Some(entity_identifier_of(b)?),
                "EmitterIndex" => self.index = b.first().copied(),
                "EmitterFunctionCode" => self.function = b.first().copied(),
                _ => {}
            }
        }
        Ok(())
    }
}

/// A remote beam as read so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeamIn {
    pub system: Option<String>,
    pub id: Option<u8>,
    pub function: Option<u8>,
    pub freq_hz: Option<f64>,
    pub range_hz: Option<f64>,
    pub erp_dbm: Option<f32>,
}

impl BeamIn {
    pub fn merge(&mut self, v: &BTreeMap<String, Vec<u8>>) -> DResult<()> {
        for (k, b) in v {
            match k.as_str() {
                "EmitterSystemIdentifier" => self.system = Some(Dec::new(b).unicode()?),
                "BeamIdentifier" => self.id = b.first().copied(),
                "BeamFunctionCode" => self.function = b.first().copied(),
                "EmissionFrequency" => self.freq_hz = Some(f32::from_be_bytes(be(b)?) as f64),
                "FrequencyRange" => self.range_hz = Some(f32::from_be_bytes(be(b)?) as f64),
                "EffectiveRadiatedPower" => self.erp_dbm = Some(f32::from_be_bytes(be(b)?)),
                _ => {}
            }
        }
        Ok(())
    }
}
