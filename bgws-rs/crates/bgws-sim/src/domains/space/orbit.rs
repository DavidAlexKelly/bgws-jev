//! Satellites, passes, imagery, GNSS and SATCOM (§8.1 Space, §8.2 SAR and EO
//! satellites, §8.12 Space; R5.3, R5.4).
//!
//! Each satellite is flown with SGP4 from elements: the scenario's
//! `OrbitPlacement` (phase and node) on the catalogue's circular orbit, or —
//! for an imaging satellite with no placement — the node and phase that put
//! its first pass over its side's area of interest early in the game (as the
//! TS engine places them). The elements are written as a TLE so that the one
//! SGP4 path serves both. Access windows over the area are found by
//! root-finding on elevation and scheduled as rise and set events.

use super::cyber::cyber_multiplier;
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_behaviour::FactValue;
use bgws_catalogue::{Domain, SensorKind as CatSensor};
use bgws_core::{EntityId, Geodetic, Side, SimDuration, SimTime};
use bgws_domains::orbit::{gmst, MU_EARTH};
use bgws_domains::space::{serve_pass, GnssGeometry, ImagerKind, ImageryParams, ImageryRequest, Pass, Satellite};
use bgws_know::{ClassEvidence, Detection, Hypothesis, IdLevel, Measurement, MotionHint, SensorKind, TruthTag};
use bgws_models::sensing::acquire::Stage;
use bgws_models::sensing::imagery::{image_identification, ImageKind};
use bgws_ops::cyber::TargetProcess;
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use nalgebra::Matrix3;
use rand::Rng;
use std::collections::BTreeMap;

const RE: f64 = 6_378_137.0;

/// What a satellite is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum SatKind {
    Eo,
    Sar,
    Warning,
    Other,
}

/// One satellite in orbit.
#[derive(Clone, Debug)]
pub struct SatRec {
    pub id: EntityId,
    pub side: Side,
    pub sat: Satellite,
    pub kind: SatKind,
    pub commercial: bool,
    pub gsd: f64,
    pub swath_m: f64,
    /// Passes still to come over its area of interest.
    pub passes: Vec<Pass>,
    pub site: Geodetic,
    /// Tasked for the pass under way.
    pub tasked: bool,
    pub alive: bool,
}

/// A batch of imagery on its way to the side (downlink and analysis).
#[derive(Clone, Debug)]
pub struct Batch {
    pub id: u64,
    pub side: Side,
    pub sat: EntityId,
    pub taken: SimTime,
    pub deliver: SimTime,
    pub detections: Vec<Detection>,
}

/// Counters for the AAR and the tests.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SpaceStats {
    pub passes: u32,
    pub images: u32,
    pub clouded: u32,
    pub detections_delivered: u32,
    pub latency_s: Vec<f64>,
}

/// The space domain's state.
#[derive(Resource)]
pub struct SpaceState {
    pub mode: DomainMode,
    pub start_jd: f64,
    pub sats: BTreeMap<EntityId, SatRec>,
    pub gnss: Vec<Satellite>,
    /// Each side's imagery requests (its tasking queue).
    pub queues: BTreeMap<Side, Vec<ImageryRequest>>,
    pub batches: BTreeMap<u64, Batch>,
    /// Named outages ("imagery:blue", "satcomLeo:red", "dazzle:<sat id>") until a time.
    pub outages: BTreeMap<String, SimTime>,
    /// GNSS quality by side under jamming, and until when.
    pub gnss_jam: BTreeMap<Side, (f64, SimTime)>,
    pub areas: BTreeMap<Side, Vec<(Geodetic, String, u8)>>,
    pub img: ImageryParams,
    pub next: u64,
    pub stats: SpaceStats,
    pub scheduled_to_jd: f64,
    /// Where each imagery request looks.
    pub req_sites: BTreeMap<u64, Geodetic>,
}

impl SpaceState {
    pub fn jd(&self, t: SimTime) -> f64 {
        self.start_jd + t.secs() / 86_400.0
    }
    pub fn time_of(&self, jd: f64) -> SimTime {
        SimTime::from_secs((jd - self.start_jd) * 86_400.0)
    }
    fn out(&self, key: &str, t: SimTime) -> bool {
        self.outages.get(key).map(|u| *u > t).unwrap_or(false)
    }
}

