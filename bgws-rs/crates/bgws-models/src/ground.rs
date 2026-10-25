//! Ground movement (RUST_ENGINE.md §8.1 "Ground").
//!
//! - **Speed** = min(road speed, trafficability speed, formation speed, order
//!   mode). Trafficability follows the NATO Reference Mobility Model's shape:
//!   a soil-strength go/no-go against the vehicle cone index, a traction
//!   limit on grade, a vegetation override and an obstacle-spacing limit, by
//!   vehicle class **[default: simplified NRMM-style curves]**.
//! - **Fuel burn** by vehicle, speed, terrain resistance and idling **[P17]**.
//! - **Dismounted pace** from the Pandolf load-carriage equation under a
//!   sustainable metabolic ceiling, capped by Tobler's hiking function, and
//!   slowed by fatigue **[P26]**.
//! - **Formations** (spacing by class) and a **bounding overwatch** state
//!   machine that alternates movers and watchers.
//! - **Fording** by depth and bank slope; **bridges** by military load class.

use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// Mobility class: sets the trafficability curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VehicleClass {
    Tracked,
    Wheeled,
    Foot,
}

impl VehicleClass {
    fn key(self) -> &'static str {
        match self {
            VehicleClass::Tracked => "tracked",
            VehicleClass::Wheeled => "wheeled",
            VehicleClass::Foot => "foot",
        }
    }
    const ALL: [VehicleClass; 3] = [VehicleClass::Tracked, VehicleClass::Wheeled, VehicleClass::Foot];
}

/// How the order says to move: a cap as a fraction of the mover's top speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoveMode {
    /// Administrative march on roads.
    March,
    /// Tactical movement, using cover.
    Tactical,
    /// The assault: as fast as the ground allows.
    Assault,
    /// Stealthy: slow and quiet.
    Stealth,
}

/// One terrain cell as the trafficability model sees it (computed by the
/// environment crate from DEM, soil, wetness and land cover).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainCell {
    /// Slope along the direction of travel, degrees (positive uphill).
    pub slope_deg: f64,
    /// Soil strength as a rating cone index (RCI). Dry, firm ground ≳ 100;
    /// saturated clay ≲ 30.
    pub rci: f64,
    /// Vegetation resistance, 0 (open) to 1 (dense forest a vehicle must override).
    pub vegetation: f64,
    /// Mean spacing between obstacles that force a detour (trees, boulders,
    /// ruins), metres. Use a large value for open ground.
    pub obstacle_spacing_m: f64,
    /// True when the route runs on a road.
    pub on_road: bool,
}

/// The mover's own limits (from its catalogue record).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoverSpec {
    pub class: VehicleClass,
    /// Top speed on a road, m/s.
    pub road_speed_mps: f64,
    /// Top speed cross-country on ideal ground, m/s.
    pub cross_country_speed_mps: f64,
}

/// Which limit set the speed: for the AAR ("slowed by mud").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedLimit {
    Road,
    CrossCountry,
    Soil,
    Slope,
    Vegetation,
    Obstacles,
    Formation,
    OrderMode,
    NoGo,
}

/// The trafficability result, with every component limit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trafficability {
    pub go: bool,
    pub soil_mps: f64,
    pub slope_mps: f64,
    pub vegetation_mps: f64,
    pub obstacle_mps: f64,
    /// min of the above and the mover's top speed for the surface.
    pub speed_mps: f64,
    pub limit: SpeedLimit,
}

/// The final speed with its limiting factor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedResult {
    pub speed_mps: f64,
    pub limit: SpeedLimit,
    pub trafficability: Trafficability,
}

/// Parameters of the movement model, read from the figures registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundParams {
    /// Vehicle cone index (minimum RCI for one pass), by class.
    pub vci: [f64; 3],
    /// RCI above which soil does not limit speed.
    pub rci_good: f64,
    /// Steepest grade (rise/run) the class can climb.
    pub max_grade: [f64; 3],
    pub slope_exponent: f64,
    /// Fractional speed loss at vegetation resistance 1, by class.
    pub veg_k: [f64; 3],
    /// Obstacle-limited speed = k·√spacing, by class (m/s per √m).
    pub obstacle_k: [f64; 3],
    /// Order-mode caps: march, tactical, assault, stealth.
    pub mode_cap: [f64; 4],
    pub formation_speed_factor: f64,
    pub spacing_m: [f64; 3],
    pub march_spacing_factor: f64,
    pub fuel_resistance: [f64; 3],
    pub mbt_idle_lph: f64,
    pub mbt_xc_lph: f64,
    pub mbt_ref_speed_mps: f64,
    pub foot_body_kg: f64,
    pub foot_metabolic_cap_w: f64,
    pub foot_tobler_max_kmh: f64,
    pub foot_eta: [f64; 5],
    pub foot_fatigue_cap_loss: f64,
    pub bound_length_m: f64,
    pub bound_overwatch_min_s: f64,
    pub bound_settle_s: f64,
    pub ford_speed_mps: f64,
    pub ford_bank_factor: f64,
    pub ford_current_limit_mps: f64,
    pub bridge_caution_factor: f64,
}

