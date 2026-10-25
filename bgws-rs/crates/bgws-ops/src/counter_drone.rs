//! Counter-drone (RUST_ENGINE.md §8.7; research R5.1–R5.3).
//!
//! - **Detection** by RF detectors (only drones that radiate: fibre FPVs are
//!   invisible to them **[R5.1]**), radar (echo ∝ σ/R⁴), acoustic and EO
//!   (∝ 1/R², visibility). Each is a hazard rate (rule 2) scaled from the
//!   sensor's reference range, so the result doesn't depend on step size.
//! - **Defeat** by jammers (a J/S budget at the drone's receiver, through
//!   [`crate::drones::link_loss_outcome`]: a jammed drone with AI terminal
//!   guidance or a preprogrammed route still comes on), guns and shotguns,
//!   interceptor drones (which slower ones can't catch: jet OWA outruns most
//!   **[R5.2]**), nets and directed energy (dwell to kill ∝ R²).
//! - **Passive measures** (cages, netted roads) don't defeat the drone; they
//!   change the terminal hit and kill-given-hit chances.
//! - [`layered_defence`] runs the layers in order and records which one
//!   stopped the drone; [`approach_kill_rate`] turns a layered defence into
//!   the hazard [`crate::drones::fpv_strike`] uses during the approach.

use crate::drones::{link_loss_outcome, DroneSpec, StrikeParams};
use crate::math::soft_step;
use bgws_core::{figure, hazard_probability, Figure, Figures};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Figures for counter-drone (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("cuas.rf.ref_range_m", 5_000.0, "m", 1_000.0, 20_000.0, "default", Default, "RF detector range at which the rate is the reference rate"),
        figure!("cuas.rf.rate_per_s", 0.5, "1/s", 0.01, 5.0, "default", Default, "RF detector detection rate at reference range"),
        figure!("cuas.radar.ref_range_m", 3_000.0, "m", 500.0, 15_000.0, "default", Default, "C-UAS radar reference range for a 0.01 m² target"),
        figure!("cuas.radar.ref_rcs_m2", 0.01, "m2", 0.001, 1.0, "default", Default, "reference RCS (small quadcopter)"),
        figure!("cuas.radar.rate_per_s", 1.0, "1/s", 0.01, 10.0, "default", Default, "radar detection rate at reference range"),
        figure!("cuas.acoustic.ref_range_m", 500.0, "m", 100.0, 3_000.0, "default", Default, "acoustic sensor reference range"),
        figure!("cuas.acoustic.rate_per_s", 0.3, "1/s", 0.01, 5.0, "default", Default, "acoustic detection rate at reference range"),
        figure!("cuas.eo.ref_range_m", 1_500.0, "m", 200.0, 10_000.0, "default", Default, "EO/IR camera reference range in clear visibility"),
        figure!("cuas.eo.rate_per_s", 0.2, "1/s", 0.01, 5.0, "default", Default, "EO detection rate at reference range"),
        figure!("cuas.jam.js_threshold_db", 0.0, "dB", -10.0, 20.0, "R5.3", Low, "J/S at the drone receiver for 50% control loss"),
        figure!("cuas.jam.width_db", 3.0, "dB", 0.5, 10.0, "default", Default, "control-loss transition width"),
        figure!("cuas.gun.pk_per_s_100m", 0.15, "1/s", 0.0, 2.0, "default", Default, "MG/autocannon kill rate on a small drone at 100 m"),
        figure!("cuas.gun.max_range_m", 800.0, "m", 100.0, 4_000.0, "default", Default, "gun effective range against small drones"),
        figure!("cuas.shotgun.pk_per_shot", 0.3, "fraction", 0.0, 1.0, "default", Default, "shotgun Pk per shot inside its range"),
        figure!("cuas.shotgun.max_range_m", 40.0, "m", 10.0, 80.0, "default", Default, "shotgun effective range"),
        figure!("cuas.shotgun.shots", 3.0, "1", 1.0, 10.0, "default", Default, "shots a shooter gets at a diving FPV"),
        figure!("cuas.interceptor.pk", 0.6, "fraction", 0.0, 1.0, "R5.2", Low, "interceptor drone Pk when it can catch the target"),
        figure!("cuas.interceptor.speed_margin", 1.2, "1", 1.0, 3.0, "default", Default, "speed ratio an interceptor needs to catch a target"),
        figure!("cuas.net.pk", 0.5, "fraction", 0.0, 1.0, "default", Default, "net launcher Pk inside its range"),
        figure!("cuas.net.max_range_m", 50.0, "m", 5.0, 300.0, "default", Default, "net launcher range"),
        figure!("cuas.de.kill_s_1km", 8.0, "s", 0.5, 60.0, "default", Default, "laser dwell to kill a small drone at 1 km"),
        figure!("cuas.de.max_range_m", 2_000.0, "m", 200.0, 10_000.0, "default", Default, "laser effective range"),
        figure!("cuas.cage.kill_factor", 0.5, "fraction", 0.0, 1.0, "default", Default, "kill-given-hit factor with an anti-drone cage"),
        figure!("cuas.cage.hit_factor", 0.9, "fraction", 0.0, 1.0, "default", Default, "hit factor with a cage (it catches some FPVs early)"),
        figure!("cuas.netted_road.hit_factor", 0.4, "fraction", 0.0, 1.0, "default", Default, "terminal hit factor on a netted road"),
    ]
}

