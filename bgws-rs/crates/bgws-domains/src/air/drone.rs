//! Drones (§8.1 Air, §8.7): multirotor power and battery endurance by wind,
//! payload, air density and temperature from momentum theory; fixed-wing
//! glide; and the fibre-optic tether, whose spool length caps range and
//! whose cable can snag **[P8]**.
//!
//! Multirotor power in forward flight (level, small tilt):
//!
//! ```text
//! T   = √(W² + D²),  D = ½ρ f V²          (f = flat-plate drag area)
//! v_h = √(T / 2ρA)                          (hover induced velocity)
//! v_i² = (−V² + √(V⁴ + 4 v_h⁴)) / 2         (momentum theory, forward flight)
//! P   = T v_i / FM + D V + P_avionics
//! ```
//!
//! Station-keeping in wind needs airspeed equal to the wind, so wind drains
//! the battery even in a hover.

use crate::atmosphere::{Air, G0};
use crate::vec3::V3;
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// A multirotor's performance record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultirotorPerf {
    pub name: String,
    /// Mass without payload, battery included, kg.
    pub empty_mass: f64,
    pub max_payload: f64,
    /// Nameplate battery energy, Wh.
    pub battery_wh: f64,
    /// Total rotor disk area, m².
    pub disk_area: f64,
    /// Rotor figure of merit including motor/ESC efficiency (0–1).
    pub figure_of_merit: f64,
    /// Flat-plate drag area, m².
    pub drag_area: f64,
    /// Avionics, video and radio power, W.
    pub avionics_w: f64,
    /// Maximum airspeed, m/s.
    pub max_airspeed: f64,
}

/// Model figures for drones.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DroneParams {
    /// Battery energy kept in reserve (not usable for the mission).
    pub battery_reserve: f64,
    /// Usable capacity lost per kelvin below the reference temperature.
    pub cold_loss_per_k: f64,
    /// Reference temperature for battery capacity, K.
    pub cold_reference_k: f64,
    /// Fibre spool length, m.
    pub fibre_spool: f64,
    /// Cable paid out per metre of straight-line range (path and sag).
    pub fibre_slack: f64,
    /// Snag hazard per km flown at low level, 1/km.
    pub fibre_snag_per_km: f64,
}

impl DroneParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            battery_reserve: f.get("air.drone.battery_reserve"),
            cold_loss_per_k: f.get("air.drone.cold_loss_per_k"),
            cold_reference_k: f.get("air.drone.cold_reference_k"),
            fibre_spool: f.get("air.drone.fibre_spool_length"),
            fibre_slack: f.get("air.drone.fibre_slack_factor"),
            fibre_snag_per_km: f.get("air.drone.fibre_snag_per_km"),
        }
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("air.drone.battery_reserve", 0.15, "fraction", 0.0, 0.4, "default", Default, "battery kept back for return/landing"),
        figure!("air.drone.cold_loss_per_k", 0.008, "1/K", 0.0, 0.03, "default", Default, "Li-ion usable capacity loss per K below reference"),
        figure!("air.drone.cold_reference_k", 293.0, "K", 273.0, 303.0, "default", Default, "temperature with full capacity"),
        figure!("air.drone.fibre_spool_length", 10_000.0, "m", 3_000.0, 41_000.0, "P8", Medium, "fibre FPV ≤20 km typical, 41 km max reported"),
        figure!("air.drone.fibre_slack_factor", 1.15, "1", 1.0, 1.6, "default", Default, "cable paid out per metre of range"),
        figure!("air.drone.fibre_snag_per_km", 0.02, "1/km", 0.0, 0.2, "default", Default, "snag hazard flying low"),
    ]
}

/// Where the power goes, W.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultirotorPower {
    pub airspeed: f64,
    pub thrust: f64,
    pub induced: f64,
    pub parasite: f64,
    pub avionics: f64,
    pub total: f64,
}

/// Endurance and range for a flight condition.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultirotorEndurance {
    pub power: MultirotorPower,
    /// Usable energy after reserve and cold loss, Wh.
    pub usable_wh: f64,
    pub endurance_s: f64,
    /// Ground distance covered in the endurance, m.
    pub range_m: f64,
    /// False when the airspeed needed exceeds the drone's maximum.
    pub feasible: bool,
}

impl MultirotorPerf {
    /// Electrical power at an airspeed with a payload.
    pub fn power(&self, air: &Air, payload: f64, airspeed: f64) -> MultirotorPower {
        let w = (self.empty_mass + payload) * G0;
        let d = 0.5 * air.density * self.drag_area * airspeed * airspeed;
        let t = w.hypot(d);
        let vh2 = t / (2.0 * air.density * self.disk_area);
        let v2 = airspeed * airspeed;
        let vi = ((-v2 + (v2 * v2 + 4.0 * vh2 * vh2).sqrt()) / 2.0).sqrt();
        let induced = t * vi / self.figure_of_merit;
        let parasite = d * airspeed;
        MultirotorPower { airspeed, thrust: t, induced, parasite, avionics: self.avionics_w, total: induced + parasite + self.avionics_w }
    }

