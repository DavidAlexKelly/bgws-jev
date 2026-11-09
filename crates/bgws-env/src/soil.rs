//! Soil, wetness and trafficability (§7.1, §8.1).
//!
//! **Wetness** (0 dry … 1 saturated) rises with rain in proportion to the
//! unfilled capacity and dries exponentially with a soil- and
//! temperature-dependent time constant. The linear ODE
//! `dW/dt = a(1−W) − W/τ` is solved exactly over a step, so the result does
//! not depend on the step size (rule 2).
//!
//! **Trafficability** follows the shape of the NATO Reference Mobility
//! Model in simplified form: soil strength is a rating cone index (RCI) that
//! falls exponentially with wetness; a vehicle class needs a vehicle cone
//! index (VCI); below it the vehicle is immobilised, above it speed recovers
//! as `1 − e^(−(RCI−VCI)/s)`. Slope, vegetation override resistance and
//! obstacle spacing multiply in. Because wheels need a much higher VCI than
//! tracks, mud season stops wheels long before tracks (§7.1).
//! All coefficients are **[default]** figures shaped on NRMM behaviour.

use crate::landcover::LandCover;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum SoilType {
    Sand = 0,
    #[default]
    Loam = 1,
    Clay = 2,
    Silt = 3,
    Peat = 4,
    Rock = 5,
}

impl SoilType {
    pub const ALL: [SoilType; 6] = [SoilType::Sand, SoilType::Loam, SoilType::Clay, SoilType::Silt, SoilType::Peat, SoilType::Rock];
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub fn key(self) -> &'static str {
        match self {
            SoilType::Sand => "sand",
            SoilType::Loam => "loam",
            SoilType::Clay => "clay",
            SoilType::Silt => "silt",
            SoilType::Peat => "peat",
            SoilType::Rock => "rock",
        }
    }
}

/// Mobility classes for trafficability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleClass {
    Tracked,
    Wheeled,
    Foot,
}

impl VehicleClass {
    pub const ALL: [VehicleClass; 3] = [VehicleClass::Tracked, VehicleClass::Wheeled, VehicleClass::Foot];
    pub fn key(self) -> &'static str {
        match self {
            VehicleClass::Tracked => "tracked",
            VehicleClass::Wheeled => "wheeled",
            VehicleClass::Foot => "foot",
        }
    }
}

/// Soil parameters for one type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoilProps {
    pub rci_dry: f64,
    pub wet_decay: f64,
    /// Wetness gained per mm of rain on dry soil.
    pub infiltration_per_mm: f64,
    /// Drying time constant at the reference temperature, hours.
    pub dry_tau_h: f64,
}

/// Vehicle-class parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VehicleProps {
    pub vci: f64,
    pub max_slope: f64,
    pub width_m: f64,
    pub max_speed_kmh: f64,
    /// Vegetation/urban factor per land-cover class.
    pub veg: [f64; 10],
}

/// All soil and trafficability parameters, from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrafficParams {
    pub soil: [SoilProps; 6],
    pub vehicle: [VehicleProps; 3],
    pub soil_margin_scale: f64,
    pub dry_ref_temp_c: f64,
    pub dry_halving_c: f64,
    pub obstacle_clear_widths: f64,
}

