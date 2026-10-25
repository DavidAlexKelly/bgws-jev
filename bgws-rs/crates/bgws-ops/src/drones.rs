//! Drones (RUST_ENGINE.md §8.7; research R5.1, R5.2, P8, P9).
//!
//! - **Types:** FPV (RF or fibre), bomber and reconnaissance multirotors,
//!   fixed-wing ISR, loitering munitions, Shahed-type one-way attack and
//!   interceptor drones **[R5.1, R5.2]**, each a [`DroneSpec`] from figures.
//! - **The drone fight is a link fight:** an RF drone needs its control
//!   uplink (at the drone) and video downlink (at the operator) against the
//!   jamming at both ends ([`assess_link`]); fibre ignores RF jamming but is
//!   capped by its spool and can snag ([`fibre_check`]).
//! - **Link loss** triggers the drone's autonomy: return, hover, continue to
//!   the last aim point, or AI terminal guidance if fitted ([`link_loss_outcome`]).
//! - **The FPV strike chain** ([`fpv_strike`]): find (camera search), approach
//!   (counter-drone fire and the link holding), terminal (operator skill and
//!   fatigue, target motion, wind, link quality, passive measures), effect
//!   (kill given hit by target class). Overall hit rates of 30–50% (≈10% for
//!   novices) and 60–80% of hits failing to destroy **[P9]** are Monte Carlo
//!   acceptance tests here, not inputs.
//! - **Stores and operators:** drones and batteries are stores; operators
//!   have skill and fatigue.

use crate::math::soft_step;
use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Drone types (§8.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DroneType {
    FpvRf,
    FpvFibre,
    Bomber,
    ReconMultirotor,
    FixedWingIsr,
    LoiteringMunition,
    OwaShahed,
    Interceptor,
}

impl DroneType {
    pub const ALL: [DroneType; 8] = [
        DroneType::FpvRf,
        DroneType::FpvFibre,
        DroneType::Bomber,
        DroneType::ReconMultirotor,
        DroneType::FixedWingIsr,
        DroneType::LoiteringMunition,
        DroneType::OwaShahed,
        DroneType::Interceptor,
    ];

    pub fn key(self) -> &'static str {
        match self {
            DroneType::FpvRf => "fpv_rf",
            DroneType::FpvFibre => "fpv_fibre",
            DroneType::Bomber => "bomber",
            DroneType::ReconMultirotor => "recon",
            DroneType::FixedWingIsr => "fixed_wing_isr",
            DroneType::LoiteringMunition => "loitering",
            DroneType::OwaShahed => "owa",
            DroneType::Interceptor => "interceptor",
        }
    }
}

/// Target classes for kill-given-hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetClass {
    Mbt,
    Ifv,
    Truck,
    Artillery,
    InfantryOpen,
    InfantryCover,
}

impl TargetClass {
    pub const ALL: [TargetClass; 6] = [TargetClass::Mbt, TargetClass::Ifv, TargetClass::Truck, TargetClass::Artillery, TargetClass::InfantryOpen, TargetClass::InfantryCover];
    pub fn key(self) -> &'static str {
        match self {
            TargetClass::Mbt => "mbt",
            TargetClass::Ifv => "ifv",
            TargetClass::Truck => "truck",
            TargetClass::Artillery => "artillery",
            TargetClass::InfantryOpen => "infantry_open",
            TargetClass::InfantryCover => "infantry_cover",
        }
    }
}

