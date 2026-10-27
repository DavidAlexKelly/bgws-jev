//! Indirect fire (RUST_ENGINE.md §8.8 "Indirect fire").
//!
//! - **Ballistics:** a point-mass trajectory with drag (constant drag
//!   coefficient, exponential atmosphere), integrated with RK4, gives range,
//!   time of flight and angle of fall; [`solve`] finds the charge and
//!   quadrant elevation for a range (low or high angle).
//! - **Delivery errors:** meteorological bias growing with the age of the met
//!   message, survey/PNT and orientation bias, and range-proportional
//!   dispersion (probable errors).
//! - **Lethality:** a Carleton damage function P(x, y) = D₀·exp(−x²/Rx² − y²/Ry²)
//!   by round and target posture (standing, prone, foxhole/trench, soft
//!   vehicle, armoured vehicle), lethal area π·Rx·Ry·D₀, integrated in closed
//!   form against Gaussian delivery error over a rectangular target, JMEM
//!   style **[default parameters]**.
//! - **Adjust fire** as an observe–correct loop, then **fire for effect**.
//! - **Counter-battery:** weapon-locating radar backtrack error and time to
//!   locate; survival as the chance the guns displace before the response
//!   lands **[P23]**, **[ref R4.3]**.
//! - **Consumption:** rounds needed for an effect, checked against **[P12]**.

use crate::math::{mean_gauss_over_interval, norm_cdf, norm_inv};
use crate::ModelError;
use bgws_core::units::G0;
use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// A projectile's ballistic description.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shell {
    pub mass_kg: f64,
    pub diameter_m: f64,
    /// Drag coefficient (constant: the simple drag law).
    pub cd: f64,
}

/// One flight's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    pub range_m: f64,
    pub time_of_flight_s: f64,
    pub apex_m: f64,
    /// Angle of fall below the horizontal, rad.
    pub angle_of_fall_rad: f64,
    pub impact_speed_mps: f64,
}

/// A firing solution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FiringSolution {
    /// Index into the charge list.
    pub charge: usize,
    pub quadrant_elevation_rad: f64,
    pub trajectory: Trajectory,
}

/// Round classes for lethality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundClass {
    Mortar81,
    Mortar120,
    Howitzer155,
    Rocket122,
}

impl RoundClass {
    const ALL: [RoundClass; 4] = [RoundClass::Mortar81, RoundClass::Mortar120, RoundClass::Howitzer155, RoundClass::Rocket122];
    fn key(self) -> &'static str {
        match self {
            RoundClass::Mortar81 => "mortar81",
            RoundClass::Mortar120 => "mortar120",
            RoundClass::Howitzer155 => "howitzer155",
            RoundClass::Rocket122 => "rocket122",
        }
    }
}

/// Target posture for the damage function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetPosture {
    Standing,
    Prone,
    /// In a foxhole or trench.
    Entrenched,
    SoftVehicle,
    ArmouredVehicle,
}

impl TargetPosture {
    const ALL: [TargetPosture; 5] = [TargetPosture::Standing, TargetPosture::Prone, TargetPosture::Entrenched, TargetPosture::SoftVehicle, TargetPosture::ArmouredVehicle];
    fn key(self) -> &'static str {
        match self {
            TargetPosture::Standing => "standing",
            TargetPosture::Prone => "prone",
            TargetPosture::Entrenched => "entrenched",
            TargetPosture::SoftVehicle => "soft_vehicle",
            TargetPosture::ArmouredVehicle => "armoured_vehicle",
        }
    }
}

/// Weapon systems that must displace after firing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiringSystem {
    SelfPropelledGun,
    TowedGun,
    Mlrs,
    Mortar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndirectParams {
    pub rho0: f64,
    pub scale_height_m: f64,
    pub dt_s: f64,
    pub shell155: Shell,
    pub charges155: Vec<f64>,
    pub met_frac0: f64,
    pub met_growth_per_h: f64,
    pub met_frac_max: f64,
    pub orientation_mil: f64,
    pub pe_range_frac: f64,
    pub pe_defl_frac: f64,
    /// Carleton for a 155 mm HE round vs standing men: Rx (deflection), Ry (range), D0.
    pub carleton_base: (f64, f64, f64),
    pub round_scale: [f64; 4],
    pub posture_area: [f64; 5],
    pub posture_d0: [f64; 5],
    pub observer_gain: f64,
    pub ffe_threshold_m: f64,
    pub max_adjust_rounds: f64,
    pub adjust_cycle_s: f64,
    pub wlr_base_m: f64,
    pub wlr_frac: f64,
    pub wlr_time_s: f64,
    pub cb_response_median_s: f64,
    pub cb_response_sigma: f64,
    pub displace_s: [f64; 4],
    pub effect_neutralise: f64,
    pub effect_destroy: f64,
    pub default_rounds_per_gun: f64,
}

