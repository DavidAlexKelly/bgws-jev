//! Airspace control (§8.1, TS `world/airspace.ts`): the side's control
//! measures (`world.measures`: corridors, restricted operating zones, kill
//! boxes, with altitude bands) say where its own aircraft are expected.
//! An own aircraft outside every such measure (or outside its band) inside
//! an active friendly AD zone is at risk: the AD unit's picture may take it
//! for hostile, and a fratricidal engagement follows. A side with no
//! airspace measures is assumed to fly under positive control (IFF only).
//!
//! [`air_superiority`] answers who holds the air over an area, for the
//! world's victory conditions.

use super::flight::{self, LaunchSpec, Purpose};
use super::{AdUnit, AirWorld, Craft, Wcs};
use crate::components::*;
use crate::sim::Sim;
use crate::systems::{alive, name};
use bgws_core::{EntityId, Enu, Geodetic, Side};
use bgws_scenario::v4::ControlMeasure;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Kinds of measure that clear own aircraft to fly.
const AIR_MEASURES: [&str; 3] = ["corridor", "roz", "killBox"];

fn active(m: &ControlMeasure, t: f64) -> bool {
    m.active_from.map(|f| t >= f).unwrap_or(true) && m.active_until.map(|u| t <= u).unwrap_or(true)
}

/// Is a point inside a polygon (ENU, even-odd rule)?
fn in_polygon(p: Enu, poly: &[Enu]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.n > p.n) != (b.n > p.n) && p.e < (b.e - a.e) * (p.n - a.n) / (b.n - a.n) + a.e {
            inside = !inside;
        }
    }
    inside
}

/// Is an aircraft at `p` (u = altitude) inside a measure?
fn inside(s: &Sim, m: &ControlMeasure, p: Enu) -> bool {
    if let Some(band) = m.alt_m {
        if p.u < band.min || p.u > band.max {
            return false;
        }
    }
    let pts: Vec<Enu> = m
        .points
        .iter()
        .map(|g| s.ctx.frame.geodetic_to_enu(*g))
        .collect();
    match pts.len() {
        0 => false,
        1 => pts[0].sub(p).horizontal() <= 5_000.0,
        2 => super::craft::seg_dist(p, pts[0], pts[1]) <= 5_000.0,
        _ if m.kind == "corridor" => pts
            .windows(2)
            .any(|w| super::craft::seg_dist(p, w[0], w[1]) <= 5_000.0),
        _ => in_polygon(p, &pts),
    }
}

/// Is an own aircraft where the side's airspace plan expects it? `None`
/// when the side has no air measures (positive control only).
pub fn in_plan(s: &Sim, side: Side, p: Enu) -> Option<bool> {
    let t = s.ctx.now.secs();
    let ms: Vec<&ControlMeasure> = s
        .ctx
        .scenario
        .world
        .measures
        .iter()
        .filter(|m| m.side == side && AIR_MEASURES.contains(&m.kind.as_str()) && active(m, t))
        .collect();
    if ms.is_empty() {
        return None;
    }
    Some(ms.iter().any(|m| inside(s, m, p)))
}

/// Fratricide risk: own aircraft outside the plan in an active AD zone.
pub fn control(s: &mut Sim, aw: &mut AirWorld) {
    if !aw.ad_on || !aw.air_on {
        return;
    }
    let now = s.ctx.now;
    let rate = aw.p.misid_per_min / 60.0;
    for &a in &aw.crafts.clone() {
        let ae = s.ctx.entity(a);
        let Some(c) = s.world.get::<Craft>(ae) else {
            continue;
        };
        if !c.airborne() || !alive(s, ae) {
            continue;
        }
        let side = s.world.get::<Ident>(ae).expect("i").side;
        let pos = s.world.get::<Kin>(ae).expect("k").pos;
        if in_plan(s, side, pos) != Some(false) {
            continue;
        }
        for &u in &aw.ad_units.clone() {
            let ue = s.ctx.entity(u);
            let Some(unit) = s.world.get::<AdUnit>(ue) else {
                continue;
            };
            if s.world.get::<Ident>(ue).map(|i| i.side) != Some(side)
                || !alive(s, ue)
                || unit.wcs != Wcs::Free
                || !super::defence::radiating(s, ue)
            {
                continue;
            }
            let up = s.world.get::<Kin>(ue).expect("k").pos;
            let Some(m) = unit
                .mounts
                .iter()
                .filter(|m| {
                    m.ready_rounds > 0
                        && m.sys
                            .pk_against(bgws_domains::air_defence::ThreatClass::Aircraft)
                            > 0.0
                })
                .find(|m| pos.sub(up).norm() <= m.max_range)
                .cloned()
            else {
                continue;
            };
            let mut rng = s.ctx.rng("airspace.misid", u.0 as u64, a.0 as u64);
            if rng.gen::<f64>() >= bgws_core::hazard_probability(rate, 1.0) {
                continue;
            }
            aw.misid.insert((u, a), now);
            let pk = m
                .sys
                .pk_against(bgws_domains::air_defence::ThreatClass::Aircraft)
                * aw.p.fratricide_pk;
            flight::launch(
                s,
                aw,
                LaunchSpec {
                    side,
                    launcher: Some(u),
                    munition: m.munition.clone(),
                    from: up,
                    to: pos,
                    target: Some(a),
                    purpose: Purpose::Intercept,
                    track: None,
                    pk,
                },
            );
            if let Some(mut unit) = s.world.get_mut::<AdUnit>(ue) {
                if let Some(mm) = unit.mounts.iter_mut().find(|x| x.mount == m.mount) {
                    mm.ready_rounds = mm.ready_rounds.saturating_sub(1);
                }
            }
            aw.ledger.entry(side).or_default().fratricide += 1;
            let text = format!(
                "{} takes {} for hostile and fires: it is outside the airspace plan",
                name(s, u),
                name(s, a)
            );
            s.ctx
                .log
                .push(now, "airspace.fratricide")
                .side(side)
                .entity(u)
                .target(a)
                .text(text)
                .data(serde_json::json!({ "pk": pk, "alt": pos.u }));
            break;
        }
    }
}