/// Counter-drone sensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuasSensor {
    RfDetector,
    Radar,
    Acoustic,
    Eo,
}

/// The drone as a detection target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectionInput {
    pub range_m: f64,
    /// Radiating (RF link): false for fibre and silent preprogrammed flight.
    pub rf_emitting: bool,
    pub rcs_m2: f64,
    /// Relative acoustic level (1 = small quadcopter; OWA engines louder).
    pub acoustic_level: f64,
    /// 0–1 optical visibility (night, fog, smoke).
    pub visibility: f64,
}

/// Detection by a set of sensors over `dt` (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectionResult {
    pub rates: Vec<(CuasSensor, f64)>,
    pub total_rate: f64,
    pub p_detect: f64,
}

/// Detection hazard rate of one sensor, 1/s.
pub fn detection_rate(s: CuasSensor, i: &DetectionInput, f: &Figures) -> f64 {
    let r = i.range_m.max(1.0);
    match s {
        CuasSensor::RfDetector if i.rf_emitting => f.get("cuas.rf.rate_per_s") * (f.get("cuas.rf.ref_range_m") / r).powi(2),
        CuasSensor::RfDetector => 0.0,
        CuasSensor::Radar => f.get("cuas.radar.rate_per_s") * (f.get("cuas.radar.ref_range_m") / r).powi(4) * i.rcs_m2 / f.get("cuas.radar.ref_rcs_m2"),
        CuasSensor::Acoustic => f.get("cuas.acoustic.rate_per_s") * (f.get("cuas.acoustic.ref_range_m") / r).powi(2) * i.acoustic_level,
        CuasSensor::Eo => f.get("cuas.eo.rate_per_s") * (f.get("cuas.eo.ref_range_m") / r).powi(2) * i.visibility.clamp(0.0, 1.0),
    }
}

/// Combined detection over `dt` seconds.
pub fn detect(sensors: &[CuasSensor], i: &DetectionInput, dt: f64, f: &Figures) -> DetectionResult {
    let rates: Vec<(CuasSensor, f64)> = sensors.iter().map(|s| (*s, detection_rate(*s, i, f))).collect();
    let total: f64 = rates.iter().map(|r| r.1).sum();
    DetectionResult { rates, total_rate: total, p_detect: hazard_probability(total, dt) }
}

/// Active defeat means.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DefeatMeans {
    /// J/S at the drone's control receiver, from [`crate::ew`].
    Jammer { js_db: f64 },
    Gun,
    Shotgun,
    InterceptorDrone { speed_mps: f64 },
    Net,
    DirectedEnergy,
}

/// Passive measures at the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassiveMeasure {
    Cage,
    NettedRoad,
}

/// One engagement of a drone by one means.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Engagement {
    pub drone: DroneSpec,
    /// Closest range during the engagement.
    pub range_m: f64,
    /// Seconds the drone is engageable.
    pub exposure_s: f64,
    pub target_speed_mps: f64,
}

/// Result of one defeat means (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DefeatResult {
    pub means: DefeatMeans,
    pub p_defeat: f64,
    pub note: String,
}

