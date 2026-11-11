//! DIS for phase 4 (docs/design/INTEROP_PLAN.md §7): Transmitter, Signal,
//! Electromagnetic Emission, Aggregate State and Minefield State PDUs from
//! [`crate::extras`], and Transmitter and Electromagnetic Emission PDUs in.
//!
//! - **Transmitter**: each radio that radiates, off / on / transmitting, its
//!   frequency, bandwidth and power; on a change and every 5 s.
//! - **Signal**: each message our comms model finished, as raw binary data:
//!   a short JSON record (what, from, to, delivered, bearer), not voice.
//! - **Electromagnetic Emission**: each entity's radars and jammers on the
//!   air (one system and beam each: frequency, bandwidth, ERP); on a change
//!   and every 10 s, and once with no systems when they go quiet.
//! - **Aggregate State**: each formation the level of detail aggregates (its
//!   centre, extent, velocity, members); on a change and every 10 s, and
//!   once as disaggregated when it splits.
//! - **Minefield State**: each land minefield (a rectangle: its four corners
//!   as perimeter points), with the lane bit once breached; on a change and
//!   every 10 s. dis-rs does not build this PDU, so it is written here from
//!   IEEE 1278.1-2012's field list; check it against a partner's tool on
//!   first use.

use crate::dis::dis_id_of;
use crate::extras::{self, AggregateTruth, EmitterTruth, MessageTruth, MinefieldTruth, RadioState, RadioTruth};
use crate::geo::{dis_orientation, place};
use crate::geoid::Geoid;
use bgws_core::{Enu, SimTime};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::remote::{EmitterKind, ExternalInput, ExternalKey, RemoteEmitter};
use bgws_sim::Sim;
use dis_rs::enumerations::*;
use dis_rs::model::{EntityId as DisId, EntityType, EventId, Location, Orientation, Pdu, PduBody, PduHeader, SimulationAddress, Timestamp, VectorF32};
use dis_rs::BodyRaw;
use std::collections::{BTreeMap, BTreeSet};

/// Aggregates are numbered from here, clear of entity numbers.
pub const AGGREGATE_BASE: u16 = 0x7000;

pub fn dbm(w: f64) -> f32 {
    (10.0 * (w.max(1e-9) * 1000.0).log10()) as f32
}

pub fn watts(dbm: f32) -> f64 {
    10f64.powf(dbm as f64 / 10.0) / 1000.0
}

fn location(sim: &Sim, pos: Enu) -> Location {
    let p = place(&sim.ctx.frame, Geoid::egm96(), pos);
    Location { x_coordinate: p.ecef[0], y_coordinate: p.ecef[1], z_coordinate: p.ecef[2] }
}

fn pos_of(sim: &Sim, id: bgws_core::EntityId) -> Enu {
    sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<bgws_sim::components::Kin>(e)).map(|k| k.pos).unwrap_or_default()
}

fn finish(spec: &InteropSpec, ty: PduType, body: PduBody, sim_s: f64) -> Pdu {
    Pdu::finalize_from_parts(PduHeader::new_v7(spec.exercise, ty), body, Timestamp::new(crate::dis::relative_timestamp(sim_s)))
}

fn entity_type(t: [u16; 7]) -> EntityType {
    EntityType { kind: EntityKind::from(t[0] as u8), domain: PlatformDomain::from(t[1] as u8), country: Country::from(t[2]), category: t[3] as u8, subcategory: t[4] as u8, specific: t[5] as u8, extra: t[6] as u8 }
}

fn side_of(sim: &Sim, id: bgws_core::EntityId) -> Option<bgws_core::Side> {
    sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<bgws_sim::components::Ident>(e)).map(|i| i.side)
}

/// A Transmitter PDU.
pub fn transmitter_pdu(sim: &Sim, spec: &InteropSpec, r: &RadioTruth) -> Pdu {
    use bgws_ops::comms::BearerKind as B;
    let country = side_of(sim, r.id).map(|s| spec.country_of(s)).unwrap_or(0);
    // SISO-REF-010 Radio Category: 1 voice, 2 data link, 3 voice and data.
    let category = match r.bearer {
        B::Vhf | B::Uhf | B::Hf | B::SatcomGeo => 3,
        _ => 2,
    };
    let state = match r.state {
        RadioState::Off => TransmitterTransmitState::Off,
        RadioState::On => TransmitterTransmitState::OnButNotTransmitting,
        RadioState::Transmitting => TransmitterTransmitState::OnAndTransmitting,
    };
    let body = dis_rs::transmitter::model::Transmitter::builder()
        .with_radio_reference_id(dis_id_of(sim, spec, Some(r.id)))
        .with_radio_number(r.number)
        .with_radio_type(entity_type([7, 1, country, category, 0, 0, 0]))
        .with_transmit_state(state)
        .with_antenna_location(location(sim, pos_of(sim, r.id)))
        .with_frequency(r.freq_hz.round().max(0.0) as u64)
        .with_transmit_frequency_bandwidth(r.bw_hz as f32)
        .with_power(dbm(r.power_w))
        .build()
        .into_pdu_body();
    finish(spec, PduType::Transmitter, body, sim.ctx.now.secs())
}