impl IndirectParams {
    pub fn from_figures(f: &Figures) -> Self {
        IndirectParams {
            rho0: f.get("indirect.atmos.rho0_kgm3"),
            scale_height_m: f.get("indirect.atmos.scale_height_m"),
            dt_s: f.get("indirect.traj.dt_s"),
            shell155: Shell { mass_kg: f.get("indirect.shell155.mass_kg"), diameter_m: f.get("indirect.shell155.diameter_m"), cd: f.get("indirect.shell155.cd") },
            charges155: (1..=5).map(|i| f.get(&format!("indirect.shell155.charge{i}_mps"))).collect(),
            met_frac0: f.get("indirect.error.met_frac0"),
            met_growth_per_h: f.get("indirect.error.met_growth_per_h"),
            met_frac_max: f.get("indirect.error.met_frac_max"),
            orientation_mil: f.get("indirect.error.orientation_mil"),
            pe_range_frac: f.get("indirect.error.pe_range_frac"),
            pe_defl_frac: f.get("indirect.error.pe_defl_frac"),
            carleton_base: (f.get("indirect.carleton.rx_m"), f.get("indirect.carleton.ry_m"), f.get("indirect.carleton.d0")),
            round_scale: RoundClass::ALL.map(|r| f.get(&format!("indirect.round_scale.{}", r.key()))),
            posture_area: TargetPosture::ALL.map(|t| f.get(&format!("indirect.posture.area_factor.{}", t.key()))),
            posture_d0: TargetPosture::ALL.map(|t| f.get(&format!("indirect.posture.d0_factor.{}", t.key()))),
            observer_gain: f.get("indirect.adjust.observer_gain"),
            ffe_threshold_m: f.get("indirect.adjust.ffe_threshold_m"),
            max_adjust_rounds: f.get("indirect.adjust.max_rounds"),
            adjust_cycle_s: f.get("indirect.adjust.cycle_s"),
            wlr_base_m: f.get("indirect.cb.wlr_base_m"),
            wlr_frac: f.get("indirect.cb.wlr_frac"),
            wlr_time_s: f.get("indirect.cb.time_to_locate_s"),
            cb_response_median_s: f.get("indirect.cb.response_median_s"),
            cb_response_sigma: f.get("indirect.cb.response_sigma"),
            displace_s: ["sp_gun", "towed_gun", "mlrs", "mortar"].map(|s| f.get(&format!("indirect.displace_s.{s}"))),
            effect_neutralise: f.get("indirect.effect.neutralise"),
            effect_destroy: f.get("indirect.effect.destroy"),
            default_rounds_per_gun: f.get("indirect.mission.default_rounds_per_gun"),
        }
    }
}

