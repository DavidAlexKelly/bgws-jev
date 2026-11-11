//! The RPR FOM's objects and interactions as BGWS uses them, encoded with the
//! HLA data encoding ([`super::encoding`]). The class, attribute and
//! parameter names and the datatypes are those of the RPR FOM 2.0 and 3.0
//! (SISO-STD-001.1): the FOM files themselves are the federation's, passed
//! at join time, not shipped here.
//!
//! - Every entity is a `BaseEntity.PhysicalEntity` subclass chosen by its
//!   DIS kind and domain, with the same truth as its DIS Entity State:
//!   identifier, type, spatial (static or DRM F,P,W), force, marking,
//!   damage, and the appearance flags the RPR FOM has attributes for.
//! - `WeaponFire` and `MunitionDetonation` carry what the DIS Fire and
//!   Detonation PDUs do; entities are named by their object instance names
//!   (RTIobjectId).

use super::encoding::{DResult, Dec, DecodeError, Enc};
use crate::model::{Damage, EntityTruth};
use std::collections::BTreeMap;

pub const BASE: &str = "HLAobjectRoot.BaseEntity.PhysicalEntity";
pub const WEAPON_FIRE: &str = "HLAinteractionRoot.WeaponFire";
pub const MUNITION_DETONATION: &str = "HLAinteractionRoot.MunitionDetonation";

/// The object class for a DIS kind and domain.
pub fn class_for(kind: u16, domain: u16) -> &'static str {
    match (kind, domain) {
        (1, 1) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.GroundVehicle",
        (1, 2) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.Aircraft",
        (1, 3) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.SurfaceVessel",
        (1, 4) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.SubmersibleVessel",
        (1, 5) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.Spacecraft",
        (3, _) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Lifeform.Human",
        (5, _) => "HLAobjectRoot.BaseEntity.PhysicalEntity.CulturalFeature",
        (9, _) => "HLAobjectRoot.BaseEntity.PhysicalEntity.Sensor",
        _ => "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform",
    }
}

/// Every class BGWS publishes and subscribes to.
pub const CLASSES: [&str; 9] = [
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.GroundVehicle",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.Aircraft",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.SurfaceVessel",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.SubmersibleVessel",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform.Spacecraft",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Lifeform.Human",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.CulturalFeature",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Sensor",
    "HLAobjectRoot.BaseEntity.PhysicalEntity.Platform",
];

/// The attributes of a class that BGWS uses (BaseEntity and PhysicalEntity,
/// plus Platform's or Lifeform's).
pub fn attributes_for(class: &str) -> Vec<&'static str> {
    let mut a = vec!["EntityIdentifier", "EntityType", "Spatial", "ForceIdentifier", "Marking", "DamageState", "IsConcealed", "FlamesPresent", "SmokePlumePresent"];
    if class.contains(".Platform") {
        a.extend(["PowerPlantOn", "FirePowerDisabled", "Immobilized"]);
    }
    if class.contains(".Lifeform") {
        a.push("StanceCode");
    }
    a
}

pub const WEAPON_FIRE_PARAMETERS: [&str; 10] = ["EventIdentifier", "FireControlSolutionRange", "FiringLocation", "FiringObjectIdentifier", "FuseType", "MunitionType", "QuantityFired", "RateOfFire", "TargetObjectIdentifier", "WarheadType"];
pub const MUNITION_DETONATION_PARAMETERS: [&str; 10] = ["DetonationLocation", "DetonationResultCode", "EventIdentifier", "FiringObjectIdentifier", "FuseType", "MunitionType", "QuantityFired", "RateOfFire", "TargetObjectIdentifier", "WarheadType"];

/// Our object instance name for an entity (its RTIobjectId).
pub fn instance_name(site: u16, application: u16, entity: u16) -> String {
    format!("BGWS-{site}-{application}-{entity}")
}

// ---- datatypes ----

/// EntityIdentifierStruct: site, application, entity number.
pub fn entity_identifier(id: [u16; 3]) -> Vec<u8> {
    Enc::new().u16(id[0]).u16(id[1]).u16(id[2]).done()
}
pub fn entity_identifier_of(b: &[u8]) -> DResult<[u16; 3]> {
    let mut d = Dec::new(b);
    Ok([d.u16()?, d.u16()?, d.u16()?])
}