/// A Signal PDU carrying a message record as raw binary data.
pub fn signal_pdu(sim: &Sim, spec: &InteropSpec, m: &MessageTruth) -> Pdu {
    let data =
        serde_json::json!({ "what": m.what, "from": dis_id_of(sim, spec, Some(m.from)).entity_id, "to": dis_id_of(sim, spec, Some(m.to)).entity_id, "delivered": m.delivered, "bearer": m.bearer, "sentS": m.sent.secs() }).to_string().into_bytes();
    let body = dis_rs::signal::model::Signal::builder()
        .with_radio_reference_id(dis_id_of(sim, spec, Some(m.from)))
        .with_radio_number(1)
        .with_encoding_scheme(dis_rs::signal::model::EncodingScheme::RawBinaryData { encoding_class: SignalEncodingClass::RawBinaryData, nr_of_messages: 1 })
        .with_tdl_type(SignalTdlType::Other_0)
        .with_samples(1)
        .with_data(data)
        .build()
        .into_pdu_body();
    finish(spec, PduType::Signal, body, sim.ctx.now.secs())
}

/// An Electromagnetic Emission PDU for one entity's emitters (none: they went quiet).
pub fn emission_pdu(sim: &Sim, spec: &InteropSpec, id: bgws_core::EntityId, emitters: &[EmitterTruth], event: u16, changed: bool) -> Pdu {
    use dis_rs::electromagnetic_emission::model::{Beam, EmitterSystem, FundamentalParameterData};
    let mut b = dis_rs::electromagnetic_emission::model::ElectromagneticEmission::builder()
        .with_emitting_entity_id(dis_id_of(sim, spec, Some(id)))
        .with_event_id(EventId { simulation_address: SimulationAddress::new(spec.site, spec.application), event_id: event })
        .with_state_update_indicator(if changed { ElectromagneticEmissionStateUpdateIndicator::ChangedDataUpdate } else { ElectromagneticEmissionStateUpdateIndicator::HeartbeatUpdate });
    for e in emitters {
        let beam = Beam::new()
            .with_number(1)
            .with_parameter_data(FundamentalParameterData::new().with_frequency(e.freq_hz as f32).with_frequency_range(e.bw_hz as f32).with_effective_power(dbm(e.erp_w)))
            .with_beam_function(if e.jammer { ElectromagneticEmissionBeamFunction::Jamming } else { ElectromagneticEmissionBeamFunction::Search })
            .with_beam_status(BeamStatusBeamState::Active);
        let system = EmitterSystem::new().with_name(EmitterName::from(0u16)).with_function(if e.jammer { EmitterSystemFunction::Jammer_42 } else { EmitterSystemFunction::Multifunction_1 }).with_number(e.number).with_beam(beam);
        b = b.with_emitter_system(system);
    }
    finish(spec, PduType::ElectromagneticEmission, b.build().into_pdu_body(), sim.ctx.now.secs())
}

/// An Aggregate State PDU.
pub fn aggregate_pdu(sim: &Sim, spec: &InteropSpec, a: &AggregateTruth, aggregated: bool) -> Pdu {
    use dis_rs::aggregate_state::model::{AggregateMarking, AggregateType};
    let placed = place(&sim.ctx.frame, Geoid::egm96(), a.centre);
    let heading = a.velocity.e.atan2(a.velocity.n);
    let o = dis_orientation(&placed.axes, if a.velocity.horizontal() > 0.2 { heading } else { 0.0 }, 0.0, 0.0);
    let v = crate::geo::to_ecef_vec(&placed.axes, a.velocity);
    let marking: String = a.name.chars().filter(char::is_ascii).take(31).collect();
    let body = dis_rs::aggregate_state::model::AggregateState::builder()
        .with_aggregate_id(DisId::new(spec.site, spec.application, AGGREGATE_BASE + a.number))
        .with_force_id(ForceId::from(spec.force_of(a.side)))
        .with_aggregate_state(if aggregated { AggregateStateAggregateState::Aggregated } else { AggregateStateAggregateState::Disaggregated })
        .with_aggregate_type(AggregateType { aggregate_kind: AggregateStateAggregateKind::MilitaryHierarchy, domain: PlatformDomain::Land, country: Country::from(spec.country_of(a.side)), ..Default::default() })
        .with_formation(AggregateStateFormation::Other)
        .with_aggregate_marking(AggregateMarking { marking_character_set: EntityMarkingCharacterSet::ASCII, marking_string: marking })
        .with_dimensions(VectorF32::new((2.0 * a.radius_m) as f32, (2.0 * a.radius_m) as f32, 0.0))
        .with_orientation(Orientation::new(o[0] as f32, o[1] as f32, o[2] as f32))
        .with_center_of_mass(Location { x_coordinate: placed.ecef[0], y_coordinate: placed.ecef[1], z_coordinate: placed.ecef[2] })
        .with_velocity(VectorF32::new(v[0] as f32, v[1] as f32, v[2] as f32))
        .with_entities(a.members.iter().map(|&m| dis_id_of(sim, spec, Some(m))).collect())
        .build()
        .into_pdu_body();
    finish(spec, PduType::AggregateState, body, sim.ctx.now.secs())
}