/// Fly a shell from the origin at `v0` and quadrant elevation `qe` until it
/// comes down through `target_alt_m` (relative to the gun). RK4 on
/// dv/dt = −k(h)·|v|·v − g, k = ρ(h)·Cd·A/(2m).
pub fn fly(v0: f64, qe: f64, shell: &Shell, target_alt_m: f64, p: &IndirectParams) -> Trajectory {
    let area = PI * shell.diameter_m * shell.diameter_m / 4.0;
    let kk = shell.cd * area / (2.0 * shell.mass_kg);
    let acc = |s: [f64; 4]| -> [f64; 4] {
        let (vx, vy) = (s[2], s[3]);
        let v = vx.hypot(vy);
        let k = kk * p.rho0 * (-s[1].max(0.0) / p.scale_height_m).exp();
        [vx, vy, -k * v * vx, -k * v * vy - G0]
    };
    let add = |a: [f64; 4], b: [f64; 4], h: f64| [a[0] + b[0] * h, a[1] + b[1] * h, a[2] + b[2] * h, a[3] + b[3] * h];
    let mut s = [0.0, 0.0, v0 * qe.cos(), v0 * qe.sin()];
    let mut t = 0.0;
    let mut apex: f64 = 0.0;
    let h = p.dt_s;
    for _ in 0..1_000_000 {
        let k1 = acc(s);
        let k2 = acc(add(s, k1, h / 2.0));
        let k3 = acc(add(s, k2, h / 2.0));
        let k4 = acc(add(s, k3, h));
        let next = [
            s[0] + h / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
            s[1] + h / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
            s[2] + h / 6.0 * (k1[2] + 2.0 * k2[2] + 2.0 * k3[2] + k4[2]),
            s[3] + h / 6.0 * (k1[3] + 2.0 * k2[3] + 2.0 * k3[3] + k4[3]),
        ];
        apex = apex.max(next[1]);
        if next[3] < 0.0 && next[1] <= target_alt_m {
            // Interpolate the crossing linearly within the step.
            let f = if (s[1] - next[1]).abs() > 1e-12 { ((s[1] - target_alt_m) / (s[1] - next[1])).clamp(0.0, 1.0) } else { 1.0 };
            let lerp = |a: f64, b: f64| a + (b - a) * f;
            let (vx, vy) = (lerp(s[2], next[2]), lerp(s[3], next[3]));
            return Trajectory {
                range_m: lerp(s[0], next[0]),
                time_of_flight_s: t + h * f,
                apex_m: apex,
                angle_of_fall_rad: (-vy).atan2(vx),
                impact_speed_mps: vx.hypot(vy),
            };
        }
        s = next;
        t += h;
    }
    Trajectory { range_m: s[0], time_of_flight_s: t, apex_m: apex, angle_of_fall_rad: 0.0, impact_speed_mps: 0.0 }
}

/// Maximum range and its elevation for a charge (golden-section search).
pub fn max_range(v0: f64, shell: &Shell, p: &IndirectParams) -> (f64, f64) {
    let (mut a, mut b) = (15f64.to_radians(), 65f64.to_radians());
    let gr = (5f64.sqrt() - 1.0) / 2.0;
    let r = |q: f64| fly(v0, q, shell, 0.0, p).range_m;
    let (mut c, mut d) = (b - gr * (b - a), a + gr * (b - a));
    let (mut fc, mut fd) = (r(c), r(d));
    for _ in 0..30 {
        if fc > fd {
            b = d;
            d = c;
            fd = fc;
            c = b - gr * (b - a);
            fc = r(c);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + gr * (b - a);
            fd = r(d);
        }
    }
    let q = (a + b) / 2.0;
    (r(q), q)
}

/// The lowest charge that reaches `range_m`, and its elevation (low or high angle).
pub fn solve(range_m: f64, charges_mps: &[f64], shell: &Shell, high_angle: bool, p: &IndirectParams) -> Result<FiringSolution, ModelError> {
    for (i, &v0) in charges_mps.iter().enumerate() {
        let (rmax, qmax) = max_range(v0, shell, p);
        if rmax < range_m {
            continue;
        }
        let (mut lo, mut hi) = if high_angle { (qmax, 85f64.to_radians()) } else { (0.2f64.to_radians(), qmax) };
        for _ in 0..50 {
            let mid = (lo + hi) / 2.0;
            let r = fly(v0, mid, shell, 0.0, p).range_m;
            // Range rises with elevation below qmax and falls above it.
            if (r < range_m) != high_angle {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let qe = (lo + hi) / 2.0;
        return Ok(FiringSolution { charge: i, quadrant_elevation_rad: qe, trajectory: fly(v0, qe, shell, 0.0, p) });
    }
    Err(ModelError::NoSolution(format!("{range_m:.0} m is beyond the top charge")))
}

/// Delivery error budget at the target, one sigma, metres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeliveryErrors {
    pub met_m: f64,
    pub survey_m: f64,
    pub orientation_m: f64,
    /// Per-mission bias, range and deflection.
    pub bias_range_m: f64,
    pub bias_defl_m: f64,
    /// Per-round dispersion, range and deflection.
    pub disp_range_m: f64,
    pub disp_defl_m: f64,
}

/// Errors for a mission at `range_m` with met data `met_age_h` old and the
/// gun's position known to `pnt_sigma_m` (GNSS jammed → large; §8.7).
pub fn delivery_errors(range_m: f64, met_age_h: f64, pnt_sigma_m: f64, p: &IndirectParams) -> DeliveryErrors {
    let met = range_m * (p.met_frac0 + p.met_growth_per_h * met_age_h.max(0.0)).min(p.met_frac_max);
    let orient = p.orientation_mil * range_m / 1000.0;
    let pe_to_sigma = 1.0 / norm_inv(0.75);
    DeliveryErrors {
        met_m: met,
        survey_m: pnt_sigma_m,
        orientation_m: orient,
        bias_range_m: met.hypot(pnt_sigma_m),
        bias_defl_m: (met.powi(2) + pnt_sigma_m.powi(2) + orient.powi(2)).sqrt(),
        disp_range_m: p.pe_range_frac * range_m * pe_to_sigma,
        disp_defl_m: p.pe_defl_frac * range_m * pe_to_sigma,
    }
}

/// A Carleton damage function.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Carleton {
    /// Deflection (across the line of fire) scale, m.
    pub rx_m: f64,
    /// Range (along the line of fire) scale, m.
    pub ry_m: f64,
    pub d0: f64,
}