impl TrafficParams {
    pub fn from_figures(f: &Figures) -> Self {
        let soil = SoilType::ALL.map(|s| {
            let k = |n: &str| f.get(&format!("env.soil.{}.{n}", s.key()));
            SoilProps { rci_dry: k("rci_dry"), wet_decay: k("wet_decay"), infiltration_per_mm: k("infiltration_per_mm"), dry_tau_h: k("dry_tau_h") }
        });
        let vehicle = VehicleClass::ALL.map(|v| {
            let k = |n: &str| f.get(&format!("env.traffic.{}.{n}", v.key()));
            let veg = LandCover::ALL.map(|c| f.get(&format!("env.traffic.{}.veg.{}", v.key(), c.key())));
            VehicleProps { vci: k("vci"), max_slope: k("max_slope"), width_m: k("width_m"), max_speed_kmh: k("max_speed_kmh"), veg }
        });
        Self {
            soil,
            vehicle,
            soil_margin_scale: f.get("env.traffic.soil_margin_scale"),
            dry_ref_temp_c: f.get("env.soil.dry_ref_temp_c"),
            dry_halving_c: f.get("env.soil.dry_halving_c"),
            obstacle_clear_widths: f.get("env.traffic.obstacle_clear_widths"),
        }
    }
    pub fn soil(&self, s: SoilType) -> &SoilProps {
        &self.soil[s as usize]
    }
    pub fn vehicle(&self, v: VehicleClass) -> &VehicleProps {
        &self.vehicle[match v {
            VehicleClass::Tracked => 0,
            VehicleClass::Wheeled => 1,
            VehicleClass::Foot => 2,
        }]
    }
}

/// Advance wetness by `dt_h` hours of rain at `rain_mm_h` and air temperature `temp_c`.
pub fn update_wetness(w: f64, soil: SoilType, rain_mm_h: f64, temp_c: f64, dt_h: f64, p: &TrafficParams) -> f64 {
    let s = p.soil(soil);
    let a = (rain_mm_h.max(0.0) * s.infiltration_per_mm).max(0.0);
    // Warmer air dries faster: τ halves every `dry_halving_c` degrees.
    let tau = s.dry_tau_h * 0.5f64.powf((temp_c - p.dry_ref_temp_c) / p.dry_halving_c);
    let rate = a + 1.0 / tau.max(1e-6);
    let w_inf = a / rate;
    (w_inf + (w.clamp(0.0, 1.0) - w_inf) * (-rate * dt_h.max(0.0)).exp()).clamp(0.0, 1.0)
}

/// Inputs describing one cell for trafficability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellGoing {
    pub soil: SoilType,
    pub wetness: f64,
    /// Slope as rise over run (tan of the angle).
    pub slope: f64,
    pub land_cover: LandCover,
    /// Mean clear gap between obstacles (trees, rubble, stumps), metres; `None` = no obstacles.
    pub obstacle_spacing_m: Option<f64>,
}

/// Trafficability with its reasons.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trafficability {
    pub rci: f64,
    pub vci: f64,
    pub soil_factor: f64,
    pub slope_factor: f64,
    pub vegetation_factor: f64,
    pub obstacle_factor: f64,
    /// Product of the factors, 0..1.
    pub speed_factor: f64,
    /// Achievable cross-country speed, km/h.
    pub speed_kmh: f64,
    pub immobilised: bool,
}

/// Trafficability of a cell for a vehicle class.
pub fn trafficability(c: &CellGoing, v: VehicleClass, p: &TrafficParams) -> Trafficability {
    let s = p.soil(c.soil);
    let vp = p.vehicle(v);
    let rci = s.rci_dry * (-s.wet_decay * c.wetness.clamp(0.0, 1.0)).exp();
    let margin = rci - vp.vci;
    let soil_factor = if margin <= 0.0 { 0.0 } else { 1.0 - (-margin / p.soil_margin_scale).exp() };
    let r = c.slope.abs() / vp.max_slope;
    let slope_factor = if r >= 1.0 { 0.0 } else { 1.0 - r * r };
    let vegetation_factor = vp.veg[c.land_cover as usize];
    let obstacle_factor = match c.obstacle_spacing_m {
        None => 1.0,
        Some(gap) => ((gap - vp.width_m) / (vp.width_m * p.obstacle_clear_widths)).clamp(0.0, 1.0),
    };
    let speed_factor = soil_factor * slope_factor * vegetation_factor * obstacle_factor;
    Trafficability {
        rci,
        vci: vp.vci,
        soil_factor,
        slope_factor,
        vegetation_factor,
        obstacle_factor,
        speed_factor,
        speed_kmh: speed_factor * vp.max_speed_kmh,
        immobilised: speed_factor <= 0.0,
    }
}

pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    // (type, rci_dry, wet_decay, infiltration per mm, dry tau h)
    let soils = [
        (SoilType::Sand, 90.0, 0.4, 0.010, 24.0),
        (SoilType::Loam, 220.0, 1.6, 0.020, 72.0),
        (SoilType::Clay, 280.0, 1.8, 0.030, 160.0),
        (SoilType::Silt, 200.0, 1.9, 0.025, 96.0),
        (SoilType::Peat, 110.0, 1.2, 0.035, 240.0),
        (SoilType::Rock, 400.0, 0.1, 0.004, 12.0),
    ];
    for (s, rci, b, inf, tau) in soils {
        let k = s.key();
        v.push(figure!(format!("env.soil.{k}.rci_dry"), rci, "RCI", 10.0, 1000.0, "default", Default, "rating cone index when dry (NRMM-shaped)"));
        v.push(figure!(format!("env.soil.{k}.wet_decay"), b, "1", 0.0, 5.0, "default", Default, "exponent of RCI loss with wetness"));
        v.push(figure!(format!("env.soil.{k}.infiltration_per_mm"), inf, "1/mm", 0.0, 0.2, "default", Default, "wetness gained per mm of rain on dry soil"));
        v.push(figure!(format!("env.soil.{k}.dry_tau_h"), tau, "h", 1.0, 2000.0, "default", Default, "drying time constant at the reference temperature"));
    }
    v.push(figure!("env.soil.dry_ref_temp_c", 15.0, "degC", -20.0, 40.0, "default", Default, "reference temperature of the drying constants"));
    v.push(figure!("env.soil.dry_halving_c", 10.0, "degC", 3.0, 30.0, "default", Default, "warming that halves the drying time"));
    v.push(figure!("env.traffic.soil_margin_scale", 40.0, "RCI", 5.0, 200.0, "default", Default, "RCI excess over VCI giving 63% of full soil speed"));
    v.push(figure!("env.traffic.obstacle_clear_widths", 2.0, "1", 0.5, 10.0, "default", Default,
        "extra vehicle widths of gap between obstacles for full speed"));
    // (class, vci, max slope, width, max speed)
    let vehs = [
        (VehicleClass::Tracked, 35.0, 0.60, 3.5, 40.0),
        (VehicleClass::Wheeled, 75.0, 0.40, 2.5, 50.0),
        (VehicleClass::Foot, 15.0, 1.00, 0.6, 3.0),
    ];
    for (c, vci, slope, w, sp) in vehs {
        let k = c.key();
        v.push(figure!(format!("env.traffic.{k}.vci"), vci, "RCI", 5.0, 300.0, "default", Default, "vehicle cone index: soil strength needed to move"));
        v.push(figure!(format!("env.traffic.{k}.max_slope"), slope, "1", 0.1, 2.0, "default", Default, "steepest climbable slope, rise/run"));
        v.push(figure!(format!("env.traffic.{k}.width_m"), w, "m", 0.3, 6.0, "default", Default, "width for obstacle gaps"));
        let (src, note) = if c == VehicleClass::Foot { ("P26", "cross-country walking speed (P26: 2–3 km/h)") } else { ("default", "best cross-country speed") };
        v.push(figure!(format!("env.traffic.{k}.max_speed_kmh"), sp, "km/h", 0.5, 120.0, src, Default, note));
    }
    // Vegetation/urban factors per class: tracked, wheeled, foot.
    let veg = [
        (LandCover::Open, [1.0, 1.0, 1.0]),
        (LandCover::Crops, [0.9, 0.85, 0.85]),
        (LandCover::Scrub, [0.8, 0.6, 0.75]),
        (LandCover::ForestLight, [0.5, 0.25, 0.65]),
        (LandCover::ForestDense, [0.15, 0.0, 0.45]),
        (LandCover::UrbanLight, [0.6, 0.6, 0.7]),
        (LandCover::UrbanDense, [0.35, 0.4, 0.5]),
        (LandCover::Water, [0.0, 0.0, 0.0]),
        (LandCover::Wetland, [0.6, 0.4, 0.5]),
        (LandCover::Bare, [1.0, 1.0, 1.0]),
    ];
    for (lc, f) in veg {
        for (i, c) in VehicleClass::ALL.iter().enumerate() {
            v.push(figure!(format!("env.traffic.{}.veg.{}", c.key(), lc.key()), f[i], "fraction", 0.0, 1.0, "default", Default,
                "speed factor from vegetation override / urban clutter"));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn p() -> TrafficParams {
        let mut all = figures();
        all.extend(crate::landcover::figures());
        TrafficParams::from_figures(&Figures::from_list(all).unwrap())
    }

    #[test]
    fn mud_season_stops_wheels_before_tracks() {
        let p = p();
        let cell = |w| CellGoing { soil: SoilType::Clay, wetness: w, slope: 0.05, land_cover: LandCover::Crops, obstacle_spacing_m: None };
        let dry_w = trafficability(&cell(0.1), VehicleClass::Wheeled, &p);
        assert!(dry_w.speed_factor > 0.7, "{dry_w:?}");
        let mud_w = trafficability(&cell(0.9), VehicleClass::Wheeled, &p);
        let mud_t = trafficability(&cell(0.9), VehicleClass::Tracked, &p);
        assert!(mud_w.immobilised, "{mud_w:?}");
        assert!(!mud_t.immobilised && mud_t.speed_factor > 0.2, "{mud_t:?}");
        // Find the wetness at which each class sticks: wheels first.
        let stick = |v| (0..=100).map(|k| k as f64 / 100.0).find(|&w| trafficability(&cell(w), v, &p).immobilised).unwrap_or(2.0);
        assert!(stick(VehicleClass::Wheeled) + 0.2 < stick(VehicleClass::Tracked));
    }

    #[test]
    fn slope_forest_and_obstacles() {
        let p = p();
        let base = CellGoing { soil: SoilType::Loam, wetness: 0.2, slope: 0.0, land_cover: LandCover::Open, obstacle_spacing_m: None };
        assert!(trafficability(&CellGoing { slope: 0.5, ..base }, VehicleClass::Wheeled, &p).immobilised);
        assert!(!trafficability(&CellGoing { slope: 0.5, ..base }, VehicleClass::Tracked, &p).immobilised);
        assert!(trafficability(&CellGoing { land_cover: LandCover::ForestDense, ..base }, VehicleClass::Wheeled, &p).immobilised);
        assert!(trafficability(&CellGoing { obstacle_spacing_m: Some(3.0), ..base }, VehicleClass::Tracked, &p).immobilised);
        assert!(!trafficability(&CellGoing { obstacle_spacing_m: Some(3.0), ..base }, VehicleClass::Foot, &p).immobilised);
    }

    #[test]
    fn wetness_rises_with_rain_and_dries() {
        let p = p();
        let mut w = 0.1;
        for _ in 0..24 {
            w = update_wetness(w, SoilType::Clay, 5.0, 10.0, 1.0, &p);
        }
        assert!(w > 0.8, "a day of steady rain soaks clay: {w}");
        let dried = update_wetness(w, SoilType::Clay, 0.0, 25.0, 72.0, &p);
        assert!(dried < w * 0.6);
        let sand = update_wetness(w, SoilType::Sand, 0.0, 25.0, 72.0, &p);
        assert!(sand < dried, "sand dries faster");
    }

    proptest! {
        #[test]
        fn wetness_step_size_independent(w0 in 0.0f64..1.0, rain in 0.0f64..20.0, t in -10.0f64..35.0) {
            let p = p();
            let one = update_wetness(w0, SoilType::Loam, rain, t, 6.0, &p);
            let mut w = w0;
            for _ in 0..6 { w = update_wetness(w, SoilType::Loam, rain, t, 1.0, &p); }
            prop_assert!((one - w).abs() < 1e-9);
        }
    }
}