/// Figures for drones (rule 3).
pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        // speed m/s, endurance s, range m by type.
        figure!("drones.fpv_rf.speed_mps", 25.0, "m/s", 10.0, 50.0, "default", Default, "FPV cruise speed"),
        figure!("drones.fpv_rf.endurance_s", 600.0, "s", 180.0, 1_800.0, "default", Default, "FPV battery endurance"),
        figure!("drones.fpv_rf.range_m", 10_000.0, "m", 2_000.0, 25_000.0, "P8", Medium, "RF FPV control range; kill zone 5–20 km"),
        figure!("drones.fpv_fibre.speed_mps", 20.0, "m/s", 10.0, 40.0, "default", Default, "fibre FPV cruise speed"),
        figure!("drones.fpv_fibre.endurance_s", 900.0, "s", 180.0, 1_800.0, "default", Default, "fibre FPV endurance"),
        figure!("drones.fpv_fibre.range_m", 15_000.0, "m", 5_000.0, 41_000.0, "P8", Medium, "spool length: ≤20 km typical, 41 km max reported"),
        figure!("drones.bomber.speed_mps", 15.0, "m/s", 5.0, 30.0, "default", Default, "heavy bomber multirotor speed"),
        figure!("drones.bomber.endurance_s", 1_800.0, "s", 600.0, 3_600.0, "default", Default, "bomber endurance"),
        figure!("drones.bomber.range_m", 10_000.0, "m", 2_000.0, 20_000.0, "R7", Low, "'the only viable way to resupply within 10 km'"),
        figure!("drones.recon.speed_mps", 15.0, "m/s", 5.0, 25.0, "default", Default, "Mavic-class speed"),
        figure!("drones.recon.endurance_s", 1_800.0, "s", 900.0, 3_000.0, "default", Default, "Mavic-class endurance"),
        figure!("drones.recon.range_m", 5_000.0, "m", 1_000.0, 15_000.0, "default", Default, "Mavic-class link range"),
        figure!("drones.fixed_wing_isr.speed_mps", 30.0, "m/s", 20.0, 60.0, "default", Default, "Orlan-class speed"),
        figure!("drones.fixed_wing_isr.endurance_s", 36_000.0, "s", 7_200.0, 72_000.0, "default", Default, "Orlan-class endurance"),
        figure!("drones.fixed_wing_isr.range_m", 120_000.0, "m", 50_000.0, 250_000.0, "default", Default, "Orlan-class link range"),
        figure!("drones.loitering.speed_mps", 30.0, "m/s", 20.0, 60.0, "default", Default, "Lancet-class speed"),
        figure!("drones.loitering.endurance_s", 2_400.0, "s", 900.0, 3_600.0, "R5.1", Low, "Switchblade 600 ~40 min"),
        figure!("drones.loitering.range_m", 40_000.0, "m", 10_000.0, 80_000.0, "R5.1", Low, "Lancet-3 / Switchblade 600 ~40 km"),
        figure!("drones.owa.speed_mps", 50.0, "m/s", 40.0, 170.0, "R5.2", Low, "Shahed ~185 km/h (jet Geran 500–600 km/h)"),
        figure!("drones.owa.endurance_s", 36_000.0, "s", 10_000.0, 60_000.0, "default", Default, "Shahed endurance"),
        figure!("drones.owa.range_m", 1_500_000.0, "m", 500_000.0, 2_500_000.0, "default", Default, "Shahed range"),
        figure!("drones.interceptor.speed_mps", 70.0, "m/s", 40.0, 120.0, "R5.2", Low, "interceptor drone speed; jet OWA outruns most"),
        figure!("drones.interceptor.endurance_s", 600.0, "s", 180.0, 1_800.0, "default", Default, "interceptor endurance"),
        figure!("drones.interceptor.range_m", 10_000.0, "m", 2_000.0, 30_000.0, "default", Default, "interceptor control range"),
        // Links.
        figure!("drones.link.control_sinr_db", 6.0, "dB", 0.0, 15.0, "default", Default, "control uplink SINR for 50% usable"),
        figure!("drones.link.video_sinr_db", 10.0, "dB", 3.0, 20.0, "default", Default, "analogue video downlink SINR for 50% usable"),
        figure!("drones.link.width_db", 2.0, "dB", 0.5, 6.0, "default", Default, "link usability transition width"),
        figure!("drones.link.coherence_s", 20.0, "s", 2.0, 120.0, "default", Default, "time scale over which link quality decorrelates"),
        figure!("drones.fibre.snag_per_km", 0.02, "1/km", 0.0, 0.2, "R5.1", Low, "fibre snag/break hazard per km paid out"),
        // Strike chain.
        figure!("drones.find.rate_per_s", 0.02, "1/s", 0.001, 0.1, "default", Default, "camera search rate at skill 1 vs an unconcealed target"),
        figure!("drones.find.search_s", 180.0, "s", 30.0, 900.0, "default", Default, "search time an FPV can spend over the target area"),
        figure!("drones.terminal.p_skilled", 0.80, "fraction", 0.5, 0.95, "P9", Medium, "terminal hit chance, skilled pilot, static target, calm, good link"),
        figure!("drones.terminal.p_novice", 0.25, "fraction", 0.05, 0.4, "P9", Medium, "terminal hit chance, novice pilot, same conditions"),
        figure!("drones.terminal.motion_per_mps", 0.08, "1/(m/s)", 0.0, 0.3, "default", Default, "exp. fall of terminal hit chance per m/s of target speed"),
        figure!("drones.terminal.wind_per_mps", 0.03, "1/(m/s)", 0.0, 0.2, "default", Default, "exp. fall of terminal hit chance per m/s of wind"),
        figure!("drones.terminal.link_exponent", 1.0, "1", 0.0, 3.0, "default", Default, "terminal hit ∝ link quality^k"),
        figure!("drones.terminal.blind_per_mps", 0.5, "1/(m/s)", 0.0, 2.0, "default", Default, "continue-to-aim-point: exp. fall per m/s of target speed"),
        figure!("drones.terminal.ai_factor", 0.6, "fraction", 0.0, 1.0, "default", Default, "AI terminal guidance hit chance relative to a skilled pilot"),
        figure!("drones.operator.fatigue_penalty", 0.5, "fraction", 0.0, 1.0, "P31", Low, "skill lost at full fatigue"),
        figure!("drones.operator.fatigue_per_h", 0.06, "1/h", 0.0, 0.3, "P31", Low, "fatigue accumulated per hour flying (onset 24–48 h scale)"),
        figure!("drones.operator.recovery_per_h", 0.12, "1/h", 0.0, 0.5, "default", Default, "fatigue recovered per hour of rest"),
        figure!("drones.batteries_per_sortie", 1.0, "1", 1.0, 4.0, "default", Default, "battery packs used per multirotor sortie"),
    ];
    // Kill given hit by target class, for an RPG-warhead FPV.
    for (t, p, note) in [
        (TargetClass::Mbt, 0.10, "MBT: most hits on armour do not destroy"),
        (TargetClass::Ifv, 0.35, "IFV/APC"),
        (TargetClass::Truck, 0.60, "soft-skinned vehicle"),
        (TargetClass::Artillery, 0.40, "towed or SP gun"),
        (TargetClass::InfantryOpen, 0.50, "infantry in the open: a casualty"),
        (TargetClass::InfantryCover, 0.20, "infantry in trench/building"),
    ] {
        v.push(figure!(format!("drones.kill_given_hit.{}", t.key()), p, "fraction", 0.0, 1.0, "default", Default, note));
    }
    v
}

