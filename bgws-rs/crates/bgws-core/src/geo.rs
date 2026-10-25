//! WGS84 geodesy (§5.2): geodetic for files, ECEF f64 in memory for anything
//! that moves, and local ENU tangent frames for computation within a
//! neighbourhood (a grid cell, an engagement, a sensor footprint).

use serde::{Deserialize, Serialize};

pub const WGS84_A: f64 = 6_378_137.0;
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;
pub const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);
pub const WGS84_E2: f64 = WGS84_F * (2.0 - WGS84_F);
/// Mean Earth radius, for spherical approximations.
pub const EARTH_RADIUS: f64 = 6_371_008.8;
/// Effective radius factor for radar and radio refraction (the 4/3-Earth model).
pub const K_RADIO: f64 = 4.0 / 3.0;

/// Latitude and longitude in degrees, height above the ellipsoid in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Geodetic {
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub h: f64,
}

/// Earth-centred, Earth-fixed metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Ecef {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// East, north, up metres in some local tangent frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Enu {
    pub e: f64,
    pub n: f64,
    pub u: f64,
}

impl Geodetic {
    pub fn new(lat: f64, lon: f64, h: f64) -> Self {
        Self { lat, lon, h }
    }

    pub fn to_ecef(self) -> Ecef {
        let (lat, lon) = (self.lat.to_radians(), self.lon.to_radians());
        let (sl, cl) = lat.sin_cos();
        let n = WGS84_A / (1.0 - WGS84_E2 * sl * sl).sqrt();
        Ecef {
            x: (n + self.h) * cl * lon.cos(),
            y: (n + self.h) * cl * lon.sin(),
            z: (n * (1.0 - WGS84_E2) + self.h) * sl,
        }
    }

    /// Great-circle surface distance (haversine on the mean sphere), metres.
    pub fn surface_distance(self, o: Geodetic) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), o.lat.to_radians());
        let dp = p2 - p1;
        let dl = (o.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS * a.sqrt().atan2((1.0 - a).sqrt())
    }

    /// Initial bearing to `o`, radians clockwise from north.
    pub fn bearing_to(self, o: Geodetic) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), o.lat.to_radians());
        let dl = (o.lon - self.lon).to_radians();
        let y = dl.sin() * p2.cos();
        let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
        y.atan2(x).rem_euclid(std::f64::consts::TAU)
    }

    /// The point `dist` metres along `bearing` (radians) on the mean sphere.
    pub fn destination(self, bearing: f64, dist: f64) -> Geodetic {
        let d = dist / EARTH_RADIUS;
        let p1 = self.lat.to_radians();
        let l1 = self.lon.to_radians();
        let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * bearing.cos()).asin();
        let l2 = l1 + (bearing.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
        Geodetic { lat: p2.to_degrees(), lon: ((l2.to_degrees() + 540.0) % 360.0) - 180.0, h: self.h }
    }
}

impl Ecef {
    /// Bowring's iterative inverse; sub-millimetre for terrestrial and orbital heights.
    pub fn to_geodetic(self) -> Geodetic {
        let p = (self.x * self.x + self.y * self.y).sqrt();
        let lon = self.y.atan2(self.x);
        if p < 1e-9 {
            let lat = if self.z >= 0.0 { 90.0 } else { -90.0 };
            return Geodetic { lat, lon: 0.0, h: self.z.abs() - WGS84_B };
        }
        let mut lat = (self.z / (p * (1.0 - WGS84_E2))).atan();
        let mut h = 0.0;
        for _ in 0..6 {
            let sl = lat.sin();
            let n = WGS84_A / (1.0 - WGS84_E2 * sl * sl).sqrt();
            h = p / lat.cos() - n;
            lat = (self.z / (p * (1.0 - WGS84_E2 * n / (n + h)))).atan();
        }
        Geodetic { lat: lat.to_degrees(), lon: lon.to_degrees(), h }
    }

    pub fn distance(self, o: Ecef) -> f64 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2) + (self.z - o.z).powi(2)).sqrt()
    }

    pub fn add(self, d: [f64; 3]) -> Ecef {
        Ecef { x: self.x + d[0], y: self.y + d[1], z: self.z + d[2] }
    }
}

impl Enu {
    pub fn new(e: f64, n: f64, u: f64) -> Self {
        Self { e, n, u }
    }
    pub fn norm(self) -> f64 {
        (self.e * self.e + self.n * self.n + self.u * self.u).sqrt()
    }
    pub fn horizontal(self) -> f64 {
        self.e.hypot(self.n)
    }
    /// Bearing, radians clockwise from north.
    pub fn bearing(self) -> f64 {
        self.e.atan2(self.n).rem_euclid(std::f64::consts::TAU)
    }
    pub fn elevation(self) -> f64 {
        self.u.atan2(self.horizontal())
    }
    pub fn sub(self, o: Enu) -> Enu {
        Enu::new(self.e - o.e, self.n - o.n, self.u - o.u)
    }
    pub fn add(self, o: Enu) -> Enu {
        Enu::new(self.e + o.e, self.n + o.n, self.u + o.u)
    }
    pub fn scale(self, k: f64) -> Enu {
        Enu::new(self.e * k, self.n * k, self.u * k)
    }
    pub fn dot(self, o: Enu) -> f64 {
        self.e * o.e + self.n * o.n + self.u * o.u
    }
}