/// A Minefield State PDU (type 37, family 8), written by hand:
/// header (12), minefield ID (6), sequence number (2), force ID (1), number
/// of perimeter points (1), minefield type (8), number of mine types (2),
/// location (24), orientation (12), appearance (2), protocol mode (2), then
/// each perimeter point (x, y as 32-bit floats in the minefield's frame)
/// and each mine type (8).
pub fn minefield_pdu(sim: &Sim, spec: &InteropSpec, m: &MinefieldTruth, sequence: u16) -> Vec<u8> {
    let placed = place(&sim.ctx.frame, Geoid::egm96(), Enu::new(m.centre.e, m.centre.n, sim.ctx.ground.height(m.centre.e, m.centre.n)));
    let o = dis_orientation(&placed.axes, m.bearing, 0.0, 0.0);
    let (hw, hd) = ((m.width_m / 2.0) as f32, (m.depth_m / 2.0) as f32);
    // x along its depth (its bearing), y across its front.
    let corners = [(-hd, -hw), (hd, -hw), (hd, hw), (-hd, hw)];
    // SISO-REF-010 Minefield State Appearance: type (0-1) mixed, active (2)
    // 0, lane (3), state (13) active.
    let appearance: u16 = if m.lanes > 0 { 1 << 3 } else { 0 };
    let mut b: Vec<u8> = Vec::with_capacity(72 + 8 * corners.len());
    b.extend_from_slice(&[7, spec.exercise, 37, 8]);
    b.extend_from_slice(&crate::dis::relative_timestamp(sim.ctx.now.secs()).to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]); // length, status, padding (length below)
    for v in [spec.site, spec.application, m.number, sequence] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.push(spec.force_of(m.owner));
    b.push(corners.len() as u8);
    b.extend_from_slice(&[0; 8]); // minefield type: Other
    b.extend_from_slice(&0u16.to_be_bytes()); // mine types listed: none
    for c in placed.ecef {
        b.extend_from_slice(&c.to_be_bytes());
    }
    for a in o {
        b.extend_from_slice(&(a as f32).to_be_bytes());
    }
    b.extend_from_slice(&appearance.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes()); // protocol mode: heartbeat
    for (x, y) in corners {
        b.extend_from_slice(&x.to_be_bytes());
        b.extend_from_slice(&y.to_be_bytes());
    }
    let len = b.len() as u16;
    b[8..10].copy_from_slice(&len.to_be_bytes());
    b
}

/// What was last sent of each item, for heartbeats and changes.
#[derive(Default)]
pub struct ExtrasBook {
    last: BTreeMap<String, (f64, String)>,
    emitting: BTreeSet<bgws_core::EntityId>,
    aggregated: BTreeSet<u16>,
    minefield_seq: BTreeMap<u16, (usize, u16)>,
    messages_after: SimTime,
    event: u16,
}

impl ExtrasBook {
    fn due(&mut self, key: String, now: f64, sig: String, every: f64) -> bool {
        match self.last.get(&key) {
            Some((t, s)) if *s == sig && now - t < every => false,
            _ => {
                self.last.insert(key, (now, sig));
                true
            }
        }
    }

    fn event(&mut self) -> u16 {
        self.event = if self.event == u16::MAX { 1 } else { self.event + 1 };
        self.event
    }

