//! Sonar (§8.2 "Acoustic and sonar"): the passive and active sonar
//! equations, the reverberation-limited active form, ambient noise from
//! shipping and sea state (Wenz/Knudsen shapes) plus own-ship self-noise by
//! speed, and Pd from the signal excess through a normal transition.
//! Transmission loss is an input (the propagation crate computes it).
//!
//! Passive: SE = SL − TL − (NL − DI) − DT.
//! Active (noise-limited): SE = SL − 2TL + TS − (NL − DI) − DT.
//! Active (reverberation-limited): SE = SL − 2TL + TS − RL − DT.
//! All levels in dB re 1 µPa (spectrum levels per Hz where noted).

use super::pd_from_signal_excess;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// Shipping density for distant-shipping noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shipping {
    Light,
    Moderate,
    Heavy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SonarParams {
    pub sigma_se_db: f64,
    pub knudsen_ss0_db: f64,
    pub knudsen_slope_db: f64,
    pub wind_spectral_slope_db_per_decade: f64,
    pub shipping_100hz_db: [f64; 3],
    pub shipping_slope_db_per_decade: f64,
    pub self_noise_ref_db: f64,
    pub self_noise_ref_kn: f64,
    pub self_noise_db_per_kn: f64,
}

impl SonarParams {
    pub fn from_figures(f: &Figures) -> Self {
        SonarParams {
            sigma_se_db: f.get("sensing.sonar.sigma_se_db"),
            knudsen_ss0_db: f.get("sensing.sonar.knudsen_ss0_db"),
            knudsen_slope_db: f.get("sensing.sonar.knudsen_slope_db"),
            wind_spectral_slope_db_per_decade: f.get("sensing.sonar.wind_spectral_slope_db_per_decade"),
            shipping_100hz_db: ["light", "moderate", "heavy"].map(|s| f.get(&format!("sensing.sonar.shipping_100hz_db.{s}"))),
            shipping_slope_db_per_decade: f.get("sensing.sonar.shipping_slope_db_per_decade"),
            self_noise_ref_db: f.get("sensing.sonar.self_noise_ref_db"),
            self_noise_ref_kn: f.get("sensing.sonar.self_noise_ref_kn"),
            self_noise_db_per_kn: f.get("sensing.sonar.self_noise_db_per_kn"),
        }
    }
}

/// Power sum of levels in dB.
pub fn power_sum_db(levels: &[f64]) -> f64 {
    10.0 * levels.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>().max(1e-300).log10()
}

/// The noise picture at one frequency, with its parts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoiseLevel {
    pub shipping_db: f64,
    pub sea_state_db: f64,
    pub self_noise_db: f64,
    /// Power sum, spectrum level dB re 1 µPa²/Hz.
    pub total_db: f64,
}

/// Ambient + self noise spectrum level at `freq_hz`.
pub fn noise_level(freq_hz: f64, shipping: Shipping, sea_state: f64, own_speed_kn: f64, p: &SonarParams) -> NoiseLevel {
    let f = freq_hz.max(1.0);
    // Shipping peaks below ~100 Hz and falls above it.
    let ship = p.shipping_100hz_db[shipping as usize] + if f > 100.0 { p.shipping_slope_db_per_decade * (f / 100.0).log10() } else { 0.0 };
    // Knudsen: level at 1 kHz by sea state, falling ~17 dB/decade.
    let ss = p.knudsen_ss0_db + p.knudsen_slope_db * (1.0 + sea_state.max(0.0)).log10() + p.wind_spectral_slope_db_per_decade * (f / 1000.0).log10();
    let selfn = p.self_noise_ref_db + p.self_noise_db_per_kn * (own_speed_kn - p.self_noise_ref_kn);
    NoiseLevel { shipping_db: ship, sea_state_db: ss, self_noise_db: selfn, total_db: power_sum_db(&[ship, ss, selfn]) }
}

/// A sonar result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SonarResult {
    pub signal_db: f64,
    pub interference_db: f64,
    pub signal_excess_db: f64,
    pub pd: f64,
}

/// Passive: SE = SL − TL − (NL − DI) − DT.
pub fn passive(sl_db: f64, tl_db: f64, nl_db: f64, di_db: f64, dt_db: f64, p: &SonarParams) -> SonarResult {
    let signal = sl_db - tl_db;
    let interference = nl_db - di_db;
    let se = signal - interference - dt_db;
    SonarResult { signal_db: signal, interference_db: interference, signal_excess_db: se, pd: pd_from_signal_excess(se, p.sigma_se_db) }
}

