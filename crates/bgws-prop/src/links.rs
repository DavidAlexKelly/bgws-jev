//! Satellite and microwave/mmWave links (§8.4).
//!
//! - **Satellite:** free space over the slant range + rain fade by band
//!   (ITU-R P.838 specific attenuation over the slant path below the rain
//!   height, with the ITU-R P.618 horizontal reduction factor
//!   `1/(1 + L_G/L_0)`, `L_0 = 35·e^(−0.015R)`) + a clear-air gas term +
//!   blockage when the satellite is below the terrain mask or the minimum
//!   elevation.
//! - **Microwave/mmWave terrestrial:** line of sight (with first-Fresnel-zone
//!   obstruction loss through J(ν)) + rain over the path with the same
//!   P.838/P.530-style reduction + gaseous absorption, which peaks at the
//!   60 GHz oxygen line **[default]**.

use crate::radio::{fresnel_v, fspl_db, knife_edge_j};
use bgws_core::geo::EARTH_RADIUS;
use bgws_core::units::SPEED_OF_LIGHT;
use bgws_core::Figures;
use bgws_env::{rain_attenuation_db_per_km, Polarisation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SatBand {
    Uhf,
    L,
    S,
    C,
    X,
    Ku,
    Ka,
    Q,
}

impl SatBand {
    /// Representative downlink frequency, Hz.
    pub fn freq_hz(self) -> f64 {
        match self {
            SatBand::Uhf => 0.3e9,
            SatBand::L => 1.6e9,
            SatBand::S => 2.5e9,
            SatBand::C => 4.0e9,
            SatBand::X => 7.5e9,
            SatBand::Ku => 12.0e9,
            SatBand::Ka => 20.0e9,
            SatBand::Q => 44.0e9,
        }
    }
}

/// Slant range to a satellite at `alt_m` seen at elevation `elev_rad` (spherical Earth).
pub fn slant_range(alt_m: f64, elev_rad: f64) -> f64 {
    let r = EARTH_RADIUS;
    let (s, c) = elev_rad.sin_cos();
    ((r + alt_m).powi(2) - (r * c).powi(2)).sqrt() - r * s
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkParams {
    pub rain_height_m: f64,
    pub min_elevation_deg: f64,
    pub gas_zenith_db: f64,
    pub oxygen_peak_db_km: f64,
    pub background_gas_db_km: f64,
    pub p618_l0_km: f64,
}

impl LinkParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            rain_height_m: f.get("prop.sat.rain_height_km") * 1000.0,
            min_elevation_deg: f.get("prop.sat.min_elevation_deg"),
            gas_zenith_db: f.get("prop.sat.gas_zenith_db"),
            oxygen_peak_db_km: f.get("prop.mw.oxygen_peak_db_km"),
            background_gas_db_km: f.get("prop.mw.background_gas_db_km"),
            p618_l0_km: f.get("prop.sat.p618_l0_km"),
        }
    }
}

/// A satellite link's loss budget.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SatLink {
    pub elevation_deg: f64,
    pub slant_range_m: f64,
    pub fspl_db: f64,
    pub rain_db: f64,
    pub gas_db: f64,
    /// Below the terrain mask or the minimum elevation.
    pub blocked: bool,
    pub total_db: f64,
}

/// Satellite link at `f_hz` to a satellite at `alt_m` seen at `elev_deg`,
/// with rain `rain_mm_h` and a terrain mask elevation `mask_deg` on that azimuth.
pub fn sat_link(f_hz: f64, alt_m: f64, elev_deg: f64, rain_mm_h: f64, mask_deg: f64, p: &LinkParams) -> SatLink {
    let el = elev_deg.to_radians();
    let range = slant_range(alt_m, el);
    let fspl = fspl_db(range, f_hz);
    let sin_el = el.sin().max(0.087); // ~5°: the flat-layer slant model is not valid lower
    let ls_km = p.rain_height_m / sin_el / 1000.0;
    let lg_km = ls_km * el.cos();
    let r = 1.0 / (1.0 + lg_km / (p.p618_l0_km * (-0.015 * rain_mm_h).exp()));
    let rain = rain_attenuation_db_per_km(f_hz / 1e9, rain_mm_h, Polarisation::Circular) * ls_km * r;
    let gas = p.gas_zenith_db * (f_hz / 10e9).max(0.1).powf(1.5).min(10.0) / sin_el;
    let blocked = elev_deg < p.min_elevation_deg || elev_deg < mask_deg;
    SatLink { elevation_deg: elev_deg, slant_range_m: range, fspl_db: fspl, rain_db: rain, gas_db: gas, blocked, total_db: fspl + rain + gas }
}

