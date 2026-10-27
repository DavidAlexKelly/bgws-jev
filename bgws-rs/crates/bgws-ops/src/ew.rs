//! Electronic warfare (RUST_ENGINE.md §8.7; research R5.3, R8 [CMO]).
//!
//! - **Jammers are emitters** with bands, ERP, an antenna pattern, a duty
//!   cycle, look-through and a power budget split across the bands they
//!   cover: a jammer can't jam everything at full power.
//! - **J/S is computed at each receiver** from the jammer's effective ERP
//!   toward it, the path loss (supplied by the caller from §8.4, so terrain
//!   shields), the receiver's antenna gain toward the jammer and the
//!   bandwidth mismatch. The communications form is
//!   J/S = (P_j G_j / P_s G_s)·(R_s² / R_j²) in free space **[R5.3]**.
//! - **Burn-through** is solved, not set: the range at which J/S falls to
//!   what the victim tolerates (bisection on any path-loss function), with
//!   the closed form for a self-screening jammer against a radar
//!   (J/S ∝ R², so R_bt = √(…)) as a check.
//! - **Radar jamming:** noise (raises the noise floor, shrinks detection
//!   range as (S/(N+J))^¼) and deception (range and velocity gate pull-off)
//!   against search and track modes, with ECCM factors: frequency agility
//!   spreads a spot jammer thin, leading-edge tracking and Doppler checks
//!   blunt RGPO/VGPO, sidelobe blanking cuts sidelobe jamming.
//! - **Signature:** a jammer is a loud emitter; [`esm_detection_range`]
//!   gives how far an ESM receiver hears it (§8.3), so it becomes a target.

use crate::math::{bisect, db_to_ratio, ratio_to_db, soft_step};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// Figures for EW (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("ew.sidelobe_dbi", -10.0, "dBi", -30.0, 0.0, "default", Default, "jammer antenna sidelobe gain"),
        figure!("ew.look_through_fraction", 0.1, "fraction", 0.0, 0.5, "default", Default, "time a jammer stops to listen"),
        figure!("ew.comms_js_required_db", 0.0, "dB", -10.0, 15.0, "default", Default, "J/S that defeats an unprotected FM/FSK link"),
        figure!("ew.fhss_processing_gain_db", 15.0, "dB", 0.0, 30.0, "default", Default, "frequency hopping gain against a partial-band jammer"),
        figure!("ew.radar.detect_snr_db", 13.0, "dB", 8.0, 20.0, "default", Default, "radar SNR for Pd≈0.9 (Swerling 1-ish)"),
        figure!("ew.radar.rgpo_js_db", 6.0, "dB", 0.0, 20.0, "R8", Low, "J/S at which RGPO captures a range gate (CMO-style)"),
        figure!("ew.radar.vgpo_js_db", 6.0, "dB", 0.0, 20.0, "R8", Low, "J/S at which VGPO captures a velocity gate"),
        figure!("ew.radar.deception_width_db", 2.0, "dB", 0.5, 6.0, "default", Default, "transition width of gate capture"),
        figure!("ew.radar.leading_edge_factor", 0.4, "fraction", 0.0, 1.0, "default", Default, "RGPO effect left with leading-edge tracking"),
        figure!("ew.radar.doppler_check_factor", 0.5, "fraction", 0.0, 1.0, "default", Default, "VGPO effect left with range-rate/Doppler cross-check"),
        figure!("ew.radar.agility_factor", 0.5, "fraction", 0.0, 1.0, "R8", Low, "deception effect left against a frequency-agile radar"),
        figure!("ew.radar.sidelobe_blanking_db", 20.0, "dB", 0.0, 40.0, "default", Default, "rejection of sidelobe jamming with SLB/SLC"),
        figure!("ew.radar.false_targets_per_db", 0.5, "1/dB", 0.0, 5.0, "default", Default, "false targets generated per dB of J/S above threshold in search"),
        figure!("ew.esm_sensitivity_dbw", -110.0, "dBW", -140.0, -70.0, "default", Default, "ESM receiver sensitivity"),
    ]
}

