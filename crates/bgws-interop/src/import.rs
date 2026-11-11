//! From DIS to the sim: received PDUs become [`ExternalInput`]s.
//!
//! - Entity State: a remote entity's state. The platform standing in for it
//!   is looked up by its DIS type in the catalogue: the same kind, domain,
//!   category and subcategory first, then kind, domain and category, then a
//!   stand-in for the kind and domain. Every stand-in is logged once. The
//!   side comes from its force (the scenario's force per side). A
//!   deactivated entity (appearance State bit) is removed.
//! - Fire: one of theirs fired, at one of ours if the target is ours.
//! - Detonation: a round went off; ours to judge if it was at, or near, one
//!   of ours. The munition is looked up as platforms are.
//! - Start/Resume, Stop/Freeze: the exercise resumes or stops.
//!
//! Our own PDUs (our site and application) and other exercises are ignored.

use crate::geo::{forward_of, local_axes, unplace};
use crate::geoid::Geoid;
use bgws_core::{Enu, EntityId, Side};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::remote::{DetonationResult, ExternalInput, ExternalKey};
use bgws_sim::Sim;
use dis_rs::model::{EntityId as DisId, EntityType as DisType, Pdu, PduBody};
use std::collections::BTreeSet;

/// A request the sender asked us to acknowledge (Start/Resume, Stop/Freeze).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AckDue {
    pub to: DisId,
    pub from: DisId,
    /// SISO-REF-010 Acknowledge Flag: 3 Start/Resume, 4 Stop/Freeze.
    pub flag: u16,
    pub request_id: u32,
}

/// Turns PDUs into inputs.
pub struct Importer {
    pub spec: InteropSpec,
    /// (platform id, DIS type) for every catalogue platform with one; generic records first.
    platforms: Vec<(String, [u16; 7])>,
    munitions: Vec<(String, [u16; 7])>,
    /// Stand-ins already reported.
    reported: BTreeSet<String>,
    /// Requests to acknowledge.
    pub acks: Vec<AckDue>,
    /// Notes for the log (stand-ins chosen).
    pub notes: Vec<String>,
}

fn ty(t: &DisType) -> [u16; 7] {
    [u8::from(t.kind) as u16, u8::from(t.domain) as u16, u16::from(t.country), t.category as u16, t.subcategory as u16, t.specific as u16, t.extra as u16]
}

fn key(id: &DisId) -> ExternalKey {
    ExternalKey { site: id.simulation_address.site_id, application: id.simulation_address.application_id, entity: id.entity_id }
}

/// The stand-in for a kind and domain nothing closer matched.
fn stand_in(kind: u16, domain: u16) -> Option<&'static str> {
    Some(match (kind, domain) {
        (1, 1) => "generic/ifv",
        (1, 2) => "generic/fighter4",
        (1, 3) => "generic/frigate",
        (1, 4) => "generic/ssk",
        (1, 5) => "generic/satEo",
        (3, _) => "generic/infantrySection",
        (5, _) => "generic/depot",
        (9, _) => "generic/sensorNode",
        _ => return None,
    })
}

impl Importer {
    pub fn new(sim: &Sim, spec: InteropSpec) -> Importer {
        let mut platforms: Vec<(bool, String, [u16; 7])> = sim.ctx.catalogue.platforms().filter_map(|p| Some((p.parent.is_some(), p.id.clone(), p.dis?.with_country(0)))).collect();
        platforms.sort();
        let munitions = sim.ctx.catalogue.munitions().filter_map(|m| Some((m.id.clone(), m.dis?.with_country(0)))).collect();
        Importer { spec, platforms: platforms.into_iter().map(|(_, id, t)| (id, t)).collect(), munitions, reported: BTreeSet::new(), acks: Vec::new(), notes: Vec::new() }
    }

    fn lookup(list: &[(String, [u16; 7])], t: [u16; 7]) -> Option<(String, bool)> {
        let same = |a: &[u16; 7], n: usize| (0..n).all(|k| k == 2 || a[k] == t[k]);
        // Kind, domain, (country ignored), category, subcategory, specific.
        for n in [6usize, 5, 4] {
            if let Some((id, _)) = list.iter().find(|(_, a)| same(a, n)) {
                return Some((id.clone(), n < 5));
            }
        }
        None
    }

    /// The catalogue platform for a DIS entity type, and whether it is a stand-in.
    pub fn platform_for(&mut self, t: [u16; 7]) -> Option<String> {
        let (id, loose) = Self::lookup(&self.platforms, t).or_else(|| stand_in(t[0], t[1]).map(|s| (s.to_string(), true)))?;
        if loose && self.reported.insert(format!("p{t:?}")) {
            self.notes.push(format!("DIS type {t:?} has no close catalogue match: shown as {id}"));
        }
        Some(id)
    }

    /// The catalogue munition for a DIS munition type.
    pub fn munition_for(&mut self, t: [u16; 7]) -> String {
        if let Some((id, loose)) = Self::lookup(&self.munitions, t) {
            if loose && self.reported.insert(format!("m{t:?}")) {
                self.notes.push(format!("DIS munition {t:?} has no close catalogue match: judged as {id}"));
            }
            return id;
        }
        // By munition domain alone.
        let id = match t[1] {
            1 => "samShort",
            2 => "atgmTandem",
            8 => "ball762",
            _ => "he155",
        };
        if self.reported.insert(format!("m{t:?}")) {
            self.notes.push(format!("DIS munition {t:?} unknown: judged as {id}"));
        }
        id.to_string()
    }

