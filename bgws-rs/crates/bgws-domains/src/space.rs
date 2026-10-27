//! Space (RUST_ENGINE.md §8.1 Space, §8.12): satellites propagated with SGP4
//! from orbital elements (AFSPC-compatible mode, matching the published
//! verification vectors), converted TEME → ECEF through GMST; ground tracks;
//! access windows to a ground point found by root-finding on elevation and
//! returned as rise / culminate / set events; swath footprints and revisit;
//! delta-v manoeuvres (two-body after the burn); imagery tasking with cloud
//! blocking for EO, downlink and analysis delays; GNSS visibility and DOP;
//! satcom beams with capacity [R5.4].

use crate::orbit::{ecef_v, gmst, inertial_to_ecef, kepler_propagate, StateVector, DAY_S, JD_J2000, MU_EARTH};
use crate::vec3::V3;
use crate::DomainError;
use bgws_core::{figure, Ecef, EnuFrame, Figure, Figures, Geodetic};
use serde::{Deserialize, Serialize};

/// A satellite: SGP4 constants plus an optional manoeuvre.
#[derive(Clone)]
pub struct Satellite {
    pub name: String,
    constants: sgp4::Constants,
    /// Epoch of the elements, Julian date (UTC).
    pub epoch_jd: f64,
    /// After a burn: the TEME state just after it, and its Julian date.
    burn: Option<(f64, StateVector)>,
}

impl std::fmt::Debug for Satellite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Satellite").field("name", &self.name).field("epoch_jd", &self.epoch_jd).field("manoeuvred", &self.burn.is_some()).finish()
    }
}

impl Satellite {
    /// From a two-line element set.
    pub fn from_tle(name: &str, line1: &str, line2: &str) -> Result<Self, DomainError> {
        let el = sgp4::Elements::from_tle(Some(name.to_string()), line1.as_bytes(), line2.as_bytes()).map_err(|e| DomainError::Elements(e.to_string()))?;
        let constants = sgp4::Constants::from_elements_afspc_compatibility_mode(&el).map_err(|e| DomainError::Elements(e.to_string()))?;
        Ok(Satellite { name: name.to_string(), constants, epoch_jd: JD_J2000 + el.epoch_afspc_compatibility_mode() * 365.25, burn: None })
    }

    /// TEME position and velocity in metres and m/s at Julian date `jd`.
    pub fn teme(&self, jd: f64) -> Result<StateVector, DomainError> {
        if let Some((jd_burn, s)) = self.burn {
            if jd >= jd_burn {
                return Ok(kepler_propagate(s, (jd - jd_burn) * DAY_S, MU_EARTH));
            }
        }
        let minutes = (jd - self.epoch_jd) * 1440.0;
        let p = self.constants.propagate_afspc_compatibility_mode(sgp4::MinutesSinceEpoch(minutes)).map_err(|e| DomainError::Propagation(e.to_string()))?;
        Ok(StateVector { r: V3::from_array(p.position) * 1000.0, v: V3::from_array(p.velocity) * 1000.0 })
    }

    /// Earth-fixed state at `jd`.
    pub fn ecef(&self, jd: f64) -> Result<StateVector, DomainError> {
        Ok(inertial_to_ecef(self.teme(jd)?, gmst(jd)))
    }

    /// Sub-satellite point and altitude.
    pub fn ground_point(&self, jd: f64) -> Result<Geodetic, DomainError> {
        let r = self.ecef(jd)?.r;
        Ok(Ecef { x: r.x, y: r.y, z: r.z }.to_geodetic())
    }

    /// The ground track from `jd0` over `minutes`, every `step_s` seconds.
    pub fn ground_track(&self, jd0: f64, minutes: f64, step_s: f64) -> Result<Vec<(f64, Geodetic)>, DomainError> {
        let n = (minutes * 60.0 / step_s).ceil() as usize;
        (0..=n).map(|i| {
            let jd = jd0 + i as f64 * step_s / DAY_S;
            self.ground_point(jd).map(|g| (jd, g))
        }).collect()
    }