/// How a drone is controlled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlLink {
    Rf,
    Fibre { spool_m: f64 },
    /// Preprogrammed route on GNSS/INS (one-way attack).
    Preprogrammed,
}

/// What a drone does when its link drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkLossBehaviour {
    ReturnHome,
    Hover,
    ContinueToAimPoint,
    AiTerminal,
}

/// A drone type's performance record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DroneSpec {
    pub kind: DroneType,
    pub speed_mps: f64,
    pub endurance_s: f64,
    pub range_m: f64,
    pub link: ControlLink,
    pub on_link_loss: LinkLossBehaviour,
    /// Expended on use (FPV, loitering, OWA, interceptor).
    pub expendable: bool,
    pub carries_warhead: bool,
}

impl DroneSpec {
    pub fn from_figures(kind: DroneType, f: &Figures) -> Self {
        let k = kind.key();
        let range = f.get(&format!("drones.{k}.range_m"));
        let (link, loss, expendable, warhead) = match kind {
            DroneType::FpvRf => (ControlLink::Rf, LinkLossBehaviour::Hover, true, true),
            DroneType::FpvFibre => (ControlLink::Fibre { spool_m: range }, LinkLossBehaviour::Hover, true, true),
            DroneType::Bomber => (ControlLink::Rf, LinkLossBehaviour::ReturnHome, false, true),
            DroneType::ReconMultirotor => (ControlLink::Rf, LinkLossBehaviour::ReturnHome, false, false),
            DroneType::FixedWingIsr => (ControlLink::Rf, LinkLossBehaviour::ReturnHome, false, false),
            DroneType::LoiteringMunition => (ControlLink::Rf, LinkLossBehaviour::ContinueToAimPoint, true, true),
            DroneType::OwaShahed => (ControlLink::Preprogrammed, LinkLossBehaviour::ContinueToAimPoint, true, true),
            DroneType::Interceptor => (ControlLink::Rf, LinkLossBehaviour::Hover, true, true),
        };
        DroneSpec {
            kind,
            speed_mps: f.get(&format!("drones.{k}.speed_mps")),
            endurance_s: f.get(&format!("drones.{k}.endurance_s")),
            range_m: range,
            link,
            on_link_loss: loss,
            expendable,
            carries_warhead: warhead,
        }
    }

    /// Fit AI terminal guidance.
    pub fn with_ai_terminal(mut self) -> Self {
        self.on_link_loss = LinkLossBehaviour::AiTerminal;
        self
    }

