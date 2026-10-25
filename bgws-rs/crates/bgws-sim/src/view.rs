//! Views (§3.2, §16): frames for the UI (truth for the umpire, else a side's
//! belief), the `SideView` deciders may read, and snapshots.

use crate::components::*;
use crate::sim::{Sim, Snapshot};
use bgws_core::{EntityId, HqId, Side};
use bgws_know::{Cop, OwnForceView, SideView};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Viewer {
    Umpire,
    Blue,
    Red,
}

impl Viewer {
    pub fn side(self) -> Option<Side> {
        match self {
            Viewer::Umpire => None,
            Viewer::Blue => Some(Side::Blue),
            Viewer::Red => Some(Side::Red),
        }
    }
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameEntity {
    pub id: u32,
    pub name: String,
    pub label: String,
    pub side: Side,
    pub domain: String,
    pub platform: String,
    pub sidc: String,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub heading_deg: f64,
    pub speed: f64,
    pub strength: f64,
    pub fit: u32,
    pub total: u32,
    pub status: String,
    pub activity: String,
    pub suppression: f64,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameTrack {
    pub id: u32,
    /// The side that holds the track.
    pub owner: Side,
    pub lat: f64,
    pub lon: f64,
    pub sx: f64,
    pub sy: f64,
    pub angle_deg: f64,
    pub class: String,
    pub p: f64,
    pub age_s: f64,
    pub quality: u8,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameView {
    pub t: f64,
    pub clock: String,
    pub viewer: Viewer,
    pub entities: Vec<FrameEntity>,
    pub tracks: Vec<FrameTrack>,
    pub waiting: usize,
    pub over: Option<crate::sim::Outcome>,
}

pub fn frame(s: &Sim, viewer: Viewer) -> FrameView {
    let mut entities = Vec::new();
    for &e in &s.ctx.order {
        let (Some(i), Some(k), Some(h)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e), s.world.get::<Health>(e)) else { continue };
        if let Some(side) = viewer.side() {
            if i.side != side {
                continue;
            }
        }
        let a = s.world.get::<Act>(e);
        let c = s.world.get::<Crew>(e);
        let g = s.ctx.frame.enu_to_geodetic(k.pos);
        let morale = c.map(|c| c.morale.state);
        let status = if h.destroyed {
            "destroyed"
        } else if matches!(morale, Some(bgws_models::people::MoraleState::Broken | bgws_models::people::MoraleState::Routed)) {
            "broken"
        } else if c.map(|c| c.suppression.effects(&s.ctx.p.people).pinned).unwrap_or(false) {
            "pinned"
        } else if c.map(|c| c.suppression.level > 0.3).unwrap_or(false) {
            "suppressed"
        } else {
            "ok"
        };
        entities.push(FrameEntity {
            id: i.id.0,
            name: i.name.clone(),
            label: i.label.clone(),
            side: i.side,
            domain: format!("{:?}", i.domain).to_lowercase(),
            platform: i.platform.name.clone(),
            sidc: if i.side == Side::Red { i.platform.sidc.red.clone() } else { i.platform.sidc.blue.clone() },
            lat: g.lat,
            lon: g.lon,
            alt: k.pos.u + k.agl,
            heading_deg: k.heading.to_degrees(),
            speed: k.vel.norm(),
            strength: h.strength(),
            fit: h.fit,
            total: h.start,
            status: status.into(),
            activity: a.map(|a| a.activity.word().to_string()).unwrap_or_default(),
            suppression: c.map(|c| c.suppression.level).unwrap_or(0.0),
        });
    }
    let mut tracks = Vec::new();
    for (side, store) in &s.ctx.tracks {
        if let Some(v) = viewer.side() {
            if v != *side {
                continue;
            }
        }
        for t in store.confirmed(&s.ctx.p.know) {
            let g = s.ctx.frame.enu_to_geodetic(t.position);
            let (class, p) = t.class_top.first().map(|(h, p)| (format!("{h:?}"), *p)).unwrap_or(("unknown".into(), 0.0));
            tracks.push(FrameTrack {
                id: t.id.0,
                owner: *side,
                lat: g.lat,
                lon: g.lon,
                sx: t.ellipse.semi_major,
                sy: t.ellipse.semi_minor,
                angle_deg: t.ellipse.orientation.to_degrees(),
                class,
                p,
                age_s: (s.ctx.now - t.last_update).secs(),
                quality: t.quality,
            });
        }
    }
    FrameView {
        t: s.ctx.now.secs(),
        clock: format!("{}", s.ctx.now),
        viewer,
        entities,
        tracks,
        waiting: s.ctx.pending.values().filter(|p| !p.answered).count(),
        over: s.ctx.over.clone(),
    }
}

/// What a side's deciders may read: its own forces (as reported) and the
/// tracks that reached its HQ, fused by covariance intersection (§9.2).
pub fn side_view(s: &Sim, side: Side) -> SideView {
    let mut cop = Cop::new(HqId(side.index() as u32), side);
    let p = &s.ctx.p.know;
    for t in s.ctx.tracks[&side].confirmed(p) {
        let from = t.sources.first().copied().unwrap_or(EntityId(0));
        cop.receive(&t, from, t.last_update, p);
    }
    let own = s
        .ctx
        .order
        .iter()
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            if i.side != side {
                return None;
            }
            let k = s.world.get::<Kin>(e)?;
            let h = s.world.get::<Health>(e)?;
            let a = s.world.get::<Act>(e)?;
            Some(OwnForceView { id: i.id, believed_position: k.pos, position_sigma: 5.0, velocity: k.vel, strength: h.strength(), reported_at: s.ctx.now, status: a.activity.word().into() })
        })
        .collect();
    SideView::for_hq(&cop, s.ctx.now, own, vec![], p)
}

pub fn snapshot(s: &Sim) -> Snapshot {
    let entities = s
        .ctx
        .order
        .iter()
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            let k = s.world.get::<Kin>(e)?;
            let h = s.world.get::<Health>(e)?;
            let a = s.world.get::<Act>(e)?;
            Some(serde_json::json!({ "id": i.id, "name": i.name, "kin": k, "health": h, "activity": a.activity, "mission": a.mission }))
        })
        .collect();
    Snapshot { at: s.ctx.now, fingerprint: s.ctx.log.fingerprint(), entities }
}