impl Carleton {
    /// The damage function for a round class against a posture.
    pub fn for_round(round: RoundClass, posture: TargetPosture, p: &IndirectParams) -> Carleton {
        let s = p.round_scale[round as usize];
        let a = p.posture_area[posture as usize].sqrt();
        Carleton { rx_m: p.carleton_base.0 * s * a, ry_m: p.carleton_base.1 * s * a, d0: (p.carleton_base.2 * p.posture_d0[posture as usize]).min(1.0) }
    }
    /// P(kill) at offset (dx deflection, dy range) from the burst.
    pub fn p_kill(&self, dx: f64, dy: f64) -> f64 {
        self.d0 * (-(dx * dx) / (self.rx_m * self.rx_m) - (dy * dy) / (self.ry_m * self.ry_m)).exp()
    }
    /// Lethal area π·Rx·Ry·D₀, m².
    pub fn lethal_area(&self) -> f64 {
        PI * self.rx_m * self.ry_m * self.d0
    }
    /// P(kill) of one round for a person at offset (tx, ty) from the aim
    /// point, the round falling with bias (bx, by) and dispersion (sx, sy).
    pub fn p_kill_point(&self, t: (f64, f64), bias: (f64, f64), sigma: (f64, f64)) -> f64 {
        let cx = self.rx_m.powi(2) + 2.0 * sigma.0.powi(2);
        let cy = self.ry_m.powi(2) + 2.0 * sigma.1.powi(2);
        self.d0 * self.rx_m / cx.sqrt() * self.ry_m / cy.sqrt() * (-(t.0 - bias.0).powi(2) / cx - (t.1 - bias.1).powi(2) / cy).exp()
    }
    /// Mean single-round P(kill) over people spread uniformly on a rectangle
    /// (`width_m` across, `depth_m` along the line of fire) centred on the aim point.
    pub fn mean_p_kill_area(&self, width_m: f64, depth_m: f64, bias: (f64, f64), sigma: (f64, f64)) -> f64 {
        let cx = self.rx_m.powi(2) + 2.0 * sigma.0.powi(2);
        let cy = self.ry_m.powi(2) + 2.0 * sigma.1.powi(2);
        self.d0 * self.rx_m / cx.sqrt() * self.ry_m / cy.sqrt() * mean_gauss_over_interval(width_m, bias.0, cx) * mean_gauss_over_interval(depth_m, bias.1, cy)
    }
}

/// Expected casualties from fire for effect: `rounds` rounds with a common
/// bias against `count` people on a rectangle. Each person survives every
/// round independently given the bias: E = Σ (1 − Π(1 − pᵣ)), averaged on a grid.
pub fn ffe_expected_casualties(c: &Carleton, count: f64, width_m: f64, depth_m: f64, rounds: f64, bias: (f64, f64), sigma: (f64, f64)) -> f64 {
    const N: usize = 15;
    let mut sum = 0.0;
    for i in 0..N {
        for j in 0..N {
            let tx = (i as f64 + 0.5) / N as f64 * width_m - width_m / 2.0;
            let ty = (j as f64 + 0.5) / N as f64 * depth_m - depth_m / 2.0;
            let pk = c.p_kill_point((tx, ty), bias, sigma).min(1.0);
            sum += 1.0 - (1.0 - pk).powf(rounds);
        }
    }
    count * sum / (N * N) as f64
}

