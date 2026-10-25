//! Direct fire (RUST_ENGINE.md §8.8 "Direct fire").
//!
//! **The error budget.** Each engagement has a fixed bias — range estimation
//! (through the trajectory's angle of fall), cant (through the drop),
//! crosswind (through the drag lag, Didion's rule), boresight and ammunition
//! jump, and the lead error from time of flight × the error in the target's
//! estimated speed — and each round has random dispersion (the gun's own,
//! the gunner's lay, and a moving firer unless stabilised). The fire-control
//! quality q ∈ [0, 1] interpolates each component between a basic (stadia,
//! no sensors) and a modern (laser, ballistic computer, stabilised) system.
//!
//! **Hit probability** is the bivariate normal integrated exactly (erf) over
//! the target's presented rectangle at its aspect and posture (hull-down
//! exposes the declared fraction of the height). Because the bias is drawn
//! once per engagement ([`Engagement`]), misses are correlated; the marginal
//! [`ph_expected`] uses σ² = σ_bias² + σ_random². Acceptance **[P27]**: a
//! modern MBT has Ph > 70% at 2 km against a stationary, fully exposed MBT.
//!
//! **Terminal effects.** Penetration by range from the munition's table
//! (KE falls off beyond it), against the armour of the zone struck (front,
//! side, rear, roof; hull or turret), with ERA and APS; a soft logistic
//! margin; kill types (mobility, firepower, catastrophic, crew) from default
//! vulnerability tables **[default]**.
//!
//! **Small arms against people:** a rate process — rounds per minute × Ph
//! against a person-sized presented area (posture, cover) × P(incapacitated
//! | hit), and near-miss pressure for suppression (§8.10).

use crate::math::{logistic, normal_interval};
use bgws_core::units::G0;
use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};

/// A gun and its ammunition: the ballistic facts the error budget needs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gun {
    pub muzzle_velocity_mps: f64,
    /// Velocity decay coefficient k in v(x) = v₀·e^(−kx), 1/m.
    pub drag_k_per_m: f64,
    /// Round-to-round dispersion, one sigma per axis, mil.
    pub dispersion_mil: f64,
}

/// The fire-control system's residual errors (one sigma each).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FireControl {
    pub range_error_frac: f64,
    pub range_error_m: f64,
    pub crosswind_mps: f64,
    pub cant_rad: f64,
    pub boresight_mil: f64,
    pub jump_mil: f64,
    pub lay_mil: f64,
    pub speed_error_frac: f64,
    pub speed_error_floor_mps: f64,
    /// 0 none … 1 perfect stabilisation of a moving firer.
    pub stabilisation: f64,
}

/// The firing situation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub range_m: f64,
    pub firer_moving: bool,
    /// Target speed across the line of fire, m/s.
    pub target_crossing_mps: f64,
    /// ≥ 1: worse crews have larger lay and estimation errors (from `people::Skill`).
    pub skill_mult: f64,
    /// Firer suppression 0..1.
    pub suppression: f64,
}

/// The engagement's bias components, metres at the target (for the AAR).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BiasComponents {
    pub range_estimation_y: f64,
    pub crosswind_x: f64,
    pub cant_x: f64,
    pub boresight_jump: f64,
    pub lead_x: f64,
}

/// The full error budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBudget {
    pub time_of_flight_s: f64,
    pub drop_m: f64,
    pub components: BiasComponents,
    pub bias_sigma_x_m: f64,
    pub bias_sigma_y_m: f64,
    pub random_sigma_x_m: f64,
    pub random_sigma_y_m: f64,
}

