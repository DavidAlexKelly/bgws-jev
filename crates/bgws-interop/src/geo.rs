//! Where an entity is, as other simulations read it: geocentric (ECEF)
//! position on WGS-84, velocity in ECEF, and the DIS orientation angles.
//!
//! The sim's ground is a flat local map: east and north metres from the
//! theatre origin, and height above mean sea level from the terrain. So
//! the horizontal position gives latitude and longitude (through the origin's
//! tangent frame), and the height is the sim's height plus the geoid
//! undulation. Running the 3-D vector through the tangent plane instead
//! would lift entities by the Earth's curvature: about 8 m at 10 km from the
//! origin and 70 m at 30 km.

use crate::geoid::Geoid;
use bgws_core::{Enu, EnuFrame, Geodetic};

/// An entity placed on the Earth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Latitude and longitude, degrees; height above the WGS-84 ellipsoid, m.
    pub geodetic: Geodetic,
    /// Geocentric position, m.
    pub ecef: [f64; 3],
    /// The local east, north and up unit vectors at the entity, in ECEF.
    pub axes: [[f64; 3]; 3],
}

/// East, north and up unit vectors in ECEF at a latitude and longitude.
pub fn local_axes(lat_deg: f64, lon_deg: f64) -> [[f64; 3]; 3] {
    let (sl, cl) = lat_deg.to_radians().sin_cos();
    let (so, co) = lon_deg.to_radians().sin_cos();
    [[-so, co, 0.0], [-sl * co, -sl * so, cl], [cl * co, cl * so, sl]]
}

/// Place a sim position (`u` above mean sea level).
pub fn place(frame: &EnuFrame, geoid: &Geoid, pos: Enu) -> Placed {
    let flat = frame.enu_to_geodetic(Enu::new(pos.e, pos.n, 0.0));
    let h = geoid.to_ellipsoid(flat.lat, flat.lon, pos.u);
    let geodetic = Geodetic { lat: flat.lat, lon: flat.lon, h };
    let e = geodetic.to_ecef();
    Placed { geodetic, ecef: [e.x, e.y, e.z], axes: local_axes(flat.lat, flat.lon) }
}

/// The inverse of [`place`]: a geocentric position to the sim's east and
/// north (the point of the origin's tangent plane over that latitude and
/// longitude) and height above mean sea level.
pub fn unplace(frame: &EnuFrame, geoid: &Geoid, ecef: [f64; 3]) -> Enu {
    let g = bgws_core::Ecef { x: ecef[0], y: ecef[1], z: ecef[2] }.to_geodetic();
    // Start from the ellipsoid point's projection, then correct east and
    // north until the tangent-plane point lies over the same latitude and
    // longitude (a few iterations converge to well under a millimetre).
    let mut p = frame.geodetic_to_enu(Geodetic { lat: g.lat, lon: g.lon, h: 0.0 });
    p.u = 0.0;
    for _ in 0..5 {
        let at = frame.enu_to_geodetic(p);
        let m_per_deg_lat = 111_132.0;
        let m_per_deg_lon = 111_320.0 * g.lat.to_radians().cos().max(1e-6);
        p.n += (g.lat - at.lat) * m_per_deg_lat;
        p.e += (g.lon - at.lon) * m_per_deg_lon;
    }
    Enu::new(p.e, p.n, geoid.to_msl(g.lat, g.lon, g.h))
}

/// A local east/north/up vector at the entity, in ECEF.
pub fn to_ecef_vec(axes: &[[f64; 3]; 3], v: Enu) -> [f64; 3] {
    let c = |j: usize| axes[0][j] * v.e + axes[1][j] * v.n + axes[2][j] * v.u;
    [c(0), c(1), c(2)]
}