/// TLE checksum: digits plus one per minus sign, mod 10.
fn checksum(line: &str) -> u32 {
    line.chars().map(|c| if c == '-' { 1 } else { c.to_digit(10).unwrap_or(0) }).sum::<u32>() % 10
}

/// A two-line element set for a near-circular orbit at the scenario's start.
pub fn circular_tle(num: u32, start: &bgws_env::UtcDateTime, alt_m: f64, incl_deg: f64, raan_deg: f64, phase_deg: f64) -> (String, String) {
    let a = RE + alt_m;
    let n = (MU_EARTH / (a * a * a)).sqrt() * 86_400.0 / std::f64::consts::TAU;
    let doy = start.day_of_year() as f64 + start.minutes_of_day() / 1440.0;
    let l1 = format!("1 {:05}U 26001A   {:02}{:012.8}  .00000000  00000-0  00000-0 0  999", num % 100_000, start.year.rem_euclid(100), doy);
    let l1 = format!("{l1}{}", checksum(&l1));
    let l2 = format!("2 {:05} {:8.4} {:8.4} 0001000 {:8.4} {:8.4} {:11.8}{:05}", num % 100_000, incl_deg, raan_deg.rem_euclid(360.0), 0.0, phase_deg.rem_euclid(360.0), n, 1);
    let l2 = format!("{l2}{}", checksum(&l2));
    (l1, l2)
}

/// Sub-satellite point of a two-body circular orbit (for placement search).
fn sub_point(alt_m: f64, incl: f64, raan: f64, phase: f64, t: f64, start_jd: f64) -> Geodetic {
    let a = RE + alt_m;
    let n = (MU_EARTH / (a * a * a)).sqrt();
    let u = phase + n * t;
    let (su, cu) = u.sin_cos();
    let (so, co) = raan.sin_cos();
    let (si, ci) = incl.sin_cos();
    let x = co * cu - so * su * ci;
    let y = so * cu + co * su * ci;
    let z = su * si;
    let lon = y.atan2(x) - gmst(start_jd + t / 86_400.0);
    let lon = (lon + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    Geodetic::new(z.asin().to_degrees(), lon.to_degrees(), 0.0)
}

/// The node and phase that put a satellite over `site` at `t` seconds.
fn place_over(alt_m: f64, incl_deg: f64, site: Geodetic, t: f64, start_jd: f64) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0, 0.0);
    let incl = incl_deg.to_radians();
    let mut phase: f64 = 0.0;
    while phase < 360.0 {
        let mut raan: f64 = 0.0;
        while raan < 360.0 {
            let d = sub_point(alt_m, incl, raan.to_radians(), phase.to_radians(), t, start_jd).surface_distance(site);
            if d < best.0 {
                best = (d, raan, phase);
            }
            raan += 2.5;
        }
        phase += 2.5;
    }
    (best.1, best.2)
}

/// Where a side wants to look: its named areas, else the enemy as briefed.
fn areas_of_interest(s: &Sim, side: Side) -> Vec<(Geodetic, String, u8)> {
    let mut out: Vec<(Geodetic, String, u8)> = s.ctx.scenario.world.nais.iter().filter(|n| n.side == side).map(|n| (n.at, n.label.clone(), n.priority.clamp(0.0, 255.0) as u8)).collect();
    let enemy: Vec<Geodetic> = s.ctx.scenario.forces_of(side.enemy()).map(|f| f.position).collect();
    if !enemy.is_empty() {
        let c = Geodetic::new(enemy.iter().map(|g| g.lat).sum::<f64>() / enemy.len() as f64, enemy.iter().map(|g| g.lon).sum::<f64>() / enemy.len() as f64, 0.0);
        out.push((c, "the enemy as briefed".into(), 1));
    }
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
    out
}

fn centroid(v: &[(Geodetic, String, u8)], fallback: Geodetic) -> Geodetic {
    if v.is_empty() {
        return fallback;
    }
    Geodetic::new(v.iter().map(|g| g.0.lat).sum::<f64>() / v.len() as f64, v.iter().map(|g| g.0.lon).sum::<f64>() / v.len() as f64, 0.0)
}

