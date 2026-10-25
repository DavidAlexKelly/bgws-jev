//! Sensing in the air domain (§8.2 "Radar"): every radar that sees air and
//! is radiating looks at every enemy aircraft and munition in flight once a
//! second. The radar range equation with the record's power, gain and
//! frequency, the target's RCS by aspect in the radar's band, Swerling 1
//! fluctuation, ground clutter for a target low against the ground, the
//! MTI Doppler notch a tangential target hides in, the 4/3-Earth horizon
//! and terrain masking for low flyers (`bgws_models::sensing::radar`) give
//! a detection rate; keyed dice turn it into plots that feed the side's
//! track store over the data link. IRST and EO turrets add short-range
//! passive looks that can tell a decoy from the real thing.
//!
//! Electronic support (ESM receivers, RWRs) locates enemy air-defence radars
//! that radiate: those fixes are what SEAD is tasked against.

use super::{AirWorld, Craft, Fix, InFlight};
use crate::components::*;
use crate::sim::Sim;
use crate::systems::alive;
use bevy_ecs::prelude::*;
use bgws_catalogue::{Aspect, Domain, SensorKind as CatSensor};
use bgws_core::geo::K_RADIO;
use bgws_core::{hazard_probability, EntityId, Enu, Side, StreamKey};
use bgws_know::{
    ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag,
};
use bgws_models::sensing::radar::{
    detect, ClutterBackground, LandCover, RadarMode, RadarSpec, RadarTarget, RcsProfile, Swerling,
};
use nalgebra::Matrix3;
use rand::Rng;
use rayon::prelude::*;

/// One radar as the sensing pass uses it.
#[derive(Clone, Debug)]
pub struct RadarSet {
    pub spec: RadarSpec,
    pub band: String,
    pub range_max: f64,
    pub height: f64,
    /// A fire-control radar (an AD unit's): classifies what it tracks.
    pub fire_control: bool,
}

/// Air-looking sensors fitted to an entity.
#[derive(Component, Clone, Debug, Default)]
pub struct AirSensors {
    pub radars: Vec<RadarSet>,
    /// IRST, EO and thermal sights that see air: (kind, range).
    pub passive: Vec<(CatSensor, f64)>,
    /// ESM receivers and RWRs: range.
    pub esm_range: f64,
    /// The scenario's emission control for it.
    pub emcon: bgws_scenario::v4::Emcon,
    /// Radar shut down until then (an ARM warning, letting a raid pass).
    pub silent_until: Option<bgws_core::SimTime>,
}

/// Build the air sensors of a platform.
pub fn sensors_of(s: &Sim, aw: &AirWorld, p: &bgws_catalogue::Platform, ad: bool) -> AirSensors {
    let mut out = AirSensors::default();
    for x in &p.sensors {
        match x.kind {
            CatSensor::Radar if x.sees_air => {
                let Some(r) = x.radar else { continue };
                let freq = if r.freq_hz > 0.0 {
                    r.freq_hz
                } else {
                    s.ctx
                        .catalogue
                        .band(&x.band)
                        .map(|b| b.centre_hz())
                        .unwrap_or(1e10)
                };
                let beam = if r.beamwidth_deg > 0.0 {
                    r.beamwidth_deg
                } else {
                    aw.p.beam_deg
                }
                .to_radians();
                let scan = if x.scan_s > 0.0 {
                    x.scan_s
                } else {
                    aw.p.default_scan_s
                };
                let pulses = r.pulses_integrated.max(1.0);
                let bw = if r.bandwidth_hz > 0.0 {
                    r.bandwidth_hz
                } else {
                    1e6
                };
                out.radars.push(RadarSet {
                    spec: RadarSpec {
                        peak_power_w: r.pt_w,
                        gain_dbi: r.gain_db,
                        freq_hz: freq,
                        bandwidth_hz: bw,
                        noise_figure_db: r.noise_figure_db,
                        losses_db: r.losses_db,
                        pulse_width_s: 1.0 / bw,
                        prf_hz: pulses * std::f64::consts::TAU / (beam * scan),
                        az_beamwidth_rad: beam,
                        scan_period_s: scan,
                        track_update_s: 1.0,
                        track_pulses: pulses,
                        mti: r.mti,
                    },
                    band: x.band.clone(),
                    range_max: x.range_max_m,
                    height: x.height_m,
                    fire_control: ad,
                });
            }
            CatSensor::Irst => out.passive.push((x.kind, x.range_max_m)),
            CatSensor::Eo | CatSensor::Thermal if (ad || p.domain == Domain::Air) && x.sees_air => {
                out.passive.push((x.kind, x.range_max_m))
            }
            CatSensor::Esm | CatSensor::Rwr => out.esm_range = out.esm_range.max(x.range_max_m),
            _ => {}
        }
    }
    out
}

