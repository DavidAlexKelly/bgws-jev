//! Signatures and deception (RUST_ENGINE.md §8.3).
//!
//! - **Signatures change with action:** moving raises acoustic signature and
//!   lifts dust when the ground is dry; firing makes a muzzle flash and a
//!   thermal bloom that decays over minutes; transmitting makes an RF
//!   signature unless EMCON holds; a running engine heats the hull.
//! - **Camouflage:** standard and multispectral nets reduce visual (and, for
//!   multispectral, IR and radar) contrast, take time to deploy and strike,
//!   and stop or slow movement.
//! - **Decoys** present signatures in chosen bands.
//! - **EMCON:** no RF signature, and no radar picture **[ref R4.4]**.
//! - **Smoke:** Gaussian puffs drifting with the wind, with a ground
//!   reflection term; transmission along a path is exp(−Σ α·CL) with the mass
//!   extinction coefficient α by smoke type and band.

use crate::math::simpson;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// A platform's signature at rest, from the catalogue and the environment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseSignature {
    /// Inherent visual contrast against the background, 0..1.
    pub visual_contrast: f64,
    /// ΔT of the cold hull against the background from solar loading (≈0 at crossover), K.
    pub solar_dt_k: f64,
    /// ΔT of a fully warm engine deck, K.
    pub engine_dt_k: f64,
    /// Idling acoustic source level, dB re 20 µPa at 1 m.
    pub acoustic_idle_db: f64,
    /// Radar cross-section, m².
    pub rcs_m2: f64,
}

/// What the platform is doing this step.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub engine_on: bool,
    pub speed_mps: f64,
    /// Rounds fired during this step.
    pub rounds_fired: u32,
    /// Seconds since the last round (None: never).
    pub since_fired_s: Option<f64>,
    pub transmitting: bool,
    pub emcon: bool,
    pub dry_ground: bool,
}

/// The slow thermal state: engine heat and the barrel/muzzle bloom.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ThermalMemory {
    /// 0 cold … 1 fully warm.
    pub engine_heat: f64,
    /// Extra ΔT from firing, K.
    pub bloom_k: f64,
}

/// The signature in every band this step (rule 10: each part explained).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub visual_contrast: f64,
    /// Multiplier on the presented size (a dust plume is bigger than the vehicle).
    pub visual_size_factor: f64,
    pub dust: bool,
    pub muzzle_flash: bool,
    pub thermal_dt_k: f64,
    pub acoustic_db: f64,
    pub rf_on: bool,
    pub rcs_m2: f64,
}

/// Camouflage net type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetKind {
    /// Visual-band net erected over a halted vehicle.
    Standard,
    /// Multispectral (visual, IR, radar) net erected over a halted vehicle.
    Multispectral,
    /// Fitted multispectral mobile camouflage: works on the move, with a speed cap.
    Mobile,
}

