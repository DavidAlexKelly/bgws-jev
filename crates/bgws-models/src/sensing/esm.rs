//! ESM, RWR and SIGINT (§8.2): received power against sensitivity, weighted
//! by the emitter's time on air and by how often its scan illuminates the
//! receiver; direction finding with a bearing error from baseline and SNR;
//! and a weighted least-squares fix from two or more bearings with its error
//! ellipse. Path loss is an input (§8.4 computes it). Time to fix and time to
//! strike are open variables **[R8, P32]** with wide ranges.

use crate::ModelError;
use bgws_core::units::SPEED_OF_LIGHT;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// How the emitter's main beam moves relative to this receiver.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum EmitterScan {
    /// Omnidirectional (a radio, a beacon): always "main lobe".
    Omni,
    /// Rotating search: the main lobe sweeps the receiver a beamwidth/2π of the time.
    Rotating { beamwidth_rad: f64 },
    /// Pointed at the receiver (a fire-control radar tracking it): RWR case.
    PointedAt,
    /// Pointed elsewhere: only side lobes reach the receiver.
    Elsewhere,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    /// Transmit power + main-lobe gain, dBW.
    pub erp_dbw: f64,
    pub freq_hz: f64,
    /// Fraction of time on air (talk time, radar on/off schedule), 0..1.
    pub duty: f64,
    pub scan: EmitterScan,
    /// Side-lobe level relative to the main lobe, dB (negative); `None` uses the default.
    pub sidelobe_db: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receiver {
    pub sensitivity_dbw: f64,
    pub gain_dbi: f64,
    pub band_lo_hz: f64,
    pub band_hi_hz: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intercept {
    pub in_band: bool,
    pub pr_main_dbw: f64,
    pub pr_side_dbw: f64,
    pub margin_main_db: f64,
    pub margin_side_db: f64,
    /// Fraction of on-air time the receiver gets enough power.
    pub illumination: f64,
    pub rate_per_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EsmParams {
    pub sidelobe_db: f64,
    pub intercept_time_s: f64,
    pub df_floor_rad: f64,
    pub time_to_fix_s: f64,
}

impl EsmParams {
    pub fn from_figures(f: &Figures) -> Self {
        EsmParams {
            sidelobe_db: f.get("sensing.esm.sidelobe_db"),
            intercept_time_s: f.get("sensing.esm.intercept_time_s"),
            df_floor_rad: f.get("sensing.df.floor_deg").to_radians(),
            time_to_fix_s: f.get("sensing.df.time_to_fix_s"),
        }
    }
}

/// Does the receiver hear the emitter, and how fast does it intercept it.
pub fn intercept(e: &Emitter, r: &Receiver, path_loss_db: f64, p: &EsmParams) -> Intercept {
    let in_band = e.freq_hz >= r.band_lo_hz && e.freq_hz <= r.band_hi_hz;
    let main = e.erp_dbw - path_loss_db + r.gain_dbi;
    let side = main + e.sidelobe_db.unwrap_or(p.sidelobe_db);
    let mm = main - r.sensitivity_dbw;
    let ms = side - r.sensitivity_dbw;
    let illum = if !in_band {
        0.0
    } else {
        match e.scan {
            EmitterScan::Omni | EmitterScan::PointedAt => (mm >= 0.0) as u8 as f64,
            EmitterScan::Elsewhere => (ms >= 0.0) as u8 as f64,
            EmitterScan::Rotating { beamwidth_rad } => {
                if ms >= 0.0 {
                    1.0
                } else if mm >= 0.0 {
                    (beamwidth_rad / (2.0 * PI)).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
        }
    };
    let rate = e.duty.clamp(0.0, 1.0) * illum / p.intercept_time_s;
    Intercept { in_band, pr_main_dbw: main, pr_side_dbw: side, margin_main_db: mm, margin_side_db: ms, illumination: illum, rate_per_s: rate }
}

/// One-sigma DF bearing error of an interferometer with this baseline at
/// this SNR: σθ = √(floor² + (λ/(2π d √(2·SNR)))²).
pub fn df_bearing_sigma(freq_hz: f64, baseline_m: f64, snr_db: f64, p: &EsmParams) -> f64 {
    let lambda = SPEED_OF_LIGHT / freq_hz;
    let snr = 10f64.powf(snr_db / 10.0).max(1e-6);
    let inst = lambda / (2.0 * PI * baseline_m.max(1e-3) * (2.0 * snr).sqrt());
    (p.df_floor_rad.powi(2) + inst.powi(2)).sqrt().min(PI)
}

/// Mean time to build a fix on an emitter on air a `duty` fraction of the time **[P32]**.
pub fn time_to_fix_s(duty: f64, p: &EsmParams) -> f64 {
    p.time_to_fix_s / duty.clamp(1e-3, 1.0)
}

/// One DF line of bearing in a local east/north frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BearingFix {
    pub east_m: f64,
    pub north_m: f64,
    /// True bearing from north, clockwise, radians.
    pub bearing_rad: f64,
    pub sigma_rad: f64,
}

/// A located emitter with its error ellipse.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub east_m: f64,
    pub north_m: f64,
    /// Covariance [[ee, en], [ne, nn]], m².
    pub cov: [[f64; 2]; 2],
    /// Semi-axes of the ellipse holding `prob` of the probability, m.
    pub semi_major_m: f64,
    pub semi_minor_m: f64,
    /// Orientation of the major axis, from north clockwise, rad.
    pub orientation_rad: f64,
    pub prob: f64,
}

/// Weighted least-squares triangulation of two or more bearings. Each line
/// n·(p − pᵢ) = 0 has residual variance (σᵢ rᵢ)²; the weights are refined
/// with the ranges from the previous estimate.
pub fn triangulate(fixes: &[BearingFix], prob: f64) -> Result<Fix, ModelError> {
    if fixes.len() < 2 {
        return Err(ModelError::InvalidInput("a fix needs two or more bearings".into()));
    }
    let solve = |weights: &[f64]| -> Result<([f64; 2], [[f64; 2]; 2]), ModelError> {
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (f, w) in fixes.iter().zip(weights) {
            let (nx, ny) = (f.bearing_rad.cos(), -f.bearing_rad.sin());
            let c = nx * f.east_m + ny * f.north_m;
            a11 += w * nx * nx;
            a12 += w * nx * ny;
            a22 += w * ny * ny;
            b1 += w * nx * c;
            b2 += w * ny * c;
        }
        let det = a11 * a22 - a12 * a12;
        if det.abs() <= 1e-12 * (a11 * a22).abs().max(1e-300) {
            return Err(ModelError::NoSolution("bearings are parallel".into()));
        }
        let inv = [[a22 / det, -a12 / det], [-a12 / det, a11 / det]];
        Ok(([inv[0][0] * b1 + inv[0][1] * b2, inv[1][0] * b1 + inv[1][1] * b2], inv))
    };
    let mut w: Vec<f64> = fixes.iter().map(|f| 1.0 / f.sigma_rad.max(1e-9).powi(2)).collect();
    let (mut pos, mut cov) = solve(&w)?;
    for _ in 0..3 {
        w = fixes
            .iter()
            .map(|f| {
                let r = ((pos[0] - f.east_m).powi(2) + (pos[1] - f.north_m).powi(2)).sqrt().max(1.0);
                1.0 / (f.sigma_rad.max(1e-9) * r).powi(2)
            })
            .collect();
        (pos, cov) = solve(&w)?;
    }
    // The fix must lie ahead of every sensor.
    for f in fixes {
        let ahead = f.bearing_rad.sin() * (pos[0] - f.east_m) + f.bearing_rad.cos() * (pos[1] - f.north_m);
        if ahead <= 0.0 {
            return Err(ModelError::NoSolution("bearings diverge".into()));
        }
    }
    // Eigen-decomposition of the 2×2 covariance.
    let (a, b, d) = (cov[0][0], cov[0][1], cov[1][1]);
    let tr = a + d;
    let disc = ((a - d).powi(2) / 4.0 + b * b).sqrt();
    let l1 = tr / 2.0 + disc;
    let l2 = (tr / 2.0 - disc).max(0.0);
    // Major-axis direction (east, north) components.
    let (ve, vn) = if b.abs() > 1e-12 { (l1 - d, b) } else if a >= d { (1.0, 0.0) } else { (0.0, 1.0) };
    let k = (-2.0 * (1.0 - prob.clamp(1e-9, 1.0 - 1e-12)).ln()).sqrt();
    Ok(Fix {
        east_m: pos[0],
        north_m: pos[1],
        cov,
        semi_major_m: k * l1.sqrt(),
        semi_minor_m: k * l2.sqrt(),
        orientation_rad: ve.atan2(vn).rem_euclid(PI),
        prob,
    })
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.esm.sidelobe_db", -25.0, "dB", -45.0, -10.0, "default", Default, "emitter side-lobe level below the main lobe"),
        figure!("sensing.esm.intercept_time_s", 1.0, "s", 0.01, 30.0, "default", Default, "on-air time in view needed to intercept and parametrise"),
        figure!("sensing.df.floor_deg", 1.0, "deg", 0.1, 5.0, "default", Default, "DF system bearing error floor (site, calibration)"),
        figure!("sensing.df.time_to_fix_s", 300.0, "s", 10.0, 3600.0, "P32", Default, "time to build a DF fix: unsourced, wide range"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;

    #[test]
    fn intercept_by_scan_and_band() {
        let p = EsmParams::from_figures(&figs());
        let rx = Receiver { sensitivity_dbw: -100.0, gain_dbi: 0.0, band_lo_hz: 2e9, band_hi_hz: 18e9 };
        let radar = Emitter { erp_dbw: 90.0, freq_hz: 3e9, duty: 1.0, scan: EmitterScan::Rotating { beamwidth_rad: 0.03 }, sidelobe_db: None };
        let near = intercept(&radar, &rx, 150.0, &p);
        assert_eq!(near.illumination, 1.0, "side lobes heard close in");
        let far = intercept(&radar, &rx, 175.0, &p);
        assert_relative_eq!(far.illumination, 0.03 / (2.0 * PI));
        let out = intercept(&Emitter { freq_hz: 0.5e9, ..radar }, &rx, 150.0, &p);
        assert!(!out.in_band && out.rate_per_s == 0.0);
        let quiet = intercept(&Emitter { duty: 0.0, ..radar }, &rx, 150.0, &p);
        assert_eq!(quiet.rate_per_s, 0.0, "EMCON: nothing to hear");
    }

    #[test]
    fn df_error_shrinks_with_baseline_and_snr() {
        let p = EsmParams::from_figures(&figs());
        let a = df_bearing_sigma(50e6, 2.0, 10.0, &p);
        let b = df_bearing_sigma(50e6, 10.0, 10.0, &p);
        let c = df_bearing_sigma(50e6, 10.0, 30.0, &p);
        assert!(a > b && b > c && c >= p.df_floor_rad);
    }

    #[test]
    fn two_bearings_cross_at_the_emitter() {
        let target = (5000.0f64, 8000.0f64);
        let fixes: Vec<BearingFix> = [(0.0, 0.0), (10_000.0, 0.0), (2000.0, -3000.0)]
            .iter()
            .map(|&(e, n)| BearingFix { east_m: e, north_m: n, bearing_rad: (target.0 - e).atan2(target.1 - n), sigma_rad: 1f64.to_radians() })
            .collect();
        let fix = triangulate(&fixes[..2], 0.5).unwrap();
        assert_relative_eq!(fix.east_m, target.0, epsilon = 1e-6);
        assert_relative_eq!(fix.north_m, target.1, epsilon = 1e-6);
        // Error ~ σ·r ≈ 0.017 × 9.4 km → ~100–250 m semi-axes.
        assert!(fix.semi_major_m > 50.0 && fix.semi_major_m < 600.0, "{fix:?}");
        assert!(fix.semi_minor_m <= fix.semi_major_m);
        let three = triangulate(&fixes, 0.5).unwrap();
        assert!(three.semi_major_m <= fix.semi_major_m * 1.0001);
        let parallel = [fixes[0], BearingFix { east_m: 100.0, ..fixes[0] }];
        assert!(triangulate(&parallel, 0.5).is_err());
        assert!(triangulate(&fixes[..1], 0.5).is_err());
    }
}
