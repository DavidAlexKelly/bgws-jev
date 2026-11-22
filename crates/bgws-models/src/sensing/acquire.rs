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
//! with N₅₀ × the TTP ratio (~2.7).
//!
//! Search follows the NVESD search model (Howe 1993, *IR/EO Systems
//! Handbook* vol. 4 ch. 2, §2.6.3, pp. 110–114; notes in
//! `docs/sources/notes/irhb-1993-v4-eo-systems-design-analysis-testing.md`): P(t) = P∞·[1 − exp(−t/τ)]
//! (eq. 2.33). Only a share P∞ of observers ever find the target; those that
//! do find it at the rate 1/τ. τ for one field of view is 3.4/P∞ s for
//! P∞ < 0.9 and 6.8·N₅₀/N s above (eq. 2.38); searching a sector of n
//! fields of view takes n times as long (eq. 2.40). A cue and the observer's
//! state scale τ (see `observer`). For search the detection N₅₀ follows the
//! clutter round the target (Table 2.5): half the medium value in low
//! clutter (a target in a field, on a road), twice it in high.

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
    /// The clutter round the target, which sets the detection N₅₀ in search.
    pub clutter: Clutter,
}

/// Clutter round a target in search (Howe 1993, Table 2.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Clutter {
    /// A target in a field or on a road.
    Low,
    /// A tank in a desert of tank-size bushes.
    #[default]
    Medium,
    /// A vehicle among similar vehicles.
    High,
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
    /// The share of observers that ever find the target (searching).
    pub p_inf: f64,
    /// Mean time to find it, for one that will, s.
    pub tau_s: f64,
    /// 1/τ: the rate at which an observer that will find the target finds
    /// it. Draw whether it ever will (P∞) once, then this rate (eq. 2.33).
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
    /// τ_FOV·P∞ for P∞ < 0.9 (3.4 s), and τ_FOV·N/N₅₀ above (6.8 s).
    pub tfov_k: f64,
    pub tfov_k_high: f64,
    /// Detection N₅₀ in low and high clutter, relative to medium.
    pub clutter_low: f64,
    pub clutter_high: f64,
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
            tfov_k: f.get("sensing.acquire.tfov_k_s"),
            tfov_k_high: f.get("sensing.acquire.tfov_k_high_s"),
            clutter_low: f.get("sensing.acquire.clutter_low"),
            clutter_high: f.get("sensing.acquire.clutter_high"),
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

/// How much longer than one field of view a search takes: the sector in
/// fields of view (eq. 2.40), a cue, and the observer's state.
pub fn search_scale(s: &Search, p: &AcquireParams) -> f64 {
    let fov = (s.fov_deg.max(0.1) / p.fov_ref_deg).powf(p.fov_exponent).max(1.0);
    let cue = if s.cued { p.cue_factor } else { 1.0 };
    fov * cue * s.observer_tau_mult.max(0.1)
}