impl ErrorBudget {
    pub fn total_sigma(&self) -> (f64, f64) {
        (self.bias_sigma_x_m.hypot(self.random_sigma_x_m), self.bias_sigma_y_m.hypot(self.random_sigma_y_m))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectFireParams {
    pub fc_basic: FireControl,
    pub fc_modern: FireControl,
    pub mbt_gun: Gun,
    pub rifle: Gun,
    pub moving_firer_mil: f64,
    pub suppression_error_k: f64,
    pub correction_after_miss: f64,
    pub hull_down_fraction: f64,
    pub person_width_m: f64,
    pub person_height_m: [f64; 3],
    pub ke_falloff_per_km: f64,
    pub pen_spread: f64,
    pub era_ke_reduction: f64,
    pub era_ce_reduction: f64,
    pub era_tandem_residual: f64,
    pub aps_p_intercept: f64,
    pub aps_max_elevation_deg: f64,
    pub overmatch_ratio: f64,
    pub marginal_kill_factor: f64,
    /// [zone][catastrophic, firepower, mobility, crew]
    pub kill_table: [[f64; 4]; 6],
    pub small_arms_p_incap: f64,
    pub small_arms_aimed_fraction: f64,
    pub near_miss_radius_m: f64,
}

fn fc_from(f: &Figures, which: &str) -> FireControl {
    let g = |k: &str| f.get(&format!("direct.fc.{which}.{k}"));
    FireControl {
        range_error_frac: g("range_error_frac"),
        range_error_m: g("range_error_m"),
        crosswind_mps: g("crosswind_mps"),
        cant_rad: g("cant_deg").to_radians(),
        boresight_mil: g("boresight_mil"),
        jump_mil: g("jump_mil"),
        lay_mil: g("lay_mil"),
        speed_error_frac: g("speed_error_frac"),
        speed_error_floor_mps: g("speed_error_floor_mps"),
        stabilisation: g("stabilisation"),
    }
}

impl DirectFireParams {
    pub fn from_figures(f: &Figures) -> Self {
        let gun = |w: &str| Gun {
            muzzle_velocity_mps: f.get(&format!("direct.gun.{w}.v0_mps")),
            drag_k_per_m: f.get(&format!("direct.gun.{w}.drag_k_per_m")),
            dispersion_mil: f.get(&format!("direct.gun.{w}.dispersion_mil")),
        };
        DirectFireParams {
            fc_basic: fc_from(f, "basic"),
            fc_modern: fc_from(f, "modern"),
            mbt_gun: gun("mbt"),
            rifle: gun("rifle"),
            moving_firer_mil: f.get("direct.moving_firer_mil"),
            suppression_error_k: f.get("direct.suppression_error_k"),
            correction_after_miss: f.get("direct.correction_after_miss"),
            hull_down_fraction: f.get("direct.hull_down_fraction"),
            person_width_m: f.get("direct.person.width_m"),
            person_height_m: ["standing", "kneeling", "prone"].map(|p| f.get(&format!("direct.person.height_m.{p}"))),
            ke_falloff_per_km: f.get("direct.ke_falloff_per_km"),
            pen_spread: f.get("direct.pen_spread"),
            era_ke_reduction: f.get("direct.era.ke_reduction"),
            era_ce_reduction: f.get("direct.era.ce_reduction"),
            era_tandem_residual: f.get("direct.era.tandem_residual"),
            aps_p_intercept: f.get("direct.aps.p_intercept"),
            aps_max_elevation_deg: f.get("direct.aps.max_elevation_deg"),
            overmatch_ratio: f.get("direct.overmatch_ratio"),
            marginal_kill_factor: f.get("direct.kill.marginal_factor"),
            kill_table: Zone::ALL.map(|z| ["catastrophic", "firepower", "mobility", "crew"].map(|k| f.get(&format!("direct.kill.{}.{k}", z.key())))),
            small_arms_p_incap: f.get("direct.small_arms.p_incap_per_hit"),
            small_arms_aimed_fraction: f.get("direct.small_arms.aimed_fraction"),
            near_miss_radius_m: f.get("direct.small_arms.near_miss_radius_m"),
        }
    }
}

impl FireControl {
    /// Interpolate every residual between the basic (q = 0) and modern (q = 1) systems.
    pub fn from_quality(q: f64, p: &DirectFireParams) -> FireControl {
        let q = q.clamp(0.0, 1.0);
        let (a, b) = (&p.fc_basic, &p.fc_modern);
        let l = |x: f64, y: f64| x + (y - x) * q;
        FireControl {
            range_error_frac: l(a.range_error_frac, b.range_error_frac),
            range_error_m: l(a.range_error_m, b.range_error_m),
            crosswind_mps: l(a.crosswind_mps, b.crosswind_mps),
            cant_rad: l(a.cant_rad, b.cant_rad),
            boresight_mil: l(a.boresight_mil, b.boresight_mil),
            jump_mil: l(a.jump_mil, b.jump_mil),
            lay_mil: l(a.lay_mil, b.lay_mil),
            speed_error_frac: l(a.speed_error_frac, b.speed_error_frac),
            speed_error_floor_mps: l(a.speed_error_floor_mps, b.speed_error_floor_mps),
            stabilisation: l(a.stabilisation, b.stabilisation),
        }
    }
}

/// Time of flight with v = v₀e^(−kx): t = (e^(kR) − 1)/(k v₀).
pub fn time_of_flight(gun: &Gun, range_m: f64) -> f64 {
    let k = gun.drag_k_per_m;
    if k <= 0.0 {
        return range_m / gun.muzzle_velocity_mps;
    }
    (k * range_m).exp_m1() / (k * gun.muzzle_velocity_mps)
}

/// The error budget for one shot.
pub fn error_budget(gun: &Gun, fc: &FireControl, shot: &Shot, p: &DirectFireParams) -> ErrorBudget {
    let r = shot.range_m.max(1.0);
    let mil = r / 1000.0; // metres per mil at this range
    let tof = time_of_flight(gun, r);
    let v_impact = gun.muzzle_velocity_mps * (-gun.drag_k_per_m * r).exp();
    let drop = 0.5 * G0 * tof * tof;
    let nerve = shot.skill_mult.max(1.0) * (1.0 + p.suppression_error_k * shot.suppression.clamp(0.0, 1.0));
    // Bias: per engagement.
    let range_sigma = fc.range_error_frac * r + fc.range_error_m;
    let range_estimation_y = range_sigma * G0 * tof / v_impact;
    let crosswind_x = fc.crosswind_mps * (tof - r / gun.muzzle_velocity_mps).max(0.0);
    let cant_x = drop * fc.cant_rad.sin();
    let boresight_jump = fc.boresight_mil.hypot(fc.jump_mil) * mil;
    let speed_sigma = (fc.speed_error_frac * shot.target_crossing_mps.abs()).max(if shot.target_crossing_mps.abs() > 0.0 { fc.speed_error_floor_mps } else { 0.0 });
    let lead_x = speed_sigma * tof * nerve;
    let bx = (crosswind_x.powi(2) + cant_x.powi(2) + boresight_jump.powi(2) + lead_x.powi(2)).sqrt();
    let by = (range_estimation_y.powi(2) + boresight_jump.powi(2)).sqrt();
    // Random: per round.
    let moving = if shot.firer_moving { p.moving_firer_mil * (1.0 - fc.stabilisation.clamp(0.0, 1.0)) } else { 0.0 };
    let rand_mil = (gun.dispersion_mil.powi(2) + (fc.lay_mil * nerve).powi(2) + moving.powi(2)).sqrt();
    ErrorBudget {
        time_of_flight_s: tof,
        drop_m: drop,
        components: BiasComponents { range_estimation_y, crosswind_x, cant_x, boresight_jump, lead_x },
        bias_sigma_x_m: bx,
        bias_sigma_y_m: by,
        random_sigma_x_m: rand_mil * mil,
        random_sigma_y_m: rand_mil * mil,
    }
}

/// A vehicle's dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VehicleDims {
    pub length_m: f64,
    pub width_m: f64,
    pub hull_height_m: f64,
    pub turret_height_m: f64,
}

/// Vehicle posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehiclePosture {
    Exposed,
    HullDown,
}