/// EntityTypeStruct: kind, domain (octets), country (16 bits), category,
/// subcategory, specific, extra (octets).
pub fn entity_type(t: [u16; 7]) -> Vec<u8> {
    Enc::new().octet(t[0] as u8).octet(t[1] as u8).u16(t[2]).octet(t[3] as u8).octet(t[4] as u8).octet(t[5] as u8).octet(t[6] as u8).done()
}
pub fn entity_type_of(b: &[u8]) -> DResult<[u16; 7]> {
    let mut d = Dec::new(b);
    entity_type_in(&mut d)
}
fn entity_type_in(d: &mut Dec) -> DResult<[u16; 7]> {
    let (k, dm, c) = (d.octet()? as u16, d.octet()? as u16, d.u16()?);
    Ok([k, dm, c, d.octet()? as u16, d.octet()? as u16, d.octet()? as u16, d.octet()? as u16])
}

/// SpatialStruct, the variant record by dead-reckoning algorithm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    /// DeadReckoningAlgorithmEnum8 (1 Static, 2 DRM F,P,W, …).
    pub algorithm: u8,
    /// WorldLocationStruct: ECEF metres.
    pub location: [f64; 3],
    pub frozen: bool,
    /// OrientationStruct: ψ, θ, φ radians.
    pub orientation: [f32; 3],
    /// VelocityVectorStruct, m/s (none for Static).
    pub velocity: [f32; 3],
}

pub fn spatial(s: &Spatial) -> Vec<u8> {
    let mut e = Enc::new();
    e.octet(s.algorithm);
    // The alternative's alignment is its widest field's: the doubles.
    e.align(8).f64x3(s.location).octet(s.frozen as u8).f32x3(s.orientation);
    if s.algorithm != 1 {
        e.f32x3(s.velocity);
    }
    e.done()
}

/// Location, orientation and velocity are the leading fields of every
/// moving alternative (F,P,W; R,P,W; R,V,W; F,V,W and their body-axis
/// forms), so one reading serves them all.
pub fn spatial_of(b: &[u8]) -> DResult<Spatial> {
    let mut d = Dec::new(b);
    let algorithm = d.octet()?;
    if algorithm == 0 || algorithm > 9 {
        return Err(DecodeError(format!("dead-reckoning algorithm {algorithm}")));
    }
    d.align(8)?;
    let location = d.f64x3()?;
    let frozen = d.octet()? != 0;
    let orientation = d.f32x3()?;
    let velocity = if algorithm == 1 { [0.0; 3] } else { d.f32x3()? };
    Ok(Spatial { algorithm, location, frozen, orientation, velocity })
}

/// MarkingStruct: encoding (1, ASCII) and 11 octets.
pub fn marking(s: &str) -> Vec<u8> {
    let mut m = [0u8; 11];
    for (i, c) in s.bytes().filter(u8::is_ascii).take(11).enumerate() {
        m[i] = c;
    }
    Enc::new().octet(1).bytes(&m).done()
}
pub fn marking_of(b: &[u8]) -> DResult<String> {
    let mut d = Dec::new(b);
    let _encoding = d.octet()?;
    Ok(d.bytes(11)?.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect::<String>().trim_end().to_string())
}

/// EventIdentifierStruct: count, then the issuing object's RTIobjectId.
pub fn event_identifier(count: u16, issuer: &str) -> Vec<u8> {
    Enc::new().u16(count).unicode(issuer).done()
}
pub fn event_identifier_of(b: &[u8]) -> DResult<(u16, String)> {
    let mut d = Dec::new(b);
    Ok((d.u16()?, d.unicode()?))
}

