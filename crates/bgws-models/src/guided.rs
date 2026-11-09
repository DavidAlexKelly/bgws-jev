//! Air-delivered and guided munitions, basics (RUST_ENGINE.md §8.8):
//! release envelopes for free-fall and glide weapons, terminal seekers
//! (laser, IR, radar, GNSS/INS) with their own acquisition, countermeasures
//! (flares, chaff, GNSS jamming) and the CEP that results. The full flyout and
//! guidance laws are §8.1's (another crate); this gives the envelope gate and
//! the end-game odds. Precision fires collapse under GNSS jamming **[P15]**.

use crate::math::norm_cdf;
use bgws_core::units::G0;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// A free-fall or glide weapon's release limits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReleaseSpec {
    /// Lift-to-drag ratio of the glide kit (0 for a free-fall bomb).
    pub glide_ratio: f64,
    pub min_release_alt_m: f64,
    pub max_release_alt_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub min_range_m: f64,
    pub max_range_m: f64,
    pub altitude_ok: bool,
}

impl Envelope {
    pub fn contains(&self, range_m: f64) -> bool {
        self.altitude_ok && range_m >= self.min_range_m && range_m <= self.max_range_m
    }
}

/// The terminal seeker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeekerKind {
    /// Semi-active laser.
    Laser,
    /// Reticle/spin-scan IR (older, flare-susceptible).
    IrReticle,
    /// Imaging IR.
    IrImaging,
    /// Active radar.
    Radar,
    /// No seeker: GNSS/INS to coordinates.
    GnssIns,
}

/// What the seeker has to work with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SeekerInput {
    Laser { designated: bool, range_m: f64, transmission: f64 },
    /// Pd from `sensing::acquire::irst_detect` (IR) or `sensing::radar::detect` (radar).
    Detection { pd: f64 },
    None,
}

