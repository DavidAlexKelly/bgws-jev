//! Radar detection (§8.2 "Radar"): the radar range equation, Swerling 0/1/3
//! detection for a given Pfa, RCS by aspect, constant-γ ground clutter by land
//! cover, sea clutter by sea state, the MTI Doppler notch a tangential target
//! hides in **[ref R8: CMO]**, and search-scan vs fire-control track modes.
//! Masking and the radar horizon come from the propagation crate (pass the
//! extra path loss in `losses_db`).
//!
//! SNR = Pt G² λ² σ / ((4π)³ R⁴ k T₀ B F L).
//! Swerling 0 (non-fluctuating): exact single-pulse Pd = Q₁(√(2S), √(−2 ln Pfa))
//! (Marcum), Albersheim's closed form for N pulses. Swerling 1: Pd = Pfa^(1/(1+S)).
//! Swerling 3: Pd = e^(−V/(1+S/2))·(1+2/S)⁻¹·[1 + V/(1+S/2) + 2/S], V = −ln Pfa.

use crate::math::marcum_q1;
use bgws_core::units::{BOLTZMANN, SPEED_OF_LIGHT, STANDARD_TEMPERATURE_K};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Target fluctuation model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Swerling {
    /// Non-fluctuating (Marcum).
    Zero,
    /// Scan-to-scan, many scatterers (Rayleigh).
    One,
    /// Scan-to-scan, one dominant scatterer plus many (chi-square, 4 dof).
    Three,
}

/// A radar's performance record.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarSpec {
    pub peak_power_w: f64,
    /// Antenna gain (transmit = receive), dBi.
    pub gain_dbi: f64,
    pub freq_hz: f64,
    pub bandwidth_hz: f64,
    pub noise_figure_db: f64,
    /// System losses, dB.
    pub losses_db: f64,
    pub pulse_width_s: f64,
    pub prf_hz: f64,
    /// Azimuth beamwidth, rad.
    pub az_beamwidth_rad: f64,
    /// Search: one full rotation, s.
    pub scan_period_s: f64,
    /// Track mode: update interval and pulses per update.
    pub track_update_s: f64,
    pub track_pulses: f64,
    pub mti: bool,
}

/// Search (rotating scan) or fire-control track (dwell on one target).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadarMode {
    Search,
    Track,
}

/// RCS by aspect, m² (catalogue, one band).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RcsProfile {
    pub front_m2: f64,
    pub side_m2: f64,
    pub rear_m2: f64,
    pub top_m2: f64,
}

impl RcsProfile {
    /// RCS seen from `azimuth_off_nose` (0 = nose-on, π/2 = beam, π = tail)
    /// and `elevation` (0 level, π/2 from directly above), interpolated in dB
    /// with sin² weights.
    pub fn at(&self, azimuth_off_nose: f64, elevation: f64) -> f64 {
        let db = |x: f64| 10.0 * x.max(1e-12).log10();
        let a = azimuth_off_nose.abs() % (2.0 * PI);
        let a = if a > PI { 2.0 * PI - a } else { a };
        let w = a.sin().powi(2);
        let horiz = if a <= PI / 2.0 { (1.0 - w) * db(self.front_m2) + w * db(self.side_m2) } else { (1.0 - w) * db(self.rear_m2) + w * db(self.side_m2) };
        let v = elevation.sin().powi(2);
        10f64.powf(((1.0 - v) * horiz + v * db(self.top_m2)) / 10.0)
    }
}

/// Land cover for constant-γ ground clutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LandCover {
    Open,
    Farmland,
    Forest,
    Urban,
    Desert,
    Wetland,
}

/// What the radar is looking at, besides the target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClutterBackground {
    /// Target against the sky (no clutter in the cell).
    None,
    Land { cover: LandCover, grazing_rad: f64 },
    Sea { sea_state: f64, grazing_rad: f64 },
}

