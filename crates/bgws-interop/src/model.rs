//! The truth a simulation publishes, protocol-free: what each entity is,
//! where, how it is moving, how it looks. DIS (now) and HLA (phase 3) both
//! encode this. It is read from the sim's ground truth, never from a side's
//! picture.

use crate::geo::{dis_orientation, place, to_ecef_vec};
use crate::geoid::Geoid;
use bgws_core::{EntityId, Side};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::components::{Act, Activity, Health, Ident, Kin, Posture, SigState};
use bgws_sim::Sim;

/// An entity's external identity: DIS site, application and entity number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalId {
    pub site: u16,
    pub application: u16,
    pub entity: u16,
}

impl ExternalId {
    /// Ours: the scenario's site and application, and the sim's entity
    /// index plus one (DIS reserves 0 for "no entity" and 65535 for "all").
    pub fn ours(spec: &InteropSpec, id: EntityId) -> Option<ExternalId> {
        let entity = u16::try_from(id.0 + 1).ok().filter(|&n| n < u16::MAX)?;
        Some(ExternalId { site: spec.site, application: spec.application, entity })
    }
}

/// Damage as DIS shows it (SISO-REF-010 Appearance-Damage / Life Form Health).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    None = 0,
    Slight = 1,
    Moderate = 2,
    Destroyed = 3,
}

/// How a person on foot is standing (SISO-REF-010 Life Form Posture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    StandingStill = 1,
    Walking = 2,
    Kneeling = 4,
    Prone = 5,
}

/// One entity as published.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityTruth {
    pub id: EntityId,
    pub external: ExternalId,
    pub side: Side,
    /// SISO-REF-010 force ID.
    pub force: u8,
    /// kind, domain, country, category, subcategory, specific, extra.
    pub entity_type: [u16; 7],
    /// The label, up to 11 ASCII characters.
    pub marking: String,
    pub ecef: [f64; 3],
    pub velocity: [f64; 3],
    /// DIS ψ, θ, φ, radians.
    pub orientation: [f64; 3],
    pub damage: Damage,
    pub mobility_killed: bool,
    pub firepower_killed: bool,
    pub engine_on: bool,
    /// In a prepared, hull-down or hidden position.
    pub concealed: bool,
    /// For people on foot.
    pub stance: Option<Stance>,
    pub moving: bool,
}

impl EntityTruth {
    pub fn kind(&self) -> u16 {
        self.entity_type[0]
    }
    pub fn domain(&self) -> u16 {
        self.entity_type[1]
    }

    /// The 32-bit DIS appearance (SISO-REF-010 Land Platform Appearance,
    /// Life Forms Appearance; for other kinds the damage bits, 3-4, which
    /// those records share).
    pub fn appearance(&self) -> u32 {
        let mut a = (self.damage as u32) << 3;
        let set = |a: &mut u32, bit: u32, on: bool| {
            if on {
                *a |= 1 << bit;
            }
        };
        match (self.kind(), self.domain()) {
            // Land platform.
            (1, 1) => {
                set(&mut a, 1, self.mobility_killed);
                set(&mut a, 2, self.firepower_killed);
                // A wreck smokes and burns.
                set(&mut a, 5, self.damage == Damage::Destroyed);
                set(&mut a, 15, self.damage == Damage::Destroyed);
                set(&mut a, 19, self.concealed);
                set(&mut a, 22, self.engine_on && self.damage != Damage::Destroyed);
            }
            // Life form.
            (3, _) => {
                if let Some(s) = self.stance {
                    a |= (s as u32) << 16;
                }
                set(&mut a, 30, self.concealed && !self.moving);
            }
            _ => {}
        }
        a
    }
}

/// How far either side of an entity the ground slope is sampled, m.
const SLOPE_M: f64 = 5.0;