    /// A delta-v burn (m/s, in TEME) at `jd`. Afterwards the orbit is two-body.
    pub fn manoeuvre(&mut self, jd: f64, dv: V3) -> Result<(), DomainError> {
        let mut s = self.teme(jd)?;
        s.v += dv;
        self.burn = Some((jd, s));
        Ok(())
    }

    /// Look angles from a ground point: (elevation, azimuth, range), radians and metres.
    pub fn look(&self, site: Geodetic, jd: f64) -> Result<(f64, f64, f64), DomainError> {
        let r = self.ecef(jd)?.r;
        let frame = EnuFrame::new(site);
        let enu = frame.to_enu(Ecef { x: r.x, y: r.y, z: r.z });
        Ok((enu.elevation(), enu.bearing(), enu.norm()))
    }

    /// Access windows over a ground point between `jd0` and `jd1` with
    /// elevation above `min_el` (radians): coarse scan at `step_s`, then
    /// bisection on each crossing and golden-section for the culmination.
    pub fn access_windows(&self, site: Geodetic, jd0: f64, jd1: f64, min_el: f64, step_s: f64) -> Result<Vec<Pass>, DomainError> {
        let el = |jd: f64| self.look(site, jd).map(|l| l.0 - min_el);
        let step = step_s / DAY_S;
        let mut passes = Vec::new();
        let mut t = jd0;
        let mut prev = el(t)?;
        let mut rise = if prev >= 0.0 { Some(jd0) } else { None };
        while t < jd1 {
            let t2 = (t + step).min(jd1);
            let cur = el(t2)?;
            if prev < 0.0 && cur >= 0.0 {
                rise = Some(bisect(&el, t, t2)?);
            } else if prev >= 0.0 && cur < 0.0 {
                let set = bisect(&el, t, t2)?;
                if let Some(r) = rise.take() {
                    passes.push(self.make_pass(site, r, set)?);
                }
            }
            prev = cur;
            t = t2;
        }
        if let Some(r) = rise {
            passes.push(self.make_pass(site, r, jd1)?);
        }
        Ok(passes)
    }

    fn make_pass(&self, site: Geodetic, rise: f64, set: f64) -> Result<Pass, DomainError> {
        // Golden-section search for maximum elevation.
        let g = 0.618_033_988_75;
        let (mut a, mut b) = (rise, set);
        for _ in 0..40 {
            let c = b - g * (b - a);
            let d = a + g * (b - a);
            if self.look(site, c)?.0 > self.look(site, d)?.0 {
                b = d;
            } else {
                a = c;
            }
        }
        let culminate = 0.5 * (a + b);
        Ok(Pass { rise, culminate, set, max_elevation: self.look(site, culminate)?.0 })
    }
}

fn bisect(f: &dyn Fn(f64) -> Result<f64, DomainError>, mut a: f64, mut b: f64) -> Result<f64, DomainError> {
    let fa0 = f(a)?;
    for _ in 0..40 {
        let m = 0.5 * (a + b);
        let fm = f(m)?;
        if (fm >= 0.0) == (fa0 >= 0.0) {
            a = m;
        } else {
            b = m;
        }
    }
    Ok(0.5 * (a + b))
}

/// One pass over a ground point: events for the scheduler (§6.2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pass {
    pub rise: f64,
    pub culminate: f64,
    pub set: f64,
    pub max_elevation: f64,
}

impl Pass {
    pub fn duration_s(&self) -> f64 {
        (self.set - self.rise) * DAY_S
    }
}

/// Mean revisit: mean gap between pass starts, seconds.
pub fn mean_revisit(passes: &[Pass]) -> Option<f64> {
    if passes.len() < 2 {
        return None;
    }
    let gaps: Vec<f64> = passes.windows(2).map(|w| (w[1].rise - w[0].rise) * DAY_S).collect();
    Some(gaps.iter().sum::<f64>() / gaps.len() as f64)
}

/// Ground swath width (m) for a nadir sensor with half field of view `half_fov`
/// (radians) at altitude `h` (m), over a spherical Earth.
pub fn swath_width(h: f64, half_fov: f64) -> f64 {
    let re = bgws_core::geo::EARTH_RADIUS;
    let s = ((re + h) / re * half_fov.sin()).min(1.0);
    let central = s.asin() - half_fov;
    2.0 * re * central
}