impl GroundParams {
    pub fn from_figures(f: &Figures) -> Self {
        let by_class = |stem: &str| VehicleClass::ALL.map(|c| f.get(&format!("ground.{stem}.{}", c.key())));
        GroundParams {
            vci: by_class("vci"),
            rci_good: f.get("ground.rci_good"),
            max_grade: by_class("max_grade"),
            slope_exponent: f.get("ground.slope_exponent"),
            veg_k: by_class("veg_k"),
            obstacle_k: by_class("obstacle_k"),
            mode_cap: ["march", "tactical", "assault", "stealth"].map(|m| f.get(&format!("ground.mode.{m}"))),
            formation_speed_factor: f.get("ground.formation.speed_factor"),
            spacing_m: by_class("formation.spacing"),
            march_spacing_factor: f.get("ground.formation.march_spacing_factor"),
            fuel_resistance: ["road", "cross_country", "soft"].map(|s| f.get(&format!("ground.fuel.resistance.{s}"))),
            mbt_idle_lph: f.get("ground.fuel.mbt_idle_lph"),
            mbt_xc_lph: f.get("ground.fuel.mbt_xc_lph"),
            mbt_ref_speed_mps: f.get("ground.fuel.mbt_ref_speed_mps"),
            foot_body_kg: f.get("ground.foot.body_mass_kg"),
            foot_metabolic_cap_w: f.get("ground.foot.metabolic_cap_w"),
            foot_tobler_max_kmh: f.get("ground.foot.tobler_max_kmh"),
            foot_eta: ["road", "dirt", "light_brush", "heavy_brush", "swamp"].map(|s| f.get(&format!("ground.foot.eta.{s}"))),
            foot_fatigue_cap_loss: f.get("ground.foot.fatigue_cap_loss"),
            bound_length_m: f.get("ground.bound.length_m"),
            bound_overwatch_min_s: f.get("ground.bound.overwatch_min_s"),
            bound_settle_s: f.get("ground.bound.settle_s"),
            ford_speed_mps: f.get("ground.ford.speed_mps"),
            ford_bank_factor: f.get("ground.ford.bank_factor"),
            ford_current_limit_mps: f.get("ground.ford.current_limit_mps"),
            bridge_caution_factor: f.get("ground.bridge.caution_factor"),
        }
    }
}

fn ci(c: VehicleClass) -> usize {
    c as usize
}

/// NRMM-style trafficability of one cell for one mover.
pub fn trafficability(spec: &MoverSpec, cell: &TerrainCell, p: &GroundParams) -> Trafficability {
    let i = ci(spec.class);
    let top = if cell.on_road { spec.road_speed_mps } else { spec.cross_country_speed_mps };
    // Soil: roads are paved; off road the RCI must exceed the VCI, and speed
    // rises to full as RCI reaches `rci_good`.
    let soil_frac = if cell.on_road {
        1.0
    } else if cell.rci <= p.vci[i] {
        0.0
    } else {
        ((cell.rci - p.vci[i]) / (p.rci_good - p.vci[i]).max(1e-6)).clamp(0.0, 1.0).sqrt()
    };
    let grade = cell.slope_deg.to_radians().tan().abs();
    let slope_frac = if grade >= p.max_grade[i] { 0.0 } else { 1.0 - (grade / p.max_grade[i]).powf(p.slope_exponent) };
    let veg_frac = if cell.on_road { 1.0 } else { (1.0 - p.veg_k[i] * cell.vegetation.clamp(0.0, 1.0)).max(0.0) };
    let obstacle_mps = if cell.on_road { f64::INFINITY } else { p.obstacle_k[i] * cell.obstacle_spacing_m.max(0.0).sqrt() };
    let soil_mps = top * soil_frac;
    let slope_mps = top * slope_frac;
    let vegetation_mps = top * veg_frac;
    let cands = [
        (top, if cell.on_road { SpeedLimit::Road } else { SpeedLimit::CrossCountry }),
        (soil_mps, SpeedLimit::Soil),
        (slope_mps, SpeedLimit::Slope),
        (vegetation_mps, SpeedLimit::Vegetation),
        (obstacle_mps, SpeedLimit::Obstacles),
    ];
    let (speed, mut limit) = cands.iter().fold((f64::INFINITY, SpeedLimit::Road), |a, &(v, l)| if v < a.0 { (v, l) } else { a });
    let go = speed > 0.0;
    if !go {
        limit = SpeedLimit::NoGo;
    }
    Trafficability { go, soil_mps, slope_mps, vegetation_mps, obstacle_mps, speed_mps: speed.max(0.0), limit }
}