/// A person's posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonPosture {
    Standing,
    Kneeling,
    Prone,
}

/// What the gunner sees: a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presented {
    pub width_m: f64,
    pub height_m: f64,
    /// Fraction of the presented width that is the front/rear face (rest is side).
    pub face_fraction: f64,
    /// Fraction of the presented height (from the top) that is turret.
    pub turret_fraction: f64,
    /// True when the facing face is the rear.
    pub rear: bool,
}

/// The presented rectangle of a vehicle seen with `aspect_rad` between its
/// facing and the direction to the firer (0 = head-on, π/2 = broadside, π = tail).
pub fn presented_vehicle(d: &VehicleDims, aspect_rad: f64, posture: VehiclePosture, p: &DirectFireParams) -> Presented {
    let face = d.width_m * aspect_rad.cos().abs();
    let side = d.length_m * aspect_rad.sin().abs();
    let full = d.hull_height_m + d.turret_height_m;
    let height = match posture {
        VehiclePosture::Exposed => full,
        VehiclePosture::HullDown => full * p.hull_down_fraction,
    };
    Presented {
        width_m: face + side,
        height_m: height,
        face_fraction: if face + side > 0.0 { face / (face + side) } else { 1.0 },
        turret_fraction: (d.turret_height_m / height.max(1e-9)).min(1.0),
        rear: aspect_rad.cos() < 0.0,
    }
}

/// A person's presented rectangle, with `cover` (0..1) of the height hidden.
pub fn presented_person(posture: PersonPosture, cover: f64, p: &DirectFireParams) -> Presented {
    Presented {
        width_m: p.person_width_m,
        height_m: p.person_height_m[posture as usize] * (1.0 - cover.clamp(0.0, 1.0)),
        face_fraction: 1.0,
        turret_fraction: 0.0,
        rear: false,
    }
}

/// P(hit) for one round given the engagement bias (μx, μy), aimed at the centre.
pub fn ph_given_bias(pres: &Presented, bias: (f64, f64), b: &ErrorBudget) -> f64 {
    normal_interval(-pres.width_m / 2.0, pres.width_m / 2.0, bias.0, b.random_sigma_x_m)
        * normal_interval(-pres.height_m / 2.0, pres.height_m / 2.0, bias.1, b.random_sigma_y_m)
}

/// The marginal first-round Ph (bias integrated out).
pub fn ph_expected(pres: &Presented, b: &ErrorBudget) -> f64 {
    let (sx, sy) = b.total_sigma();
    normal_interval(-pres.width_m / 2.0, pres.width_m / 2.0, 0.0, sx) * normal_interval(-pres.height_m / 2.0, pres.height_m / 2.0, 0.0, sy)
}

/// One engagement: a bias drawn once, so misses are correlated, and
/// corrected after each observed miss.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Engagement {
    pub bias: (f64, f64),
    pub rounds: u32,
    pub hits: u32,
}

/// One round's fall of shot relative to the aim point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundResult {
    pub hit: bool,
    pub x_m: f64,
    pub y_m: f64,
    /// Where on the presented rectangle (0..1 across, 0..1 from the top) a hit landed.
    pub x_frac: f64,
    pub y_frac: f64,
    pub ph: f64,
}

impl Engagement {
    /// Draw the engagement's fixed bias.
    pub fn new(b: &ErrorBudget, rng: &mut impl Rng) -> Self {
        let bx = Normal::new(0.0, b.bias_sigma_x_m.max(1e-12)).map(|n| n.sample(rng)).unwrap_or(0.0);
        let by = Normal::new(0.0, b.bias_sigma_y_m.max(1e-12)).map(|n| n.sample(rng)).unwrap_or(0.0);
        Engagement { bias: (bx, by), rounds: 0, hits: 0 }
    }

    pub fn ph(&self, pres: &Presented, b: &ErrorBudget) -> f64 {
        ph_given_bias(pres, self.bias, b)
    }