    /// Whether RF jamming can touch it.
    pub fn jammable(&self) -> bool {
        matches!(self.link, ControlLink::Rf)
    }
}

/// Link parameters from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkParams {
    pub control_sinr_db: f64,
    pub video_sinr_db: f64,
    pub width_db: f64,
    pub coherence_s: f64,
    pub snag_per_km: f64,
}

impl LinkParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            control_sinr_db: f.get("drones.link.control_sinr_db"),
            video_sinr_db: f.get("drones.link.video_sinr_db"),
            width_db: f.get("drones.link.width_db"),
            coherence_s: f.get("drones.link.coherence_s"),
            snag_per_km: f.get("drones.fibre.snag_per_km"),
        }
    }
}

/// Link inputs at both ends, from the propagation and EW models.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkInputs {
    /// Operator's control signal at the drone, dBW.
    pub uplink_signal_dbw: f64,
    /// Jamming at the drone's receiver, dBW (−∞ if none).
    pub uplink_jam_dbw: f64,
    /// Drone's video signal at the operator, dBW.
    pub downlink_signal_dbw: f64,
    /// Jamming at the operator's receiver, dBW.
    pub downlink_jam_dbw: f64,
    pub noise_dbw: f64,
}

/// The link's state (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkAssessment {
    pub uplink_sinr_db: f64,
    pub downlink_sinr_db: f64,
    pub uplink_js_db: f64,
    pub downlink_js_db: f64,
    pub p_control: f64,
    pub p_video: f64,
    /// Probability the link is usable now (both directions).
    pub quality: f64,
    pub jammable: bool,
}

fn sinr(s: f64, j: f64, n: f64) -> f64 {
    let jn = crate::math::db_to_ratio(n) + if j.is_finite() { crate::math::db_to_ratio(j) } else { 0.0 };
    s - crate::math::ratio_to_db(jn)
}

/// The control/video link against jamming at both ends (§8.7). Fibre and
/// preprogrammed drones ignore RF jamming.
pub fn assess_link(spec: &DroneSpec, p: &LinkParams, i: &LinkInputs) -> LinkAssessment {
    let up = sinr(i.uplink_signal_dbw, i.uplink_jam_dbw, i.noise_dbw);
    let down = sinr(i.downlink_signal_dbw, i.downlink_jam_dbw, i.noise_dbw);
    if !spec.jammable() {
        return LinkAssessment {
            uplink_sinr_db: up,
            downlink_sinr_db: down,
            uplink_js_db: i.uplink_jam_dbw - i.uplink_signal_dbw,
            downlink_js_db: i.downlink_jam_dbw - i.downlink_signal_dbw,
            p_control: 1.0,
            p_video: 1.0,
            quality: 1.0,
            jammable: false,
        };
    }
    let pc = soft_step(up, p.control_sinr_db, p.width_db);
    let pv = soft_step(down, p.video_sinr_db, p.width_db);
    LinkAssessment {
        uplink_sinr_db: up,
        downlink_sinr_db: down,
        uplink_js_db: i.uplink_jam_dbw - i.uplink_signal_dbw,
        downlink_js_db: i.downlink_jam_dbw - i.downlink_signal_dbw,
        p_control: pc,
        p_video: pv,
        quality: pc * pv,
        jammable: true,
    }
}

/// Probability the link holds for `duration_s` at a given quality: the
/// quality is re-drawn every coherence time, so P = q^(T/τ).
pub fn p_link_holds(quality: f64, duration_s: f64, p: &LinkParams) -> f64 {
    quality.clamp(0.0, 1.0).powf((duration_s / p.coherence_s).max(1.0))
}

/// Fibre limits (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FibreCheck {
    pub within_spool: bool,
    pub p_snag: f64,
    pub p_ok: f64,
}

/// Fibre: within the spool, and a snag hazard per km paid out.
pub fn fibre_check(distance_m: f64, spool_m: f64, p: &LinkParams) -> FibreCheck {
    let within = distance_m <= spool_m;
    let p_snag = bgws_core::hazard_probability(p.snag_per_km, distance_m / 1000.0);
    FibreCheck { within_spool: within, p_snag, p_ok: if within { 1.0 - p_snag } else { 0.0 } }
}

/// What link loss leads to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkLossOutcome {
    pub behaviour: LinkLossBehaviour,
    /// The strike can still hit (with this factor on the terminal chance).
    pub hit_factor: f64,
    pub drone_recovered: bool,
}

