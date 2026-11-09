//! Acoustic transmission loss (§8.4 → §8.2, §8.6).
//!
//! **In water** (passive and active sonar):
//! - absorption by Thorp's formula (dB/km, f in kHz)
//!   `α = 0.11f²/(1+f²) + 44f²/(4100+f²) + 2.75e−4·f² + 0.003`;
//! - spherical spreading `20·log r` near the source, changing to
//!   cylindrical `10·log r` beyond a transition range in a surface duct
//!   (Urick's `r₀ = √(R·D/8)`, `R = c/g` the ray radius of curvature in the
//!   layer) or in shallow water (`r₀ ≈ H`), with duct leakage or bottom
//!   loss per km; a duct traps only frequencies above its cutoff
//!   (`λ_max ≈ 0.0085·D^1.5`, D and λ in feet — Urick);
//! - a layer shadow: source and receiver on opposite sides of the sonic
//!   layer pay a cross-layer loss beyond the transition range;
//! - convergence zones where the sound-speed profile has depth excess
//!   (the CZ range comes from `bgws_env::SoundSpeedProfile::cz_range`, so
//!   no CZ appears in water too shallow for it): annuli at n·R_cz with a
//!   focusing gain, a direct-path zone near the source and a shadow
//!   (bottom-bounce floor) between **[default gains and widths]**.
//!
//! **In air** (vehicles, drones, guns heard by people and microphones):
//! spherical spreading from 1 m, ISO 9613-1 atmospheric absorption, a
//! ground/excess attenuation term, and a wind term: upwind refraction
//! shadows add loss, downwind adds a little gain **[default]**.

use bgws_core::Figures;
use bgws_env::SoundSpeedProfile;
use serde::{Deserialize, Serialize};

