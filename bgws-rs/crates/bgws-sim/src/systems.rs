//! The land systems at the base rate (§6.1): posture, movement, sensing,
//! contact reports, direct fire, people, the autopilot's own events and the
//! victory check. Each iterates entities in id order; heavy per-pair work
//! (line of sight, detection hazards) runs in parallel with `rayon` into
//! per-pair results that are then applied in order, so the outcome does not
//! depend on threads (§12).

use crate::components::*;
use crate::sim::{Ctx, Outcome, Sim, Trigger};
use bgws_catalogue::{Domain, MoverKind, SensorKind as CatSensor, Vulnerability};
use bgws_core::{hazard_probability, Enu, EntityId, Side, SimTime, StreamKey};
use bgws_env::{transmission, OpticalBand};
use bgws_know::{ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::direct_fire::{self as df, ArmourZones, FireControl, Gun, PersonPosture, Protection, Shot, StrikeOutcome, VehicleDims, VehiclePosture, Warhead, Zone};
use bgws_models::people::{IncomingFire, MoraleState, StressInputs, UnitQuality, WorkLoad};
use bgws_models::sensing::acquire::{acquire, AcquireInput, Contrast, Metric, Search, Stage};
use bgws_models::sensing::observer::{tau_multiplier, ObserverState};
use bevy_ecs::prelude::Entity;
use nalgebra::Matrix3;
use rand::Rng;
use rayon::prelude::*;

pub(crate) fn alive(s: &Sim, e: Entity) -> bool {
    s.world.get::<Health>(e).map(|h| !h.destroyed).unwrap_or(false)
}

fn is_land(s: &Sim, e: Entity) -> bool {
    s.world.get::<Ident>(e).map(|i| i.domain == Domain::Land).unwrap_or(false)
}

/// Its level of detail (§6.4); entity level unless the LOD domain says otherwise.
pub(crate) fn lod(s: &Sim, e: Entity) -> Lod {
    s.world.get::<Lod>(e).copied().unwrap_or_default()
}

/// Posture from how long it has been still (moving, halted, settled, hull-down).
pub fn posture(s: &mut Sim) {
    let now = s.ctx.now;
    let (settle, hd) = (s.ctx.p.settle_s, s.ctx.p.hulldown_s);
    for &e in &s.ctx.order.clone() {
        let pos = match s.world.get::<Kin>(e) {
            Some(k) => k.pos,
            None => continue,
        };
        let cover = s.ctx.ground.micro(pos.e, pos.n).cover;
        let Some(mut a) = s.world.get_mut::<Act>(e) else { continue };
        let moving = a.activity.moving();
        if moving {
            a.posture = Posture::Moving;
            a.still_since = now;
        } else {
            let still = (now - a.still_since).secs();
            a.posture = if still >= settle + hd && cover > 0.3 {
                Posture::HullDown
            } else if still >= settle {
                Posture::Settled
            } else {
                Posture::Halted
            };
        }
    }
}

pub(crate) fn mover_class(k: MoverKind) -> (bgws_env::VehicleClass, bool) {
    match k {
        MoverKind::Tracked => (bgws_env::VehicleClass::Tracked, true),
        MoverKind::Wheeled => (bgws_env::VehicleClass::Wheeled, true),
        MoverKind::Foot => (bgws_env::VehicleClass::Foot, true),
        _ => (bgws_env::VehicleClass::Wheeled, false),
    }
}

/// Ground movement along routes (§8.1 Ground): speed from the cell's
/// trafficability, the vehicle's speeds, the order's mode and suppression.
pub fn movement(s: &mut Sim, dt: f64) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        // Aggregated formations move by their formation mover (domains::lod).
        if !alive(s, e) || !is_land(s, e) || lod(s, e) == Lod::Aggregated {
            continue;
        }
        let (platform, can_move) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            let h = s.world.get::<Health>(e).expect("health");
            (i.platform.clone(), h.can_move())
        };
        let (class, ground_mover) = mover_class(platform.mover.kind);
        if !ground_mover {
            continue;
        }
        let supp_mult = {
            let c = s.world.get::<Crew>(e).expect("crew");
            c.suppression.effects(&s.ctx.p.people).movement_mult
        };
        let act = s.world.get::<Act>(e).expect("act").activity.clone();
        let (route, mode, withdrawing) = match &act {
            Activity::Move { route, mode, .. } => (route.clone(), *mode, false),
            Activity::Withdraw { route, .. } => (route.clone(), MoveModeS::March, true),
            _ => {
                if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                    k.vel = Enu::default();
                }
                continue;
            }
        };
        if !can_move {
            continue;
        }
        let Some(&wp) = route.first() else {
            arrive(s, e);
            continue;
        };
        let k = *s.world.get::<Kin>(e).expect("kin");
        let going = s.ctx.ground.going(k.pos.e, k.pos.n, class);
        let off_road = platform.mover.cross_country_kmh.max(1.0).min(going.speed_kmh.max(0.0));
        let cap = platform.mover.road_kmh.max(platform.mover.cross_country_kmh) * bgws_models::ground::mode_cap(mode.into(), &s.ctx.p.ground);
        let speed_kmh = if withdrawing { off_road } else { off_road.min(cap) } * supp_mult;
        let v = speed_kmh / 3.6;
        let d = Enu::new(wp.e - k.pos.e, wp.n - k.pos.n, 0.0);
        let dist = d.horizontal();
        let step = (v * dt).min(dist);
        let (ne, nn) = if dist > 1e-6 { (k.pos.e + d.e / dist * step, k.pos.n + d.n / dist * step) } else { (wp.e, wp.n) };
        let nu = s.ctx.ground.height(ne, nn);
        let heading = if dist > 1e-6 { d.bearing() } else { k.heading };
        {
            let mut km = s.world.get_mut::<Kin>(e).expect("kin");
            km.pos = Enu::new(ne, nn, nu);
            km.vel = if dt > 0.0 { Enu::new((ne - k.pos.e) / dt, (nn - k.pos.n) / dt, 0.0) } else { Enu::default() };
            km.heading = heading;
        }
        if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
            sig.last_moved = Some(now);
            sig.engine_on = platform.mover.kind != MoverKind::Foot;
        }
        if dist - step <= s.ctx.p.arrive_m {
            let mut a = s.world.get_mut::<Act>(e).expect("act");
            match &mut a.activity {
                Activity::Move { route, .. } | Activity::Withdraw { route, .. } => {
                    route.remove(0);
                }
                _ => {}
            }
        }
    }
}

/// Reached the end of a route: carry on with `then`, else the next phase, else hold.
pub(crate) fn arrive(s: &mut Sim, e: Entity) {
    let id = s.world.get::<Ident>(e).expect("ident").id;
    let mut a = s.world.get_mut::<Act>(e).expect("act");
    let then = match &a.activity {
        Activity::Move { then, .. } | Activity::Withdraw { then, .. } => then.clone(),
        _ => None,
    };
    let was_phase = a.mission.phase < a.mission.phases.len();
    match then {
        Some(t) => a.activity = *t,
        None => {
            if was_phase && a.mission.phase + 1 < a.mission.phases.len() {
                a.mission.phase += 1;
                a.activity = a.mission.phases[a.mission.phase].clone();
                return;
            }
            a.activity = Activity::Overwatch;
            s.ctx.triggers.push(Trigger { entity: id, point: "D10", about: None });
        }
    }
}

