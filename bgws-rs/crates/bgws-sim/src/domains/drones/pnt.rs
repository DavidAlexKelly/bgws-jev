//! Position, navigation and timing (§8.7 PNT).
//!
//! - Every land unit carries a GNSS receiver (`bgws_ops::pnt::GnssTracker`)
//!   and an INS of its grade. Every 10 s the J at its antenna from every
//!   jammer covering L1 (through terrain path loss) sets its C/N₀; it loses
//!   lock below the tracking threshold and reacquires with hysteresis. A
//!   scripted theatre GNSS jam adds a floor of J for a side.
//! - Without a fix, the INS drifts from the last good one (`NavState`): the
//!   unit's self-position error grows, and goes into every position it
//!   reports (`comms`) and into what it believes of itself.
//! - [`nav_error_m`] gives the fires and air modules a unit's self-position
//!   error; [`precision_accuracy`] the CEP of a GNSS-guided round from the
//!   jamming at the gun and along the trajectory (Excalibur under jamming
//!   **[P15]**).

use super::{alive_id, pos_of, side_of, C4};
use crate::components::*;
use crate::sim::Sim;
use bgws_core::{Enu, EntityId, Side, SimTime};
use bgws_ops::ew::combined_jamming_dbw;
use bgws_ops::pnt::{GnssParams, GnssTracker, InsModel, MunitionAccuracy, MunitionNav, NavState, SelfPosition, CEP_PER_SIGMA};
use bgws_prop::{terrain_path_loss, Terminal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One unit's navigation.
#[derive(Clone, Debug)]
pub struct NavRec {
    pub tracker: GnssTracker,
    pub nav: NavState,
    pub has_fix: bool,
    pub cn0: f64,
}

/// What a gun does about GNSS-guided rounds while GNSS is denied (`ew.*` answers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PntPolicy {
    FireAnyway,
    SwitchMunition,
    Wait,
    DroneGuidance,
}

#[derive(Default)]
pub struct PntState {
    pub recs: BTreeMap<EntityId, NavRec>,
    /// A scripted theatre GNSS jam: (J dBW, until).
    pub side_jam: BTreeMap<Side, (f64, Option<SimTime>)>,
    pub policy: BTreeMap<EntityId, PntPolicy>,
}

pub fn setup(s: &mut Sim, c4: &mut C4) {
    let gp = GnssParams::from_figures(&s.ctx.figures);
    for (id, r) in &c4.roles {
        let Some(grade) = r.ins else { continue };
        let ins = InsModel::from_figures(grade, &s.ctx.figures);
        c4.pnt.recs.insert(*id, NavRec { tracker: GnssTracker::new(gp.clone(), SimTime::ZERO), nav: NavState::new(ins, SimTime::ZERO, c4.p.gnss_fix_sigma_m), has_fix: true, cn0: gp.cn0_nominal_dbhz });
    }
}

/// 0.1 Hz: every receiver against the jamming.
pub fn slow(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    let now = s.ctx.now;
    c4.pnt.side_jam.retain(|_, (_, until)| until.map(|u| u > now).unwrap_or(true));
    let any_gnss_jammer = c4.ew.jammers.iter().any(|j| j.jam.on && j.bands.iter().any(|b| b == "gnss"));
    let jammed_before = c4.pnt.recs.values().any(|r| !r.has_fix);
    if !any_gnss_jammer && c4.pnt.side_jam.is_empty() && !jammed_before {
        super::put(s, c4);
        return;
    }
    let ids: Vec<EntityId> = c4.pnt.recs.keys().copied().filter(|id| alive_id(s, *id)).collect();
    let victims: Vec<super::ew::Victim> = ids
        .iter()
        .filter_map(|id| Some(super::ew::Victim { key: id.0 as u64, pos: pos_of(s, *id)?, agl: 2.0, side: side_of(s, *id)?, freq_hz: c4.p.l1_hz, bw_hz: c4.p.jam_width_gnss, gnss: true, rx_gain_dbi: 0.0 }))
        .collect();
    let js = super::ew::jamming_dbw(s, &mut c4, &victims);
    let fix_sigma = c4.p.gnss_fix_sigma_m;
    let mut events = Vec::new();
    for (v, j) in victims.iter().zip(js) {
        let id = EntityId(v.key as u32);
        let floor = c4.pnt.side_jam.get(&v.side).map(|x| x.0).unwrap_or(f64::NEG_INFINITY);
        let j = combined_jamming_dbw([j, floor]);
        let Some(rec) = c4.pnt.recs.get_mut(&id) else { continue };
        let st = rec.tracker.update(now, j, None);
        rec.nav.update(&st, fix_sigma);
        if st.has_fix != rec.has_fix {
            events.push((id, v.side, st.has_fix, st.cn0_dbhz));
        }
        rec.has_fix = st.has_fix;
        rec.cn0 = st.cn0_dbhz;
    }
    super::put(s, c4);
    for (id, side, fix, cn0) in events {
        let text = format!("{} {} GNSS (C/N₀ {:.0} dB-Hz)", crate::systems::name(s, id), if fix { "regains" } else { "loses" }, cn0);
        s.ctx.log.push(now, if fix { "pnt.regained" } else { "pnt.lost" }).side(side).entity(id).text(text).data(serde_json::json!({ "cn0": cn0 }));
    }
}