/// The net's state machine: none → deploying → deployed → striking → stowed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CamoState {
    Stowed,
    Deploying { left_s: f64 },
    Deployed,
    Striking { left_s: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camouflage {
    pub kind: NetKind,
    pub state: CamoState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignatureParams {
    pub engine_warm_tau_s: f64,
    pub engine_cool_tau_s: f64,
    pub bloom_per_round_k: f64,
    pub bloom_tau_s: f64,
    pub running_gear_dt_k: f64,
    pub flash_window_s: f64,
    pub acoustic_speed_db: f64,
    pub dust_speed_mps: f64,
    pub dust_size_factor: f64,
    pub dust_contrast: f64,
    pub net_deploy_s: f64,
    pub net_strike_s: f64,
    pub net_visual_reduction: f64,
    pub net_ir_reduction: f64,
    pub net_radar_factor: f64,
    pub mobile_visual_reduction: f64,
    pub mobile_speed_cap: f64,
    pub decoy_visual_fidelity: f64,
    pub decoy_thermal_dt_k: f64,
    pub corner_reflector_rcs_m2: f64,
    pub decoy_rcs_factor: f64,
    pub smoke_alpha: [[f64; 5]; 3],
    pub smoke_sigma0_m: f64,
    pub smoke_growth_h_mps: f64,
    pub smoke_growth_z_mps: f64,
    pub smoke_life_s: f64,
}

impl SignatureParams {
    pub fn from_figures(f: &Figures) -> Self {
        let alpha = SmokeKind::ALL.map(|k| Band::ALL.map(|b| f.get(&format!("signature.smoke.alpha.{}.{}", k.key(), b.key()))));
        SignatureParams {
            engine_warm_tau_s: f.get("signature.engine_warm_tau_s"),
            engine_cool_tau_s: f.get("signature.engine_cool_tau_s"),
            bloom_per_round_k: f.get("signature.bloom_per_round_k"),
            bloom_tau_s: f.get("signature.bloom_tau_s"),
            running_gear_dt_k: f.get("signature.running_gear_dt_k"),
            flash_window_s: f.get("signature.flash_window_s"),
            acoustic_speed_db: f.get("signature.acoustic_speed_db"),
            dust_speed_mps: f.get("signature.dust.speed_mps"),
            dust_size_factor: f.get("signature.dust.size_factor"),
            dust_contrast: f.get("signature.dust.contrast"),
            net_deploy_s: f.get("signature.net.deploy_s"),
            net_strike_s: f.get("signature.net.strike_s"),
            net_visual_reduction: f.get("signature.net.visual_reduction"),
            net_ir_reduction: f.get("signature.net.ir_reduction"),
            net_radar_factor: f.get("signature.net.radar_factor"),
            mobile_visual_reduction: f.get("signature.net.mobile_visual_reduction"),
            mobile_speed_cap: f.get("signature.net.mobile_speed_cap"),
            decoy_visual_fidelity: f.get("signature.decoy.visual_fidelity"),
            decoy_thermal_dt_k: f.get("signature.decoy.thermal_dt_k"),
            corner_reflector_rcs_m2: f.get("signature.decoy.corner_reflector_rcs_m2"),
            decoy_rcs_factor: f.get("signature.decoy.rcs_factor"),
            smoke_alpha: alpha,
            smoke_sigma0_m: f.get("signature.smoke.sigma0_m"),
            smoke_growth_h_mps: f.get("signature.smoke.growth_h_mps"),
            smoke_growth_z_mps: f.get("signature.smoke.growth_z_mps"),
            smoke_life_s: f.get("signature.smoke.life_s"),
        }
    }
}

impl ThermalMemory {
    /// Advance `dt` s: the engine warms or cools exponentially; each round
    /// adds bloom, which decays. Exact for constant inputs over the step.
    pub fn step(&mut self, dt: f64, a: &Activity, p: &SignatureParams) {
        let dt = dt.max(0.0);
        if a.engine_on {
            self.engine_heat = 1.0 - (1.0 - self.engine_heat) * (-dt / p.engine_warm_tau_s).exp();
        } else {
            self.engine_heat *= (-dt / p.engine_cool_tau_s).exp();
        }
        self.bloom_k = self.bloom_k * (-dt / p.bloom_tau_s).exp() + p.bloom_per_round_k * a.rounds_fired as f64;
    }
}

impl Camouflage {
    pub fn new(kind: NetKind) -> Self {
        Camouflage { kind, state: if kind == NetKind::Mobile { CamoState::Deployed } else { CamoState::Stowed } }
    }
    /// Start erecting the net (a no-op unless stowed).
    pub fn deploy(&mut self, p: &SignatureParams) {
        if self.state == CamoState::Stowed {
            self.state = CamoState::Deploying { left_s: p.net_deploy_s };
        }
    }
    /// Start striking the net to move (a no-op for mobile camouflage).
    pub fn strike(&mut self, p: &SignatureParams) {
        if self.kind != NetKind::Mobile && matches!(self.state, CamoState::Deployed | CamoState::Deploying { .. }) {
            self.state = CamoState::Striking { left_s: p.net_strike_s };
        }
    }
    pub fn step(&mut self, dt: f64) {
        self.state = match self.state {
            CamoState::Deploying { left_s } if left_s - dt <= 0.0 => CamoState::Deployed,
            CamoState::Deploying { left_s } => CamoState::Deploying { left_s: left_s - dt },
            CamoState::Striking { left_s } if left_s - dt <= 0.0 => CamoState::Stowed,
            CamoState::Striking { left_s } => CamoState::Striking { left_s: left_s - dt },
            s => s,
        };
    }
    /// Movement allowed, as a fraction of normal speed (0 = must strike first).
    pub fn movement_factor(&self, p: &SignatureParams) -> f64 {
        match (self.kind, self.state) {
            (NetKind::Mobile, _) => p.mobile_speed_cap,
            (_, CamoState::Stowed) => 1.0,
            _ => 0.0,
        }
    }
    /// (visual, IR, radar) multipliers on contrast, ΔT and RCS.
    pub fn factors(&self, p: &SignatureParams) -> (f64, f64, f64) {
        if self.state != CamoState::Deployed {
            return (1.0, 1.0, 1.0);
        }
        match self.kind {
            NetKind::Standard => (1.0 - p.net_visual_reduction, 1.0, 1.0),
            NetKind::Multispectral => (1.0 - p.net_visual_reduction, 1.0 - p.net_ir_reduction, p.net_radar_factor),
            NetKind::Mobile => (1.0 - p.mobile_visual_reduction, 1.0 - p.net_ir_reduction, p.net_radar_factor),
        }
    }
}

/// The signature this step from the base, the activity, the thermal memory and any camouflage.
pub fn signature(base: &BaseSignature, a: &Activity, mem: &ThermalMemory, camo: Option<&Camouflage>, p: &SignatureParams) -> Signature {
    let (fv, fi, fr) = camo.map(|c| c.factors(p)).unwrap_or((1.0, 1.0, 1.0));
    let moving = a.speed_mps > 0.0;
    let dust = a.dry_ground && a.speed_mps >= p.dust_speed_mps;
    let flash = a.rounds_fired > 0 || a.since_fired_s.is_some_and(|s| s <= p.flash_window_s);
    let mut contrast = base.visual_contrast * fv;
    if dust {
        contrast = contrast.max(p.dust_contrast);
    }
    if flash {
        contrast = 1.0;
    }
    let dt = (base.solar_dt_k + base.engine_dt_k * mem.engine_heat + if moving { p.running_gear_dt_k } else { 0.0 }) * fi + mem.bloom_k;
    let acoustic = if a.engine_on { base.acoustic_idle_db + p.acoustic_speed_db * (1.0 + a.speed_mps.max(0.0)).log10() } else { 0.0 };
    Signature {
        visual_contrast: contrast.min(1.0),
        visual_size_factor: if dust { p.dust_size_factor } else { 1.0 },
        dust,
        muzzle_flash: flash,
        thermal_dt_k: dt,
        acoustic_db: acoustic,
        rf_on: a.transmitting && !a.emcon,
        rcs_m2: base.rcs_m2 * fr,
    }
}

/// Decoy types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoyKind {
    /// Inflatable or dummy: visual (and a little radar).
    Inflatable,
    /// Inflatable with heaters: visual and thermal.
    Thermal,
    /// An emitter decoy: RF only.
    Emitter,
    /// A corner reflector: radar only.
    CornerReflector,
}

/// The signature a decoy presents, modelled on the real platform it imitates.
pub fn decoy_signature(kind: DecoyKind, real: &BaseSignature, p: &SignatureParams) -> Signature {
    let mut s = Signature {
        visual_contrast: 0.0,
        visual_size_factor: 1.0,
        dust: false,
        muzzle_flash: false,
        thermal_dt_k: 0.0,
        acoustic_db: 0.0,
        rf_on: false,
        rcs_m2: 0.0,
    };
    match kind {
        DecoyKind::Inflatable => {
            s.visual_contrast = real.visual_contrast * p.decoy_visual_fidelity;
            s.rcs_m2 = real.rcs_m2 * p.decoy_rcs_factor;
        }
        DecoyKind::Thermal => {
            s.visual_contrast = real.visual_contrast * p.decoy_visual_fidelity;
            s.thermal_dt_k = p.decoy_thermal_dt_k;
            s.rcs_m2 = real.rcs_m2 * p.decoy_rcs_factor;
        }
        DecoyKind::Emitter => s.rf_on = true,
        DecoyKind::CornerReflector => s.rcs_m2 = p.corner_reflector_rcs_m2,
    }
    s
}

/// Smoke (obscurant) type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmokeKind {
    /// Hexachloroethane / fog oil: visual screening.
    Hc,
    /// White phosphorus: visual and some IR.
    Wp,
    /// Multispectral (flakes, graphite): visual, IR and some millimetre wave.
    Multispectral,
}

