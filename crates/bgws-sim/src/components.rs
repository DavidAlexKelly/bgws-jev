//! Entity components (§5.1). Every platform or element is an ECS entity
//! built from these; systems ask for exactly what they need.
//!
//! Truth lives here. Nothing that decides reads these directly: deciders get
//! a `SideView` or a question's words, built from `Aware` and the side's
//! track store (rule 1).

use bevy_ecs::prelude::*;
use bgws_catalogue::{Domain, Platform};
use bgws_core::{Enu, EntityId, Side, SimTime};
use bgws_models::direct_fire::Engagement as FireEngagement;
use bgws_models::ground::MoveMode;
use bgws_models::people::{Fatigue, Morale, Skill, Suppression};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Who and what it is.
#[derive(Component, Clone, Debug)]
pub struct Ident {
    pub id: EntityId,
    /// The scenario's stable id.
    pub name: String,
    pub label: String,
    pub side: Side,
    pub platform: Arc<Platform>,
    pub domain: Domain,
    /// The behaviour template that answers its decision points.
    pub template: String,
    /// Troop quality 1–6 (4 regular).
    pub quality: u8,
    pub dummy: bool,
    /// A command post.
    pub hq: bool,
}

/// Where it is and how it moves, in the theatre's ENU frame.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Kin {
    pub pos: Enu,
    pub vel: Enu,
    /// Radians clockwise from north.
    pub heading: f64,
    /// Height above the ground (aircraft, drones) or 0.
    pub agl: f64,
}

/// What is left of it: platforms (vehicles, teams, aircraft) fit of the start.
#[derive(Component, Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub start: u32,
    pub fit: u32,
    pub mobility_kills: u32,
    pub firepower_kills: u32,
    pub destroyed: bool,
    pub losses: Vec<SimTime>,
}

impl Health {
    pub fn new(n: u32) -> Self {
        Health { start: n.max(1), fit: n.max(1), mobility_kills: 0, firepower_kills: 0, destroyed: false, losses: Vec::new() }
    }
    /// Fraction of the start still fit.
    pub fn strength(&self) -> f64 {
        if self.destroyed {
            0.0
        } else {
            self.fit as f64 / self.start as f64
        }
    }
    pub fn can_move(&self) -> bool {
        !self.destroyed && self.mobility_kills < self.fit
    }
    pub fn can_fire(&self) -> bool {
        !self.destroyed && self.firepower_kills < self.fit
    }
}

/// The people in it (§8.10).
#[derive(Component, Clone, Debug)]
pub struct Crew {
    pub suppression: Suppression,
    pub morale: Morale,
    pub fatigue: Fatigue,
    pub skill: Skill,
    /// Near misses and blasts this minute, decayed each tick.
    pub near_misses_per_min: f64,
    pub blasts_per_min: f64,
    /// Casualties since the last morale step, as a fraction of the start.
    pub new_casualties: f64,
    /// Who last fired at it, and when.
    pub last_incoming: Option<(SimTime, EntityId)>,
    pub success: bool,
}

/// One weapon mount's state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MountState {
    pub index: usize,
    pub munition: String,
    pub rounds: u32,
    pub ready_at: SimTime,
}

#[derive(Component, Clone, Debug, Default)]
pub struct Arms {
    pub mounts: Vec<MountState>,
    /// Per-target engagements: the fixed bias of this firer against that target.
    pub engagements: BTreeMap<EntityId, FireEngagement>,
}

impl Arms {
    pub fn rounds(&self) -> u32 {
        self.mounts.iter().map(|m| m.rounds).sum()
    }
}

/// When a waiting unit's moment comes (decision point D4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WaitTrigger {
    Range { within_m: f64 },
    Flank,
}

/// What it is doing this second. The autopilot carries it out every tick;
/// a decision changes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Activity {
    Hold,
    /// Hold and fire at anything in reach the ROE allows.
    Overwatch,
    /// Hold, do not fire, watch and report.
    Observe,
    Move { route: Vec<Enu>, mode: MoveModeS, then: Option<Box<Activity>> },
    Withdraw { route: Vec<Enu>, then: Option<Box<Activity>> },
    Engage { target: EntityId, volleys: Option<u32>, then: Option<Box<Activity>> },
    Wait { target: EntityId, trigger: WaitTrigger, auto_fire: bool, met: bool },
    Search { bearing: f64, until: SimTime },
    /// Hidden and silent: no fire, no movement, no transmissions.
    Quiet,
}

impl Activity {
    pub fn word(&self) -> &'static str {
        match self {
            Activity::Hold => "holding",
            Activity::Overwatch => "on overwatch",
            Activity::Observe => "observing",
            Activity::Move { .. } => "moving",
            Activity::Withdraw { .. } => "withdrawing",
            Activity::Engage { .. } => "engaging",
            Activity::Wait { .. } => "waiting in ambush",
            Activity::Search { .. } => "searching",
            Activity::Quiet => "gone quiet",
        }
    }
    pub fn moving(&self) -> bool {
        matches!(self, Activity::Move { .. } | Activity::Withdraw { .. })
    }
    pub fn may_fire(&self) -> bool {
        !matches!(self, Activity::Observe | Activity::Quiet | Activity::Withdraw { .. } | Activity::Wait { met: false, .. })
    }
}

/// `MoveMode` with serde, for activities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MoveModeS {
    March,
    Tactical,
    Assault,
    Stealth,
}

impl From<MoveModeS> for MoveMode {
    fn from(m: MoveModeS) -> MoveMode {
        match m {
            MoveModeS::March => MoveMode::March,
            MoveModeS::Tactical => MoveMode::Tactical,
            MoveModeS::Assault => MoveMode::Assault,
            MoveModeS::Stealth => MoveMode::Stealth,
        }
    }
}

/// How exposed it is, from how long it has been still and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Posture {
    Moving,
    Halted,
    Settled,
    HullDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RoeS {
    Never,
    IfFiredUpon,
    WithinShortRange,
    #[default]
    Always,
}

/// What it is for (outlasts any one order), and its phased orders.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mission {
    pub purpose: String,
    pub phases: Vec<Activity>,
    pub phase: usize,
    pub urgent: bool,
}

#[derive(Component, Clone, Debug)]
pub struct Act {
    pub activity: Activity,
    pub roe: RoeS,
    pub posture: Posture,
    pub still_since: SimTime,
    pub mission: Mission,
    /// A question about it is out, or its answer is waiting to apply.
    pub deciding: bool,
    pub last_shot: Option<SimTime>,
    pub last_event: SimTime,
    pub volleys: u32,
}

/// What it has seen itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Seen {
    pub first: SimTime,
    pub last: SimTime,
    /// 0 detect, 1 recognise, 2 identify.
    pub level: u8,
    pub at: Enu,
    pub moving: bool,
}

/// Someone fired at it from a bearing it has not located.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suspect {
    pub bearing: f64,
    pub since: SimTime,
    pub last: SimTime,
}

#[derive(Component, Clone, Debug, Default)]
pub struct Aware {
    pub seen: BTreeMap<EntityId, Seen>,
    pub suspects: BTreeMap<EntityId, Suspect>,
    /// When it last reported each enemy to its side.
    pub reported: BTreeMap<EntityId, SimTime>,
}

/// Signature state (§8.3).
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize)]
pub struct SigState {
    pub last_fired: Option<SimTime>,
    pub last_moved: Option<SimTime>,
    pub engine_on: bool,
    pub emitting: bool,
}

/// Level of detail (§6.4).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Lod {
    #[default]
    Entity,
    Aggregated,
    Far,
}