/// One target in the beam.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarTarget {
    pub rcs_m2: f64,
    pub range_m: f64,
    /// Closing (or opening) speed along the line of sight, m/s.
    pub radial_speed_mps: f64,
    pub swerling: Swerling,
    /// Extra one-way… two-way path losses from propagation (masking, multipath, rain), dB.
    pub path_loss_db: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarParams {
    pub pfa: f64,
    pub noncoherent_exponent: f64,
    pub mti_improvement_db: f64,
    pub notch_mps: f64,
    /// γ (dB) for open, farmland, forest, urban, desert, wetland.
    pub gamma_db: [f64; 6],
    pub sea_base_db: f64,
    pub sea_slope_db: f64,
}

impl RadarParams {
    pub fn from_figures(f: &Figures) -> Self {
        RadarParams {
            pfa: f.get("sensing.radar.pfa"),
            noncoherent_exponent: f.get("sensing.radar.noncoherent_exponent"),
            mti_improvement_db: f.get("sensing.radar.mti_improvement_db"),
            notch_mps: f.get("sensing.radar.notch_mps"),
            gamma_db: ["open", "farmland", "forest", "urban", "desert", "wetland"].map(|c| f.get(&format!("sensing.clutter.gamma_db.{c}"))),
            sea_base_db: f.get("sensing.clutter.sea_base_db"),
            sea_slope_db: f.get("sensing.clutter.sea_slope_db"),
        }
    }
}

/// Single-pulse SNR (linear) from the radar range equation.
pub fn snr(spec: &RadarSpec, rcs_m2: f64, range_m: f64, extra_loss_db: f64) -> f64 {
    let lambda = SPEED_OF_LIGHT / spec.freq_hz;
    let g = 10f64.powf(spec.gain_dbi / 10.0);
    let f = 10f64.powf(spec.noise_figure_db / 10.0);
    let l = 10f64.powf((spec.losses_db + extra_loss_db) / 10.0);
    spec.peak_power_w * g * g * lambda * lambda * rcs_m2
        / ((4.0 * PI).powi(3) * range_m.max(1.0).powi(4) * BOLTZMANN * STANDARD_TEMPERATURE_K * spec.bandwidth_hz * f * l)
}

/// Albersheim's required single-pulse SNR (dB) for Pd, Pfa and N
/// non-coherently integrated pulses (Swerling 0).
pub fn albersheim_snr_db(pd: f64, pfa: f64, n: f64) -> f64 {
    let a = (0.62 / pfa).ln();
    let b = (pd / (1.0 - pd)).ln();
    -5.0 * n.log10() + (6.2 + 4.54 / (n + 0.44).sqrt()) * (a + 0.12 * a * b + 1.7 * b).log10()
}

/// Pd for a single-pulse (or already integrated) SNR, by Swerling case;
/// Swerling 0 with `n > 1` uses Albersheim inverted for Pd.
pub fn swerling_pd(model: Swerling, snr_linear: f64, pfa: f64, n_pulses: f64) -> f64 {
    let s = snr_linear.max(0.0);
    let v = -pfa.ln();
    let pd = match model {
        Swerling::Zero if n_pulses <= 1.0 => marcum_q1((2.0 * s).sqrt(), (2.0 * v).sqrt()),
        Swerling::Zero => {
            // Invert Albersheim: Z = (SNR_dB + 5 log N)/(6.2 + 4.54/√(N+0.44));
            // 10^Z = A + 0.12AB + 1.7B  →  B; Pd = 1/(1 + e^(−B)).
            if s <= 0.0 {
                return pfa;
            }
            let a = (0.62 / pfa).ln();
            let z = (10.0 * s.log10() + 5.0 * n_pulses.log10()) / (6.2 + 4.54 / (n_pulses + 0.44).sqrt());
            let b = (10f64.powf(z) - a) / (0.12 * a + 1.7);
            1.0 / (1.0 + (-b).exp())
        }
        Swerling::One => pfa.powf(1.0 / (1.0 + s)),
        Swerling::Three => {
            if s <= 0.0 {
                return pfa;
            }
            let h = 1.0 + s / 2.0;
            (-v / h).exp() * (1.0 / (1.0 + 2.0 / s)) * (1.0 + v / h + 2.0 / s)
        }
    };
    pd.clamp(pfa.min(1.0), 1.0)
}