    /// This second's phase 4 PDUs, encoded.
    pub fn pdus(&mut self, sim: &Sim, spec: &InteropSpec) -> Vec<Vec<u8>> {
        let now = sim.ctx.now.secs();
        let mut out = vec![];
        for r in extras::radios(sim, spec) {
            if self.due(format!("t{}:{}", r.id.0, r.number), now, format!("{:?}{}", r.state, r.freq_hz), 5.0) {
                out.push(crate::dis::encode(&transmitter_pdu(sim, spec, &r)));
            }
        }
        for m in extras::messages(sim, spec, self.messages_after) {
            self.messages_after = self.messages_after.max(m.done);
            out.push(crate::dis::encode(&signal_pdu(sim, spec, &m)));
        }
        let mut by_entity: BTreeMap<bgws_core::EntityId, Vec<EmitterTruth>> = BTreeMap::new();
        for e in extras::emitters(sim, spec) {
            by_entity.entry(e.id).or_default().push(e);
        }
        let quiet: Vec<_> = self.emitting.iter().filter(|id| !by_entity.contains_key(id)).copied().collect();
        for id in quiet {
            self.emitting.remove(&id);
            self.last.remove(&format!("e{}", id.0));
            let ev = self.event();
            out.push(crate::dis::encode(&emission_pdu(sim, spec, id, &[], ev, true)));
        }
        for (id, list) in by_entity {
            let sig = list.iter().map(|e| format!("{}:{}:{:.0}:{:.0}:{:.1}", e.number, e.jammer, e.freq_hz, e.bw_hz, e.erp_w)).collect::<Vec<_>>().join(";");
            let fresh = self.emitting.insert(id);
            let changed = fresh || self.last.get(&format!("e{}", id.0)).is_some_and(|(_, s)| *s != sig);
            if self.due(format!("e{}", id.0), now, sig, 10.0) {
                let ev = self.event();
                out.push(crate::dis::encode(&emission_pdu(sim, spec, id, &list, ev, changed)));
            }
        }
        for a in extras::aggregates(sim, spec) {
            if a.aggregated {
                self.aggregated.insert(a.number);
                let sig = format!("{:.0}:{:.0}:{}:{:.0}", a.centre.e, a.centre.n, a.members.len(), a.radius_m);
                if self.due(format!("a{}", a.number), now, sig, 10.0) {
                    out.push(crate::dis::encode(&aggregate_pdu(sim, spec, &a, true)));
                }
            } else if self.aggregated.remove(&a.number) {
                self.last.remove(&format!("a{}", a.number));
                out.push(crate::dis::encode(&aggregate_pdu(sim, spec, &a, false)));
            }
        }
        for m in extras::minefields(sim) {
            let seq = self.minefield_seq.entry(m.number).or_insert((m.lanes, 1));
            if seq.0 != m.lanes {
                *seq = (m.lanes, seq.1.wrapping_add(1));
            }
            let s = seq.1;
            if self.due(format!("m{}", m.number), now, format!("{}", m.lanes), 10.0) {
                out.push(minefield_pdu(sim, spec, &m, s));
            }
        }
        out
    }
}

/// Inbound: a Transmitter PDU's radio, as an emitter (on the air only while
/// transmitting).
pub fn transmitter_input(t: &dis_rs::transmitter::model::Transmitter, key: ExternalKey) -> ExternalInput {
    let emitter = (t.transmit_state == TransmitterTransmitState::OnAndTransmitting).then(|| RemoteEmitter { what: EmitterKind::Radio, freq_hz: t.frequency as f64, bw_hz: t.transmit_frequency_bandwidth as f64, erp_w: watts(t.power), duty: 1.0 });
    ExternalInput::Emitter { key, slot: (0, t.radio_number), emitter }
}

/// Inbound: an Electromagnetic Emission PDU's beams, as emitters; `before`
/// holds the slots the entity's last one had, so those now gone go off.
pub fn emission_inputs(ee: &dis_rs::electromagnetic_emission::model::ElectromagneticEmission, key: ExternalKey, before: &mut BTreeSet<u16>) -> Vec<ExternalInput> {
    let mut now = BTreeSet::new();
    let mut out = vec![];
    for s in &ee.emitter_systems {
        for b in &s.beams {
            let slot = ((s.number as u16) << 8) | b.number as u16;
            if b.beam_status == BeamStatusBeamState::Deactivated {
                continue;
            }
            now.insert(slot);
            let jam = s.function == EmitterSystemFunction::Jammer_42 || b.beam_function == ElectromagneticEmissionBeamFunction::Jamming;
            let p = &b.parameter_data;
            out.push(ExternalInput::Emitter {
                key,
                slot: (1, slot),
                emitter: Some(RemoteEmitter { what: if jam { EmitterKind::Jammer } else { EmitterKind::Radar }, freq_hz: p.frequency as f64, bw_hz: p.frequency_range as f64, erp_w: watts(p.effective_power), duty: 1.0 }),
            });
        }
    }
    for gone in before.difference(&now) {
        out.push(ExternalInput::Emitter { key, slot: (1, *gone), emitter: None });
    }
    *before = now;
    out
}