/// A jammer antenna pattern.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AntennaPattern {
    Omni { gain_dbi: f64 },
    /// A main lobe of `beamwidth_rad` (−3 dB full width) with Gaussian roll-off, floored at the sidelobe level.
    Sector { gain_dbi: f64, beamwidth_rad: f64, sidelobe_dbi: f64 },
}

impl AntennaPattern {
    /// Gain toward a direction `off_boresight_rad` from the main axis.
    pub fn gain_dbi(&self, off_boresight_rad: f64) -> f64 {
        match *self {
            AntennaPattern::Omni { gain_dbi } => gain_dbi,
            AntennaPattern::Sector { gain_dbi, beamwidth_rad, sidelobe_dbi } => {
                let x = off_boresight_rad.abs() / (beamwidth_rad / 2.0).max(1e-6);
                (gain_dbi - 3.0 * x * x).max(sidelobe_dbi)
            }
        }
    }
}

/// A frequency band a jammer covers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JamBand {
    pub centre_hz: f64,
    pub width_hz: f64,
    /// Share of the power budget (normalised across bands).
    pub weight: f64,
}

impl JamBand {
    pub fn covers(&self, f_hz: f64, bw_hz: f64) -> bool {
        let lo = self.centre_hz - self.width_hz / 2.0;
        let hi = self.centre_hz + self.width_hz / 2.0;
        f_hz + bw_hz / 2.0 > lo && f_hz - bw_hz / 2.0 < hi
    }
}

/// A jammer as an emitter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Jammer {
    /// Total transmitter power, W (before antenna gain).
    pub power_w: f64,
    pub pattern: AntennaPattern,
    pub bands: Vec<JamBand>,
    /// Fraction of time radiating (pulsed or sweeping).
    pub duty_cycle: f64,
    /// Fraction of time it stops to listen.
    pub look_through: f64,
    pub on: bool,
}

/// What a jammer puts into one receiver (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JamAtReceiver {
    pub band: Option<usize>,
    /// ERP toward the receiver in that band, dBW.
    pub erp_dbw: f64,
    pub power_split_db: f64,
    pub pattern_gain_dbi: f64,
    pub bandwidth_factor_db: f64,
    pub duty_factor_db: f64,
    pub path_loss_db: f64,
    pub rx_gain_dbi: f64,
    /// Jamming power in the receiver's bandwidth, dBW (−∞ if not covered).
    pub j_dbw: f64,
}

/// A victim receiver.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receiver {
    pub freq_hz: f64,
    pub bandwidth_hz: f64,
    /// Wanted signal at the receiver, dBW.
    pub signal_dbw: f64,
    /// Thermal noise in the bandwidth, dBW.
    pub noise_dbw: f64,
    /// Processing gain against jamming (spread spectrum, hopping), dB.
    pub processing_gain_db: f64,
}

/// J/S and SINR at a receiver (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsResult {
    pub jam: JamAtReceiver,
    pub js_db: f64,
    pub sinr_db: f64,
    pub snr_db: f64,
}

/// General EW parameters from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EwParams {
    pub sidelobe_dbi: f64,
    pub look_through: f64,
    pub comms_js_required_db: f64,
    pub fhss_gain_db: f64,
    pub radar_detect_snr_db: f64,
    pub esm_sensitivity_dbw: f64,
}

impl EwParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            sidelobe_dbi: f.get("ew.sidelobe_dbi"),
            look_through: f.get("ew.look_through_fraction"),
            comms_js_required_db: f.get("ew.comms_js_required_db"),
            fhss_gain_db: f.get("ew.fhss_processing_gain_db"),
            radar_detect_snr_db: f.get("ew.radar.detect_snr_db"),
            esm_sensitivity_dbw: f.get("ew.esm_sensitivity_dbw"),
        }
    }

    /// A sector antenna with the declared sidelobe level.
    pub fn sector(&self, gain_dbi: f64, beamwidth_rad: f64) -> AntennaPattern {
        AntennaPattern::Sector { gain_dbi, beamwidth_rad, sidelobe_dbi: self.sidelobe_dbi }
    }
}

/// Is a communications link jammed? (rule 10)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommsJamVerdict {
    pub raw_js_db: f64,
    /// After frequency-hopping gain, if the waveform hops.
    pub effective_js_db: f64,
    pub required_js_db: f64,
    pub jammed: bool,
}