/// The DIS Euler angles (ψ, θ, φ), radians: the body's yaw, pitch and roll
/// about the geocentric axes (IEEE 1278.1 §1.3.2 and annex on coordinate
/// systems), from its heading (clockwise from north), pitch (nose up) and
/// roll (right side down) in the local level frame.
pub fn dis_orientation(axes: &[[f64; 3]; 3], heading: f64, pitch: f64, roll: f64) -> [f64; 3] {
    let (sh, ch) = heading.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    // Body axes in local ENU: x forward, y right, z down (right-handed).
    let fwd = Enu::new(sh * cp, ch * cp, sp);
    let right0 = Enu::new(ch, -sh, 0.0);
    let up0 = Enu::new(-sh * sp, -ch * sp, cp);
    let y = right0.scale(cr).add(up0.scale(-sr));
    let x = to_ecef_vec(axes, fwd);
    let y = to_ecef_vec(axes, y);
    let z = cross(x, y);
    let psi = x[1].atan2(x[0]);
    let theta = (-x[2]).clamp(-1.0, 1.0).asin();
    let phi = y[2].atan2(z[2]);
    [psi, theta, phi]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Back from DIS angles to the body's forward vector in ECEF (for tests and
/// for dead reckoning's orientation check).
pub fn forward_of(o: [f64; 3]) -> [f64; 3] {
    let (sps, cps) = o[0].sin_cos();
    let (sth, cth) = o[1].sin_cos();
    [cps * cth, sps * cth, -sth]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn level_and_facing_north_at_the_equator_and_prime_meridian() {
        // North there is +Z, so the body points along the Earth's axis:
        // ψ = 0, θ = −90°, the known DIS case.
        let axes = local_axes(0.0, 0.0);
        let o = dis_orientation(&axes, 0.0, 0.0, 0.0);
        assert!(close(o[1], -std::f64::consts::FRAC_PI_2, 1e-9), "θ {}", o[1].to_degrees());
        // Facing east there: along +Y, level: ψ = 90°, θ = 0, φ = 0 (down is −X... the body's z points down, toward the centre).
        let o = dis_orientation(&axes, std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        assert!(close(o[0], std::f64::consts::FRAC_PI_2, 1e-9) && close(o[1], 0.0, 1e-9), "{o:?}");
    }

    #[test]
    fn angles_give_back_the_forward_direction() {
        let axes = local_axes(54.2, 18.6);
        for (h, p, r) in [(0.3, 0.05, -0.02), (2.5, -0.1, 0.08), (-1.2, 0.0, 0.0)] {
            let o = dis_orientation(&axes, h, p, r);
            let (sh, ch) = f64::sin_cos(h);
            let (sp, cp) = f64::sin_cos(p);
            let want = to_ecef_vec(&axes, Enu::new(sh * cp, ch * cp, sp));
            let got = forward_of(o);
            for k in 0..3 {
                assert!(close(got[k], want[k], 1e-9), "{h} {p} {r}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn a_placed_entity_round_trips_to_its_latitude_longitude_and_height() {
        let frame = EnuFrame::new(Geodetic { lat: 54.2, lon: 18.6, h: 0.0 });
        let g = Geoid::egm96();
        for pos in [Enu::new(0.0, 0.0, 112.0), Enu::new(10_000.0, -7_000.0, 40.0), Enu::new(-30_000.0, 25_000.0, 0.0)] {
            let p = place(&frame, g, pos);
            let back = bgws_core::Ecef { x: p.ecef[0], y: p.ecef[1], z: p.ecef[2] }.to_geodetic();
            assert!(close(back.lat, p.geodetic.lat, 1e-9) && close(back.lon, p.geodetic.lon, 1e-9));
            // Height above the ellipsoid is the sim's height plus N, within 0.1 m,
            // with no curvature lift however far from the origin.
            let n = g.undulation(p.geodetic.lat, p.geodetic.lon);
            assert!(close(back.h, pos.u + n, 0.1), "at {pos:?}: h {:.2}, MSL {} + N {n:.2}", back.h, pos.u);
            // And back to the sim's position, to 1 cm.
            let back = unplace(&frame, g, p.ecef);
            assert!(close(back.e, pos.e, 0.01) && close(back.n, pos.n, 0.01) && close(back.u, pos.u, 0.01), "{pos:?} came back as {back:?}");
        }
    }
}