/// A target snapshot for the parallel pass.
#[derive(Clone)]
struct Tgt {
    id: EntityId,
    side: Side,
    pos: Enu,
    vel: Enu,
    heading: f64,
    agl: f64,
    rcs: std::collections::BTreeMap<String, bgws_catalogue::AspectRcs>,
    rcs_flat: f64,
    hyp: Hypothesis,
    decoy: bool,
    ir: f64,
}

#[derive(Clone)]
struct Obs {
    id: EntityId,
    side: Side,
    pos: Enu,
    agl: f64,
    sensors: AirSensors,
}

struct Plot {
    obs: usize,
    tgt: usize,
    sensor: SensorKind,
    level: IdLevel,
    sigma: f64,
    resolves: bool,
    snr_db: f64,
}

fn hypothesis(s: &Sim, e: Entity) -> (Hypothesis, bool) {
    if let Some(f) = s.world.get::<InFlight>(e) {
        return match f.class {
            Some(bgws_domains::air_defence::ThreatClass::OneWayDrone) => {
                (Hypothesis::Drone, f.decoy)
            }
            _ => (Hypothesis::Missile, f.decoy),
        };
    }
    let Some(i) = s.world.get::<Ident>(e) else {
        return (Hypothesis::Fighter, false);
    };
    let p = &i.platform;
    let h = if p.mover.kind == bgws_catalogue::MoverKind::Rotary {
        Hypothesis::Helicopter
    } else if p.body.crew == 0 {
        Hypothesis::Drone
    } else if p.class == "fighter" {
        Hypothesis::Fighter
    } else {
        Hypothesis::Bomber
    };
    (h, i.dummy)
}

/// Is this entity's radar on? (EMCON, a shutdown after an ARM warning.)
pub(crate) fn radiating(s: &Sim, e: Entity) -> bool {
    super::defence::radiating(s, e)
}

fn to_cover(c: bgws_env::LandCover) -> LandCover {
    use bgws_env::LandCover as L;
    match c {
        L::Open | L::Bare => LandCover::Open,
        L::Crops | L::Scrub => LandCover::Farmland,
        L::ForestLight | L::ForestDense => LandCover::Forest,
        L::UrbanLight | L::UrbanDense => LandCover::Urban,
        L::Water | L::Wetland => LandCover::Wetland,
    }
}