/// The order mode's cap as a fraction of the mover's top speed.
pub fn mode_cap(mode: MoveMode, p: &GroundParams) -> f64 {
    p.mode_cap[mode as usize]
}

/// Speed = min(road/cross-country speed, trafficability, formation speed, order mode).
/// `formation_mps` is the slowest member's speed (or `None` when moving alone).
pub fn ground_speed(spec: &MoverSpec, cell: &TerrainCell, mode: MoveMode, formation_mps: Option<f64>, p: &GroundParams) -> SpeedResult {
    let t = trafficability(spec, cell, p);
    let mut speed = t.speed_mps;
    let mut limit = t.limit;
    if let Some(f) = formation_mps {
        let fv = f * p.formation_speed_factor;
        if fv < speed {
            speed = fv;
            limit = SpeedLimit::Formation;
        }
    }
    let mc = spec.road_speed_mps.max(spec.cross_country_speed_mps) * mode_cap(mode, p);
    if mc < speed {
        speed = mc;
        limit = SpeedLimit::OrderMode;
    }
    if !t.go {
        speed = 0.0;
        limit = SpeedLimit::NoGo;
    }
    SpeedResult { speed_mps: speed, limit, trafficability: t }
}

/// Surface class for fuel burn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Road,
    CrossCountry,
    /// Mud, sand, snow.
    Soft,
}

/// A vehicle's fuel curve: idling burn and the cross-country burn at a reference speed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FuelProfile {
    pub idle_lph: f64,
    pub cross_country_lph: f64,
    pub ref_speed_mps: f64,
}

impl FuelProfile {
    /// The Abrams-class main battle tank **[P17]**: ~10 gal/h idle, ~60 gal/h cross-country.
    pub fn mbt(p: &GroundParams) -> Self {
        FuelProfile { idle_lph: p.mbt_idle_lph, cross_country_lph: p.mbt_xc_lph, ref_speed_mps: p.mbt_ref_speed_mps }
    }
}

/// Litres per hour: idle burn plus a moving term proportional to speed and
/// terrain resistance (cross-country is 1 by definition).
pub fn fuel_burn_lph(fuel: &FuelProfile, speed_mps: f64, surface: Surface, engine_on: bool, p: &GroundParams) -> f64 {
    if !engine_on {
        return 0.0;
    }
    let r = p.fuel_resistance[surface as usize] / p.fuel_resistance[Surface::CrossCountry as usize];
    let moving = (fuel.cross_country_lph - fuel.idle_lph).max(0.0) * (speed_mps.max(0.0) / fuel.ref_speed_mps.max(1e-6)) * r;
    fuel.idle_lph + moving
}

/// Ground cover for foot movement (Pandolf terrain factor η).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootTerrain {
    Road,
    DirtTrack,
    LightBrush,
    HeavyBrush,
    Swamp,
}

/// What sets a dismounted pace, for the AAR.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FootPace {
    /// The pace the metabolic ceiling allows (Pandolf), m/s.
    pub pandolf_mps: f64,
    /// Tobler's hiking-function cap for this slope, m/s.
    pub tobler_mps: f64,
    pub metabolic_cap_w: f64,
    pub speed_mps: f64,
}

