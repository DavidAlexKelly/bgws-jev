//! DIS (IEEE 1278.1-2012, version 7): Entity State, Fire and Detonation PDUs
//! over UDP.
//!
//! The PDUs are built and serialised with `dis-rs` (MIT); the byte layout is
//! checked against the standard's own offsets in this module's tests.

use crate::dr::{Algorithm, DeadReckoner, DrSettings};
use crate::events::{DetonationOut, FireBook, FireOut, Outgoing};
use crate::geo::place;
use crate::geoid::Geoid;
use crate::model::{truths, EntityTruth, ExternalId};
use bgws_core::{EntityId, LogEvent};
use dis_rs::detonation::model::{Detonation, DetonationDescriptor};
use dis_rs::enumerations::{DetonationResult, MunitionDescriptorFuse, MunitionDescriptorWarhead};
use dis_rs::fire::model::{Fire, FireDescriptor};
use dis_rs::model::{EventId, MunitionDescriptor, SimulationAddress};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::Sim;
use bytes::BytesMut;
use dis_rs::BodyRaw;
use dis_rs::entity_state::model::{DrParameters, EntityAppearance, EntityMarking, EntityState};
use dis_rs::enumerations::{Country, DeadReckoningAlgorithm, EntityKind, ForceId, PduType, PlatformDomain};
use dis_rs::model::{EntityId as DisId, EntityType as DisType, Location, Orientation, Pdu, PduHeader, Timestamp, VectorF32};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// The DIS port by convention.
pub const DEFAULT_PORT: u16 = 3000;

/// A DIS relative timestamp for a sim time: the time within the hour, in
/// units of 3600 s / 2³¹, shifted left one bit with the low bit 0
/// (relative: our clock, not UTC).
pub fn relative_timestamp(sim_s: f64) -> u32 {
    let within = sim_s.rem_euclid(3600.0) / 3600.0;
    let units = ((within * f64::from(1u32 << 31)) as u64).min((1u64 << 31) - 1) as u32;
    units << 1
}

fn dis_type(t: &[u16; 7]) -> DisType {
    DisType {
        kind: EntityKind::from(t[0] as u8),
        domain: PlatformDomain::from(t[1] as u8),
        country: Country::from(t[2]),
        category: t[3] as u8,
        subcategory: t[4] as u8,
        specific: t[5] as u8,
        extra: t[6] as u8,
    }
}

/// A marking as DIS carries it: at most 11 ASCII characters. (`dis-rs`
/// 0.14 pads with `11 - len` and so must never be given more: a longer
/// string underflows, and in a release build loops until memory runs out.)
pub fn marking(label: &str) -> String {
    label.chars().filter(char::is_ascii).take(11).collect()
}

/// One entity's Entity State PDU.
pub fn entity_state(t: &EntityTruth, exercise: u8, sim_s: f64) -> Pdu {
    let ty = dis_type(&t.entity_type);
    let v = |a: [f64; 3]| VectorF32 { first_vector_component: a[0] as f32, second_vector_component: a[1] as f32, third_vector_component: a[2] as f32 };
    let algorithm = match Algorithm::for_truth(t) {
        Algorithm::Static => DeadReckoningAlgorithm::StaticNonmovingEntity,
        Algorithm::Fpw => DeadReckoningAlgorithm::DRM_FPW_ConstantVelocityLowAccelerationLinearMotionEntity,
    };
    let body = EntityState::builder()
        .with_entity_id(DisId::new(t.external.site, t.external.application, t.external.entity))
        .with_force_id(ForceId::from(t.force))
        .with_entity_type(ty)
        .with_alternative_entity_type(ty)
        .with_velocity(v(t.velocity))
        .with_location(Location { x_coordinate: t.ecef[0], y_coordinate: t.ecef[1], z_coordinate: t.ecef[2] })
        .with_orientation(Orientation { psi: t.orientation[0] as f32, theta: t.orientation[1] as f32, phi: t.orientation[2] as f32 })
        .with_appearance(EntityAppearance::from_bytes(t.appearance(), &ty))
        .with_dead_reckoning_parameters(DrParameters::default().with_algorithm(algorithm))
        .with_marking(EntityMarking::new_ascii(marking(&t.marking)))
        .build()
        .into_pdu_body();
    Pdu::finalize_from_parts(PduHeader::new_v7(exercise, PduType::EntityState), body, Timestamp::new(relative_timestamp(sim_s)))
}

