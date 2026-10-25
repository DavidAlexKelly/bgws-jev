//! Finding ships and submarines (§8.2 radar and sonar, §8.6 detection).
//!
//! - **Radar** (ships, coastal batteries, ports, aircraft with radars) against
//!   surface contacts: the radar range equation with Swerling-1 fluctuation
//!   and sea clutter by sea state (`bgws_models::sensing::radar::detect`),
//!   inside the 4/3-Earth radar horizon between mast and target heights.
//! - **Sonar** (submarines and ASW ships): the passive and active sonar
//!   equations (`bgws_models::sensing::sonar`) with transmission loss from
//!   `bgws_prop::acoustic::water_tl` through the region's sound-speed
//!   profile (layer, duct, convergence zones, shallow water) and ambient
//!   noise by sea state and own speed.
//!
//! Detections go to the side's track store as sea-domain detections after the
//! link delay; the contact decision points (`surfaceContact`, `sonarContact`,
//! `counterDetected`) follow from the side's picture.

use super::fleet::{alive, radiated_db, ShipCombat, Vessel, VesselKind};
use super::{label, put, take};
use crate::components::*;
use crate::sim::Sim;
use bgws_catalogue::{Domain, SensorKind as CatSensor};
use bgws_core::{hazard_probability, Enu, EntityId, Side, SimDuration, StreamKey};
use bgws_domains::missile::radar_horizon;
use bgws_know::{ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::sensing::radar::{detect, ClutterBackground, RadarMode, RadarParams, RadarSpec, RadarTarget, Swerling};
use bgws_models::sensing::sonar::{active, noise_level, passive, ActiveLevels, Shipping, SonarParams};
use bgws_prop::acoustic::{water_tl, AcousticParams, WaterGeometry};
use nalgebra::Matrix3;
use rand::Rng;
use rayon::prelude::*;

/// What a sea contact is, as a sensor would report it.
fn hypothesis(s: &Sim, id: EntityId) -> Hypothesis {
    let e = s.ctx.entity(id);
    match s.world.get::<Vessel>(e).map(|v| v.kind) {
        Some(VesselKind::Submarine) => Hypothesis::Submarine,
        Some(VesselKind::Usv) | Some(VesselKind::SmallCraft) => Hypothesis::Usv,
        Some(VesselKind::Merchant) => Hypothesis::Merchant,
        _ => Hypothesis::Warship,
    }
}

#[derive(Clone)]
struct Emitter {
    id: EntityId,
    side: Side,
    pos: Enu,
    h: f64,
    spec: RadarSpec,
    range_max: f64,
}

#[derive(Clone)]
struct Contact {
    id: EntityId,
    side: Side,
    pos: Enu,
    h: f64,
    rcs: f64,
    moving: bool,
    radial: Enu,
}

fn radar_spec(r: &bgws_catalogue::RadarParams, scan_s: f64, prf: f64, default_scan: f64) -> RadarSpec {
    RadarSpec {
        peak_power_w: r.pt_w,
        gain_dbi: r.gain_db,
        freq_hz: r.freq_hz,
        bandwidth_hz: r.bandwidth_hz.max(1e5),
        noise_figure_db: r.noise_figure_db,
        losses_db: r.losses_db,
        pulse_width_s: 1.0 / r.bandwidth_hz.max(1e5),
        prf_hz: prf,
        az_beamwidth_rad: r.beamwidth_deg.max(0.5).to_radians(),
        scan_period_s: if scan_s > 0.0 { scan_s } else { default_scan },
        track_update_s: 1.0,
        track_pulses: r.pulses_integrated.max(1.0),
        mti: r.mti,
    }
}

/// Surface search by radar (slow rate).
pub(crate) fn radar(s: &mut Sim, dt: f64) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    let prf = st.p.g("radar_prf_hz");
    let scan = st.p.g("radar_scan_s");
    let mut emitters = Vec::new();
    let mut contacts = Vec::new();
    for &e in &s.ctx.order {
        if !alive(s, e) {
            continue;
        }
        let (Some(i), Some(k)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e)) else { continue };
        if i.side != Side::Neutral && i.domain != Domain::Space {
            let v = s.world.get::<Vessel>(e);
            let silent = v.map(|v| v.silent).unwrap_or(false);
            let sea_role = match i.domain {
                Domain::Surface => v.map(|v| v.kind == VesselKind::Warship).unwrap_or(false),
                Domain::Air | Domain::Node => true,
                Domain::Land => i.platform.mounts.iter().any(|m| m.kind == bgws_catalogue::WeaponKind::Ascm),
                _ => false,
            };
            if sea_role && !silent {
                for x in i.platform.sensors.iter().filter(|x| x.kind == CatSensor::Radar) {
                    if let Some(r) = &x.radar {
                        let h = match i.domain {
                            Domain::Air => k.agl.max(x.height_m),
                            _ => x.height_m.max(3.0),
                        };
                        emitters.push(Emitter { id: i.id, side: i.side, pos: k.pos, h, spec: radar_spec(r, x.scan_s, prf, scan), range_max: x.range_max_m });
                    }
                }
            }
        }
        if let Some(v) = s.world.get::<Vessel>(e) {
            if v.depth <= 20.0 {
                let rcs = i.platform.signatures.rcs.values().next().map(|a| a.side).unwrap_or(1_000.0) * if v.depth > 0.0 { 0.01 } else { 1.0 };
                contacts.push(Contact { id: i.id, side: i.side, pos: k.pos, h: if v.depth > 0.0 { 2.0 } else { v.mast_h }, rcs, moving: v.speed > 0.5, radial: k.vel });
            }
        }
    }
    let rp = RadarParams::from_figures(&s.ctx.figures);
    let mut pairs = Vec::new();
    for (oi, o) in emitters.iter().enumerate() {
        for (ti, t) in contacts.iter().enumerate() {
            if t.side == o.side || t.id == o.id {
                continue;
            }
            let r = t.pos.sub(o.pos).horizontal();
            if r <= o.range_max && r <= radar_horizon(o.h, t.h) {
                pairs.push((oi, ti, r));
            }
        }
    }
    let seed = s.ctx.seed;
    let ss_at = |p: Enu| st.sea_state(p.e, p.n, now);
    let hits: Vec<(usize, usize, f64, f64)> = pairs
        .par_iter()
        .filter_map(|&(oi, ti, r)| {
            let o = &emitters[oi];
            let t = &contacts[ti];
            let dir = t.pos.sub(o.pos).scale(1.0 / r.max(1.0));
            let radial = t.radial.dot(dir);
            let tgt = RadarTarget { rcs_m2: t.rcs, range_m: r, radial_speed_mps: radial, swerling: Swerling::One, path_loss_db: 0.0 };
            let bg = ClutterBackground::Sea { sea_state: ss_at(t.pos), grazing_rad: (o.h / r.max(1.0)).atan() };
            let d = detect(&o.spec, &tgt, bg, RadarMode::Search, &rp);
            let p = hazard_probability(d.rate_per_s, dt);
            let mut rng = seed.stream(StreamKey::new("naval.radar", o.id.0 as u64, t.id.0 as u64, now.0));
            (rng.gen::<f64>() < p).then_some((oi, ti, r, p))
        })
        .collect();
    let sigma = st.p.g("radar_sigma_m");
    let identify = st.p.g("identify_m");
    let delay = SimDuration::from_secs(st.p.g("report_delay_s"));
    for (oi, ti, r, p) in hits {
        let (o, t) = (&emitters[oi], &contacts[ti]);
        let level = if r <= identify { IdLevel::Identify } else { IdLevel::Detect };
        let hyp = hypothesis(s, t.id);
        let mut rng = s.ctx.rng("naval.radar.fix", o.id.0 as u64, t.id.0 as u64);
        let noise = Enu::new(rng.gen::<f64>() * 2.0 - 1.0, rng.gen::<f64>() * 2.0 - 1.0, 0.0).scale(sigma * 1.2);
        let det = Detection {
            time: now,
            observer: o.id,
            sensor: SensorKind::Radar,
            domain: bgws_know::Domain::Sea,
            motion: if t.moving { MotionHint::Moving } else { MotionHint::Static },
            measurement: Measurement::Position { pos: Enu::new(t.pos.e, t.pos.n, 0.0).add(noise), cov: Matrix3::from_diagonal(&nalgebra::Vector3::new(sigma * sigma, sigma * sigma, 100.0)) },
            class: ClassEvidence { level, reported: (level != IdLevel::Detect).then_some(hyp), resolves_decoys: true },
            truth: TruthTag::of(t.id),
        };
        s.ctx.reports.push((now + delay, o.side, det));
        if let Some(mut c) = s.world.get_mut::<ShipCombat>(s.ctx.entity(o.id)) {
            c.contacts.insert(t.id, now);
        }
        if st.seen.insert((o.id, t.id)) {
            st.stats.radar_contacts += 1;
            let text = format!("{} holds {} on radar at {:.0} km", label(s, o.id), label(s, t.id), r / 1000.0);
            s.ctx.log.push(now, "naval.radarContact").side(o.side).entity(o.id).target(t.id).text(text).data(serde_json::json!({ "rangeM": r, "p": p }));
        }
    }
    put(s, st);
}