/// Compare J/S at a comms receiver against what defeats it; a hopping
/// waveform (SINCGARS-like) needs that much more.
pub fn comms_jammed(raw_js_db: f64, hopping: bool, p: &EwParams) -> CommsJamVerdict {
    let eff = raw_js_db - if hopping { p.fhss_gain_db } else { 0.0 };
    CommsJamVerdict { raw_js_db, effective_js_db: eff, required_js_db: p.comms_js_required_db, jammed: eff >= p.comms_js_required_db }
}

/// Detection margin of a radar under jamming: SNR + jamming loss − detection threshold, dB.
pub fn radar_detection_margin_db(snr_db: f64, effect: &RadarJamEffect, p: &EwParams) -> f64 {
    snr_db + effect.snr_loss_db - p.radar_detect_snr_db
}

impl Jammer {
    /// A continuous jammer with the declared look-through.
    pub fn new(power_w: f64, pattern: AntennaPattern, bands: Vec<JamBand>, p: &EwParams) -> Self {
        Self { power_w, pattern, bands, duty_cycle: 1.0, look_through: p.look_through, on: true }
    }

    /// Power split across bands: each band's share of the budget, dB (≤ 0).
    pub fn split_db(&self, band: usize) -> f64 {
        let total: f64 = self.bands.iter().map(|b| b.weight.max(0.0)).sum();
        if total <= 0.0 {
            return f64::NEG_INFINITY;
        }
        ratio_to_db(self.bands[band].weight.max(0.0) / total)
    }

    /// Jamming delivered into a receiver tuned to `freq_hz` with bandwidth
    /// `bw_hz`, given the path loss and geometry.
    pub fn at_receiver(&self, freq_hz: f64, bw_hz: f64, path_loss_db: f64, off_boresight_rad: f64, rx_gain_dbi: f64) -> JamAtReceiver {
        let band = self.bands.iter().position(|b| b.covers(freq_hz, bw_hz));
        let pattern = self.pattern.gain_dbi(off_boresight_rad);
        let duty = ratio_to_db((self.duty_cycle * (1.0 - self.look_through)).clamp(1e-9, 1.0));
        match (band, self.on) {
            (Some(i), true) => {
                let b = self.bands[i];
                let split = self.split_db(i);
                // Only the part of the jamming that falls in the receiver's bandwidth counts.
                let bwf = if b.width_hz > bw_hz { ratio_to_db(bw_hz / b.width_hz) } else { 0.0 };
                let erp = ratio_to_db(self.power_w) + split + pattern;
                let j = erp + bwf + duty - path_loss_db + rx_gain_dbi;
                JamAtReceiver { band: Some(i), erp_dbw: erp, power_split_db: split, pattern_gain_dbi: pattern, bandwidth_factor_db: bwf, duty_factor_db: duty, path_loss_db, rx_gain_dbi, j_dbw: j }
            }
            _ => JamAtReceiver {
                band: None,
                erp_dbw: f64::NEG_INFINITY,
                power_split_db: 0.0,
                pattern_gain_dbi: pattern,
                bandwidth_factor_db: 0.0,
                duty_factor_db: duty,
                path_loss_db,
                rx_gain_dbi,
                j_dbw: f64::NEG_INFINITY,
            },
        }
    }

    /// J/S and SINR at a receiver, with its processing gain applied to J.
    pub fn js_at(&self, rx: &Receiver, path_loss_db: f64, off_boresight_rad: f64, rx_gain_dbi: f64) -> JsResult {
        let jam = self.at_receiver(rx.freq_hz, rx.bandwidth_hz, path_loss_db, off_boresight_rad, rx_gain_dbi);
        let j_eff = jam.j_dbw - rx.processing_gain_db;
        let js_db = j_eff - rx.signal_dbw;
        let n_plus_j = db_to_ratio(rx.noise_dbw) + if j_eff.is_finite() { db_to_ratio(j_eff) } else { 0.0 };
        JsResult { jam, js_db, sinr_db: rx.signal_dbw - ratio_to_db(n_plus_j), snr_db: rx.signal_dbw - rx.noise_dbw }
    }
}