/// A snapshot of one entity for the parallel sensing pass.
#[derive(Clone)]
struct SenseEnt {
    e: Entity,
    id: EntityId,
    side: Side,
    pos: Enu,
    agl: f64,
    eye_h: f64,
    crit: f64,
    contrast: f64,
    delta_t: f64,
    concealment: f64,
    moving: bool,
    sensors: Vec<(CatSensor, f64, f64, [f64; 3])>,
    observer: ObserverState,
    hypothesis: Hypothesis,
    dummy: bool,
}

pub(crate) fn hypothesis_of(p: &bgws_catalogue::Platform) -> Hypothesis {
    let r = p.role.to_lowercase();
    let c = p.class.to_lowercase();
    match p.domain {
        Domain::Air => {
            if c.contains("drone") || r.contains("drone") || c.contains("uav") {
                Hypothesis::Drone
            } else if c.contains("heli") || p.mover.kind == MoverKind::Rotary {
                Hypothesis::Helicopter
            } else {
                Hypothesis::Fighter
            }
        }
        Domain::Surface => Hypothesis::Warship,
        Domain::Subsurface => Hypothesis::Submarine,
        _ => {
            if c.contains("mbt") || r.contains("tank") {
                Hypothesis::Mbt
            } else if c.contains("ifv") {
                Hypothesis::Ifv
            } else if c.contains("apc") {
                Hypothesis::Apc
            } else if p.mover.kind == MoverKind::Foot {
                Hypothesis::Infantry
            } else if r.contains("artillery") || r.contains("mortar") || r.contains("fires") {
                Hypothesis::Artillery
            } else {
                Hypothesis::Truck
            }
        }
    }
}

/// The entities the land sensing pass works on: living land units at entity
/// level (aggregated and Far units are carried by `domains::lod` and neither
/// observe nor are observed here, §6.4).
fn snapshot_for_sensing(s: &Sim) -> Vec<SenseEnt> {
    let now = s.ctx.now;
    s.ctx
        .order
        .iter()
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            if i.domain != Domain::Land || lod(s, e) != Lod::Entity {
                return None;
            }
            let h = s.world.get::<Health>(e)?;
            if h.destroyed {
                return None;
            }
            let k = s.world.get::<Kin>(e)?;
            let a = s.world.get::<Act>(e)?;
            let c = s.world.get::<Crew>(e)?;
            let sig = s.world.get::<SigState>(e)?;
            let p = &i.platform;
            let v = &p.signatures.visual.contrast;
            let contrast = match a.posture {
                Posture::Moving => v.moving,
                Posture::Halted => v.halted,
                Posture::Settled => v.halted.min(v.camouflaged.max(v.hull_down)),
                Posture::HullDown => v.hull_down,
            };
            let dk = &p.signatures.ir.delta_k;
            let fired = sig.last_fired.map(|t| (now - t).secs() < 120.0).unwrap_or(false);
            let moved = sig.last_moved.map(|t| (now - t).secs() < 600.0).unwrap_or(false);
            let delta_t = if fired { dk.fired } else if moved { dk.moved } else if sig.engine_on { dk.engine_on } else { dk.cold };
            let micro = s.ctx.ground.micro(k.pos.e, k.pos.n);
            let conceal_w = match a.posture {
                Posture::Moving => 0.3,
                Posture::Halted => 0.6,
                _ => 1.0,
            };
            let crit = if a.posture == Posture::HullDown { p.signatures.visual.critical_dim_m * 0.6 } else { p.signatures.visual.critical_dim_m } * (h.fit.max(1) as f64).powf(0.15);
            let fat = c.fatigue.effects(&s.ctx.p.people);
            Some(SenseEnt {
                e,
                id: i.id,
                side: i.side,
                pos: k.pos,
                agl: k.agl,
                eye_h: p.sensors.iter().map(|x| x.height_m).fold(2.0, f64::max).min(6.0),
                crit,
                contrast,
                delta_t,
                concealment: micro.concealment * conceal_w,
                moving: a.posture == Posture::Moving,
                sensors: p
                    .sensors
                    .iter()
                    .filter(|x| matches!(x.kind, CatSensor::Eye | CatSensor::Optic | CatSensor::Thermal | CatSensor::ImageIntensifier | CatSensor::Eo))
                    .map(|x| {
                        let o = x.optics.as_ref();
                        (x.kind, x.range_max_m, o.map(|o| o.resolution_cyc_per_mrad).unwrap_or(0.5), o.map(|o| [o.n50.detect, o.n50.recognise, o.n50.identify]).unwrap_or([1.0, 3.0, 6.0]))
                    })
                    .collect(),
                observer: ObserverState {
                    suppression: c.suppression.level,
                    fatigue: 1.0 - fat.effectiveness,
                    overwatch: matches!(a.activity, Activity::Overwatch | Activity::Observe | Activity::Wait { .. }),
                    moving: a.activity.moving(),
                    other_tasks: if a.last_shot.map(|t| (now - t).secs() < 10.0).unwrap_or(false) { 1 } else { 0 },
                    contacts_held: s.world.get::<Aware>(e).map(|w| w.seen.len() as u32).unwrap_or(0),
                },
                hypothesis: hypothesis_of(p),
                dummy: i.dummy,
            })
        })
        .collect()
}

/// One observer–target result.
struct Sight {
    obs: usize,
    tgt: usize,
    p: f64,
    level: u8,
    range: f64,
    sensor: SensorKind,
}

/// The best sensor of an observer against one target (highest detection rate).
struct Best {
    rate: f64,
    sensor: SensorKind,
    contrast: Contrast,
    res: f64,
    n50: [f64; 3],
    /// The pair's first die.
    u: f64,
}

/// The hash-grid layer of a side's units.
fn side_layer(side: Side) -> u32 {
    match side {
        Side::Blue => 0,
        Side::Red => 1,
        Side::Neutral => 2,
    }
}

