//! The `SideView` (§2 rule 1, §9.2): the only knowledge decision code may
//! read. Plain data the simulation fills: the viewer's side and time, its
//! own forces as the side believes them (with self-position error, so a
//! GNSS-jammed unit is unsure where it is), the tracks that reached the
//! viewer, and recent messages.
//!
//! None of its types can carry truth: they are built from [`KnownTrack`]s
//! (which come from [`crate::store::TrackView`]s) and from own-force beliefs
//! the simulation supplies. A property test serialises views built from
//! stores fed with truth-tagged detections and checks no audit field appears.

use crate::knowledge::{AgentKnowledge, Cop, KnownTrack};
use crate::params::KnowParams;
use bgws_core::{EntityId, Enu, HqId, MessageId, Side, SimTime, TrackId};
use serde::{Deserialize, Serialize};

/// Who is looking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Viewer {
    Agent(EntityId),
    Hq(HqId),
}

/// One own unit as the side believes it to be.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnForceView {
    pub id: EntityId,
    /// Where it believes it is (its navigation solution).
    pub believed_position: Enu,
    /// 1σ error of that belief, metres (grows under GNSS denial).
    pub position_sigma: f64,
    pub velocity: Enu,
    /// Fighting strength as last reported, 0–1.
    pub strength: f64,
    /// When this state was last reported.
    pub reported_at: SimTime,
    /// A short status ("moving", "in position", "suppressed").
    pub status: String,
}

/// A message that reached the viewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageView {
    pub id: MessageId,
    pub from: EntityId,
    pub sent: SimTime,
    pub received: SimTime,
    /// "contact", "sitrep", "request", …
    pub kind: String,
    pub text: String,
    /// Tracks the message reported.
    pub tracks: Vec<TrackId>,
}

/// What a decider may read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SideView {
    pub side: Side,
    pub time: SimTime,
    pub viewer: Viewer,
    pub own_forces: Vec<OwnForceView>,
    pub tracks: Vec<KnownTrack>,
    pub messages: Vec<MessageView>,
}

impl SideView {
    /// A unit's view: its own sightings and what it has been told.
    pub fn for_agent(
        k: &AgentKnowledge,
        time: SimTime,
        own_forces: Vec<OwnForceView>,
        messages: Vec<MessageView>,
        p: &KnowParams,
    ) -> SideView {
        SideView { side: k.side, time, viewer: Viewer::Agent(k.agent), own_forces, tracks: k.picture(time, p), messages }
    }

    /// An HQ's view: its COP.
    pub fn for_hq(cop: &Cop, time: SimTime, own_forces: Vec<OwnForceView>, messages: Vec<MessageView>, p: &KnowParams) -> SideView {
        SideView { side: cop.side, time, viewer: Viewer::Hq(cop.hq), own_forces, tracks: cop.picture(time, p), messages }
    }

    /// The known track nearest a point, by estimated position.
    pub fn nearest_track(&self, at: Enu) -> Option<&KnownTrack> {
        self.tracks.iter().min_by(|a, b| {
            let da = a.estimate.position().sub(at).horizontal();
            let db = b.estimate.position().sub(at).horizontal();
            da.total_cmp(&db).then(a.track.cmp(&b.track))
        })
    }
}
