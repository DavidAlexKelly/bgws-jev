//! Atmospheric transmission (§7.3).
//!
//! - **Koschmieder:** meteorological visibility to extinction,
//!   `σ = −ln(ε)/V` (3.912/V for the 2% contrast threshold).
//! - **Beer–Lambert** over a path, `τ = e^(−σR)`, per optical band (visible,
//!   NIR, MWIR, LWIR). The visible extinction is split into rain (large
//!   drops, the same in every band), fog and haze; fog and haze scale to
//!   each band by ratios, and a molecular term (water vapour, scaling with
//!   absolute humidity) is added **[default coefficients]**.
//! - **Radar rain attenuation:** `γ = k·R^α` dB/km with the ITU-R
//!   P.838-3 coefficients, `k` interpolated log-log and `α` lin-log in
//!   frequency, for horizontal, vertical or circular polarisation; plus
//!   cloud/fog liquid-water attenuation in the ITU-R P.840 form `γ = K_l·M`.

use crate::weather::WeatherCell;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpticalBand {
    Visible,
    Nir,
    Mwir,
    Lwir,
}

impl OpticalBand {
    pub const ALL: [OpticalBand; 4] = [OpticalBand::Visible, OpticalBand::Nir, OpticalBand::Mwir, OpticalBand::Lwir];
    pub fn key(self) -> &'static str {
        match self {
            OpticalBand::Visible => "visible",
            OpticalBand::Nir => "nir",
            OpticalBand::Mwir => "mwir",
            OpticalBand::Lwir => "lwir",
        }
    }
}

/// Band coefficients, from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AtmosParams {
    pub contrast_threshold: f64,
    pub haze_ratio: [f64; 4],
    pub fog_ratio: [f64; 4],
    /// Molecular extinction per km at 10 g/m³ absolute humidity.
    pub molecular_per_km: [f64; 4],
    pub rain_a_per_km: f64,
    pub rain_b: f64,
    /// Cloud/fog liquid-water attenuation K_l = c·f^e dB/km per g/m³ (f GHz).
    pub kl_coeff: f64,
    pub kl_exp: f64,
}

impl AtmosParams {
    pub fn from_figures(f: &Figures) -> Self {
        let per = |n: &str| OpticalBand::ALL.map(|b| f.get(&format!("env.optics.{}.{n}", b.key())));
        Self {
            contrast_threshold: f.get("env.optics.contrast_threshold"),
            haze_ratio: per("haze_ratio"),
            fog_ratio: per("fog_ratio"),
            molecular_per_km: per("molecular_per_km"),
            rain_a_per_km: f.get("env.optics.rain_a_per_km"),
            rain_b: f.get("env.optics.rain_b"),
            kl_coeff: f.get("env.radar.kl_coeff"),
            kl_exp: f.get("env.radar.kl_exp"),
        }
    }
}

/// Koschmieder extinction coefficient per metre from visibility (metres).
pub fn extinction_from_visibility(visibility_m: f64, contrast_threshold: f64) -> f64 {
    -contrast_threshold.ln() / visibility_m.max(1.0)
}

/// Absolute humidity, g/m³, from temperature (°C) and relative humidity (0–1).
pub fn absolute_humidity(t_c: f64, rh: f64) -> f64 {
    let es = 6.112 * (17.62 * t_c / (243.12 + t_c)).exp();
    216.7 * rh.clamp(0.0, 1.0) * es / (273.15 + t_c)
}

/// The parts of an optical extinction coefficient, per km.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extinction {
    pub haze_per_km: f64,
    pub fog_per_km: f64,
    pub rain_per_km: f64,
    pub molecular_per_km: f64,
    pub total_per_km: f64,
}

/// Extinction in a band for the weather in a cell.
pub fn band_extinction(band: OpticalBand, w: &WeatherCell, p: &AtmosParams) -> Extinction {
    let b = band as usize;
    let total_vis = extinction_from_visibility(w.visibility_m, p.contrast_threshold) * 1000.0;
    let rain = if w.precip_mm_h > 0.0 { (p.rain_a_per_km * w.precip_mm_h.powf(p.rain_b)).min(total_vis) } else { 0.0 };
    let particulate = (total_vis - rain).max(0.0);
    let fog_share = w.fog.clamp(0.0, 1.0);
    let haze = particulate * (1.0 - fog_share) * p.haze_ratio[b];
    let fog = particulate * fog_share * p.fog_ratio[b];
    let molecular = p.molecular_per_km[b] * absolute_humidity(w.temperature_c, w.humidity) / 10.0;
    Extinction { haze_per_km: haze, fog_per_km: fog, rain_per_km: rain, molecular_per_km: molecular, total_per_km: haze + fog + rain + molecular }
}