/// Candidate pairs (§11.1): land observers × enemy land targets within the
/// observer's reach, in (observer, target) order. The kernel layer's hash grid
/// narrows (with a hair of slack on the range), the exact horizontal test here
/// decides, so the list is the one the all-pairs loop gave.
fn candidate_pairs(s: &Sim, ents: &[SenseEnt], max_r: f64) -> Vec<(usize, usize, f64)> {
    use bgws_gpu::{Kernels, PairEntity, PairParams};
    let reach: Vec<f64> = ents.iter().map(|o| if o.sensors.is_empty() || o.dummy { -1.0 } else { o.sensors.iter().map(|x| x.1).fold(0.0, f64::max).min(max_r) }).collect();
    let pe: Vec<PairEntity> = ents
        .iter()
        .zip(&reach)
        .map(|(x, &r)| {
            let own = side_layer(x.side);
            PairEntity {
                x: x.pos.e,
                y: x.pos.n,
                z: 0.0,
                layer: own,
                sees_layers: if r >= 0.0 { 0b111 & !(1u32 << own) } else { 0 },
                sensor_range: if r >= 0.0 { r * (1.0 + 1e-9) + 1e-6 } else { 0.0 },
                heading: 0.0,
                half_fov: std::f64::consts::PI,
            }
        })
        .collect();
    let params = PairParams { cell_m: 0.0, ..PairParams::from_figures(&s.ctx.figures) };
    s.ctx
        .ground
        .backend
        .candidate_pairs(&pe, &params)
        .into_iter()
        .filter_map(|c| {
            let (oi, ti) = (c.observer as usize, c.target as usize);
            let r = ents[ti].pos.sub(ents[oi].pos).horizontal();
            (r <= reach[oi]).then_some((oi, ti, r))
        })
        .collect()
}

/// Sensing (§8.2): candidate pairs in range on the hash grid, the best
/// sensor's ACQUIRE/TTP detection rate, line of sight over the terrain for
/// the pairs that could detect (batched through the kernel layer, §11), then
/// keyed dice; recognition and identification are worked out only for the
/// detections. The work is ordered for speed; every draw and every number is
/// the one the plain per-pair loop gave.
pub fn sensing(s: &mut Sim, dt: f64) {
    let now = s.ctx.now;
    let ents = {
        prof_span!("sensing.snapshot");
        snapshot_for_sensing(s)
    };
    let lat = s.ctx.frame.origin.lat;
    let lon = s.ctx.frame.origin.lon;
    let cloud = s.ctx.weather.sample(0.0, 0.0, now).cloud_cover;
    let lux = bgws_env::illuminance_at(&s.ctx.start.at(now), lat, lon, cloud, &s.ctx.p.light).total_lux;
    let night = lux < s.ctx.p.night_lux;
    let max_r = s.ctx.p.sense_max_m;
    let pairs = {
        prof_span!("sensing.pairs");
        candidate_pairs(s, &ents, max_r)
    };
    let ground = &s.ctx.ground;
    let p = &s.ctx.p;
    let seed = s.ctx.seed;
    let world = &s.world;
    let cued = |o: &SenseEnt, t: &SenseEnt| world.get::<Aware>(o.e).map(|a| a.seen.contains_key(&t.id)).unwrap_or(false);
    // 1. The best sensor per pair and its first die (no terrain yet).
    let best: Vec<Option<Best>> = {
        prof_span!("sensing.acquire");
        pairs
            .par_iter()
            .map(|&(oi, ti, range)| {
                let o = &ents[oi];
                let t = &ents[ti];
                let w = s.ctx.weather.sample((o.pos.e + t.pos.e) / 2.0, (o.pos.n + t.pos.n) / 2.0, now);
                let cued = cued(o, t);
                let tau_mult = tau_multiplier(&o.observer, &p.observer);
                let mut best: Option<Best> = None;
                for &(kind, rmax, res, n50) in &o.sensors {
                    if range > rmax {
                        continue;
                    }
                    let (contrast, mut res, sk) = match kind {
                        CatSensor::Thermal => (Contrast::Thermal { delta_t_k: t.delta_t * (1.0 - 0.7 * t.concealment), transmission: transmission(OpticalBand::Lwir, range, &w, &p.atmos) }, res, SensorKind::Thermal),
                        _ => (Contrast::Visual { inherent: t.contrast * (1.0 - t.concealment), transmission: transmission(OpticalBand::Visible, range, &w, &p.atmos) }, res, SensorKind::Visual),
                    };
                    if night && matches!(kind, CatSensor::Eye | CatSensor::Optic | CatSensor::Eo) {
                        res *= p.eye_night_factor;
                    }
                    let mut ap = p.acquire.clone();
                    ap.n50 = n50;
                    let input = AcquireInput {
                        critical_dim_m: t.crit,
                        range_m: range,
                        cycles_per_mrad: res,
                        metric: Metric::Johnson,
                        atmos_mtf: 1.0,
                        contrast,
                        stage: Stage::Detect,
                        search: Search { fov_deg: p.search_fov_deg, cued, observer_tau_mult: tau_mult },
                    };
                    let det = acquire(&input, &ap);
                    if best.as_ref().map(|b| det.rate_per_s > b.rate).unwrap_or(true) {
                        best = Some(Best { rate: det.rate_per_s, sensor: sk, contrast, res, n50, u: 1.0 });
                    }
                }
                // The pair's first die, drawn now from its keyed stream (the
                // same value the dice below would draw). Line of sight only
                // lowers the chance, so a die that fails with a clear line
                // fails anyway: those pairs need no terrain at all.
                let b = best.filter(|b| b.rate > 0.0)?;
                let mut rng = seed.stream(StreamKey::new("sense", o.id.0 as u64, t.id.0 as u64, now.0));
                let u: f64 = rng.gen();
                (u < hazard_probability(b.rate, dt)).then_some(Best { u, ..b })
            })
            .collect()
    };
    // 2. Line of sight for the pairs whose die could still pass, in one batch.
    let live: Vec<usize> = (0..pairs.len()).filter(|&i| best[i].is_some()).collect();
    let los = {
        prof_span!("sensing.los");
        let queries: Vec<bgws_gpu::LosQuery> = live
            .iter()
            .map(|&i| {
                let (o, t) = (&ents[pairs[i].0], &ents[pairs[i].1]);
                bgws_gpu::LosQuery { obs_x: o.pos.e, obs_y: o.pos.n, obs_agl: o.eye_h + o.agl, tgt_x: t.pos.e, tgt_y: t.pos.n, tgt_agl: 1.5 + t.agl }
            })
            .collect();
        ground.los_batch(&queries)
    };
    // 3. The dice, then recognition and identification for detections.
    let sights: Vec<Sight> = {
        prof_span!("sensing.dice");
        live.par_iter()
            .zip(los.par_iter())
            .filter_map(|(&i, los)| {
                if los.p_visible <= 0.0 {
                    return None;
                }
                let (oi, ti, range) = pairs[i];
                let (o, t) = (&ents[oi], &ents[ti]);
                let b = best[i].as_ref()?;
                let pdet = los.p_visible * hazard_probability(b.rate, dt);
                if b.u >= pdet {
                    return None;
                }
                let mut rng = seed.stream(StreamKey::new("sense", o.id.0 as u64, t.id.0 as u64, now.0));
                let _first: f64 = rng.gen();
                let mut ap = p.acquire.clone();
                ap.n50 = b.n50;
                let search = Search { fov_deg: p.search_fov_deg, cued: cued(o, t), observer_tau_mult: tau_multiplier(&o.observer, &p.observer) };
                let input = |stage| AcquireInput { critical_dim_m: t.crit, range_m: range, cycles_per_mrad: b.res, metric: Metric::Johnson, atmos_mtf: 1.0, contrast: b.contrast, stage, search };
                let rec = acquire(&input(Stage::Recognise), &ap).p_inf;
                let idn = acquire(&input(Stage::Identify), &ap).p_inf;
                let u: f64 = rng.gen();
                let level = if u < idn { 2 } else if u < rec { 1 } else { 0 };
                Some(Sight { obs: oi, tgt: ti, p: pdet, level, range, sensor: b.sensor })
            })
            .collect()
    };
    // Apply in pair order.
    let mut detections: Vec<(Side, Detection)> = Vec::new();
    for sg in sights {
        let o = &ents[sg.obs];
        let t = &ents[sg.tgt];
        let new = {
            let mut aw = s.world.get_mut::<Aware>(o.e).expect("aware");
            let fresh = !aw.seen.contains_key(&t.id);
            let entry = aw.seen.entry(t.id).or_insert(Seen { first: now, last: now, level: sg.level, at: t.pos, moving: t.moving });
            entry.last = now;
            entry.level = entry.level.max(sg.level);
            entry.at = t.pos;
            entry.moving = t.moving;
            aw.suspects.remove(&t.id);
            fresh
        };
        if new {
            { let __text = format!("{} spots {} at {:.0} m", name(s, o.id), name(s, t.id), sg.range); s.ctx.log.push(now, "sense.detect").side(o.side).entity(o.id).target(t.id).text(__text).data(serde_json::json!({ "p": sg.p, "level": sg.level, "sensor": format!("{:?}", sg.sensor) })); }
            s.ctx.triggers.push(Trigger { entity: o.id, point: "D1", about: Some(t.id) });
            if let Some(mut a) = s.world.get_mut::<Act>(o.e) {
                a.last_event = now;
                // React to contact: a marching unit drops to tactical, and fires.
                if let Activity::Move { mode, .. } = &mut a.activity {
                    if *mode == MoveModeS::March {
                        *mode = MoveModeS::Tactical;
                    }
                }
            }
        }
        let level = match sg.level {
            0 => IdLevel::Detect,
            1 => IdLevel::Recognise,
            _ => IdLevel::Identify,
        };
        let sigma = p.fix_sigma_m * (1.0 + sg.range / 2000.0);
        let mut rng = s.ctx.rng("sense.report", o.id.0 as u64, t.id.0 as u64);
        let noise = Enu::new(rng.gen::<f64>() * 2.0 - 1.0, rng.gen::<f64>() * 2.0 - 1.0, 0.0).scale(sigma * 1.2);
        let reported = if level == IdLevel::Detect { None } else { Some(if t.dummy { Hypothesis::Mbt } else { t.hypothesis }) };
        detections.push((
            o.side,
            Detection {
                time: now,
                observer: o.id,
                sensor: sg.sensor,
                domain: bgws_know::Domain::Land,
                motion: if t.moving { MotionHint::Moving } else { MotionHint::Static },
                measurement: Measurement::Position { pos: t.pos.add(noise), cov: Matrix3::from_diagonal_element(sigma * sigma) },
                class: ClassEvidence { level, reported, resolves_decoys: sg.level >= 2 && sg.range < 1500.0 },
                truth: TruthTag::of(t.id),
            },
        ));
    }
    // Forget stale sightings.
    let forget = s.ctx.p.forget_s;
    for &e in &s.ctx.order.clone() {
        if let Some(mut aw) = s.world.get_mut::<Aware>(e) {
            aw.seen.retain(|_, v| (now - v.last).secs() <= forget);
        }
    }
    // Reports reach the side picture after the report delay (no nets modelled
    // here; the comms system replaces this when the scope has comms).
    let delay = bgws_core::SimDuration::from_secs(s.ctx.p.report_delay_s);
    for (side, d) in detections {
        s.ctx.reports.push((now + delay, side, d));
    }
}

