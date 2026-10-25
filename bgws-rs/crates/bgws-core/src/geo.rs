//! Geodesy (§5.2). Positions are WGS84 latitude, longitude and height. Long
//! paths (satellites, ballistic missiles, theatre radar) use ECEF exactly;
//! everything within a few hundred kilometres uses a local tangent plane at
//! the pair's midpoint, which is accurate to well under 0.1% there and fast.
//!
//! Earth curvature and refraction enter line of sight and propagation through
//! the effective-Earth-radius factor k: the ray's height above the surface
//! along a path loses d1·d2 / (2·k·R) (4/3 for radio and radar, about 7/6 for
//! light) [default].

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

pub const WGS84_A: f64 = 6_378_137.0;
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);
/// Mean Earth radius (IUGG), for spherical approximations.
pub const R_EARTH: f64 = 6_371_008.8;
/// Effective Earth radius factor for radio and radar (standard atmosphere).
pub const K_RADIO: f64 = 4.0 / 3.0;
/// Effective Earth radius factor for visible light [default].
pub const K_OPTICAL: f64 = 7.0 / 6.0;

/// A point on the map, degrees.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub const fn new(lat: f64, lon: f64) -> LatLon {
        LatLon { lat, lon }
    }
    pub fn with_h(self, h: f64) -> Geo {
        Geo { lat: self.lat, lon: self.lon, h }
    }
}

/// A point with height above mean sea level, metres (the geoid is taken as
/// the ellipsoid: the difference never matters within one theatre).
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Geo {
    pub lat: f64,
    pub lon: f64,
    pub h: f64,
}

impl Geo {
    pub fn ll(self) -> LatLon {
        LatLon { lat: self.lat, lon: self.lon }
    }
    pub fn ecef(self) -> DVec3 {
        geodetic_to_ecef(self)
    }
}

pub fn geodetic_to_ecef(g: Geo) -> DVec3 {
    let (lat, lon) = (g.lat.to_radians(), g.lon.to_radians());
    let (sl, cl) = lat.sin_cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sl * sl).sqrt();
    DVec3::new((n + g.h) * cl * lon.cos(), (n + g.h) * cl * lon.sin(), (n * (1.0 - WGS84_E2) + g.h) * sl)
}

/// ECEF to geodetic, by Bowring's method with two refinements: millimetre-exact at any altitude used here.
pub fn ecef_to_geodetic(p: DVec3) -> Geo {
    let lon = p.y.atan2(p.x);
    let r = (p.x * p.x + p.y * p.y).sqrt();
    if r < 1e-9 {
        let h = p.z.abs() - WGS84_B;
        return Geo { lat: if p.z >= 0.0 { 90.0 } else { -90.0 }, lon: 0.0, h };
    }
    let ep2 = (WGS84_A * WGS84_A - WGS84_B * WGS84_B) / (WGS84_B * WGS84_B);
    let mut lat = (p.z / (r * (1.0 - WGS84_E2))).atan();
    for _ in 0..4 {
        let beta = ((1.0 - WGS84_F) * lat.tan()).atan();
        let (sb, cb) = beta.sin_cos();
        lat = (p.z + ep2 * WGS84_B * sb * sb * sb).atan2(r - WGS84_E2 * WGS84_A * cb * cb * cb);
    }
    let sl = lat.sin();
    let n = WGS84_A / (1.0 - WGS84_E2 * sl * sl).sqrt();
    let h = if lat.cos().abs() > 1e-6 { r / lat.cos() - n } else { p.z.abs() / sl.abs() - n * (1.0 - WGS84_E2) };
    Geo { lat: lat.to_degrees(), lon: lon.to_degrees(), h }
}

/// A local east-north-up frame at an origin: for exact work around one point.
#[derive(Copy, Clone, Debug)]
pub struct EnuFrame {
    pub origin: Geo,
    ecef0: DVec3,
    /// Rows: east, north, up, in ECEF.
    rot: DMat3,
}