    /// Usable battery energy after the reserve and cold derating, Wh.
    pub fn usable_wh(&self, p: &DroneParams, air: &Air) -> f64 {
        let cold = ((p.cold_reference_k - air.temperature).max(0.0) * p.cold_loss_per_k).min(0.9);
        self.battery_wh * (1.0 - p.battery_reserve) * (1.0 - cold)
    }

    /// Endurance flying at ground velocity `ground_vel` in `wind` (both ENU m/s).
    /// Hovering on station is `ground_vel = 0`.
    pub fn endurance(&self, p: &DroneParams, air: &Air, payload: f64, ground_vel: V3, wind: V3) -> MultirotorEndurance {
        let airspeed = (ground_vel - wind).horizontal();
        let power = self.power(air, payload.min(self.max_payload), airspeed);
        let usable = self.usable_wh(p, air);
        let endurance_s = usable * 3600.0 / power.total;
        MultirotorEndurance {
            power,
            usable_wh: usable,
            endurance_s,
            range_m: ground_vel.horizontal() * endurance_s,
            feasible: airspeed <= self.max_airspeed && payload <= self.max_payload,
        }
    }
}

/// A battery being drained.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    pub capacity_wh: f64,
    pub remaining_wh: f64,
}

impl Battery {
    pub fn full(capacity_wh: f64) -> Self {
        Self { capacity_wh, remaining_wh: capacity_wh }
    }
    /// Drain at `watts` for `dt` seconds; returns false when empty.
    pub fn drain(&mut self, watts: f64, dt: f64) -> bool {
        self.remaining_wh = (self.remaining_wh - watts * dt / 3600.0).max(0.0);
        self.remaining_wh > 0.0
    }
    pub fn fraction(&self) -> f64 {
        if self.capacity_wh > 0.0 {
            self.remaining_wh / self.capacity_wh
        } else {
            0.0
        }
    }
}

/// An unpowered fixed-wing glide from a height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glide {
    pub airspeed: f64,
    pub lift_to_drag: f64,
    pub sink_rate: f64,
    pub time_s: f64,
    /// Ground distance with the wind component along track, m.
    pub ground_range: f64,
}

/// Glide at best L/D from `height_agl` with `tailwind` m/s along track
/// (negative for a headwind).
pub fn glide(airspeed: f64, lift_to_drag: f64, height_agl: f64, tailwind: f64) -> Glide {
    let sink = airspeed / lift_to_drag.max(0.1);
    let time = height_agl.max(0.0) / sink;
    Glide { airspeed, lift_to_drag, sink_rate: sink, time_s: time, ground_range: ((airspeed + tailwind) * time).max(0.0) }
}

/// A fibre-optic drone's tether.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FibreTether {
    pub spool_length: f64,
    pub paid_out: f64,
    pub snagged: bool,
}

/// The tether's state after a leg.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TetherStatus {
    pub paid_out: f64,
    pub remaining: f64,
    /// Straight-line range still reachable, m.
    pub reach_remaining: f64,
    pub at_limit: bool,
    pub snag_probability: f64,
}

impl FibreTether {
    pub fn new(p: &DroneParams) -> Self {
        Self { spool_length: p.fibre_spool, paid_out: 0.0, snagged: false }
    }

    /// Maximum straight-line range from the launch point, m.
    pub fn max_range(&self, p: &DroneParams) -> f64 {
        self.spool_length / p.fibre_slack
    }

    /// Fly a leg of `distance` metres (outbound, increasing range). Pays out
    /// cable, caps at the spool, and gives the snag probability (a hazard
    /// over the low-level distance; the caller draws it).
    pub fn fly(&mut self, p: &DroneParams, distance: f64, low_level: bool) -> TetherStatus {
        let need = distance.max(0.0) * p.fibre_slack;
        let can = (self.spool_length - self.paid_out).max(0.0);
        let used = need.min(can);
        self.paid_out += used;
        let km = used / p.fibre_slack / 1000.0;
        let snag = if low_level { bgws_core::hazard_probability(p.fibre_snag_per_km, km) } else { 0.0 };
        TetherStatus {
            paid_out: self.paid_out,
            remaining: self.spool_length - self.paid_out,
            reach_remaining: (self.spool_length - self.paid_out) / p.fibre_slack,
            at_limit: need >= can,
            snag_probability: snag,
        }
    }
}