fn gnss_constellation(s: &Sim) -> Vec<Satellite> {
    let f = &s.ctx.figures;
    let planes = f.get("sim.space.gnss_planes") as u32;
    let per = f.get("sim.space.gnss_per_plane") as u32;
    let alt = f.get("sim.space.gnss_alt_km") * 1000.0;
    let incl = f.get("sim.space.gnss_incl_deg");
    let mut v = Vec::new();
    for p in 0..planes {
        for k in 0..per {
            let raan = 360.0 * p as f64 / planes as f64;
            let phase = 360.0 * k as f64 / per as f64 + 15.0 * p as f64;
            let (l1, l2) = circular_tle(90_000 + p * 100 + k, &s.ctx.start, alt, incl, raan, phase);
            if let Ok(sat) = Satellite::from_tle(&format!("gnss-{p}-{k}"), &l1, &l2) {
                v.push(sat);
            }
        }
    }
    v
}

pub(crate) fn setup(s: &mut Sim) {
    let mode = s.ctx.scenario.scope.mode(ScopeDomain::Space);
    if mode == DomainMode::Off {
        return;
    }
    let start_jd = s.ctx.start.julian_day();
    let f = s.ctx.figures.clone();
    let mut st = SpaceState {
        mode,
        start_jd,
        sats: BTreeMap::new(),
        gnss: Vec::new(),
        queues: BTreeMap::new(),
        batches: BTreeMap::new(),
        outages: BTreeMap::new(),
        gnss_jam: BTreeMap::new(),
        areas: Side::PLAYERS.iter().map(|sd| (*sd, areas_of_interest(s, *sd))).collect(),
        img: ImageryParams::from_figures(&f),
        next: 0,
        stats: SpaceStats::default(),
        scheduled_to_jd: start_jd,
        req_sites: BTreeMap::new(),
    };
    if mode == DomainMode::Simulated {
        st.gnss = gnss_constellation(s);
    }
    let mut placed = 0u32;
    for (idx, &e) in s.ctx.order.clone().iter().enumerate() {
        let i = s.world.get::<Ident>(e).expect("ident").clone();
        if i.domain != Domain::Space {
            continue;
        }
        // Entities spawned during play (munitions, flights) have no scenario force.
        let Some(force) = s.ctx.scenario.forces.get(idx) else { continue };
        let p = &i.platform;
        let kind = if p.sensors.iter().any(|x| x.kind == CatSensor::Sar) {
            SatKind::Sar
        } else if p.sensors.iter().any(|x| x.kind == CatSensor::EoSat) {
            SatKind::Eo
        } else if p.sensors.iter().any(|x| x.kind == CatSensor::MissileWarning) {
            SatKind::Warning
        } else {
            SatKind::Other
        };
        let imaging = p.sensors.iter().find_map(|x| x.imaging);
        let alt = if p.mover.orbit_alt_km > 0.0 { p.mover.orbit_alt_km * 1000.0 } else { 550_000.0 };
        let incl = p.mover.inclination_deg;
        let site = centroid(&st.areas[&if i.side == Side::Neutral { Side::Blue } else { i.side }], s.ctx.scenario.theatre.origin);
        let (raan, phase) = match force.orbit {
            Some(o) => (o.raan_deg, o.phase_deg),
            None if matches!(kind, SatKind::Eo | SatKind::Sar) => {
                let t = f.get("sim.space.first_pass_s") + placed as f64 * f.get("sim.space.pass_spacing_s");
                placed += 1;
                place_over(alt, incl, site, t, start_jd)
            }
            None => (0.0, 0.0),
        };
        let (l1, l2) = circular_tle(1_000 + i.id.0, &s.ctx.start, alt, incl, raan, phase);
        let sat = match Satellite::from_tle(&i.name, &l1, &l2) {
            Ok(x) => x,
            Err(err) => {
                s.ctx.log.push(SimTime::ZERO, "space.error").entity(i.id).text(format!("{}: no orbit ({err})", i.label));
                continue;
            }
        };
        st.sats.insert(
            i.id,
            SatRec {
                id: i.id,
                side: i.side,
                sat,
                kind,
                commercial: force.provider.as_deref() == Some("commercial"),
                gsd: imaging.as_ref().map(|x| x.gsd_m).unwrap_or(1.0),
                swath_m: imaging.as_ref().map(|x| x.swath_km * 1000.0).unwrap_or(20_000.0),
                passes: Vec::new(),
                site,
                tasked: false,
                alive: true,
            },
        );
    }
    s.world.insert_resource(st);
    if mode == DomainMode::Simulated {
        schedule_passes(s, 0.0);
    }
    minute(s);
}