impl SmokeKind {
    const ALL: [SmokeKind; 3] = [SmokeKind::Hc, SmokeKind::Wp, SmokeKind::Multispectral];
    fn key(self) -> &'static str {
        match self {
            SmokeKind::Hc => "hc",
            SmokeKind::Wp => "wp",
            SmokeKind::Multispectral => "multispectral",
        }
    }
}

/// Sensor band for obscuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Visual,
    NearIr,
    MidIr,
    FarIr,
    MmWave,
}

impl Band {
    const ALL: [Band; 5] = [Band::Visual, Band::NearIr, Band::MidIr, Band::FarIr, Band::MmWave];
    fn key(self) -> &'static str {
        match self {
            Band::Visual => "visual",
            Band::NearIr => "near_ir",
            Band::MidIr => "mid_ir",
            Band::FarIr => "far_ir",
            Band::MmWave => "mm_wave",
        }
    }
}

/// One instantaneous release of obscurant (a canister, a grenade, a generator's
/// output over one interval) in a local east/north/up frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmokePuff {
    pub kind: SmokeKind,
    pub mass_g: f64,
    /// Release point (east, north, up), m.
    pub at: [f64; 3],
    pub released_s: f64,
}

/// A smoke cloud: puffs and the wind that carries them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SmokeCloud {
    pub puffs: Vec<SmokePuff>,
    /// Wind (east, north), m/s.
    pub wind: [f64; 2],
}