/// The autonomy behaviour on link loss, and what is left of the strike.
pub fn link_loss_outcome(spec: &DroneSpec, target_speed_mps: f64, f: &StrikeParams) -> LinkLossOutcome {
    match spec.on_link_loss {
        LinkLossBehaviour::ReturnHome => LinkLossOutcome { behaviour: LinkLossBehaviour::ReturnHome, hit_factor: 0.0, drone_recovered: true },
        LinkLossBehaviour::Hover => LinkLossOutcome { behaviour: LinkLossBehaviour::Hover, hit_factor: 0.0, drone_recovered: false },
        LinkLossBehaviour::ContinueToAimPoint => LinkLossOutcome {
            behaviour: LinkLossBehaviour::ContinueToAimPoint,
            hit_factor: (-f.blind_per_mps * target_speed_mps).exp(),
            drone_recovered: false,
        },
        LinkLossBehaviour::AiTerminal => LinkLossOutcome { behaviour: LinkLossBehaviour::AiTerminal, hit_factor: f.ai_factor, drone_recovered: false },
    }
}

/// A drone operator.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operator {
    /// 0 = novice, 1 = expert.
    pub skill: f64,
    /// 0 = fresh, 1 = exhausted.
    pub fatigue: f64,
}

impl Operator {
    pub fn effective_skill(&self, f: &StrikeParams) -> f64 {
        (self.skill * (1.0 - f.fatigue_penalty * self.fatigue.clamp(0.0, 1.0))).clamp(0.0, 1.0)
    }
    /// Fly for `hours`: fatigue builds.
    pub fn work(&mut self, hours: f64, f: &StrikeParams) {
        self.fatigue = (self.fatigue + f.fatigue_per_h * hours).min(1.0);
    }
    /// Rest for `hours`: fatigue falls.
    pub fn rest(&mut self, hours: f64, f: &StrikeParams) {
        self.fatigue = (self.fatigue - f.recovery_per_h * hours).max(0.0);
    }
}

/// Strike-chain parameters from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeParams {
    pub find_rate: f64,
    pub search_s: f64,
    pub p_skilled: f64,
    pub p_novice: f64,
    pub motion_per_mps: f64,
    pub wind_per_mps: f64,
    pub link_exponent: f64,
    pub blind_per_mps: f64,
    pub ai_factor: f64,
    pub fatigue_penalty: f64,
    pub fatigue_per_h: f64,
    pub recovery_per_h: f64,
    pub kill_given_hit: BTreeMap<TargetClass, f64>,
    pub link: LinkParams,
}

impl StrikeParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            find_rate: f.get("drones.find.rate_per_s"),
            search_s: f.get("drones.find.search_s"),
            p_skilled: f.get("drones.terminal.p_skilled"),
            p_novice: f.get("drones.terminal.p_novice"),
            motion_per_mps: f.get("drones.terminal.motion_per_mps"),
            wind_per_mps: f.get("drones.terminal.wind_per_mps"),
            link_exponent: f.get("drones.terminal.link_exponent"),
            blind_per_mps: f.get("drones.terminal.blind_per_mps"),
            ai_factor: f.get("drones.terminal.ai_factor"),
            fatigue_penalty: f.get("drones.operator.fatigue_penalty"),
            fatigue_per_h: f.get("drones.operator.fatigue_per_h"),
            recovery_per_h: f.get("drones.operator.recovery_per_h"),
            kill_given_hit: TargetClass::ALL.iter().map(|t| (*t, f.get(&format!("drones.kill_given_hit.{}", t.key())))).collect(),
            link: LinkParams::from_figures(f),
        }
    }
}

/// The situation of one FPV strike.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeInputs {
    pub operator: Operator,
    pub target: TargetClass,
    pub target_speed_mps: f64,
    /// 0 = in the open, 1 = invisible to the camera.
    pub concealment: f64,
    pub wind_mps: f64,
    /// Link quality now, from [`assess_link`] (1 for fibre).
    pub link_quality: f64,
    /// Fibre only: distance flown.
    pub distance_m: f64,
    /// Time from detection to terminal dive, exposed to counter-drone fire.
    pub approach_s: f64,
    /// Counter-drone kill hazard during the approach, 1/s (from counter_drone).
    pub counter_drone_rate: f64,
    /// Passive measures at the target: factors on hit and on kill-given-hit.
    pub hit_modifier: f64,
    pub kill_modifier: f64,
}

/// How far the strike got.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrikeStage {
    NotFound,
    ShotDown,
    LinkLost,
    Missed,
    HitNoKill,
    Destroyed,
}