    /// The side a DIS force is (by the scenario's force per side).
    pub fn side_for(&self, force: u8) -> Side {
        [Side::Blue, Side::Red, Side::Neutral].into_iter().find(|&s| self.spec.force_of(s) == force).unwrap_or(Side::Neutral)
    }

    fn ours(&self, id: &DisId) -> bool {
        id.simulation_address.site_id == self.spec.site && id.simulation_address.application_id == self.spec.application
    }

    /// Our entity a DIS entity ID names (ours are our index + 1).
    fn our_entity(&self, sim: &Sim, id: &DisId) -> Option<EntityId> {
        (self.ours(id) && id.entity_id > 0).then(|| EntityId(id.entity_id as u32 - 1)).filter(|e| e.index() < sim.ctx.order.len())
    }

    /// The inputs a PDU makes.
    pub fn from_pdu(&mut self, sim: &Sim, pdu: &Pdu) -> Vec<ExternalInput> {
        if pdu.header.exercise_id != self.spec.exercise {
            return vec![];
        }
        let geoid = Geoid::egm96();
        let frame = &sim.ctx.frame;
        match &pdu.body {
            PduBody::EntityState(es) => {
                if self.ours(&es.entity_id) {
                    return vec![];
                }
                let t = ty(&es.entity_type);
                let k = key(&es.entity_id);
                let appearance = u32::from(&es.entity_appearance);
                // Appearance State (bit 23): deactivated.
                if appearance & (1 << 23) != 0 {
                    return vec![ExternalInput::EntityRemoved { key: k }];
                }
                let Some(platform) = self.platform_for(t) else { return vec![] };
                let l = &es.entity_location;
                let ecef = [l.x_coordinate, l.y_coordinate, l.z_coordinate];
                let pos = unplace(frame, geoid, ecef);
                let g = bgws_core::Ecef { x: ecef[0], y: ecef[1], z: ecef[2] }.to_geodetic();
                let axes = local_axes(g.lat, g.lon);
                let to_enu = |v: [f64; 3]| Enu::new((0..3).map(|j| axes[0][j] * v[j]).sum(), (0..3).map(|j| axes[1][j] * v[j]).sum(), (0..3).map(|j| axes[2][j] * v[j]).sum());
                let v = &es.entity_linear_velocity;
                let vel = to_enu([v.first_vector_component as f64, v.second_vector_component as f64, v.third_vector_component as f64]);
                let o = &es.entity_orientation;
                let fwd = to_enu(forward_of([o.psi as f64, o.theta as f64, o.phi as f64]));
                let heading = fwd.e.atan2(fwd.n).rem_euclid(std::f64::consts::TAU);
                // Damage (bits 3-4), shared by platform and life-form appearances.
                let damage = if matches!(t[0], 1 | 3) { ((appearance >> 3) & 3) as u8 } else { 0 };
                let label = es.entity_marking.marking_string.trim_end_matches(['\0', ' ']).to_string();
                vec![ExternalInput::EntityState { key: k, platform, side: self.side_for(u8::from(es.force_id)), label, pos, vel, heading, damage }]
            }
            PduBody::Fire(f) => {
                if self.ours(&f.firing_entity_id) {
                    return vec![];
                }
                let munition = match &f.descriptor {
                    dis_rs::fire::model::FireDescriptor::Munition(m) => self.munition_for(ty(&m.entity_type)),
                    _ => return vec![],
                };
                vec![ExternalInput::Fired { firer: key(&f.firing_entity_id), target: self.our_entity(sim, &f.target_entity_id), munition }]
            }
            PduBody::Detonation(d) => {
                if self.ours(&d.source_entity_id) || self.ours(&d.event_id.simulation_address_as_id()) {
                    return vec![];
                }
                let (munition, rounds) = match &d.descriptor {
                    dis_rs::detonation::model::DetonationDescriptor::Munition(m) => (self.munition_for(ty(&m.entity_type)), m.quantity.max(1) as u32),
                    _ => return vec![],
                };
                let l = &d.location_in_world_coordinates;
                let at = unplace(frame, geoid, [l.x_coordinate, l.y_coordinate, l.z_coordinate]);
                let result = match u8::from(d.detonation_result) {
                    1 => DetonationResult::Impact,
                    2 => DetonationResult::Proximate,
                    _ => DetonationResult::Ground,
                };
                let ev = &d.event_id;
                vec![ExternalInput::Detonation {
                    firer: Some(key(&d.source_entity_id)),
                    target: self.our_entity(sim, &d.target_entity_id),
                    munition,
                    at,
                    result,
                    rounds,
                    event: (ev.simulation_address.site_id, ev.simulation_address.application_id, ev.event_id),
                }]
            }
            PduBody::StopFreeze(s) => {
                self.acks.push(AckDue { to: s.originating_id, from: s.receiving_id, flag: 4, request_id: s.request_id });
                vec![ExternalInput::Freeze { frozen: true }]
            }
            PduBody::StartResume(s) => {
                self.acks.push(AckDue { to: s.originating_id, from: s.receiving_id, flag: 3, request_id: s.request_id });
                vec![ExternalInput::Freeze { frozen: false }]
            }
            _ => vec![],
        }
    }
}

/// An event ID's simulation address as an entity ID, to compare with ours.
trait AsId {
    fn simulation_address_as_id(&self) -> DisId;
}

impl AsId for dis_rs::model::EventId {
    fn simulation_address_as_id(&self) -> DisId {
        DisId::new(self.simulation_address.site_id, self.simulation_address.application_id, 0)
    }
}