impl SmokeCloud {
    /// Mass concentration (g/m³) of one kind at a point and time.
    pub fn concentration(&self, kind: SmokeKind, x: [f64; 3], t: f64, p: &SignatureParams) -> f64 {
        self.puffs
            .iter()
            .filter(|q| q.kind == kind && t >= q.released_s)
            .map(|q| {
                let age = t - q.released_s;
                let sh = p.smoke_sigma0_m + p.smoke_growth_h_mps * age;
                let sz = p.smoke_sigma0_m + p.smoke_growth_z_mps * age;
                let m = q.mass_g * (-age / p.smoke_life_s).exp();
                let cx = q.at[0] + self.wind[0] * age;
                let cy = q.at[1] + self.wind[1] * age;
                let h = q.at[2].max(0.0);
                let dh2 = (x[0] - cx).powi(2) + (x[1] - cy).powi(2);
                let z = x[2].max(0.0);
                let vert = (-(z - h).powi(2) / (2.0 * sz * sz)).exp() + (-(z + h).powi(2) / (2.0 * sz * sz)).exp();
                m / ((2.0 * PI).powf(1.5) * sh * sh * sz) * (-dh2 / (2.0 * sh * sh)).exp() * vert
            })
            .sum()
    }

    /// Path-integrated concentration CL (g/m²) of one kind along a → b at time t.
    pub fn path_integral(&self, kind: SmokeKind, a: [f64; 3], b: [f64; 3], t: f64, p: &SignatureParams) -> f64 {
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        if len <= 0.0 {
            return 0.0;
        }
        let at = |s: f64| [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s, a[2] + (b[2] - a[2]) * s];
        let n = ((len / 2.0).ceil() as usize).clamp(32, 2048);
        len * simpson(|s| self.concentration(kind, at(s), t, p), 0.0, 1.0, n)
    }