/// Rounds needed to reach a casualty fraction (`effect_neutralise` / `effect_destroy`), or None.
pub fn rounds_for_effect(c: &Carleton, width_m: f64, depth_m: f64, fraction: f64, bias: (f64, f64), sigma: (f64, f64), max_rounds: u32) -> Option<u32> {
    (1..=max_rounds).find(|&r| ffe_expected_casualties(c, 1.0, width_m, depth_m, r as f64, bias, sigma) >= fraction)
}

/// One adjusting round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustRound {
    pub impact: (f64, f64),
    pub observed: (f64, f64),
    pub bias_after: (f64, f64),
    pub ready_for_ffe: bool,
}

/// Adjust fire: each round's fall is observed (with the observer's error)
/// and the correction applied to the aim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustFire {
    /// Current aim error relative to the target (deflection, range), m.
    pub bias: (f64, f64),
    pub rounds: u32,
    pub elapsed_s: f64,
    pub ready: bool,
}

impl AdjustFire {
    pub fn new(initial_bias: (f64, f64)) -> Self {
        AdjustFire { bias: initial_bias, rounds: 0, elapsed_s: 0.0, ready: false }
    }

    /// Fire one adjusting round: four normal draws in fixed order.
    pub fn step(&mut self, disp: (f64, f64), observer_sigma_m: f64, p: &IndirectParams, rng: &mut impl Rng) -> AdjustRound {
        let n = |s: f64, r: &mut dyn rand::RngCore| Normal::new(0.0, s.max(1e-9)).map(|d| d.sample(r)).unwrap_or(0.0);
        let impact = (self.bias.0 + n(disp.0, rng), self.bias.1 + n(disp.1, rng));
        let observed = (impact.0 + n(observer_sigma_m, rng), impact.1 + n(observer_sigma_m, rng));
        self.rounds += 1;
        self.elapsed_s += p.adjust_cycle_s;
        let miss = observed.0.hypot(observed.1);
        if miss <= p.ffe_threshold_m || self.rounds as f64 >= p.max_adjust_rounds {
            self.ready = true;
        }
        self.bias = (self.bias.0 - p.observer_gain * observed.0, self.bias.1 - p.observer_gain * observed.1);
        AdjustRound { impact, observed, bias_after: self.bias, ready_for_ffe: self.ready }
    }
}

/// A weapon-locating radar's fix on a firing battery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CbLocation {
    pub sigma_m: f64,
    pub cep_m: f64,
    pub time_to_locate_s: f64,
}

/// Backtrack error grows with range and shrinks with the number of rounds tracked.
pub fn wlr_locate(range_m: f64, rounds_tracked: u32, p: &IndirectParams) -> CbLocation {
    let n = rounds_tracked.max(1) as f64;
    let sigma = (p.wlr_base_m + p.wlr_frac * range_m) / n.sqrt();
    // CEP of a circular normal: σ·√(2 ln 2).
    CbLocation { sigma_m: sigma, cep_m: sigma * (2.0 * 2f64.ln()).sqrt(), time_to_locate_s: p.wlr_time_s }
}

/// Time a system needs to displace after firing **[P23]**.
pub fn displacement_time_s(system: FiringSystem, p: &IndirectParams) -> f64 {
    p.displace_s[system as usize]
}

/// P(the battery is gone before counter-battery fire lands), with the
/// response time lognormal around the declared median **[P23, R4.3]**.
pub fn p_survive_counter_battery(time_on_position_s: f64, p: &IndirectParams) -> f64 {
    if time_on_position_s <= 0.0 {
        return 1.0;
    }
    1.0 - norm_cdf((time_on_position_s / p.cb_response_median_s).ln() / p.cb_response_sigma)
}

