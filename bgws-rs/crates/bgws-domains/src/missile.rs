//! Munitions in flight (RUST_ENGINE.md §8.1 Missiles and munitions), at the
//! level a theatre wargame needs: each munition is an entity on a kinematic
//! profile from launch point to aim point, with a speed, an altitude profile
//! and a time of flight, so the defender's sensors and layers see it at the
//! right place and time, and the radar horizon and the inner-layer reaction
//! window emerge from geometry [R6.2].
//!
//! Accuracy at the aim point is a circular error probable that grows with the
//! munition's navigation error (from the PNT model, §8.7). This module does
//! not model guidance laws or seekers: arrival error is a declared figure per
//! class, degraded by navigation quality.

use crate::vec3::V3;
use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// The flight classes the engine distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlightClass {
    /// Subsonic cruise missile at low level.
    Cruise,
    /// Sea-skimming anti-ship missile.
    SeaSkimmer,
    /// Ballistic missile: a lofted arc.
    Ballistic,
    /// Long-range one-way attack drone (Shahed-like), slow and low.
    OneWayDrone,
    /// Artillery rocket.
    Rocket,
    /// Glide bomb released from an aircraft.
    GlideBomb,
}

impl FlightClass {
    pub const ALL: [FlightClass; 6] = [
        FlightClass::Cruise,
        FlightClass::SeaSkimmer,
        FlightClass::Ballistic,
        FlightClass::OneWayDrone,
        FlightClass::Rocket,
        FlightClass::GlideBomb,
    ];

    pub fn key(self) -> &'static str {
        match self {
            FlightClass::Cruise => "cruise",
            FlightClass::SeaSkimmer => "seaSkimmer",
            FlightClass::Ballistic => "ballistic",
            FlightClass::OneWayDrone => "owa",
            FlightClass::Rocket => "rocket",
            FlightClass::GlideBomb => "glide",
        }
    }
}

/// The kinematic profile of one class, built from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub class: FlightClass,
    /// Mean ground speed, m/s.
    pub speed: f64,
    /// Cruise height above the surface, m (ignored for lofted classes).
    pub cruise_alt: f64,
    /// Apogee as a fraction of ground range (lofted classes only).
    pub loft: f64,
    /// CEP at the aim point with a good navigation solution, m.
    pub cep: f64,
    /// Extra CEP per metre of navigation error at the terminal phase.
    pub cep_per_nav_m: f64,
}

impl Profile {
    pub fn from_figures(class: FlightClass, f: &Figures) -> Self {
        let k = class.key();
        Profile {
            class,
            speed: f.get(&format!("missile.{k}.speed")),
            cruise_alt: f.get(&format!("missile.{k}.cruise_alt")),
            loft: f.get(&format!("missile.{k}.loft")),
            cep: f.get(&format!("missile.{k}.cep")),
            cep_per_nav_m: f.get("missile.cep_per_nav_m"),
        }
    }

    fn lofted(&self) -> bool {
        self.loft > 0.0
    }
}

/// One munition's flight from launch to its aim point in a local ENU frame
/// (x east, y north, z up, metres).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flight {
    pub profile: Profile,
    pub from: V3,
    pub to: V3,
    /// Launch time, s.
    pub launched: f64,
}

impl Flight {
    pub fn new(profile: Profile, from: V3, to: V3, launched: f64) -> Self {
        Flight { profile, from, to, launched }
    }

    pub fn ground_range(&self) -> f64 {
        (self.to - self.from).horizontal()
    }

    /// Time of flight, s.
    pub fn time_of_flight(&self) -> f64 {
        self.ground_range() / self.profile.speed.max(1.0)
    }

    pub fn arrival(&self) -> f64 {
        self.launched + self.time_of_flight()
    }

    /// Fraction of the flight done at time `t`, clamped to [0, 1].
    pub fn progress(&self, t: f64) -> f64 {
        let tof = self.time_of_flight();
        if tof <= 0.0 {
            return 1.0;
        }
        ((t - self.launched) / tof).clamp(0.0, 1.0)
    }

    /// Height above the launch-to-aim chord at progress `s`.
    fn height(&self, s: f64) -> f64 {
        let p = &self.profile;
        if p.lofted() {
            // A parabola through both ends with its apex at loft × range.
            4.0 * p.loft * self.ground_range() * s * (1.0 - s)
        } else {
            // Climb out and dive in over the first and last 5% of the flight.
            let ramp = (s / 0.05).min((1.0 - s) / 0.05).clamp(0.0, 1.0);
            p.cruise_alt * ramp
        }
    }

    /// Position at time `t`.
    pub fn position(&self, t: f64) -> V3 {
        let s = self.progress(t);
        let base = self.from + (self.to - self.from) * s;
        base + V3::new(0.0, 0.0, self.height(s))
    }

    /// Velocity at time `t` (finite difference over 0.1 s).
    pub fn velocity(&self, t: f64) -> V3 {
        let dt = 0.1;
        (self.position(t + dt) - self.position(t - dt)) * (1.0 / (2.0 * dt))
    }

    /// Remaining time to impact at `t`.
    pub fn time_to_go(&self, t: f64) -> f64 {
        (self.arrival() - t).max(0.0)
    }

    /// The time at which the munition first rises above a sensor's radar
    /// horizon (4/3 Earth), seen from a sensor at `sensor` with antenna height
    /// `h_sensor` above the surface: the start of the defender's window.
    pub fn first_visible(&self, sensor: V3, h_sensor: f64) -> Option<f64> {
        let tof = self.time_of_flight();
        let steps = 400usize;
        (0..=steps).map(|i| self.launched + tof * i as f64 / steps as f64).find(|&t| {
            let p = self.position(t);
            let d = (p - sensor).horizontal();
            d <= radar_horizon(h_sensor, p.z.max(0.0))
        })
    }