/// Probability one means defeats the drone in this engagement.
pub fn defeat(means: DefeatMeans, e: &Engagement, f: &Figures) -> DefeatResult {
    let (p, note) = match means {
        DefeatMeans::Jammer { js_db } => {
            if !e.drone.jammable() {
                (0.0, "not RF-controlled: jamming has no effect".to_string())
            } else {
                let p_loss = soft_step(js_db, f.get("cuas.jam.js_threshold_db"), f.get("cuas.jam.width_db"));
                let sp = StrikeParams::from_figures(f);
                let after = link_loss_outcome(&e.drone, e.target_speed_mps, &sp);
                // Defeat only counts if the drone can no longer strike.
                (p_loss * (1.0 - after.hit_factor), format!("control lost {:.2}; {:?} keeps {:.2} of the strike", p_loss, after.behaviour, after.hit_factor))
            }
        }
        DefeatMeans::Gun => {
            if e.range_m > f.get("cuas.gun.max_range_m") {
                (0.0, "out of gun range".into())
            } else {
                let rate = f.get("cuas.gun.pk_per_s_100m") * (100.0 / e.range_m.max(10.0)).powi(2).min(4.0);
                (hazard_probability(rate, e.exposure_s), format!("kill rate {rate:.3}/s"))
            }
        }
        DefeatMeans::Shotgun => {
            if e.range_m > f.get("cuas.shotgun.max_range_m") {
                (0.0, "out of shotgun range".into())
            } else {
                let p1 = f.get("cuas.shotgun.pk_per_shot");
                let n = f.get("cuas.shotgun.shots");
                (1.0 - (1.0 - p1).powf(n), format!("{n} shots at {p1}"))
            }
        }
        DefeatMeans::InterceptorDrone { speed_mps } => {
            if speed_mps < f.get("cuas.interceptor.speed_margin") * e.drone.speed_mps {
                (0.0, "target outruns the interceptor".into())
            } else {
                (f.get("cuas.interceptor.pk"), "interceptor catches it".into())
            }
        }
        DefeatMeans::Net => {
            if e.range_m > f.get("cuas.net.max_range_m") {
                (0.0, "out of net range".into())
            } else {
                (f.get("cuas.net.pk"), "net".into())
            }
        }
        DefeatMeans::DirectedEnergy => {
            if e.range_m > f.get("cuas.de.max_range_m") {
                (0.0, "out of laser range".into())
            } else {
                let kill_s = f.get("cuas.de.kill_s_1km") * (e.range_m / 1000.0).powi(2);
                (hazard_probability(1.0 / kill_s.max(1e-3), e.exposure_s), format!("dwell to kill {kill_s:.1} s"))
            }
        }
    };
    DefeatResult { means, p_defeat: p.clamp(0.0, 1.0), note }
}

/// Terminal factors from passive measures (on hit, on kill-given-hit).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalModifiers {
    pub hit_factor: f64,
    pub kill_factor: f64,
}

pub fn passive_modifiers(measures: &[PassiveMeasure], f: &Figures) -> TerminalModifiers {
    let mut m = TerminalModifiers { hit_factor: 1.0, kill_factor: 1.0 };
    for p in measures {
        match p {
            PassiveMeasure::Cage => {
                m.hit_factor *= f.get("cuas.cage.hit_factor");
                m.kill_factor *= f.get("cuas.cage.kill_factor");
            }
            PassiveMeasure::NettedRoad => m.hit_factor *= f.get("cuas.netted_road.hit_factor"),
        }
    }
    m
}

/// A layered defence's outcome (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayeredOutcome {
    pub layers: Vec<DefeatResult>,
    /// Which layer stopped it, if any.
    pub defeated_by: Option<usize>,
    /// Analytic probability the drone gets through every layer.
    pub p_leak: f64,
}

/// Run each layer in order; a draw per layer.
pub fn layered_defence(layers: &[(DefeatMeans, Engagement)], f: &Figures, rng: &mut impl Rng) -> LayeredOutcome {
    let results: Vec<DefeatResult> = layers.iter().map(|(m, e)| defeat(*m, e, f)).collect();
    let p_leak = results.iter().map(|r| 1.0 - r.p_defeat).product();
    let defeated_by = results.iter().position(|r| rng.gen::<f64>() < r.p_defeat);
    LayeredOutcome { layers: results, defeated_by, p_leak }
}