/// The once-a-second air sensing pass.
pub fn sense(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    // Targets: airborne aircraft and threat munitions in flight.
    let mut tgts: Vec<Tgt> = Vec::new();
    for id in aw.crafts.iter().chain(aw.flights.iter()) {
        let e = s.ctx.entity(*id);
        let Some(k) = s.world.get::<Kin>(e) else {
            continue;
        };
        let (side, rcs, rcs_flat, ir) = if let Some(f) = s.world.get::<InFlight>(e) {
            if !f.alive || f.class.is_none() {
                continue;
            }
            let rcs = s
                .world
                .get::<Ident>(e)
                .map(|i| i.platform.signatures.rcs.clone())
                .unwrap_or_default();
            (f.side, rcs, f.rcs_m2, 0.5)
        } else if let Some(c) = s.world.get::<Craft>(e) {
            if !c.airborne() || !alive(s, e) {
                continue;
            }
            let i = s.world.get::<Ident>(e).expect("i");
            (
                i.side,
                i.platform.signatures.rcs.clone(),
                i.platform.signatures.rcs_m2("x", Aspect::Front),
                i.platform.signatures.index.ir.max(0.5),
            )
        } else {
            continue;
        };
        let (hyp, decoy) = hypothesis(s, e);
        tgts.push(Tgt {
            id: *id,
            side,
            pos: k.pos,
            vel: k.vel,
            heading: k.heading,
            agl: k.agl,
            rcs,
            rcs_flat,
            hyp,
            decoy,
            ir,
        });
    }
    if tgts.is_empty() {
        return;
    }
    // Observers: entities with air sensors, alive, airborne if aircraft.
    let mut obs: Vec<Obs> = Vec::new();
    for (idx, &e) in s.ctx.order.iter().enumerate() {
        let Some(sens) = s.world.get::<AirSensors>(e) else {
            continue;
        };
        if !alive(s, e) {
            continue;
        }
        if let Some(c) = s.world.get::<Craft>(e) {
            if !c.airborne() {
                continue;
            }
        }
        let i = s.world.get::<Ident>(e).expect("i");
        let k = s.world.get::<Kin>(e).expect("k");
        let mut sensors = sens.clone();
        if !radiating(s, e) {
            sensors.radars.clear();
        }
        if sensors.radars.is_empty() && sensors.passive.is_empty() {
            continue;
        }
        obs.push(Obs {
            id: EntityId(idx as u32),
            side: i.side,
            pos: k.pos,
            agl: k.agl,
            sensors,
        });
    }
    let mut pairs = Vec::new();
    for (oi, o) in obs.iter().enumerate() {
        let reach = o
            .sensors
            .radars
            .iter()
            .map(|r| r.range_max * 1.5)
            .chain(o.sensors.passive.iter().map(|p| p.1))
            .fold(0.0, f64::max);
        for (ti, t) in tgts.iter().enumerate() {
            if t.side == o.side || t.side == Side::Neutral {
                continue;
            }
            let d = t.pos.sub(o.pos);
            if d.horizontal() <= reach {
                pairs.push((oi, ti));
            }
        }
    }
    let ground = &s.ctx.ground;
    let seed = s.ctx.seed;
    let p = &aw.p;
    let plots: Vec<Plot> = pairs
        .par_iter()
        .filter_map(|&(oi, ti)| {
            let o = &obs[oi];
            let t = &tgts[ti];
            let rel = t.pos.sub(o.pos);
            let range = rel.norm().max(1.0);
            let mut rng = seed.stream(StreamKey::new(
                "air.radar",
                o.id.0 as u64,
                t.id.0 as u64,
                now.0,
            ));
            let mut best: Option<Plot> = None;
            for r in &o.sensors.radars {
                let h_o = o.agl.max(0.0) + r.height;
                // The 4/3-Earth horizon [R6.2], then terrain for low flyers.
                if rel.horizontal()
                    > bgws_core::geo::mutual_horizon(h_o.max(1.0), t.agl.max(1.0), K_RADIO)
                {
                    continue;
                }
                if t.agl < 500.0
                    && range < p.terrain_check_m
                    && !ground
                        .los(o.pos, h_o, t.pos, t.agl.max(1.0))
                        .terrain_visible
                {
                    continue;
                }
                // RCS by aspect: azimuth off the target's nose, elevation from below/above.
                let los_bearing = rel.bearing();
                let off_nose = ((los_bearing + std::f64::consts::PI) - t.heading)
                    .rem_euclid(std::f64::consts::TAU);
                let elev = (-(rel.u) / range).clamp(-1.0, 1.0).asin().abs();
                let a = t.rcs.get(&r.band).or_else(|| t.rcs.get("x")).copied();
                let rcs = match a {
                    Some(a) if a.front > 0.0 => RcsProfile {
                        front_m2: a.front,
                        side_m2: a.side.max(a.front),
                        rear_m2: a.rear.max(a.front),
                        top_m2: a.top.max(a.front),
                    }
                    .at(off_nose, elev),
                    _ => t.rcs_flat.max(1e-4),
                };
                let radial = t.vel.dot(rel.scale(1.0 / range));
                // Clutter when the beam sees the ground behind the target.
                let elev_up = (rel.u / range).clamp(-1.0, 1.0).asin();
                let bg = if elev_up < r.spec.az_beamwidth_rad / 2.0 && t.agl < 3_000.0 {
                    let cover = to_cover(ground.land_cover(t.pos.e, t.pos.n));
                    let grazing = ((h_o - t.agl).abs() / range).clamp(1e-3, 1.0).asin();
                    ClutterBackground::Land {
                        cover,
                        grazing_rad: grazing,
                    }
                } else {
                    ClutterBackground::None
                };
                let tgt = RadarTarget {
                    rcs_m2: rcs,
                    range_m: range,
                    radial_speed_mps: radial,
                    swerling: Swerling::One,
                    path_loss_db: 0.0,
                };
                let det = detect(&r.spec, &tgt, bg, RadarMode::Search, &p.radar);
                let pd = hazard_probability(det.rate_per_s, 1.0);
                if rng.gen::<f64>() >= pd {
                    continue;
                }
                let sigma = (range * r.spec.az_beamwidth_rad / 2.5).max(p.sigma_min);
                let level = if r.fire_control || range < p.nctr_m {
                    IdLevel::Recognise
                } else {
                    IdLevel::Detect
                };
                if best.as_ref().map(|b| det.snr_db > b.snr_db).unwrap_or(true) {
                    best = Some(Plot {
                        obs: oi,
                        tgt: ti,
                        sensor: SensorKind::Radar,
                        level,
                        sigma,
                        resolves: false,
                        snr_db: det.snr_db,
                    });
                }
            }
            if best.is_none() {
                for &(kind, reach) in &o.sensors.passive {
                    // Passive: the target's heat over the background, inside the reach.
                    let eff = reach * t.ir.sqrt().min(2.0);
                    if range > eff {
                        continue;
                    }
                    if t.agl < 500.0
                        && !ground
                            .los(o.pos, o.agl.max(2.0), t.pos, t.agl.max(1.0))
                            .terrain_visible
                    {
                        continue;
                    }
                    let rate = p.irst_rate * (1.0 - range / eff).max(0.05);
                    if rng.gen::<f64>() >= hazard_probability(rate, 1.0) {
                        continue;
                    }
                    let close = range < eff * 0.3;
                    best = Some(Plot {
                        obs: oi,
                        tgt: ti,
                        sensor: SensorKind::Thermal,
                        level: if close {
                            IdLevel::Identify
                        } else {
                            IdLevel::Recognise
                        },
                        sigma: (range * 0.002).max(p.sigma_min)
                            * if kind == CatSensor::Irst { 3.0 } else { 1.0 },
                        resolves: close,
                        snr_db: 0.0,
                    });
                    break;
                }
            }
            best
        })
        .collect();
    // The side's air picture takes one plot per target per scan (the best
    // one): several radars on one target in the same second are fused, not
    // separate tracks.
    let mut best: std::collections::BTreeMap<(Side, usize), Plot> = Default::default();
    for pl in plots {
        let key = (obs[pl.obs].side, pl.tgt);
        let better = best
            .get(&key)
            .map(|b| (pl.level, -(pl.sigma as i64)) > (b.level, -(b.sigma as i64)))
            .unwrap_or(true);
        if better {
            best.insert(key, pl);
        }
    }
    // Plots into each side's store.
    let mut by_side: std::collections::BTreeMap<Side, Vec<Detection>> = Default::default();
    for (_, pl) in best {
        let o = &obs[pl.obs];
        let t = &tgts[pl.tgt];
        let mut rng = s.ctx.rng("air.plot", o.id.0 as u64, t.id.0 as u64);
        let g = |r: &mut rand_chacha::ChaCha8Rng| {
            let (u1, u2): (f64, f64) = (r.gen::<f64>().max(1e-12), r.gen());
            (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
        };
        let noise = Enu::new(g(&mut rng), g(&mut rng), g(&mut rng) * 0.5).scale(pl.sigma);
        let reported = if pl.level == IdLevel::Detect {
            None
        } else if t.decoy && !pl.resolves {
            Some(Hypothesis::Drone)
        } else if t.decoy {
            Some(Hypothesis::AirDecoy)
        } else {
            Some(t.hyp)
        };
        by_side.entry(o.side).or_default().push(Detection {
            time: now,
            observer: o.id,
            sensor: pl.sensor,
            domain: bgws_know::Domain::Air,
            motion: MotionHint::Moving,
            measurement: Measurement::Position {
                pos: t.pos.add(noise),
                cov: Matrix3::from_diagonal_element(pl.sigma * pl.sigma),
            },
            class: ClassEvidence {
                level: pl.level,
                reported,
                resolves_decoys: pl.resolves,
            },
            truth: TruthTag::of(t.id),
        });
    }
    for (side, dets) in by_side {
        let kp = s.ctx.p.know.clone();
        if let Some(store) = s.ctx.tracks.get_mut(&side) {
            store.ingest(dets, &kp);
        }
    }
}

/// ESM and RWRs locate enemy AD radars that radiate (every 10 s).
pub fn esm_fixes(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    let mut found: Vec<(Side, EntityId, Enu, f64)> = Vec::new();
    let listeners: Vec<(Side, EntityId, Enu, f64, f64)> = s
        .ctx
        .order
        .iter()
        .enumerate()
        .filter_map(|(i, &e)| {
            let sens = s.world.get::<AirSensors>(e)?;
            if sens.esm_range <= 0.0 || !alive(s, e) {
                return None;
            }
            if let Some(c) = s.world.get::<Craft>(e) {
                if !c.airborne() {
                    return None;
                }
            }
            let k = s.world.get::<Kin>(e)?;
            Some((
                s.world.get::<Ident>(e)?.side,
                EntityId(i as u32),
                k.pos,
                k.agl.max(2.0),
                sens.esm_range,
            ))
        })
        .collect();
    for &site in &aw.ad_units {
        let e = s.ctx.entity(site);
        if !alive(s, e) || !radiating(s, e) {
            continue;
        }
        let Some(k) = s.world.get::<Kin>(e) else {
            continue;
        };
        let side = s.world.get::<Ident>(e).expect("i").side;
        let h_site = s
            .world
            .get::<AirSensors>(e)
            .and_then(|x| x.radars.first().map(|r| r.height))
            .unwrap_or(5.0);
        let best = listeners
            .iter()
            .filter(|l| l.0 == side.enemy())
            .filter(|l| {
                let d = k.pos.sub(l.2).horizontal();
                d <= l.4 && d <= bgws_core::geo::mutual_horizon(l.3, h_site, K_RADIO)
            })
            .map(|l| k.pos.sub(l.2).horizontal())
            .fold(f64::INFINITY, f64::min);
        if best.is_finite() {
            // A DF fix: about 1° of bearing error at the range of the nearest listener.
            let err = (best * 0.0175).max(200.0);
            let mut rng = s.ctx.rng("air.esm", site.0 as u64, 0);
            let off = Enu::new(
                (rng.gen::<f64>() - 0.5) * err,
                (rng.gen::<f64>() - 0.5) * err,
                0.0,
            );
            found.push((side.enemy(), site, k.pos.add(off), err));
        }
    }
    for (side, site, at, err) in found {
        let fresh = !aw
            .fixes
            .get(&side)
            .map(|m| m.contains_key(&site))
            .unwrap_or(false);
        aw.fixes.entry(side).or_default().insert(
            site,
            Fix {
                at: Enu::new(at.e, at.n, 0.0),
                error_m: err,
                time: now,
            },
        );
        if fresh {
            s.ctx
                .log
                .push(now, "air.esmFix")
                .side(side)
                .target(site)
                .text(format!(
                    "{side} locates an air-defence radar to {:.0} m",
                    err
                ))
                .data(serde_json::json!({ "errorM": err }));
        }
    }
    // Fixes of destroyed sites or long silent ones go stale.
    for m in aw.fixes.values_mut() {
        m.retain(|site, f| alive(s, s.ctx.entity(*site)) && (now - f.time).secs() < 3_600.0);
    }
}
