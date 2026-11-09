//! ACQUIRE / TTP target acquisition for eyes, optics, image intensifiers and
//! EO/thermal imagers (§8.2) **[R8, P22]**, and IRST point-target detection.
//!
//! Resolved cycles across the target:
//! N = (critical dimension / range) [mrad] × sensor resolution [cycles/mrad]
//!     × atmospheric MTF × contrast factor.
//! The contrast factor is 0 at the threshold contrast and 1 at the reference
//! contrast, logarithmic in between (the shape of a minimum resolvable
//! contrast/temperature curve). Visual contrast and thermal ΔT both arrive
//! through the path's transmission.
//!
//! P∞ = (N/N₅₀)^E / (1 + (N/N₅₀)^E), E = 2.7 + 0.7·(N/N₅₀), with separate
//! N₅₀ for detect, recognise and identify. A TTP sensor's cycles are compared
//! with N₅₀ × the TTP ratio (~2.7). The search rate is λ = −ln(1 − P∞)/τ with
//! τ from the field of view, cueing and the observer (see `observer`).

use crate::math::norm_cdf;
use crate::math::norm_inv;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// The acquisition stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Detect,
    Recognise,
    Identify,
}

/// Which cycle metric the sensor's resolution is quoted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    /// Classic Johnson cycles (ACQUIRE).
    Johnson,
    /// Targeting Task Performance (modern sampled imagers).
    Ttp,
}

/// The target's contrast against its background, as the sensor sees it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Contrast {
    /// Inherent luminance contrast |Lt − Lb|/Lb, and the path's contrast transmittance.
    Visual { inherent: f64, transmission: f64 },
    /// Inherent ΔT (K) against the background, and the band's path transmission.
    Thermal { delta_t_k: f64, transmission: f64 },
}

/// How the observer searches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Search {
    /// Field of view being searched, degrees.
    pub fov_deg: f64,
    /// Told where to look (a cue shrinks the search area).
    pub cued: bool,
    /// Observer multiplier on τ (≥ 1 slower) from `observer::tau_multiplier`.
    pub observer_tau_mult: f64,
}

/// One acquisition question.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcquireInput {
    /// √(width × height) of the presented target, m.
    pub critical_dim_m: f64,
    pub range_m: f64,
    /// Sensor resolution, cycles per milliradian, in `metric`.
    pub cycles_per_mrad: f64,
    pub metric: Metric,
    /// Atmospheric MTF at the relevant spatial frequency (turbulence, aerosol blur), 0..1.
    pub atmos_mtf: f64,
    pub contrast: Contrast,
    pub stage: Stage,
    pub search: Search,
}

/// The result, with every intermediate value (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcquireResult {
    pub stage: Stage,
    /// Apparent contrast (visual) or apparent ΔT in K (thermal).
    pub apparent_contrast: f64,
    pub contrast_factor: f64,
    pub cycles: f64,
    pub n50: f64,
    pub ratio: f64,
    pub exponent: f64,
    pub p_inf: f64,
    pub tau_s: f64,
    pub rate_per_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcquireParams {
    /// Johnson N₅₀: detect, recognise, identify.
    pub n50: [f64; 3],
    pub e_base: f64,
    pub e_slope: f64,
    pub ttp_ratio: f64,
    pub visual_c_threshold: f64,
    pub visual_c_ref: f64,
    pub thermal_dt_threshold_k: f64,
    pub thermal_dt_ref_k: f64,
    pub tau_base_s: f64,
    pub fov_ref_deg: f64,
    pub fov_exponent: f64,
    pub cue_factor: f64,
    pub p_inf_max: f64,
    pub irst_pfa: f64,
    pub irst_frame_s: f64,
}

