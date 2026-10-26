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

/// Something on the air (§16 spectrum view): jammers, radars, drone links
/// and radios that transmitted in the last second.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameEmitter {
    pub entity: u32,
    pub side: Side,
    /// "jammer", "radar", "drone link" or "radio".
    pub kind: String,
    pub freq_mhz: f64,
    pub bw_mhz: f64,
    pub erp_w: f64,
    pub lat: f64,
    pub lon: f64,
    /// A jammer's nominal radius, m (0 for other emitters).
    pub radius_m: f64,
}

/// A comms net link from a member to its HQ post (§16 network graph).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameLink {
    pub from: u32,
    pub to: u32,
    pub side: Side,
    /// The bearer ("Vhf", "Hf", "SatcomLeo"…).
    pub bearer: String,
    /// Neither end is dead or cut off.
    pub up: bool,
    /// Messages on their way between the two now.
    pub traffic: u32,
}

/// A unit's longest-reaching sensor (§16 sensor footprints).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameSensor {
    pub entity: u32,
    pub side: Side,
    pub kind: String,
    pub range_m: f64,
    /// Field of regard; 360 is all round.
    pub for_deg: f64,
    pub heading_deg: f64,
    pub active: bool,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameView {
    pub t: f64,
    pub clock: String,
    pub viewer: Viewer,
    pub entities: Vec<FrameEntity>,
    pub tracks: Vec<FrameTrack>,
    pub emitters: Vec<FrameEmitter>,
    pub links: Vec<FrameLink>,
    pub sensors: Vec<FrameSensor>,
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
    // Drones in flight are records of the drones domain, not entities: show them too.
    for (fid, team, side, kind, pos, agl, heading) in crate::domains::drones::drones::airborne(s) {
        if viewer.side().map(|v| v != side).unwrap_or(false) {
            continue;
        }
        let g = s.ctx.frame.enu_to_geodetic(pos);
        let team_name = s.world.get::<Ident>(s.ctx.entity(team)).map(|i| i.name.clone()).unwrap_or_default();
        entities.push(FrameEntity {
            id: 1_000_000 + fid as u32,
            name: format!("{team_name} drone {fid}"),
            label: format!("{kind:?}"),
            side,
            domain: "drone".into(),
            platform: format!("{kind:?}"),
            sidc: String::new(),
            lat: g.lat,
            lon: g.lon,
            alt: pos.u + agl,
            heading_deg: heading.to_degrees(),
            speed: 0.0,
            strength: 1.0,
            fit: 1,
            total: 1,
            status: "ok".into(),
            activity: "flying".into(),
            suppression: 0.0,
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
    let (emitters, links) = spectrum_and_nets(s, viewer);
    let sensors = sensors(s, viewer);
    FrameView {
        t: s.ctx.now.secs(),
        clock: format!("{}", s.ctx.now),
        viewer,
        entities,
        tracks,
        emitters,
        links,
        sensors,
        waiting: s.ctx.pending.values().filter(|p| !p.answered).count(),
        over: s.ctx.over.clone(),
    }
}

fn sees(viewer: Viewer, side: Side) -> bool {
    viewer.side().is_none_or(|v| v == side)
}

/// How long after a radio transmission the spectrum view still shows it, s.
const RADIO_SHOWN_S: f64 = 10.0;

/// What is on the air, and the nets, from the C4 domain (empty without it).
fn spectrum_and_nets(s: &Sim, viewer: Viewer) -> (Vec<FrameEmitter>, Vec<FrameLink>) {
    use crate::domains::drones::C4;
    let Some(c4) = s.world.get_resource::<C4>() else { return (Vec::new(), Vec::new()) };
    let at = |id: EntityId| {
        let e = s.ctx.entity(id);
        s.world.get::<crate::components::Kin>(e).map(|k| s.ctx.frame.enu_to_geodetic(k.pos))
    };
    let mut emitters = Vec::new();
    let mut push = |id: EntityId, side: Side, kind: &str, f: f64, bw: f64, erp: f64, radius: f64| {
        if !sees(viewer, side) {
            return;
        }
        if let Some(g) = at(id) {
            emitters.push(FrameEmitter { entity: id.0, side, kind: kind.into(), freq_mhz: f / 1e6, bw_mhz: bw / 1e6, erp_w: erp, lat: g.lat, lon: g.lon, radius_m: radius });
        }
    };
    for j in c4.ew.jammers.iter().filter(|j| j.jam.on) {
        for b in &j.jam.bands {
            push(j.entity, j.side, "jammer", b.centre_hz, b.width_hz, j.jam.power_w / j.jam.bands.len().max(1) as f64, j.radius_m);
        }
    }
    let mut seen_radio = std::collections::BTreeSet::new();
    let heard = c4.ew.last_radio.values().filter(|(at, _)| (s.ctx.now - *at).secs() <= RADIO_SHOWN_S).map(|(_, t)| t);
    for t in c4.ew.last_tx.iter().filter(|t| t.what != "radio").chain(heard) {
        if t.what != "jammer" && seen_radio.insert((t.entity, t.what)) {
            push(t.entity, t.side, if t.what.starts_with("radar") || t.what.starts_with("drone") { t.what } else { "radio" }, t.freq_hz, t.bw_hz, t.erp_w, 0.0);
        }
    }
    for (id, role) in &c4.roles {
        let side = s.world.get::<Ident>(s.ctx.entity(*id)).map(|i| i.side);
        let Some(side) = side.filter(|_| crate::systems::alive(s, s.ctx.entity(*id))) else { continue };
        if c4.may_emit(*id) && !seen_radio.contains(&(*id, "radar")) {
            for (f, erp, _) in &role.radars {
                push(*id, side, "radar", *f, 1e6, *erp, 0.0);
            }
        }
        if let Some((f, erp, bw)) = role.drone_link {
            if !seen_radio.contains(&(*id, "drone link")) && c4.drones.flights.values().any(|fl| fl.team == *id && !fl.done && fl.spec.jammable() && !fl.link.lost) {
                push(*id, side, "drone link", f, bw, erp, 0.0);
            }
        }
    }
    let mut links = Vec::new();
    if c4.comms.active {
        for net in c4.comms.nets.iter().filter(|n| sees(viewer, n.side)) {
            let Some(post) = c4.c2.hqs.get(net.hq).map(|h| h.post) else { continue };
            let down = |id: &EntityId| c4.comms.dead.contains(id) || c4.comms.cut.contains(id);
            for m in net.members.iter().filter(|m| **m != post) {
                let traffic = c4.comms.flying.values().filter(|f| (f.from == *m && f.to == post) || (f.from == post && f.to == *m)).count() as u32;
                links.push(FrameLink { from: m.0, to: post.0, side: net.side, bearer: format!("{:?}", net.kind), up: !down(m) && !down(&post), traffic });
            }
        }
    }
    (emitters, links)
}

/// Each unit's longest-reaching sensor.
fn sensors(s: &Sim, viewer: Viewer) -> Vec<FrameSensor> {
    let mut out = Vec::new();
    for &e in &s.ctx.order {
        let (Some(i), Some(k)) = (s.world.get::<Ident>(e), s.world.get::<crate::components::Kin>(e)) else { continue };
        if !sees(viewer, i.side) || !crate::systems::alive(s, e) {
            continue;
        }
        if let Some(best) = i.platform.sensors.iter().max_by(|a, b| a.range_max_m.total_cmp(&b.range_max_m)) {
            out.push(FrameSensor { entity: i.id.0, side: i.side, kind: format!("{:?}", best.kind), range_m: best.range_max_m, for_deg: best.for_deg, heading_deg: k.heading.to_degrees(), active: best.active });
        }
    }
    out
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