/// Find and schedule the ISR passes over each area for the next horizon.
pub(crate) fn schedule_passes(s: &mut Sim, _key: f64) {
    let Some(mut st) = s.world.remove_resource::<SpaceState>() else { return };
    let f = s.ctx.figures.clone();
    let jd0 = st.scheduled_to_jd.max(st.jd(s.ctx.now));
    let horizon_d = f.get("sim.space.horizon_h") / 24.0;
    let jd1 = jd0 + horizon_d;
    let min_el = f.get("sim.space.isr_min_el_deg").to_radians();
    let step = f.get("sim.space.pass_step_s");
    for rec in st.sats.values_mut() {
        if !matches!(rec.kind, SatKind::Eo | SatKind::Sar) || rec.side == Side::Neutral {
            continue;
        }
        let passes = rec.sat.access_windows(rec.site, jd0, jd1, min_el, step).unwrap_or_default();
        for p in passes {
            let rise = SimTime::from_secs(((p.rise - st.start_jd) * 86_400.0).max(s.ctx.now.secs()));
            let set = SimTime::from_secs((p.set - st.start_jd) * 86_400.0).max(rise + SimDuration::from_secs(1.0));
            s.ctx.queue.schedule(rise, Priority::Physical, Scheduled::SatellitePass { satellite: rec.id, rising: true });
            s.ctx.queue.schedule(set, Priority::Physical, Scheduled::SatellitePass { satellite: rec.id, rising: false });
            rec.passes.push(p);
        }
    }
    st.scheduled_to_jd = jd1;
    // Look again when this horizon runs out.
    let next = st.time_of(jd1);
    s.ctx.queue.schedule(next, Priority::Process, Scheduled::Timer { system: "space.passes".into(), key: 0 });
    s.world.insert_resource(st);
}

/// Satellites' positions for the view (minute rate): ENU of the sub-point,
/// at orbital height.
pub(crate) fn minute(s: &mut Sim) {
    let Some(st) = s.world.get_resource::<SpaceState>() else { return };
    let jd = st.jd(s.ctx.now);
    let pts: Vec<(EntityId, Geodetic)> = st.sats.values().filter(|r| r.alive).filter_map(|r| r.sat.ground_point(jd).ok().map(|g| (r.id, g))).collect();
    for (id, g) in pts {
        let e = s.ctx.entity(id);
        let enu = s.ctx.frame.geodetic_to_enu(Geodetic::new(g.lat, g.lon, 0.0));
        if let Some(mut k) = s.world.get_mut::<Kin>(e) {
            k.pos = bgws_core::Enu::new(enu.e, enu.n, g.h);
            k.agl = g.h;
        }
    }
}

fn cloud_at(s: &Sim, site: Geodetic) -> f64 {
    if let Some(c) = s.ctx.scenario.environment.cloud_cover {
        return c.clamp(0.0, 1.0);
    }
    let v = s.ctx.frame.geodetic_to_enu(site);
    s.ctx.weather.sample(v.e, v.n, s.ctx.now).cloud_cover
}