impl AcquireParams {
    pub fn from_figures(f: &Figures) -> Self {
        AcquireParams {
            n50: [f.get("sensing.acquire.n50.detect"), f.get("sensing.acquire.n50.recognise"), f.get("sensing.acquire.n50.identify")],
            e_base: f.get("sensing.acquire.e_base"),
            e_slope: f.get("sensing.acquire.e_slope"),
            ttp_ratio: f.get("sensing.acquire.ttp_ratio"),
            visual_c_threshold: f.get("sensing.acquire.visual_contrast_threshold"),
            visual_c_ref: f.get("sensing.acquire.visual_contrast_ref"),
            thermal_dt_threshold_k: f.get("sensing.acquire.thermal_dt_threshold_k"),
            thermal_dt_ref_k: f.get("sensing.acquire.thermal_dt_ref_k"),
            tau_base_s: f.get("sensing.acquire.tau_base_s"),
            fov_ref_deg: f.get("sensing.acquire.fov_ref_deg"),
            fov_exponent: f.get("sensing.acquire.fov_exponent"),
            cue_factor: f.get("sensing.acquire.cue_factor"),
            p_inf_max: f.get("sensing.acquire.p_inf_max"),
            irst_pfa: f.get("sensing.irst.pfa"),
            irst_frame_s: f.get("sensing.irst.frame_s"),
        }
    }

    /// N₅₀ for a stage in the sensor's metric.
    pub fn n50(&self, stage: Stage, metric: Metric) -> f64 {
        let j = self.n50[stage as usize];
        match metric {
            Metric::Johnson => j,
            Metric::Ttp => j * self.ttp_ratio,
        }
    }
}

/// The ACQUIRE target transfer probability function P∞(N/N₅₀).
pub fn p_infinity(cycles: f64, n50: f64, p: &AcquireParams) -> (f64, f64) {
    if cycles <= 0.0 || n50 <= 0.0 {
        return (0.0, p.e_base);
    }
    let r = cycles / n50;
    let e = p.e_base + p.e_slope * r;
    let x = r.powf(e);
    ((x / (1.0 + x)).min(p.p_inf_max), e)
}

/// Contrast factor from an apparent contrast (or ΔT) and its threshold/reference.
pub fn contrast_factor(apparent: f64, threshold: f64, reference: f64) -> f64 {
    if apparent <= threshold {
        return 0.0;
    }
    ((apparent / threshold).ln() / (reference / threshold).ln()).clamp(0.0, 1.0)
}

/// The search time constant τ, seconds.
pub fn search_tau(s: &Search, p: &AcquireParams) -> f64 {
    let fov = (s.fov_deg.max(0.1) / p.fov_ref_deg).powf(p.fov_exponent);
    let cue = if s.cued { p.cue_factor } else { 1.0 };
    p.tau_base_s * fov * cue * s.observer_tau_mult.max(0.1)
}

/// Resolved cycles, P∞ and the acquisition hazard for one observer–target pair.
pub fn acquire(i: &AcquireInput, p: &AcquireParams) -> AcquireResult {
    let (apparent, cf) = match i.contrast {
        Contrast::Visual { inherent, transmission } => {
            let a = inherent.abs() * transmission.clamp(0.0, 1.0);
            (a, contrast_factor(a, p.visual_c_threshold, p.visual_c_ref))
        }
        Contrast::Thermal { delta_t_k, transmission } => {
            let a = delta_t_k.abs() * transmission.clamp(0.0, 1.0);
            (a, contrast_factor(a, p.thermal_dt_threshold_k, p.thermal_dt_ref_k))
        }
    };
    let subtense_mrad = i.critical_dim_m.max(0.0) / i.range_m.max(1.0) * 1000.0;
    let cycles = subtense_mrad * i.cycles_per_mrad.max(0.0) * i.atmos_mtf.clamp(0.0, 1.0) * cf;
    let n50 = p.n50(i.stage, i.metric);
    let (p_inf, e) = p_infinity(cycles, n50, p);
    let tau = search_tau(&i.search, p);
    let rate = if p_inf <= 0.0 { 0.0 } else { -(-p_inf).ln_1p() / tau };
    AcquireResult { stage: i.stage, apparent_contrast: apparent, contrast_factor: cf, cycles, n50, ratio: cycles / n50, exponent: e, p_inf, tau_s: tau, rate_per_s: rate }
}