/// Ground clutter reflectivity σ⁰ = γ·sin ψ.
pub fn ground_sigma0(cover: LandCover, grazing_rad: f64, p: &RadarParams) -> f64 {
    10f64.powf(p.gamma_db[cover as usize] / 10.0) * grazing_rad.max(1e-4).sin()
}

/// Sea clutter reflectivity: σ⁰(dB) = base + slope·SS + 10 log sin ψ.
pub fn sea_sigma0(sea_state: f64, grazing_rad: f64, p: &RadarParams) -> f64 {
    10f64.powf((p.sea_base_db + p.sea_slope_db * sea_state.max(0.0)) / 10.0) * grazing_rad.max(1e-4).sin()
}

/// Resolution cell area at low grazing: A = R·θaz·(cτ/2)·sec ψ.
pub fn clutter_area(spec: &RadarSpec, range_m: f64, grazing_rad: f64) -> f64 {
    range_m * spec.az_beamwidth_rad * (SPEED_OF_LIGHT * spec.pulse_width_s / 2.0) / grazing_rad.cos().max(1e-3)
}

/// The detection result, with the chain of numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarDetection {
    pub snr_db: f64,
    /// Clutter RCS in the cell, m² (0 with no clutter).
    pub clutter_rcs_m2: f64,
    pub scr_db: f64,
    pub pulses: f64,
    pub integrated_sir_db: f64,
    pub in_notch: bool,
    pub pd_per_look: f64,
    pub look_s: f64,
    pub rate_per_s: f64,
}