/// A local tangent frame at an origin: converts between ECEF and ENU exactly
/// (a rotation and translation), so a neighbourhood computes in flat metres
/// without accumulating a curvature error.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnuFrame {
    pub origin: Geodetic,
    origin_ecef: Ecef,
    /// Rows: east, north, up unit vectors in ECEF.
    r: [[f64; 3]; 3],
}

impl EnuFrame {
    pub fn new(origin: Geodetic) -> Self {
        let (lat, lon) = (origin.lat.to_radians(), origin.lon.to_radians());
        let (sl, cl) = lat.sin_cos();
        let (so, co) = lon.sin_cos();
        let r = [[-so, co, 0.0], [-sl * co, -sl * so, cl], [cl * co, cl * so, sl]];
        Self { origin, origin_ecef: origin.to_ecef(), r }
    }

    pub fn to_enu(&self, p: Ecef) -> Enu {
        let d = [p.x - self.origin_ecef.x, p.y - self.origin_ecef.y, p.z - self.origin_ecef.z];
        let row = |i: usize| self.r[i][0] * d[0] + self.r[i][1] * d[1] + self.r[i][2] * d[2];
        Enu { e: row(0), n: row(1), u: row(2) }
    }

    pub fn to_ecef(&self, v: Enu) -> Ecef {
        let c = |j: usize| self.r[0][j] * v.e + self.r[1][j] * v.n + self.r[2][j] * v.u;
        Ecef { x: self.origin_ecef.x + c(0), y: self.origin_ecef.y + c(1), z: self.origin_ecef.z + c(2) }
    }

    /// Rotate a vector (a velocity) from ENU to ECEF, without translation.
    pub fn vec_to_ecef(&self, v: Enu) -> [f64; 3] {
        let c = |j: usize| self.r[0][j] * v.e + self.r[1][j] * v.n + self.r[2][j] * v.u;
        [c(0), c(1), c(2)]
    }

    pub fn vec_to_enu(&self, d: [f64; 3]) -> Enu {
        let row = |i: usize| self.r[i][0] * d[0] + self.r[i][1] * d[1] + self.r[i][2] * d[2];
        Enu { e: row(0), n: row(1), u: row(2) }
    }

    pub fn geodetic_to_enu(&self, g: Geodetic) -> Enu {
        self.to_enu(g.to_ecef())
    }

    pub fn enu_to_geodetic(&self, v: Enu) -> Geodetic {
        self.to_ecef(v).to_geodetic()
    }
}

/// How far the Earth's surface drops below a tangent plane at ground range
/// `d`, with effective-radius factor `k` (1 for geometry, 4/3 for radio,
/// about 1.15 for optical refraction).
pub fn curvature_drop(d: f64, k: f64) -> f64 {
    d * d / (2.0 * k * EARTH_RADIUS)
}

/// Distance to the horizon from height `h` metres, with factor `k`.
pub fn horizon_distance(h: f64, k: f64) -> f64 {
    (2.0 * k * EARTH_RADIUS * h.max(0.0)).sqrt()
}

/// The maximum range at which two heights see each other over a smooth Earth.
pub fn mutual_horizon(h1: f64, h2: f64, k: f64) -> f64 {
    horizon_distance(h1, k) + horizon_distance(h2, k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    #[test]
    fn known_points() {
        let e = Geodetic::new(0.0, 0.0, 0.0).to_ecef();
        assert_relative_eq!(e.x, WGS84_A, epsilon = 1e-6);
        let p = Geodetic::new(90.0, 0.0, 0.0).to_ecef();
        assert_relative_eq!(p.z, WGS84_B, epsilon = 1e-6);
    }

    #[test]
    fn radar_horizon_from_ten_metres_is_about_thirteen_km() {
        let d = horizon_distance(10.0, K_RADIO);
        assert!((d - 13_030.0).abs() < 50.0, "{d}");
    }

    #[test]
    fn destination_and_distance_agree() {
        let a = Geodetic::new(50.0, 30.0, 0.0);
        let b = a.destination(1.0, 25_000.0);
        assert_relative_eq!(a.surface_distance(b), 25_000.0, epsilon = 1e-3);
        assert_relative_eq!(a.bearing_to(b), 1.0, epsilon = 1e-4);
    }

    proptest! {
        #[test]
        fn ecef_round_trip(lat in -89.9f64..89.9, lon in -179.9f64..179.9, h in -500.0f64..40_000_000.0) {
            let g = Geodetic::new(lat, lon, h);
            let back = g.to_ecef().to_geodetic();
            prop_assert!((back.lat - lat).abs() < 1e-8);
            prop_assert!((back.lon - lon).abs() < 1e-8);
            prop_assert!((back.h - h).abs() < 1e-3);
        }

        #[test]
        fn enu_round_trip(lat in -80.0f64..80.0, lon in -179.0f64..179.0, e in -1e5f64..1e5, n in -1e5f64..1e5, u in -1e3f64..1e4) {
            let f = EnuFrame::new(Geodetic::new(lat, lon, 0.0));
            let v = Enu::new(e, n, u);
            let back = f.to_enu(f.to_ecef(v));
            prop_assert!((back.e - e).abs() < 1e-6 && (back.n - n).abs() < 1e-6 && (back.u - u).abs() < 1e-6);
        }
    }
}