/// IRST point-target detection: an unresolved hot spot. The target fills a
/// fraction of one pixel's footprint; the signal is ΔT × transmission × fill
/// against the detector's NETD; Pd = Φ(SNR − Φ⁻¹(1 − Pfa)) per frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IrstResult {
    pub fill_fraction: f64,
    pub snr: f64,
    pub threshold: f64,
    pub pd_frame: f64,
    pub rate_per_s: f64,
}

pub fn irst_detect(delta_t_k: f64, target_area_m2: f64, range_m: f64, transmission: f64, ifov_mrad: f64, netd_k: f64, p: &AcquireParams) -> IrstResult {
    let footprint = (ifov_mrad * 1e-3 * range_m.max(1.0)).powi(2);
    let fill = (target_area_m2.max(0.0) / footprint).min(1.0);
    let snr = delta_t_k.abs() * transmission.clamp(0.0, 1.0) * fill / netd_k.max(1e-6);
    let threshold = norm_inv(1.0 - p.irst_pfa);
    let pd = norm_cdf(snr - threshold);
    IrstResult { fill_fraction: fill, snr, threshold, pd_frame: pd, rate_per_s: super::rate_from_look(pd, p.irst_frame_s) }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.acquire.n50.detect", 0.75, "cycles", 0.5, 1.5, "R8", Medium, "Johnson N50 for detection"),
        figure!("sensing.acquire.n50.recognise", 3.0, "cycles", 2.0, 4.5, "R8", Medium, "Johnson N50 for recognition"),
        figure!("sensing.acquire.n50.identify", 6.0, "cycles", 4.5, 9.0, "R8", Medium, "Johnson N50 for identification"),
        figure!("sensing.acquire.e_base", 2.7, "1", 2.7, 2.7, "P22", High, "ACQUIRE exponent E = 2.7 + 0.7 N/N50"),
        figure!("sensing.acquire.e_slope", 0.7, "1", 0.7, 0.7, "P22", High, "ACQUIRE exponent slope"),
        figure!("sensing.acquire.ttp_ratio", 2.7, "1", 2.0, 3.5, "R8", Medium, "TTP V50 / Johnson N50"),
        figure!("sensing.acquire.visual_contrast_threshold", 0.02, "fraction", 0.005, 0.1, "default", Default, "threshold apparent contrast"),
        figure!("sensing.acquire.visual_contrast_ref", 0.5, "fraction", 0.1, 1.0, "default", Default, "contrast at which full resolution is reached"),
        figure!("sensing.acquire.thermal_dt_threshold_k", 0.05, "K", 0.01, 0.3, "default", Default, "MRTD floor (≈ NETD) of a modern imager"),
        figure!("sensing.acquire.thermal_dt_ref_k", 4.0, "K", 0.5, 20.0, "default", Default, "ΔT at which full resolution is reached"),
        figure!("sensing.acquire.tau_base_s", 8.0, "s", 1.0, 60.0, "default", Default, "search time constant at the reference FoV"),
        figure!("sensing.acquire.fov_ref_deg", 10.0, "deg", 1.0, 60.0, "default", Default, "reference search field"),
        figure!("sensing.acquire.fov_exponent", 0.5, "1", 0.0, 2.0, "default", Default, "τ grows as (FoV/ref)^k"),
        figure!("sensing.acquire.cue_factor", 0.25, "fraction", 0.05, 1.0, "default", Default, "τ multiplier when cued"),
        figure!("sensing.acquire.p_inf_max", 0.999, "fraction", 0.9, 0.999999, "default", Default, "cap on P∞ so λ stays finite"),
        figure!("sensing.irst.pfa", 1e-6, "fraction", 1e-9, 1e-2, "default", Default, "IRST per-frame per-pixel false alarm probability"),
        figure!("sensing.irst.frame_s", 1.0, "s", 0.01, 10.0, "default", Default, "IRST revisit of a given direction"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn tank_at(range: f64, contrast: Contrast, dim: f64, stage: Stage) -> AcquireInput {
        AcquireInput {
            critical_dim_m: dim,
            range_m: range,
            cycles_per_mrad: 8.0,
            metric: Metric::Johnson,
            atmos_mtf: 0.8,
            contrast,
            stage,
            search: Search { fov_deg: 10.0, cued: false, observer_tau_mult: 1.0 },
        }
    }

    #[test]
    fn p_inf_at_n50_is_half() {
        let p = AcquireParams::from_figures(&figs());
        let (pi, e) = p_infinity(3.0, 3.0, &p);
        assert_relative_eq!(pi, 0.5, epsilon = 1e-12);
        assert_relative_eq!(e, 3.4);
        let (p2, _) = p_infinity(6.0, 3.0, &p);
        assert_relative_eq!(p2, 2f64.powf(4.1) / (1.0 + 2f64.powf(4.1)), epsilon = 1e-12);
        assert_relative_eq!(p.n50(Stage::Identify, Metric::Ttp), 6.0 * 2.7);
    }

    #[test]
    fn stages_and_range() {
        let p = AcquireParams::from_figures(&figs());
        let c = Contrast::Visual { inherent: 0.4, transmission: 0.8 };
        let d = acquire(&tank_at(2000.0, c, 2.9, Stage::Detect), &p);
        let r = acquire(&tank_at(2000.0, c, 2.9, Stage::Recognise), &p);
        let i = acquire(&tank_at(2000.0, c, 2.9, Stage::Identify), &p);
        assert!(d.p_inf > r.p_inf && r.p_inf > i.p_inf);
        let far = acquire(&tank_at(6000.0, c, 2.9, Stage::Identify), &p);
        assert!(far.p_inf < i.p_inf);
        // λ = −ln(1 − P∞)/τ.
        assert_relative_eq!(d.rate_per_s, -(1.0 - d.p_inf).ln() / d.tau_s, epsilon = 1e-12);
        // Cueing searches a small area: faster.
        let mut cued = tank_at(2000.0, c, 2.9, Stage::Detect);
        cued.search.cued = true;
        assert!(acquire(&cued, &p).rate_per_s > d.rate_per_s);
    }

    #[test]
    fn thermal_crossover_vs_just_fired() {
        // A cold hull-down tank at thermal crossover is nearly invisible; one that has just fired is not.
        let p = AcquireParams::from_figures(&figs());
        let hull_down_dim = (1.0f64 * 3.6).sqrt();
        let crossover = acquire(&tank_at(2000.0, Contrast::Thermal { delta_t_k: 0.08, transmission: 0.7 }, hull_down_dim, Stage::Detect), &p);
        let fired = acquire(&tank_at(2000.0, Contrast::Thermal { delta_t_k: 12.0, transmission: 0.7 }, hull_down_dim, Stage::Detect), &p);
        assert!(crossover.p_inf < 0.05, "{crossover:?}");
        assert!(fired.p_inf > 0.95, "{fired:?}");
    }

    #[test]
    fn irst_point_target() {
        let p = AcquireParams::from_figures(&figs());
        let near = irst_detect(30.0, 10.0, 20_000.0, 0.6, 0.1, 0.03, &p);
        let far = irst_detect(30.0, 10.0, 300_000.0, 0.1, 0.1, 0.03, &p);
        assert!(near.pd_frame > 0.99 && far.pd_frame < near.pd_frame);
        assert_relative_eq!(near.threshold, 4.753, epsilon = 1e-3);
    }

    proptest! {
        #[test]
        fn p_inf_monotone(n in 0.0f64..50.0, dn in 0.0f64..5.0) {
            let p = AcquireParams::from_figures(&figs());
            let a = p_infinity(n, 3.0, &p).0;
            let b = p_infinity(n + dn, 3.0, &p).0;
            prop_assert!((0.0..=1.0).contains(&a));
            prop_assert!(b >= a - 1e-12);
        }
    }
}