/// Dismounted pace for a soldier carrying `load_kg` on a slope (grade in %,
/// positive uphill), with physical fatigue 0..1. Pandolf et al. (1977):
/// M = 1.5W + 2(W+L)(L/W)² + η(W+L)(1.5V² + 0.35·V·G), solved for V at the
/// sustainable metabolic rate, then capped by Tobler. **[P26]**
pub fn foot_pace(load_kg: f64, grade_pct: f64, terrain: FootTerrain, fatigue: f64, p: &GroundParams) -> FootPace {
    let w = p.foot_body_kg;
    let l = load_kg.max(0.0);
    let eta = p.foot_eta[terrain as usize];
    let cap = p.foot_metabolic_cap_w * (1.0 - p.foot_fatigue_cap_loss * fatigue.clamp(0.0, 1.0));
    let g = grade_pct.max(0.0);
    let a = eta * (w + l) * 1.5;
    let b = eta * (w + l) * 0.35 * g;
    let c = 1.5 * w + 2.0 * (w + l) * (l / w).powi(2) - cap;
    let disc = b * b - 4.0 * a * c;
    let pandolf = if c >= 0.0 || disc < 0.0 { 0.0 } else { (-b + disc.sqrt()) / (2.0 * a) };
    // Tobler: v = vmax·exp(−3.5·|tanθ + 0.05|) with vmax·e^(−0.175) = the
    // declared flat-ground maximum… Tobler's 6 km/h peak sits at −5% grade.
    let tobler = p.foot_tobler_max_kmh / 3.6 * (-3.5 * (grade_pct / 100.0 + 0.05).abs()).exp();
    FootPace { pandolf_mps: pandolf, tobler_mps: tobler, metabolic_cap_w: cap, speed_mps: pandolf.min(tobler) }
}

/// Formation shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Formation {
    Column,
    Line,
    Wedge,
    Echelon,
}

/// Slot offsets (right, forward) in metres for `n` members in a formation
/// with the class's tactical spacing (halved-ish on a march by the declared factor).
pub fn formation_slots(formation: Formation, class: VehicleClass, n: usize, march: bool, p: &GroundParams) -> Vec<(f64, f64)> {
    let s = p.spacing_m[ci(class)] * if march { p.march_spacing_factor } else { 1.0 };
    (0..n)
        .map(|k| {
            let k = k as f64;
            match formation {
                Formation::Column => (0.0, -k * s),
                Formation::Line => ((k - (n as f64 - 1.0) / 2.0) * s, 0.0),
                Formation::Wedge => {
                    if k == 0.0 {
                        (0.0, 0.0)
                    } else {
                        let rank = ((k + 1.0) / 2.0).floor();
                        let side = if (k as usize) % 2 == 1 { 1.0 } else { -1.0 };
                        (side * rank * s * 0.7, -rank * s * 0.7)
                    }
                }
                Formation::Echelon => (k * s * 0.7, -k * s * 0.7),
            }
        })
        .collect()
}

/// Formation speed: the slowest member's speed.
pub fn formation_speed(member_speeds: &[f64]) -> f64 {
    member_speeds.iter().copied().fold(f64::INFINITY, f64::min).max(0.0)
}

/// The two elements of a bounding pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Element {
    A,
    B,
}

impl Element {
    pub fn other(self) -> Element {
        match self {
            Element::A => Element::B,
            Element::B => Element::A,
        }
    }
}

/// The posture an element holds this second (sets its signature and its search τ).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundPosture {
    Moving,
    /// Halted, settling into a firing position.
    Settling,
    /// Halted, scanning a sector.
    Overwatching,
}

/// Bounding overwatch: one element bounds while the other watches, then they
/// swap. The mover may only start once the watcher has watched at least
/// `overwatch_min_s` (it must have settled and searched its sector).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingOverwatch {
    pub mover: Element,
    /// Metres left in the current bound.
    pub remaining_m: f64,
    /// Seconds the watcher has spent in overwatch (after settling).
    pub watched_s: f64,
    /// Seconds the last mover has spent settling since it halted.
    pub settle_left_s: f64,
    /// Waiting for the watcher to be ready before the next bound.
    pub waiting: bool,
    pub bound_length_m: f64,
    pub bounds_completed: u32,
}

/// What happened during one step of bounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundEvent {
    BoundStarted(Element),
    BoundCompleted(Element),
}

