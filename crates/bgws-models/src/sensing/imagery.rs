//! SAR and EO satellite (or high-altitude) imagery (§8.2): an image of a
//! footprint during an access window. EO needs a cloud-free line of sight;
//! SAR always images but is harder to interpret (a higher N₅₀). Whether an
//! analyst detects, recognises or identifies a target depends on how many
//! resolved cycles the ground sample distance (GSD) puts across it, through
//! the same ACQUIRE curve as every other imager. Downlink and analysis
//! latency are §8.11's queue, not this model's.

use super::acquire::{p_infinity, AcquireParams, Stage};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageKind {
    Eo,
    Sar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageryParams {
    pub acquire: AcquireParams,
    pub sar_n50_factor: f64,
    pub pixels_per_cycle: f64,
}

impl ImageryParams {
    pub fn from_figures(f: &Figures) -> Self {
        ImageryParams {
            acquire: AcquireParams::from_figures(f),
            sar_n50_factor: f.get("sensing.imagery.sar_n50_factor"),
            pixels_per_cycle: f.get("sensing.imagery.pixels_per_cycle"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageryResult {
    /// Probability the line of sight is cloud-free (1 for SAR).
    pub p_clear: f64,
    pub cycles: f64,
    pub n50: f64,
    /// P(stage | the image was taken and is clear).
    pub p_stage: f64,
    /// Overall: p_clear × p_stage.
    pub p: f64,
}

/// The chance one image lets an analyst reach `stage` on a target with
/// critical dimension `critical_dim_m` at ground sample distance `gsd_m`.
/// `quality` (0..1) folds in image MTF, look angle and analyst skill.
pub fn image_identification(kind: ImageKind, critical_dim_m: f64, gsd_m: f64, quality: f64, cloud_free: f64, stage: Stage, p: &ImageryParams) -> ImageryResult {
    let cycles = critical_dim_m.max(0.0) / (p.pixels_per_cycle * gsd_m.max(1e-3)) * quality.clamp(0.0, 1.0);
    let base = p.acquire.n50[stage as usize];
    let (n50, p_clear) = match kind {
        ImageKind::Eo => (base, cloud_free.clamp(0.0, 1.0)),
        ImageKind::Sar => (base * p.sar_n50_factor, 1.0),
    };
    let (ps, _) = p_infinity(cycles, n50, &p.acquire);
    ImageryResult { p_clear, cycles, n50, p_stage: ps, p: p_clear * ps }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.imagery.sar_n50_factor", 1.5, "1", 1.0, 3.0, "default", Default, "SAR images need more cycles than EO for the same task"),
        figure!("sensing.imagery.pixels_per_cycle", 2.0, "px", 1.5, 3.0, "default", Default, "pixels per resolved cycle (Nyquist)"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;

    #[test]
    fn gsd_sets_the_task() {
        let p = ImageryParams::from_figures(&figs());
        // A 3 m tank: 0.3 m GSD identifies it, 3 m GSD barely detects it.
        let fine = image_identification(ImageKind::Eo, 3.0, 0.3, 1.0, 1.0, Stage::Identify, &p);
        let coarse = image_identification(ImageKind::Eo, 3.0, 3.0, 1.0, 1.0, Stage::Identify, &p);
        assert!(fine.p > 0.3 && coarse.p < 0.01, "{fine:?} {coarse:?}");
        let cloudy = image_identification(ImageKind::Eo, 3.0, 0.3, 1.0, 0.2, Stage::Identify, &p);
        assert!(cloudy.p < fine.p);
        let sar = image_identification(ImageKind::Sar, 3.0, 0.3, 1.0, 0.0, Stage::Identify, &p);
        assert!(sar.p_clear == 1.0 && sar.p_stage < fine.p_stage);
    }
}