/// Every entity a game publishes, now: each of ours with a DIS type (a
/// cyber cell has none), alive or wrecked; not another simulation's.
pub fn truths(sim: &Sim, spec: &InteropSpec) -> Vec<EntityTruth> {
    let geoid = Geoid::egm96();
    let frame = &sim.ctx.frame;
    let mut out = Vec::new();
    for (idx, &e) in sim.ctx.order.iter().enumerate() {
        let (Some(i), Some(k), Some(h)) = (sim.world.get::<Ident>(e), sim.world.get::<Kin>(e), sim.world.get::<Health>(e)) else { continue };
        let Some(dis) = i.platform.dis else { continue };
        // Another simulation's entity is its owner's to publish.
        if bgws_sim::remote::is_remote(sim, e) {
            continue;
        }
        let id = EntityId(idx as u32);
        let Some(external) = ExternalId::ours(spec, id) else { continue };
        let act = sim.world.get::<Act>(e);
        let posture = act.map(|a| a.posture).unwrap_or(Posture::Halted);
        let moving = k.vel.horizontal() > 0.2 || act.is_some_and(|a| a.activity.moving() && posture == Posture::Moving);
        let placed = place(frame, geoid, k.pos);
        // A vehicle on the ground follows its slope; aircraft fly level here.
        let (pitch, roll) = if k.agl <= 0.0 && dis.kind != 3 {
            let (sh, ch) = k.heading.sin_cos();
            let g = |de: f64, dn: f64| sim.ctx.ground.height(k.pos.e + de, k.pos.n + dn);
            let pitch = ((g(sh * SLOPE_M, ch * SLOPE_M) - g(-sh * SLOPE_M, -ch * SLOPE_M)) / (2.0 * SLOPE_M)).atan();
            // Right side lower: right side down, positive roll.
            let roll = ((g(-ch * SLOPE_M, sh * SLOPE_M) - g(ch * SLOPE_M, -sh * SLOPE_M)) / (2.0 * SLOPE_M)).atan();
            (pitch, roll)
        } else {
            (0.0, 0.0)
        };
        let damage = if h.destroyed {
            Damage::Destroyed
        } else if h.strength() < 0.5 {
            Damage::Moderate
        } else if h.fit < h.start {
            Damage::Slight
        } else {
            Damage::None
        };
        let concealed = posture == Posture::HullDown || act.is_some_and(|a| matches!(a.activity, Activity::Quiet));
        let stance = (dis.kind == 3).then_some(match posture {
            _ if moving => Stance::Walking,
            Posture::HullDown => Stance::Prone,
            Posture::Settled => Stance::Kneeling,
            _ => Stance::StandingStill,
        });
        out.push(EntityTruth {
            id,
            external,
            side: i.side,
            force: spec.force_of(i.side),
            entity_type: dis.with_country(spec.country_of(i.side)),
            marking: i.label.chars().filter(char::is_ascii).take(11).collect(),
            ecef: placed.ecef,
            velocity: to_ecef_vec(&placed.axes, k.vel),
            orientation: dis_orientation(&placed.axes, k.heading, pitch, roll),
            damage,
            mobility_killed: !h.destroyed && !h.can_move(),
            firepower_killed: !h.destroyed && !h.can_fire(),
            engine_on: sim.world.get::<SigState>(e).is_some_and(|s| s.engine_on),
            concealed,
            stance,
            moving,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth() -> EntityTruth {
        EntityTruth {
            id: EntityId(0),
            external: ExternalId { site: 1, application: 1, entity: 1 },
            side: Side::Blue,
            force: 1,
            entity_type: [1, 1, 0, 1, 0, 0, 0],
            marking: "b-1".into(),
            ecef: [0.0; 3],
            velocity: [0.0; 3],
            orientation: [0.0; 3],
            damage: Damage::None,
            mobility_killed: false,
            firepower_killed: false,
            engine_on: true,
            concealed: false,
            stance: None,
            moving: false,
        }
    }

    #[test]
    fn appearance_bits_follow_siso_ref_010() {
        let mut t = truth();
        assert_eq!(t.appearance(), 1 << 22, "a running tank: power plant on");
        t.damage = Damage::Destroyed;
        // Damage 3 at bits 3-4, smoke (5) and flames (15); the engine is out.
        assert_eq!(t.appearance(), (3 << 3) | (1 << 5) | (1 << 15));
        let mut t = truth();
        t.entity_type = [3, 1, 0, 10, 5, 0, 0];
        t.stance = Some(Stance::Prone);
        t.concealed = true;
        t.damage = Damage::Slight;
        // Health 1 at bits 3-4, posture 5 (prone) at bits 16-19, concealed stationary (30).
        assert_eq!(t.appearance(), (1 << 3) | (5 << 16) | (1 << 30));
    }

    #[test]
    fn external_ids_skip_dis_reserved_numbers() {
        let spec = InteropSpec { site: 7, application: 9, ..Default::default() };
        assert_eq!(ExternalId::ours(&spec, EntityId(0)), Some(ExternalId { site: 7, application: 9, entity: 1 }));
        assert_eq!(ExternalId::ours(&spec, EntityId(65_533)).map(|x| x.entity), Some(65_534));
        assert_eq!(ExternalId::ours(&spec, EntityId(65_534)), None, "65535 means all entities");
    }
}