/// A pass rises (ask the side's space cell: image or hold) or sets (serve
/// the queue: collect, then deliver after downlink and analysis).
pub(crate) fn pass(s: &mut Sim, satellite: EntityId, rising: bool) {
    let now = s.ctx.now;
    let Some(st) = s.world.get_resource::<SpaceState>() else { return };
    let Some(rec) = st.sats.get(&satellite).cloned() else { return };
    if !rec.alive || !crate::systems::alive(s, s.ctx.entity(satellite)) {
        return;
    }
    if rising {
        let cloud = cloud_at(s, rec.site);
        let skip = rec.kind == SatKind::Eo && cloud > s.ctx.figures.get("sim.space.eo_skip_cloud");
        let text = format!("{} rises over its area ({:.0}% cloud)", crate::systems::name(s, satellite), cloud * 100.0);
        s.ctx.log.push(now, "space.pass").side(rec.side).entity(satellite).text(text).data(serde_json::json!({ "kind": rec.kind, "cloud": cloud }));
        if let Some(mut st) = s.world.get_resource_mut::<SpaceState>() {
            st.stats.passes += 1;
        }
        let (opt, action) = if skip { ("skip", "space.skipPass") } else { ("image", "space.imageArea") };
        super::ask_or_do(s, satellite, "spaceTasking", opt, action);
        return;
    }
    // The pass sets: serve the queue.
    let mut st = s.world.remove_resource::<SpaceState>().expect("state");
    let jd = st.jd(now);
    let Some(idx) = st.sats[&satellite].passes.iter().position(|p| (p.set - jd).abs() * 86_400.0 < 120.0) else {
        s.world.insert_resource(st);
        return;
    };
    let p = st.sats.get_mut(&satellite).expect("sat").passes.remove(idx);
    let tasked = std::mem::replace(&mut st.sats.get_mut(&satellite).expect("sat").tasked, false);
    let side = rec.side;
    let name = crate::systems::name(s, satellite);
    let blocked = if rec.commercial && st.out(&format!("imagery:{}", side.as_str()), now) {
        Some("the commercial provider has cut its service")
    } else if st.out(&format!("dazzle:{}", rec.id.0), now) {
        Some("it is dazzled")
    } else {
        None
    };
    if !tasked {
        s.world.insert_resource(st);
        return;
    }
    if let Some(why) = blocked {
        let text = format!("{name} passes, but {why}: no image");
        s.ctx.log.push(now, "space.blocked").side(side).entity(satellite).text(text);
        s.world.insert_resource(st);
        return;
    }
    let kind = if rec.kind == SatKind::Sar { ImagerKind::Sar } else { ImagerKind::Eo };
    let mut params = st.img.clone();
    params.gsd_eo = rec.gsd;
    params.gsd_sar = rec.gsd;
    let cloud = cloud_at(s, rec.site);
    let mut rng = s.ctx.rng("space.cloud", satellite.0 as u64, st.stats.images as u64);
    let mut u = || rng.gen::<f64>();
    let queue = st.queues.entry(side).or_default();
    let products = serve_pass(queue, &p, kind, &params, &|_g| cloud, &mut u);
    let quality = s.ctx.figures.get("sim.space.image_quality");
    let radius = rec.swath_m / 2.0 * s.ctx.figures.get("sim.space.footprint_factor");
    let sigma = s.ctx.figures.get("sim.space.fix_sigma_m");
    let ip = bgws_models::sensing::imagery::ImageryParams::from_figures(&s.ctx.figures);
    let img_kind = if rec.kind == SatKind::Sar { ImageKind::Sar } else { ImageKind::Eo };
    for prod in products {
        st.stats.images += 1;
        if prod.clouded {
            st.stats.clouded += 1;
            let text = format!("{name}'s image is clouded over");
            s.ctx.log.push(now, "space.clouded").side(side).entity(satellite).text(text).data(serde_json::json!({ "cloud": cloud }));
            continue;
        }
        let site = st.req_sites.get(&prod.request).copied().unwrap_or(rec.site);
        let centre = s.ctx.frame.geodetic_to_enu(site);
        let mut dets = Vec::new();
        for &o in &s.ctx.order {
            let (Some(i), Some(k)) = (s.world.get::<Ident>(o), s.world.get::<Kin>(o)) else { continue };
            if i.side != side.enemy() || !crate::systems::alive(s, o) || !matches!(i.domain, Domain::Land | Domain::Surface | Domain::Node) {
                continue;
            }
            if k.pos.sub(centre).horizontal() > radius {
                continue;
            }
            let crit = i.platform.signatures.visual.critical_dim_m.max(0.5);
            let stage = |st: Stage| image_identification(img_kind, crit, prod.gsd_m, quality, 1.0, st, &ip).p_stage;
            let (pd, pr, pi) = (stage(Stage::Detect), stage(Stage::Recognise), stage(Stage::Identify));
            let mut rng = s.ctx.rng("space.image", prod.request, i.id.0 as u64);
            let roll: f64 = rng.gen();
            if roll >= pd {
                continue;
            }
            let level = if roll < pi { IdLevel::Identify } else if roll < pr { IdLevel::Recognise } else { IdLevel::Detect };
            let sea = i.domain == Domain::Surface;
            let noise = bgws_core::Enu::new(rng.gen::<f64>() * 2.0 - 1.0, rng.gen::<f64>() * 2.0 - 1.0, 0.0).scale(sigma);
            dets.push(Detection {
                time: now,
                observer: satellite,
                sensor: if rec.kind == SatKind::Sar { SensorKind::Sar } else { SensorKind::Visual },
                domain: if sea { bgws_know::Domain::Sea } else { bgws_know::Domain::Land },
                motion: if k.vel.horizontal() > 0.5 { MotionHint::Moving } else { MotionHint::Static },
                measurement: Measurement::Position { pos: k.pos.add(noise), cov: Matrix3::from_diagonal_element(sigma * sigma) },
                class: ClassEvidence { level, reported: (level != IdLevel::Detect).then(|| hypothesis(&i.platform, i.dummy)), resolves_decoys: level == IdLevel::Identify && rec.kind == SatKind::Eo },
                truth: TruthTag::of(i.id),
            });
        }
        let deliver = st.time_of(prod.delivered_jd).max(now);
        st.next += 1;
        let id = st.next;
        let text = format!("{name} images {} ({} things seen); analysis until +{:.0} min", site_label(&st, side, site), dets.len(), (deliver - now).secs() / 60.0);
        s.ctx.log.push(now, "space.imaged").side(side).entity(satellite).text(text).data(serde_json::json!({ "request": prod.request, "detections": dets.len(), "gsdM": prod.gsd_m, "deliverAt": deliver.secs() }));
        st.batches.insert(id, Batch { id, side, sat: satellite, taken: now, deliver, detections: dets });
        s.ctx.queue.schedule(deliver, Priority::Comms, Scheduled::Timer { system: "space.deliver".into(), key: id });
    }
    s.world.insert_resource(st);
}