/// Active: SE = SL − 2TL + TS − (NL − DI ⊕ RL) − DT. With `rl_db = None` it
/// is noise-limited; otherwise noise and reverberation are power-summed (the
/// reverberation-limited form when RL dominates).
pub fn active(sl_db: f64, tl_db: f64, ts_db: f64, nl_db: f64, di_db: f64, rl_db: Option<f64>, dt_db: f64, p: &SonarParams) -> SonarResult {
    let signal = sl_db - 2.0 * tl_db + ts_db;
    let noise = nl_db - di_db;
    let interference = match rl_db {
        Some(rl) => power_sum_db(&[noise, rl]),
        None => noise,
    };
    let se = signal - interference - dt_db;
    SonarResult { signal_db: signal, interference_db: interference, signal_excess_db: se, pd: pd_from_signal_excess(se, p.sigma_se_db) }
}

/// Boundary (surface or bottom) reverberation level:
/// RL = SL − 2TL + Sₛ + 10 log A, A = the insonified patch area (m²).
pub fn reverberation_level(sl_db: f64, tl_db: f64, scattering_strength_db: f64, area_m2: f64) -> f64 {
    sl_db - 2.0 * tl_db + scattering_strength_db + 10.0 * area_m2.max(1e-9).log10()
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.sonar.sigma_se_db", 6.0, "dB", 2.0, 10.0, "default", Default, "width of the SE→Pd normal transition"),
        figure!("sensing.sonar.knudsen_ss0_db", 44.5, "dB", 40.0, 50.0, "default", Default, "Knudsen sea-state 0 spectrum level at 1 kHz"),
        figure!("sensing.sonar.knudsen_slope_db", 30.8, "dB", 20.0, 40.0, "default", Default, "Knudsen level rise per log10(1+SS)"),
        figure!("sensing.sonar.wind_spectral_slope_db_per_decade", -17.0, "dB", -20.0, -15.0, "default", Default, "Knudsen spectral slope"),
        figure!("sensing.sonar.shipping_100hz_db.light", 60.0, "dB", 50.0, 70.0, "default", Default, "Wenz shipping noise, light traffic"),
        figure!("sensing.sonar.shipping_100hz_db.moderate", 68.0, "dB", 58.0, 78.0, "default", Default, "Wenz shipping noise, moderate"),
        figure!("sensing.sonar.shipping_100hz_db.heavy", 76.0, "dB", 66.0, 86.0, "default", Default, "Wenz shipping noise, heavy"),
        figure!("sensing.sonar.shipping_slope_db_per_decade", -20.0, "dB", -30.0, -10.0, "default", Default, "shipping noise fall-off above 100 Hz"),
        figure!("sensing.sonar.self_noise_ref_db", 50.0, "dB", 35.0, 70.0, "default", Default, "own-ship self-noise at reference speed"),
        figure!("sensing.sonar.self_noise_ref_kn", 10.0, "kn", 3.0, 20.0, "default", Default, "reference speed for self-noise"),
        figure!("sensing.sonar.self_noise_db_per_kn", 1.5, "dB", 0.5, 4.0, "default", Default, "self-noise rise per knot"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;

    #[test]
    fn sonar_equations() {
        let p = SonarParams::from_figures(&figs());
        let r = passive(140.0, 80.0, 60.0, 15.0, 10.0, &p);
        assert_relative_eq!(r.signal_excess_db, 5.0);
        assert!(r.pd > 0.5);
        let a = active(220.0, 70.0, 10.0, 60.0, 20.0, None, 15.0, &p);
        assert_relative_eq!(a.signal_excess_db, 220.0 - 140.0 + 10.0 - 40.0 - 15.0);
        let rl = reverberation_level(220.0, 70.0, -30.0, 1.0e5);
        let rev = active(220.0, 70.0, 10.0, 60.0, 20.0, Some(rl), 15.0, &p);
        assert!(rev.signal_excess_db < a.signal_excess_db);
        assert_relative_eq!(pd_from_signal_excess(0.0, 6.0), 0.5);
    }

    #[test]
    fn noise_rises_with_sea_state_shipping_and_speed() {
        let p = SonarParams::from_figures(&figs());
        let calm = noise_level(1000.0, Shipping::Light, 0.0, 5.0, &p);
        let rough = noise_level(1000.0, Shipping::Light, 6.0, 5.0, &p);
        assert_relative_eq!(rough.sea_state_db, 44.5 + 30.8 * 7f64.log10(), epsilon = 1e-9);
        assert!(rough.total_db > calm.total_db);
        let busy = noise_level(100.0, Shipping::Heavy, 2.0, 5.0, &p);
        assert!(busy.shipping_db > noise_level(100.0, Shipping::Light, 2.0, 5.0, &p).shipping_db);
        let fast = noise_level(1000.0, Shipping::Light, 2.0, 25.0, &p);
        assert!(fast.self_noise_db > calm.self_noise_db + 20.0);
        assert_relative_eq!(power_sum_db(&[60.0, 60.0]), 63.0103, epsilon = 1e-4);
    }
}
