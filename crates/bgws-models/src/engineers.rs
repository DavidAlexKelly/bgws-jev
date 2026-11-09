//! Engineers and obstacles (RUST_ENGINE.md §8.9) **[ref R4.5]**.
//!
//! - **Minefields:** encounter probability per crossing from density and
//!   path, P = 1 − exp(−ρ·w·L); belts about 500 m deep **[P13]**.
//! - **Breaching** by plough, roller, line charge or by hand: a lane about
//!   6 m wide takes about 1.5 h through a 500 m belt by line charge **[P13]**,
//!   longer when the belt is observed, with the breach team exposed for the
//!   duration and a residual mine fraction left in the lane.
//! - **Obstacles:** anti-tank ditches, dragon's teeth, wire and craters, with
//!   crossing times by vehicle class and engineer reduction times.
//! - **Gap crossing:** AVLB and ribbon bridges with span, load class and launch time.
//! - **Field fortifications:** digging time by soil, tools and diggers, and
//!   the cover a position gives (feeding the lethality tables of §8.8).

use crate::direct_fire::PersonPosture;
use crate::ground::VehicleClass;
use crate::indirect_fire::TargetPosture;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// P(at least one mine encountered) crossing a field: 1 − exp(−ρ·w·L).
pub fn encounter_probability(density_per_m2: f64, track_width_m: f64, path_length_m: f64) -> f64 {
    let x = density_per_m2.max(0.0) * track_width_m.max(0.0) * path_length_m.max(0.0);
    -(-x).exp_m1()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreachMethod {
    Plough,
    Roller,
    LineCharge,
    Manual,
}

impl BreachMethod {
    const ALL: [BreachMethod; 4] = [BreachMethod::Plough, BreachMethod::Roller, BreachMethod::LineCharge, BreachMethod::Manual];
    fn key(self) -> &'static str {
        match self {
            BreachMethod::Plough => "plough",
            BreachMethod::Roller => "roller",
            BreachMethod::LineCharge => "line_charge",
            BreachMethod::Manual => "manual",
        }
    }
}

/// A minefield belt.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Minefield {
    pub depth_m: f64,
    /// Mines per m².
    pub density_per_m2: f64,
    /// Under enemy observation (and so pre-registered fire).
    pub observed: bool,
}

/// One lane's breach, explained.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BreachPlan {
    pub method: BreachMethod,
    pub lane_width_m: f64,
    pub time_s: f64,
    /// Seconds the breach force is exposed in the open.
    pub exposure_s: f64,
    /// Fraction of mines left in the lane.
    pub residual_fraction: f64,
    /// P(a vehicle following the lane meets a mine).
    pub p_encounter_in_lane: f64,
    /// P(a vehicle crossing off the lane meets a mine), for comparison.
    pub p_encounter_unbreached: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Obstacle {
    AntiTankDitch,
    DragonsTeeth,
    Wire,
    Crater,
}