/// Why `observer` would see `target` now or not, with every factor the
/// sensing system uses (§16 detection inspector): light, each sensor's
/// contrast, resolution and ACQUIRE terms, the line of sight through terrain
/// and canopy, and the chance of detection over `over_s` seconds. Read-only:
/// no dice are drawn. Truth, so for the umpire only.
pub fn explain_detection(s: &Sim, observer: EntityId, target: EntityId, over_s: f64) -> Option<serde_json::Value> {
    use serde_json::json;
    let now = s.ctx.now;
    let ents = snapshot_for_sensing(s);
    let o = ents.iter().find(|x| x.id == observer)?;
    let t = ents.iter().find(|x| x.id == target)?;
    let p = &s.ctx.p;
    let range = o.pos.sub(t.pos).horizontal();
    let (lat, lon) = (s.ctx.frame.origin.lat, s.ctx.frame.origin.lon);
    let w = s.ctx.weather.sample((o.pos.e + t.pos.e) / 2.0, (o.pos.n + t.pos.n) / 2.0, now);
    let lux = bgws_env::illuminance_at(&s.ctx.start.at(now), lat, lon, w.cloud_cover, &p.light).total_lux;
    let night = lux < p.night_lux;
    let cued = s.world.get::<Aware>(o.e).map(|a| a.seen.contains_key(&t.id)).unwrap_or(false);
    let tau_mult = tau_multiplier(&o.observer, &p.observer);
    let mut sensors = Vec::new();
    let mut best_rate = 0.0f64;
    for &(kind, rmax, res0, n50) in &o.sensors {
        let in_range = range <= rmax && range <= p.sense_max_m;
        let (contrast, mut res) = match kind {
            CatSensor::Thermal => (Contrast::Thermal { delta_t_k: t.delta_t * (1.0 - 0.7 * t.concealment), transmission: transmission(OpticalBand::Lwir, range, &w, &p.atmos) }, res0),
            _ => (Contrast::Visual { inherent: t.contrast * (1.0 - t.concealment), transmission: transmission(OpticalBand::Visible, range, &w, &p.atmos) }, res0),
        };
        let night_degraded = night && matches!(kind, CatSensor::Eye | CatSensor::Optic | CatSensor::Eo);
        if night_degraded {
            res *= p.eye_night_factor;
        }
        let mut ap = p.acquire.clone();
        ap.n50 = n50;
        let input = AcquireInput {
            critical_dim_m: t.crit,
            range_m: range,
            cycles_per_mrad: res,
            metric: Metric::Johnson,
            atmos_mtf: 1.0,
            contrast,
            stage: Stage::Detect,
            search: Search { fov_deg: p.search_fov_deg, cued, observer_tau_mult: tau_mult },
        };
        let det = acquire(&input, &ap);
        if in_range {
            best_rate = best_rate.max(det.rate_per_s);
        }
        sensors.push(json!({ "sensor": format!("{kind:?}"), "maxRangeM": rmax, "inRange": in_range, "cyclesPerMrad": res, "nightDegraded": night_degraded, "contrast": contrast_json(&contrast), "acquire": det }));
    }
    let los = s.ctx.ground.los_batch(&[bgws_gpu::LosQuery { obs_x: o.pos.e, obs_y: o.pos.n, obs_agl: o.eye_h + o.agl, tgt_x: t.pos.e, tgt_y: t.pos.n, tgt_agl: 1.5 + t.agl }]).pop()?;
    let p_window = los.p_visible * hazard_probability(best_rate, over_s);
    Some(json!({
        "observer": name(s, observer), "target": name(s, target), "rangeM": range, "atS": now.secs(),
        "light": { "lux": lux, "night": night }, "visibilityM": w.visibility_m,
        "target_signature": { "criticalDimM": t.crit, "contrast": t.contrast, "deltaTK": t.delta_t, "concealment": t.concealment, "moving": t.moving },
        "observerState": { "cued": cued, "tauMultiplier": tau_mult, "eyeHeightM": o.eye_h + o.agl, "searchFovDeg": p.search_fov_deg, "maxSearchRangeM": p.sense_max_m },
        "lineOfSight": { "terrainVisible": los.terrain_visible, "clearanceM": los.clearance_m, "pClearOfCanopy": los.p_clear, "pVisible": los.p_visible, "canopyPathM": los.canopy_path_m },
        "sensors": sensors,
        "bestRatePerS": best_rate,
        "windowS": over_s,
        "pDetectInWindow": p_window,
        "alreadySeen": cued,
    }))
}