/// The DIS identity of any entity in the game: ours from the scenario's
/// site and application, another simulation's from its key; none for "no
/// entity" (0:0:0).
pub fn dis_id_of(sim: &Sim, spec: &InteropSpec, id: Option<EntityId>) -> DisId {
    let Some(id) = id else { return DisId::new(0, 0, 0) };
    let Some(&e) = sim.ctx.order.get(id.index()) else { return DisId::new(0, 0, 0) };
    if let Some(r) = sim.world.get::<bgws_sim::remote::Remote>(e) {
        return DisId::new(r.key.site, r.key.application, r.key.entity);
    }
    ExternalId::ours(spec, id).map(|x| DisId::new(x.site, x.application, x.entity)).unwrap_or(DisId::new(0, 0, 0))
}

/// A munition's descriptor: its DIS type (the firer's side's country for a
/// generic round), warhead and fuse by kind, the rounds.
fn munition_descriptor(sim: &Sim, spec: &InteropSpec, firer: Option<EntityId>, munition: &str, rounds: u32) -> MunitionDescriptor {
    let m = sim.ctx.catalogue.munition(munition);
    let side = firer.and_then(|f| sim.ctx.order.get(f.index())).and_then(|&e| sim.world.get::<bgws_sim::components::Ident>(e)).map(|i| i.side);
    let country = side.map(|s| spec.country_of(s)).unwrap_or(0);
    let ty = m.and_then(|m| m.dis).map(|d| d.with_country(country)).unwrap_or([2, 0, country, 0, 0, 0, 0]);
    use bgws_catalogue::MunitionKind as K;
    // SISO-REF-010 Munition Descriptor-Warhead and -Fuse.
    let warhead: u16 = match m.map(|m| m.kind) {
        Some(K::Ke | K::SmallArms) => 5000,
        Some(K::Heat | K::Atgm | K::Fpv | K::Loitering) => 1600,
        _ => 1000,
    };
    MunitionDescriptor { entity_type: dis_type(&ty), warhead: MunitionDescriptorWarhead::from(warhead), fuse: MunitionDescriptorFuse::from(1100u16), quantity: rounds.min(u16::MAX as u32) as u16, rate: 0 }
}

fn ecef_of(sim: &Sim, pos: bgws_core::Enu) -> Location {
    let p = place(&sim.ctx.frame, Geoid::egm96(), pos);
    Location { x_coordinate: p.ecef[0], y_coordinate: p.ecef[1], z_coordinate: p.ecef[2] }
}

/// A Fire PDU.
pub fn fire_pdu(sim: &Sim, spec: &InteropSpec, f: &FireOut) -> Pdu {
    let body = Fire::builder()
        .with_firing_entity_id(dis_id_of(sim, spec, Some(f.firer)))
        .with_target_entity_id(dis_id_of(sim, spec, f.target))
        .with_entity_id(DisId::new(0, 0, 0))
        .with_event_id(EventId { simulation_address: SimulationAddress::new(spec.site, spec.application), event_id: f.event })
        .with_location_in_world(ecef_of(sim, f.from))
        .with_descriptor(FireDescriptor::Munition(munition_descriptor(sim, spec, Some(f.firer), &f.munition, f.rounds)))
        .with_range(f.range_m as f32)
        .build()
        .into_pdu_body();
    Pdu::finalize_from_parts(PduHeader::new_v7(spec.exercise, PduType::Fire), body, Timestamp::new(relative_timestamp(sim.ctx.now.secs())))
}