fn site_label(st: &SpaceState, side: Side, at: Geodetic) -> String {
    st.areas.get(&side).and_then(|v| v.iter().find(|a| a.0.surface_distance(at) < 1.0)).map(|a| a.1.clone()).unwrap_or_else(|| "its area".into())
}

fn hypothesis(p: &bgws_catalogue::Platform, dummy: bool) -> Hypothesis {
    let r = p.role.to_lowercase();
    let c = p.class.to_lowercase();
    if p.domain == Domain::Surface {
        return if c.contains("merchant") { Hypothesis::Merchant } else if c.contains("small") { Hypothesis::Usv } else { Hypothesis::Warship };
    }
    if dummy {
        return Hypothesis::Mbt;
    }
    if c.contains("mbt") || r.contains("tank") {
        Hypothesis::Mbt
    } else if c.contains("ifv") {
        Hypothesis::Ifv
    } else if c.contains("apc") {
        Hypothesis::Apc
    } else if p.mover.kind == bgws_catalogue::MoverKind::Foot {
        Hypothesis::Infantry
    } else if r.contains("artillery") || r.contains("rocket") || r.contains("air defence") || r.contains("missile") {
        Hypothesis::Artillery
    } else {
        Hypothesis::Truck
    }
}

/// Imagery reaches the side: its detections enter the track store, old but there.
pub(crate) fn deliver(s: &mut Sim, key: u64) {
    let Some(mut st) = s.world.remove_resource::<SpaceState>() else { return };
    if let Some(b) = st.batches.remove(&key) {
        let now = s.ctx.now;
        let n = b.detections.len();
        st.stats.detections_delivered += n as u32;
        st.stats.latency_s.push((now - b.taken).secs());
        for mut d in b.detections {
            d.time = now;
            s.ctx.reports.push((now, b.side, d));
        }
        let text = format!("imagery from {} reaches {}: {n} detections, {:.0} min old", crate::systems::name(s, b.sat), b.side, (now - b.taken).secs() / 60.0);
        s.ctx.log.push(now, "space.imagery").side(b.side).entity(b.sat).text(text).data(serde_json::json!({ "detections": n, "latencyS": (now - b.taken).secs() }));
    }
    s.world.insert_resource(st);
}

pub(crate) fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    let Some(st) = s.world.get_resource::<SpaceState>() else { return };
    let Some(i) = s.world.get::<Ident>(e) else { return };
    if let Some(r) = st.sats.get(&i.id) {
        m.insert("agent.domain".into(), FactValue::Text("space".into()));
        m.insert("agent.archetype".into(), FactValue::Text(i.platform.id.clone()));
        m.insert("env.badWeather".into(), FactValue::Bool(r.kind == SatKind::Eo && cloud_at(s, r.site) > 0.6));
    }
}