    /// The CEP at arrival with a terminal navigation error of `nav_error_m`.
    pub fn cep(&self, nav_error_m: f64) -> f64 {
        self.profile.cep + self.profile.cep_per_nav_m * nav_error_m.max(0.0)
    }

    /// Sample the impact point: a circular normal with this CEP
    /// (σ = CEP / 1.1774).
    pub fn sample_impact(&self, nav_error_m: f64, rng: &mut impl Rng) -> (V3, f64) {
        let sigma = self.cep(nav_error_m) / 1.177_410_022_515_474_6;
        let (u1, u2): (f64, f64) = (rng.gen::<f64>().max(1e-12), rng.gen());
        let r = sigma * (-2.0 * u1.ln()).sqrt();
        let a = std::f64::consts::TAU * u2;
        let miss = V3::new(r * a.cos(), r * a.sin(), 0.0);
        (self.to + miss, r)
    }
}

/// Smooth-Earth radar horizon between two heights, 4/3-Earth model:
/// d ≈ 4.12 (√h₁ + √h₂) km [R6.2].
pub fn radar_horizon(h1: f64, h2: f64) -> f64 {
    bgws_core::geo::mutual_horizon(h1, h2, bgws_core::geo::K_RADIO)
}

/// Probability that a point target within `r` metres of the aim point is hit
/// by a munition with this CEP.
pub fn p_within(cep: f64, r: f64) -> f64 {
    if cep <= 0.0 {
        return 1.0;
    }
    1.0 - 0.5f64.powf((r / cep).powi(2))
}

pub fn figures() -> Vec<Figure> {
    use FlightClass::*;
    let mut v = vec![figure!("missile.cep_per_nav_m", 1.0, "m/m", 0.3, 3.0, "default", Default, "CEP growth per metre of terminal navigation error")];
    let rows: [(FlightClass, f64, f64, f64, f64, &str); 6] = [
        (Cruise, 240.0, 50.0, 0.0, 10.0, "subsonic cruise missile at low level"),
        (SeaSkimmer, 290.0, 5.0, 0.0, 5.0, "sea-skimmer at about 5 m [R6.2]"),
        (Ballistic, 1800.0, 0.0, 0.25, 50.0, "short-range ballistic missile, lofted"),
        (OneWayDrone, 50.0, 300.0, 0.0, 15.0, "Shahed-type one-way drone at ~180 km/h"),
        (Rocket, 700.0, 0.0, 0.2, 60.0, "guided or unguided artillery rocket"),
        (GlideBomb, 200.0, 0.0, 0.08, 15.0, "glide bomb from release altitude"),
    ];
    for (c, speed, alt, loft, cep, note) in rows {
        let k = c.key();
        v.push(figure!(format!("missile.{k}.speed"), speed, "m/s", speed * 0.5, speed * 1.5, "default", Default, note));
        v.push(figure!(format!("missile.{k}.cruise_alt"), alt, "m", 0.0, alt * 3.0 + 1.0, "default", Default, "cruise height"));
        v.push(figure!(format!("missile.{k}.loft"), loft, "fraction", 0.0, (loft * 2.0f64).max(0.01), "default", Default, "apogee as a fraction of range"));
        v.push(figure!(format!("missile.{k}.cep"), cep, "m", cep * 0.3, cep * 4.0, "default", Default, "CEP with a good navigation solution"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn figs() -> Figures {
        Figures::from_list(figures()).unwrap()
    }

    #[test]
    fn sea_skimmer_gives_a_short_inner_window() {
        let f = figs();
        let p = Profile::from_figures(FlightClass::SeaSkimmer, &f);
        let ship = V3::ZERO;
        let fl = Flight::new(p, V3::new(150_000.0, 0.0, 0.0), ship, 0.0);
        let seen = fl.first_visible(ship, 20.0).unwrap();
        let window = fl.arrival() - seen;
        // 20 m mast vs a 5 m skimmer: ~27 km, so 30-120 s of warning [R6.2].
        assert!((60.0..130.0).contains(&window), "{window}");
        assert!((radar_horizon(20.0, 5.0) - 27_640.0).abs() < 1500.0);
    }

    #[test]
    fn ballistic_arc_lofts_and_lands() {
        let f = figs();
        let p = Profile::from_figures(FlightClass::Ballistic, &f);
        let fl = Flight::new(p, V3::ZERO, V3::new(300_000.0, 0.0, 0.0), 10.0);
        let mid = fl.position(fl.launched + fl.time_of_flight() / 2.0);
        assert!((mid.z - 75_000.0).abs() < 1.0);
        assert!(fl.position(fl.arrival() + 5.0).z.abs() < 1e-6);
    }

    #[test]
    fn cep_statistics() {
        let f = figs();
        let p = Profile::from_figures(FlightClass::Cruise, &f);
        let fl = Flight::new(p, V3::ZERO, V3::new(10_000.0, 0.0, 0.0), 0.0);
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let cep = fl.cep(20.0);
        let n = 20_000;
        let inside = (0..n).filter(|_| fl.sample_impact(20.0, &mut rng).1 <= cep).count();
        assert!((inside as f64 / n as f64 - 0.5).abs() < 0.015);
        assert!((p_within(cep, cep) - 0.5).abs() < 1e-12);
    }
}