fn contrast_json(c: &Contrast) -> serde_json::Value {
    match c {
        Contrast::Visual { inherent, transmission } => serde_json::json!({ "kind": "visual", "inherent": inherent, "transmission": transmission }),
        Contrast::Thermal { delta_t_k, transmission } => serde_json::json!({ "kind": "thermal", "deltaTK": delta_t_k, "transmission": transmission }),
    }
}

/// Deliver due contact reports into each side's track store.
pub fn reports(s: &mut Sim) {
    let now = s.ctx.now;
    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut s.ctx.reports).into_iter().partition(|(t, _, _)| *t <= now);
    s.ctx.reports = later;
    let mut by_side: std::collections::BTreeMap<Side, Vec<Detection>> = Default::default();
    for (_, side, d) in due {
        by_side.entry(side).or_default().push(d);
    }
    for side in Side::PLAYERS {
        let p = &s.ctx.p.know;
        let store = s.ctx.tracks.get_mut(&side).expect("side");
        if let Some(dets) = by_side.remove(&side) {
            store.ingest(dets, p);
        }
        store.predict_to(now, p);
    }
}

pub(crate) fn name(s: &Sim, id: EntityId) -> String {
    s.world.get::<Ident>(s.ctx.entity(id)).map(|i| i.label.clone()).unwrap_or_else(|| id.to_string())
}

fn warhead_of(m: &bgws_catalogue::Munition) -> Warhead {
    use bgws_catalogue::MunitionKind as K;
    match m.kind {
        K::Ke => Warhead::Kinetic,
        _ if m.tandem => Warhead::TandemShapedCharge,
        _ => Warhead::ShapedCharge,
    }
}

fn armour_zones(p: &bgws_catalogue::Platform) -> ArmourZones {
    let a = &p.body.armour;
    let z = |name: &str, ke: f64, ce: f64| a.zones.get(name).map(|z| (z.ke.front.max(ke), z.ce.front.max(ce))).unwrap_or((ke, ce));
    let (tf_ke, tf_ce) = z("turret", a.ke.front * 1.2, a.ce.front * 1.2);
    // Zone order: HullFront, TurretFront, HullSide, TurretSide, Rear, Roof.
    ArmourZones { ke_mm: [a.ke.front, tf_ke, a.ke.side, a.ke.side * 1.2, a.ke.rear, a.ke.top], ce_mm: [a.ce.front, tf_ce, a.ce.side, a.ce.side * 1.2, a.ce.rear, a.ce.top] }
}

/// Direct fire (§8.8): each ready firer with a target it has seen, in reach
/// and allowed by its ROE and activity, fires a volley: an error budget with
/// a per-engagement bias, the hit probability over the presented area, then
/// penetration and kill type by zone; misses close by suppress.
pub fn direct_fire(s: &mut Sim) {
    let now = s.ctx.now;
    let order = s.ctx.order.clone();
    // Fire within a tick is simultaneous: every firer acts on its state at the
    // start of the tick, so being earlier in the id order is no advantage.
    let start: Vec<Option<(bool, u32)>> = order.iter().map(|&e| s.world.get::<Health>(e).map(|h| (h.can_fire(), h.fit))).collect();
    for (k, &e) in order.iter().enumerate() {
        let Some((could_fire, fit0)) = start[k] else { continue };
        // Aggregated and Far units do not fight individually (§6.4).
        if !could_fire || !is_land(s, e) || lod(s, e) != Lod::Entity {
            continue;
        }
        // Nothing seen in the last 3 s means no target: the same test as the
        // candidate list below, made before anything is cloned.
        if !s.world.get::<Aware>(e).map(|a| a.seen.values().any(|v| v.last == now || (now - v.last).secs() < 3.0)).unwrap_or(false) {
            continue;
        }
        let (id, side, platform, quality) = {
            let i = s.world.get::<Ident>(e).expect("ident");
            (i.id, i.side, i.platform.clone(), i.quality)
        };
        let act = s.world.get::<Act>(e).expect("act").clone();
        if !act.activity.may_fire() || act.roe == RoeS::Never {
            continue;
        }
        let k = *s.world.get::<Kin>(e).expect("kin");
        let seen: Vec<(EntityId, Seen)> = s.world.get::<Aware>(e).expect("aware").seen.iter().filter(|(_, v)| v.last == now || (now - v.last).secs() < 3.0).map(|(k, v)| (*k, v.clone())).collect();
        // Pick a mount with rounds, ready.
        let arms = s.world.get::<Arms>(e).expect("arms").clone();
        // Indirect-fire mounts fire missions (domains::fires), not direct volleys.
        let Some(mount) = arms.mounts.iter().find(|m| m.rounds > 0 && m.ready_at <= now && !crate::domains::fires::indirect_mount(&platform, m.index)) else { continue };
        let Some(mun) = s.ctx.catalogue.munition(&mount.munition).cloned() else { continue };
        let cat_mount = &platform.mounts[mount.index];
        // Target: the ordered one if seen, else the nearest seen in reach.
        let fired_upon = s.world.get::<Crew>(e).expect("crew").last_incoming.is_some();
        let roe_ok = |r: f64| match act.roe {
            RoeS::Always => true,
            RoeS::IfFiredUpon => fired_upon,
            RoeS::WithinShortRange => r <= mun.max_range_m * 0.5,
            RoeS::Never => false,
        };
        let wanted = match &act.activity {
            Activity::Engage { target, .. } => Some(*target),
            Activity::Wait { target, met: true, .. } => Some(*target),
            _ => None,
        };
        let mut candidates: Vec<(EntityId, f64)> = seen
            .iter()
            .map(|(t, v)| (*t, v.at.sub(k.pos).horizontal()))
            .filter(|(t, r)| *r <= mun.max_range_m && *r >= mun.min_range_m && roe_ok(*r) && alive(s, s.ctx.entity(*t)))
            .collect();
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let target = wanted.and_then(|w| candidates.iter().find(|c| c.0 == w).copied()).or_else(|| if matches!(act.activity, Activity::Move { mode: MoveModeS::March, .. }) { None } else { candidates.first().copied() });
        let Some((tid, range)) = target else { continue };
        fire_volley(s, e, id, side, fit0, quality, mount.index, &mun, cat_mount, tid, range);
    }
}