/// `space.imageArea`: put the side's areas in the queue for this pass;
/// `space.skipPass`: hold it.
pub(crate) fn apply(s: &mut Sim, _e: Entity, id: EntityId, action: &str, _about: Option<EntityId>) -> bool {
    let Some(mut st) = s.world.remove_resource::<SpaceState>() else { return false };
    let handled = match action {
        "space.imageArea" => {
            if let Some(rec) = st.sats.get(&id).cloned() {
                let side = rec.side;
                let jd = st.jd(s.ctx.now);
                let areas = st.areas.get(&side).cloned().unwrap_or_default();
                let mut added = 0;
                for (at, _label, pri) in areas.iter().filter(|a| a.0.surface_distance(rec.site) <= 300_000.0) {
                    let known = st.queues.get(&side).map(|q| q.iter().any(|r| r.site.surface_distance(*at) < 1.0)).unwrap_or(false);
                    if known {
                        continue;
                    }
                    st.next += 1;
                    let rid = st.next;
                    st.req_sites.insert(rid, *at);
                    st.queues.entry(side).or_default().push(ImageryRequest { id: rid, site: *at, requested_jd: jd, priority: *pri });
                    added += 1;
                }
                st.sats.get_mut(&id).expect("sat").tasked = true;
                let text = format!("{} is tasked on this pass ({} new requests, {} in the queue)", crate::systems::name(s, id), added, st.queues.get(&side).map(|q| q.len()).unwrap_or(0));
                s.ctx.log.push(s.ctx.now, "space.tasked").side(side).entity(id).text(text);
            }
            true
        }
        "space.skipPass" => {
            if let Some(r) = st.sats.get_mut(&id) {
                r.tasked = false;
            }
            true
        }
        _ => false,
    };
    s.world.insert_resource(st);
    handled
}

/// The scripted space events (W9): provider cut-offs, counterspace jamming,
/// dazzling, a kinetic ASAT, and restoration.
pub(crate) fn script_event(s: &mut Sim, e: &bgws_scenario::v4::ScriptedEvent) {
    let Some(mut st) = s.world.remove_resource::<SpaceState>() else { return };
    let now = s.ctx.now;
    let until = e.duration_s.map(|d| now + SimDuration::from_secs(d)).unwrap_or(SimTime::MAX);
    let side = e.side;
    let sd = side.as_str();
    let target = e.target.as_ref().and_then(|t| s.ctx.by_name.get(t).copied());
    let text = match e.kind.as_str() {
        "cutCommercialImagery" => {
            st.outages.insert(format!("imagery:{sd}"), until);
            Some(format!("the commercial imagery provider cuts {sd} off"))
        }
        "cutLeoSatcom" => {
            st.outages.insert(format!("satcomLeo:{sd}"), until);
            Some(format!("the LEO SATCOM provider cuts {sd}'s terminals off"))
        }
        "jamSatcom" => {
            st.outages.insert(format!("satcomMil:{sd}"), until);
            Some(format!("counterspace jamming of {sd}'s military SATCOM uplinks"))
        }
        "jamGnss" => {
            let q = e.quality.unwrap_or(s.ctx.figures.get("sim.space.jam_gnss_quality")).clamp(0.0, 1.0);
            st.gnss_jam.insert(side, (q, until));
            Some(format!("counterspace jamming: {sd}'s GNSS falls to {:.0}%", q * 100.0))
        }
        "dazzleEo" => {
            if let Some(t) = target {
                st.outages.insert(format!("dazzle:{}", t.0), until);
            }
            Some(format!("{} is dazzled", e.target.clone().unwrap_or_else(|| "an imaging satellite".into())))
        }
        "asat" => {
            if let Some(t) = target {
                if let Some(r) = st.sats.get_mut(&t) {
                    r.alive = false;
                }
                if let Some(mut h) = s.world.get_mut::<Health>(s.ctx.entity(t)) {
                    h.destroyed = true;
                    h.fit = 0;
                    h.losses.push(now);
                }
            }
            Some(format!("a kinetic ASAT destroys {}", e.target.clone().unwrap_or_else(|| "a satellite".into())))
        }
        "restore" => {
            st.outages.retain(|k, _| !k.ends_with(&format!(":{sd}")));
            st.gnss_jam.remove(&side);
            Some(format!("{sd}'s space services are restored"))
        }
        _ => None,
    };
    if let Some(t) = text {
        s.ctx.log.push(now, "space.event").side(side).text(t).data(serde_json::json!({ "kind": e.kind, "until": if until == SimTime::MAX { None } else { Some(until.secs()) } }));
    }
    s.world.insert_resource(st);
}

