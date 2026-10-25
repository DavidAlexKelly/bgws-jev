//! Electronic warfare (§8.7, §8.3, §8.2 ESM).
//!
//! - **Jammers are emitters** (`bgws_ops::ew::Jammer`): the catalogue's
//!   jammer ERP split across the bands it jams, each band a channel as wide
//!   as the victims it works against (a responsive spot on a VHF net, the
//!   drone links' video band, GNSS L1), with a duty cycle and look-through.
//!   The J at each receiver comes through `bgws_prop` terrain path loss, so
//!   terrain shields and range matter; own radios and drone links are
//!   shielded by frequency planning, own GNSS is not.
//! - **ESM** (`bgws_models::sensing::esm`): every emission this second —
//!   radio transmissions from the nets, jammers, radars, drone ground
//!   stations — is heard by enemy listeners in range and in band at a rate
//!   from the received power; each intercept adds a bearing with an error
//!   from the DF floor and SNR. Bearings from places far enough apart are
//!   fixed by `bgws_know::df::triangulate`; a fix good enough to strike is
//!   reported over the nets as a detection and the EW team is asked.
//! - **EMCON and exposure:** silent units don't transmit; units on the air
//!   long enough are asked whether to move or go silent.

use super::{alive_id, pos_of, push_trigger, side_of, LossQuery, QCtx, C4};
use crate::components::*;
use crate::sim::Sim;
use bgws_core::{hazard_probability, Enu, EntityId, Side, SimTime};
use bgws_know::df::{triangulate, BearingLine};
use bgws_know::{ClassEvidence, Detection, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::sensing::esm::{df_bearing_sigma, intercept, Emitter as EsmEmitter, EmitterScan, EsmParams, Receiver as EsmReceiver};
use bgws_ops::ew::{combined_jamming_dbw, AntennaPattern, EwParams, JamBand, Jammer};
use bgws_scenario::v4::Emcon;
use nalgebra::{Matrix2, Matrix3};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A jammer and whose it is.
#[derive(Clone, Debug)]
pub struct JammerRec {
    pub entity: EntityId,
    pub side: Side,
    pub jam: Jammer,
    pub bands: Vec<String>,
    pub agl: f64,
    /// The catalogue's nominal radius (for moving into reach).
    pub radius_m: f64,
}

/// One emission this second.
#[derive(Clone, Debug)]
pub struct TxEmission {
    pub entity: EntityId,
    pub side: Side,
    pub freq_hz: f64,
    pub bw_hz: f64,
    pub erp_w: f64,
    pub on_air_s: f64,
    pub duty: f64,
    pub what: &'static str,
}

#[derive(Clone, Debug)]
pub struct BearingObs {
    pub origin: Enu,
    pub bearing: f64,
    pub sigma: f64,
    pub time: SimTime,
    pub listener: EntityId,
}

/// A side's DF fix on an emitter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixRec {
    pub at: Enu,
    pub semi_major_m: f64,
    pub time: SimTime,
    pub lines: usize,
    pub what: String,
    pub first_heard: SimTime,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EwStats {
    pub intercepts: u32,
    pub fixes: u32,
    /// First intercept to a strike-quality fix, s.
    pub time_to_fix_s: Vec<f64>,
}

#[derive(Default)]
pub struct EwState {
    pub jammers: Vec<JammerRec>,
    pub tx: Vec<TxEmission>,
    pub bearings: BTreeMap<(Side, EntityId), Vec<BearingObs>>,
    pub fixes: BTreeMap<(Side, EntityId), FixRec>,
    pub first_heard: BTreeMap<(Side, EntityId), SimTime>,
    pub exposed_asked: BTreeMap<EntityId, SimTime>,
    pub jam_asked: BTreeMap<EntityId, SimTime>,
    pub pnt_asked: BTreeMap<EntityId, SimTime>,
    pub stats: EwStats,
    pub esm: Option<EsmParams>,
    pub ew: Option<EwParams>,
}

/// A receiver the jammers may reach.
#[derive(Clone, Copy, Debug)]
pub struct Victim {
    pub key: u64,
    pub pos: Enu,
    pub agl: f64,
    pub side: Side,
    pub freq_hz: f64,
    pub bw_hz: f64,
    /// GNSS: no frequency planning shields it from own jammers.
    pub gnss: bool,
    pub rx_gain_dbi: f64,
}

/// The jamming band centre a jammer aims at, by band id.
fn band_centre(c4: &C4, s: &Sim, band: &str) -> f64 {
    match band {
        "gnss" => c4.p.l1_hz,
        "l" => c4.p.drone_link_hz,
        _ => s.ctx.catalogue.band(band).map(|b| (b.f_min_hz * b.f_max_hz).sqrt()).unwrap_or(1e8),
    }
}

pub fn setup(s: &mut Sim, c4: &mut C4) {
    let ewp = EwParams::from_figures(&s.ctx.figures);
    c4.ew.esm = Some(EsmParams::from_figures(&s.ctx.figures));
    if super::mode(s, bgws_scenario::scope::ScopeDomain::Ew) != bgws_scenario::scope::DomainMode::Off {
        let mut jams = Vec::new();
        for (id, r) in &c4.roles {
            let Some(e) = &r.jammer else { continue };
            let Some(side) = side_of(s, *id) else { continue };
            let bands: Vec<JamBand> = e.jams.iter().map(|b| JamBand { centre_hz: band_centre(c4, s, b), width_hz: c4.p.jam_width(b), weight: 1.0 }).collect();
            let mut jam = Jammer::new(e.erp_w.max(0.0), AntennaPattern::Omni { gain_dbi: 0.0 }, bands, &ewp);
            jam.duty_cycle = e.duty_cycle.clamp(0.0, 1.0);
            jam.on = c4.emcon_of(*id) == Emcon::Free;
            let agl = r.radios.first().map(|x| x.agl).unwrap_or(c4.p.ground_antenna_m).max(5.0);
            jams.push(JammerRec { entity: *id, side, jam, bands: e.jams.clone(), agl, radius_m: e.jam_radius_m.max(1000.0) });
        }
        c4.ew.jammers = jams;
    }
    c4.ew.ew = Some(ewp);
}

/// The J (dBW) at each victim from every jammer that is on, in one batch.
pub fn jamming_dbw(s: &Sim, c4: &mut C4, victims: &[Victim]) -> Vec<f64> {
    let mut qs = Vec::new();
    let mut who: Vec<(usize, usize)> = Vec::new();
    for (vi, v) in victims.iter().enumerate() {
        for (ji, j) in c4.ew.jammers.iter().enumerate() {
            if !j.jam.on || j.entity.0 as u64 == v.key || !j.jam.bands.iter().any(|b| b.covers(v.freq_hz, v.bw_hz)) {
                continue;
            }
            let Some(jp) = pos_of(s, j.entity) else { continue };
            qs.push(LossQuery { a: j.entity.0 as u64, b: v.key, f_hz: v.freq_hz, pa: jp, agl_a: j.agl, pb: v.pos, agl_b: v.agl });
            who.push((vi, ji));
        }
    }
    let mut out = vec![Vec::new(); victims.len()];
    if !qs.is_empty() {
        let losses = c4.losses.resolve(&s.ctx.ground.terrain, &c4.radio, c4.p.move_tolerance_m, s.ctx.now, &qs);
        for ((vi, ji), l) in who.into_iter().zip(losses) {
            let v = &victims[vi];
            let j = &c4.ew.jammers[ji];
            let mut jdb = j.jam.at_receiver(v.freq_hz, v.bw_hz, l, 0.0, v.rx_gain_dbi).j_dbw;
            if j.side == v.side && !v.gnss {
                jdb -= c4.p.own_deconfliction_db;
            }
            out[vi].push(jdb);
        }
    }
    out.into_iter().map(combined_jamming_dbw).collect()
}

/// A point `dist` away from what threatens a unit (its enemy's side of the field).
pub fn point_away(s: &Sim, id: EntityId, dist: f64) -> Enu {
    let me = pos_of(s, id).unwrap_or_default();
    let side = side_of(s, id).unwrap_or(Side::Blue);
    let enemy: Vec<Enu> = s.ctx.scenario.forces_of(side.enemy()).map(|f| s.ctx.frame.geodetic_to_enu(f.position)).collect();
    let dir = if enemy.is_empty() {
        Enu::new(0.0, if side == Side::Blue { -1.0 } else { 1.0 }, 0.0)
    } else {
        let c = enemy.iter().fold(Enu::default(), |a, b| a.add(*b)).scale(1.0 / enemy.len() as f64);
        let d = me.sub(c);
        let n = d.horizontal().max(1.0);
        Enu::new(d.e / n, d.n / n, 0.0)
    };
    me.add(dir.scale(dist))
}

fn gauss(rng: &mut impl Rng) -> f64 {
    let u1: f64 = rng.gen::<f64>().max(1e-12);
    let u2: f64 = rng.gen();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// 1 Hz: jammers on or off, continuous emitters, ESM intercepts and fixes,
/// exposure and jamming questions.
pub fn base(s: &mut Sim, dt: f64) {
    let Some(mut c4) = super::take(s) else { return };
    let now = s.ctx.now;
    let ew_on = super::mode(s, bgws_scenario::scope::ScopeDomain::Ew) != bgws_scenario::scope::DomainMode::Off;
    // Jammers follow EMCON and life.
    for j in 0..c4.ew.jammers.len() {
        let id = c4.ew.jammers[j].entity;
        let on = alive_id(s, id) && c4.may_emit(id);
        c4.ew.jammers[j].jam.on = on;
    }
    let mut tx = std::mem::take(&mut c4.ew.tx);
    // Continuous emitters.
    for j in &c4.ew.jammers {
        if !j.jam.on {
            continue;
        }
        let split = 1.0 / j.jam.bands.len().max(1) as f64;
        for b in &j.jam.bands {
            tx.push(TxEmission { entity: j.entity, side: j.side, freq_hz: b.centre_hz, bw_hz: b.width_hz, erp_w: j.jam.power_w * split, on_air_s: dt, duty: j.jam.duty_cycle, what: "jammer" });
        }
    }
    let ids: Vec<EntityId> = c4.roles.keys().copied().collect();
    for id in ids {
        if !alive_id(s, id) {
            continue;
        }
        let Some(side) = side_of(s, id) else { continue };
        let role = &c4.roles[&id];
        let mut add: Vec<TxEmission> = Vec::new();
        if c4.may_emit(id) {
            for (f, erp, duty) in &role.radars {
                add.push(TxEmission { entity: id, side, freq_hz: *f, bw_hz: 1e6, erp_w: *erp, on_air_s: dt, duty: *duty, what: "radar" });
            }
        }
        if let Some((f, erp, bw)) = role.drone_link {
            if c4.drones.flights.values().any(|fl| fl.team == id && fl.spec.jammable() && !fl.link.lost) {
                add.push(TxEmission { entity: id, side, freq_hz: f, bw_hz: bw, erp_w: erp, on_air_s: dt, duty: 1.0, what: "drone link" });
            }
        }
        for a in add {
            c4.add_on_air(id, a.on_air_s * a.duty);
            tx.push(a);
        }
    }
    for j in &c4.ew.jammers {
        if j.jam.on {
            *c4.on_air.entry(j.entity).or_insert(0.0) += dt;
        }
    }
    if ew_on {
        esm(s, &mut c4, &tx);
        if now.0 % 10_000_000 == 0 {
            exposure(s, &mut c4);
            jamming_orders(s, &mut c4);
            pnt_questions(s, &mut c4);
        }
    } else if now.0 % 10_000_000 == 0 {
        exposure(s, &mut c4);
    }
    super::put(s, c4);
}

fn esm(s: &mut Sim, c4: &mut C4, tx: &[TxEmission]) {
    let now = s.ctx.now;
    let Some(ep) = c4.ew.esm.clone() else { return };
    // Listeners.
    let listeners: Vec<(EntityId, Side, Enu, f64, f64, bgws_catalogue::EsmParams)> = c4
        .roles
        .iter()
        .filter_map(|(id, r)| {
            let (range, p, h) = r.esm?;
            if !alive_id(s, *id) {
                return None;
            }
            Some((*id, side_of(s, *id)?, pos_of(s, *id)?, h, range, p))
        })
        .collect();
    if listeners.is_empty() || tx.is_empty() {
        return;
    }
    let mut qs = Vec::new();
    let mut pairs = Vec::new();
    for (li, l) in listeners.iter().enumerate() {
        for (ti, t) in tx.iter().enumerate() {
            if t.side == l.1 || t.side == Side::Neutral || t.freq_hz < l.5.freq_min_hz || t.freq_hz > l.5.freq_max_hz {
                continue;
            }
            let Some(tp) = pos_of(s, t.entity) else { continue };
            if tp.sub(l.2).horizontal() > l.4 {
                continue;
            }
            let agl = c4.roles.get(&t.entity).and_then(|r| r.radios.first()).map(|r| r.agl).unwrap_or(c4.p.ground_antenna_m);
            qs.push(LossQuery { a: l.0 .0 as u64, b: t.entity.0 as u64, f_hz: t.freq_hz, pa: l.2, agl_a: l.3, pb: tp, agl_b: agl });
            pairs.push((li, ti, tp));
        }
    }
    if qs.is_empty() {
        return;
    }
    let losses = c4.losses.resolve(&s.ctx.ground.terrain, &c4.radio, c4.p.move_tolerance_m, now, &qs);
    let mut heard: BTreeSet<(Side, EntityId)> = BTreeSet::new();
    for ((li, ti, tp), pl) in pairs.into_iter().zip(losses) {
        let l = &listeners[li];
        let t = &tx[ti];
        let em = EsmEmitter { erp_dbw: 10.0 * t.erp_w.max(1e-9).log10(), freq_hz: t.freq_hz, duty: t.duty, scan: EmitterScan::Omni, sidelobe_db: None };
        let rx = EsmReceiver { sensitivity_dbw: l.5.sensitivity_dbm - 30.0, gain_dbi: l.5.gain_dbi, band_lo_hz: l.5.freq_min_hz, band_hi_hz: l.5.freq_max_hz };
        let ic = intercept(&em, &rx, pl, &ep);
        if ic.rate_per_s <= 0.0 {
            continue;
        }
        let p = hazard_probability(ic.rate_per_s, t.on_air_s);
        let mut rng = s.ctx.rng("esm.intercept", l.0 .0 as u64, ((t.entity.0 as u64) << 20) ^ (t.freq_hz / 1e3) as u64);
        if rng.gen::<f64>() >= p || heard.contains(&(l.1, t.entity)) {
            continue;
        }
        heard.insert((l.1, t.entity));
        let truth = tp.sub(l.2).bearing();
        let sigma = df_bearing_sigma(t.freq_hz, 2.0, ic.margin_main_db, &ep).max(l.5.df_rms_deg.to_radians());
        let bearing = truth + sigma * gauss(&mut rng);
        c4.ew.stats.intercepts += 1;
        let key = (l.1, t.entity);
        if let std::collections::btree_map::Entry::Vacant(v) = c4.ew.first_heard.entry(key) {
            v.insert(now);
            let text = format!("{} hears a {} {:.0} km to the {}", crate::systems::name(s, l.0), t.what, tp.sub(l.2).horizontal() / 1000.0, crate::decide::compass(bearing));
            s.ctx.log.push(now, "ew.intercept").side(l.1).entity(l.0).target(t.entity).text(text).data(serde_json::json!({ "what": t.what, "freqHz": t.freq_hz, "marginDb": ic.margin_main_db, "sigmaDeg": sigma.to_degrees() }));
        }
        let list = c4.ew.bearings.entry(key).or_default();
        list.push(BearingObs { origin: l.2, bearing, sigma, time: now, listener: l.0 });
        let window = c4.p.bearing_window_s;
        list.retain(|b| (now - b.time).secs() <= window);
        if list.len() > 24 {
            list.remove(0);
        }
        try_fix(s, c4, key, t.what);
    }
}

fn try_fix(s: &mut Sim, c4: &mut C4, key: (Side, EntityId), what: &str) {
    let now = s.ctx.now;
    let list = &c4.ew.bearings[&key];
    // Lines from places at least a baseline apart (the newest from each place).
    let mut lines: Vec<BearingLine> = Vec::new();
    let mut origins: Vec<Enu> = Vec::new();
    for b in list.iter().rev() {
        if origins.iter().any(|o| o.sub(b.origin).horizontal() < c4.p.min_baseline_m) {
            continue;
        }
        origins.push(b.origin);
        lines.push(BearingLine { origin: b.origin, bearing: b.bearing, sigma: b.sigma });
    }
    if lines.len() < 2 {
        return;
    }
    let Some(fix) = triangulate(&lines, c4.p.min_crossing_rad, c4.p.max_fix_range_m) else { return };
    let ell = bgws_know::Ellipse::from_cov(&Matrix2::new(fix.cov[(0, 0)], fix.cov[(0, 1)], fix.cov[(1, 0)], fix.cov[(1, 1)]));
    if ell.semi_major > c4.p.fix_target_m {
        return;
    }
    let prev = c4.ew.fixes.get(&key);
    if prev.map(|p| (now - p.time).secs() < 600.0 && p.semi_major_m <= ell.semi_major * 2.0).unwrap_or(false) {
        return;
    }
    let first = c4.ew.first_heard.get(&key).copied().unwrap_or(now);
    let at = Enu::new(fix.pos.x, fix.pos.y, s.ctx.ground.height(fix.pos.x, fix.pos.y));
    c4.ew.fixes.insert(key, FixRec { at, semi_major_m: ell.semi_major, time: now, lines: fix.lines, what: what.to_string(), first_heard: first });
    c4.ew.stats.fixes += 1;
    c4.ew.stats.time_to_fix_s.push((now - first).secs());
    let listener = list.last().map(|b| b.listener).expect("lines");
    let (side, emitter) = key;
    let text = format!("{} fixes a {} to {:.0} m from {} bearings, {:.0} s after first hearing it", crate::systems::name(s, listener), what, ell.semi_major, fix.lines, (now - first).secs());
    s.ctx.log.push(now, "ew.fix").side(side).entity(listener).target(emitter).text(text).data(serde_json::json!({ "errorM": ell.semi_major, "lines": fix.lines, "crossingDeg": fix.crossing_angle.to_degrees(), "afterS": (now - first).secs(), "what": what }));
    // The fix is a detection: into the side's picture over the nets.
    let det = Detection {
        time: now,
        observer: listener,
        sensor: SensorKind::Esm,
        domain: bgws_know::Domain::Land,
        motion: MotionHint::Static,
        measurement: Measurement::Position { pos: at, cov: Matrix3::new(fix.cov[(0, 0)], fix.cov[(0, 1)], 0.0, fix.cov[(1, 0)], fix.cov[(1, 1)], 0.0, 0.0, 0.0, 100.0) },
        class: ClassEvidence { level: IdLevel::Detect, reported: None, resolves_decoys: false },
        truth: TruthTag::of(emitter),
    };
    super::comms::queue_detection(s, c4, listener, side, det);
    push_trigger(s, c4, listener, "emitterDetected", None, QCtx { emitter: Some(emitter), at: Some(at), ..Default::default() });
}

/// Units on the air too long are asked (posts, drone teams and others by their own trigger).
fn exposure(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let ids: Vec<(EntityId, f64)> = c4.on_air.iter().map(|(k, v)| (*k, *v)).collect();
    for (id, t) in ids {
        if !alive_id(s, id) {
            continue;
        }
        let (limit, point): (f64, &'static str) = if c4.c2.post_of(id).is_some() {
            (c4.p.emit_limit_s, "emittingTooLong")
        } else if c4.roles.get(&id).map(|r| r.team.is_some()).unwrap_or(false) {
            (c4.p.operator_exposed_s, "operatorLocated")
        } else {
            (c4.p.expose_s, "ownEmissionsExposed")
        };
        if t < limit || c4.emcon_of(id) == Emcon::Silent {
            continue;
        }
        // An air-defence unit with hostile air tracks inbound is fighting, not
        // loitering on the air: it is not asked to relocate mid-raid.
        if air_threat_near(s, id) {
            continue;
        }
        if c4.ew.exposed_asked.get(&id).map(|x| (now - *x).secs() < 900.0).unwrap_or(false) {
            continue;
        }
        c4.ew.exposed_asked.insert(id, now);
        let text = format!("{} has been on the air {:.0} min", crate::systems::name(s, id), t / 60.0);
        let sd = side_of(s, id).unwrap_or(Side::Neutral);
        s.ctx.log.push(now, "ew.exposed").side(sd).entity(id).text(text).data(serde_json::json!({ "onAirS": t, "trigger": point }));
        push_trigger(s, c4, id, point, None, QCtx::default());
    }
}

/// Does the side's picture hold an air track within the unit's AD reach?
fn air_threat_near(s: &Sim, id: EntityId) -> bool {
    let e = s.ctx.entity(id);
    if s.world.get::<crate::domains::air::defence::AdUnit>(e).is_none() {
        return false;
    }
    let (Some(side), Some(k)) = (side_of(s, id), s.world.get::<crate::components::Kin>(e)) else { return false };
    let reach = s.ctx.p.ad_quiet_reach_m;
    s.ctx.tracks.get(&side).map(|st| st.views(&s.ctx.p.know).iter().any(|t| t.domain == bgws_know::Domain::Air && t.position.sub(k.pos).horizontal() <= reach)).unwrap_or(false)
}

fn jamming_orders(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    for j in 0..c4.ew.jammers.len() {
        let id = c4.ew.jammers[j].entity;
        if !alive_id(s, id) {
            continue;
        }
        if c4.ew.jam_asked.get(&id).map(|x| (now - *x).secs() < c4.p.jam_review_s).unwrap_or(false) {
            continue;
        }
        c4.ew.jam_asked.insert(id, now);
        push_trigger(s, c4, id, "jammingOrder", None, QCtx::default());
    }
}

/// A gun whose GNSS is lost is asked what to do about its precision rounds.
fn pnt_questions(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let guns: Vec<EntityId> = c4.roles.iter().filter(|(_, r)| r.artillery).map(|(k, _)| *k).collect();
    for id in guns {
        let lost = c4.pnt.recs.get(&id).map(|r| !r.has_fix).unwrap_or(false);
        if !lost || !alive_id(s, id) || c4.ew.pnt_asked.get(&id).map(|x| (now - *x).secs() < 900.0).unwrap_or(false) {
            continue;
        }
        c4.ew.pnt_asked.insert(id, now);
        push_trigger(s, c4, id, "pntDegraded", None, QCtx::default());
    }
}

pub fn slow(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    let now = s.ctx.now;
    let w = c4.p.bearing_window_s;
    for v in c4.ew.bearings.values_mut() {
        v.retain(|b| (now - b.time).secs() <= w);
    }
    c4.ew.bearings.retain(|_, v| !v.is_empty());
    super::put(s, c4);
}

/// The EW counters (AAR, tests).
pub fn stats(s: &Sim) -> Option<EwStats> {
    s.world.get_resource::<C4>().map(|c| c.ew.stats.clone())
}

/// A side's fixes (AAR, tests).
pub fn fixes(s: &Sim, side: Side) -> Vec<(EntityId, FixRec)> {
    s.world.get_resource::<C4>().map(|c| c.ew.fixes.iter().filter(|(k, _)| k.0 == side).map(|(k, v)| (k.1, v.clone())).collect()).unwrap_or_default()
}

/// The nearest friendly gun or mortar to a point.
pub(crate) fn nearest_guns(s: &Sim, c4: &C4, side: Side, at: Enu) -> Option<EntityId> {
    c4.roles
        .iter()
        .filter(|(id, r)| r.artillery && side_of(s, **id) == Some(side) && alive_id(s, **id))
        .filter_map(|(id, _)| Some((*id, pos_of(s, *id)?.sub(at).horizontal())))
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|x| x.0)
}

// ── Actions (ew.*) ──────────────────────────────────────────────────────

pub fn apply_action(s: &mut Sim, e: bevy_ecs::prelude::Entity, id: EntityId, action: &str, _about: Option<EntityId>) -> bool {
    let Some(mut c4) = super::take(s) else { return false };
    let q = c4.qctx.get(&id).cloned().unwrap_or_default();
    let side = side_of(s, id).unwrap_or(Side::Neutral);
    let mut move_to: Option<(Enu, MoveModeS)> = None;
    let handled = match action {
        "locate" | "ignore" | "carryOn" => true,
        "jam" | "jamSector" => {
            c4.emcon.insert(id, Emcon::Free);
            if let (Some(at), Some(j)) = (q.at, c4.ew.jammers.iter().find(|j| j.entity == id)) {
                if let Some(me) = pos_of(s, id) {
                    let d = at.sub(me).horizontal();
                    if d > j.radius_m * 0.6 {
                        move_to = Some((at.add(me.sub(at).scale(j.radius_m * 0.5 / d)), MoveModeS::Tactical));
                    }
                }
            }
            true
        }
        "reportForStrike" => {
            if let (Some(em), Some(at)) = (q.emitter, q.at) {
                if let Some(g) = nearest_guns(s, &c4, side, at) {
                    push_trigger(s, &mut c4, g, "fireRequestReceived", Some(em), QCtx { target: Some(em), at: Some(at), ..Default::default() });
                }
                let text = format!("{} reports an emitter fix for a strike", crate::systems::name(s, id));
                s.ctx.log.push(s.ctx.now, "ew.report").side(side).entity(id).target(em).text(text);
            }
            true
        }
        "goSilent" => {
            c4.emcon.insert(id, Emcon::Silent);
            c4.on_air.insert(id, 0.0);
            true
        }
        "switchBearer" => {
            c4.emcon.insert(id, Emcon::Limited);
            c4.on_air.insert(id, 0.0);
            true
        }
        "relocateAfter" => {
            c4.on_air.insert(id, 0.0);
            move_to = Some((point_away(s, id, c4.p.relocate_m), MoveModeS::March));
            true
        }
        "protectAdvance" => {
            c4.emcon.insert(id, Emcon::Free);
            // Up behind the friend nearest the enemy, a few kilometres back.
            let enemy: Vec<Enu> = s.ctx.scenario.forces_of(side.enemy()).map(|f| s.ctx.frame.geodetic_to_enu(f.position)).collect();
            if !enemy.is_empty() {
                let c = enemy.iter().fold(Enu::default(), |a, b| a.add(*b)).scale(1.0 / enemy.len() as f64);
                let lead = s.ctx.order.iter().filter_map(|&x| {
                    let i = s.world.get::<Ident>(x)?;
                    (i.side == side && i.id != id && i.domain == bgws_catalogue::Domain::Land && crate::systems::alive(s, x)).then(|| (i.id, s.world.get::<Kin>(x).map(|k| k.pos).unwrap_or_default()))
                }).min_by(|a, b| a.1.sub(c).horizontal().total_cmp(&b.1.sub(c).horizontal()).then(a.0.cmp(&b.0)));
                if let Some((_, lp)) = lead {
                    let d = lp.sub(c);
                    let n = d.horizontal().max(1.0);
                    move_to = Some((lp.add(d.scale(3000.0 / n)), MoveModeS::Tactical));
                }
            }
            true
        }
        "fireAnyway" | "switchMunition" | "waitForPnt" | "requestDroneGuidance" => {
            let p = match action {
                "fireAnyway" => super::pnt::PntPolicy::FireAnyway,
                "switchMunition" => super::pnt::PntPolicy::SwitchMunition,
                "waitForPnt" => super::pnt::PntPolicy::Wait,
                _ => super::pnt::PntPolicy::DroneGuidance,
            };
            c4.pnt.policy.insert(id, p);
            true
        }
        _ => false,
    };
    super::put(s, c4);
    if let Some((to, mode)) = move_to {
        let route = crate::decide::route_for(s, e, to);
        if let Some(mut a) = s.world.get_mut::<Act>(e) {
            a.activity = Activity::Move { route, mode, then: Some(Box::new(Activity::Hold)) };
        }
    }
    handled
}