/// One strike's result with every stage's probability (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeOutcome {
    pub p_find: f64,
    pub p_survive_approach: f64,
    pub p_link_holds: f64,
    pub p_terminal_hit: f64,
    /// Hit chance after link loss, if the drone can still strike.
    pub p_hit_after_link_loss: f64,
    pub p_kill_given_hit: f64,
    pub effective_skill: f64,
    pub stage: StrikeStage,
    pub link_loss: Option<LinkLossOutcome>,
}

impl StrikeOutcome {
    pub fn hit(&self) -> bool {
        self.stage >= StrikeStage::HitNoKill
    }
    /// The analytic chance of a hit, before the draws.
    pub fn p_hit(&self) -> f64 {
        self.p_find * self.p_survive_approach * (self.p_link_holds * self.p_terminal_hit + (1.0 - self.p_link_holds) * self.p_hit_after_link_loss)
    }
}

/// Terminal hit chance: skill (after fatigue) interpolates novice → skilled;
/// target motion and wind fall off exponentially; the link scales it.
pub fn terminal_hit_probability(p: &StrikeParams, i: &StrikeInputs) -> f64 {
    let s = i.operator.effective_skill(p);
    let base = p.p_novice + (p.p_skilled - p.p_novice) * s;
    let motion = (-p.motion_per_mps * i.target_speed_mps.max(0.0)).exp();
    let wind = (-p.wind_per_mps * i.wind_mps.max(0.0)).exp();
    let link = i.link_quality.clamp(0.0, 1.0).powf(p.link_exponent);
    (base * motion * wind * link * i.hit_modifier).clamp(0.0, 1.0)
}

/// An FPV strike as a chain of draws (§8.7).
pub fn fpv_strike(spec: &DroneSpec, p: &StrikeParams, i: &StrikeInputs, rng: &mut impl Rng) -> StrikeOutcome {
    let skill = i.operator.effective_skill(p);
    let p_find = 1.0 - (-p.find_rate * (0.25 + skill) * (1.0 - i.concealment.clamp(0.0, 1.0)) * p.search_s).exp();
    let p_survive = (-i.counter_drone_rate.max(0.0) * i.approach_s.max(0.0)).exp();
    let p_link = match spec.link {
        ControlLink::Rf => p_link_holds(i.link_quality, i.approach_s + p.search_s, &p.link),
        ControlLink::Fibre { spool_m } => fibre_check(i.distance_m, spool_m, &p.link).p_ok,
        ControlLink::Preprogrammed => 1.0,
    };
    let p_term = terminal_hit_probability(p, i);
    let loss = link_loss_outcome(spec, i.target_speed_mps, p);
    // After link loss the autonomy flies the terminal at a skilled-pilot-equivalent base.
    let p_after = if spec.carries_warhead {
        let calm = StrikeInputs { link_quality: 1.0, operator: Operator { skill: 1.0, fatigue: 0.0 }, ..i.clone() };
        terminal_hit_probability(p, &calm) * loss.hit_factor
    } else {
        0.0
    };
    let p_kill = (p.kill_given_hit.get(&i.target).copied().unwrap_or(0.0) * i.kill_modifier).clamp(0.0, 1.0);
    let mut out = StrikeOutcome {
        p_find,
        p_survive_approach: p_survive,
        p_link_holds: p_link,
        p_terminal_hit: p_term,
        p_hit_after_link_loss: p_after,
        p_kill_given_hit: p_kill,
        effective_skill: skill,
        stage: StrikeStage::NotFound,
        link_loss: None,
    };
    if rng.gen::<f64>() >= p_find {
        return out;
    }
    if rng.gen::<f64>() >= p_survive {
        out.stage = StrikeStage::ShotDown;
        return out;
    }
    let p_hit = if rng.gen::<f64>() < p_link {
        p_term
    } else {
        out.link_loss = Some(loss);
        p_after
    };
    if rng.gen::<f64>() >= p_hit {
        out.stage = if out.link_loss.is_some() { StrikeStage::LinkLost } else { StrikeStage::Missed };
        return out;
    }
    out.stage = if rng.gen::<f64>() < p_kill { StrikeStage::Destroyed } else { StrikeStage::HitNoKill };
    out
}

#[derive(Debug, Error, PartialEq)]
pub enum StoreError {
    #[error("no {0:?} drones left")]
    NoDrones(DroneType),
    #[error("no batteries left")]
    NoBatteries,
}