    /// Fire one round: always two draws in a fixed order (replay-exact).
    pub fn fire(&mut self, pres: &Presented, b: &ErrorBudget, p: &DirectFireParams, rng: &mut impl Rng) -> RoundResult {
        let ph = self.ph(pres, b);
        let nx = Normal::new(0.0, b.random_sigma_x_m.max(1e-12)).map(|n| n.sample(rng)).unwrap_or(0.0);
        let ny = Normal::new(0.0, b.random_sigma_y_m.max(1e-12)).map(|n| n.sample(rng)).unwrap_or(0.0);
        let (x, y) = (self.bias.0 + nx, self.bias.1 + ny);
        let hit = x.abs() <= pres.width_m / 2.0 && y.abs() <= pres.height_m / 2.0;
        self.rounds += 1;
        if hit {
            self.hits += 1;
        } else {
            // The crew sees the miss and corrects part of the bias.
            self.bias.0 *= 1.0 - p.correction_after_miss;
            self.bias.1 *= 1.0 - p.correction_after_miss;
        }
        RoundResult { hit, x_m: x, y_m: y, x_frac: (x / pres.width_m.max(1e-9) + 0.5).clamp(0.0, 1.0), y_frac: (0.5 - y / pres.height_m.max(1e-9)).clamp(0.0, 1.0), ph }
    }
}

/// Armour zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    HullFront,
    TurretFront,
    HullSide,
    TurretSide,
    Rear,
    Roof,
}

impl Zone {
    pub const ALL: [Zone; 6] = [Zone::HullFront, Zone::TurretFront, Zone::HullSide, Zone::TurretSide, Zone::Rear, Zone::Roof];
    fn key(self) -> &'static str {
        match self {
            Zone::HullFront => "hull_front",
            Zone::TurretFront => "turret_front",
            Zone::HullSide => "hull_side",
            Zone::TurretSide => "turret_side",
            Zone::Rear => "rear",
            Zone::Roof => "roof",
        }
    }
}

/// The zone a hit at (x_frac, y_frac) on the presented rectangle strikes.
/// The face occupies the first `face_fraction` of the width (rear when tail-on).
pub fn zone_at(pres: &Presented, x_frac: f64, y_frac: f64, top_attack: bool) -> Zone {
    if top_attack {
        return Zone::Roof;
    }
    let turret = y_frac < pres.turret_fraction;
    if x_frac < pres.face_fraction {
        if pres.rear {
            Zone::Rear
        } else if turret {
            Zone::TurretFront
        } else {
            Zone::HullFront
        }
    } else if turret {
        Zone::TurretSide
    } else {
        Zone::HullSide
    }
}

/// Zone probabilities for a hit uniformly distributed over the presented rectangle.
pub fn zone_probabilities(pres: &Presented, top_attack: bool) -> Vec<(Zone, f64)> {
    if top_attack {
        return vec![(Zone::Roof, 1.0)];
    }
    let (f, t) = (pres.face_fraction, pres.turret_fraction);
    let mut v = Vec::new();
    if pres.rear {
        v.push((Zone::Rear, f));
    } else {
        v.push((Zone::TurretFront, f * t));
        v.push((Zone::HullFront, f * (1.0 - t)));
    }
    v.push((Zone::TurretSide, (1.0 - f) * t));
    v.push((Zone::HullSide, (1.0 - f) * (1.0 - t)));
    v.retain(|(_, p)| *p > 1e-12);
    v
}

/// Warhead type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Warhead {
    Kinetic,
    ShapedCharge,
    TandemShapedCharge,
}

/// Penetration (mm RHAe) by range; KE beyond the table falls off per km.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PenetrationTable {
    pub warhead: Warhead,
    /// (range m, penetration mm), sorted by range.
    pub points: Vec<(f64, f64)>,
}

impl PenetrationTable {
    pub fn at(&self, range_m: f64, p: &DirectFireParams) -> f64 {
        let Some(last) = self.points.last() else { return 0.0 };
        if range_m > last.0 && self.warhead == Warhead::Kinetic {
            return last.1 * (1.0 - (range_m - last.0) / 1000.0 * p.ke_falloff_per_km).max(0.0);
        }
        crate::math::interp(&self.points, range_m).unwrap_or(0.0)
    }
}

/// Armour by zone (mm RHAe) against KE and against chemical energy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArmourZones {
    pub ke_mm: [f64; 6],
    pub ce_mm: [f64; 6],
}

/// Add-on protection.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Protection {
    /// ERA on the front and sides.
    pub era: bool,
    /// An active protection system (hard kill against missiles and rockets).
    pub aps: bool,
}

/// Kill-type probabilities given a penetration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillProbs {
    pub catastrophic: f64,
    pub firepower: f64,
    pub mobility: f64,
    pub crew: f64,
}

/// The odds of one strike on one zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrikeOdds {
    pub zone: Zone,
    pub penetration_mm: f64,
    pub armour_mm: f64,
    pub p_intercept: f64,
    pub p_penetrate: f64,
    pub kill: KillProbs,
    /// P(any kill) for one strike: (1 − intercept)·penetrate·(1 − Π(1 − p_k)).
    pub p_any_kill: f64,
}

/// What a vehicle suffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KillType {
    pub catastrophic: bool,
    pub firepower: bool,
    pub mobility: bool,
    pub crew: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrikeOutcome {
    Intercepted,
    NoPenetration,
    Penetrated(KillType),
}

