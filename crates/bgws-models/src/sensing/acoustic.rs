//! Battlefield acoustic sensing in air (§8.2): gunfire, drones and vehicles
//! heard by microphone arrays or by people, through the same passive
//! equation as sonar: SE = SL − TL − (NL − DI) − DT, Pd from a normal
//! transition. TL (spreading, atmospheric absorption, ground and wind
//! excess attenuation) is an input from the propagation crate; `spherical_tl`
//! is a fallback for tests. Levels in dB re 20 µPa (band levels).

use super::pd_from_signal_excess;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// What makes the noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcousticSource {
    RifleShot,
    MachineGunBurst,
    ArtilleryFiring,
    MortarFiring,
    SmallMultirotor,
    FpvDrone,
    PistonDrone,
    TrackedVehicle,
    WheeledVehicle,
}

impl AcousticSource {
    const ALL: [AcousticSource; 9] = [
        AcousticSource::RifleShot,
        AcousticSource::MachineGunBurst,
        AcousticSource::ArtilleryFiring,
        AcousticSource::MortarFiring,
        AcousticSource::SmallMultirotor,
        AcousticSource::FpvDrone,
        AcousticSource::PistonDrone,
        AcousticSource::TrackedVehicle,
        AcousticSource::WheeledVehicle,
    ];
    fn key(self) -> &'static str {
        match self {
            AcousticSource::RifleShot => "rifle_shot",
            AcousticSource::MachineGunBurst => "mg_burst",
            AcousticSource::ArtilleryFiring => "artillery",
            AcousticSource::MortarFiring => "mortar",
            AcousticSource::SmallMultirotor => "small_multirotor",
            AcousticSource::FpvDrone => "fpv",
            AcousticSource::PistonDrone => "piston_drone",
            AcousticSource::TrackedVehicle => "tracked",
            AcousticSource::WheeledVehicle => "wheeled",
        }
    }
    /// Impulsive sources (a shot) are detected per event, not continuously.
    pub fn impulsive(self) -> bool {
        matches!(self, AcousticSource::RifleShot | AcousticSource::MachineGunBurst | AcousticSource::ArtilleryFiring | AcousticSource::MortarFiring)
    }
}

/// Background noise class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcousticAmbient {
    QuietRural,
    Battlefield,
    Urban,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcousticParams {
    pub source_db: [f64; 9],
    pub ambient_db: [f64; 3],
    pub dt_db: f64,
    pub sigma_db: f64,
    pub wind_noise_db_per_mps: f64,
}

impl AcousticParams {
    pub fn from_figures(f: &Figures) -> Self {
        AcousticParams {
            source_db: AcousticSource::ALL.map(|s| f.get(&format!("sensing.acoustic.sl_db.{}", s.key()))),
            ambient_db: ["quiet_rural", "battlefield", "urban"].map(|a| f.get(&format!("sensing.acoustic.ambient_db.{a}"))),
            dt_db: f.get("sensing.acoustic.dt_db"),
            sigma_db: f.get("sensing.acoustic.sigma_db"),
            wind_noise_db_per_mps: f.get("sensing.acoustic.wind_noise_db_per_mps"),
        }
    }
}

/// Spherical spreading only: TL = 20 log10(r / 1 m). A fallback; real TL comes from §8.4.
pub fn spherical_tl(range_m: f64) -> f64 {
    20.0 * range_m.max(1.0).log10()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcousticResult {
    pub source_db: f64,
    pub noise_db: f64,
    pub signal_excess_db: f64,
    pub pd: f64,
}

/// Does a sensor with directivity `di_db` hear `source` through `tl_db` in
/// this ambient (with wind-induced microphone noise)?
pub fn hear(source: AcousticSource, tl_db: f64, ambient: AcousticAmbient, wind_mps: f64, di_db: f64, p: &AcousticParams) -> AcousticResult {
    let sl = p.source_db[source as usize];
    let nl = p.ambient_db[ambient as usize] + p.wind_noise_db_per_mps * wind_mps.max(0.0);
    let se = sl - tl_db - (nl - di_db) - p.dt_db;
    AcousticResult { source_db: sl, noise_db: nl, signal_excess_db: se, pd: pd_from_signal_excess(se, p.sigma_db) }
}

pub fn figures() -> Vec<Figure> {
    let sl = [
        ("rifle_shot", 155.0, 140.0, 170.0, "rifle muzzle blast at 1 m"),
        ("mg_burst", 158.0, 140.0, 170.0, "machine-gun burst at 1 m"),
        ("artillery", 185.0, 170.0, 200.0, "155 mm gun firing at 1 m"),
        ("mortar", 170.0, 155.0, 185.0, "mortar firing at 1 m"),
        ("small_multirotor", 80.0, 65.0, 95.0, "small quadcopter at 1 m"),
        ("fpv", 88.0, 75.0, 100.0, "FPV racer quad at 1 m"),
        ("piston_drone", 105.0, 90.0, 120.0, "piston-engined one-way drone at 1 m"),
        ("tracked", 110.0, 95.0, 125.0, "tracked vehicle moving at 1 m"),
        ("wheeled", 100.0, 85.0, 115.0, "wheeled vehicle moving at 1 m"),
    ];
    let mut v: Vec<Figure> = sl
        .iter()
        .map(|(k, val, lo, hi, note)| figure!(format!("sensing.acoustic.sl_db.{k}"), *val, "dB", *lo, *hi, "default", Default, *note))
        .collect();
    v.extend([
        figure!("sensing.acoustic.ambient_db.quiet_rural", 35.0, "dB", 20.0, 50.0, "default", Default, "rural night ambient band level"),
        figure!("sensing.acoustic.ambient_db.battlefield", 65.0, "dB", 45.0, 90.0, "default", Default, "active battlefield ambient"),
        figure!("sensing.acoustic.ambient_db.urban", 60.0, "dB", 45.0, 80.0, "default", Default, "urban ambient"),
        figure!("sensing.acoustic.dt_db", 10.0, "dB", 0.0, 20.0, "default", Default, "detection threshold"),
        figure!("sensing.acoustic.sigma_db", 5.0, "dB", 2.0, 10.0, "default", Default, "SE→Pd transition width"),
        figure!("sensing.acoustic.wind_noise_db_per_mps", 1.5, "dB", 0.0, 5.0, "default", Default, "microphone wind noise per m/s"),
    ]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;

    #[test]
    fn drones_heard_close_guns_far() {
        let p = AcousticParams::from_figures(&figs());
        let quad_near = hear(AcousticSource::SmallMultirotor, spherical_tl(50.0), AcousticAmbient::QuietRural, 0.0, 0.0, &p);
        let quad_far = hear(AcousticSource::SmallMultirotor, spherical_tl(1000.0), AcousticAmbient::Battlefield, 0.0, 0.0, &p);
        assert!(quad_near.pd > 0.5 && quad_far.pd < 0.05, "{quad_near:?} {quad_far:?}");
        let gun = hear(AcousticSource::ArtilleryFiring, spherical_tl(10_000.0) + 15.0, AcousticAmbient::Battlefield, 5.0, 10.0, &p);
        assert!(gun.pd > 0.9, "{gun:?}");
        assert!(AcousticSource::RifleShot.impulsive() && !AcousticSource::FpvDrone.impulsive());
    }
}