#[derive(Clone)]
struct Hunter {
    id: EntityId,
    side: Side,
    pos: Enu,
    depth: f64,
    speed_kn: f64,
    sonars: Vec<bgws_catalogue::Sensor>,
    pinging: bool,
}

#[derive(Clone)]
struct Quarry {
    id: EntityId,
    side: Side,
    pos: Enu,
    depth: f64,
    sl: f64,
    sub: bool,
}

/// Passive and active sonar (slow rate).
pub(crate) fn sonar(s: &mut Sim, dt: f64) {
    let Some(mut st) = take(s) else { return };
    let now = s.ctx.now;
    let mut hunters = Vec::new();
    let mut quarry = Vec::new();
    for &e in &s.ctx.order {
        if !alive(s, e) {
            continue;
        }
        let (Some(i), Some(k), Some(v)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e), s.world.get::<Vessel>(e)) else { continue };
        let sonars: Vec<_> = i.platform.sensors.iter().filter(|x| matches!(x.kind, CatSensor::SonarPassive | CatSensor::SonarActive) && x.sonar.is_some()).cloned().collect();
        if !sonars.is_empty() && i.side != Side::Neutral && !v.task.going_home() {
            let pinging = v.kind == VesselKind::Warship && !v.silent && sonars.iter().any(|x| x.kind == CatSensor::SonarActive);
            hunters.push(Hunter { id: i.id, side: i.side, pos: k.pos, depth: v.depth.max(8.0), speed_kn: v.speed / 0.514_444, sonars, pinging });
        }
        let sl = match &v.sub {
            Some(sub) if !sub.nuclear => sub.source_level(&st.p.sub, v.speed / 0.514_444),
            _ => radiated_db(&i.platform, v.speed),
        };
        quarry.push(Quarry { id: i.id, side: i.side, pos: k.pos, depth: v.depth.max(5.0), sl, sub: v.kind == VesselKind::Submarine });
    }
    if hunters.is_empty() {
        put(s, st);
        return;
    }
    let ap = AcousticParams::from_figures(&s.ctx.figures);
    let sp = SonarParams::from_figures(&s.ctx.figures);
    let cz = st.profile.cz_range(ap.cz_vertex_margin_m_s).unwrap_or(0.0);
    let mut pairs = Vec::new();
    for (hi, h) in hunters.iter().enumerate() {
        let reach = h.sonars.iter().map(|x| x.range_max_m).fold(0.0, f64::max).max(if st.water_depth > ap.shallow_depth_m { cz * 1.1 } else { 0.0 });
        for (qi, q) in quarry.iter().enumerate() {
            if q.side == h.side || q.side == Side::Neutral || q.id == h.id {
                continue;
            }
            let r = q.pos.sub(h.pos).horizontal();
            if r <= reach {
                pairs.push((hi, qi, r));
            }
        }
    }
    let seed = s.ctx.seed;
    let profile = &st.profile;
    let water = st.water_depth;
    let ts_sub = st.p.g("ts_sub_db");
    let ts_ship = st.p.g("ts_ship_db");
    let ss = st.sea_state(0.0, 0.0, now);
    let found: Vec<(usize, usize, f64, f64, bool)> = pairs
        .par_iter()
        .filter_map(|&(hi, qi, r)| {
            let h = &hunters[hi];
            let q = &quarry[qi];
            let mut best = (0.0f64, false);
            for x in &h.sonars {
                let son = x.sonar.as_ref()?;
                let rcv = if son.towed { (h.depth + 50.0).min(water - 5.0).max(5.0) } else { h.depth };
                let g = WaterGeometry { f_hz: son.freq_hz, range_m: r, src_depth_m: q.depth.min(water - 1.0), rcv_depth_m: rcv.min(water - 1.0), water_depth_m: water };
                let tl = water_tl(&g, profile, &ap).total_db;
                let nl = noise_level(son.freq_hz, Shipping::Moderate, ss, h.speed_kn, &sp).total_db;
                let res = match x.kind {
                    CatSensor::SonarActive if h.pinging && r <= x.range_max_m => active(&ActiveLevels { sl_db: son.source_level_db, tl_db: tl, ts_db: if q.sub { ts_sub } else { ts_ship }, nl_db: nl, di_db: son.di_db, rl_db: None, dt_db: son.dt_db }, &sp),
                    CatSensor::SonarPassive => passive(q.sl, tl, nl, son.di_db, son.dt_db, &sp),
                    _ => continue,
                };
                if res.pd > best.0 {
                    best = (res.pd, x.kind == CatSensor::SonarActive);
                }
            }
            let p = best.0 * (dt / 10.0).min(1.0);
            let mut rng = seed.stream(StreamKey::new("naval.sonar", h.id.0 as u64, q.id.0 as u64, now.0));
            (rng.gen::<f64>() < p).then_some((hi, qi, r, p, best.1))
        })
        .collect();
    let bearing_sigma = st.p.g("sonar_bearing_sigma");
    let delay = SimDuration::from_secs(st.p.g("report_delay_s"));
    st.fresh_sonar.clear();
    for (hi, qi, r, p, act) in found {
        let (h, q) = (&hunters[hi], &quarry[qi]);
        let sigma = (r * bearing_sigma).max(50.0);
        let mut rng = s.ctx.rng("naval.sonar.fix", h.id.0 as u64, q.id.0 as u64);
        let noise = Enu::new(rng.gen::<f64>() * 2.0 - 1.0, rng.gen::<f64>() * 2.0 - 1.0, 0.0).scale(sigma);
        let hyp = hypothesis(s, q.id);
        let det = Detection {
            time: now,
            observer: h.id,
            sensor: SensorKind::Sonar,
            domain: bgws_know::Domain::Sea,
            motion: MotionHint::Moving,
            measurement: Measurement::Position { pos: Enu::new(q.pos.e, q.pos.n, 0.0).add(noise), cov: Matrix3::from_diagonal(&nalgebra::Vector3::new(sigma * sigma, sigma * sigma, 1.0e4)) },
            class: ClassEvidence { level: IdLevel::Recognise, reported: Some(hyp), resolves_decoys: true },
            truth: TruthTag::of(q.id),
        };
        s.ctx.reports.push((now + delay, h.side, det));
        if let Some(mut c) = s.world.get_mut::<ShipCombat>(s.ctx.entity(h.id)) {
            c.contacts.insert(q.id, now);
        }
        st.fresh_sonar.push((h.id, q.id));
        if st.seen.insert((h.id, q.id)) {
            st.stats.sonar_contacts += 1;
            let text = format!("{} has {} on {} sonar at {:.1} km", label(s, h.id), label(s, q.id), if act { "active" } else { "passive" }, r / 1000.0);
            s.ctx.log.push(now, "naval.sonarContact").side(h.side).entity(h.id).target(q.id).text(text).data(serde_json::json!({ "rangeM": r, "p": p, "active": act }));
        }
    }
    // A submarine hears an active sonar long before the sonar hears it.
    let factor = st.p.g("ping_heard_factor");
    let gap = st.p.g("contact_ask_s");
    for h in hunters.iter().filter(|h| h.pinging) {
        let reach = h.sonars.iter().filter(|x| x.kind == CatSensor::SonarActive).map(|x| x.range_max_m).fold(0.0, f64::max) * factor;
        for q in quarry.iter().filter(|q| q.sub && q.side != h.side && q.side != Side::Neutral) {
            if q.pos.sub(h.pos).horizontal() <= reach && !st.recently_asked((q.id, "counterDetected", Some(h.id)), now, gap) {
                st.asked.insert((q.id, "counterDetected", Some(h.id)), now);
                st.deferred.push((q.id, "counterDetected", Some(h.id), "silent", "sea.goSilent"));
            }
        }
    }
    put(s, st);
}