/// A unit's self-position estimate: covariance, CEP and any spoofed bias.
pub fn self_position(s: &Sim, e: bevy_ecs::prelude::Entity) -> Option<SelfPosition> {
    let c4 = s.world.get_resource::<C4>()?;
    let id = s.world.get::<Ident>(e)?.id;
    Some(c4.pnt.recs.get(&id)?.nav.self_position(s.ctx.now))
}

/// A unit's self-position error, m: its CEP plus any bias. A unit with no
/// navigation record has a good GNSS fix.
pub fn nav_error_m(s: &Sim, e: bevy_ecs::prelude::Entity) -> f64 {
    match self_position(s, e) {
        Some(sp) => sp.cep_m + sp.bias.norm(),
        None => CEP_PER_SIGMA * s.ctx.figures.get("pnt.gnss.fix_sigma_m"),
    }
}

/// Is GNSS denied at a point (J above the loss-of-lock threshold)?
fn denied_at(s: &Sim, c4: &C4, p: Enu, agl: f64, side: Option<Side>) -> bool {
    let gp = GnssParams::from_figures(&s.ctx.figures);
    let mut levels = Vec::new();
    for j in &c4.ew.jammers {
        if !j.jam.on || !j.bands.iter().any(|b| b == "gnss") {
            continue;
        }
        let Some(jp) = pos_of(s, j.entity) else { continue };
        let pl = terrain_path_loss(&s.ctx.ground.terrain, Terminal { e: jp.e, n: jp.n, agl: j.agl }, Terminal { e: p.e, n: p.n, agl: agl.max(0.5) }, c4.p.l1_hz, &c4.radio).total_db;
        levels.push(j.jam.at_receiver(c4.p.l1_hz, c4.p.jam_width_gnss, pl, 0.0, 0.0).j_dbw);
    }
    if let Some(side) = side {
        if let Some((floor, _)) = c4.pnt.side_jam.get(&side) {
            levels.push(*floor);
        }
    }
    let j = combined_jamming_dbw(levels);
    gp.effective_cn0(j) < gp.track_dbhz
}

/// The accuracy of a GNSS-guided round fired from `from` at `to` with this
/// flight time: denied at the gun, it flies on its launch error and INS;
/// denied over the target, it loses GNSS for the rest of its flight.
pub fn precision_accuracy(s: &Sim, from: Enu, to: Enu, flight_time_s: f64, side: Option<Side>) -> MunitionAccuracy {
    let f = &s.ctx.figures;
    let Some(c4) = s.world.get_resource::<C4>() else {
        return MunitionNav::guided_shell(f, flight_time_s, None).accuracy();
    };
    if denied_at(s, c4, from, 2.0, side) {
        return MunitionNav::guided_shell(f, flight_time_s, Some(flight_time_s)).accuracy();
    }
    // Along the trajectory (a parabola, apex from the range), the first denied point.
    let range = to.sub(from).horizontal();
    let apex = (range * 0.25).clamp(200.0, 15_000.0);
    let n = 20;
    let mut lost = None;
    for k in 1..=n {
        let x = k as f64 / n as f64;
        let p = from.add(to.sub(from).scale(x));
        let agl = 4.0 * apex * x * (1.0 - x) + 2.0;
        if denied_at(s, c4, p, agl, side) {
            lost = Some(flight_time_s * (1.0 - (k as f64 - 1.0) / n as f64));
            break;
        }
    }
    MunitionNav::guided_shell(f, flight_time_s, lost).accuracy()
}

/// The gun's choice about GNSS-guided rounds, if it has been asked.
pub fn policy(s: &Sim, e: bevy_ecs::prelude::Entity) -> Option<PntPolicy> {
    let c4 = s.world.get_resource::<C4>()?;
    let id = s.world.get::<Ident>(e)?.id;
    c4.pnt.policy.get(&id).copied()
}

/// Whether a unit has a GNSS fix now.
pub fn has_fix(s: &Sim, id: EntityId) -> bool {
    s.world.get_resource::<C4>().and_then(|c| c.pnt.recs.get(&id)).map(|r| r.has_fix).unwrap_or(true)
}