/// Several jammers into one receiver: powers add.
pub fn combined_jamming_dbw(levels: impl IntoIterator<Item = f64>) -> f64 {
    let s: f64 = levels.into_iter().filter(|l| l.is_finite()).map(db_to_ratio).sum();
    if s > 0.0 {
        ratio_to_db(s)
    } else {
        f64::NEG_INFINITY
    }
}

/// Free-space communications J/S **[R5.3]**:
/// (P_j G_j / P_s G_s)·(R_s² / R_j²), in dB.
pub fn comms_js_free_space_db(pj_w: f64, gj: f64, ps_w: f64, gs: f64, rs_m: f64, rj_m: f64) -> f64 {
    ratio_to_db((pj_w * gj) / (ps_w * gs) * (rs_m * rs_m) / (rj_m * rj_m))
}

/// The range from the jammer at which J/S falls to `required_js_db`:
/// inside it the receiver is jammed. `js_at_range` maps jammer-to-receiver
/// range to J/S (any path-loss model). `None` if J/S never crosses.
pub fn jamming_radius(js_at_range: impl Fn(f64) -> f64, required_js_db: f64, max_m: f64) -> Option<f64> {
    bisect(1.0, max_m, |r| js_at_range(r) - required_js_db)
}

/// Self-screening jammer burn-through against a monostatic radar:
/// J/S = (P_j G_j 4π R²) / (P_t G_t σ) (with bandwidth and loss terms in
/// `jam_factor`), so R_bt = √(js_required · P_t G_t σ / (P_j G_j 4π · jam_factor)).
pub fn self_screen_burn_through_m(pt_w: f64, gt: f64, sigma_m2: f64, pj_w: f64, gj: f64, jam_factor: f64, js_required_db: f64) -> f64 {
    (db_to_ratio(js_required_db) * pt_w * gt * sigma_m2 / (pj_w * gj * 4.0 * std::f64::consts::PI * jam_factor)).sqrt()
}

/// J/S of a self-screening jammer at range R (the same terms).
pub fn self_screen_js_db(pt_w: f64, gt: f64, sigma_m2: f64, pj_w: f64, gj: f64, jam_factor: f64, r_m: f64) -> f64 {
    ratio_to_db(pj_w * gj * 4.0 * std::f64::consts::PI * r_m * r_m * jam_factor / (pt_w * gt * sigma_m2))
}

/// Radar modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadarMode {
    Search,
    Track,
}

/// Radar jamming techniques.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadarJamTechnique {
    /// Spot noise on the radar's current frequency.
    SpotNoise,
    /// Barrage noise across the radar's agile band.
    BarrageNoise,
    /// Range gate pull-off.
    Rgpo,
    /// Velocity gate pull-off.
    Vgpo,
}

/// Electronic counter-countermeasures fitted to a radar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Eccm {
    /// Agile bandwidth over instantaneous bandwidth (1 = not agile).
    pub agility_ratio: f64,
    pub leading_edge_tracking: bool,
    pub doppler_check: bool,
    pub sidelobe_blanking: bool,
}

/// What jamming does to a radar (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarJamEffect {
    pub technique: RadarJamTechnique,
    pub mode: RadarMode,
    /// J/S after ECCM.
    pub effective_js_db: f64,
    /// S/(N+J) − S/N, dB (≤ 0).
    pub snr_loss_db: f64,
    /// Detection range relative to unjammed, (S/(N+J) / (S/N))^¼.
    pub range_factor: f64,
    /// Probability the track is broken (deception against track mode).
    pub p_break_lock: f64,
    /// Expected false targets (deception against search mode).
    pub false_targets: f64,
    pub notes: Vec<String>,
}

/// Radar jamming parameters from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadarEwParams {
    pub rgpo_js_db: f64,
    pub vgpo_js_db: f64,
    pub width_db: f64,
    pub leading_edge_factor: f64,
    pub doppler_check_factor: f64,
    pub agility_factor: f64,
    pub sidelobe_blanking_db: f64,
    pub false_targets_per_db: f64,
}