impl BoundingOverwatch {
    pub fn new(first_mover: Element, p: &GroundParams) -> Self {
        BoundingOverwatch {
            mover: first_mover,
            remaining_m: p.bound_length_m,
            watched_s: p.bound_overwatch_min_s, // the watcher starts in place
            settle_left_s: 0.0,
            waiting: false,
            bound_length_m: p.bound_length_m,
            bounds_completed: 0,
        }
    }

    /// The posture of `e` now.
    pub fn posture(&self, e: Element) -> BoundPosture {
        if e == self.mover && !self.waiting {
            BoundPosture::Moving
        } else if e != self.mover && self.settle_left_s > 0.0 {
            BoundPosture::Settling
        } else {
            BoundPosture::Overwatching
        }
    }

    /// Advance `dt` seconds with the mover at `speed_mps`. Returns the events in order.
    pub fn step(&mut self, dt: f64, speed_mps: f64, p: &GroundParams) -> Vec<BoundEvent> {
        let mut events = Vec::new();
        let mut t = dt.max(0.0);
        let mut guard = 0;
        while t > 1e-9 && guard < 64 {
            guard += 1;
            // The watcher settles, then watches.
            if self.settle_left_s > 0.0 {
                let d = t.min(self.settle_left_s);
                self.settle_left_s -= d;
                if !self.waiting {
                    // The mover moves while the other settles.
                    self.advance_mover(d, speed_mps, p, &mut events);
                }
                t -= d;
                continue;
            }
            if self.waiting {
                let need = (p.bound_overwatch_min_s - self.watched_s).max(0.0);
                let d = t.min(need);
                self.watched_s += d;
                t -= d;
                if self.watched_s >= p.bound_overwatch_min_s - 1e-9 {
                    self.waiting = false;
                    events.push(BoundEvent::BoundStarted(self.mover));
                }
                continue;
            }
            self.watched_s += t;
            self.advance_mover(t, speed_mps, p, &mut events);
            t = 0.0;
        }
        events
    }

    fn advance_mover(&mut self, d: f64, speed: f64, p: &GroundParams, events: &mut Vec<BoundEvent>) {
        if speed <= 0.0 || self.waiting {
            return;
        }
        self.remaining_m -= speed * d;
        if self.remaining_m <= 0.0 {
            events.push(BoundEvent::BoundCompleted(self.mover));
            self.bounds_completed += 1;
            // Swap: the old mover halts and settles; the old watcher waits
            // until the new watcher has settled and searched.
            self.mover = self.mover.other();
            self.remaining_m = self.bound_length_m;
            self.settle_left_s = p.bound_settle_s;
            self.watched_s = 0.0;
            self.waiting = true;
        }
    }
}

/// A vehicle's fording limits (catalogue record).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FordingSpec {
    pub class: VehicleClass,
    /// Unprepared fording depth, m.
    pub max_depth_m: f64,
}

/// Why a ford succeeds or fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FordResult {
    pub possible: bool,
    pub reason: String,
    /// Time to cross the water gap, s (when possible).
    pub time_s: f64,
}

/// Can this vehicle ford a gap of `width_m` with this depth, bank slope and current?
pub fn ford(spec: &FordingSpec, depth_m: f64, bank_slope_deg: f64, current_mps: f64, width_m: f64, p: &GroundParams) -> FordResult {
    let bank_limit = p.max_grade[ci(spec.class)] * p.ford_bank_factor;
    let bank = bank_slope_deg.to_radians().tan().abs();
    let fail = |r: &str| FordResult { possible: false, reason: r.into(), time_s: f64::INFINITY };
    if depth_m > spec.max_depth_m {
        return fail("too deep");
    }
    if bank > bank_limit {
        return fail("exit bank too steep");
    }
    if current_mps > p.ford_current_limit_mps {
        return fail("current too strong");
    }
    FordResult { possible: true, reason: "fordable".into(), time_s: width_m.max(0.0) / p.ford_speed_mps }
}

/// Bridge check by military load class. A caution crossing (one vehicle at a
/// time, slow, centred) allows up to the declared factor over the posted class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCrossing {
    Normal,
    Caution,
    Prohibited,
}

pub fn bridge_crossing(bridge_mlc: f64, vehicle_mlc: f64, p: &GroundParams) -> BridgeCrossing {
    if vehicle_mlc <= bridge_mlc {
        BridgeCrossing::Normal
    } else if vehicle_mlc <= bridge_mlc * p.bridge_caution_factor {
        BridgeCrossing::Caution
    } else {
        BridgeCrossing::Prohibited
    }
}