/// The odds of a strike on `zone` with a round of `penetration_mm`, arriving at
/// `attack_elevation_deg` (for the APS elevation limit **[R4.2]**).
pub fn strike_odds(warhead: Warhead, penetration_mm: f64, zone: Zone, armour: &ArmourZones, prot: &Protection, attack_elevation_deg: f64, p: &DirectFireParams) -> StrikeOdds {
    let z = zone as usize;
    let ce = warhead != Warhead::Kinetic;
    let base = if ce { armour.ce_mm[z] } else { armour.ke_mm[z] };
    // ERA on front and side zones degrades the penetrator; a tandem precursor strips most of it.
    let era_applies = prot.era && !matches!(zone, Zone::Rear | Zone::Roof);
    let pen = if era_applies {
        let red = match warhead {
            Warhead::Kinetic => p.era_ke_reduction,
            Warhead::ShapedCharge => p.era_ce_reduction,
            Warhead::TandemShapedCharge => p.era_ce_reduction * p.era_tandem_residual,
        };
        penetration_mm * (1.0 - red)
    } else {
        penetration_mm
    };
    let p_intercept = if prot.aps && ce && attack_elevation_deg <= p.aps_max_elevation_deg { p.aps_p_intercept } else { 0.0 };
    let p_pen = if base <= 0.0 { 1.0 } else { logistic((pen - base) / (p.pen_spread * base)) };
    let over = if base <= 0.0 { 1.0 } else { ((pen / base - 1.0) / (p.overmatch_ratio - 1.0)).clamp(0.0, 1.0) };
    let row = p.kill_table[z];
    let scale = p.marginal_kill_factor + (1.0 - p.marginal_kill_factor) * over;
    let kill = KillProbs { catastrophic: row[0] * scale, firepower: row[1], mobility: row[2], crew: row[3] };
    let none = (1.0 - kill.catastrophic) * (1.0 - kill.firepower) * (1.0 - kill.mobility) * (1.0 - kill.crew);
    StrikeOdds { zone, penetration_mm: pen, armour_mm: base, p_intercept, p_penetrate: p_pen, kill, p_any_kill: (1.0 - p_intercept) * p_pen * (1.0 - none) }
}

/// Roll a strike: always six uniform draws in a fixed order.
pub fn roll_strike(o: &StrikeOdds, rng: &mut impl Rng) -> StrikeOutcome {
    let u: [f64; 6] = std::array::from_fn(|_| rng.gen::<f64>());
    if u[0] < o.p_intercept {
        return StrikeOutcome::Intercepted;
    }
    if u[1] >= o.p_penetrate {
        return StrikeOutcome::NoPenetration;
    }
    let catastrophic = u[2] < o.kill.catastrophic;
    StrikeOutcome::Penetrated(KillType {
        catastrophic,
        firepower: catastrophic || u[3] < o.kill.firepower,
        mobility: catastrophic || u[4] < o.kill.mobility,
        crew: catastrophic || u[5] < o.kill.crew,
    })
}

