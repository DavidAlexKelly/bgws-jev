//! HF propagation (§8.4): ground wave by frequency and ground constants, and
//! NVIS skywave for about 2–10 MHz over about 35–450 km with no skip zone,
//! with a time-of-day ionospheric availability **[P5, R2]**.
//!
//! - **Ground wave:** Norton's flat-Earth surface-wave attenuation for
//!   vertical polarisation, `A(p) ≈ (2 + 0.3p)/(2 + p + 0.6p²) −
//!   √(p/2)·e^(−5p/8)·sin b`, with the numerical distance
//!   `p = (πd/λx)·cos b`, `x = σ/(ωε₀)`, `tan b = (ε_r + 1)/x`; field
//!   doubled over the ground plane, so `L = FSPL − 20·log10(2A)`. Ground
//!   constants by type follow ITU-R P.527. Earth curvature beyond the
//!   flat-Earth range is neglected (pessimistic only past ~100 km at low HF).
//! - **NVIS:** the usable ceiling (optimum working frequency at vertical
//!   incidence) moves between a day and a night value with the sun's
//!   elevation — ~10 MHz by day, ~6–7 MHz at night or solar minimum [R2] —
//!   scaled by sec θ for the path's obliquity off a reflecting layer at
//!   `h_F`; the lowest usable frequency rises by day with D-layer
//!   absorption. A link is available inside [LUF, OWF] and the [P5] range
//!   bracket; loss is free space over the up-and-down slant path plus
//!   D-layer absorption `A·cos(χ)^0.75·sec θ / f²`.

use bgws_core::units::SPEED_OF_LIGHT;
use bgws_core::Figures;
use serde::{Deserialize, Serialize};

pub use bgws_gpu::field::fspl_db;

const EPS0: f64 = 8.854_187_812_8e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundType {
    SeaWater,
    FreshWater,
    WetGround,
    MediumDry,
    VeryDry,
}

impl GroundType {
    /// (conductivity S/m, relative permittivity), ITU-R P.527.
    pub fn constants(self) -> (f64, f64) {
        match self {
            GroundType::SeaWater => (5.0, 70.0),
            GroundType::FreshWater => (0.003, 80.0),
            GroundType::WetGround => (0.01, 30.0),
            GroundType::MediumDry => (0.001, 15.0),
            GroundType::VeryDry => (0.0001, 3.0),
        }
    }

    /// Engine soil and land cover → ground type.
    pub fn from_env(soil: bgws_env::SoilType, wetness: f64, lc: bgws_env::LandCover) -> GroundType {
        use bgws_env::{LandCover, SoilType};
        match (lc, soil) {
            (LandCover::Water, _) => GroundType::FreshWater,
            (LandCover::Wetland, _) | (_, SoilType::Peat) => GroundType::WetGround,
            (_, SoilType::Rock) | (_, SoilType::Sand) if wetness < 0.3 => GroundType::VeryDry,
            _ if wetness > 0.6 => GroundType::WetGround,
            _ => GroundType::MediumDry,
        }
    }
}

/// Ground-wave loss with its parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundWave {
    pub fspl_db: f64,
    pub numerical_distance: f64,
    pub attenuation_factor: f64,
    pub total_db: f64,
}

/// Ground-wave basic transmission loss at `f_hz` over `d_m` of `ground`.
pub fn ground_wave(f_hz: f64, d_m: f64, ground: GroundType) -> GroundWave {
    let (sigma, er) = ground.constants();
    let lambda = SPEED_OF_LIGHT / f_hz;
    let x = sigma / (std::f64::consts::TAU * f_hz * EPS0);
    let b = ((er + 1.0) / x).atan();
    let p = std::f64::consts::PI * d_m / (lambda * x) * b.cos();
    let a = ((2.0 + 0.3 * p) / (2.0 + p + 0.6 * p * p) - (p / 2.0).sqrt() * (-5.0 * p / 8.0).exp() * b.sin()).clamp(1e-12, 1.0);
    let fspl = fspl_db(d_m, f_hz);
    GroundWave { fspl_db: fspl, numerical_distance: p, attenuation_factor: a, total_db: fspl - 20.0 * (2.0 * a).log10() }
}

/// NVIS settings, from figures.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NvisParams {
    pub day_ceiling_mhz: f64,
    pub night_ceiling_mhz: f64,
    pub day_luf_mhz: f64,
    pub night_luf_mhz: f64,
    pub min_range_m: f64,
    pub max_range_m: f64,
    pub layer_height_m: f64,
    pub absorption_db_mhz2: f64,
    /// Ceiling multiplier at solar minimum (1 at solar maximum).
    pub solar_min_factor: f64,
}

