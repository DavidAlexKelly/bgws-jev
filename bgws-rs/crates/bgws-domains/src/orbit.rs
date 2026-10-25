//! Two-body orbital mechanics and Earth-rotation frames shared by the
//! ballistic-missile midcourse (§8.1 "a midcourse arc with Earth rotation")
//! and the space models (§8.1 Space, §8.12): universal-variable Kepler
//! propagation, Greenwich mean sidereal time, and the TEME/ECI ↔ ECEF
//! rotation. Physical constants only; no figures.

use crate::vec3::V3;
use bgws_core::Ecef;
use serde::{Deserialize, Serialize};

/// Earth's gravitational parameter (WGS84), m³/s².
pub const MU_EARTH: f64 = 3.986_004_418e14;
/// Earth's rotation rate (WGS84), rad/s.
pub const OMEGA_EARTH: f64 = 7.292_115e-5;
/// Julian date of J2000.0.
pub const JD_J2000: f64 = 2_451_545.0;
/// Seconds per day.
pub const DAY_S: f64 = 86_400.0;

/// Position and velocity, metres and m/s, in an inertial or Earth-fixed frame
/// as the caller states.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct StateVector {
    pub r: V3,
    pub v: V3,
}

impl StateVector {
    /// Specific orbital energy, J/kg.
    pub fn energy(&self) -> f64 {
        0.5 * self.v.norm2() - MU_EARTH / self.r.norm()
    }
    /// Specific angular momentum vector.
    pub fn angular_momentum(&self) -> V3 {
        self.r.cross(self.v)
    }
    /// Semi-major axis (negative for hyperbolic), m.
    pub fn semi_major_axis(&self) -> f64 {
        -MU_EARTH / (2.0 * self.energy())
    }
    /// Eccentricity vector magnitude.
    pub fn eccentricity(&self) -> f64 {
        let h = self.angular_momentum();
        (self.v.cross(h) * (1.0 / MU_EARTH) - self.r.unit()).norm()
    }
    /// Apogee and perigee radii (m) of the osculating ellipse.
    pub fn apsides(&self) -> (f64, f64) {
        let a = self.semi_major_axis();
        let e = self.eccentricity();
        (a * (1.0 + e), a * (1.0 - e))
    }
}

fn stumpff(psi: f64) -> (f64, f64) {
    if psi > 1e-6 {
        let s = psi.sqrt();
        ((1.0 - s.cos()) / psi, (s - s.sin()) / (s * psi))
    } else if psi < -1e-6 {
        let s = (-psi).sqrt();
        ((1.0 - s.cosh()) / psi, (s.sinh() - s) / (s * -psi))
    } else {
        (0.5 - psi / 24.0, 1.0 / 6.0 - psi / 120.0)
    }
}

/// Propagate a two-body state by `dt` seconds with the universal-variable
/// formulation (Vallado, Algorithm 8). Works for elliptic, parabolic and
/// hyperbolic arcs and for negative `dt`.
pub fn kepler_propagate(s: StateVector, dt: f64, mu: f64) -> StateVector {
    if dt == 0.0 {
        return s;
    }
    let r0 = s.r.norm();
    let v0 = s.v.norm();
    let rv = s.r.dot(s.v);
    let smu = mu.sqrt();
    let alpha = 2.0 / r0 - v0 * v0 / mu;
    let mut chi = if alpha > 1e-12 {
        smu * dt * alpha
    } else if alpha < -1e-12 {
        let a = 1.0 / alpha;
        let sg = dt.signum();
        let num = -2.0 * mu * alpha * dt;
        let den = rv + sg * (-mu * a).sqrt() * (1.0 - r0 * alpha);
        sg * (-a).sqrt() * (num / den).abs().ln()
    } else {
        smu * dt / r0
    };
    let mut r = r0;
    let (mut c2, mut c3) = (0.5, 1.0 / 6.0);
    for _ in 0..60 {
        let psi = chi * chi * alpha;
        let (a2, a3) = stumpff(psi);
        c2 = a2;
        c3 = a3;
        r = chi * chi * c2 + rv / smu * chi * (1.0 - psi * c3) + r0 * (1.0 - psi * c2);
        let f = smu * dt - chi.powi(3) * c3 - rv / smu * chi * chi * c2 - r0 * chi * (1.0 - psi * c3);
        let d = f / r;
        chi += d;
        if d.abs() < 1e-9 {
            break;
        }
    }
    let psi = chi * chi * alpha;
    let f = 1.0 - chi * chi / r0 * c2;
    let g = dt - chi.powi(3) / smu * c3;
    let gdot = 1.0 - chi * chi / r * c2;
    let fdot = smu / (r * r0) * chi * (psi * c3 - 1.0);
    StateVector { r: s.r * f + s.v * g, v: s.r * fdot + s.v * gdot }
}