/// Small-arms fire on a group of people.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmallArmsFire {
    pub rounds_per_min: f64,
    pub range_m: f64,
    pub skill_mult: f64,
    pub suppression: f64,
    /// Sights: 0 iron … 1 magnified optic with a rangefinder.
    pub sight_quality: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeopleTarget {
    pub count: u32,
    pub posture: PersonPosture,
    /// Fraction of the body behind hard cover, 0..1.
    pub cover: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmallArmsEffect {
    /// Ph of one aimed round against one visible person.
    pub ph_per_round: f64,
    /// Incapacitation hazard per person, 1/s (use with `hazard_probability`).
    pub casualty_rate_per_person: f64,
    /// Expected casualties over one minute of this fire.
    pub casualties_per_min: f64,
    /// Rounds per minute passing within the near-miss radius of someone (suppression pressure).
    pub near_misses_per_min: f64,
}

/// The casualty and suppression process from small-arms fire.
pub fn small_arms_effect(fire: &SmallArmsFire, target: &PeopleTarget, p: &DirectFireParams) -> SmallArmsEffect {
    if target.count == 0 {
        return SmallArmsEffect { ph_per_round: 0.0, casualty_rate_per_person: 0.0, casualties_per_min: 0.0, near_misses_per_min: 0.0 };
    }
    let fc = FireControl::from_quality(fire.sight_quality, p);
    let shot = Shot { range_m: fire.range_m, firer_moving: false, target_crossing_mps: 0.0, skill_mult: fire.skill_mult, suppression: fire.suppression };
    let b = error_budget(&p.rifle, &fc, &shot, p);
    let pres = presented_person(target.posture, target.cover, p);
    let ph = ph_expected(&pres, &b);
    let (sx, sy) = b.total_sigma();
    let r = p.near_miss_radius_m;
    let p_near = normal_interval(-r, r, 0.0, sx) * normal_interval(-r, r, 0.0, sy) - ph;
    // Most fire is at positions, not at men in the sight: only the aimed fraction can hit.
    let n = target.count as f64;
    let rate = fire.rounds_per_min / 60.0 * p.small_arms_aimed_fraction * ph * p.small_arms_p_incap / n;
    let cas = n * bgws_core::hazard_probability(rate, 60.0);
    SmallArmsEffect { ph_per_round: ph, casualty_rate_per_person: rate, casualties_per_min: cas, near_misses_per_min: fire.rounds_per_min * p_near.max(0.0) }
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("direct.gun.mbt.v0_mps", 1700.0, "m/s", 1400.0, 1900.0, "default", Default, "120/125 mm APFSDS muzzle velocity"),
        figure!("direct.gun.mbt.drag_k_per_m", 4.0e-5, "1/m", 1.0e-5, 1.5e-4, "default", Default, "APFSDS velocity decay (~60 m/s per km)"),
        figure!("direct.gun.mbt.dispersion_mil", 0.25, "mil", 0.1, 0.6, "P27", Low, "tank gun round-to-round dispersion"),
        figure!("direct.gun.rifle.v0_mps", 900.0, "m/s", 700.0, 1000.0, "default", Default, "5.56/7.62 mm muzzle velocity"),
        figure!("direct.gun.rifle.drag_k_per_m", 1.2e-3, "1/m", 5e-4, 3e-3, "default", Default, "rifle bullet velocity decay"),
        figure!("direct.gun.rifle.dispersion_mil", 2.0, "mil", 0.5, 6.0, "default", Default, "combat rifle/automatic fire dispersion"),
        figure!("direct.moving_firer_mil", 2.0, "mil", 0.5, 6.0, "default", Default, "added dispersion firing on the move, unstabilised"),
        figure!("direct.suppression_error_k", 2.0, "1", 0.5, 9.0, "P31", Low, "lay error multiplies by 1 + k·S (accuracy −50–90%)"),
        figure!("direct.correction_after_miss", 0.5, "fraction", 0.0, 0.9, "default", Default, "fraction of bias the crew corrects after an observed miss"),
        figure!("direct.hull_down_fraction", 0.5, "fraction", 0.3, 0.7, "default", Default, "hull-down halves the exposed height (plan §8.8)"),
        figure!("direct.person.width_m", 0.5, "m", 0.3, 0.7, "default", Default, "person presented width"),
        figure!("direct.person.height_m.standing", 1.75, "m", 1.5, 1.9, "default", Default, "standing height"),
        figure!("direct.person.height_m.kneeling", 1.0, "m", 0.8, 1.2, "default", Default, "kneeling height"),
        figure!("direct.person.height_m.prone", 0.35, "m", 0.2, 0.5, "default", Default, "prone presented height"),
        figure!("direct.ke_falloff_per_km", 0.06, "1/km", 0.02, 0.15, "default", Default, "KE penetration loss per km beyond the table"),
        figure!("direct.pen_spread", 0.12, "fraction", 0.03, 0.3, "default", Default, "softness of the penetration margin"),
        figure!("direct.era.ke_reduction", 0.2, "fraction", 0.0, 0.5, "default", Default, "heavy ERA against long rods"),
        figure!("direct.era.ce_reduction", 0.6, "fraction", 0.2, 0.9, "default", Default, "ERA against single shaped charges"),
        figure!("direct.era.tandem_residual", 0.1, "fraction", 0.0, 0.5, "default", Default, "ERA effect left against a tandem warhead"),
        figure!("direct.aps.p_intercept", 0.8, "fraction", 0.3, 0.99, "default", Default, "APS hard-kill probability per missile/rocket"),
        figure!("direct.aps.max_elevation_deg", 55.0, "deg", 30.0, 90.0, "R4.2", Medium, "Trophy elevation limit: steep FPVs uncovered"),
        figure!("direct.overmatch_ratio", 1.5, "1", 1.1, 3.0, "default", Default, "penetration/armour counted as full overmatch"),
        figure!("direct.kill.marginal_factor", 0.5, "fraction", 0.1, 1.0, "default", Default, "catastrophic kill scale for a marginal penetration"),
        figure!("direct.small_arms.p_incap_per_hit", 0.7, "fraction", 0.3, 1.0, "default", Default, "a bullet hit incapacitates"),
        figure!("direct.small_arms.aimed_fraction", 0.1, "fraction", 0.01, 1.0, "default", Default, "fraction of rounds aimed at a visible man (the rest is area fire)"),
        figure!("direct.small_arms.near_miss_radius_m", 3.0, "m", 1.0, 10.0, "default", Default, "a round this close suppresses"),
    ];
    let fc = [
        ("range_error_frac", 0.10, 0.0, "fraction", 0.0, 0.3, "range estimation σ as a fraction of range"),
        ("range_error_m", 0.0, 5.0, "m", 0.0, 50.0, "range estimation σ, absolute (laser)"),
        ("crosswind_mps", 4.0, 1.5, "m/s", 0.0, 10.0, "residual crosswind σ"),
        ("cant_deg", 3.0, 0.5, "deg", 0.0, 10.0, "residual cant σ"),
        ("boresight_mil", 0.4, 0.15, "mil", 0.0, 2.0, "boresight/zeroing σ"),
        ("jump_mil", 0.3, 0.2, "mil", 0.0, 2.0, "ammunition lot/jump bias σ"),
        ("lay_mil", 0.4, 0.15, "mil", 0.0, 3.0, "gunner lay σ per round"),
        ("speed_error_frac", 0.3, 0.1, "fraction", 0.0, 1.0, "target speed estimate σ"),
        ("speed_error_floor_mps", 1.0, 0.3, "m/s", 0.0, 5.0, "floor of the target speed estimate σ"),
        ("stabilisation", 0.0, 0.9, "fraction", 0.0, 1.0, "fraction of moving-firer dispersion removed"),
    ];
    for (k, basic, modern, unit, lo, hi, note) in fc {
        v.push(figure!(format!("direct.fc.basic.{k}"), basic, unit, lo, hi, "default", Default, format!("basic fire control: {note}")));
        v.push(figure!(format!("direct.fc.modern.{k}"), modern, unit, lo, hi, "P27", Low, format!("modern fire control: {note}")));
    }
    // Default vulnerability table: P(kill type | penetration) by zone.
    let kt = [
        ("hull_front", [0.35, 0.3, 0.5, 0.5]),
        ("turret_front", [0.45, 0.7, 0.1, 0.6]),
        ("hull_side", [0.5, 0.3, 0.6, 0.5]),
        ("turret_side", [0.6, 0.7, 0.1, 0.7]),
        ("rear", [0.4, 0.2, 0.8, 0.3]),
        ("roof", [0.6, 0.6, 0.2, 0.7]),
    ];
    for (z, vals) in kt {
        for (k, val) in ["catastrophic", "firepower", "mobility", "crew"].iter().zip(vals) {
            v.push(figure!(format!("direct.kill.{z}.{k}"), val, "fraction", 0.0, 1.0, "default", Default, "vulnerability table: P(kill type | penetration)"));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{figs, rng};
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn mbt() -> VehicleDims {
        VehicleDims { length_m: 7.9, width_m: 3.7, hull_height_m: 1.5, turret_height_m: 0.9 }
    }
    fn still(range: f64) -> Shot {
        Shot { range_m: range, firer_moving: false, target_crossing_mps: 0.0, skill_mult: 1.0, suppression: 0.0 }
    }

    #[test]
    fn acceptance_p27_modern_mbt_ph_at_2km() {
        let p = DirectFireParams::from_figures(&figs());
        let fc = FireControl::from_quality(1.0, &p);
        let b = error_budget(&p.mbt_gun, &fc, &still(2000.0), &p);
        let pres = presented_vehicle(&mbt(), 0.0, VehiclePosture::Exposed, &p);
        let ph = ph_expected(&pres, &b);
        assert!(ph > 0.7, "P27: Ph {ph} at 2 km, budget {b:?}");
        // Monte Carlo with per-engagement bias agrees with the marginal.
        let mut r = rng(7);
        let n = 20_000;
        let hits = (0..n).filter(|_| Engagement::new(&b, &mut r).fire(&pres, &b, &p, &mut r).hit).count();
        assert_relative_eq!(hits as f64 / n as f64, ph, epsilon = 0.015);
        // A basic fire-control system is much worse at 2 km.
        let basic = error_budget(&p.mbt_gun, &FireControl::from_quality(0.0, &p), &still(2000.0), &p);
        assert!(ph_expected(&pres, &basic) < 0.4);
        // Hull-down, moving and suppressed all lower it.
        let hd = presented_vehicle(&mbt(), 0.0, VehiclePosture::HullDown, &p);
        assert!(ph_expected(&hd, &b) < ph);
        let moving = error_budget(&p.mbt_gun, &fc, &Shot { firer_moving: true, target_crossing_mps: 8.0, ..still(2000.0) }, &p);
        assert!(ph_expected(&pres, &moving) < ph);
        // Ph falls with range.
        let far = error_budget(&p.mbt_gun, &fc, &still(3500.0), &p);
        assert!(ph_expected(&pres, &far) < ph);
    }

    #[test]
    fn exact_rectangle_integral() {
        let p = DirectFireParams::from_figures(&figs());
        let b = ErrorBudget {
            time_of_flight_s: 1.0,
            drop_m: 0.0,
            components: BiasComponents { range_estimation_y: 0.0, crosswind_x: 0.0, cant_x: 0.0, boresight_jump: 0.0, lead_x: 0.0 },
            bias_sigma_x_m: 0.0,
            bias_sigma_y_m: 0.0,
            random_sigma_x_m: 1.0,
            random_sigma_y_m: 1.0,
        };
        let pres = Presented { width_m: 2.0, height_m: 2.0, face_fraction: 1.0, turret_fraction: 0.0, rear: false };
        assert_relative_eq!(ph_given_bias(&pres, (0.0, 0.0), &b), 0.682_689_5f64.powi(2), epsilon = 1e-6);
        // Bias of one sigma: Φ(0) − Φ(−2) = 0.4772.
        assert_relative_eq!(ph_given_bias(&pres, (1.0, 0.0), &b), 0.477_25 * 0.682_689_5, epsilon = 1e-5);
        let _ = p;
    }

    #[test]
    fn correlated_misses_then_correction() {
        let p = DirectFireParams::from_figures(&figs());
        let fc = FireControl::from_quality(0.3, &p);
        let b = error_budget(&p.mbt_gun, &fc, &still(2500.0), &p);
        let pres = presented_vehicle(&mbt(), 0.0, VehiclePosture::Exposed, &p);
        let mut r = rng(3);
        // Second-round Ph after a miss is higher than first-round on average.
        let (mut first, mut second, mut n2) = (0.0, 0.0, 0.0);
        for _ in 0..3000 {
            let mut e = Engagement::new(&b, &mut r);
            let r1 = e.fire(&pres, &b, &p, &mut r);
            first += r1.ph;
            if !r1.hit {
                second += e.ph(&pres, &b);
                n2 += 1.0;
            }
        }
        assert!(second / n2 > first / 3000.0 * 0.9);
    }

    #[test]
    fn penetration_era_aps_and_zones() {
        let p = DirectFireParams::from_figures(&figs());
        let table = PenetrationTable { warhead: Warhead::Kinetic, points: vec![(0.0, 750.0), (2000.0, 680.0)] };
        assert_relative_eq!(table.at(1000.0, &p), 715.0);
        assert!(table.at(3000.0, &p) < 680.0);
        let armour = ArmourZones { ke_mm: [600.0, 800.0, 80.0, 250.0, 40.0, 40.0], ce_mm: [900.0, 1100.0, 120.0, 350.0, 60.0, 60.0] };
        let none = Protection::default();
        let front = strike_odds(Warhead::Kinetic, 680.0, Zone::HullFront, &armour, &none, 0.0, &p);
        let turret = strike_odds(Warhead::Kinetic, 680.0, Zone::TurretFront, &armour, &none, 0.0, &p);
        let side = strike_odds(Warhead::Kinetic, 680.0, Zone::HullSide, &armour, &none, 0.0, &p);
        assert!(turret.p_penetrate < front.p_penetrate && front.p_penetrate < side.p_penetrate);
        assert!(side.kill.catastrophic > front.kill.catastrophic, "overmatch raises catastrophic kills");
        let era = Protection { era: true, aps: false };
        let atgm = strike_odds(Warhead::ShapedCharge, 400.0, Zone::HullSide, &armour, &era, 0.0, &p);
        let tandem = strike_odds(Warhead::TandemShapedCharge, 400.0, Zone::HullSide, &armour, &era, 0.0, &p);
        assert!(tandem.p_penetrate > atgm.p_penetrate);
        let aps = Protection { era: false, aps: true };
        let flat = strike_odds(Warhead::ShapedCharge, 800.0, Zone::Roof, &armour, &aps, 10.0, &p);
        let dive = strike_odds(Warhead::ShapedCharge, 800.0, Zone::Roof, &armour, &aps, 80.0, &p);
        assert!(flat.p_intercept > 0.0 && dive.p_intercept == 0.0, "steep FPV is above the APS limit");
        let ke = strike_odds(Warhead::Kinetic, 800.0, Zone::Roof, &armour, &aps, 0.0, &p);
        assert_eq!(ke.p_intercept, 0.0);
        // Zones from aspect.
        let broadside = presented_vehicle(&mbt(), std::f64::consts::FRAC_PI_2, VehiclePosture::Exposed, &p);
        let zp = zone_probabilities(&broadside, false);
        let s: f64 = zp.iter().map(|z| z.1).sum();
        assert_relative_eq!(s, 1.0, epsilon = 1e-9);
        assert!(zp.iter().all(|(z, _)| matches!(z, Zone::HullSide | Zone::TurretSide)));
        let tail = presented_vehicle(&mbt(), std::f64::consts::PI, VehiclePosture::Exposed, &p);
        assert_eq!(zone_at(&tail, 0.1, 0.9, false), Zone::Rear);
        let hd = presented_vehicle(&mbt(), 0.0, VehiclePosture::HullDown, &p);
        assert_eq!(zone_at(&hd, 0.5, 0.5, false), Zone::TurretFront);
        // Rolling is deterministic for a given stream.
        let o1 = roll_strike(&side, &mut rng(1));
        let o2 = roll_strike(&side, &mut rng(1));
        assert_eq!(o1, o2);
    }

    #[test]
    fn small_arms_by_range_posture_cover() {
        let p = DirectFireParams::from_figures(&figs());
        let fire = SmallArmsFire { rounds_per_min: 300.0, range_m: 300.0, skill_mult: 1.0, suppression: 0.0, sight_quality: 0.3 };
        let open = PeopleTarget { count: 8, posture: PersonPosture::Standing, cover: 0.0 };
        let prone = PeopleTarget { posture: PersonPosture::Prone, ..open };
        let covered = PeopleTarget { cover: 0.7, ..prone };
        let a = small_arms_effect(&fire, &open, &p);
        let b = small_arms_effect(&fire, &prone, &p);
        let c = small_arms_effect(&fire, &covered, &p);
        assert!(a.casualties_per_min > b.casualties_per_min && b.casualties_per_min > c.casualties_per_min, "{a:?} {b:?} {c:?}");
        assert!(c.near_misses_per_min > 10.0, "fire still suppresses men in cover");
        let far = small_arms_effect(&SmallArmsFire { range_m: 600.0, ..fire }, &open, &p);
        assert!(far.casualties_per_min < a.casualties_per_min);
        assert!(a.casualties_per_min < 8.0);
    }

    proptest! {
        #[test]
        fn ph_in_unit_interval_and_falls_with_range(r in 100.0f64..4000.0, q in 0.0f64..1.0) {
            let p = DirectFireParams::from_figures(&figs());
            let fc = FireControl::from_quality(q, &p);
            let pres = presented_vehicle(&mbt(), 0.3, VehiclePosture::Exposed, &p);
            let a = ph_expected(&pres, &error_budget(&p.mbt_gun, &fc, &still(r), &p));
            let b = ph_expected(&pres, &error_budget(&p.mbt_gun, &fc, &still(r + 200.0), &p));
            prop_assert!((0.0..=1.0).contains(&a));
            prop_assert!(b <= a + 1e-12);
        }
    }
}