impl NvisParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            day_ceiling_mhz: f.get("prop.nvis.day_ceiling_mhz"),
            night_ceiling_mhz: f.get("prop.nvis.night_ceiling_mhz"),
            day_luf_mhz: f.get("prop.nvis.day_luf_mhz"),
            night_luf_mhz: f.get("prop.nvis.night_luf_mhz"),
            min_range_m: f.get("prop.nvis.min_range_km") * 1000.0,
            max_range_m: f.get("prop.nvis.max_range_km") * 1000.0,
            layer_height_m: f.get("prop.nvis.layer_height_km") * 1000.0,
            absorption_db_mhz2: f.get("prop.nvis.absorption_db_mhz2"),
            solar_min_factor: f.get("prop.nvis.solar_min_factor"),
        }
    }
}

/// Why an NVIS link does or does not work.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NvisResult {
    pub available: bool,
    /// Usable ceiling (OWF) for this path, MHz.
    pub owf_mhz: f64,
    pub luf_mhz: f64,
    pub in_range: bool,
    pub slant_path_m: f64,
    pub absorption_db: f64,
    pub total_loss_db: f64,
}

/// NVIS link at `f_hz` over ground range `d_m`, with the sun at
/// `sun_elev_deg` at the path midpoint and solar activity `solar` in 0
/// (minimum) … 1 (maximum).
pub fn nvis(f_hz: f64, d_m: f64, sun_elev_deg: f64, solar: f64, p: &NvisParams) -> NvisResult {
    let f_mhz = f_hz / 1e6;
    // Day weight: 0 at night, 1 with the sun 20° up; smooth through twilight.
    let day = ((sun_elev_deg + 6.0) / 26.0).clamp(0.0, 1.0);
    let solar_factor = p.solar_min_factor + (1.0 - p.solar_min_factor) * solar.clamp(0.0, 1.0);
    let vertical_ceiling = (p.night_ceiling_mhz + (p.day_ceiling_mhz - p.night_ceiling_mhz) * day) * solar_factor;
    let half = d_m / 2.0;
    let sec_theta = (half * half + p.layer_height_m * p.layer_height_m).sqrt() / p.layer_height_m;
    let owf = vertical_ceiling * sec_theta;
    let luf = p.night_luf_mhz + (p.day_luf_mhz - p.night_luf_mhz) * day;
    let slant = 2.0 * (half * half + p.layer_height_m * p.layer_height_m).sqrt();
    let cos_chi = sun_elev_deg.to_radians().sin().max(0.0);
    let absorption = p.absorption_db_mhz2 * cos_chi.powf(0.75) * sec_theta / (f_mhz * f_mhz);
    let in_range = d_m >= p.min_range_m && d_m <= p.max_range_m;
    NvisResult {
        available: in_range && f_mhz <= owf && f_mhz >= luf,
        owf_mhz: owf,
        luf_mhz: luf,
        in_range,
        slant_path_m: slant,
        absorption_db: absorption,
        total_loss_db: fspl_db(slant, f_hz) + absorption,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np() -> NvisParams {
        NvisParams::from_figures(&crate::default_figures())
    }

    #[test]
    fn ground_wave_sea_beats_dry_land() {
        let sea = ground_wave(3e6, 100_000.0, GroundType::SeaWater);
        let dry = ground_wave(3e6, 100_000.0, GroundType::MediumDry);
        assert!(dry.total_db - sea.total_db > 20.0, "{sea:?} {dry:?}");
        // Over sea at short range the ground wave is ≈ free space − 6 dB (field doubled).
        let near = ground_wave(2e6, 1_000.0, GroundType::SeaWater);
        assert!((near.total_db - (near.fspl_db - 6.02)).abs() < 0.5, "{near:?}");
        // Higher HF dies faster over land.
        assert!(ground_wave(10e6, 30_000.0, GroundType::MediumDry).total_db > ground_wave(3e6, 30_000.0, GroundType::MediumDry).total_db + 10.0);
    }

    #[test]
    fn nvis_window_by_day_and_night() {
        let p = np();
        // Noon, 5 MHz at 200 km: works, no skip zone at 40 km either.
        assert!(nvis(5e6, 200_000.0, 50.0, 0.5, &p).available);
        assert!(nvis(5e6, 40_000.0, 50.0, 0.5, &p).available);
        // Outside the 35–450 km bracket.
        assert!(!nvis(5e6, 20_000.0, 50.0, 0.5, &p).in_range);
        assert!(!nvis(5e6, 600_000.0, 50.0, 0.5, &p).available);
        // 9 MHz: by day yes, at night the ceiling (6–7 MHz) drops below it.
        assert!(nvis(9e6, 150_000.0, 50.0, 1.0, &p).available);
        let night = nvis(9e6, 150_000.0, -30.0, 1.0, &p);
        assert!(!night.available && night.owf_mhz < 7.5, "{night:?}");
        // 2.5 MHz: absorbed by day (below the LUF), fine at night.
        assert!(!nvis(2.5e6, 150_000.0, 50.0, 0.5, &p).available);
        assert!(nvis(2.5e6, 150_000.0, -30.0, 0.5, &p).available);
        // Solar minimum lowers the ceiling.
        assert!(nvis(5e6, 150_000.0, 50.0, 0.0, &p).owf_mhz < nvis(5e6, 150_000.0, 50.0, 1.0, &p).owf_mhz);
    }
}