/// The ground figures.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("ground.vci.tracked", 30.0, "RCI", 10.0, 80.0, "default", Default, "vehicle cone index, tracked (NRMM-style)"),
        figure!("ground.vci.wheeled", 45.0, "RCI", 15.0, 100.0, "default", Default, "vehicle cone index, wheeled"),
        figure!("ground.vci.foot", 12.0, "RCI", 2.0, 40.0, "default", Default, "minimum soil strength for walking"),
        figure!("ground.rci_good", 120.0, "RCI", 60.0, 300.0, "default", Default, "soil strength above which soil does not limit speed"),
        figure!("ground.max_grade.tracked", 0.6, "fraction", 0.3, 1.0, "default", Default, "steepest grade, tracked"),
        figure!("ground.max_grade.wheeled", 0.45, "fraction", 0.2, 0.8, "default", Default, "steepest grade, wheeled"),
        figure!("ground.max_grade.foot", 1.0, "fraction", 0.5, 2.0, "default", Default, "steepest grade, on foot"),
        figure!("ground.slope_exponent", 1.5, "1", 0.5, 4.0, "default", Default, "shape of the speed loss with grade"),
        figure!("ground.veg_k.tracked", 0.6, "fraction", 0.1, 1.0, "default", Default, "speed loss in dense vegetation, tracked"),
        figure!("ground.veg_k.wheeled", 0.9, "fraction", 0.2, 1.0, "default", Default, "speed loss in dense vegetation, wheeled"),
        figure!("ground.veg_k.foot", 0.5, "fraction", 0.1, 1.0, "default", Default, "speed loss in dense vegetation, on foot"),
        figure!("ground.obstacle_k.tracked", 1.2, "m/s/sqrt(m)", 0.2, 5.0, "default", Default, "obstacle-spacing speed limit coefficient"),
        figure!("ground.obstacle_k.wheeled", 1.0, "m/s/sqrt(m)", 0.2, 5.0, "default", Default, "obstacle-spacing speed limit coefficient"),
        figure!("ground.obstacle_k.foot", 0.8, "m/s/sqrt(m)", 0.1, 5.0, "default", Default, "obstacle-spacing speed limit coefficient"),
        figure!("ground.mode.march", 1.0, "fraction", 0.5, 1.0, "default", Default, "march speed cap"),
        figure!("ground.mode.tactical", 0.5, "fraction", 0.2, 1.0, "default", Default, "tactical movement speed cap"),
        figure!("ground.mode.assault", 0.8, "fraction", 0.3, 1.0, "default", Default, "assault speed cap"),
        figure!("ground.mode.stealth", 0.2, "fraction", 0.05, 0.6, "default", Default, "stealthy movement speed cap"),
        figure!("ground.formation.speed_factor", 0.9, "fraction", 0.5, 1.0, "default", Default, "formation-keeping loss on the slowest member's speed"),
        figure!("ground.formation.spacing.tracked", 100.0, "m", 25.0, 300.0, "default", Default, "tactical spacing, tracked"),
        figure!("ground.formation.spacing.wheeled", 75.0, "m", 20.0, 300.0, "default", Default, "tactical spacing, wheeled"),
        figure!("ground.formation.spacing.foot", 10.0, "m", 3.0, 30.0, "default", Default, "tactical spacing between soldiers"),
        figure!("ground.formation.march_spacing_factor", 0.5, "fraction", 0.1, 1.0, "default", Default, "march spacing as a fraction of tactical"),
        figure!("ground.fuel.resistance.road", 0.6, "1", 0.3, 1.0, "default", Default, "rolling resistance relative to cross-country"),
        figure!("ground.fuel.resistance.cross_country", 1.0, "1", 1.0, 1.0, "default", Default, "reference resistance"),
        figure!("ground.fuel.resistance.soft", 1.6, "1", 1.0, 3.0, "default", Default, "mud, sand, snow"),
        figure!("ground.fuel.mbt_idle_lph", 38.0, "L/h", 20.0, 60.0, "P17", Medium, "Abrams idle ~10 gal/h"),
        figure!("ground.fuel.mbt_xc_lph", 227.0, "L/h", 150.0, 350.0, "P17", Medium, "Abrams cross-country ~60 gal/h"),
        figure!("ground.fuel.mbt_ref_speed_mps", 8.3, "m/s", 4.0, 15.0, "default", Default, "speed at which the cross-country burn applies"),
        figure!("ground.foot.body_mass_kg", 80.0, "kg", 55.0, 110.0, "default", Default, "soldier body mass"),
        figure!("ground.foot.metabolic_cap_w", 425.0, "W", 250.0, 700.0, "default", Default, "sustainable marching metabolic rate"),
        figure!("ground.foot.tobler_max_kmh", 6.0, "km/h", 4.0, 8.0, "default", Default, "Tobler hiking function peak"),
        figure!("ground.foot.eta.road", 1.0, "1", 1.0, 1.0, "default", Default, "Pandolf terrain factor, blacktop"),
        figure!("ground.foot.eta.dirt", 1.2, "1", 1.0, 1.5, "default", Default, "Pandolf terrain factor, dirt road"),
        figure!("ground.foot.eta.light_brush", 1.5, "1", 1.2, 2.0, "default", Default, "Pandolf terrain factor, light brush"),
        figure!("ground.foot.eta.heavy_brush", 1.8, "1", 1.4, 2.5, "default", Default, "Pandolf terrain factor, heavy brush"),
        figure!("ground.foot.eta.swamp", 2.1, "1", 1.5, 3.5, "default", Default, "Pandolf terrain factor, swamp/loose sand"),
        figure!("ground.foot.fatigue_cap_loss", 0.4, "fraction", 0.1, 0.8, "P31", Low, "metabolic ceiling lost at full fatigue"),
        figure!("ground.bound.length_m", 300.0, "m", 50.0, 1000.0, "default", Default, "length of one bound"),
        figure!("ground.bound.overwatch_min_s", 30.0, "s", 5.0, 300.0, "default", Default, "time the watcher scans before the next bound"),
        figure!("ground.bound.settle_s", 15.0, "s", 0.0, 120.0, "default", Default, "time to settle into an overwatch position"),
        figure!("ground.ford.speed_mps", 1.5, "m/s", 0.5, 5.0, "default", Default, "speed through a ford"),
        figure!("ground.ford.bank_factor", 0.8, "fraction", 0.3, 1.0, "default", Default, "exit bank limit as a fraction of max grade (wet bank)"),
        figure!("ground.ford.current_limit_mps", 1.5, "m/s", 0.5, 3.0, "default", Default, "strongest current a vehicle fords"),
        figure!("ground.bridge.caution_factor", 1.1, "1", 1.0, 1.5, "default", Default, "caution crossing allowance over posted MLC"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn mbt() -> MoverSpec {
        MoverSpec { class: VehicleClass::Tracked, road_speed_mps: 18.0, cross_country_speed_mps: 13.0 }
    }
    fn open(rci: f64) -> TerrainCell {
        TerrainCell { slope_deg: 0.0, rci, vegetation: 0.0, obstacle_spacing_m: 1e4, on_road: false }
    }

    #[test]
    fn soil_go_no_go_and_limits() {
        let p = GroundParams::from_figures(&figs());
        let t = trafficability(&mbt(), &open(200.0), &p);
        assert!(t.go);
        assert_relative_eq!(t.speed_mps, 13.0);
        let mud = trafficability(&mbt(), &open(35.0), &p);
        assert_eq!(mud.limit, SpeedLimit::Soil);
        assert!(mud.speed_mps < 6.0);
        let wheeled = MoverSpec { class: VehicleClass::Wheeled, ..mbt() };
        assert!(!trafficability(&wheeled, &open(35.0), &p).go, "wheels bog where tracks pass");
        let steep = TerrainCell { slope_deg: 35.0, ..open(200.0) };
        assert!(!trafficability(&wheeled, &steep, &p).go);
        let r = ground_speed(&mbt(), &TerrainCell { on_road: true, ..open(200.0) }, MoveMode::Tactical, None, &p);
        assert_eq!(r.limit, SpeedLimit::OrderMode);
        let f = ground_speed(&mbt(), &open(200.0), MoveMode::Assault, Some(4.0), &p);
        assert_eq!(f.limit, SpeedLimit::Formation);
    }

    #[test]
    fn abrams_fuel_over_eight_hours() {
        // P17: ~300 gal (1136 L) per 8 h of mixed operations.
        let p = GroundParams::from_figures(&figs());
        let fuel = FuelProfile::mbt(&p);
        let idle = 4.0 * fuel_burn_lph(&fuel, 0.0, Surface::CrossCountry, true, &p);
        let xc = 3.0 * fuel_burn_lph(&fuel, 8.3, Surface::CrossCountry, true, &p);
        let road = 1.0 * fuel_burn_lph(&fuel, 12.0, Surface::Road, true, &p);
        let total = idle + xc + road;
        assert!((900.0..1400.0).contains(&total), "{total} L");
        assert_eq!(fuel_burn_lph(&fuel, 0.0, Surface::Road, false, &p), 0.0);
    }

    #[test]
    fn dismounted_pace_matches_p26() {
        let p = GroundParams::from_figures(&figs());
        let road = foot_pace(25.0, 0.0, FootTerrain::Road, 0.0, &p).speed_mps * 3.6;
        assert!((3.5..5.5).contains(&road), "road {road} km/h");
        let xc = foot_pace(35.0, 5.0, FootTerrain::LightBrush, 0.0, &p).speed_mps * 3.6;
        assert!((1.5..3.5).contains(&xc), "cross-country {xc} km/h");
        let tired = foot_pace(35.0, 5.0, FootTerrain::LightBrush, 1.0, &p).speed_mps * 3.6;
        assert!(tired < xc);
        let heavy = foot_pace(45.0, 0.0, FootTerrain::Road, 0.0, &p).speed_mps;
        assert!(heavy * 3.6 < road);
    }

    #[test]
    fn bounding_alternates() {
        let p = GroundParams::from_figures(&figs());
        let mut b = BoundingOverwatch::new(Element::A, &p);
        assert_eq!(b.posture(Element::A), BoundPosture::Moving);
        assert_eq!(b.posture(Element::B), BoundPosture::Overwatching);
        let mut events = Vec::new();
        for _ in 0..400 {
            events.extend(b.step(1.0, 4.0, &p));
        }
        let done: Vec<_> = events.iter().filter_map(|e| if let BoundEvent::BoundCompleted(x) = e { Some(*x) } else { None }).collect();
        assert!(done.len() >= 3);
        for w in done.windows(2) {
            assert_ne!(w[0], w[1], "movers alternate");
        }
        // After a bound completes, the new mover waits for the watcher.
        let mut b = BoundingOverwatch::new(Element::A, &p);
        b.step(300.0 / 4.0 + 0.5, 4.0, &p);
        assert_eq!(b.mover, Element::B);
        assert_eq!(b.posture(Element::A), BoundPosture::Settling);
        assert_eq!(b.posture(Element::B), BoundPosture::Overwatching);
    }

    #[test]
    fn formations_fords_bridges() {
        let p = GroundParams::from_figures(&figs());
        let slots = formation_slots(Formation::Line, VehicleClass::Tracked, 4, false, &p);
        assert_relative_eq!(slots[1].0 - slots[0].0, 100.0);
        assert_eq!(formation_speed(&[5.0, 3.0, 4.0]), 3.0);
        let spec = FordingSpec { class: VehicleClass::Tracked, max_depth_m: 1.2 };
        assert!(ford(&spec, 1.0, 10.0, 0.5, 20.0, &p).possible);
        assert!(!ford(&spec, 1.5, 10.0, 0.5, 20.0, &p).possible);
        assert_eq!(ford(&spec, 1.0, 40.0, 0.5, 20.0, &p).reason, "exit bank too steep");
        assert_eq!(bridge_crossing(60.0, 70.0, &p), BridgeCrossing::Prohibited);
        assert_eq!(bridge_crossing(70.0, 70.0, &p), BridgeCrossing::Normal);
        assert_eq!(bridge_crossing(65.0, 70.0, &p), BridgeCrossing::Caution);
    }

    proptest! {
        #[test]
        fn speed_is_bounded_and_monotone_in_soil(rci in 0.0f64..300.0, slope in -40.0f64..40.0, veg in 0.0f64..1.0) {
            let p = GroundParams::from_figures(&figs());
            let cell = TerrainCell { slope_deg: slope, rci, vegetation: veg, obstacle_spacing_m: 50.0, on_road: false };
            let t = trafficability(&mbt(), &cell, &p);
            prop_assert!(t.speed_mps >= 0.0 && t.speed_mps <= 13.0 + 1e-9);
            let better = trafficability(&mbt(), &TerrainCell { rci: rci + 10.0, ..cell }, &p);
            prop_assert!(better.speed_mps >= t.speed_mps - 1e-12);
        }
    }
}