impl RadarEwParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            rgpo_js_db: f.get("ew.radar.rgpo_js_db"),
            vgpo_js_db: f.get("ew.radar.vgpo_js_db"),
            width_db: f.get("ew.radar.deception_width_db"),
            leading_edge_factor: f.get("ew.radar.leading_edge_factor"),
            doppler_check_factor: f.get("ew.radar.doppler_check_factor"),
            agility_factor: f.get("ew.radar.agility_factor"),
            sidelobe_blanking_db: f.get("ew.radar.sidelobe_blanking_db"),
            false_targets_per_db: f.get("ew.radar.false_targets_per_db"),
        }
    }
}

/// Effect of a jamming technique on a radar mode, given the raw J/S at the
/// radar (jamming power over target echo in the radar's instantaneous
/// bandwidth, before ECCM), the radar's unjammed SNR and whether the jamming
/// arrives through the sidelobes. **[R8: CMO]** models agility and gate
/// pull-off as modifiers; this follows suit.
pub fn radar_jam_effect(p: &RadarEwParams, technique: RadarJamTechnique, mode: RadarMode, raw_js_db: f64, snr_db: f64, eccm: &Eccm, via_sidelobes: bool) -> RadarJamEffect {
    let mut notes = Vec::new();
    let mut js = raw_js_db;
    if via_sidelobes && eccm.sidelobe_blanking {
        js -= p.sidelobe_blanking_db;
        notes.push(format!("sidelobe blanking −{:.0} dB", p.sidelobe_blanking_db));
    }
    let agility = eccm.agility_ratio.max(1.0);
    let agile = agility > 1.0;
    match technique {
        RadarJamTechnique::SpotNoise if agile => {
            // The radar hops away; a spot jammer only hits 1/ratio of pulses.
            js -= ratio_to_db(agility);
            notes.push(format!("frequency agility spreads spot noise: −{:.1} dB", ratio_to_db(agility)));
        }
        RadarJamTechnique::BarrageNoise => {
            // A barrage already covers the agile band but at lower density.
            js -= ratio_to_db(agility);
            if agile {
                notes.push(format!("barrage spread over agile band: −{:.1} dB", ratio_to_db(agility)));
            }
        }
        _ => {}
    }
    let (snr_loss_db, range_factor) = match technique {
        RadarJamTechnique::SpotNoise | RadarJamTechnique::BarrageNoise => {
            // J/N = J/S + S/N; S/(N+J) = S/N − 10log(1 + J/N).
            let jn = js + snr_db;
            let loss = -ratio_to_db(1.0 + db_to_ratio(jn));
            (loss, db_to_ratio(loss).powf(0.25))
        }
        _ => (0.0, 1.0),
    };
    let (mut p_break, mut false_targets) = (0.0, 0.0);
    if matches!(technique, RadarJamTechnique::Rgpo | RadarJamTechnique::Vgpo) {
        let thr = if technique == RadarJamTechnique::Rgpo { p.rgpo_js_db } else { p.vgpo_js_db };
        let mut eff = 1.0;
        if technique == RadarJamTechnique::Rgpo && eccm.leading_edge_tracking {
            eff *= p.leading_edge_factor;
            notes.push("leading-edge tracking".into());
        }
        if technique == RadarJamTechnique::Vgpo && eccm.doppler_check {
            eff *= p.doppler_check_factor;
            notes.push("Doppler/range-rate cross-check".into());
        }
        if agile {
            eff *= p.agility_factor;
            notes.push("frequency agility vs repeater".into());
        }
        let capture = soft_step(js, thr, p.width_db);
        match mode {
            RadarMode::Track => p_break = capture * eff,
            RadarMode::Search => false_targets = ((js - thr).max(0.0) * p.false_targets_per_db * eff).max(0.0),
        }
    }
    RadarJamEffect { technique, mode, effective_js_db: js, snr_loss_db, range_factor, p_break_lock: p_break, false_targets, notes }
}

/// How far an ESM receiver hears an emitter of `erp_dbw` (§8.3 signature),
/// given a path-loss function of range and the receiver's sensitivity.
/// ESM detection range with the declared ESM sensitivity.
pub fn esm_range(erp_dbw: f64, path_loss_db: impl Fn(f64) -> f64, max_m: f64, p: &EwParams) -> f64 {
    esm_detection_range(erp_dbw, p.esm_sensitivity_dbw, path_loss_db, max_m)
}