#[allow(clippy::too_many_arguments)]
fn fire_volley(s: &mut Sim, e: Entity, id: EntityId, side: Side, fit: u32, _quality: u8, mount_i: usize, mun: &bgws_catalogue::Munition, cat_mount: &bgws_catalogue::Mount, tid: EntityId, range: f64) {
    let now = s.ctx.now;
    let te = s.ctx.entity(tid);
    let tk = *s.world.get::<Kin>(te).expect("kin");
    let k = *s.world.get::<Kin>(e).expect("kin");
    let tplat = s.world.get::<Ident>(te).expect("ident").platform.clone();
    let tposture = s.world.get::<Act>(te).expect("act").posture;
    let tcover = s.ctx.ground.micro(tk.pos.e, tk.pos.n).cover;
    let p = s.ctx.p.direct.clone();
    let crew = s.world.get::<Crew>(e).expect("crew").clone();
    let skill = crew.skill.error_mult(&s.ctx.p.people);
    let supp = crew.suppression.level;
    let fc: FireControl = FireControl::from_quality(cat_mount.fire_control.quality, &p);
    let gun: Gun = if mun.kind == bgws_catalogue::MunitionKind::SmallArms { p.rifle } else { Gun { muzzle_velocity_mps: mun.speed_ms.max(100.0), ..p.mbt_gun } };
    let firer_moving = matches!(s.world.get::<Act>(e).expect("act").posture, Posture::Moving);
    let rel = tk.pos.sub(k.pos);
    let los_dir = if rel.horizontal() > 0.0 { rel.scale(1.0 / rel.horizontal()) } else { Enu::new(0.0, 1.0, 0.0) };
    let crossing = (tk.vel.e * los_dir.n - tk.vel.n * los_dir.e).abs();
    let shot = Shot { range_m: range, firer_moving: firer_moving && !cat_mount.fire_control.stabilised, target_crossing_mps: crossing, skill_mult: skill, suppression: supp };
    let budget = df::error_budget(&gun, &fc, &shot, &p);
    let soft = matches!(tplat.body.vulnerability, Vulnerability::Personnel);
    let aspect = (rel.bearing() + std::f64::consts::PI - tk.heading).rem_euclid(std::f64::consts::TAU);
    let pres = if soft {
        df::presented_person(if tposture == Posture::Moving { PersonPosture::Kneeling } else { PersonPosture::Prone }, tcover, &p)
    } else {
        let b = &tplat.body;
        let dims = VehicleDims { length_m: b.length_m.max(3.0), width_m: b.width_m.max(2.0), hull_height_m: (b.height_m * 0.6).max(1.0), turret_height_m: (b.height_m * 0.4).max(0.3) };
        df::presented_vehicle(&dims, aspect, if tposture == Posture::HullDown { VehiclePosture::HullDown } else { VehiclePosture::Exposed }, &p)
    };
    let mut rng = s.ctx.rng("fire", id.0 as u64, tid.0 as u64);
    let rounds = ((fit as f64 * s.ctx.p.rounds_per_volley).round() as u32).max(1).min(s.world.get::<Arms>(e).expect("arms").mounts.iter().find(|m| m.index == mount_i).map(|m| m.rounds).unwrap_or(0));
    if rounds == 0 {
        return;
    }
    let existing = s.world.get::<Arms>(e).expect("arms").engagements.get(&tid).cloned();
    let Some(mut engagement) = existing else {
        // A new target: the crew lays on it before the first round (§8.8).
        let lay = cat_mount.fire_control.lay_s.max(s.ctx.p.lay_min_s) * (1.0 + supp);
        let fresh = df::Engagement::new(&budget, &mut rng);
        let mut arms = s.world.get_mut::<Arms>(e).expect("arms");
        arms.engagements.insert(tid, fresh);
        if let Some(m) = arms.mounts.iter_mut().find(|m| m.index == mount_i) {
            m.ready_at = now + bgws_core::SimDuration::from_secs(lay);
        }
        return;
    };
    let (mut hits, mut kills, mut near) = (0u32, 0u32, 0.0f64);
    let mut ph_last = 0.0;
    let armour = armour_zones(&tplat);
    let prot = Protection { era: tplat.body.era, aps: tplat.body.aps.is_some() };
    let pen = mun.penetration_at(range);
    for _ in 0..rounds {
        let r = engagement.fire(&pres, &budget, &p, &mut rng);
        ph_last = r.ph;
        if !r.hit {
            if r.x_m.abs() < p.near_miss_radius_m * 3.0 && r.y_m.abs() < p.near_miss_radius_m * 3.0 {
                near += s.ctx.p.near_miss_per_round;
            }
            continue;
        }
        hits += 1;
        let killed = if soft || matches!(tplat.body.vulnerability, Vulnerability::Soft | Vulnerability::Light) && pen > 50.0 {
            let pk = if soft { mun.pk.max(0.3) } else { 0.8 };
            rng.gen::<f64>() < pk
        } else {
            let zone: Zone = df::zone_at(&pres, r.x_frac, r.y_frac, mun.top_attack);
            let odds = df::strike_odds(warhead_of(mun), pen, zone, &armour, &prot, if mun.top_attack { 45.0 } else { 0.0 }, &p);
            match df::roll_strike(&odds, &mut rng) {
                StrikeOutcome::Penetrated(kt) => {
                    let mut h = s.world.get_mut::<Health>(te).expect("h");
                    if kt.catastrophic || kt.crew || (kt.firepower && kt.mobility) {
                        true
                    } else {
                        if kt.mobility {
                            h.mobility_kills += 1;
                        }
                        if kt.firepower {
                            h.firepower_kills += 1;
                        }
                        false
                    }
                }
                _ => {
                    near += 1.0;
                    false
                }
            }
        };
        if killed {
            kills += 1;
            near += 2.0;
        }
    }
    s.world.get_mut::<Arms>(e).expect("arms").engagements.insert(tid, engagement);
    {
        let mut arms = s.world.get_mut::<Arms>(e).expect("arms");
        if let Some(m) = arms.mounts.iter_mut().find(|m| m.index == mount_i) {
            m.rounds -= rounds;
            m.ready_at = now + bgws_core::SimDuration::from_secs(cat_mount.reload_s.max(4.0));
        }
    }
    {
        let mut a = s.world.get_mut::<Act>(e).expect("act");
        a.last_shot = Some(now);
        a.last_event = now;
        a.volleys += 1;
        if hits > 0 {
            a.volleys = 0;
        }
        if let Activity::Engage { volleys: Some(v), .. } = &mut a.activity {
            *v = v.saturating_sub(1);
        }
    }
    s.world.get_mut::<SigState>(e).expect("sig").last_fired = Some(now);
    // Effects on the target.
    let mut destroyed_now = false;
    if kills > 0 {
        let mut h = s.world.get_mut::<Health>(te).expect("h");
        let k = kills.min(h.fit);
        h.fit -= k;
        for _ in 0..k {
            h.losses.push(now);
        }
        if h.fit == 0 {
            h.destroyed = true;
            destroyed_now = true;
        }
        let frac = k as f64 / h.start as f64;
        s.world.get_mut::<Crew>(te).expect("crew").new_casualties += frac;
    }
    {
        let mut c = s.world.get_mut::<Crew>(te).expect("crew");
        c.near_misses_per_min += near * 60.0 / 10.0;
        c.last_incoming = Some((now, id));
    }
    { let __text = format!(
        "{} fires {} rounds at {} ({:.0} m): {} hits, {} knocked out",
        name(s, id),
        rounds,
        name(s, tid),
        range,
        hits,
        kills
    ); s.ctx.log.push(now, if kills > 0 { "fire.kill" } else if hits > 0 { "fire.hit" } else { "fire.miss" }).side(side).entity(id).target(tid).text(__text).data(serde_json::json!({ "munition": mun.id, "ph": ph_last, "biasSigma": [budget.bias_sigma_x_m, budget.bias_sigma_y_m], "penetrationMm": pen, "rounds": rounds, "rangeM": range, "hits": hits, "kills": kills })); }
    if destroyed_now {
        { let __text = format!("{} is destroyed", name(s, tid)); s.ctx.log.push(now, "unit.destroyed").side(side.enemy()).entity(tid).text(__text); }
        s.world.get_mut::<Crew>(e).expect("crew").success = true;
        // The shooter's target is gone (D7); friends nearby lost a friend.
        s.ctx.triggers.push(Trigger { entity: id, point: "D7", about: Some(tid) });
    } else {
        // Under fire: located (D2) if it has seen the shooter, else a bearing (D3).
        let located = s.world.get::<Aware>(te).map(|a| a.seen.contains_key(&id)).unwrap_or(false);
        if !located {
            let mut aw = s.world.get_mut::<Aware>(te).expect("aware");
            let bearing = k.pos.sub(tk.pos).bearing();
            let entry = aw.suspects.entry(id).or_insert(Suspect { bearing, since: now, last: now });
            entry.last = now;
            entry.bearing = bearing;
        }
        s.ctx.triggers.push(Trigger { entity: tid, point: if located { "D2" } else { "D3" }, about: Some(id) });
        let ineffective = s.world.get::<Act>(e).expect("act").volleys >= s.ctx.p.ineffective_volleys;
        if ineffective {
            s.world.get_mut::<Act>(e).expect("act").volleys = 0;
            s.ctx.triggers.push(Trigger { entity: id, point: "D8", about: Some(tid) });
        } else if hits > 0 {
            s.ctx.triggers.push(Trigger { entity: id, point: "D6", about: Some(tid) });
        }
    }
}