/// GNSS geometry (DOP) at a point now, for the PNT code: the synthetic
/// GPS-like constellation through SGP4 when space is simulated, the nominal
/// geometry when it is abstracted, none when off.
pub fn gnss_geometry(s: &Sim, at: Geodetic) -> Option<GnssGeometry> {
    let st = s.world.get_resource::<SpaceState>()?;
    if st.gnss.is_empty() {
        let h = s.ctx.figures.get("sim.space.gnss_hdop_nominal");
        return Some(GnssGeometry { visible: 8, gdop: h * 1.8, pdop: h * 1.6, hdop: h, vdop: h * 1.3 });
    }
    let mask = s.ctx.figures.get("space.gnss.mask_deg").to_radians();
    bgws_domains::space::gnss_geometry(&st.gnss, at, st.jd(s.ctx.now), mask).ok().flatten()
}

/// A side's GNSS quality (0–1) under counterspace jamming.
pub fn gnss_quality(s: &Sim, side: Side) -> f64 {
    s.world.get_resource::<SpaceState>().and_then(|st| st.gnss_jam.get(&side).filter(|(_, u)| *u > s.ctx.now).map(|(q, _)| *q)).unwrap_or(1.0)
}

/// SATCOM capacity a side has in the theatre (bit/s): the beam's capacity,
/// zero under a provider cut-off or uplink jamming, times any cyber effect
/// on its terminals.
pub fn satcom_capacity_bps(s: &Sim, side: Side, leo: bool) -> f64 {
    let f = &s.ctx.figures;
    let base = if leo { f.get("sim.space.satcom_leo_bps") } else { f.get("sim.space.satcom_mil_bps") };
    let cut = s.world.get_resource::<SpaceState>().map(|st| st.out(&format!("{}:{}", if leo { "satcomLeo" } else { "satcomMil" }, side.as_str()), s.ctx.now)).unwrap_or(false);
    let beam = bgws_domains::space::Beam { centre: s.ctx.scenario.theatre.origin, radius_m: f.get("sim.space.beam_radius_km") * 1000.0, capacity_bps: base, on: !cut };
    let cap = beam.allocate(&[base])[0];
    cap * cyber_multiplier(s, side, &TargetProcess::SatcomTerminals)
}

/// Can the side get imagery from its commercial provider now?
pub fn imagery_available(s: &Sim, side: Side) -> bool {
    s.world.get_resource::<SpaceState>().map(|st| !st.out(&format!("imagery:{}", side.as_str()), s.ctx.now)).unwrap_or(false)
}

/// Does the side have a working missile-warning satellite?
pub fn missile_warning(s: &Sim, side: Side) -> bool {
    s.world.get_resource::<SpaceState>().map(|st| st.sats.values().any(|r| r.side == side && r.kind == SatKind::Warning && r.alive)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesised_tle_flies_the_orbit_it_was_given() {
        let start = bgws_env::UtcDateTime::new(2026, 6, 1, 0, 45, 36.0);
        let (l1, l2) = circular_tle(1234, &start, 550_000.0, 97.0, 40.0, 10.0);
        assert_eq!(l1.len(), 69, "{l1}");
        assert_eq!(l2.len(), 69, "{l2}");
        let sat = Satellite::from_tle("t", &l1, &l2).unwrap();
        assert!((sat.epoch_jd - start.julian_day()).abs() * 86_400.0 < 1.0, "epoch at the start");
        for k in 0..10 {
            let g = sat.ground_point(start.julian_day() + k as f64 * 0.01).unwrap();
            assert!((g.h - 550_000.0).abs() < 30_000.0, "{}", g.h);
        }
        // Placement: the engine's two-body search puts the first pass where asked.
        let site = Geodetic::new(55.0, 18.6, 0.0);
        let (raan, phase) = place_over(550_000.0, 97.0, site, 900.0, start.julian_day());
        let (l1, l2) = circular_tle(1, &start, 550_000.0, 97.0, raan, phase);
        let sat = Satellite::from_tle("p", &l1, &l2).unwrap();
        let d = sat.ground_point(start.julian_day() + 900.0 / 86_400.0).unwrap().surface_distance(site);
        assert!(d < 400_000.0, "sub-point {d} m from the site at the planned time");
    }
}
