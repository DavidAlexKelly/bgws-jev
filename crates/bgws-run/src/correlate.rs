//! Terrain correlation (docs/design/INTEROP_PLAN.md, phase 2): how far our
//! ground differs from another simulation's, before an exercise.
//!
//! Other simulations keep their own terrain; DIS and HLA only say where
//! things are. Where the grounds differ, a vehicle floats or sinks in the
//! other world, and a sight line one side has the other does not. This
//! compares our ground with a partner's elevation:
//! - heights on a grid of sample points: the error's mean, RMS, 95th
//!   percentile and worst, and every point beyond a tolerance;
//! - sight lines between sample points 1-4 km apart, eye height 2 m: the
//!   share where one ground sees through and the other does not.
//!
//! Both sight-line checks use the same profile method (terrain only, the
//! Earth's curvature with 4/3 refraction), so a disagreement is the ground's,
//! not the algorithms'. Heights are above mean sea level on both sides; a
//! partner whose heights are above the ellipsoid gives `ellipsoidal`, and the
//! geoid is taken off its heights.

use bgws_core::{Enu, EnuFrame};
use rand::{Rng, SeedableRng};
use serde::Serialize;

/// A scenario's sim with its ground as a game would have it (its pack, or
/// generated), to read the ground from.
pub fn ground_sim(scenario: &bgws_scenario::v4::ScenarioV4) -> anyhow::Result<bgws_sim::Sim> {
    let mut opts = bgws_sim::SimOptions::default();
    if let Some(g) = crate::host::load_pack(scenario)? {
        opts.terrain = Some(g.terrain);
        opts.roads = Some(g.roads);
        opts.rivers = Some(g.rivers);
    }
    bgws_sim::Sim::with_options(scenario, bgws_core::Seed(1), crate::default_figures(), opts).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Our ground's area (east/north extent of its grid).
pub fn ground_area(g: &bgws_sim::ground::Ground) -> [f64; 4] {
    let s = g.terrain.spec;
    [s.origin_e, s.origin_n, s.origin_e + s.cell * (s.width as f64 - 1.0), s.origin_n + s.cell * (s.height as f64 - 1.0)]
}

/// Write our ground as a GeoTIFF on a latitude/longitude grid of `cell_m`
/// (about), heights above mean sea level: what a partner builds its terrain
/// from when a federation shares our generated ground.
pub fn export_ground(sim: &bgws_sim::Sim, path: &std::path::Path, cell_m: f64) -> anyhow::Result<(u32, u32)> {
    let [e0, n0, e1, n1] = ground_area(&sim.ctx.ground);
    let f = &sim.ctx.frame;
    let nw = f.enu_to_geodetic(Enu::new(e0, n1, 0.0));
    let se = f.enu_to_geodetic(Enu::new(e1, n0, 0.0));
    let dlat = cell_m / 111_132.0;
    let dlon = cell_m / (111_320.0 * f.origin.lat.to_radians().cos());
    let w = ((se.lon - nw.lon) / dlon).floor().max(1.0) as u32;
    let h = ((nw.lat - se.lat) / dlat).floor().max(1.0) as u32;
    let mut heights = Vec::with_capacity((w * h) as usize);
    for r in 0..h {
        for c in 0..w {
            // Pixel centres.
            let (lat, lon) = (nw.lat - (r as f64 + 0.5) * dlat, nw.lon + (c as f64 + 0.5) * dlon);
            let p = f.geodetic_to_enu(bgws_core::Geodetic { lat, lon, h: 0.0 });
            heights.push(sim.ctx.ground.height(p.e, p.n) as f32);
        }
    }
    crate::geotiff::write_dem(path, (nw.lon, nw.lat), (dlon, dlat), (w, h), &heights)?;
    Ok((w, h))
}

/// Effective Earth radius for optical sight lines (4/3 refraction), m.
const R_EFF: f64 = 6_371_000.0 * 4.0 / 3.0;
const EYE_M: f64 = 2.0;
const STEP_M: f64 = 15.0;

/// A place the two grounds differ by more than the tolerance.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    pub lat: f64,
    pub lon: f64,
    pub ours_m: f64,
    pub theirs_m: f64,
}

/// A sight line the two grounds disagree on.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SightDisagreement {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub range_m: f64,
    pub ours_visible: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorrelationReport {
    pub samples: usize,
    /// Sample points the partner's elevation does not cover.
    pub uncovered: usize,
    /// Ours minus theirs, m.
    pub mean_error_m: f64,
    pub rms_error_m: f64,
    pub p95_abs_error_m: f64,
    pub max_abs_error_m: f64,
    pub tolerance_m: f64,
    pub beyond_tolerance: Vec<Mismatch>,
    pub sight_lines: usize,
    pub sight_disagreements: Vec<SightDisagreement>,
}