/// People (§8.10): suppression from incoming fire, morale from casualties,
/// suppression, isolation and success; cohesion changes run the unit itself.
pub fn people(s: &mut Sim, dt: f64) {
    let now = s.ctx.now;
    let order = s.ctx.order.clone();
    // Living friend positions for isolation, on a grid of isolation-range cells.
    let iso = s.ctx.p.isolation_m;
    let friends = FriendGrid::new(order.iter().enumerate().filter(|(_, &e)| alive(s, e)).filter_map(|(j, &e)| Some((j, s.world.get::<Ident>(e)?.side, s.world.get::<Kin>(e)?.pos))), iso);
    let pp = s.ctx.p.people.clone();
    for (idx, &e) in order.iter().enumerate() {
        // An aggregated formation carries its people as distributions (§6.4).
        if !alive(s, e) || !is_land(s, e) || lod(s, e) == Lod::Aggregated {
            continue;
        }
        let (id, side, quality) = {
            let i = s.world.get::<Ident>(e).expect("i");
            (i.id, i.side, i.quality)
        };
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        let cover = s.ctx.ground.micro(pos.e, pos.n).cover;
        let isolated = !friends.any_within(idx, side, pos, iso);
        let q = UnitQuality { leadership: (quality as f64 - 1.0) / 5.0, training: (quality as f64 - 1.0) / 5.0 };
        let mut rng = s.ctx.rng("morale", id.0 as u64, 0);
        let observed = crate::domains::drones::drone_observed(s, id);
        let change = {
            let mut c = s.world.get_mut::<Crew>(e).expect("crew");
            let incoming = IncomingFire { near_misses_per_min: c.near_misses_per_min, blasts_per_min: c.blasts_per_min };
            c.suppression.step(dt, &incoming, cover, &pp);
            // Fire felt this tick fades over a minute.
            let fade = (-dt / 60.0f64).exp();
            c.near_misses_per_min *= fade;
            c.blasts_per_min *= fade;
            let inputs = StressInputs { new_casualties: c.new_casualties, suppression: c.suppression.level, isolated, surprised: false, drone_observed: observed, success: c.success };
            c.new_casualties = 0.0;
            c.success = false;
            if c.last_incoming.map(|(t, _)| (now - t).secs() > 120.0).unwrap_or(false) {
                c.last_incoming = None;
            }
            c.morale.step(dt, &inputs, &q, &pp, &mut rng)
        };
        if let Some(ch) = change {
            { let __text = format!("{} is {:?} (was {:?})", name(s, id), ch.to, ch.from); s.ctx.log.push(now, "people.morale").side(side).entity(id).text(__text).data(serde_json::json!({ "stress": ch.stress, "casualties": ch.casualties })); }
            match ch.to {
                MoraleState::Broken | MoraleState::Routed => {
                    // Runs itself: falls back away from the threat.
                    let away = threat_bearing(s, e).map(|b| b + std::f64::consts::PI).unwrap_or(match side {
                        Side::Blue => std::f64::consts::PI,
                        _ => 0.0,
                    });
                    let to = pos.add(Enu::new(away.sin(), away.cos(), 0.0).scale(s.ctx.p.withdraw_m * 1.5));
                    let mut a = s.world.get_mut::<Act>(e).expect("act");
                    a.activity = Activity::Withdraw { route: vec![to], then: Some(Box::new(Activity::Quiet)) };
                    a.roe = RoeS::IfFiredUpon;
                }
                MoraleState::Steady if matches!(ch.from, MoraleState::Broken | MoraleState::Routed | MoraleState::Shaken) => {
                    s.ctx.triggers.push(Trigger { entity: id, point: "D11", about: None });
                }
                _ => {}
            }
        }
    }
}

/// Friends by grid cell, for "is any living friend within r" in O(k) instead
/// of a scan of every unit (the same answer).
struct FriendGrid {
    cell: f64,
    /// ((cell x, cell y), order index, side, position), sorted by cell then index.
    bins: Vec<((i64, i64), usize, Side, Enu)>,
}