/// Shooters with anti-ship missiles: (id, launch point, munition, rounds, max range).
pub(crate) fn strikers(s: &Sim, side: Side) -> Vec<(EntityId, Enu, String, u32, f64)> {
    let mut out = Vec::new();
    for &e in &s.ctx.order {
        if !alive(s, e) {
            continue;
        }
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        if i.side != side {
            continue;
        }
        let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
        for m in i.platform.mounts.iter().filter(|m| m.kind == bgws_catalogue::WeaponKind::Ascm) {
            let Some(mu) = m.munitions.first() else { continue };
            let rounds = if let Some(c) = s.world.get::<ShipCombat>(e) {
                if c.state.mission_kill || s.world.get::<Vessel>(e).map(|v| v.task.going_home()).unwrap_or(false) {
                    0
                } else {
                    c.magazine.get(mu).copied().unwrap_or(0)
                }
            } else {
                s.world.get::<Arms>(e).map(|a| a.mounts.iter().filter(|x| &x.munition == mu).map(|x| x.rounds).sum()).unwrap_or(0)
            };
            let range = s.ctx.catalogue.munition(mu).map(|x| x.max_range_m).unwrap_or(0.0);
            if rounds > 0 {
                out.push((i.id, pos, mu.clone(), rounds, range));
            }
        }
    }
    out
}