/// Default mission size when no effect calculation is made **[P12]**.
pub fn default_mission_rounds(guns: u32, p: &IndirectParams) -> f64 {
    guns as f64 * p.default_rounds_per_gun
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("indirect.atmos.rho0_kgm3", 1.225, "kg/m3", 1.0, 1.4, "default", Default, "sea-level air density (standard atmosphere)"),
        figure!("indirect.atmos.scale_height_m", 8500.0, "m", 7000.0, 10000.0, "default", Default, "density scale height"),
        figure!("indirect.traj.dt_s", 0.05, "s", 0.005, 0.5, "default", Default, "RK4 step"),
        figure!("indirect.shell155.mass_kg", 43.0, "kg", 40.0, 50.0, "default", Default, "155 mm HE shell mass"),
        figure!("indirect.shell155.diameter_m", 0.155, "m", 0.155, 0.155, "default", Default, "calibre"),
        figure!("indirect.shell155.cd", 0.29, "1", 0.1, 0.5, "default", Default, "simple-drag-law coefficient (base-bleed lower)"),
        figure!("indirect.shell155.charge1_mps", 300.0, "m/s", 200.0, 400.0, "default", Default, "charge 1 muzzle velocity"),
        figure!("indirect.shell155.charge2_mps", 400.0, "m/s", 300.0, 500.0, "default", Default, "charge 2 muzzle velocity"),
        figure!("indirect.shell155.charge3_mps", 520.0, "m/s", 400.0, 650.0, "default", Default, "charge 3 muzzle velocity"),
        figure!("indirect.shell155.charge4_mps", 680.0, "m/s", 550.0, 800.0, "default", Default, "charge 4 muzzle velocity"),
        figure!("indirect.shell155.charge5_mps", 827.0, "m/s", 700.0, 950.0, "default", Default, "top charge muzzle velocity"),
        figure!("indirect.error.met_frac0", 0.003, "fraction", 0.0005, 0.01, "default", Default, "met bias σ as a fraction of range, fresh met"),
        figure!("indirect.error.met_growth_per_h", 0.0015, "1/h", 0.0, 0.01, "default", Default, "met bias growth per hour of met age"),
        figure!("indirect.error.met_frac_max", 0.02, "fraction", 0.005, 0.05, "default", Default, "met bias with no valid met"),
        figure!("indirect.error.orientation_mil", 1.0, "mil", 0.1, 5.0, "default", Default, "gun orientation error σ"),
        figure!("indirect.error.pe_range_frac", 0.0025, "fraction", 0.001, 0.006, "default", Default, "range probable error / range"),
        figure!("indirect.error.pe_defl_frac", 0.0008, "fraction", 0.0003, 0.003, "default", Default, "deflection probable error / range"),
        figure!("indirect.carleton.rx_m", 25.0, "m", 10.0, 50.0, "default", Default, "155 HE vs standing: deflection scale"),
        figure!("indirect.carleton.ry_m", 15.0, "m", 5.0, 40.0, "default", Default, "155 HE vs standing: range scale"),
        figure!("indirect.carleton.d0", 0.9, "fraction", 0.3, 1.0, "default", Default, "155 HE vs standing: peak P(kill)"),
        figure!("indirect.adjust.observer_gain", 1.0, "fraction", 0.3, 1.0, "default", Default, "fraction of the observed miss corrected"),
        figure!("indirect.adjust.ffe_threshold_m", 50.0, "m", 10.0, 200.0, "default", Default, "observed miss at which fire for effect is called"),
        figure!("indirect.adjust.max_rounds", 6.0, "rounds", 1.0, 12.0, "default", Default, "adjusting rounds before FFE regardless"),
        figure!("indirect.adjust.cycle_s", 60.0, "s", 10.0, 300.0, "P24", Default, "observe–correct–fire cycle (call for fire 3–15 min overall)"),
        figure!("indirect.cb.wlr_base_m", 20.0, "m", 5.0, 100.0, "default", Default, "WLR backtrack error floor"),
        figure!("indirect.cb.wlr_frac", 0.004, "fraction", 0.001, 0.02, "R4.3", Low, "WLR backtrack error per metre of range"),
        figure!("indirect.cb.time_to_locate_s", 30.0, "s", 5.0, 300.0, "default", Default, "radar track and compute time"),
        figure!("indirect.cb.response_median_s", 210.0, "s", 60.0, 900.0, "R4.3", Low, "counter-battery response median (2–5 min)"),
        figure!("indirect.cb.response_sigma", 0.4, "1", 0.1, 1.0, "default", Default, "lognormal spread of the response"),
        figure!("indirect.displace_s.sp_gun", 120.0, "s", 60.0, 180.0, "P23", Default, "SP gun displaces 1–3 min after firing"),
        figure!("indirect.displace_s.towed_gun", 600.0, "s", 300.0, 1800.0, "default", Default, "towed gun march order"),
        figure!("indirect.displace_s.mlrs", 60.0, "s", 30.0, 180.0, "default", Default, "MLRS shoot and scoot"),
        figure!("indirect.displace_s.mortar", 180.0, "s", 60.0, 600.0, "default", Default, "mortar out of action"),
        figure!("indirect.effect.neutralise", 0.1, "fraction", 0.05, 0.2, "default", Default, "casualty fraction counted as neutralised"),
        figure!("indirect.effect.destroy", 0.3, "fraction", 0.2, 0.5, "default", Default, "casualty fraction counted as destroyed"),
        figure!("indirect.mission.default_rounds_per_gun", 3.0, "rounds", 1.0, 20.0, "P12", Low, "rounds per gun per mission; scale checked against P12 daily totals"),
    ];
    let scale = [("mortar81", 0.55, "81 mm mortar"), ("mortar120", 0.85, "120 mm mortar"), ("howitzer155", 1.0, "155 mm"), ("rocket122", 1.1, "122 mm rocket")];
    for (k, s, n) in scale {
        v.push(figure!(format!("indirect.round_scale.{k}"), s, "1", 0.2, 2.0, "default", Default, format!("{n} Carleton length scale vs 155 HE")));
    }
    let posture = [
        ("standing", 1.0, 1.0),
        ("prone", 0.5, 0.9),
        ("entrenched", 0.12, 0.6),
        ("soft_vehicle", 0.35, 0.7),
        ("armoured_vehicle", 0.02, 0.4),
    ];
    for (k, a, d) in posture {
        v.push(figure!(format!("indirect.posture.area_factor.{k}"), a, "fraction", 0.001, 1.0, "default", Default, "lethal area vs standing"));
        v.push(figure!(format!("indirect.posture.d0_factor.{k}"), d, "fraction", 0.05, 1.0, "default", Default, "peak P(kill) vs standing"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{figs, rng};
    use approx::assert_relative_eq;

    #[test]
    fn vacuum_trajectory_matches_analytic() {
        let p = IndirectParams::from_figures(&figs());
        let vac = Shell { cd: 0.0, ..p.shell155 };
        let (v0, qe) = (300.0, 45f64.to_radians());
        let t = fly(v0, qe, &vac, 0.0, &p);
        assert_relative_eq!(t.range_m, v0 * v0 * (2.0 * qe).sin() / G0, max_relative = 1e-6);
        assert_relative_eq!(t.time_of_flight_s, 2.0 * v0 * qe.sin() / G0, max_relative = 1e-6);
        assert_relative_eq!(t.apex_m, (v0 * qe.sin()).powi(2) / (2.0 * G0), max_relative = 1e-3);
    }

    #[test]
    fn drag_and_firing_solutions() {
        let p = IndirectParams::from_figures(&figs());
        let (rmax, qmax) = max_range(p.charges155[4], &p.shell155, &p);
        assert!((18_000.0..30_000.0).contains(&rmax), "155 mm top charge reaches {rmax} m (R4.3: 24–30 km)");
        // Thin air aloft: long-range guns get their maximum above 45°, still far short of vacuum.
        assert!((40.0..60.0).contains(&qmax.to_degrees()), "{}", qmax.to_degrees());
        assert!(rmax < p.charges155[4].powi(2) / G0 * 0.5);
        let s = solve(12_000.0, &p.charges155, &p.shell155, false, &p).unwrap();
        assert_relative_eq!(s.trajectory.range_m, 12_000.0, epsilon = 1.0);
        let hi = solve(12_000.0, &p.charges155, &p.shell155, true, &p).unwrap();
        assert!(hi.quadrant_elevation_rad > s.quadrant_elevation_rad && hi.trajectory.time_of_flight_s > s.trajectory.time_of_flight_s);
        assert!(solve(60_000.0, &p.charges155, &p.shell155, false, &p).is_err());
    }

    #[test]
    fn met_age_grows_bias() {
        let p = IndirectParams::from_figures(&figs());
        let fresh = delivery_errors(20_000.0, 0.0, 10.0, &p);
        let stale = delivery_errors(20_000.0, 6.0, 10.0, &p);
        let jammed = delivery_errors(20_000.0, 0.0, 150.0, &p);
        assert!(stale.bias_range_m > fresh.bias_range_m * 2.0);
        assert!(jammed.bias_defl_m > fresh.bias_defl_m);
        assert_relative_eq!(fresh.disp_range_m, 0.0025 * 20_000.0 / 0.674_489_75, epsilon = 1e-3);
    }

    #[test]
    fn carleton_closed_form_and_postures() {
        let p = IndirectParams::from_figures(&figs());
        let c = Carleton::for_round(RoundClass::Howitzer155, TargetPosture::Standing, &p);
        assert_relative_eq!(c.lethal_area(), PI * 25.0 * 15.0 * 0.9, epsilon = 1e-9);
        // Integral of the damage function over the plane equals the lethal area.
        let num = crate::math::simpson(|x| crate::math::simpson(|y| c.p_kill(x, y), -100.0, 100.0, 200), -150.0, 150.0, 200);
        assert_relative_eq!(num, c.lethal_area(), max_relative = 1e-4);
        // Closed form vs Monte Carlo for a point target.
        let mut r = rng(9);
        let n = Normal::new(0.0, 1.0).unwrap();
        let (bias, sigma) = ((10.0, -5.0), (30.0, 40.0));
        let mc: f64 = (0..40_000).map(|_| c.p_kill(bias.0 + sigma.0 * n.sample(&mut r), bias.1 + sigma.1 * n.sample(&mut r))).sum::<f64>() / 40_000.0;
        assert_relative_eq!(c.p_kill_point((0.0, 0.0), bias, sigma), mc, epsilon = 0.01);
        let prone = Carleton::for_round(RoundClass::Howitzer155, TargetPosture::Prone, &p);
        let dug = Carleton::for_round(RoundClass::Howitzer155, TargetPosture::Entrenched, &p);
        let tank = Carleton::for_round(RoundClass::Howitzer155, TargetPosture::ArmouredVehicle, &p);
        assert!(c.lethal_area() > prone.lethal_area() && prone.lethal_area() > dug.lethal_area() && dug.lethal_area() > tank.lethal_area());
    }

    #[test]
    fn adjust_then_fire_for_effect() {
        let p = IndirectParams::from_figures(&figs());
        let err = delivery_errors(15_000.0, 3.0, 20.0, &p);
        let disp = (err.disp_defl_m, err.disp_range_m);
        let mut r = rng(4);
        let mut total_rounds = 0;
        let mut final_bias = 0.0;
        for _ in 0..200 {
            let mut a = AdjustFire::new((150.0, -200.0));
            while !a.ready {
                a.step(disp, 15.0, &p, &mut r);
            }
            total_rounds += a.rounds;
            final_bias += a.bias.0.hypot(a.bias.1);
        }
        let mean_bias = final_bias / 200.0;
        assert!(mean_bias < 80.0, "adjustment removes most of a 250 m error: {mean_bias}");
        assert!(total_rounds as f64 / 200.0 <= 6.0);
        // FFE: adjusted fire kills more than unadjusted.
        let c = Carleton::for_round(RoundClass::Howitzer155, TargetPosture::Prone, &p);
        let adjusted = ffe_expected_casualties(&c, 30.0, 150.0, 100.0, 18.0, (20.0, 20.0), disp);
        let unadjusted = ffe_expected_casualties(&c, 30.0, 150.0, 100.0, 18.0, (150.0, -200.0), disp);
        assert!(adjusted > unadjusted * 3.0 && adjusted < 30.0, "{adjusted} vs {unadjusted}");
        let n = rounds_for_effect(&c, 150.0, 100.0, p.effect_neutralise, (20.0, 20.0), disp, 500).unwrap();
        assert!(n >= 1 && n as f64 <= 60.0, "{n} rounds to neutralise");
        assert!(ffe_expected_casualties(&c, 1.0, 150.0, 100.0, n as f64, (20.0, 20.0), disp) >= p.effect_neutralise);
        assert!(default_mission_rounds(6, &p) > 0.0);
    }

    #[test]
    fn counter_battery_rewards_displacement() {
        // P23: a gun survives if it moves within about 2–3 minutes.
        let p = IndirectParams::from_figures(&figs());
        let quick = p_survive_counter_battery(displacement_time_s(FiringSystem::SelfPropelledGun, &p) + 30.0, &p);
        let slow = p_survive_counter_battery(displacement_time_s(FiringSystem::TowedGun, &p) + 60.0, &p);
        assert!(quick > 0.6 && slow < 0.05, "{quick} {slow}");
        let near = wlr_locate(10_000.0, 1, &p);
        let far = wlr_locate(30_000.0, 1, &p);
        let many = wlr_locate(30_000.0, 4, &p);
        assert!(far.sigma_m > near.sigma_m && many.sigma_m < far.sigma_m);
    }
}