/// Can this radar see this target in this mode, and how fast.
pub fn detect(spec: &RadarSpec, t: &RadarTarget, bg: ClutterBackground, mode: RadarMode, p: &RadarParams) -> RadarDetection {
    let s1 = snr(spec, t.rcs_m2, t.range_m, t.path_loss_db);
    let (pulses, look) = match mode {
        RadarMode::Search => ((spec.az_beamwidth_rad / (2.0 * PI) * spec.scan_period_s * spec.prf_hz).max(1.0), spec.scan_period_s),
        RadarMode::Track => (spec.track_pulses.max(1.0), spec.track_update_s),
    };
    let clutter_rcs = match bg {
        ClutterBackground::None => 0.0,
        ClutterBackground::Land { cover, grazing_rad } => ground_sigma0(cover, grazing_rad, p) * clutter_area(spec, t.range_m, grazing_rad),
        ClutterBackground::Sea { sea_state, grazing_rad } => sea_sigma0(sea_state, grazing_rad, p) * clutter_area(spec, t.range_m, grazing_rad),
    };
    let in_notch = spec.mti && t.radial_speed_mps.abs() < p.notch_mps;
    let mti = if spec.mti { 10f64.powf(p.mti_improvement_db / 10.0) } else { 1.0 };
    // MTI cancels clutter; a target in the notch is cancelled with it.
    let target_rcs_eff = if in_notch { t.rcs_m2 / mti } else { t.rcs_m2 };
    let scr = if clutter_rcs > 0.0 { target_rcs_eff * mti / clutter_rcs } else { f64::INFINITY };
    let snr_eff = s1 * target_rcs_eff / t.rcs_m2.max(1e-12);
    // Noise integrates over the dwell; clutter is correlated and does not.
    let snr_int = snr_eff * pulses.powf(p.noncoherent_exponent);
    let sir = 1.0 / (1.0 / snr_int.max(1e-300) + if scr.is_finite() { 1.0 / scr.max(1e-300) } else { 0.0 });
    let pd = swerling_pd(t.swerling, sir, p.pfa, 1.0);
    RadarDetection {
        snr_db: 10.0 * s1.log10(),
        clutter_rcs_m2: clutter_rcs,
        scr_db: if scr.is_finite() { 10.0 * scr.log10() } else { f64::INFINITY },
        pulses,
        integrated_sir_db: 10.0 * sir.log10(),
        in_notch,
        pd_per_look: pd,
        look_s: look,
        rate_per_s: super::rate_from_look(pd, look),
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.radar.pfa", 1e-6, "fraction", 1e-10, 1e-3, "default", Default, "detection threshold false-alarm probability"),
        figure!("sensing.radar.noncoherent_exponent", 0.8, "1", 0.5, 1.0, "default", Default, "integration gain N^k over a dwell"),
        figure!("sensing.radar.mti_improvement_db", 30.0, "dB", 10.0, 60.0, "default", Default, "MTI clutter improvement factor"),
        figure!("sensing.radar.notch_mps", 3.0, "m/s", 0.5, 30.0, "R8", Low, "MTI notch half-width: tangential targets hide here (CMO)"),
        figure!("sensing.clutter.gamma_db.open", -18.0, "dB", -30.0, -5.0, "default", Default, "constant-γ, grassland"),
        figure!("sensing.clutter.gamma_db.farmland", -15.0, "dB", -30.0, -5.0, "default", Default, "constant-γ, farmland"),
        figure!("sensing.clutter.gamma_db.forest", -10.0, "dB", -20.0, 0.0, "default", Default, "constant-γ, wooded"),
        figure!("sensing.clutter.gamma_db.urban", -5.0, "dB", -15.0, 5.0, "default", Default, "constant-γ, built-up"),
        figure!("sensing.clutter.gamma_db.desert", -22.0, "dB", -35.0, -10.0, "default", Default, "constant-γ, desert"),
        figure!("sensing.clutter.gamma_db.wetland", -20.0, "dB", -35.0, -5.0, "default", Default, "constant-γ, marsh"),
        figure!("sensing.clutter.sea_base_db", -34.0, "dB", -50.0, -20.0, "default", Default, "sea σ0 at sea state 0 (before sin ψ)"),
        figure!("sensing.clutter.sea_slope_db", 4.0, "dB", 1.0, 8.0, "default", Default, "sea σ0 increase per sea state"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn spec() -> RadarSpec {
        RadarSpec {
            peak_power_w: 1.0e6,
            gain_dbi: 40.0,
            freq_hz: 3.0e9,
            bandwidth_hz: 1.0e6,
            noise_figure_db: 3.0,
            losses_db: 6.0,
            pulse_width_s: 1e-6,
            prf_hz: 1000.0,
            az_beamwidth_rad: 1.5f64.to_radians(),
            scan_period_s: 10.0,
            track_update_s: 0.1,
            track_pulses: 20.0,
            mti: true,
        }
    }

    #[test]
    fn range_equation_matches_hand_calculation() {
        // 1 MW, 40 dBi, 10 cm, 1 m², 100 km, F 3 dB, L 6 dB, B 1 MHz.
        let s = snr(&spec(), 1.0, 100_000.0, 0.0);
        let lambda: f64 = 0.099_930_819;
        let num = 1e6 * 1e8 * lambda * lambda;
        let den = (4.0 * PI).powi(3) * 1e20 * 1.380_649e-23 * 290.0 * 1e6 * 10f64.powf(0.9);
        assert_relative_eq!(s, num / den, max_relative = 1e-9);
        // R⁻⁴: half the range, +12.04 dB.
        let s2 = snr(&spec(), 1.0, 50_000.0, 0.0);
        assert_relative_eq!(10.0 * (s2 / s).log10(), 12.041, epsilon = 1e-3);
    }

    #[test]
    fn swerling_reference_points() {
        // Pfa 1e-6, Pd 0.9 needs ~13.2 dB (Swerling 0) and ~21.1 dB (Swerling 1) single pulse.
        let pfa = 1e-6;
        let s0 = swerling_pd(Swerling::Zero, 10f64.powf(1.32), pfa, 1.0);
        assert!((s0 - 0.9).abs() < 0.02, "{s0}");
        let s1 = swerling_pd(Swerling::One, 10f64.powf(2.12), pfa, 1.0);
        assert!((s1 - 0.9).abs() < 0.01, "{s1}");
        // Swerling 3 sits between 0 and 1.
        let s3 = swerling_pd(Swerling::Three, 10f64.powf(1.6), pfa, 1.0);
        let s1b = swerling_pd(Swerling::One, 10f64.powf(1.6), pfa, 1.0);
        let s0b = swerling_pd(Swerling::Zero, 10f64.powf(1.6), pfa, 1.0);
        assert!(s1b < s3 && s3 < s0b, "{s1b} {s3} {s0b}");
        // Albersheim round trip.
        let need = albersheim_snr_db(0.9, pfa, 10.0);
        assert_relative_eq!(swerling_pd(Swerling::Zero, 10f64.powf(need / 10.0), pfa, 10.0), 0.9, epsilon = 1e-9);
        assert!((albersheim_snr_db(0.9, pfa, 1.0) - 13.1).abs() < 0.3);
    }

    #[test]
    fn tangential_target_hides_in_notch() {
        let p = RadarParams::from_figures(&figs());
        let bg = ClutterBackground::Land { cover: LandCover::Farmland, grazing_rad: 0.005 };
        let mk = |v| RadarTarget { rcs_m2: 5.0, range_m: 60_000.0, radial_speed_mps: v, swerling: Swerling::One, path_loss_db: 0.0 };
        let closing = detect(&spec(), &mk(200.0), bg, RadarMode::Search, &p);
        let beaming = detect(&spec(), &mk(1.0), bg, RadarMode::Search, &p);
        assert!(beaming.in_notch && !closing.in_notch);
        assert!(closing.pd_per_look > 0.8, "{closing:?}");
        assert!(beaming.pd_per_look < 0.1, "{beaming:?}");
        let track = detect(&spec(), &mk(200.0), bg, RadarMode::Track, &p);
        assert!(track.rate_per_s > closing.rate_per_s);
    }

    #[test]
    fn clutter_by_cover_and_sea_state() {
        let p = RadarParams::from_figures(&figs());
        assert!(ground_sigma0(LandCover::Urban, 0.05, &p) > ground_sigma0(LandCover::Desert, 0.05, &p));
        assert!(sea_sigma0(5.0, 0.02, &p) > sea_sigma0(1.0, 0.02, &p));
        let rcs = RcsProfile { front_m2: 1.0, side_m2: 10.0, rear_m2: 3.0, top_m2: 20.0 };
        assert_relative_eq!(rcs.at(0.0, 0.0), 1.0, epsilon = 1e-9);
        assert_relative_eq!(rcs.at(PI / 2.0, 0.0), 10.0, epsilon = 1e-9);
        assert_relative_eq!(rcs.at(PI, 0.0), 3.0, epsilon = 1e-9);
        assert_relative_eq!(rcs.at(0.3, PI / 2.0), 20.0, epsilon = 1e-9);
    }

    proptest! {
        #[test]
        fn pd_monotone_in_snr(snr_db in -10.0f64..30.0, model in 0u8..3) {
            let m = [Swerling::Zero, Swerling::One, Swerling::Three][model as usize];
            let a = swerling_pd(m, 10f64.powf(snr_db / 10.0), 1e-6, 1.0);
            let b = swerling_pd(m, 10f64.powf((snr_db + 1.0) / 10.0), 1e-6, 1.0);
            prop_assert!((0.0..=1.0).contains(&a));
            prop_assert!(b >= a - 1e-6);
        }
    }
}