    /// Transmission along a → b in a band: exp(−Σ_kinds α·CL).
    pub fn transmission(&self, band: Band, a: [f64; 3], b: [f64; 3], t: f64, p: &SignatureParams) -> f64 {
        let od: f64 = SmokeKind::ALL
            .iter()
            .filter(|k| self.puffs.iter().any(|q| q.kind == **k))
            .map(|&k| p.smoke_alpha[k as usize][band as usize] * self.path_integral(k, a, b, t, p))
            .sum();
        (-od).exp()
    }
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("signature.engine_warm_tau_s", 600.0, "s", 60.0, 3600.0, "default", Default, "engine deck warm-up time constant"),
        figure!("signature.engine_cool_tau_s", 1800.0, "s", 300.0, 10800.0, "default", Default, "engine deck cool-down time constant"),
        figure!("signature.bloom_per_round_k", 3.0, "K", 0.5, 10.0, "default", Default, "thermal bloom added per main-gun round"),
        figure!("signature.bloom_tau_s", 180.0, "s", 30.0, 900.0, "default", Default, "barrel/muzzle bloom decay"),
        figure!("signature.running_gear_dt_k", 2.0, "K", 0.0, 10.0, "default", Default, "tracks/wheels friction heat while moving"),
        figure!("signature.flash_window_s", 1.0, "s", 0.1, 5.0, "default", Default, "how long a muzzle flash/blast cue lasts"),
        figure!("signature.acoustic_speed_db", 15.0, "dB", 5.0, 30.0, "default", Default, "acoustic rise per decade of (1 + speed)"),
        figure!("signature.dust.speed_mps", 3.0, "m/s", 1.0, 10.0, "default", Default, "speed above which a vehicle raises dust on dry ground"),
        figure!("signature.dust.size_factor", 4.0, "1", 1.0, 20.0, "default", Default, "dust plume size against the vehicle"),
        figure!("signature.dust.contrast", 0.5, "fraction", 0.1, 1.0, "default", Default, "visual contrast of a dust plume"),
        figure!("signature.net.deploy_s", 600.0, "s", 60.0, 3600.0, "default", Default, "time to erect a camouflage net"),
        figure!("signature.net.strike_s", 300.0, "s", 30.0, 1800.0, "default", Default, "time to strike a camouflage net"),
        figure!("signature.net.visual_reduction", 0.7, "fraction", 0.3, 0.95, "default", Default, "visual contrast removed by a net"),
        figure!("signature.net.ir_reduction", 0.6, "fraction", 0.2, 0.9, "default", Default, "thermal contrast removed by a multispectral net"),
        figure!("signature.net.radar_factor", 0.5, "fraction", 0.1, 1.0, "default", Default, "RCS multiplier under a multispectral net"),
        figure!("signature.net.mobile_visual_reduction", 0.4, "fraction", 0.1, 0.8, "default", Default, "visual contrast removed by fitted mobile camouflage"),
        figure!("signature.net.mobile_speed_cap", 0.7, "fraction", 0.3, 1.0, "default", Default, "speed cap with fitted mobile camouflage"),
        figure!("signature.decoy.visual_fidelity", 0.9, "fraction", 0.3, 1.0, "default", Default, "decoy visual contrast against the real thing"),
        figure!("signature.decoy.thermal_dt_k", 5.0, "K", 0.0, 30.0, "default", Default, "heated decoy ΔT"),
        figure!("signature.decoy.corner_reflector_rcs_m2", 100.0, "m2", 1.0, 10000.0, "default", Default, "corner reflector RCS"),
        figure!("signature.decoy.rcs_factor", 0.1, "fraction", 0.0, 1.0, "default", Default, "inflatable decoy RCS against the real platform"),
        figure!("signature.smoke.sigma0_m", 2.0, "m", 0.5, 10.0, "default", Default, "initial puff size"),
        figure!("signature.smoke.growth_h_mps", 0.5, "m/s", 0.05, 3.0, "default", Default, "horizontal puff growth"),
        figure!("signature.smoke.growth_z_mps", 0.2, "m/s", 0.02, 1.5, "default", Default, "vertical puff growth"),
        figure!("signature.smoke.life_s", 300.0, "s", 30.0, 1800.0, "default", Default, "obscurant mass e-folding time (burn-out, settling)"),
    ];
    // Mass extinction coefficients α (m²/g) by smoke type and band.
    let alpha = [
        ("hc", [4.0, 2.0, 0.3, 0.2, 0.001]),
        ("wp", [4.0, 2.5, 0.6, 0.4, 0.001]),
        ("multispectral", [2.0, 1.8, 1.2, 1.0, 0.05]),
    ];
    for (k, vals) in alpha {
        for (b, a) in Band::ALL.iter().zip(vals) {
            v.push(figure!(format!("signature.smoke.alpha.{k}.{}", b.key()), a, "m2/g", 0.0, 10.0, "default", Default, "smoke mass extinction coefficient"));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;

    fn tank() -> BaseSignature {
        BaseSignature { visual_contrast: 0.3, solar_dt_k: 0.05, engine_dt_k: 8.0, acoustic_idle_db: 95.0, rcs_m2: 20.0 }
    }
    fn still() -> Activity {
        Activity { engine_on: false, speed_mps: 0.0, rounds_fired: 0, since_fired_s: None, transmitting: false, emcon: false, dry_ground: true }
    }

    #[test]
    fn firing_blooms_then_decays() {
        let p = SignatureParams::from_figures(&figs());
        let mut mem = ThermalMemory::default();
        let cold = signature(&tank(), &still(), &mem, None, &p);
        assert!(cold.thermal_dt_k < 0.1, "cold tank at crossover");
        let fire = Activity { rounds_fired: 3, ..still() };
        mem.step(1.0, &fire, &p);
        let hot = signature(&tank(), &fire, &mem, None, &p);
        assert!(hot.thermal_dt_k > 8.0 && hot.muzzle_flash && hot.visual_contrast == 1.0);
        mem.step(900.0, &still(), &p);
        assert!(mem.bloom_k < 0.1);
    }

    #[test]
    fn engine_dust_emcon() {
        let p = SignatureParams::from_figures(&figs());
        let mut mem = ThermalMemory::default();
        let run = Activity { engine_on: true, speed_mps: 8.0, transmitting: true, ..still() };
        mem.step(3600.0, &run, &p);
        let s = signature(&tank(), &run, &mem, None, &p);
        assert!(s.dust && s.visual_size_factor > 1.0 && s.rf_on && s.thermal_dt_k > 8.0);
        let quiet = signature(&tank(), &Activity { emcon: true, ..run }, &mem, None, &p);
        assert!(!quiet.rf_on);
        let wet = signature(&tank(), &Activity { dry_ground: false, ..run }, &mem, None, &p);
        assert!(!wet.dust);
    }

    #[test]
    fn nets_deploy_strike_and_reduce() {
        let p = SignatureParams::from_figures(&figs());
        let mut net = Camouflage::new(NetKind::Multispectral);
        assert_eq!(net.movement_factor(&p), 1.0);
        net.deploy(&p);
        net.step(300.0);
        assert!(matches!(net.state, CamoState::Deploying { .. }));
        assert_eq!(net.movement_factor(&p), 0.0);
        net.step(400.0);
        assert_eq!(net.state, CamoState::Deployed);
        let mem = ThermalMemory { engine_heat: 1.0, bloom_k: 0.0 };
        let s = signature(&tank(), &still(), &mem, Some(&net), &p);
        assert_relative_eq!(s.visual_contrast, 0.3 * 0.3, epsilon = 1e-12);
        assert!(s.thermal_dt_k < 8.0 * 0.5 && s.rcs_m2 < 20.0);
        net.strike(&p);
        net.step(301.0);
        assert_eq!(net.state, CamoState::Stowed);
        assert!(Camouflage::new(NetKind::Mobile).movement_factor(&p) > 0.0);
    }

    #[test]
    fn decoys_by_band() {
        let p = SignatureParams::from_figures(&figs());
        assert!(decoy_signature(DecoyKind::Thermal, &tank(), &p).thermal_dt_k > 0.0);
        assert_eq!(decoy_signature(DecoyKind::Inflatable, &tank(), &p).thermal_dt_k, 0.0);
        assert!(decoy_signature(DecoyKind::Emitter, &tank(), &p).rf_on);
        assert!(decoy_signature(DecoyKind::CornerReflector, &tank(), &p).rcs_m2 > 20.0);
    }

    #[test]
    fn smoke_blocks_visual_more_than_thermal_and_drifts() {
        let p = SignatureParams::from_figures(&figs());
        let mk = |kind| SmokeCloud { puffs: vec![SmokePuff { kind, mass_g: 2000.0, at: [0.0, 0.0, 1.0], released_s: 0.0 }], wind: [2.0, 0.0] };
        let hc = mk(SmokeKind::Hc);
        // Line of sight north–south through where the puff has drifted (60 m east at t = 30 s).
        let (a, b) = ([60.0, -300.0, 2.0], [60.0, 300.0, 2.0]);
        let vis = hc.transmission(Band::Visual, a, b, 30.0, &p);
        let fir = hc.transmission(Band::FarIr, a, b, 30.0, &p);
        assert!(vis < 0.01 && fir > 0.2, "vis {vis} fir {fir}");
        let ms = mk(SmokeKind::Multispectral).transmission(Band::FarIr, a, b, 30.0, &p);
        assert!(ms < fir * 0.2, "multispectral blocks thermal: {ms}");
        // The original position is clear by now.
        let upwind = hc.transmission(Band::Visual, [-60.0, -300.0, 2.0], [-60.0, 300.0, 2.0], 30.0, &p);
        assert!(upwind > 0.9);
        // Mass conservation: ∫C dV over all space ≈ mass × decay.
        let total = simpson(
            |x| simpson(|y| simpson(|z| hc.concentration(SmokeKind::Hc, [x, y, z], 30.0, &p), 0.0, 80.0, 40), -90.0, 90.0, 40),
            -30.0,
            150.0,
            40,
        );
        assert_relative_eq!(total, 2000.0 * (-30.0f64 / 300.0).exp(), max_relative = 0.02);
    }
}