/// Greenwich mean sidereal time (IAU-82), radians, for a UT1 Julian date
/// (UTC is used for UT1; the < 0.9 s difference is ~400 m at the equator).
pub fn gmst(jd_ut1: f64) -> f64 {
    let t = (jd_ut1 - JD_J2000) / 36_525.0;
    let secs = 67_310.548_41 + (876_600.0 * 3600.0 + 8_640_184.812_866) * t + 0.093_104 * t * t - 6.2e-6 * t * t * t;
    (secs.rem_euclid(DAY_S) / 240.0).to_radians()
}

/// Rotate an inertial (TEME or simple ECI) state into ECEF at sidereal angle
/// `theta` (polar motion ignored). Velocity includes the −ω×r term.
pub fn inertial_to_ecef(s: StateVector, theta: f64) -> StateVector {
    let r = s.r.rotate_z(-theta);
    let w = V3::new(0.0, 0.0, OMEGA_EARTH);
    let v = s.v.rotate_z(-theta) - w.cross(r);
    StateVector { r, v }
}

/// The inverse of [`inertial_to_ecef`].
pub fn ecef_to_inertial(s: StateVector, theta: f64) -> StateVector {
    let w = V3::new(0.0, 0.0, OMEGA_EARTH);
    let v = (s.v + w.cross(s.r)).rotate_z(theta);
    StateVector { r: s.r.rotate_z(theta), v }
}

/// Convert a core ECEF point to a vector.
pub fn ecef_v(e: Ecef) -> V3 {
    V3::new(e.x, e.y, e.z)
}
/// Convert a vector to a core ECEF point.
pub fn v_ecef(v: V3) -> Ecef {
    Ecef { x: v.x, y: v.y, z: v.z }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    #[test]
    fn circular_orbit_returns_after_one_period() {
        let r = 7_000_000.0;
        let v = (MU_EARTH / r).sqrt();
        let s = StateVector { r: V3::new(r, 0.0, 0.0), v: V3::new(0.0, v, 0.0) };
        let period = std::f64::consts::TAU * (r.powi(3) / MU_EARTH).sqrt();
        let back = kepler_propagate(s, period, MU_EARTH);
        assert!((back.r - s.r).norm() < 1e-3, "{:?}", back.r);
        let half = kepler_propagate(s, period / 2.0, MU_EARTH);
        assert_relative_eq!(half.r.x, -r, epsilon = 1e-3);
    }

    #[test]
    fn gmst_reference() {
        // Vallado Example 3-5: 1992-08-20 12:14 UT1 → GMST 152.578788°.
        let jd = 2_448_855.009_722;
        assert_relative_eq!(gmst(jd).to_degrees(), 152.578_788, epsilon = 1e-3);
    }

    proptest! {
        #[test]
        fn kepler_conserves_energy_and_momentum(
            rx in 6.6e6f64..4.0e7, vy in 3000.0f64..11_000.0, vz in -2000.0f64..2000.0, dt in -20_000.0f64..20_000.0
        ) {
            let s = StateVector { r: V3::new(rx, 0.0, 0.0), v: V3::new(300.0, vy, vz) };
            let p = kepler_propagate(s, dt, MU_EARTH);
            prop_assert!(((p.energy() - s.energy()) / s.energy()).abs() < 1e-7);
            prop_assert!((p.angular_momentum() - s.angular_momentum()).norm() / s.angular_momentum().norm() < 1e-7);
            let back = kepler_propagate(p, -dt, MU_EARTH);
            prop_assert!((back.r - s.r).norm() < 1.0);
        }

        #[test]
        fn frame_round_trip(x in -7e6f64..7e6, y in -7e6f64..7e6, th in 0.0f64..6.0) {
            let s = StateVector { r: V3::new(x, y, 1e6), v: V3::new(10.0, -20.0, 30.0) };
            let b = ecef_to_inertial(inertial_to_ecef(s, th), th);
            prop_assert!((b.r - s.r).norm() < 1e-6 && (b.v - s.v).norm() < 1e-9);
        }
    }
}