impl FriendGrid {
    fn new(items: impl Iterator<Item = (usize, Side, Enu)>, r: f64) -> Self {
        let cell = r.max(1.0);
        let mut bins: Vec<((i64, i64), usize, Side, Enu)> = items.map(|(j, side, p)| (((p.e / cell).floor() as i64, (p.n / cell).floor() as i64), j, side, p)).collect();
        bins.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        FriendGrid { cell, bins }
    }

    /// Is a unit of `side` other than `me` within `r` of `pos`? (`r` at most the cell.)
    fn any_within(&self, me: usize, side: Side, pos: Enu, r: f64) -> bool {
        let (cx, cy) = ((pos.e / self.cell).floor() as i64, (pos.n / self.cell).floor() as i64);
        for gx in cx - 1..=cx + 1 {
            let lo = self.bins.partition_point(|b| b.0 < (gx, cy - 1));
            let hi = self.bins.partition_point(|b| b.0 <= (gx, cy + 1));
            if self.bins[lo..hi].iter().any(|b| b.1 != me && b.2 == side && b.3.sub(pos).horizontal() <= r) {
                return true;
            }
        }
        false
    }
}

/// Bearing to the nearest thing that threatens a unit (seen enemy or suspect).
pub(crate) fn threat_bearing(s: &Sim, e: Entity) -> Option<f64> {
    let pos = s.world.get::<Kin>(e)?.pos;
    let aw = s.world.get::<Aware>(e)?;
    let mut best: Option<(f64, f64)> = None;
    for v in aw.seen.values() {
        let d = v.at.sub(pos);
        if best.map(|b| d.horizontal() < b.0).unwrap_or(true) {
            best = Some((d.horizontal(), d.bearing()));
        }
    }
    best.map(|b| b.1).or_else(|| aw.suspects.values().next().map(|x| x.bearing))
}

/// Fatigue and sleep debt (§8.10), each minute.
pub fn fatigue(s: &mut Sim, dt: f64) {
    let now = s.ctx.now;
    let pp = s.ctx.p.people.clone();
    for &e in &s.ctx.order.clone() {
        let busy = s.world.get::<Act>(e).map(|a| a.last_shot.map(|t| (now - t).secs() < 300.0).unwrap_or(false) || a.activity.moving()).unwrap_or(false);
        if let Some(mut c) = s.world.get_mut::<Crew>(e) {
            c.fatigue.step(dt / 3600.0, if busy { WorkLoad::Combat } else { WorkLoad::Light }, &pp);
        }
    }
}

/// The autopilot's own events: a waiting unit's trigger met (D4), an
/// engagement finished, a target lost, nothing happening for too long (D10).
pub fn autopilot_events(s: &mut Sim) {
    let now = s.ctx.now;
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) || !is_land(s, e) {
            continue;
        }
        let id = s.world.get::<Ident>(e).expect("i").id;
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        let act = s.world.get::<Act>(e).expect("a").clone();
        let seen_target = |t: EntityId| s.world.get::<Aware>(e).and_then(|a| a.seen.get(&t).cloned());
        match &act.activity {
            Activity::Wait { target, trigger, auto_fire, met: false } => {
                if let Some(v) = seen_target(*target) {
                    let r = v.at.sub(pos).horizontal();
                    let met = match trigger {
                        WaitTrigger::Range { within_m } => r <= *within_m,
                        WaitTrigger::Flank => {
                            let tk = s.world.get::<Kin>(s.ctx.entity(*target)).expect("k");
                            let aspect = (pos.sub(v.at).bearing() - tk.heading).rem_euclid(std::f64::consts::TAU);
                            (aspect - std::f64::consts::PI).abs() < 2.4
                        }
                    };
                    if met {
                        let mut a = s.world.get_mut::<Act>(e).expect("a");
                        if *auto_fire {
                            a.activity = Activity::Engage { target: *target, volleys: None, then: Some(Box::new(Activity::Overwatch)) };
                        } else {
                            if let Activity::Wait { met, .. } = &mut a.activity {
                                *met = true;
                            }
                            s.ctx.triggers.push(Trigger { entity: id, point: "D4", about: Some(*target) });
                        }
                    }
                }
            }
            Activity::Engage { target, volleys, then } => {
                let gone = !alive(s, s.ctx.entity(*target)) || seen_target(*target).is_none();
                let done = volleys.map(|v| v == 0).unwrap_or(false);
                if gone || done {
                    let next = then.clone().map(|b| *b).unwrap_or(Activity::Overwatch);
                    s.world.get_mut::<Act>(e).expect("a").activity = next;
                    if gone && alive(s, s.ctx.entity(*target)) {
                        s.ctx.triggers.push(Trigger { entity: id, point: "D7", about: Some(*target) });
                    }
                }
            }
            Activity::Search { until, .. } if *until <= now => {
                s.world.get_mut::<Act>(e).expect("a").activity = Activity::Overwatch;
                s.ctx.triggers.push(Trigger { entity: id, point: "D3", about: None });
            }
            _ => {}
        }
        if (now - act.last_event).secs() >= s.ctx.p.idle_s && !act.activity.moving() {
            s.world.get_mut::<Act>(e).expect("a").last_event = now;
            s.ctx.triggers.push(Trigger { entity: id, point: "D10", about: None });
        }
    }
}

/// Victory (§10, scenario): breakpoints, the time limit, or one side gone.
pub fn victory(s: &mut Sim) {
    if s.ctx.over.is_some() {
        return;
    }
    let now = s.ctx.now;
    let strength = s.strength();
    let mut result: Option<(Option<Side>, String)> = None;
    for side in Side::PLAYERS {
        let st = strength.get(&side).copied().unwrap_or(0.0);
        let has_forces = s.ctx.scenario.forces_of(side).next().is_some();
        if has_forces && st <= 0.0 {
            result = Some((Some(side.enemy()), format!("{side} has nothing left")));
            break;
        }
        if let Some(pct) = s.ctx.scenario.breakpoint_pct(side) {
            if has_forces && st <= 1.0 - pct / 100.0 {
                result = Some((Some(side.enemy()), format!("{side} reached its breakpoint ({:.0}% lost)", (1.0 - st) * 100.0)));
                break;
            }
        }
    }
    if result.is_none() && now >= s.ctx.duration_limit() {
        let b = strength.get(&Side::Blue).copied().unwrap_or(0.0);
        let r = strength.get(&Side::Red).copied().unwrap_or(0.0);
        let winner = if (b - r).abs() < 0.05 { None } else if b > r { Some(Side::Blue) } else { Some(Side::Red) };
        result = Some((winner, "time limit".into()));
    }
    if let Some(r) = crate::world::victory(s) {
        result = result.or(Some(r));
    }
    if let Some((winner, reason)) = result {
        s.ctx.log.push(now, "sim.over").text(format!("Game over at {now}: {} ({reason})", winner.map(|w| format!("{w} wins")).unwrap_or_else(|| "draw".into()))).data(serde_json::json!({ "strength": strength }));
        s.ctx.over = Some(Outcome { at: now, winner, reason, strength });
    }
}

#[allow(dead_code)]
pub(crate) fn ctx_now(c: &Ctx) -> SimTime {
    c.now
}