/// Is a ground point inside the swath at `jd`?
pub fn in_swath(sat: &Satellite, site: Geodetic, jd: f64, half_fov: f64) -> Result<bool, DomainError> {
    let sub = sat.ground_point(jd)?;
    Ok(sub.surface_distance(site) <= swath_width(sub.h, half_fov) / 2.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImagerKind {
    Eo,
    Sar,
}

/// An imagery request waiting for a pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageryRequest {
    pub id: u64,
    pub site: Geodetic,
    pub requested_jd: f64,
    pub priority: u8,
}

/// What came of a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageryProduct {
    pub request: u64,
    pub collected_jd: f64,
    /// When the analysed product reaches the requester.
    pub delivered_jd: f64,
    pub clouded: bool,
    pub gsd_m: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageryParams {
    pub per_pass: usize,
    pub downlink_min: f64,
    pub analysis_min: f64,
    pub gsd_eo: f64,
    pub gsd_sar: f64,
}

impl ImageryParams {
    pub fn from_figures(f: &Figures) -> Self {
        ImageryParams {
            per_pass: f.get("space.img.per_pass") as usize,
            downlink_min: f.get("space.img.downlink_min"),
            analysis_min: f.get("space.img.analysis_min"),
            gsd_eo: f.get("space.img.gsd_eo"),
            gsd_sar: f.get("space.img.gsd_sar"),
        }
    }
}

/// Serve a tasking queue in priority then request order over one pass:
/// EO is blocked when the site is clouded (`cloud(site)` gives cloud cover
/// 0–1, `u` the caller's uniform draw per request); SAR always collects.
pub fn serve_pass(queue: &mut Vec<ImageryRequest>, pass: &Pass, kind: ImagerKind, p: &ImageryParams, cloud: &dyn Fn(Geodetic) -> f64, u: &mut dyn FnMut() -> f64) -> Vec<ImageryProduct> {
    queue.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.requested_jd.total_cmp(&b.requested_jd)).then(a.id.cmp(&b.id)));
    let take = queue.len().min(p.per_pass);
    let served: Vec<ImageryRequest> = queue.drain(..take).collect();
    served
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let clouded = kind == ImagerKind::Eo && u() < cloud(r.site);
            let collected = pass.culminate;
            let delivered = pass.set + (p.downlink_min + p.analysis_min * (1 + i) as f64) / 1440.0;
            ImageryProduct { request: r.id, collected_jd: collected, delivered_jd: delivered, clouded, gsd_m: if kind == ImagerKind::Eo { p.gsd_eo } else { p.gsd_sar } }
        })
        .collect()
}

/// GNSS geometry at a site: satellites above the mask and the dilution of precision.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GnssGeometry {
    pub visible: usize,
    pub gdop: f64,
    pub pdop: f64,
    pub hdop: f64,
    pub vdop: f64,
}

/// DOP from line-of-sight unit vectors in the local ENU frame.
pub fn dop(los_enu: &[V3]) -> Option<GnssGeometry> {
    if los_enu.len() < 4 {
        return None;
    }
    let mut n = [[0.0f64; 4]; 4];
    for u in los_enu {
        let row = [-u.x, -u.y, -u.z, 1.0];
        for i in 0..4 {
            for j in 0..4 {
                n[i][j] += row[i] * row[j];
            }
        }
    }
    let q = invert4(n)?;
    let pdop = (q[0][0] + q[1][1] + q[2][2]).sqrt();
    Some(GnssGeometry { visible: los_enu.len(), gdop: (q[0][0] + q[1][1] + q[2][2] + q[3][3]).sqrt(), pdop, hdop: (q[0][0] + q[1][1]).sqrt(), vdop: q[2][2].sqrt() })
}