/// Thorp absorption, dB/km, for frequency in Hz.
pub fn thorp_db_per_km(f_hz: f64) -> f64 {
    let f = f_hz / 1000.0;
    let f2 = f * f;
    0.11 * f2 / (1.0 + f2) + 44.0 * f2 / (4100.0 + f2) + 2.75e-4 * f2 + 0.003
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcousticParams {
    pub cz_gain_db: f64,
    pub cz_width_fraction: f64,
    pub direct_path_fraction: f64,
    pub shadow_db: f64,
    pub layer_loss_db: f64,
    pub duct_leakage_db_km: f64,
    pub duct_cutoff_coeff: f64,
    pub shallow_depth_m: f64,
    pub bottom_loss_db_km: f64,
    pub cz_vertex_margin_m_s: f64,
    pub air_excess_db: f64,
    pub air_upwind_db_per_m_s_km: f64,
    pub air_upwind_max_db: f64,
    pub air_downwind_max_db: f64,
}

impl AcousticParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            cz_gain_db: f.get("prop.acoustic.cz_gain_db"),
            cz_width_fraction: f.get("prop.acoustic.cz_width_fraction"),
            direct_path_fraction: f.get("prop.acoustic.direct_path_fraction"),
            shadow_db: f.get("prop.acoustic.shadow_db"),
            layer_loss_db: f.get("prop.acoustic.layer_loss_db"),
            duct_leakage_db_km: f.get("prop.acoustic.duct_leakage_db_km"),
            duct_cutoff_coeff: f.get("prop.acoustic.duct_cutoff_coeff"),
            shallow_depth_m: f.get("prop.acoustic.shallow_depth_m"),
            bottom_loss_db_km: f.get("prop.acoustic.bottom_loss_db_km"),
            cz_vertex_margin_m_s: f.get("env.sea.cz_vertex_margin_m_s"),
            air_excess_db: f.get("prop.acoustic.air_excess_db"),
            air_upwind_db_per_m_s_km: f.get("prop.acoustic.air_upwind_db_per_m_s_km"),
            air_upwind_max_db: f.get("prop.acoustic.air_upwind_max_db"),
            air_downwind_max_db: f.get("prop.acoustic.air_downwind_max_db"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcousticPath {
    Direct,
    SurfaceDuct,
    ShallowWater,
    CrossLayer,
    ConvergenceZone(u32),
    Shadow,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterGeometry {
    pub f_hz: f64,
    pub range_m: f64,
    pub src_depth_m: f64,
    pub rcv_depth_m: f64,
    pub water_depth_m: f64,
}

/// Underwater transmission loss with its parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterTl {
    pub path: AcousticPath,
    pub alpha_db_km: f64,
    pub spreading_db: f64,
    pub absorption_db: f64,
    /// Path-specific term: leakage, layer/shadow loss (positive) or CZ gain (negative).
    pub path_db: f64,
    pub layer_depth_m: f64,
    pub cz_range_m: Option<f64>,
    pub duct_cutoff_hz: Option<f64>,
    pub total_db: f64,
}

/// Transmission loss in water through a sound-speed profile.
pub fn water_tl(g: &WaterGeometry, profile: &SoundSpeedProfile, p: &AcousticParams) -> WaterTl {
    let r = g.range_m.max(1.0);
    let alpha = thorp_db_per_km(g.f_hz);
    let absorption = alpha * r / 1000.0;
    let spherical = 20.0 * r.log10();
    let layer = profile.layer_depth().min(g.water_depth_m);
    let base = |path, spreading: f64, extra: f64, cz: Option<f64>, cut: Option<f64>| WaterTl {
        path,
        alpha_db_km: alpha,
        spreading_db: spreading,
        absorption_db: absorption,
        path_db: extra,
        layer_depth_m: layer,
        cz_range_m: cz,
        duct_cutoff_hz: cut,
        total_db: spreading + absorption + extra,
    };

    if g.water_depth_m <= p.shallow_depth_m {
        let r0 = g.water_depth_m.max(1.0);
        let spreading = if r <= r0 { spherical } else { 20.0 * r0.log10() + 10.0 * (r / r0).log10() };
        let bottom = p.bottom_loss_db_km * (r - r0).max(0.0) / 1000.0;
        return base(AcousticPath::ShallowWater, spreading, bottom, None, None);
    }

    let mut options: Vec<WaterTl> = Vec::new();
    // Surface duct.
    let c0 = profile.c_at(0.0);
    let grad = if layer > 0.0 { (profile.c_at(layer) - c0) / layer } else { 0.0 };
    let duct = (layer >= 10.0 && grad > 0.0).then(|| {
        let r0 = ((c0 / grad) * layer / 8.0).sqrt();
        let d_ft = layer * 3.2808;
        let cutoff = c0 / (p.duct_cutoff_coeff * d_ft.powf(1.5) / 3.2808);
        (r0, cutoff)
    });
    let in_layer = |z: f64| z <= layer;
    if let Some((r0, cutoff)) = duct {
        let both_in = in_layer(g.src_depth_m) && in_layer(g.rcv_depth_m);
        let split = in_layer(g.src_depth_m) != in_layer(g.rcv_depth_m);
        if both_in && g.f_hz >= cutoff {
            let spreading = if r <= r0 { spherical } else { 20.0 * r0.log10() + 10.0 * (r / r0).log10() };
            options.push(base(AcousticPath::SurfaceDuct, spreading, p.duct_leakage_db_km * (r - r0).max(0.0) / 1000.0, None, Some(cutoff)));
        } else if split && r > r0 {
            options.push(base(AcousticPath::CrossLayer, spherical, p.layer_loss_db, None, Some(cutoff)));
        }
    }
    // Deep paths: direct, convergence zones, shadow.
    let cz = profile.cz_range(p.cz_vertex_margin_m_s).filter(|_| profile.max_depth() + 1.0 >= g.water_depth_m.min(profile.max_depth()));
    match cz {
        Some(rcz) => {
            let n = (r / rcz).round().max(1.0);
            let half_width = 0.5 * p.cz_width_fraction * rcz * n;
            if r <= p.direct_path_fraction * rcz {
                options.push(base(AcousticPath::Direct, spherical, 0.0, Some(rcz), None));
            } else if (r - n * rcz).abs() <= half_width {
                options.push(base(AcousticPath::ConvergenceZone(n as u32), spherical, -p.cz_gain_db, Some(rcz), None));
            } else {
                options.push(base(AcousticPath::Shadow, spherical, p.shadow_db, Some(rcz), None));
            }
        }
        None => options.push(base(AcousticPath::Direct, spherical, 0.0, None, None)),
    }
    options.into_iter().min_by(|a, b| a.total_db.total_cmp(&b.total_db)).expect("at least one path")
}

/// ISO 9613-1 atmospheric absorption, dB/m, at `f_hz`, temperature °C,
/// relative humidity (0–1) and pressure in kPa.
pub fn iso9613_alpha_db_per_m(f_hz: f64, temp_c: f64, rh: f64, pressure_kpa: f64) -> f64 {
    let t = temp_c + 273.15;
    let t0 = 293.15;
    let t01 = 273.16;
    let pr = 101.325;
    let pa = pressure_kpa;
    let c = -6.8346 * (t01 / t).powf(1.261) + 4.6151;
    let h = rh.clamp(0.0, 1.0) * 100.0 * 10f64.powf(c) * (pr / pa);
    let fro = (pa / pr) * (24.0 + 4.04e4 * h * (0.02 + h) / (0.391 + h));
    let frn = (pa / pr) * (t / t0).powf(-0.5) * (9.0 + 280.0 * h * (-4.170 * ((t / t0).powf(-1.0 / 3.0) - 1.0)).exp());
    let f2 = f_hz * f_hz;
    8.686
        * f2
        * (1.84e-11 * (pr / pa) * (t / t0).sqrt()
            + (t / t0).powf(-2.5) * (0.012_75 * (-2239.1 / t).exp() / (fro + f2 / fro) + 0.1068 * (-3352.0 / t).exp() / (frn + f2 / frn)))
}

/// Air transmission loss with its parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirTl {
    pub spreading_db: f64,
    pub absorption_db: f64,
    pub excess_db: f64,
    pub wind_db: f64,
    pub total_db: f64,
}

/// Loss in air over `range_m` (re 1 m) with the wind component along the
/// path from source to listener `wind_along_m_s` (positive = downwind).
pub fn air_tl(f_hz: f64, range_m: f64, temp_c: f64, rh: f64, wind_along_m_s: f64, p: &AcousticParams) -> AirTl {
    let r = range_m.max(1.0);
    let spreading = 20.0 * r.log10();
    let absorption = iso9613_alpha_db_per_m(f_hz, temp_c, rh, 101.325) * r;
    let r_km = r / 1000.0;
    let wind = if wind_along_m_s < 0.0 {
        (p.air_upwind_db_per_m_s_km * -wind_along_m_s * r_km).min(p.air_upwind_max_db)
    } else {
        -(p.air_upwind_db_per_m_s_km * 0.3 * wind_along_m_s * r_km).min(p.air_downwind_max_db)
    };
    let excess = if r > 50.0 { p.air_excess_db } else { p.air_excess_db * r / 50.0 };
    AirTl { spreading_db: spreading, absorption_db: absorption, excess_db: excess, wind_db: wind, total_db: spreading + absorption + excess + wind }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use bgws_env::{OceanRegion, Season};

    fn ap() -> AcousticParams {
        AcousticParams::from_figures(&crate::default_figures())
    }

    #[test]
    fn thorp_values() {
        assert_relative_eq!(thorp_db_per_km(1_000.0), 0.0688, epsilon = 0.001);
        assert_relative_eq!(thorp_db_per_km(10_000.0), 1.187, epsilon = 0.01);
        // ~30 dB/km at 100 kHz: high-frequency sonar is short-ranged.
        assert!((thorp_db_per_km(100_000.0) - 36.0).abs() < 4.0, "{}", thorp_db_per_km(100_000.0));
    }

    #[test]
    fn convergence_zone_only_in_deep_water() {
        let p = ap();
        let deep = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Summer, 5000.0);
        let rcz = deep.cz_range(p.cz_vertex_margin_m_s).unwrap();
        let g = WaterGeometry { f_hz: 1_000.0, range_m: rcz, src_depth_m: 50.0, rcv_depth_m: 50.0, water_depth_m: 5000.0 };
        let at_cz = water_tl(&g, &deep, &p);
        assert_eq!(at_cz.path, AcousticPath::ConvergenceZone(1));
        let gap = water_tl(&WaterGeometry { range_m: 0.6 * rcz, ..g }, &deep, &p);
        assert_eq!(gap.path, AcousticPath::Shadow);
        assert!(gap.total_db > at_cz.total_db + 10.0, "the CZ is louder than the shadow nearer in");
        assert_eq!(water_tl(&WaterGeometry { range_m: 2.0 * rcz, ..g }, &deep, &p).path, AcousticPath::ConvergenceZone(2));
        // Same range, 1500 m of water: no CZ.
        let mid = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Summer, 1500.0);
        let r = water_tl(&WaterGeometry { water_depth_m: 1500.0, ..g }, &mid, &p);
        assert!(!matches!(r.path, AcousticPath::ConvergenceZone(_)) && r.cz_range_m.is_none());
        // Shallow shelf: cylindrical spreading.
        let shelf = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Summer, 100.0);
        let s = water_tl(&WaterGeometry { water_depth_m: 100.0, range_m: 10_000.0, ..g }, &shelf, &p);
        assert_eq!(s.path, AcousticPath::ShallowWater);
        assert_relative_eq!(s.spreading_db, 40.0 + 20.0, epsilon = 1e-9);
    }

    #[test]
    fn surface_duct_and_layer_shadow() {
        let p = ap();
        let winter = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Winter, 4000.0);
        let g = WaterGeometry { f_hz: 3_000.0, range_m: 15_000.0, src_depth_m: 30.0, rcv_depth_m: 60.0, water_depth_m: 4000.0 };
        let duct = water_tl(&g, &winter, &p);
        assert_eq!(duct.path, AcousticPath::SurfaceDuct, "{duct:?}");
        assert!(duct.spreading_db < 20.0 * 15_000f64.log10() - 10.0);
        // Below the layer, the same range is in the layer shadow.
        let deep_rx = water_tl(&WaterGeometry { rcv_depth_m: 400.0, ..g }, &winter, &p);
        assert!(deep_rx.total_db > duct.total_db + 15.0, "{deep_rx:?}");
        // Very low frequencies leak out of a thin duct.
        assert!(duct.duct_cutoff_hz.unwrap() > 20.0);
        let low = water_tl(&WaterGeometry { f_hz: 10.0, ..g }, &winter, &p);
        assert_ne!(low.path, AcousticPath::SurfaceDuct);
    }

    #[test]
    fn air_absorption_and_wind() {
        let p = ap();
        // ISO 9613-1: 20 °C, 70 % RH, 1 kHz ≈ 4.98 dB/km; 4 kHz ≈ 22.9 dB/km.
        assert_relative_eq!(iso9613_alpha_db_per_m(1_000.0, 20.0, 0.7, 101.325) * 1000.0, 4.98, epsilon = 0.25);
        assert_relative_eq!(iso9613_alpha_db_per_m(4_000.0, 20.0, 0.7, 101.325) * 1000.0, 22.9, epsilon = 1.2);
        let calm = air_tl(500.0, 2_000.0, 15.0, 0.6, 0.0, &p);
        let up = air_tl(500.0, 2_000.0, 15.0, 0.6, -6.0, &p);
        let down = air_tl(500.0, 2_000.0, 15.0, 0.6, 6.0, &p);
        assert!(up.total_db > calm.total_db + 5.0 && down.total_db < calm.total_db);
        assert_relative_eq!(calm.spreading_db, 66.02, epsilon = 0.01);
    }
}