/// Specific gaseous absorption, dB/km: background plus the 60 GHz oxygen line.
pub fn gas_absorption_db_km(f_hz: f64, p: &LinkParams) -> f64 {
    let f = f_hz / 1e9;
    p.background_gas_db_km * (f / 10.0).max(0.1) + p.oxygen_peak_db_km * (-((f - 60.0) / 4.0).powi(2)).exp()
}

/// A terrestrial microwave/mmWave link's loss budget.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MicrowaveLink {
    pub fspl_db: f64,
    /// Fresnel-zone obstruction loss.
    pub obstruction_db: f64,
    pub rain_db: f64,
    pub gas_db: f64,
    pub total_db: f64,
}

/// Microwave link over `d_m` with the worst obstruction `clearance_m` (the
/// ray's height above the highest obstacle, negative if blocked) at
/// distance `obstacle_at_m` from the transmitter.
pub fn microwave_link(f_hz: f64, d_m: f64, clearance_m: f64, obstacle_at_m: f64, rain_mm_h: f64, p: &LinkParams) -> MicrowaveLink {
    let lambda = SPEED_OF_LIGHT / f_hz;
    let fspl = fspl_db(d_m, f_hz);
    let v = fresnel_v(-clearance_m, obstacle_at_m, (d_m - obstacle_at_m).max(1.0), lambda);
    let obstruction = knife_edge_j(v);
    let d_km = d_m / 1000.0;
    // P.530-style path reduction for terrestrial rain cells.
    let r = 1.0 / (1.0 + d_km / (p.p618_l0_km * (-0.015 * rain_mm_h).exp()));
    let rain = rain_attenuation_db_per_km(f_hz / 1e9, rain_mm_h, Polarisation::Vertical) * d_km * r;
    let gas = gas_absorption_db_km(f_hz, p) * d_km;
    MicrowaveLink { fspl_db: fspl, obstruction_db: obstruction, rain_db: rain, gas_db: gas, total_db: fspl + obstruction + rain + gas }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn lp() -> LinkParams {
        LinkParams::from_figures(&crate::default_figures())
    }

    #[test]
    fn geo_link_magnitudes() {
        let p = lp();
        // GEO at zenith: slant = 35 786 km; Ku FSPL ≈ 205 dB.
        assert_relative_eq!(slant_range(35_786e3, std::f64::consts::FRAC_PI_2), 35_786e3, epsilon = 1.0);
        let clear = sat_link(SatBand::Ku.freq_hz(), 35_786e3, 40.0, 0.0, 0.0, &p);
        assert!((clear.fspl_db - 205.6).abs() < 0.6, "{clear:?}");
        // Heavy rain: Ka fades far more than Ku, X barely; L none.
        let ka = sat_link(SatBand::Ka.freq_hz(), 35_786e3, 30.0, 25.0, 0.0, &p);
        let ku = sat_link(SatBand::Ku.freq_hz(), 35_786e3, 30.0, 25.0, 0.0, &p);
        let x = sat_link(SatBand::X.freq_hz(), 35_786e3, 30.0, 25.0, 0.0, &p);
        let l = sat_link(SatBand::L.freq_hz(), 35_786e3, 30.0, 25.0, 0.0, &p);
        assert!(ka.rain_db > 2.0 * ku.rain_db && ku.rain_db > 3.0 && x.rain_db < 2.5 && l.rain_db < 0.05, "{ka:?} {ku:?} {x:?}");
        // Low elevation behind a ridge is blocked; low elevation also lengthens the rain path.
        assert!(sat_link(SatBand::Ku.freq_hz(), 35_786e3, 8.0, 0.0, 12.0, &p).blocked);
        assert!(sat_link(SatBand::Ku.freq_hz(), 35_786e3, 10.0, 25.0, 0.0, &p).rain_db > ku.rain_db);
        // LEO at 550 km: far less free-space loss.
        assert!(sat_link(SatBand::Ku.freq_hz(), 550e3, 40.0, 0.0, 0.0, &p).fspl_db < clear.fspl_db - 25.0);
    }

    #[test]
    fn microwave_rain_and_oxygen() {
        let p = lp();
        let clear = microwave_link(18e9, 10_000.0, 20.0, 5_000.0, 0.0, &p);
        assert!(clear.obstruction_db < 0.5, "{clear:?}");
        let rain = microwave_link(18e9, 10_000.0, 20.0, 5_000.0, 50.0, &p);
        assert!(rain.rain_db > 10.0, "{rain:?}");
        let grazing = microwave_link(18e9, 10_000.0, 0.0, 5_000.0, 0.0, &p);
        assert_relative_eq!(grazing.obstruction_db, 6.03, epsilon = 0.05);
        let o2 = microwave_link(60e9, 1_000.0, 20.0, 500.0, 0.0, &p);
        let e = microwave_link(80e9, 1_000.0, 20.0, 500.0, 0.0, &p);
        assert!(o2.gas_db > 10.0 && e.gas_db < 2.0, "{o2:?} {e:?}");
    }
}