/// A Detonation PDU.
pub fn detonation_pdu(sim: &Sim, spec: &InteropSpec, d: &DetonationOut) -> Pdu {
    let body = Detonation::builder()
        .with_source_entity_id(dis_id_of(sim, spec, d.firer))
        .with_target_entity_id(dis_id_of(sim, spec, d.target))
        .with_exploding_entity_id(DisId::new(0, 0, 0))
        .with_event_id(EventId { simulation_address: SimulationAddress::new(spec.site, spec.application), event_id: d.event })
        .with_world_location(ecef_of(sim, d.at))
        .with_descriptor(DetonationDescriptor::Munition(munition_descriptor(sim, spec, d.firer, &d.munition, d.rounds)))
        .with_detonation_result(DetonationResult::from(d.result as u8))
        .build()
        .into_pdu_body();
    Pdu::finalize_from_parts(PduHeader::new_v7(spec.exercise, PduType::Detonation), body, Timestamp::new(relative_timestamp(sim.ctx.now.secs())))
}

/// A PDU on the wire.
pub fn encode(pdu: &Pdu) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(pdu.pdu_length() as usize);
    let _ = pdu.serialize(&mut buf);
    buf.to_vec()
}

/// Publishes a game's entities over DIS: each tick, the states dead
/// reckoning says are due.
pub struct DisPublisher {
    socket: UdpSocket,
    dest: SocketAddr,
    pub spec: InteropSpec,
    pub dr: DeadReckoner,
    pub fires: FireBook,
    /// PDUs sent so far.
    pub sent: u64,
}

impl DisPublisher {
    /// Send to `dest` (`host:port`; a broadcast address is allowed).
    pub fn new(dest: &str, spec: InteropSpec, settings: DrSettings) -> std::io::Result<DisPublisher> {
        let dest = dest.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no address"))?;
        let socket = UdpSocket::bind(if dest.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
        socket.set_broadcast(true)?;
        Ok(DisPublisher { socket, dest, spec, dr: DeadReckoner::new(settings), fires: FireBook::default(), sent: 0 })
    }

    /// Send raw bytes (an acknowledgement).
    pub fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.socket.send_to(bytes, self.dest)?;
        self.sent += 1;
        Ok(())
    }

    /// Send the states due now, and a Fire and Detonation for each of our
    /// shots in `events` (this tick's); returns how many PDUs were sent.
    pub fn publish(&mut self, sim: &Sim, events: &[LogEvent]) -> std::io::Result<usize> {
        let now = sim.ctx.now.secs();
        let mut n = 0;
        for o in self.fires.from_events(sim, events) {
            let pdu = match &o {
                Outgoing::Fire(f) => fire_pdu(sim, &self.spec, f),
                Outgoing::Detonation(d) => detonation_pdu(sim, &self.spec, d),
            };
            self.socket.send_to(&encode(&pdu), self.dest)?;
            n += 1;
        }
        for t in truths(sim, &self.spec) {
            if self.dr.due(now, &t).is_none() {
                continue;
            }
            let bytes = encode(&entity_state(&t, self.spec.exercise, now));
            self.socket.send_to(&bytes, self.dest)?;
            self.dr.sent(now, &t);
            n += 1;
        }
        self.sent += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Damage, ExternalId, Stance};
    use bgws_core::{EntityId, Side};

    fn tank() -> EntityTruth {
        EntityTruth {
            id: EntityId(41),
            external: ExternalId { site: 12, application: 34, entity: 42 },
            side: Side::Red,
            force: 2,
            entity_type: [1, 1, 222, 1, 0, 0, 0],
            marking: "Red 1 tanks plus".into(),
            ecef: [3_515_000.25, 1_183_000.5, 5_150_000.75],
            velocity: [1.5, -2.5, 0.25],
            orientation: [0.5, -0.25, 0.125],
            damage: Damage::Slight,
            mobility_killed: false,
            firepower_killed: true,
            engine_on: true,
            concealed: false,
            stance: None,
            moving: true,
        }
    }

    fn be_u16(b: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([b[at], b[at + 1]])
    }
    fn be_f64(b: &[u8], at: usize) -> f64 {
        f64::from_be_bytes(b[at..at + 8].try_into().unwrap())
    }
    fn be_f32(b: &[u8], at: usize) -> f32 {
        f32::from_be_bytes(b[at..at + 4].try_into().unwrap())
    }