/// The mean time to find a target in one field of view, for an observer
/// that will, s (Howe 1993, eq. 2.38).
pub fn tau_fov(p_inf: f64, cycles: f64, n50: f64, p: &AcquireParams) -> f64 {
    if p_inf <= 0.0 {
        return f64::INFINITY;
    }
    if p_inf < 0.9 || cycles <= 0.0 {
        p.tfov_k / p_inf
    } else {
        p.tfov_k_high * n50 / cycles
    }
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
    let mut n50 = p.n50(i.stage, i.metric);
    if i.stage == Stage::Detect {
        n50 *= match i.search.clutter {
            Clutter::Low => p.clutter_low,
            Clutter::Medium => 1.0,
            Clutter::High => p.clutter_high,
        };
    }
    let (p_inf, e) = p_infinity(cycles, n50, p);
    let tau = tau_fov(p_inf, cycles, n50, p) * search_scale(&i.search, p);
    let rate = if p_inf <= 0.0 || !tau.is_finite() { 0.0 } else { 1.0 / tau };
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
        figure!("sensing.acquire.tfov_k_s", 3.4, "s", 2.0, 6.0, "Howe 1993 eq. 2.35/2.38", Medium, "mean time to find a target in one field of view is this / P∞ (P∞ < 0.9, low to medium clutter; NVESD search model, validated mainly with thermal imagers)"),
        figure!("sensing.acquire.tfov_k_high_s", 6.8, "s", 4.0, 12.0, "Howe 1993 eq. 2.36/2.38", Medium, "for P∞ ≥ 0.9 the time in one field of view is this × N50/N"),
        figure!("sensing.acquire.clutter_low", 0.5, "×", 0.3, 1.0, "Howe 1993 Table 2.5", Medium, "search detection N50 in low clutter (a target in a field, on a road), relative to medium (0.5 vs 1.0 cycles)"),
        figure!("sensing.acquire.clutter_high", 2.0, "×", 1.0, 3.0, "Howe 1993 Table 2.5", Medium, "search detection N50 in high clutter (a vehicle among similar vehicles), relative to medium (2.0 vs 1.0 cycles)"),
        figure!("sensing.acquire.fov_ref_deg", 10.0, "deg", 3.0, 20.0, "default", Default, "the field of view a sector is searched with (a gunner's sight's wide field is about 10–15°); a sector of n of these takes n times as long"),
        figure!("sensing.acquire.fov_exponent", 1.0, "1", 0.5, 1.0, "Howe 1993 eq. 2.40", Medium, "search time grows as (sector / field of view)^k: n fields of view take n times as long (k = 1 for a band of fixed height)"),
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
            search: Search { fov_deg: 10.0, cued: false, observer_tau_mult: 1.0, clutter: Clutter::Medium },
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
        // The rate for an observer that will find it: 1/τ.
        assert_relative_eq!(d.rate_per_s, 1.0 / d.tau_s, epsilon = 1e-12);
        // Cueing searches a small area: faster.
        let mut cued = tank_at(2000.0, c, 2.9, Stage::Detect);
        cued.search.cued = true;
        assert!(acquire(&cued, &p).rate_per_s > d.rate_per_s);
    }

    /// Howe's worked example (p. 114): N = 2.56 cycles in medium clutter
    /// (N50 = 1) gives P∞ = 0.99 and τ_FOV = 2.65 s; N = 1.34 gives P∞ = 0.74
    /// and τ_FOV = 4.52 s (1-D cycles; the 2-D model scales N50 by 0.75).
    /// A sector five fields of view wide takes five times as long.
    #[test]
    fn howe_worked_example() {
        let p = AcquireParams::from_figures(&figs());
        let (good, _) = p_infinity(2.56, 1.0, &p);
        assert!((good - 0.99).abs() < 0.01, "{good}");
        assert!((tau_fov(good, 2.56, 1.0, &p) - 2.65).abs() < 0.02);
        let (poor, _) = p_infinity(1.34, 1.0, &p);
        assert!((poor - 0.74).abs() < 0.01, "{poor}");
        assert!((tau_fov(poor, 1.34, 1.0, &p) - 4.52).abs() < 0.15, "{}", tau_fov(poor, 1.34, 1.0, &p));
        let one = Search { fov_deg: p.fov_ref_deg, cued: false, observer_tau_mult: 1.0, clutter: Clutter::Medium };
        let five = Search { fov_deg: 5.0 * p.fov_ref_deg, ..one };
        assert_relative_eq!(search_scale(&five, &p), 5.0 * search_scale(&one, &p), epsilon = 1e-12);
    }

    /// The same target is found more often in low clutter than in high.
    #[test]
    fn clutter_moves_detection() {
        let p = AcquireParams::from_figures(&figs());
        let c = Contrast::Visual { inherent: 0.4, transmission: 0.8 };
        let at = |clutter| {
            let mut i = tank_at(12_000.0, c, 2.9, Stage::Detect);
            i.search.clutter = clutter;
            acquire(&i, &p)
        };
        let (low, mid, high) = (at(Clutter::Low), at(Clutter::Medium), at(Clutter::High));
        assert!(low.p_inf > mid.p_inf && mid.p_inf > high.p_inf, "{} {} {}", low.p_inf, mid.p_inf, high.p_inf);
        assert!(low.rate_per_s >= mid.rate_per_s && mid.rate_per_s >= high.rate_per_s);
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