pub fn octet(v: u8) -> Vec<u8> {
    vec![v]
}
pub fn u16v(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
pub fn i32v(v: i32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
pub fn unicode(s: &str) -> Vec<u8> {
    Enc::new().unicode(s).done()
}
pub fn world_location(v: [f64; 3]) -> Vec<u8> {
    Enc::new().f64x3(v).done()
}
fn be<const N: usize>(b: &[u8]) -> DResult<[u8; N]> {
    b.get(..N).and_then(|s| s.try_into().ok()).ok_or_else(|| DecodeError(format!("{} octets, wanted {N}", b.len())))
}

// ---- entities ----

/// An entity's attribute values, as published.
pub fn entity_attributes(t: &EntityTruth, class: &str) -> Vec<(&'static str, Vec<u8>)> {
    let spatial_v = Spatial { algorithm: if t.moving { 2 } else { 1 }, location: t.ecef, frozen: false, orientation: t.orientation.map(|x| x as f32), velocity: t.velocity.map(|x| x as f32) };
    let destroyed = t.damage == Damage::Destroyed;
    let mut out = vec![
        ("EntityIdentifier", entity_identifier([t.external.site, t.external.application, t.external.entity])),
        ("EntityType", entity_type(t.entity_type)),
        ("Spatial", spatial(&spatial_v)),
        ("ForceIdentifier", octet(t.force)),
        ("Marking", marking(&t.marking)),
        // DamageStatusEnum32: the DIS damage values.
        ("DamageState", i32v(t.damage as i32)),
        ("IsConcealed", octet(t.concealed as u8)),
        ("FlamesPresent", octet(destroyed as u8)),
        ("SmokePlumePresent", octet(destroyed as u8)),
    ];
    if class.contains(".Platform") {
        out.push(("PowerPlantOn", octet((t.engine_on && !destroyed) as u8)));
        out.push(("FirePowerDisabled", octet(t.firepower_killed as u8)));
        out.push(("Immobilized", octet(t.mobility_killed as u8)));
    }
    if class.contains(".Lifeform") {
        // StanceCodeEnum32: the DIS life form postures.
        out.push(("StanceCode", i32v(t.stance.map(|s| s as i32).unwrap_or(0))));
    }
    out
}

/// What is known of a reflected entity; attributes come when they change,
/// so each update fills in what it carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityAttrs {
    pub id: Option<[u16; 3]>,
    pub entity_type: Option<[u16; 7]>,
    pub spatial: Option<Spatial>,
    pub force: Option<u8>,
    pub marking: Option<String>,
    pub damage: Option<i32>,
}

impl EntityAttrs {
    /// Take in a reflection's values (by attribute name); a value that does
    /// not decode is skipped and reported.
    pub fn merge(&mut self, values: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
        let mut bad = vec![];
        for (k, v) in values {
            let r: DResult<()> = (|| {
                match k.as_str() {
                    "EntityIdentifier" => self.id = Some(entity_identifier_of(v)?),
                    "EntityType" => self.entity_type = Some(entity_type_of(v)?),
                    "Spatial" => self.spatial = Some(spatial_of(v)?),
                    "ForceIdentifier" => self.force = Some(*v.first().ok_or_else(|| DecodeError("empty".into()))?),
                    "Marking" => self.marking = Some(marking_of(v)?),
                    "DamageState" => self.damage = Some(i32::from_be_bytes(be::<4>(v)?)),
                    _ => {}
                }
                Ok(())
            })();
            if let Err(e) = r {
                bad.push(format!("{k}: {e}"));
            }
        }
        bad
    }
}

// ---- interactions ----

/// A WeaponFire or MunitionDetonation's contents.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub event: (u16, String),
    pub firer: String,
    /// Empty when there is no target entity.
    pub target: String,
    /// Firing location (fire) or detonation location, ECEF.
    pub location: [f64; 3],
    pub munition_type: [u16; 7],
    pub warhead: u16,
    pub fuse: u16,
    pub quantity: u16,
    pub rate: u16,
    /// Fire: the range to the target, m.
    pub range_m: f32,
    /// Detonation: DetonationResultCodeEnum8.
    pub result: u8,
}

pub fn weapon_fire(s: &Shot) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("EventIdentifier", event_identifier(s.event.0, &s.event.1)),
        ("FireControlSolutionRange", s.range_m.to_be_bytes().to_vec()),
        ("FiringLocation", world_location(s.location)),
        ("FiringObjectIdentifier", unicode(&s.firer)),
        ("FuseType", u16v(s.fuse)),
        ("MunitionType", entity_type(s.munition_type)),
        ("QuantityFired", u16v(s.quantity)),
        ("RateOfFire", u16v(s.rate)),
        ("TargetObjectIdentifier", unicode(&s.target)),
        ("WarheadType", u16v(s.warhead)),
    ]
}

pub fn munition_detonation(s: &Shot) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("DetonationLocation", world_location(s.location)),
        ("DetonationResultCode", octet(s.result)),
        ("EventIdentifier", event_identifier(s.event.0, &s.event.1)),
        ("FiringObjectIdentifier", unicode(&s.firer)),
        ("FuseType", u16v(s.fuse)),
        ("MunitionType", entity_type(s.munition_type)),
        ("QuantityFired", u16v(s.quantity)),
        ("RateOfFire", u16v(s.rate)),
        ("TargetObjectIdentifier", unicode(&s.target)),
        ("WarheadType", u16v(s.warhead)),
    ]
}