impl CorrelationReport {
    pub fn sight_disagreement_share(&self) -> f64 {
        self.sight_disagreements.len() as f64 / self.sight_lines.max(1) as f64
    }
}

/// Is `b` visible from `a` over a ground (heights by east/north)?
fn visible(h: &dyn Fn(f64, f64) -> Option<f64>, a: Enu, b: Enu) -> Option<bool> {
    let d = b.sub(a).horizontal();
    let (ha, hb) = (h(a.e, a.n)? + EYE_M, h(b.e, b.n)? + EYE_M);
    let n = (d / STEP_M).ceil().max(1.0) as usize;
    for k in 1..n {
        let f = k as f64 / n as f64;
        let (e, nn) = (a.e + (b.e - a.e) * f, a.n + (b.n - a.n) * f);
        let (d1, d2) = (d * f, d * (1.0 - f));
        let ray = ha + (hb - ha) * f;
        // The ground rises toward the ray by the curvature's bulge.
        if h(e, nn)? + d1 * d2 / (2.0 * R_EFF) > ray {
            return Some(false);
        }
    }
    Some(true)
}

/// Compare our ground (`ours`, by east/north) with a partner's (`theirs`, by
/// latitude and longitude) over the area `[e0, e1] × [n0, n1]`.
#[allow(clippy::too_many_arguments)]
pub fn correlate(frame: &EnuFrame, area: [f64; 4], spacing_m: f64, tolerance_m: f64, sight_lines: usize, ours: &dyn Fn(f64, f64) -> f64, theirs: &dyn Fn(f64, f64) -> Option<f64>, seed: u64) -> CorrelationReport {
    let [e0, n0, e1, n1] = area;
    let latlon = |e: f64, n: f64| {
        let g = frame.enu_to_geodetic(Enu::new(e, n, 0.0));
        (g.lat, g.lon)
    };
    let theirs_en = |e: f64, n: f64| {
        let (lat, lon) = latlon(e, n);
        theirs(lat, lon)
    };
    let ours_en = |e: f64, n: f64| Some(ours(e, n));
    let mut errors = Vec::new();
    let mut beyond = Vec::new();
    let (mut samples, mut uncovered) = (0, 0);
    let mut points = Vec::new();
    let mut n = n0;
    while n <= n1 + 1e-6 {
        let mut e = e0;
        while e <= e1 + 1e-6 {
            samples += 1;
            let (lat, lon) = latlon(e, n);
            match theirs(lat, lon) {
                Some(t) => {
                    let o = ours(e, n);
                    errors.push(o - t);
                    points.push(Enu::new(e, n, 0.0));
                    if (o - t).abs() > tolerance_m {
                        beyond.push(Mismatch { lat, lon, ours_m: o, theirs_m: t });
                    }
                }
                None => uncovered += 1,
            }
            e += spacing_m;
        }
        n += spacing_m;
    }
    let count = errors.len().max(1) as f64;
    let mean = errors.iter().sum::<f64>() / count;
    let rms = (errors.iter().map(|x| x * x).sum::<f64>() / count).sqrt();
    let mut abs: Vec<f64> = errors.iter().map(|x| x.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let p95 = abs.get(((abs.len() as f64 * 0.95) as usize).min(abs.len().saturating_sub(1))).copied().unwrap_or(0.0);
    let max = abs.last().copied().unwrap_or(0.0);
    // Sight lines between sample points 1-4 km apart, the same pairs for both.
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let (mut tested, mut disagree) = (0, Vec::new());
    let mut tries = 0;
    while tested < sight_lines && points.len() > 1 && tries < sight_lines * 50 {
        tries += 1;
        let (a, b) = (points[rng.gen_range(0..points.len())], points[rng.gen_range(0..points.len())]);
        let d = b.sub(a).horizontal();
        if !(1_000.0..=4_000.0).contains(&d) {
            continue;
        }
        let (Some(o), Some(t)) = (visible(&ours_en, a, b), visible(&theirs_en, a, b)) else {
            continue;
        };
        tested += 1;
        if o != t {
            let (la, lo) = latlon(a.e, a.n);
            let (lb, lob) = latlon(b.e, b.n);
            disagree.push(SightDisagreement { from: [la, lo], to: [lb, lob], range_m: d, ours_visible: o });
        }
    }
    CorrelationReport { samples, uncovered, mean_error_m: mean, rms_error_m: rms, p95_abs_error_m: p95, max_abs_error_m: max, tolerance_m, beyond_tolerance: beyond, sight_lines: tested, sight_disagreements: disagree }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Geodetic;

    fn frame() -> EnuFrame {
        EnuFrame::new(Geodetic { lat: 54.2, lon: 18.6, h: 0.0 })
    }

    /// Rolling ground with a north-south ridge at e = 0, 40 m high.
    fn ground(e: f64, n: f64) -> f64 {
        100.0 + 5.0 * (n / 700.0).sin() + 40.0 * (-(e / 150.0).powi(2)).exp()
    }

    fn theirs_from<G: Fn(f64, f64) -> f64 + 'static>(f: &EnuFrame, g: G) -> impl Fn(f64, f64) -> Option<f64> + '_ {
        move |lat: f64, lon: f64| {
            let p = f.geodetic_to_enu(Geodetic { lat, lon, h: 0.0 });
            Some(g(p.e, p.n))
        }
    }

    const AREA: [f64; 4] = [-3_000.0, -3_000.0, 3_000.0, 3_000.0];

    #[test]
    fn the_same_ground_correlates_exactly() {
        let f = frame();
        let theirs = theirs_from(&f, ground);
        let r = correlate(&f, AREA, 250.0, 2.0, 300, &ground, &theirs, 1);
        assert!(r.max_abs_error_m < 0.05, "{r:?}");
        assert!(r.beyond_tolerance.is_empty() && r.sight_disagreements.is_empty());
        assert_eq!(r.sight_lines, 300);
    }

    #[test]
    fn a_uniform_shift_is_reported_and_changes_no_sight_line() {
        let f = frame();
        let theirs = theirs_from(&f, |e, n| ground(e, n) - 3.0);
        let r = correlate(&f, AREA, 250.0, 2.0, 300, &ground, &theirs, 1);
        assert!((r.mean_error_m - 3.0).abs() < 0.05 && (r.rms_error_m - 3.0).abs() < 0.05, "{r:?}");
        assert_eq!(r.beyond_tolerance.len(), r.samples - r.uncovered, "every point is 3 m off");
        assert!(r.sight_disagreements.is_empty(), "a uniform shift blocks nothing new");
    }

    #[test]
    fn a_ridge_one_side_lacks_is_found_on_exactly_the_lines_it_blocks() {
        let f = frame();
        let flat = |e: f64, n: f64| 100.0 + 5.0 * (n / 700.0).sin() + 0.0 * e;
        let theirs = theirs_from(&f, flat);
        let r = correlate(&f, AREA, 250.0, 2.0, 400, &ground, &theirs, 2);
        // Heights differ along the ridge only.
        assert!(r.beyond_tolerance.iter().all(|m| m.ours_m > m.theirs_m), "only the ridge is higher here");
        assert!(r.max_abs_error_m > 35.0);
        // Every disagreement is real (the two grounds do differ on that line)
        // and comes from the ridge: the line crosses it, or starts or ends on
        // it (a ridge also lifts an observer to see over smaller rises).
        assert!(!r.sight_disagreements.is_empty());
        let ours_en = |e: f64, n: f64| Some(ground(e, n));
        let theirs_en = |e: f64, n: f64| Some(flat(e, n));
        for d in &r.sight_disagreements {
            let a = f.geodetic_to_enu(Geodetic { lat: d.from[0], lon: d.from[1], h: 0.0 });
            let b = f.geodetic_to_enu(Geodetic { lat: d.to[0], lon: d.to[1], h: 0.0 });
            let (a, b) = (Enu::new(a.e, a.n, 0.0), Enu::new(b.e, b.n, 0.0));
            assert_eq!(visible(&ours_en, a, b), Some(d.ours_visible));
            assert_eq!(visible(&theirs_en, a, b), Some(!d.ours_visible));
            assert!(a.e.signum() != b.e.signum() || a.e.abs() < 450.0 || b.e.abs() < 450.0, "{d:?} is nowhere near the ridge");
        }
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn our_ground_correlates_with_its_own_export() {
        let sc = crate::load_scenario(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios/gen-ambush.json")).unwrap();
        let sim = super::ground_sim(&sc).unwrap();
        let dir = std::env::temp_dir().join(format!("bgws-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ground.tif");
        super::export_ground(&sim, &path, 30.0).unwrap();
        let dem = crate::geotiff::Dem::read(&path).unwrap();
        let ground = &sim.ctx.ground;
        let ours = |e: f64, n: f64| ground.height(e, n);
        let theirs = |lat: f64, lon: f64| dem.height(lat, lon);
        let r = super::correlate(&sim.ctx.frame, super::ground_area(ground), 500.0, 2.0, 200, &ours, &theirs, 1);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.samples > 100, "{r:?}");
        // Resampled at 30 m: small errors, about none past 2 m, sight lines agree.
        assert!(r.mean_error_m.abs() < 0.3 && r.rms_error_m < 1.5, "{} {}", r.mean_error_m, r.rms_error_m);
        assert!(r.sight_disagreement_share() < 0.05, "{}", r.sight_disagreement_share());
    }
}