/// Beer–Lambert transmission over `range_m` in a band.
pub fn transmission(band: OpticalBand, range_m: f64, w: &WeatherCell, p: &AtmosParams) -> f64 {
    (-band_extinction(band, w, p).total_per_km * range_m.max(0.0) / 1000.0).exp()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarisation {
    Horizontal,
    Vertical,
    Circular,
}

/// ITU-R P.838-3 coefficients: (f GHz, kH, αH, kV, αV).
const P838: [(f64, f64, f64, f64, f64); 20] = [
    (1.0, 0.000_025_9, 0.9691, 0.000_030_8, 0.8592),
    (2.0, 0.000_084_7, 1.0664, 0.000_099_8, 0.9490),
    (4.0, 0.000_107_1, 1.6009, 0.000_246_1, 1.2476),
    (6.0, 0.000_705_6, 1.5900, 0.000_487_8, 1.5728),
    (7.0, 0.001_915, 1.4810, 0.001_425, 1.4745),
    (8.0, 0.004_115, 1.3905, 0.003_450, 1.3797),
    (10.0, 0.012_17, 1.2571, 0.011_29, 1.2156),
    (12.0, 0.023_86, 1.1825, 0.024_55, 1.1216),
    (15.0, 0.044_81, 1.1233, 0.050_08, 1.0440),
    (20.0, 0.091_64, 1.0568, 0.096_11, 0.9847),
    (25.0, 0.157_1, 0.9991, 0.153_3, 0.9491),
    (30.0, 0.240_3, 0.9485, 0.229_1, 0.9129),
    (35.0, 0.337_4, 0.9047, 0.322_4, 0.8761),
    (40.0, 0.443_1, 0.8673, 0.427_4, 0.8421),
    (50.0, 0.660_0, 0.8084, 0.647_2, 0.7871),
    (60.0, 0.860_6, 0.7656, 0.851_5, 0.7486),
    (70.0, 1.031_5, 0.7345, 1.025_3, 0.7215),
    (80.0, 1.170_4, 0.7115, 1.166_8, 0.7021),
    (90.0, 1.280_7, 0.6944, 1.279_5, 0.6876),
    (100.0, 1.367_1, 0.6815, 1.368_0, 0.6765),
];

/// (k, α) for a frequency (GHz) and polarisation, interpolated per P.838.
pub fn p838_coefficients(f_ghz: f64, pol: Polarisation) -> (f64, f64) {
    let f = f_ghz.clamp(P838[0].0, P838[P838.len() - 1].0);
    let i = P838.partition_point(|r| r.0 < f).clamp(1, P838.len() - 1);
    let (a, b) = (P838[i - 1], P838[i]);
    let t = (f.ln() - a.0.ln()) / (b.0.ln() - a.0.ln());
    let logk = |ka: f64, kb: f64| (ka.ln() + (kb.ln() - ka.ln()) * t).exp();
    let lin = |xa: f64, xb: f64| xa + (xb - xa) * t;
    let (kh, ah) = (logk(a.1, b.1), lin(a.2, b.2));
    let (kv, av) = (logk(a.3, b.3), lin(a.4, b.4));
    match pol {
        Polarisation::Horizontal => (kh, ah),
        Polarisation::Vertical => (kv, av),
        Polarisation::Circular => {
            let k = (kh + kv) / 2.0;
            (k, (kh * ah + kv * av) / (2.0 * k))
        }
    }
}

/// Rain specific attenuation γ = k·R^α, dB/km.
pub fn rain_attenuation_db_per_km(f_ghz: f64, rain_mm_h: f64, pol: Polarisation) -> f64 {
    if rain_mm_h <= 0.0 {
        return 0.0;
    }
    let (k, a) = p838_coefficients(f_ghz, pol);
    k * rain_mm_h.powf(a)
}

/// Cloud/fog attenuation, dB/km, for liquid water content `lwc` g/m³ (P.840 form).
pub fn liquid_water_attenuation_db_per_km(f_ghz: f64, lwc_g_m3: f64, p: &AtmosParams) -> f64 {
    p.kl_coeff * f_ghz.max(0.0).powf(p.kl_exp) * lwc_g_m3.max(0.0)
}

/// Fog liquid water content from fog visibility (Eldridge-type fit), g/m³.
pub fn fog_lwc_from_visibility(visibility_m: f64) -> f64 {
    (0.024 / (visibility_m.max(10.0) / 1000.0)).powf(1.54)
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("env.optics.contrast_threshold", 0.02, "fraction", 0.01, 0.05, "default", Medium,
            "Koschmieder contrast threshold (0.02 gives σ = 3.912/V)"),
        figure!("env.optics.rain_a_per_km", 0.25, "1/km", 0.1, 1.0, "default", Default, "optical rain extinction σ = a·R^b: a"),
        figure!("env.optics.rain_b", 0.67, "1", 0.4, 1.0, "default", Default, "optical rain extinction: b"),
        figure!("env.radar.kl_coeff", 0.0006, "dB/km/(g/m3)", 0.0002, 0.002, "default", Default, "P.840-form K_l = c·f^e: c (f in GHz, ~10 °C)"),
        figure!("env.radar.kl_exp", 1.95, "1", 1.5, 2.2, "default", Default, "P.840-form K_l exponent"),
    ];
    let table = [
        (OpticalBand::Visible, 1.0, 1.0, 0.01),
        (OpticalBand::Nir, 0.7, 1.0, 0.02),
        (OpticalBand::Mwir, 0.35, 0.9, 0.08),
        (OpticalBand::Lwir, 0.2, 0.6, 0.12),
    ];
    for (b, haze, fog, mol) in table {
        let k = b.key();
        v.push(figure!(format!("env.optics.{k}.haze_ratio"), haze, "1", 0.0, 1.5, "default", Default, "haze extinction relative to visible"));
        v.push(figure!(format!("env.optics.{k}.fog_ratio"), fog, "1", 0.0, 1.5, "default", Default, "fog extinction relative to visible"));
        v.push(figure!(format!("env.optics.{k}.molecular_per_km"), mol, "1/km", 0.0, 1.0, "default", Default,
            "molecular (water vapour) extinction at 10 g/m3"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::WeatherPreset;
    use approx::assert_relative_eq;

    fn p() -> AtmosParams {
        AtmosParams::from_figures(&Figures::from_list(figures()).unwrap())
    }

    #[test]
    fn koschmieder() {
        assert_relative_eq!(extinction_from_visibility(10_000.0, 0.02), 3.912e-4, epsilon = 1e-6);
        // At the visibility range the contrast is exactly the threshold.
        let w = WeatherCell { visibility_m: 5_000.0, humidity: 0.0, ..WeatherPreset::Clear.cell() };
        assert_relative_eq!(transmission(OpticalBand::Visible, 5_000.0, &w, &p()), 0.02, epsilon = 1e-9);
    }

    #[test]
    fn ir_sees_through_haze_better_than_fog() {
        let p = p();
        let haze = WeatherCell { visibility_m: 3_000.0, fog: 0.0, ..WeatherPreset::Clear.cell() };
        let fog = WeatherCell { visibility_m: 300.0, fog: 1.0, ..WeatherPreset::Fog.cell() };
        let r = 2_000.0;
        assert!(transmission(OpticalBand::Lwir, r, &haze, &p) > 5.0 * transmission(OpticalBand::Visible, r, &haze, &p));
        assert!(transmission(OpticalBand::Lwir, r, &fog, &p) < 1e-3, "thick fog defeats LWIR at 2 km too");
        let rain = WeatherPreset::HeavyRain.cell();
        let e = band_extinction(OpticalBand::Lwir, &rain, &p);
        assert!(e.rain_per_km > 1.0);
    }

    #[test]
    fn p838_values() {
        let p = p();
        // 10 GHz, 25 mm/h, horizontal: 0.01217·25^1.2571 ≈ 0.70 dB/km.
        assert_relative_eq!(rain_attenuation_db_per_km(10.0, 25.0, Polarisation::Horizontal), 0.7004, epsilon = 0.01);
        // Tabulated point reproduced exactly; 30 GHz 50 mm/h ≈ 9.9 dB/km.
        assert_relative_eq!(rain_attenuation_db_per_km(30.0, 50.0, Polarisation::Horizontal), 0.2403 * 50f64.powf(0.9485), epsilon = 1e-9);
        // Monotone in frequency and rain; negligible at L band.
        assert!(rain_attenuation_db_per_km(1.5, 25.0, Polarisation::Circular) < 0.01);
        assert!(rain_attenuation_db_per_km(35.0, 10.0, Polarisation::Vertical) > rain_attenuation_db_per_km(15.0, 10.0, Polarisation::Vertical));
        // Interpolated between rows sits between them.
        let mid = rain_attenuation_db_per_km(11.0, 10.0, Polarisation::Horizontal);
        assert!(mid > rain_attenuation_db_per_km(10.0, 10.0, Polarisation::Horizontal) && mid < rain_attenuation_db_per_km(12.0, 10.0, Polarisation::Horizontal));
        // Fog at 35 GHz with 0.3 g/m³: a fraction of a dB per km.
        let g = liquid_water_attenuation_db_per_km(35.0, 0.3, &p);
        assert!(g > 0.05 && g < 0.5, "{g}");
        assert!(fog_lwc_from_visibility(100.0) > fog_lwc_from_visibility(500.0));
    }
}
