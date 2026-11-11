//! Dead reckoning: when to send an entity's state.
//!
//! Receivers extrapolate each entity from its last state. A sender sends
//! again only when its own extrapolation of what it sent has strayed from
//! the truth: position by more than `position_m` (1 m by convention),
//! orientation by more than `orientation_deg` (3°), or when the appearance
//! changes; and in any case every `heartbeat_s` (5 s in DIS v7). An entity
//! never seen before is sent at once.

use crate::model::EntityTruth;
use bgws_core::EntityId;
use std::collections::BTreeMap;

/// Thresholds (the DIS defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrSettings {
    pub position_m: f64,
    pub orientation_deg: f64,
    pub heartbeat_s: f64,
}

impl Default for DrSettings {
    fn default() -> Self {
        DrSettings { position_m: 1.0, orientation_deg: 3.0, heartbeat_s: 5.0 }
    }
}

/// The DIS dead-reckoning algorithms used (SISO-REF-010 uid 44).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// 1: not moving.
    Static = 1,
    /// 2, DRM(F,P,W): constant velocity, orientation held.
    Fpw = 2,
}

impl Algorithm {
    pub fn for_truth(t: &EntityTruth) -> Algorithm {
        if t.moving {
            Algorithm::Fpw
        } else {
            Algorithm::Static
        }
    }
}

/// Why a state is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    First,
    Heartbeat,
    Position,
    Orientation,
    Appearance,
}

#[derive(Clone, Debug)]
struct Sent {
    at_s: f64,
    ecef: [f64; 3],
    velocity: [f64; 3],
    orientation: [f64; 3],
    appearance: u32,
    algorithm: Algorithm,
}

/// The sender's record of what it last sent for each entity.
#[derive(Clone, Debug, Default)]
pub struct DeadReckoner {
    pub settings: DrSettings,
    sent: BTreeMap<EntityId, Sent>,
}

fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

impl DeadReckoner {
    pub fn new(settings: DrSettings) -> Self {
        DeadReckoner { settings, sent: BTreeMap::new() }
    }

    /// Where a receiver puts the entity now, from what was last sent.
    fn extrapolated(s: &Sent, now_s: f64) -> [f64; 3] {
        let dt = (now_s - s.at_s).max(0.0);
        match s.algorithm {
            Algorithm::Static => s.ecef,
            Algorithm::Fpw => [s.ecef[0] + s.velocity[0] * dt, s.ecef[1] + s.velocity[1] * dt, s.ecef[2] + s.velocity[2] * dt],
        }
    }

    /// Whether (and why) this entity's state should be sent now.
    pub fn due(&self, now_s: f64, t: &EntityTruth) -> Option<Due> {
        let Some(s) = self.sent.get(&t.id) else { return Some(Due::First) };
        if now_s - s.at_s >= self.settings.heartbeat_s {
            return Some(Due::Heartbeat);
        }
        if t.appearance() != s.appearance || Algorithm::for_truth(t) != s.algorithm {
            return Some(Due::Appearance);
        }
        let p = Self::extrapolated(s, now_s);
        let err = ((p[0] - t.ecef[0]).powi(2) + (p[1] - t.ecef[1]).powi(2) + (p[2] - t.ecef[2]).powi(2)).sqrt();
        if err > self.settings.position_m {
            return Some(Due::Position);
        }
        let turn = (0..3).map(|k| wrap(t.orientation[k] - s.orientation[k]).abs()).fold(0.0, f64::max);
        if turn.to_degrees() > self.settings.orientation_deg {
            return Some(Due::Orientation);
        }
        None
    }

    /// Note that this state was sent.
    pub fn sent(&mut self, now_s: f64, t: &EntityTruth) {
        self.sent.insert(t.id, Sent { at_s: now_s, ecef: t.ecef, velocity: t.velocity, orientation: t.orientation, appearance: t.appearance(), algorithm: Algorithm::for_truth(t) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Damage, ExternalId};
    use bgws_core::Side;

    fn at(x: f64, vx: f64, heading: f64) -> EntityTruth {
        EntityTruth {
            id: EntityId(3),
            external: ExternalId { site: 1, application: 1, entity: 4 },
            side: Side::Blue,
            force: 1,
            entity_type: [1, 1, 0, 1, 0, 0, 0],
            marking: "t".into(),
            ecef: [x, 0.0, 0.0],
            velocity: [vx, 0.0, 0.0],
            orientation: [heading, 0.0, 0.0],
            damage: Damage::None,
            mobility_killed: false,
            firepower_killed: false,
            engine_on: true,
            concealed: false,
            stance: None,
            moving: vx != 0.0,
        }
    }

    #[test]
    fn a_steady_track_is_sent_rarely_and_never_strays_past_the_threshold() {
        let mut dr = DeadReckoner::default();
        let mut sends = 0;
        // 10 m/s, steady for 60 s, then a 1 m/s² slow-down for 5 s.
        let (mut x, mut v) = (0.0, 10.0);
        let mut worst: f64 = 0.0;
        for step in 0..65 {
            let t = step as f64;
            if step >= 60 {
                v -= 1.0;
            }
            x += v;
            let truth = at(x, v, 0.0);
            if let Some(s) = dr.sent.get(&truth.id) {
                let p = DeadReckoner::extrapolated(s, t);
                worst = worst.max((p[0] - x).abs());
            }
            if dr.due(t, &truth).is_some() {
                dr.sent(t, &truth);
                sends += 1;
            }
        }
        // Heartbeats only (every 5 s) while steady, then a few while braking.
        assert!(sends < 20, "{sends} states for 65 s of one entity");
        // What a receiver saw was never more than the threshold (plus one step's motion) off.
        assert!(worst <= 1.0 + 10.0, "worst error {worst:.1} m");
    }

    #[test]
    fn a_turn_or_a_change_of_look_is_sent_at_once() {
        let mut dr = DeadReckoner::default();
        let t = at(0.0, 0.0, 0.0);
        assert_eq!(dr.due(0.0, &t), Some(Due::First));
        dr.sent(0.0, &t);
        assert_eq!(dr.due(1.0, &t), None);
        assert_eq!(dr.due(1.0, &at(0.0, 0.0, 4f64.to_radians())), Some(Due::Orientation));
        let mut hit = at(0.0, 0.0, 0.0);
        hit.damage = Damage::Destroyed;
        assert_eq!(dr.due(1.0, &hit), Some(Due::Appearance));
        assert_eq!(dr.due(5.0, &t), Some(Due::Heartbeat));
        // An angle across ±180° is a small turn, not a full one.
        dr.sent(6.0, &at(0.0, 0.0, 179f64.to_radians()));
        assert_eq!(dr.due(7.0, &at(0.0, 0.0, (-179f64).to_radians())), None);
    }
}