pub fn esm_detection_range(erp_dbw: f64, sensitivity_dbw: f64, path_loss_db: impl Fn(f64) -> f64, max_m: f64) -> f64 {
    bisect(1.0, max_m, |r| erp_dbw - path_loss_db(r) - sensitivity_dbw).unwrap_or(if erp_dbw - path_loss_db(max_m) > sensitivity_dbw { max_m } else { 0.0 })
}

/// The jammer's signature: peak ERP per band (the loudest thing ESM sees)
/// and its average radiated power.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JammerSignature {
    pub band_erp_dbw: Vec<f64>,
    pub mean_power_w: f64,
    pub duty: f64,
}

pub fn jammer_signature(j: &Jammer) -> JammerSignature {
    let main = j.pattern.gain_dbi(0.0);
    let band_erp_dbw = (0..j.bands.len()).map(|i| ratio_to_db(j.power_w) + j.split_db(i) + main).collect();
    let duty = j.duty_cycle * (1.0 - j.look_through);
    JammerSignature { band_erp_dbw, mean_power_w: if j.on { j.power_w * duty } else { 0.0 }, duty }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::free_space_loss_db;
    use approx::assert_relative_eq;

    fn jammer(bands: usize) -> Jammer {
        Jammer {
            power_w: 100.0,
            pattern: AntennaPattern::Omni { gain_dbi: 3.0 },
            bands: (0..bands).map(|i| JamBand { centre_hz: 60e6 + i as f64 * 1e9, width_hz: 10e6, weight: 1.0 }).collect(),
            duty_cycle: 1.0,
            look_through: 0.0,
            on: true,
        }
    }

    #[test]
    fn power_split_and_bandwidth_mismatch() {
        let one = jammer(1).at_receiver(60e6, 25e3, 100.0, 0.0, 0.0);
        let two = jammer(2).at_receiver(60e6, 25e3, 100.0, 0.0, 0.0);
        assert_relative_eq!(one.j_dbw - two.j_dbw, 3.0103, epsilon = 1e-3);
        // 25 kHz of a 10 MHz barrage: −26 dB.
        assert_relative_eq!(one.bandwidth_factor_db, -26.02, epsilon = 0.01);
        // 20 dBW + 3 dBi − 26 − 100.
        assert_relative_eq!(one.j_dbw, 20.0 + 3.0 - 26.0206 - 100.0, epsilon = 1e-3);
        assert_eq!(jammer(1).at_receiver(300e6, 25e3, 100.0, 0.0, 0.0).j_dbw, f64::NEG_INFINITY);
    }

    #[test]
    fn free_space_js_matches_the_formula() {
        // Equal powers and gains, jammer twice as far as the signal: −6 dB.
        assert_relative_eq!(comms_js_free_space_db(10.0, 1.0, 10.0, 1.0, 1000.0, 2000.0), -6.0206, epsilon = 1e-3);
        // And the full receiver chain in free space agrees.
        let f = 60e6;
        let mut j = jammer(1);
        j.pattern = AntennaPattern::Omni { gain_dbi: 0.0 };
        j.bands[0].width_hz = 25e3;
        j.power_w = 10.0;
        let s = 10.0 - free_space_loss_db(1000.0, f);
        let rx = Receiver { freq_hz: f, bandwidth_hz: 25e3, signal_dbw: s, noise_dbw: -150.0, processing_gain_db: 0.0 };
        let r = j.js_at(&rx, free_space_loss_db(2000.0, f), 0.0, 0.0);
        assert_relative_eq!(r.js_db, -6.0206, epsilon = 1e-3);
    }

    #[test]
    fn burn_through_bisection_matches_closed_form() {
        let (pt, gt, sigma, pj, gj, k) = (1e6, db_to_ratio(35.0), 5.0, 100.0, db_to_ratio(10.0), 0.01);
        let req = 10.0;
        let closed = self_screen_burn_through_m(pt, gt, sigma, pj, gj, k, req);
        let solved = jamming_radius(|r| self_screen_js_db(pt, gt, sigma, pj, gj, k, r), req, 1e7).unwrap();
        assert_relative_eq!(closed, solved, max_relative = 1e-6);
    }

    #[test]
    fn pattern_and_duty_reduce_jamming() {
        let mut j = jammer(1);
        j.pattern = AntennaPattern::Sector { gain_dbi: 15.0, beamwidth_rad: 0.5, sidelobe_dbi: -10.0 };
        let main = j.at_receiver(60e6, 25e3, 100.0, 0.0, 0.0).j_dbw;
        let edge = j.at_receiver(60e6, 25e3, 100.0, 0.25, 0.0).j_dbw;
        let side = j.at_receiver(60e6, 25e3, 100.0, 2.0, 0.0).j_dbw;
        assert_relative_eq!(main - edge, 3.0, epsilon = 1e-9);
        assert_relative_eq!(main - side, 25.0, epsilon = 1e-9);
        j.duty_cycle = 0.5;
        j.look_through = 0.2;
        assert_relative_eq!(main - j.at_receiver(60e6, 25e3, 100.0, 0.0, 0.0).j_dbw, -ratio_to_db(0.4), epsilon = 1e-9);
    }

    #[test]
    fn eccm_blunts_jamming() {
        let p = RadarEwParams::from_figures(&crate::default_figures());
        let none = Eccm::default();
        let agile = Eccm { agility_ratio: 20.0, leading_edge_tracking: true, doppler_check: true, sidelobe_blanking: true };
        let a = radar_jam_effect(&p, RadarJamTechnique::SpotNoise, RadarMode::Search, 10.0, 15.0, &none, false);
        let b = radar_jam_effect(&p, RadarJamTechnique::SpotNoise, RadarMode::Search, 10.0, 15.0, &agile, false);
        assert!(a.range_factor < b.range_factor);
        assert!(a.range_factor < 0.3);
        let c = radar_jam_effect(&p, RadarJamTechnique::Rgpo, RadarMode::Track, 15.0, 15.0, &none, false);
        let d = radar_jam_effect(&p, RadarJamTechnique::Rgpo, RadarMode::Track, 15.0, 15.0, &agile, false);
        assert!(c.p_break_lock > 0.9 && d.p_break_lock < 0.25, "{} {}", c.p_break_lock, d.p_break_lock);
        let e = radar_jam_effect(&p, RadarJamTechnique::Vgpo, RadarMode::Search, 16.0, 15.0, &none, false);
        assert!(e.false_targets > 1.0);
        let s = radar_jam_effect(&p, RadarJamTechnique::BarrageNoise, RadarMode::Search, 10.0, 15.0, &agile, true);
        assert!(s.effective_js_db < -20.0);
    }

    #[test]
    fn params_drive_verdicts() {
        let f = crate::default_figures();
        let p = EwParams::from_figures(&f);
        assert!(comms_jammed(5.0, false, &p).jammed);
        assert!(!comms_jammed(5.0, true, &p).jammed);
        let j = Jammer::new(100.0, p.sector(15.0, 0.5), vec![JamBand { centre_hz: 60e6, width_hz: 1e6, weight: 1.0 }], &p);
        assert_relative_eq!(j.pattern.gain_dbi(3.0), p.sidelobe_dbi);
        assert!(j.look_through > 0.0);
        let rp = RadarEwParams::from_figures(&f);
        let e = radar_jam_effect(&rp, RadarJamTechnique::SpotNoise, RadarMode::Search, 0.0, 20.0, &Eccm::default(), false);
        assert!(radar_detection_margin_db(20.0, &e, &p) < 20.0 - p.radar_detect_snr_db);
        assert!(esm_range(10.0, |d| free_space_loss_db(d, 60e6), 1e8, &p) > 1e5);
    }

    #[test]
    fn esm_hears_a_jammer_far_away() {
        let sig = jammer_signature(&jammer(2));
        assert_relative_eq!(sig.band_erp_dbw[0], 20.0 - 3.0103 + 3.0, epsilon = 1e-3);
        let r = esm_detection_range(sig.band_erp_dbw[0], -110.0, |d| free_space_loss_db(d, 60e6), 1e7);
        // 20 dBW at 60 MHz against −110 dBW: 130 dB of free space ≈ 1.25e3 km.
        assert!(r > 1e6, "{r}");
    }
}