/// Drones and batteries as stores (§8.7).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DroneStores {
    pub drones: BTreeMap<DroneType, u32>,
    pub batteries: u32,
}

impl DroneStores {
    /// Launch a sortie: takes a drone and its batteries (multirotors and FPVs).
    pub fn launch(&mut self, spec: &DroneSpec, f: &Figures) -> Result<(), StoreError> {
        let need = if matches!(spec.kind, DroneType::FixedWingIsr | DroneType::OwaShahed) { 0 } else { f.get("drones.batteries_per_sortie").round() as u32 };
        let n = self.drones.get(&spec.kind).copied().unwrap_or(0);
        if n == 0 {
            return Err(StoreError::NoDrones(spec.kind));
        }
        if self.batteries < need {
            return Err(StoreError::NoBatteries);
        }
        self.batteries -= need;
        self.drones.insert(spec.kind, n - 1);
        Ok(())
    }

    /// A drone comes back (not expendables, which never do).
    pub fn recover(&mut self, spec: &DroneSpec) {
        if !spec.expendable {
            *self.drones.entry(spec.kind).or_insert(0) += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{Seed, StreamKey};

    fn figs() -> Figures {
        crate::default_figures()
    }

    /// A representative mix of FPV targets (armour, IFVs, trucks, guns, infantry).
    fn target_for(u: f64) -> TargetClass {
        match u {
            x if x < 0.15 => TargetClass::Mbt,
            x if x < 0.40 => TargetClass::Ifv,
            x if x < 0.60 => TargetClass::Truck,
            x if x < 0.70 => TargetClass::Artillery,
            x if x < 0.80 => TargetClass::InfantryOpen,
            _ => TargetClass::InfantryCover,
        }
    }

    struct Tally {
        sorties: u32,
        hits: u32,
        kills: u32,
    }

    fn campaign(skill: (f64, f64), n: u32, seed: u64) -> Tally {
        let f = figs();
        let p = StrikeParams::from_figures(&f);
        let rf = DroneSpec::from_figures(DroneType::FpvRf, &f);
        let fibre = DroneSpec::from_figures(DroneType::FpvFibre, &f);
        let mut t = Tally { sorties: 0, hits: 0, kills: 0 };
        for i in 0..n {
            let mut r = Seed(seed).stream(StreamKey::new("test.fpv", i as u64, 0, 0));
            let spec = if r.gen::<f64>() < 0.2 { &fibre } else { &rf };
            let jammed = r.gen::<f64>() < 0.3;
            let inputs = StrikeInputs {
                operator: Operator { skill: r.gen_range(skill.0..skill.1), fatigue: r.gen_range(0.0..0.4) },
                target: target_for(r.gen()),
                target_speed_mps: if r.gen::<f64>() < 0.5 { 0.0 } else { r.gen_range(2.0..8.0) },
                concealment: r.gen_range(0.0..0.5),
                wind_mps: r.gen_range(0.0..8.0),
                link_quality: if jammed { r.gen_range(0.85..0.98) } else { 1.0 },
                distance_m: r.gen_range(2_000.0..12_000.0),
                approach_s: 60.0,
                counter_drone_rate: if r.gen::<f64>() < 0.3 { 0.003 } else { 0.0 },
                hit_modifier: 1.0,
                kill_modifier: 1.0,
            };
            let o = fpv_strike(spec, &p, &inputs, &mut r);
            t.sorties += 1;
            if o.hit() {
                t.hits += 1;
            }
            if o.stage == StrikeStage::Destroyed {
                t.kills += 1;
            }
        }
        t
    }

    /// Acceptance [P9]: skilled crews hit 30–50% of sorties, novices ≈10%,
    /// and 60–80% of hits fail to destroy.
    #[test]
    fn fpv_hit_rates_match_p9() {
        let skilled = campaign((0.7, 0.95), 4_000, 1);
        let rate = skilled.hits as f64 / skilled.sorties as f64;
        assert!((0.30..=0.50).contains(&rate), "skilled hit rate {rate}");
        let novice = campaign((0.0, 0.15), 4_000, 2);
        let nrate = novice.hits as f64 / novice.sorties as f64;
        assert!((0.05..=0.15).contains(&nrate), "novice hit rate {nrate}");
        let fail = 1.0 - skilled.kills as f64 / skilled.hits as f64;
        assert!((0.60..=0.80).contains(&fail), "fail-to-destroy {fail}");
    }

    #[test]
    fn link_against_jamming_at_both_ends() {
        let f = figs();
        let p = LinkParams::from_figures(&f);
        let rf = DroneSpec::from_figures(DroneType::FpvRf, &f);
        let clear = LinkInputs { uplink_signal_dbw: -90.0, uplink_jam_dbw: f64::NEG_INFINITY, downlink_signal_dbw: -95.0, downlink_jam_dbw: f64::NEG_INFINITY, noise_dbw: -130.0 };
        assert!(assess_link(&rf, &p, &clear).quality > 0.99);
        let jam_drone = LinkInputs { uplink_jam_dbw: -85.0, ..clear };
        let a = assess_link(&rf, &p, &jam_drone);
        assert!(a.p_control < 0.01 && a.p_video > 0.99);
        let jam_op = LinkInputs { downlink_jam_dbw: -90.0, ..clear };
        assert!(assess_link(&rf, &p, &jam_op).p_video < 0.01);
        let fibre = DroneSpec::from_figures(DroneType::FpvFibre, &f);
        assert_eq!(assess_link(&fibre, &p, &jam_drone).quality, 1.0);
        // Fibre: spool cap and snags.
        let ControlLink::Fibre { spool_m } = fibre.link else { panic!() };
        assert_eq!(fibre_check(spool_m + 1.0, spool_m, &p).p_ok, 0.0);
        let c = fibre_check(10_000.0, spool_m, &p);
        assert!(c.p_snag > 0.15 && c.p_snag < 0.2);
        assert!(p_link_holds(0.9, 200.0, &p) < p_link_holds(0.9, 20.0, &p));
    }

    #[test]
    fn link_loss_behaviours() {
        let f = figs();
        let p = StrikeParams::from_figures(&f);
        let fpv = DroneSpec::from_figures(DroneType::FpvRf, &f);
        assert_eq!(link_loss_outcome(&fpv, 0.0, &p).hit_factor, 0.0);
        let ai = fpv.clone().with_ai_terminal();
        assert!(link_loss_outcome(&ai, 5.0, &p).hit_factor > 0.5);
        let lm = DroneSpec::from_figures(DroneType::LoiteringMunition, &f);
        assert!(link_loss_outcome(&lm, 0.0, &p).hit_factor > link_loss_outcome(&lm, 5.0, &p).hit_factor);
        assert!(link_loss_outcome(&DroneSpec::from_figures(DroneType::ReconMultirotor, &f), 0.0, &p).drone_recovered);
        // A jammed FPV with AI terminal still hits sometimes; without, never after loss.
        let inputs = StrikeInputs {
            operator: Operator { skill: 0.8, fatigue: 0.0 },
            target: TargetClass::Truck,
            target_speed_mps: 0.0,
            concealment: 0.0,
            wind_mps: 0.0,
            link_quality: 0.3,
            distance_m: 5_000.0,
            approach_s: 60.0,
            counter_drone_rate: 0.0,
            hit_modifier: 1.0,
            kill_modifier: 1.0,
        };
        let mut r = Seed(1).stream(StreamKey::new("t", 0, 0, 0));
        let plain = fpv_strike(&fpv, &p, &inputs, &mut r);
        let smart = fpv_strike(&ai, &p, &inputs, &mut r);
        assert!(smart.p_hit() > 3.0 * plain.p_hit(), "{} {}", smart.p_hit(), plain.p_hit());
    }

    #[test]
    fn fatigue_and_stores() {
        let f = figs();
        let p = StrikeParams::from_figures(&f);
        let mut op = Operator { skill: 0.8, fatigue: 0.0 };
        op.work(10.0, &p);
        assert!(op.effective_skill(&p) < 0.8);
        op.rest(10.0, &p);
        assert_eq!(op.fatigue, 0.0);
        let mut s = DroneStores { drones: [(DroneType::FpvRf, 1), (DroneType::ReconMultirotor, 1)].into(), batteries: 1 };
        let fpv = DroneSpec::from_figures(DroneType::FpvRf, &f);
        let recon = DroneSpec::from_figures(DroneType::ReconMultirotor, &f);
        s.launch(&fpv, &f).unwrap();
        assert_eq!(s.launch(&fpv, &f), Err(StoreError::NoDrones(DroneType::FpvRf)));
        assert_eq!(s.launch(&recon, &f), Err(StoreError::NoBatteries));
        s.batteries = 2;
        s.launch(&recon, &f).unwrap();
        s.recover(&recon);
        s.recover(&fpv);
        assert_eq!(s.drones[&DroneType::ReconMultirotor], 1);
        assert_eq!(s.drones[&DroneType::FpvRf], 0);
    }
}