impl EnuFrame {
    pub fn new(origin: Geo) -> EnuFrame {
        let (lat, lon) = (origin.lat.to_radians(), origin.lon.to_radians());
        let (sl, cl) = lat.sin_cos();
        let (so, co) = lon.sin_cos();
        let east = DVec3::new(-so, co, 0.0);
        let north = DVec3::new(-sl * co, -sl * so, cl);
        let up = DVec3::new(cl * co, cl * so, sl);
        // glam matrices are column-major: build the transpose (columns = e, n, u) and transpose it.
        let rot = DMat3::from_cols(east, north, up).transpose();
        EnuFrame { origin, ecef0: geodetic_to_ecef(origin), rot }
    }
    pub fn to_enu(&self, ecef: DVec3) -> DVec3 {
        self.rot * (ecef - self.ecef0)
    }
    pub fn from_enu(&self, enu: DVec3) -> DVec3 {
        self.rot.transpose() * enu + self.ecef0
    }
    pub fn geo_to_enu(&self, g: Geo) -> DVec3 {
        self.to_enu(geodetic_to_ecef(g))
    }
    pub fn enu_to_geo(&self, enu: DVec3) -> Geo {
        ecef_to_geodetic(self.from_enu(enu))
    }
    /// A direction (velocity) in ENU at this origin, as ECEF.
    pub fn dir_to_ecef(&self, enu: DVec3) -> DVec3 {
        self.rot.transpose() * enu
    }
}

/// East and north metres from `a` to `b`, in the tangent plane at their midpoint.
/// Accurate to about 0.1% at 500 km; use ECEF beyond that.
#[inline]
pub fn local_en(a: LatLon, b: LatLon) -> (f64, f64) {
    let mid = ((a.lat + b.lat) * 0.5).to_radians();
    let mut dlon = b.lon - a.lon;
    if dlon > 180.0 {
        dlon -= 360.0;
    } else if dlon < -180.0 {
        dlon += 360.0;
    }
    (dlon.to_radians() * R_EARTH * mid.cos(), (b.lat - a.lat).to_radians() * R_EARTH)
}

/// Ground distance, metres: the local plane up to 200 km, the haversine beyond.
#[inline]
pub fn ground_dist(a: LatLon, b: LatLon) -> f64 {
    let (e, n) = local_en(a, b);
    let d = (e * e + n * n).sqrt();
    if d < 200_000.0 {
        d
    } else {
        haversine(a, b)
    }
}

pub fn haversine(a: LatLon, b: LatLon) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp * 0.5).sin().powi(2) + p1.cos() * p2.cos() * (dl * 0.5).sin().powi(2);
    2.0 * R_EARTH * h.sqrt().min(1.0).asin()
}

/// Slant distance between two points with heights, metres (ECEF chord).
pub fn slant_dist(a: Geo, b: Geo) -> f64 {
    let d = ground_dist(a.ll(), b.ll());
    if d < 50_000.0 {
        let dh = b.h - a.h;
        (d * d + dh * dh).sqrt()
    } else {
        (a.ecef() - b.ecef()).length()
    }
}

/// Initial bearing from `a` to `b`, degrees clockwise from north, 0–360.
pub fn bearing_deg(a: LatLon, b: LatLon) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// The point `dist_m` from `a` along `bearing_deg` (spherical).
pub fn destination(a: LatLon, bearing_deg: f64, dist_m: f64) -> LatLon {
    let d = dist_m / R_EARTH;
    let b = bearing_deg.to_radians();
    let p1 = a.lat.to_radians();
    let l1 = a.lon.to_radians();
    let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * b.cos()).asin();
    let l2 = l1 + (b.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
    LatLon { lat: p2.to_degrees(), lon: ((l2.to_degrees() + 540.0) % 360.0) - 180.0 }
}

