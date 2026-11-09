//! Sensing (RUST_ENGINE.md §8.2). Every sensor answers three questions the
//! same way: **can it see** (geometry, masking — computed elsewhere and
//! passed in as path transmission or loss), **how strongly** (resolved cycles,
//! SNR or signal excess) and **how fast it detects** (a hazard rate from that
//! number, rule 2). Identification and classification are separate stages.
//!
//! - [`acquire`]: eye, optics, image intensifiers, EO and thermal imagers —
//!   ACQUIRE/TTP **[R8, P22]**, plus IRST point detection.
//! - [`radar`]: the radar range equation, Swerling 0/1/3, clutter, MTI notch,
//!   scan and track modes.
//! - [`esm`]: ESM/RWR/SIGINT intercept, DF bearing error, least-squares fixes.
//! - [`sonar`]: passive and active sonar equations, noise, reverberation.
//! - [`acoustic`]: battlefield acoustic sensors in air.
//! - [`imagery`]: SAR and EO satellite image identification by GSD.
//! - [`observer`]: human search with attention limits.

pub mod acoustic;
pub mod acquire;
pub mod esm;
pub mod imagery;
pub mod observer;
pub mod radar;
pub mod sonar;

use crate::math::norm_cdf;
use bgws_core::Figure;

/// The sensing figures.
pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    v.extend(acquire::figures());
    v.extend(radar::figures());
    v.extend(esm::figures());
    v.extend(sonar::figures());
    v.extend(acoustic::figures());
    v.extend(imagery::figures());
    v.extend(observer::figures());
    v
}

/// Probability of detection from a signal excess (dB) through a normal
/// transition of width `sigma_db` (the sonar-equation convention): Pd = Φ(SE/σ).
pub fn pd_from_signal_excess(se_db: f64, sigma_db: f64) -> f64 {
    if sigma_db <= 0.0 {
        return if se_db >= 0.0 { 1.0 } else { 0.0 };
    }
    norm_cdf(se_db / sigma_db)
}

/// The hazard rate (per second) that gives probability `p_look` per look of
/// `look_s` seconds: λ = −ln(1 − p)/τ (rule 2, §8.2).
pub fn rate_from_look(p_look: f64, look_s: f64) -> f64 {
    bgws_core::rate_from_probability(p_look.clamp(0.0, 1.0 - 1e-12), look_s)
}