impl Obstacle {
    const ALL: [Obstacle; 4] = [Obstacle::AntiTankDitch, Obstacle::DragonsTeeth, Obstacle::Wire, Obstacle::Crater];
    fn key(self) -> &'static str {
        match self {
            Obstacle::AntiTankDitch => "ditch",
            Obstacle::DragonsTeeth => "dragons_teeth",
            Obstacle::Wire => "wire",
            Obstacle::Crater => "crater",
        }
    }
    /// Can this class get across unaided at all? (Structural, not a figure.)
    pub fn passable(self, class: VehicleClass) -> bool {
        match (self, class) {
            (_, VehicleClass::Foot) => true,
            (Obstacle::AntiTankDitch | Obstacle::DragonsTeeth, _) => false,
            (Obstacle::Wire | Obstacle::Crater, VehicleClass::Tracked) => true,
            (Obstacle::Wire | Obstacle::Crater, VehicleClass::Wheeled) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeKind {
    /// Armoured vehicle-launched bridge: one span.
    Avlb,
    /// Ribbon (floating) bridge assembled bay by bay.
    Ribbon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GapResult {
    pub possible: bool,
    pub launch_s: f64,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Soil {
    Sand,
    Loam,
    Clay,
    Rocky,
    Frozen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tools {
    Hand,
    Excavator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    ShellScrape,
    /// A two-man fighting position.
    Foxhole,
    /// Ten metres of fire trench.
    Trench,
    /// A vehicle hull-down position.
    VehicleHullDown,
}

impl Position {
    const ALL: [Position; 4] = [Position::ShellScrape, Position::Foxhole, Position::Trench, Position::VehicleHullDown];
    fn key(self) -> &'static str {
        match self {
            Position::ShellScrape => "shell_scrape",
            Position::Foxhole => "foxhole",
            Position::Trench => "trench",
            Position::VehicleHullDown => "vehicle_hull_down",
        }
    }
}

/// The protection a finished position gives, in the terms of §8.8.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FortCover {
    /// The Carleton posture against indirect fire.
    pub indirect: TargetPosture,
    /// The posture against small arms, and the fraction of the body behind cover.
    pub small_arms_posture: PersonPosture,
    pub small_arms_cover: f64,
    /// The vehicle fights hull-down from it.
    pub vehicle_hull_down: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineerParams {
    pub lane_width_m: f64,
    pub setup_s: [f64; 4],
    pub rate_mph: [f64; 4],
    pub residual: [f64; 4],
    pub line_charge_length_m: f64,
    pub line_charge_cycle_s: f64,
    pub observed_factor: f64,
    pub vehicle_track_width_m: f64,
    /// [obstacle][tracked, wheeled, foot] crossing time, s.
    pub crossing_s: [[f64; 3]; 4],
    /// [obstacle][equipped, manual] reduction time, s.
    pub reduction_s: [[f64; 2]; 4],
    pub avlb_span_m: f64,
    pub avlb_mlc: f64,
    pub avlb_launch_s: f64,
    pub ribbon_bay_m: f64,
    pub ribbon_mlc: f64,
    pub ribbon_base_s: f64,
    pub ribbon_per_bay_s: f64,
    pub ribbon_max_bays: f64,
    pub dig_volume_m3: [f64; 4],
    pub dig_rate_m3ph: [f64; 2],
    pub soil_factor: [f64; 5],
    pub cover: [f64; 3],
}

impl EngineerParams {
    pub fn from_figures(f: &Figures) -> Self {
        let m = |s: &str| BreachMethod::ALL.map(|b| f.get(&format!("engineers.breach.{s}.{}", b.key())));
        EngineerParams {
            lane_width_m: f.get("engineers.breach.lane_width_m"),
            setup_s: m("setup_s"),
            rate_mph: m("rate_mph"),
            residual: m("residual"),
            line_charge_length_m: f.get("engineers.breach.line_charge_length_m"),
            line_charge_cycle_s: f.get("engineers.breach.line_charge_cycle_s"),
            observed_factor: f.get("engineers.breach.observed_factor"),
            vehicle_track_width_m: f.get("engineers.mine.vehicle_track_width_m"),
            crossing_s: Obstacle::ALL.map(|o| ["tracked", "wheeled", "foot"].map(|c| f.get(&format!("engineers.obstacle.{}.cross_s.{c}", o.key())))),
            reduction_s: Obstacle::ALL.map(|o| ["equipped", "manual"].map(|c| f.get(&format!("engineers.obstacle.{}.reduce_s.{c}", o.key())))),
            avlb_span_m: f.get("engineers.bridge.avlb_span_m"),
            avlb_mlc: f.get("engineers.bridge.avlb_mlc"),
            avlb_launch_s: f.get("engineers.bridge.avlb_launch_s"),
            ribbon_bay_m: f.get("engineers.bridge.ribbon_bay_m"),
            ribbon_mlc: f.get("engineers.bridge.ribbon_mlc"),
            ribbon_base_s: f.get("engineers.bridge.ribbon_base_s"),
            ribbon_per_bay_s: f.get("engineers.bridge.ribbon_per_bay_s"),
            ribbon_max_bays: f.get("engineers.bridge.ribbon_max_bays"),
            dig_volume_m3: Position::ALL.map(|p| f.get(&format!("engineers.dig.volume_m3.{}", p.key()))),
            dig_rate_m3ph: [f.get("engineers.dig.rate_m3ph.hand"), f.get("engineers.dig.rate_m3ph.excavator")],
            soil_factor: ["sand", "loam", "clay", "rocky", "frozen"].map(|s| f.get(&format!("engineers.dig.soil_factor.{s}"))),
            cover: [f.get("engineers.cover.scrape"), f.get("engineers.cover.foxhole"), f.get("engineers.cover.trench")],
        }
    }
}

/// Plan one lane through a minefield by `method`.
pub fn breach_lane(method: BreachMethod, field: &Minefield, p: &EngineerParams) -> BreachPlan {
    let i = method as usize;
    let work = match method {
        // Fire, proof and move up behind each charge.
        BreachMethod::LineCharge => (field.depth_m / p.line_charge_length_m).ceil().max(1.0) * p.line_charge_cycle_s + field.depth_m / p.rate_mph[i] * 3600.0,
        _ => field.depth_m / p.rate_mph[i] * 3600.0,
    };
    let time = (p.setup_s[i] + work) * if field.observed { p.observed_factor } else { 1.0 };
    let residual = p.residual[i];
    BreachPlan {
        method,
        lane_width_m: p.lane_width_m,
        time_s: time,
        exposure_s: time,
        residual_fraction: residual,
        p_encounter_in_lane: encounter_probability(field.density_per_m2 * residual, p.vehicle_track_width_m, field.depth_m),
        p_encounter_unbreached: encounter_probability(field.density_per_m2, p.vehicle_track_width_m, field.depth_m),
    }
}

/// Time for one vehicle (or a section on foot) to cross, or None if it cannot unaided.
pub fn crossing_time_s(o: Obstacle, class: VehicleClass, p: &EngineerParams) -> Option<f64> {
    o.passable(class).then(|| p.crossing_s[o as usize][class as usize])
}

/// Engineer time to reduce (fill, blow, cut) an obstacle for vehicles.
pub fn reduction_time_s(o: Obstacle, equipped: bool, p: &EngineerParams) -> f64 {
    p.reduction_s[o as usize][if equipped { 0 } else { 1 }]
}

/// Can a bridge of this kind span `gap_m` for a vehicle of `vehicle_mlc`, and how long to launch it.
pub fn gap_crossing(kind: BridgeKind, gap_m: f64, vehicle_mlc: f64, p: &EngineerParams) -> GapResult {
    let fail = |r: &str| GapResult { possible: false, launch_s: f64::INFINITY, reason: r.into() };
    match kind {
        BridgeKind::Avlb => {
            if gap_m > p.avlb_span_m {
                return fail("gap wider than the AVLB span");
            }
            if vehicle_mlc > p.avlb_mlc {
                return fail("vehicle over the bridge's load class");
            }
            GapResult { possible: true, launch_s: p.avlb_launch_s, reason: "AVLB".into() }
        }
        BridgeKind::Ribbon => {
            let bays = (gap_m / p.ribbon_bay_m).ceil().max(1.0);
            if bays > p.ribbon_max_bays {
                return fail("not enough bays");
            }
            if vehicle_mlc > p.ribbon_mlc {
                return fail("vehicle over the bridge's load class");
            }
            GapResult { possible: true, launch_s: p.ribbon_base_s + bays * p.ribbon_per_bay_s, reason: format!("ribbon, {bays} bays") }
        }
    }
}

/// Time to dig a position: volume / (rate × soil factor × diggers).
pub fn dig_time_s(pos: Position, soil: Soil, tools: Tools, diggers: u32, p: &EngineerParams) -> f64 {
    let n = if tools == Tools::Excavator { 1.0 } else { diggers.max(1) as f64 };
    p.dig_volume_m3[pos as usize] / (p.dig_rate_m3ph[tools as usize] * p.soil_factor[soil as usize] * n) * 3600.0
}

/// The cover a finished position gives.
pub fn fort_cover(pos: Position, p: &EngineerParams) -> FortCover {
    match pos {
        Position::ShellScrape => FortCover { indirect: TargetPosture::Prone, small_arms_posture: PersonPosture::Prone, small_arms_cover: p.cover[0], vehicle_hull_down: false },
        Position::Foxhole => FortCover { indirect: TargetPosture::Entrenched, small_arms_posture: PersonPosture::Standing, small_arms_cover: p.cover[1], vehicle_hull_down: false },
        Position::Trench => FortCover { indirect: TargetPosture::Entrenched, small_arms_posture: PersonPosture::Standing, small_arms_cover: p.cover[2], vehicle_hull_down: false },
        Position::VehicleHullDown => FortCover { indirect: TargetPosture::ArmouredVehicle, small_arms_posture: PersonPosture::Standing, small_arms_cover: 0.0, vehicle_hull_down: true },
    }
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("engineers.breach.lane_width_m", 6.0, "m", 4.0, 10.0, "P13", Medium, "breach lane width"),
        figure!("engineers.breach.line_charge_length_m", 100.0, "m", 50.0, 150.0, "R4.5", Medium, "lane cleared by one line charge (MICLIC ~100 × 8 m)"),
        figure!("engineers.breach.line_charge_cycle_s", 720.0, "s", 300.0, 2400.0, "P13", Low, "emplace, fire and proof one charge (1.5 h per 500 m belt)"),
        figure!("engineers.breach.observed_factor", 2.0, "1", 1.0, 5.0, "R4.5", Low, "breach time multiplier under observation and fire"),
        figure!("engineers.mine.vehicle_track_width_m", 3.5, "m", 1.0, 5.0, "default", Default, "swept width of a vehicle's tracks/wheels"),
        figure!("engineers.bridge.avlb_span_m", 18.0, "m", 15.0, 26.0, "R4.5", Medium, "AVLB span 18–20 m"),
        figure!("engineers.bridge.avlb_mlc", 70.0, "MLC", 40.0, 100.0, "default", Default, "AVLB load class"),
        figure!("engineers.bridge.avlb_launch_s", 300.0, "s", 120.0, 900.0, "default", Default, "AVLB launch time"),
        figure!("engineers.bridge.ribbon_bay_m", 6.7, "m", 5.0, 10.0, "default", Default, "ribbon bridge bay length"),
        figure!("engineers.bridge.ribbon_mlc", 70.0, "MLC", 40.0, 100.0, "default", Default, "ribbon bridge load class"),
        figure!("engineers.bridge.ribbon_base_s", 900.0, "s", 300.0, 3600.0, "R4.5", Low, "ribbon bridge set-up (assembles in under 1 h)"),
        figure!("engineers.bridge.ribbon_per_bay_s", 120.0, "s", 30.0, 600.0, "R4.5", Low, "ribbon bridge time per bay"),
        figure!("engineers.bridge.ribbon_max_bays", 30.0, "bays", 5.0, 60.0, "default", Default, "bays held by one bridge company"),
        figure!("engineers.dig.rate_m3ph.hand", 0.5, "m3/h", 0.1, 1.5, "default", Default, "hand digging per soldier, loam"),
        figure!("engineers.dig.rate_m3ph.excavator", 30.0, "m3/h", 10.0, 100.0, "default", Default, "engineer excavator, loam"),
        figure!("engineers.cover.scrape", 0.5, "fraction", 0.2, 0.8, "default", Default, "body behind cover in a shell scrape (prone)"),
        figure!("engineers.cover.foxhole", 0.85, "fraction", 0.5, 0.95, "default", Default, "body behind cover in a foxhole"),
        figure!("engineers.cover.trench", 0.85, "fraction", 0.5, 0.95, "default", Default, "body behind cover in a fire trench"),
    ];
    let breach = [
        ("plough", 300.0, 600.0, 0.1),
        ("roller", 300.0, 900.0, 0.25),
        ("line_charge", 900.0, 2000.0, 0.15),
        ("manual", 600.0, 60.0, 0.02),
    ];
    for (k, setup, rate, resid) in breach {
        v.push(figure!(format!("engineers.breach.setup_s.{k}"), setup, "s", 0.0, 3600.0, "default", Default, "breach set-up time"));
        v.push(figure!(format!("engineers.breach.rate_mph.{k}"), rate, "m/h", 1.0, 3000.0, "default", Default, "breach advance rate (line charge: moving up between charges)"));
        v.push(figure!(format!("engineers.breach.residual.{k}"), resid, "fraction", 0.0, 0.6, "default", Default, "mines left in the lane"));
    }
    let obstacles = [
        ("ditch", [600.0, 600.0, 120.0], [1800.0, 7200.0]),
        ("dragons_teeth", [600.0, 600.0, 30.0], [1200.0, 5400.0]),
        ("wire", [30.0, 300.0, 300.0], [300.0, 900.0]),
        ("crater", [120.0, 600.0, 60.0], [1200.0, 5400.0]),
    ];
    for (k, cross, reduce) in obstacles {
        for (c, t) in ["tracked", "wheeled", "foot"].iter().zip(cross) {
            v.push(figure!(format!("engineers.obstacle.{k}.cross_s.{c}"), t, "s", 5.0, 7200.0, "default", Default, "crossing time where passable"));
        }
        for (c, t) in ["equipped", "manual"].iter().zip(reduce) {
            v.push(figure!(format!("engineers.obstacle.{k}.reduce_s.{c}"), t, "s", 60.0, 36000.0, "default", Default, "engineer reduction time"));
        }
    }
    for (k, vol) in [("shell_scrape", 0.4), ("foxhole", 2.5), ("trench", 8.0), ("vehicle_hull_down", 60.0)] {
        v.push(figure!(format!("engineers.dig.volume_m3.{k}"), vol, "m3", 0.1, 200.0, "default", Default, "spoil to move"));
    }
    for (k, f) in [("sand", 1.2), ("loam", 1.0), ("clay", 0.7), ("rocky", 0.3), ("frozen", 0.2)] {
        v.push(figure!(format!("engineers.dig.soil_factor.{k}"), f, "1", 0.05, 2.0, "default", Default, "digging rate relative to loam"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    #[test]
    fn breach_time_matches_p13() {
        let p = EngineerParams::from_figures(&figs());
        let belt = Minefield { depth_m: 500.0, density_per_m2: 0.01, observed: false };
        let lc = breach_lane(BreachMethod::LineCharge, &belt, &p);
        let hours = lc.time_s / 3600.0;
        assert!((1.2..1.8).contains(&hours), "P13: ~1.5 h per lane, got {hours}");
        assert_relative_eq!(lc.lane_width_m, 6.0);
        let observed = breach_lane(BreachMethod::LineCharge, &Minefield { observed: true, ..belt }, &p);
        assert!(observed.time_s > lc.time_s * 1.5);
        let manual = breach_lane(BreachMethod::Manual, &belt, &p);
        assert!(manual.time_s > lc.time_s && manual.residual_fraction < lc.residual_fraction);
        assert!(lc.p_encounter_in_lane < lc.p_encounter_unbreached);
        assert!(lc.p_encounter_unbreached > 0.9, "500 m of 0.01/m² is near-certain death off the lane");
    }

    #[test]
    fn obstacles_bridges_digging() {
        let p = EngineerParams::from_figures(&figs());
        assert_eq!(crossing_time_s(Obstacle::AntiTankDitch, VehicleClass::Tracked, &p), None);
        assert!(crossing_time_s(Obstacle::Wire, VehicleClass::Tracked, &p).is_some());
        assert!(reduction_time_s(Obstacle::AntiTankDitch, true, &p) < reduction_time_s(Obstacle::AntiTankDitch, false, &p));
        assert!(gap_crossing(BridgeKind::Avlb, 15.0, 65.0, &p).possible);
        assert!(!gap_crossing(BridgeKind::Avlb, 25.0, 65.0, &p).possible);
        let ribbon = gap_crossing(BridgeKind::Ribbon, 100.0, 65.0, &p);
        assert!(ribbon.possible && ribbon.launch_s < 3600.0, "ribbon in under an hour: {ribbon:?}");
        let fox_loam = dig_time_s(Position::Foxhole, Soil::Loam, Tools::Hand, 2, &p);
        let fox_frozen = dig_time_s(Position::Foxhole, Soil::Frozen, Tools::Hand, 2, &p);
        assert!((3600.0..4.0 * 3600.0).contains(&fox_loam) && fox_frozen > fox_loam * 3.0);
        assert!(dig_time_s(Position::VehicleHullDown, Soil::Loam, Tools::Excavator, 1, &p) < 3.0 * 3600.0);
        assert_eq!(fort_cover(Position::Foxhole, &p).indirect, TargetPosture::Entrenched);
    }

    proptest! {
        #[test]
        fn encounter_bounded_and_monotone(rho in 0.0f64..0.05, l in 0.0f64..1000.0) {
            let a = encounter_probability(rho, 3.5, l);
            let b = encounter_probability(rho, 3.5, l + 10.0);
            prop_assert!((0.0..=1.0).contains(&a));
            prop_assert!(b >= a);
        }
    }
}