/// Who holds the air over an area, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirSuperiority {
    pub holder: Option<Side>,
    /// Airborne combat aircraft over the area, by side.
    pub aircraft: BTreeMap<Side, u32>,
    /// Air-defence units whose missiles reach the area, by side.
    pub ad_cover: BTreeMap<Side, u32>,
    pub reason: String,
}

/// Air superiority over a circle: a side holds it when it has combat
/// aircraft over the area or air defence covering it, and the enemy has
/// neither. Contested or empty air: nobody holds it.
pub fn air_superiority(s: &Sim, at: Geodetic, radius_m: f64) -> AirSuperiority {
    let centre = s.ctx.frame.geodetic_to_enu(at);
    let mut aircraft: BTreeMap<Side, u32> = BTreeMap::new();
    let mut ad: BTreeMap<Side, u32> = BTreeMap::new();
    let Some(aw) = super::state(s) else {
        return AirSuperiority {
            holder: None,
            aircraft,
            ad_cover: ad,
            reason: "no air domain".into(),
        };
    };
    for &id in &aw.crafts {
        let e = s.ctx.entity(id);
        let (Some(c), Some(i), Some(k)) = (
            s.world.get::<Craft>(e),
            s.world.get::<Ident>(e),
            s.world.get::<Kin>(e),
        ) else {
            continue;
        };
        let armed = super::craft::mount_of(s, e, &[bgws_catalogue::WeaponKind::Aam]).is_some();
        if c.airborne() && alive(s, e) && armed && k.pos.sub(centre).horizontal() <= radius_m {
            *aircraft.entry(i.side).or_default() +=
                s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
        }
    }
    for &id in &aw.ad_units {
        let e = s.ctx.entity(id);
        let (Some(u), Some(i), Some(k)) = (
            s.world.get::<AdUnit>(e),
            s.world.get::<Ident>(e),
            s.world.get::<Kin>(e),
        ) else {
            continue;
        };
        if !alive(s, e) {
            continue;
        }
        let reach = u
            .mounts
            .iter()
            .filter(|m| {
                m.sys
                    .pk_against(bgws_domains::air_defence::ThreatClass::Aircraft)
                    > 0.0
            })
            .map(|m| m.max_range)
            .fold(0.0, f64::max);
        let stocked = s
            .world
            .get::<Arms>(e)
            .map(|a| a.rounds() > 0)
            .unwrap_or(false);
        if stocked && reach > 0.0 && k.pos.sub(centre).horizontal() <= reach + radius_m {
            *ad.entry(i.side).or_default() += 1;
        }
    }
    let present =
        |sd: Side| aircraft.get(&sd).copied().unwrap_or(0) + ad.get(&sd).copied().unwrap_or(0) > 0;
    let holder = Side::PLAYERS
        .into_iter()
        .find(|sd| present(*sd) && !present(sd.enemy()));
    let reason = match holder {
        Some(sd) => format!(
            "{sd} has {} aircraft and {} AD units over the area; the enemy has none",
            aircraft.get(&sd).copied().unwrap_or(0),
            ad.get(&sd).copied().unwrap_or(0)
        ),
        None if Side::PLAYERS.iter().all(|sd| present(*sd)) => "contested".into(),
        None => "nobody flies or defends there".into(),
    };
    AirSuperiority {
        holder,
        aircraft,
        ad_cover: ad,
        reason,
    }
}

/// Own aircraft taken for hostile so far: (AD unit, aircraft).
pub fn misidentifications(s: &Sim) -> Vec<(EntityId, EntityId)> {
    super::state(s)
        .map(|a| a.misid.keys().copied().collect())
        .unwrap_or_default()
}