/// Move a point by east and north metres (local plane).
#[inline]
pub fn offset_en(a: LatLon, east_m: f64, north_m: f64) -> LatLon {
    let lat = a.lat + (north_m / R_EARTH).to_degrees();
    let lon = a.lon + (east_m / (R_EARTH * a.lat.to_radians().cos().max(1e-6))).to_degrees();
    LatLon { lat, lon }
}

/// Linear interpolation between two nearby points.
#[inline]
pub fn lerp_ll(a: LatLon, b: LatLon, t: f64) -> LatLon {
    LatLon { lat: a.lat + (b.lat - a.lat) * t, lon: a.lon + (b.lon - a.lon) * t }
}

/// How much the Earth's bulge (with refraction factor k) lowers the line of
/// sight at a point d1 from one end and d2 from the other, metres.
#[inline]
pub fn bulge_m(d1: f64, d2: f64, k: f64) -> f64 {
    d1 * d2 / (2.0 * k * R_EARTH)
}

/// The horizon distance between two heights above smooth Earth, metres:
/// d = √(2kR·h1) + √(2kR·h2). With k = 4/3 this is the radar horizon
/// d ≈ 4.12(√h1 + √h2) km (h in metres).
pub fn horizon_m(h1: f64, h2: f64, k: f64) -> f64 {
    let kr2 = 2.0 * k * R_EARTH;
    (kr2 * h1.max(0.0)).sqrt() + (kr2 * h2.max(0.0)).sqrt()
}

/// Elevation angle of `b` seen from `a`, degrees, with Earth curvature.
pub fn elevation_deg(a: Geo, b: Geo) -> f64 {
    let f = EnuFrame::new(a);
    let v = f.geo_to_enu(b);
    let horiz = (v.x * v.x + v.y * v.y).sqrt();
    v.z.atan2(horiz).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn ecef_round_trip() {
        for &(lat, lon, h) in &[(0.0, 0.0, 0.0), (51.5, -0.12, 100.0), (-33.9, 151.2, 8000.0), (80.0, 170.0, 550_000.0), (48.0, 37.0, -50.0)] {
            let g = Geo { lat, lon, h };
            let back = ecef_to_geodetic(geodetic_to_ecef(g));
            assert_relative_eq!(back.lat, lat, epsilon = 1e-9);
            assert_relative_eq!(back.lon, lon, epsilon = 1e-9);
            assert_relative_eq!(back.h, h, epsilon = 1e-3);
        }
    }

    #[test]
    fn enu_round_trip_and_axes() {
        let f = EnuFrame::new(Geo { lat: 48.0, lon: 37.0, h: 200.0 });
        let p = Geo { lat: 48.01, lon: 37.0, h: 200.0 };
        let enu = f.geo_to_enu(p);
        assert!(enu.y > 1100.0 && enu.y < 1115.0 && enu.x.abs() < 1.0, "{enu:?}");
        let back = f.enu_to_geo(enu);
        assert_relative_eq!(back.lat, p.lat, epsilon = 1e-10);
    }

    #[test]
    fn local_distance_matches_haversine() {
        let a = LatLon::new(48.0, 37.0);
        for d in [100.0, 10_000.0, 150_000.0, 400_000.0] {
            let b = destination(a, 63.0, d);
            let local = {
                let (e, n) = local_en(a, b);
                (e * e + n * n).sqrt()
            };
            assert!((local - d).abs() / d < 1.5e-3, "{d} {local}");
            assert!((ground_dist(a, b) - d).abs() / d < 1.5e-3);
            assert!((bearing_deg(a, b) - 63.0).abs() < 1.0);
        }
    }

    #[test]
    fn radar_horizon() {
        // 4.12 (√h1 + √h2) km with k = 4/3.
        let d = horizon_m(25.0, 9.0, K_RADIO);
        assert_relative_eq!(d / 1000.0, 4.12 * (5.0 + 3.0), max_relative = 0.01);
    }
}