    /// The Entity State PDU's layout in IEEE 1278.1-2012 (§7.2.2, with no
    /// variable parameters): 144 octets, fields at fixed offsets.
    #[test]
    fn the_entity_state_pdu_has_the_standard_layout() {
        let t = tank();
        let b = encode(&entity_state(&t, 9, 1800.0));
        assert_eq!(b.len(), 144);
        // Header: version 7, exercise, PDU type 1, family 1 (entity information), timestamp, length.
        assert_eq!(&b[0..4], &[7, 9, 1, 1]);
        assert_eq!(u32::from_be_bytes(b[4..8].try_into().unwrap()), relative_timestamp(1800.0));
        assert_eq!(be_u16(&b, 8), 144);
        // Entity ID, force, no variable parameters.
        assert_eq!((be_u16(&b, 12), be_u16(&b, 14), be_u16(&b, 16)), (12, 34, 42));
        assert_eq!((b[18], b[19]), (2, 0));
        // Entity type: kind, domain, country (16 bits), category, subcategory, specific, extra.
        assert_eq!((b[20], b[21], be_u16(&b, 22), b[24], b[25], b[26], b[27]), (1, 1, 222, 1, 0, 0, 0));
        // Velocity (3 × f32), location (3 × f64), orientation (3 × f32).
        assert_eq!((be_f32(&b, 36), be_f32(&b, 40), be_f32(&b, 44)), (1.5, -2.5, 0.25));
        assert_eq!((be_f64(&b, 48), be_f64(&b, 56), be_f64(&b, 64)), (3_515_000.25, 1_183_000.5, 5_150_000.75));
        assert_eq!((be_f32(&b, 72), be_f32(&b, 76), be_f32(&b, 80)), (0.5, -0.25, 0.125));
        // Appearance, then the dead-reckoning algorithm (2, FPW: it is moving).
        assert_eq!(u32::from_be_bytes(b[84..88].try_into().unwrap()), t.appearance());
        assert_eq!(b[88], 2);
        // Marking: ASCII (1), then the label cut to 11 characters.
        assert_eq!(b[128], 1);
        assert_eq!(&b[129..140], b"Red 1 tanks");
    }

    #[test]
    fn it_parses_back() {
        let mut t = tank();
        t.entity_type = [3, 1, 0, 10, 50, 0, 0];
        t.stance = Some(Stance::Prone);
        let b = encode(&entity_state(&t, 1, 10.0));
        let pdus = dis_rs::parse(&b).expect("parses");
        assert_eq!(pdus.len(), 1);
        let dis_rs::model::PduBody::EntityState(es) = &pdus[0].body else { panic!("not an entity state") };
        assert_eq!(es.entity_id, DisId::new(12, 34, 42));
        assert_eq!(u8::from(es.entity_type.kind), 3);
        assert_eq!(es.entity_type.subcategory, 50);
        assert_eq!(es.entity_marking.marking_string.trim_end_matches(['\0', ' ']), "Red 1 tanks");
    }

    #[test]
    fn long_and_non_ascii_labels_are_cut_to_eleven_ascii_characters() {
        assert_eq!(marking("Blue 10 mech dismounts"), "Blue 10 mec");
        assert_eq!(marking("Größe 1"), "Gre 1");
        let mut t = tank();
        t.marking = "a label far longer than eleven characters".into();
        assert_eq!(encode(&entity_state(&t, 1, 0.0)).len(), 144);
    }

    #[test]
    fn timestamps_count_within_the_hour() {
        assert_eq!(relative_timestamp(0.0), 0);
        assert_eq!(relative_timestamp(3600.0), 0);
        let half = relative_timestamp(1800.0);
        assert_eq!(half & 1, 0, "relative");
        assert_eq!(half >> 1, 1 << 30);
    }

    #[test]
    fn states_arrive_over_udp() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let dest = rx.local_addr().unwrap().to_string();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = encode(&entity_state(&tank(), 1, 0.0));
        tx.send_to(&b, &dest).unwrap();
        let mut buf = [0u8; 1500];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], &b[..]);
    }
}