/// The constant hazard over `exposure_s` equivalent to a set of layers, for
/// the approach stage of [`crate::drones::fpv_strike`].
pub fn approach_kill_rate(layers: &[(DefeatMeans, Engagement)], exposure_s: f64, f: &Figures) -> f64 {
    let p_leak: f64 = layers.iter().map(|(m, e)| 1.0 - defeat(*m, e, f).p_defeat).product();
    bgws_core::rate_from_probability(1.0 - p_leak, exposure_s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drones::DroneType;
    use approx::assert_relative_eq;
    use bgws_core::{Seed, StreamKey};

    fn figs() -> Figures {
        crate::default_figures()
    }

    fn eng(kind: DroneType, range: f64, f: &Figures) -> Engagement {
        Engagement { drone: DroneSpec::from_figures(kind, f), range_m: range, exposure_s: 10.0, target_speed_mps: 0.0 }
    }

    #[test]
    fn rf_detectors_cannot_see_fibre() {
        let f = figs();
        let rf = DetectionInput { range_m: 2_000.0, rf_emitting: true, rcs_m2: 0.01, acoustic_level: 1.0, visibility: 1.0 };
        let fibre = DetectionInput { rf_emitting: false, ..rf };
        assert!(detection_rate(CuasSensor::RfDetector, &rf, &f) > 0.0);
        assert_eq!(detection_rate(CuasSensor::RfDetector, &fibre, &f), 0.0);
        // Radar falls as R^-4.
        let near = detection_rate(CuasSensor::Radar, &DetectionInput { range_m: 1_000.0, ..rf }, &f);
        assert_relative_eq!(near / detection_rate(CuasSensor::Radar, &rf, &f), 16.0, epsilon = 1e-9);
        let all = detect(&[CuasSensor::RfDetector, CuasSensor::Radar, CuasSensor::Acoustic, CuasSensor::Eo], &rf, 5.0, &f);
        let none_rf = detect(&[CuasSensor::RfDetector, CuasSensor::Radar, CuasSensor::Acoustic, CuasSensor::Eo], &fibre, 5.0, &f);
        assert!(all.p_detect > none_rf.p_detect);
        // Step-size independent.
        let one = detect(&[CuasSensor::Radar], &rf, 10.0, &f).p_detect;
        let two = 1.0 - (1.0 - detect(&[CuasSensor::Radar], &rf, 5.0, &f).p_detect).powi(2);
        assert_relative_eq!(one, two, epsilon = 1e-12);
    }

    #[test]
    fn jammer_defeat_depends_on_link_and_autonomy() {
        let f = figs();
        let strong = DefeatMeans::Jammer { js_db: 15.0 };
        assert!(defeat(strong, &eng(DroneType::FpvRf, 300.0, &f), &f).p_defeat > 0.95);
        assert_eq!(defeat(strong, &eng(DroneType::FpvFibre, 300.0, &f), &f).p_defeat, 0.0);
        assert_eq!(defeat(strong, &eng(DroneType::OwaShahed, 300.0, &f), &f).p_defeat, 0.0);
        let mut ai = eng(DroneType::FpvRf, 300.0, &f);
        ai.drone = ai.drone.with_ai_terminal();
        assert!(defeat(strong, &ai, &f).p_defeat < 0.5);
        assert!(defeat(DefeatMeans::Jammer { js_db: -10.0 }, &eng(DroneType::FpvRf, 300.0, &f), &f).p_defeat < 0.05);
    }

    #[test]
    fn kinetic_means_and_passive_measures() {
        let f = figs();
        assert!(defeat(DefeatMeans::Shotgun, &eng(DroneType::FpvRf, 30.0, &f), &f).p_defeat > 0.6);
        assert_eq!(defeat(DefeatMeans::Shotgun, &eng(DroneType::FpvRf, 100.0, &f), &f).p_defeat, 0.0);
        assert!(defeat(DefeatMeans::Gun, &eng(DroneType::FpvRf, 100.0, &f), &f).p_defeat > defeat(DefeatMeans::Gun, &eng(DroneType::FpvRf, 400.0, &f), &f).p_defeat);
        // A slow interceptor can't catch a Shahed; a fast one can.
        assert_eq!(defeat(DefeatMeans::InterceptorDrone { speed_mps: 50.0 }, &eng(DroneType::OwaShahed, 500.0, &f), &f).p_defeat, 0.0);
        assert!(defeat(DefeatMeans::InterceptorDrone { speed_mps: 70.0 }, &eng(DroneType::OwaShahed, 500.0, &f), &f).p_defeat > 0.5);
        assert!(defeat(DefeatMeans::DirectedEnergy, &eng(DroneType::FpvRf, 500.0, &f), &f).p_defeat > defeat(DefeatMeans::DirectedEnergy, &eng(DroneType::FpvRf, 1_500.0, &f), &f).p_defeat);
        assert!(defeat(DefeatMeans::Net, &eng(DroneType::FpvRf, 20.0, &f), &f).p_defeat > 0.0);
        let m = passive_modifiers(&[PassiveMeasure::Cage, PassiveMeasure::NettedRoad], &f);
        assert!(m.hit_factor < 0.5 && m.kill_factor < 1.0);
    }

    #[test]
    fn layers_compose() {
        let f = figs();
        let layers = vec![(DefeatMeans::Jammer { js_db: 0.0 }, eng(DroneType::FpvRf, 500.0, &f)), (DefeatMeans::Shotgun, eng(DroneType::FpvRf, 30.0, &f))];
        let mut r = Seed(3).stream(StreamKey::new("t", 0, 0, 0));
        let o = layered_defence(&layers, &f, &mut r);
        let p0 = o.layers[0].p_defeat;
        let p1 = o.layers[1].p_defeat;
        assert_relative_eq!(o.p_leak, (1.0 - p0) * (1.0 - p1), epsilon = 1e-12);
        let rate = approach_kill_rate(&layers, 60.0, &f);
        assert_relative_eq!(hazard_probability(rate, 60.0), 1.0 - o.p_leak, epsilon = 1e-9);
    }
}