/// The side's belief about a contact (track view by truth), if it holds one.
pub(crate) fn track_of(s: &Sim, side: Side, truth: EntityId) -> Option<bgws_know::store::TrackView> {
    let store = s.ctx.tracks.get(&side)?;
    let by = store.audit().live_by_truth();
    let ids = by.get(&truth)?;
    ids.iter().filter_map(|t| store.view(*t, &s.ctx.p.know)).max_by(|a, b| a.last_update.cmp(&b.last_update).then(b.id.cmp(&a.id)))
}

/// Is a missile of this side already in the air at this ship?
fn missiles_at(s: &Sim, side: Side, target: EntityId) -> bool {
    s.world
        .get_resource::<super::NavalState>()
        .map(|st| st.inbound.values().any(|i| i.side == side && i.target == target && i.state != super::InboundState::Done && i.entity.is_none()))
        .unwrap_or(false)
}

/// The contact decision points (slow rate): a surface contact is put to a
/// shooter that can reach it; a sonar contact to the hunter.
pub(crate) fn contacts(s: &mut Sim) {
    let now = s.ctx.now;
    let Some(st) = s.world.get_resource::<super::NavalState>() else { return };
    let gap = st.p.g("contact_ask_s");
    let fresh = st.fresh_sonar.clone();
    let mut asks: Vec<(EntityId, &'static str, Option<EntityId>, &'static str, &'static str)> = Vec::new();
    let mut marks = Vec::new();
    for side in Side::PLAYERS {
        let shooters = strikers(s, side);
        if shooters.is_empty() {
            continue;
        }
        let Some(store) = s.ctx.tracks.get(&side) else { continue };
        let by = store.audit().live_by_truth();
        for (truth, tids) in by {
            let te = s.ctx.entity(truth);
            if !alive(s, te) {
                continue;
            }
            let Some(ti) = s.world.get::<Ident>(te) else { continue };
            if ti.side == side || s.world.get::<Vessel>(te).map(|v| matches!(v.kind, VesselKind::Usv | VesselKind::Submarine)).unwrap_or(true) {
                continue;
            }
            let Some(view) = tids.iter().filter_map(|t| store.view(*t, &s.ctx.p.know)).find(|v| v.domain == bgws_know::Domain::Sea) else { continue };
            if (now - view.last_update).secs() > 120.0 || missiles_at(s, side, truth) {
                continue;
            }
            let top = view.class_top.first().copied();
            let identified = view.best_level >= IdLevel::Recognise;
            if identified && matches!(top, Some((Hypothesis::Merchant, p)) if p >= 0.5) {
                continue;
            }
            let hostile = identified && matches!(top, Some((Hypothesis::Warship, p)) | Some((Hypothesis::Usv, p)) if p >= 0.5);
            for (sid, at, _, _, range) in &shooters {
                if view.position.sub(*at).horizontal() > *range {
                    continue;
                }
                if st.recently_asked((*sid, "surfaceContact", Some(truth)), now, gap) {
                    continue;
                }
                // Weapons free on an unidentified contact only for a coastal
                // battery denying the sea; a ship identifies first (it may be
                // a merchant).
                let se = s.ctx.entity(*sid);
                let coastal = s.world.get::<Ident>(se).map(|i| i.domain == Domain::Land).unwrap_or(false);
                let free = coastal && s.world.get::<Act>(se).map(|a| a.roe == RoeS::Always).unwrap_or(false);
                let (opt, action) = if hostile || free { ("engage", "sea.engageBeyondHorizon") } else { ("identify", "sea.closeToIdentify") };
                marks.push((*sid, "surfaceContact", Some(truth)));
                asks.push((*sid, "surfaceContact", Some(truth), opt, action));
                break;
            }
        }
    }
    for (h, q) in fresh {
        if st.recently_asked((h, "sonarContact", Some(q)), now, gap) || marks.contains(&(h, "sonarContact", Some(q))) {
            continue;
        }
        let he = s.ctx.entity(h);
        let torpedo = s.world.get::<Ident>(he).and_then(|i| i.platform.mounts.iter().find(|m| m.kind == bgws_catalogue::WeaponKind::Torpedo).and_then(|m| m.munitions.first().cloned()));
        let armed = torpedo.as_ref().map(|t| s.world.get::<ShipCombat>(he).map(|c| c.magazine.get(t).copied().unwrap_or(0) > 0).unwrap_or(false)).unwrap_or(false);
        let (opt, action) = if armed { ("attack", "sea.attack") } else { ("track", "sea.track") };
        marks.push((h, "sonarContact", Some(q)));
        asks.push((h, "sonarContact", Some(q), opt, action));
    }
    if let Some(mut st) = take(s) {
        for m in marks {
            st.asked.insert(m, now);
        }
        st.deferred.extend(asks);
        put(s, st);
    }
}