fn invert4(m: [[f64; 4]; 4]) -> Option<[[f64; 4]; 4]> {
    let mut a = [[0.0f64; 8]; 4];
    for i in 0..4 {
        a[i][..4].copy_from_slice(&m[i]);
        a[i][4 + i] = 1.0;
    }
    for c in 0..4 {
        let p = (c..4).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        let d = a[c][c];
        for v in a[c].iter_mut() {
            *v /= d;
        }
        for r in 0..4 {
            if r != c {
                let k = a[r][c];
                let pivot = a[c];
                for (v, pv) in a[r].iter_mut().zip(pivot.iter()) {
                    *v -= k * pv;
                }
            }
        }
    }
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        out[i].copy_from_slice(&a[i][4..]);
    }
    Some(out)
}

/// GNSS geometry at a site from a constellation, with an elevation mask.
pub fn gnss_geometry(constellation: &[Satellite], site: Geodetic, jd: f64, mask: f64) -> Result<Option<GnssGeometry>, DomainError> {
    let frame = EnuFrame::new(site);
    let mut los = Vec::new();
    for s in constellation {
        let r = s.ecef(jd)?.r;
        let e = frame.to_enu(Ecef { x: r.x, y: r.y, z: r.z });
        if e.elevation() >= mask {
            let n = e.norm();
            los.push(V3::new(e.e / n, e.n / n, e.u / n));
        }
    }
    Ok(dop(&los).or(if los.is_empty() { None } else { Some(GnssGeometry { visible: los.len(), gdop: f64::INFINITY, pdop: f64::INFINITY, hdop: f64::INFINITY, vdop: f64::INFINITY }) }))
}

/// A satcom beam with shared capacity; a provider can switch it off [R5.4].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beam {
    pub centre: Geodetic,
    pub radius_m: f64,
    pub capacity_bps: f64,
    pub on: bool,
}

impl Beam {
    pub fn covers(&self, p: Geodetic) -> bool {
        self.on && self.centre.surface_distance(p) <= self.radius_m
    }
    /// Share capacity among demands (bps) proportionally when oversubscribed.
    pub fn allocate(&self, demands: &[f64]) -> Vec<f64> {
        if !self.on {
            return vec![0.0; demands.len()];
        }
        let total: f64 = demands.iter().sum();
        let k = if total > self.capacity_bps { self.capacity_bps / total } else { 1.0 };
        demands.iter().map(|d| d * k).collect()
    }
}

/// Convert a scenario clock to a Julian date: `start_jd` + seconds.
pub fn jd_at(start_jd: f64, seconds: f64) -> f64 {
    start_jd + seconds / DAY_S
}

/// Julian date of a UTC calendar date and time.
pub fn julian_date(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> f64 {
    let (y, m) = if month <= 2 { (year - 1, month + 12) } else { (year, month) };
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y as f64 + 4716.0)).floor() + (30.6001 * (m as f64 + 1.0)).floor() + day as f64 + b - 1524.5 + (hour as f64 + minute as f64 / 60.0 + second / 3600.0) / 24.0
}