/// Navigation state at the terminal phase.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavState {
    pub gnss_available: bool,
    /// Seconds flown on INS alone since the last good GNSS fix.
    pub ins_only_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Countermeasures {
    pub flares: bool,
    pub chaff: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuidedResult {
    pub p_acquire: f64,
    pub p_decoyed: f64,
    pub seeker_cep_m: f64,
    pub nav_cep_m: f64,
    pub p_hit: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuidedParams {
    pub arming_time_s: f64,
    pub gnss_cep_m: f64,
    pub ins_drift_mps: f64,
    pub seeker_cep_m: [f64; 4],
    pub laser_ref_snr_db: f64,
    pub laser_ref_range_m: f64,
    pub laser_threshold_db: f64,
    pub laser_sigma_db: f64,
    pub flare_p_decoy_reticle: f64,
    pub flare_p_decoy_imaging: f64,
    pub chaff_p_decoy: f64,
}

impl GuidedParams {
    pub fn from_figures(f: &Figures) -> Self {
        GuidedParams {
            arming_time_s: f.get("guided.arming_time_s"),
            gnss_cep_m: f.get("guided.gnss_cep_m"),
            ins_drift_mps: f.get("guided.ins_drift_mps"),
            seeker_cep_m: ["laser", "ir_reticle", "ir_imaging", "radar"].map(|s| f.get(&format!("guided.seeker_cep_m.{s}"))),
            laser_ref_snr_db: f.get("guided.laser.ref_snr_db"),
            laser_ref_range_m: f.get("guided.laser.ref_range_m"),
            laser_threshold_db: f.get("guided.laser.threshold_db"),
            laser_sigma_db: f.get("guided.laser.sigma_db"),
            flare_p_decoy_reticle: f.get("guided.flare.p_decoy_reticle"),
            flare_p_decoy_imaging: f.get("guided.flare.p_decoy_imaging"),
            chaff_p_decoy: f.get("guided.chaff.p_decoy"),
        }
    }
}

/// Release envelope from altitude and speed (level release). Free fall:
/// range = v·√(2h/g). Glide: range ≈ L/D·(h + v²/2g) (energy height × glide
/// ratio), never less than the free-fall throw. Minimum range: the distance
/// flown while the fuze arms.
pub fn release_envelope(spec: &ReleaseSpec, altitude_m: f64, speed_mps: f64, p: &GuidedParams) -> Envelope {
    let h = altitude_m.max(0.0);
    let throw = speed_mps * (2.0 * h / G0).sqrt();
    let glide = spec.glide_ratio * (h + speed_mps * speed_mps / (2.0 * G0));
    Envelope {
        min_range_m: speed_mps * p.arming_time_s,
        max_range_m: throw.max(glide),
        altitude_ok: altitude_m >= spec.min_release_alt_m && altitude_m <= spec.max_release_alt_m,
    }
}

/// P(within r) for a circular normal with this CEP: 1 − 2^(−(r/CEP)²).
pub fn p_within(radius_m: f64, cep_m: f64) -> f64 {
    if cep_m <= 0.0 {
        return 1.0;
    }
    1.0 - 2f64.powf(-(radius_m / cep_m).powi(2))
}

/// Navigation CEP: GNSS when available, else GNSS error plus INS drift.
pub fn nav_cep(nav: &NavState, p: &GuidedParams) -> f64 {
    if nav.gnss_available {
        p.gnss_cep_m
    } else {
        p.gnss_cep_m.hypot(p.ins_drift_mps * nav.ins_only_s.max(0.0))
    }
}

/// Terminal end-game odds against a target of effective radius `target_radius_m`.
pub fn guided_engagement(seeker: SeekerKind, input: &SeekerInput, nav: &NavState, cm: &Countermeasures, target_radius_m: f64, p: &GuidedParams) -> GuidedResult {
    let p_acq = match (seeker, input) {
        (SeekerKind::GnssIns, _) => 0.0,
        (SeekerKind::Laser, SeekerInput::Laser { designated, range_m, transmission }) => {
            if !designated {
                0.0
            } else {
                let snr = p.laser_ref_snr_db - 20.0 * (range_m.max(1.0) / p.laser_ref_range_m).log10() + 10.0 * transmission.max(1e-12).log10();
                norm_cdf((snr - p.laser_threshold_db) / p.laser_sigma_db)
            }
        }
        (_, SeekerInput::Detection { pd }) => pd.clamp(0.0, 1.0),
        _ => 0.0,
    };
    let p_decoy = match seeker {
        SeekerKind::IrReticle if cm.flares => p.flare_p_decoy_reticle,
        SeekerKind::IrImaging if cm.flares => p.flare_p_decoy_imaging,
        SeekerKind::Radar if cm.chaff => p.chaff_p_decoy,
        _ => 0.0,
    };
    let s_cep = match seeker {
        SeekerKind::Laser => p.seeker_cep_m[0],
        SeekerKind::IrReticle => p.seeker_cep_m[1],
        SeekerKind::IrImaging => p.seeker_cep_m[2],
        SeekerKind::Radar => p.seeker_cep_m[3],
        SeekerKind::GnssIns => f64::INFINITY,
    };
    let n_cep = nav_cep(nav, p);
    // Acquired and not decoyed: seeker accuracy. Not acquired: falls back on navigation.
    let p_hit = p_acq * (1.0 - p_decoy) * p_within(target_radius_m, s_cep) + (1.0 - p_acq) * p_within(target_radius_m, n_cep);
    GuidedResult { p_acquire: p_acq, p_decoyed: p_acq * p_decoy, seeker_cep_m: s_cep, nav_cep_m: n_cep, p_hit }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("guided.arming_time_s", 3.0, "s", 0.5, 10.0, "default", Default, "fuze arming time after release"),
        figure!("guided.gnss_cep_m", 5.0, "m", 1.0, 30.0, "P15", Low, "GNSS-guided CEP unjammed"),
        figure!("guided.ins_drift_mps", 1.5, "m/s", 0.1, 10.0, "P15", Low, "INS-only error growth after GNSS loss"),
        figure!("guided.seeker_cep_m.laser", 1.0, "m", 0.3, 5.0, "default", Default, "semi-active laser CEP"),
        figure!("guided.seeker_cep_m.ir_reticle", 3.0, "m", 0.5, 10.0, "default", Default, "reticle IR seeker CEP"),
        figure!("guided.seeker_cep_m.ir_imaging", 1.5, "m", 0.3, 5.0, "default", Default, "imaging IR seeker CEP"),
        figure!("guided.seeker_cep_m.radar", 3.0, "m", 0.5, 15.0, "default", Default, "active radar seeker CEP"),
        figure!("guided.laser.ref_snr_db", 30.0, "dB", 10.0, 50.0, "default", Default, "laser spot SNR at the reference range, clear air"),
        figure!("guided.laser.ref_range_m", 5000.0, "m", 1000.0, 20000.0, "default", Default, "laser reference range"),
        figure!("guided.laser.threshold_db", 13.0, "dB", 5.0, 20.0, "default", Default, "laser seeker lock threshold"),
        figure!("guided.laser.sigma_db", 3.0, "dB", 1.0, 6.0, "default", Default, "laser lock transition width"),
        figure!("guided.flare.p_decoy_reticle", 0.6, "fraction", 0.2, 0.9, "default", Default, "flares decoy a reticle seeker"),
        figure!("guided.flare.p_decoy_imaging", 0.1, "fraction", 0.0, 0.5, "default", Default, "flares decoy an imaging seeker"),
        figure!("guided.chaff.p_decoy", 0.3, "fraction", 0.0, 0.8, "default", Default, "chaff decoys a radar seeker"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;

    #[test]
    fn envelopes() {
        let p = GuidedParams::from_figures(&figs());
        let bomb = ReleaseSpec { glide_ratio: 0.0, min_release_alt_m: 100.0, max_release_alt_m: 12_000.0 };
        let jdam_er = ReleaseSpec { glide_ratio: 8.0, ..bomb };
        let e = release_envelope(&bomb, 5000.0, 250.0, &p);
        assert_relative_eq!(e.max_range_m, 250.0 * (2.0 * 5000.0 / G0).sqrt(), epsilon = 1e-9);
        let g = release_envelope(&jdam_er, 10_000.0, 250.0, &p);
        assert!(g.max_range_m > 80_000.0 && g.contains(50_000.0) && !g.contains(500.0));
        assert!(!release_envelope(&bomb, 50.0, 250.0, &p).altitude_ok);
    }

    #[test]
    fn gnss_jamming_collapses_precision_p15() {
        let p = GuidedParams::from_figures(&figs());
        let none = Countermeasures::default();
        let ok = guided_engagement(SeekerKind::GnssIns, &SeekerInput::None, &NavState { gnss_available: true, ins_only_s: 0.0 }, &none, 10.0, &p);
        let jammed = guided_engagement(SeekerKind::GnssIns, &SeekerInput::None, &NavState { gnss_available: false, ins_only_s: 60.0 }, &none, 10.0, &p);
        assert!(ok.p_hit > 0.5 && jammed.p_hit < 0.1, "{ok:?} {jammed:?}");
    }

    #[test]
    fn seekers_and_countermeasures() {
        let p = GuidedParams::from_figures(&figs());
        let nav = NavState { gnss_available: false, ins_only_s: 30.0 };
        let none = Countermeasures::default();
        let lased = guided_engagement(SeekerKind::Laser, &SeekerInput::Laser { designated: true, range_m: 4000.0, transmission: 0.7 }, &nav, &none, 3.0, &p);
        let unlased = guided_engagement(SeekerKind::Laser, &SeekerInput::Laser { designated: false, range_m: 4000.0, transmission: 0.7 }, &nav, &none, 3.0, &p);
        assert!(lased.p_hit > 0.8 && unlased.p_hit < 0.1);
        let flares = Countermeasures { flares: true, chaff: false };
        let ret = guided_engagement(SeekerKind::IrReticle, &SeekerInput::Detection { pd: 0.95 }, &nav, &flares, 5.0, &p);
        let img = guided_engagement(SeekerKind::IrImaging, &SeekerInput::Detection { pd: 0.95 }, &nav, &flares, 5.0, &p);
        assert!(img.p_hit > ret.p_hit);
        assert_relative_eq!(p_within(5.0, 5.0), 0.5);
    }
}
