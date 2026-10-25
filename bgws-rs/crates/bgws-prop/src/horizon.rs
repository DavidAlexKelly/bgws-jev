//! Radar and radio horizons (§8.4): the 4/3-Earth effective radius for
//! smooth-Earth reach, and the terrain horizon elevation angle along an
//! azimuth (what blocks a satellite at low elevation, §8.4 satellite links).

use bgws_core::geo::{horizon_distance, mutual_horizon, EARTH_RADIUS};
use bgws_env::Terrain;
use serde::{Deserialize, Serialize};

/// Smooth-Earth reach between two heights with the radio/radar factor `k`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HorizonResult {
    pub sensor_horizon_m: f64,
    pub target_horizon_m: f64,
    /// Maximum range at which the two see each other over a smooth Earth.
    pub max_range_m: f64,
}

/// Radar (or radio) horizon for a sensor at `h_sensor` and a target at `h_target` metres.
pub fn radar_horizon(h_sensor: f64, h_target: f64, k: f64) -> HorizonResult {
    HorizonResult {
        sensor_horizon_m: horizon_distance(h_sensor, k),
        target_horizon_m: horizon_distance(h_target, k),
        max_range_m: mutual_horizon(h_sensor, h_target, k),
    }
}

/// The lowest altitude visible at ground range `d` from a sensor at
/// `h_sensor` over a smooth Earth (the masked "radar shadow" ceiling).
pub fn shadow_ceiling(h_sensor: f64, d: f64, k: f64) -> f64 {
    let dh = horizon_distance(h_sensor, k);
    if d <= dh {
        0.0
    } else {
        (d - dh).powi(2) / (2.0 * k * EARTH_RADIUS)
    }
}

/// Terrain horizon elevation angle (radians, positive above the
/// horizontal) looking along `azimuth` (radians from north) from a point at
/// `agl` metres above ground, out to `max_range` metres, with factor `k`.
pub fn terrain_horizon_elevation(t: &Terrain, e: f64, n: f64, agl: f64, azimuth: f64, max_range: f64, k: f64) -> f64 {
    let z0 = t.height(e, n) + agl;
    let step = t.spec.cell * 0.5;
    let steps = (max_range / step).ceil() as usize;
    let (se, ce) = azimuth.sin_cos();
    let mut best = f64::NEG_INFINITY;
    for s in 1..=steps {
        let d = s as f64 * step;
        let (x, y) = (e + d * se, n + d * ce);
        if !t.spec.contains(x, y) {
            break;
        }
        let h = t.height(x, y) - d * d / (2.0 * k * EARTH_RADIUS);
        best = best.max(((h - z0) / d).atan());
    }
    if best.is_finite() {
        best
    } else {
        -(2.0 * agl.max(0.0) / (k * EARTH_RADIUS)).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::geo::K_RADIO;
    use bgws_core::Geodetic;
    use bgws_env::{GridSpec, Raster};

    #[test]
    fn horizons() {
        // A 10 m radar sees about 13 km to the surface (4.12·√h km).
        let r = radar_horizon(10.0, 0.0, K_RADIO);
        assert!((r.sensor_horizon_m - 13_030.0).abs() < 50.0);
        // Ship's radar at 20 m vs a sea-skimmer at 5 m: ≈ 18.4 + 9.2 = 27.6 km.
        let s = radar_horizon(20.0, 5.0, K_RADIO);
        assert!((s.max_range_m - 27_640.0).abs() < 150.0, "{}", s.max_range_m);
        // An AEW aircraft at 9 km pushes the horizon past 390 km.
        assert!(radar_horizon(9_000.0, 5.0, K_RADIO).max_range_m > 390_000.0);
        // Below the shadow ceiling a target is hidden; it rises with range.
        assert_eq!(shadow_ceiling(20.0, 10_000.0, K_RADIO), 0.0);
        assert!(shadow_ceiling(20.0, 40_000.0, K_RADIO) > 5.0);
    }

    #[test]
    fn ridge_raises_the_horizon() {
        let spec = GridSpec::new(201, 3, 50.0, 0.0, 0.0);
        let mut t = bgws_env::Terrain::flat(Geodetic::default(), spec, 0.0);
        t.elevation = Raster::from_fn(spec, |i, _| if i == 100 { 500.0 } else { 0.0 });
        let east = terrain_horizon_elevation(&t, 0.0, 50.0, 2.0, std::f64::consts::FRAC_PI_2, 9_000.0, K_RADIO);
        assert!((east.to_degrees() - (498.0f64 / 5000.0).atan().to_degrees()).abs() < 0.05, "{}", east.to_degrees());
    }
}