/// The ECEF vector of a ground point.
pub fn site_vector(site: Geodetic) -> V3 {
    ecef_v(site.to_ecef())
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("space.img.per_pass", 4.0, "count", 1.0, 20.0, "default", Default, "collections per pass over an area"),
        figure!("space.img.downlink_min", 20.0, "min", 2.0, 120.0, "R5.4", Default, "pass end to ground station downlink"),
        figure!("space.img.analysis_min", 30.0, "min", 5.0, 240.0, "default", Default, "analysis time per product in the queue"),
        figure!("space.img.gsd_eo", 0.5, "m", 0.3, 5.0, "default", Default, "EO ground sample distance"),
        figure!("space.img.gsd_sar", 1.0, "m", 0.3, 10.0, "default", Default, "SAR resolution"),
        figure!("space.access.min_el_deg", 10.0, "deg", 0.0, 45.0, "default", Default, "minimum elevation for an access window"),
        figure!("space.gnss.mask_deg", 5.0, "deg", 0.0, 20.0, "default", Default, "GNSS elevation mask"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const L1: &str = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const L2: &str = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
    const ISS1: &str = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927";
    const ISS2: &str = "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";

    #[test]
    fn sgp4_reference_vectors_00005() {
        let s = Satellite::from_tle("vanguard", L1, L2).unwrap();
        let p0 = s.teme(s.epoch_jd).unwrap().r * 1e-3;
        assert!((p0 - V3::new(7022.46529266, -1400.08296755, 0.03995155)).norm() < 1e-5, "{p0:?}");
        let p6 = s.teme(s.epoch_jd + 360.0 / 1440.0).unwrap().r * 1e-3;
        assert!((p6 - V3::new(-7154.03120202, -3783.17682504, -3536.19412294)).norm() < 1e-5, "{p6:?}");
    }

    #[test]
    fn leo_passes_over_a_site() {
        let s = Satellite::from_tle("iss", ISS1, ISS2).unwrap();
        let site = Geodetic::new(51.5, 0.0, 0.0);
        let passes = s.access_windows(site, s.epoch_jd, s.epoch_jd + 1.0, 10f64.to_radians(), 30.0).unwrap();
        assert!(!passes.is_empty() && passes.len() < 10, "{}", passes.len());
        for p in &passes {
            assert!(p.duration_s() > 0.0 && p.duration_s() < 900.0, "{}", p.duration_s());
            assert!(p.max_elevation >= 10f64.to_radians() - 1e-6);
            assert!(p.rise < p.culminate && p.culminate < p.set);
        }
        let alt = s.ground_point(s.epoch_jd).unwrap().h;
        assert!((300e3..450e3).contains(&alt), "{alt}");
    }

    #[test]
    fn manoeuvre_raises_the_orbit() {
        let mut s = Satellite::from_tle("iss", ISS1, ISS2).unwrap();
        let jd = s.epoch_jd + 0.1;
        let st = s.teme(jd).unwrap();
        s.manoeuvre(jd, st.v.unit() * 50.0).unwrap();
        let after = s.teme(jd + 0.05).unwrap();
        assert!(after.semi_major_axis() > st.semi_major_axis() + 50e3);
    }

    #[test]
    fn swath_and_dop() {
        let w = swath_width(500e3, 5f64.to_radians());
        assert!((w - 2.0 * 500e3 * 5f64.to_radians().tan()).abs() / w < 0.02);
        // Four satellites: one overhead, three low and spread: a good geometry.
        let mut los = vec![V3::new(0.0, 0.0, 1.0)];
        for k in 0..3 {
            let az = k as f64 * std::f64::consts::TAU / 3.0;
            let el = 15f64.to_radians();
            los.push(V3::new(az.sin() * el.cos(), az.cos() * el.cos(), el.sin()));
        }
        let g = dop(&los).unwrap();
        assert!(g.pdop > 1.0 && g.pdop < 4.0, "{g:?}");
        assert!(dop(&los[..3]).is_none());
    }

    #[test]
    fn imagery_queue_and_beams() {
        let f = Figures::from_list(figures()).unwrap();
        let p = ImageryParams::from_figures(&f);
        let pass = Pass { rise: 0.0, culminate: 0.002, set: 0.004, max_elevation: 1.0 };
        let mut q: Vec<ImageryRequest> = (0..6).map(|i| ImageryRequest { id: i, site: Geodetic::new(50.0, 30.0, 0.0), requested_jd: 0.0, priority: (i % 3) as u8 }).collect();
        let mut u = || 0.5;
        let eo = serve_pass(&mut q, &pass, ImagerKind::Eo, &p, &|_| 0.9, &mut u);
        assert_eq!(eo.len(), 4);
        assert!(eo.iter().all(|x| x.clouded));
        assert_eq!(q.len(), 2);
        let sar = serve_pass(&mut q, &pass, ImagerKind::Sar, &p, &|_| 0.9, &mut u);
        assert!(sar.iter().all(|x| !x.clouded));
        let beam = Beam { centre: Geodetic::new(50.0, 30.0, 0.0), radius_m: 300e3, capacity_bps: 100.0, on: true };
        assert_eq!(beam.allocate(&[100.0, 100.0]), vec![50.0, 50.0]);
        assert!((julian_date(2000, 1, 1, 12, 0, 0.0) - JD_J2000).abs() < 1e-9);
    }
}