/// Read either interaction's parameters (by name); missing ones are zero.
pub fn shot_of(p: &BTreeMap<String, Vec<u8>>) -> DResult<Shot> {
    let get = |k: &str| p.get(k).map(Vec::as_slice);
    let u16_of = |k: &str| -> DResult<u16> { get(k).map(|b| be::<2>(b).map(u16::from_be_bytes)).transpose().map(Option::unwrap_or_default) };
    let name = |k: &str| -> DResult<String> { get(k).map(|b| Dec::new(b).unicode()).transpose().map(Option::unwrap_or_default) };
    let location = match get("DetonationLocation").or(get("FiringLocation")) {
        Some(b) => Dec::new(b).f64x3()?,
        None => [0.0; 3],
    };
    Ok(Shot {
        event: get("EventIdentifier").map(event_identifier_of).transpose()?.unwrap_or_default(),
        firer: name("FiringObjectIdentifier")?,
        target: name("TargetObjectIdentifier")?,
        location,
        munition_type: get("MunitionType").map(entity_type_of).transpose()?.unwrap_or([2, 0, 0, 0, 0, 0, 0]),
        warhead: u16_of("WarheadType")?,
        fuse: u16_of("FuseType")?,
        quantity: u16_of("QuantityFired")?,
        rate: u16_of("RateOfFire")?,
        range_m: get("FireControlSolutionRange").map(|b| be::<4>(b).map(f32::from_be_bytes)).transpose()?.unwrap_or(0.0),
        result: get("DetonationResultCode").and_then(|b| b.first().copied()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_type_and_identifier_fixtures() {
        // Kind, domain, then the 16-bit country already on a 2-octet boundary.
        assert_eq!(entity_type([1, 1, 225, 1, 1, 3, 0]), [1, 1, 0, 225, 1, 1, 3, 0]);
        assert_eq!(entity_type_of(&[1, 1, 0, 225, 1, 1, 3, 0]).unwrap(), [1, 1, 225, 1, 1, 3, 0]);
        assert_eq!(entity_identifier([1, 2, 300]), [0, 1, 0, 2, 1, 44]);
        assert_eq!(marking("b-1"), [1, b'b', b'-', b'1', 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(marking_of(&marking("Red 1 tanks plus")).unwrap(), "Red 1 tanks");
    }

    #[test]
    fn spatial_fixture_pads_to_the_doubles() {
        let s = Spatial { algorithm: 2, location: [1.0, 2.0, 3.0], frozen: false, orientation: [0.5, -0.25, 0.125], velocity: [1.5, -2.5, 0.25] };
        let b = spatial(&s);
        // Discriminant, 7 octets of padding, 24 of location, IsFrozen,
        // 3 of padding, 12 of orientation, 12 of velocity.
        assert_eq!(b.len(), 60);
        assert_eq!(b[..8], [2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(b[8..16], 1.0f64.to_be_bytes());
        assert_eq!(b[32..36], [0, 0, 0, 0]);
        assert_eq!(b[36..40], 0.5f32.to_be_bytes());
        assert_eq!(b[48..52], 1.5f32.to_be_bytes());
        assert_eq!(spatial_of(&b).unwrap(), s);
        let still = Spatial { algorithm: 1, velocity: [0.0; 3], ..s };
        assert_eq!(spatial(&still).len(), 48);
        assert_eq!(spatial_of(&spatial(&still)).unwrap(), still);
        assert!(spatial_of(&[0; 60]).is_err(), "algorithm Other");
    }

    #[test]
    fn event_identifier_and_shots_round_trip() {
        // Count, 2 octets of padding to the string's 32-bit length, UTF-16.
        assert_eq!(event_identifier(7, "AB"), [0, 7, 0, 0, 0, 0, 0, 2, 0, b'A', 0, b'B']);
        let s = Shot {
            event: (9, "BGWS-1-1-3".into()),
            firer: "BGWS-1-1-3".into(),
            target: "other-7".into(),
            location: [3.5e6, 1.2e6, 5.1e6],
            munition_type: [2, 2, 225, 1, 1, 0, 0],
            warhead: 1600,
            fuse: 1100,
            quantity: 2,
            rate: 0,
            range_m: 0.0,
            result: 1,
        };
        let p: BTreeMap<String, Vec<u8>> = munition_detonation(&s).into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(shot_of(&p).unwrap(), s);
        let f = Shot { range_m: 2400.0, result: 0, ..s.clone() };
        let p: BTreeMap<String, Vec<u8>> = weapon_fire(&f).into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(shot_of(&p).unwrap(), f);
    }
}
