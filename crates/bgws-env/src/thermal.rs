//! The thermal state (§7.4): a diurnal surface-temperature model per land
//! cover class and per vehicle state, giving background and target
//! temperatures through the day and the real thermal crossover times,
//! instead of a fixed dawn-and-dusk window [R4.1].
//!
//! Each surface relaxes toward an equilibrium temperature
//! `T_eq = T_air + (α·S − L_net) / h`, where `S` is the solar irradiance on
//! the ground (from the NOAA sun position, reduced by cloud), `L_net` the
//! net long-wave loss (large under clear skies, small under cloud) and `h`
//! a convective coefficient rising with wind; the lag `τ` is the surface's
//! thermal inertia (water: a day; grass: under an hour; a steel hull: well
//! under an hour). A vehicle adds internal heat that decays after the
//! engine stops or the gun last fired. The ODE is integrated in fixed
//! five-minute steps after a two-day spin-up, so the cycle is periodic.
//! All coefficients are **[default]**.

use crate::landcover::LandCover;
use crate::light::{sun_position, UtcDateTime};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// A day's mean conditions for the thermal model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiurnalWeather {
    pub mean_air_c: f64,
    /// Half-range of air temperature over the day.
    pub air_amp_c: f64,
    pub cloud_cover: f64,
    pub wind_m_s: f64,
}

/// What a vehicle has been doing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VehicleThermalState {
    /// Parked long enough to match its surroundings' physics.
    ColdSoaked,
    /// Engine running throughout.
    EngineOn,
    /// Moved, engine stopped `minutes_ago` before the start of the day.
    RecentlyMoved { minutes_ago: f64 },
    /// Fired `minutes_ago` before the start of the day (barrel and engine warm).
    RecentlyFired { minutes_ago: f64 },
}

/// What is being modelled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Surface {
    Ground(LandCover),
    Vehicle(VehicleThermalState),
}

/// Thermal coefficients, from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThermalParams {
    /// (absorptivity, τ hours, coupling to the deep store W/m²/K) per land-cover class.
    pub ground: [(f64, f64, f64); 10],
    pub hull_longwave_factor: f64,
    pub hull_absorptivity: f64,
    pub hull_tau_h: f64,
    pub engine_excess_c: f64,
    pub engine_cool_tau_min: f64,
    pub fired_excess_c: f64,
    pub solar_constant_ground: f64,
    pub longwave_clear: f64,
    pub longwave_cloud_reduction: f64,
    pub h0: f64,
    pub h_wind: f64,
    pub cloud_a: f64,
    pub cloud_b: f64,
}

impl ThermalParams {
    pub fn from_figures(f: &Figures) -> Self {
        let ground = LandCover::ALL.map(|c| {
            (
                f.get(&format!("env.thermal.{}.absorptivity", c.key())),
                f.get(&format!("env.thermal.{}.tau_h", c.key())),
                f.get(&format!("env.thermal.{}.ground_coupling_w_m2_k", c.key())),
            )
        });
        Self {
            ground,
            hull_absorptivity: f.get("env.thermal.hull_absorptivity"),
            hull_tau_h: f.get("env.thermal.hull_tau_h"),
            hull_longwave_factor: f.get("env.thermal.hull_longwave_factor"),
            engine_excess_c: f.get("env.thermal.engine_excess_c"),
            engine_cool_tau_min: f.get("env.thermal.engine_cool_tau_min"),
            fired_excess_c: f.get("env.thermal.fired_excess_c"),
            solar_constant_ground: f.get("env.thermal.clear_sky_irradiance_w_m2"),
            longwave_clear: f.get("env.thermal.longwave_clear_w_m2"),
            longwave_cloud_reduction: f.get("env.thermal.longwave_cloud_reduction"),
            h0: f.get("env.thermal.h0_w_m2_k"),
            h_wind: f.get("env.thermal.h_wind_w_m2_k_per_m_s"),
            cloud_a: f.get("env.light.cloud_a"),
            cloud_b: f.get("env.light.cloud_b"),
        }
    }
}

/// Air temperature at a local solar hour (minimum near dawn, maximum mid-afternoon).
pub fn air_temp(w: &DiurnalWeather, local_hour: f64) -> f64 {
    w.mean_air_c + w.air_amp_c * (1.0 - 0.6 * w.cloud_cover) * (std::f64::consts::TAU * (local_hour - 15.0) / 24.0).cos()
}

fn internal_excess(s: &VehicleThermalState, minutes_since_start: f64, p: &ThermalParams) -> f64 {
    match *s {
        VehicleThermalState::ColdSoaked => 0.0,
        VehicleThermalState::EngineOn => p.engine_excess_c,
        VehicleThermalState::RecentlyMoved { minutes_ago } => p.engine_excess_c * (-(minutes_ago + minutes_since_start) / p.engine_cool_tau_min).exp(),
        VehicleThermalState::RecentlyFired { minutes_ago } => {
            (p.engine_excess_c + p.fired_excess_c) * (-(minutes_ago + minutes_since_start) / p.engine_cool_tau_min).exp()
        }
    }
}

/// One day of temperatures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThermalDay {
    /// Sample times, hours after 00:00 UTC.
    pub hours_utc: Vec<f64>,
    pub air_c: Vec<f64>,
    pub background_c: Vec<f64>,
    pub target_c: Vec<f64>,
    /// Hours (UTC) at which target and background temperatures cross.
    pub crossovers_utc_h: Vec<f64>,
    /// Largest |target − background| over the day, °C.
    pub max_contrast_c: f64,
}

impl ThermalDay {
    /// Contrast (target − background) at a UTC hour, interpolated.
    pub fn contrast_at(&self, hour: f64) -> f64 {
        let k = self.hours_utc.partition_point(|h| *h < hour).clamp(1, self.hours_utc.len() - 1);
        let (h0, h1) = (self.hours_utc[k - 1], self.hours_utc[k]);
        let f = ((hour - h0) / (h1 - h0)).clamp(0.0, 1.0);
        let c0 = self.target_c[k - 1] - self.background_c[k - 1];
        let c1 = self.target_c[k] - self.background_c[k];
        c0 + (c1 - c0) * f
    }
}

/// Simulate one UTC day at a place for a target surface against a background.
pub fn thermal_day(date: &UtcDateTime, lat: f64, lon: f64, w: &DiurnalWeather, background: LandCover, target: Surface, p: &ThermalParams) -> ThermalDay {
    let dt_min = 5.0;
    let day0 = UtcDateTime::new(date.year, date.month, date.day, 0, 0, 0.0);
    let cloud_t = (1.0 - p.cloud_a * w.cloud_cover.clamp(0.0, 1.0).powf(p.cloud_b)).clamp(0.0, 1.0);
    let lw = p.longwave_clear * (1.0 - p.longwave_cloud_reduction * w.cloud_cover.clamp(0.0, 1.0));
    let h = p.h0 + p.h_wind * w.wind_m_s.max(0.0);
    // (absorptivity, τ, deep coupling, long-wave factor)
    let props = |s: &Surface| -> (f64, f64, f64, f64) {
        match s {
            Surface::Ground(c) => {
                let g = p.ground[*c as usize];
                (g.0, g.1, g.2, 1.0)
            }
            Surface::Vehicle(_) => (p.hull_absorptivity, p.hull_tau_h, 0.0, p.hull_longwave_factor),
        }
    };
    let bg = Surface::Ground(background);
    let (a_b, tau_b, g_b, l_b) = props(&bg);
    let (a_t, tau_t, g_t, l_t) = props(&target);
    // The deep store (soil below the diurnal layer, masonry) sits at the daily mean.
    let t_deep = w.mean_air_c;
    let spin_steps = (2.0 * 1440.0 / dt_min) as i64;
    let day_steps = (1440.0 / dt_min) as i64;
    let start = day0.add_seconds(-(spin_steps as f64) * dt_min * 60.0);
    let first = start.add_seconds(0.0);
    let mut tb = air_temp(w, first.local_solar_hour(lon));
    let mut tt_base = tb;
    let mut out = ThermalDay { hours_utc: vec![], air_c: vec![], background_c: vec![], target_c: vec![], crossovers_utc_h: vec![], max_contrast_c: 0.0 };
    for step in 0..=(spin_steps + day_steps) {
        let t = start.add_seconds(step as f64 * dt_min * 60.0);
        let air = air_temp(w, t.local_solar_hour(lon));
        let sun = sun_position(&t, lat, lon);
        let s = if sun.elevation_deg > 0.0 { p.solar_constant_ground * sun.elevation_deg.to_radians().sin().powf(1.15) * cloud_t } else { 0.0 };
        let teq = |a: f64, g: f64, lf: f64| (h * air + a * s - lf * lw + g * t_deep) / (h + g);
        // Exact first-order relaxation over the step (rule 2).
        let relax = |cur: f64, eq: f64, tau_h: f64| eq + (cur - eq) * (-(dt_min / 60.0) / tau_h).exp();
        tb = relax(tb, teq(a_b, g_b, l_b), tau_b);
        tt_base = relax(tt_base, teq(a_t, g_t, l_t), tau_t);
        if step >= spin_steps {
            let mins = (step - spin_steps) as f64 * dt_min;
            let tt = match &target {
                Surface::Vehicle(v) => tt_base + internal_excess(v, mins, p),
                Surface::Ground(_) => tt_base,
            };
            out.hours_utc.push(mins / 60.0);
            out.air_c.push(air);
            out.background_c.push(tb);
            out.target_c.push(tt);
        }
    }
    for k in 1..out.hours_utc.len() {
        let c0 = out.target_c[k - 1] - out.background_c[k - 1];
        let c1 = out.target_c[k] - out.background_c[k];
        out.max_contrast_c = out.max_contrast_c.max(c1.abs());
        if (c0 < 0.0) != (c1 < 0.0) {
            let f = c0 / (c0 - c1);
            out.crossovers_utc_h.push(out.hours_utc[k - 1] + f * (out.hours_utc[k] - out.hours_utc[k - 1]));
        }
    }
    out
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("env.thermal.hull_absorptivity", 0.8, "fraction", 0.3, 0.98, "default", Default, "solar absorptivity of painted armour"),
        figure!("env.thermal.hull_longwave_factor", 1.3, "1", 0.8, 2.0, "default", Default,
            "hull long-wave loss relative to ground (thin metal radiating to the sky)"),
        figure!("env.thermal.hull_tau_h", 0.6, "h", 0.1, 3.0, "default", Default, "thermal time constant of a vehicle hull"),
        figure!("env.thermal.engine_excess_c", 15.0, "degC", 2.0, 60.0, "R4.1", Default, "mean hull excess temperature with the engine running"),
        figure!("env.thermal.engine_cool_tau_min", 45.0, "min", 5.0, 240.0, "default", Default, "decay of engine heat after shutdown"),
        figure!("env.thermal.fired_excess_c", 10.0, "degC", 0.0, 60.0, "default", Default, "extra mean excess just after firing"),
        figure!("env.thermal.clear_sky_irradiance_w_m2", 1000.0, "W/m2", 700.0, 1200.0, "default", Low, "clear-sky global irradiance, sun at zenith"),
        figure!("env.thermal.longwave_clear_w_m2", 100.0, "W/m2", 30.0, 200.0, "default", Default, "net long-wave loss under clear skies"),
        figure!("env.thermal.longwave_cloud_reduction", 0.8, "fraction", 0.3, 1.0, "default", Default, "share of long-wave loss removed by full overcast"),
        figure!("env.thermal.h0_w_m2_k", 15.0, "W/m2/K", 3.0, 40.0, "default", Default, "still-air surface heat transfer"),
        figure!("env.thermal.h_wind_w_m2_k_per_m_s", 4.0, "W/m2/K/(m/s)", 0.5, 10.0, "default", Default, "wind increase of heat transfer"),
    ];
    // (class, absorptivity, τ hours, deep coupling W/m²/K)
    let table = [
        (LandCover::Open, 0.75, 0.8, 8.0),
        (LandCover::Crops, 0.75, 1.0, 8.0),
        (LandCover::Scrub, 0.8, 1.0, 6.0),
        (LandCover::ForestLight, 0.85, 1.5, 3.0),
        (LandCover::ForestDense, 0.9, 2.0, 2.0),
        (LandCover::UrbanLight, 0.7, 3.0, 15.0),
        (LandCover::UrbanDense, 0.7, 5.0, 20.0),
        (LandCover::Water, 0.9, 30.0, 0.0),
        (LandCover::Wetland, 0.85, 6.0, 10.0),
        (LandCover::Bare, 0.7, 1.2, 10.0),
    ];
    for (c, a, tau, g) in table {
        v.push(figure!(format!("env.thermal.{}.ground_coupling_w_m2_k", c.key()), g, "W/m2/K", 0.0, 100.0, "default", Default,
            "conduction to the deep store at the daily mean temperature"));
        v.push(figure!(format!("env.thermal.{}.absorptivity", c.key()), a, "fraction", 0.1, 1.0, "default", Default, "solar absorptivity"));
        v.push(figure!(format!("env.thermal.{}.tau_h", c.key()), tau, "h", 0.1, 100.0, "default", Default, "surface thermal time constant"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> ThermalParams {
        ThermalParams::from_figures(&crate::default_figures())
    }

    #[test]
    fn cold_soaked_vehicle_crosses_over_near_dawn_and_dusk() {
        let p = p();
        // June, 50°N 30°E (local solar ≈ UTC+2), clear and light wind.
        let d = UtcDateTime::new(2024, 6, 15, 0, 0, 0.0);
        let w = DiurnalWeather { mean_air_c: 18.0, air_amp_c: 6.0, cloud_cover: 0.1, wind_m_s: 2.0 };
        let day = thermal_day(&d, 50.0, 30.0, &w, LandCover::Open, Surface::Vehicle(VehicleThermalState::ColdSoaked), &p);
        assert_eq!(day.crossovers_utc_h.len(), 2, "{:?}", day.crossovers_utc_h);
        let (morning, evening) = (day.crossovers_utc_h[0], day.crossovers_utc_h[1]);
        // Sunrise ≈ 01:50 UTC, sunset ≈ 18:15 UTC there.
        assert!(morning > 1.5 && morning < 7.0, "morning crossover {morning}");
        assert!(evening > 14.0 && evening < 20.5, "evening crossover {evening}");
        assert!(day.contrast_at(10.0) > 0.0, "hull hotter by day");
        assert!(day.contrast_at(23.0) < 0.0, "hull colder at night");
        // Engine running: hot all day, no crossover.
        let hot = thermal_day(&d, 50.0, 30.0, &w, LandCover::Open, Surface::Vehicle(VehicleThermalState::EngineOn), &p);
        assert!(hot.crossovers_utc_h.is_empty());
        // A vehicle that moved 5 min before midnight is still warm at 00:30.
        let moved = thermal_day(&d, 50.0, 30.0, &w, LandCover::Open, Surface::Vehicle(VehicleThermalState::RecentlyMoved { minutes_ago: 5.0 }), &p);
        assert!(moved.contrast_at(0.5) > day.contrast_at(0.5) + 5.0);
        // Overcast flattens the contrast.
        let ov = thermal_day(&d, 50.0, 30.0, &DiurnalWeather { cloud_cover: 1.0, ..w }, LandCover::Open, Surface::Vehicle(VehicleThermalState::ColdSoaked), &p);
        assert!(ov.max_contrast_c < day.max_contrast_c * 0.6);
        // Water lags: its cycle is much flatter than open ground's.
        let water = thermal_day(&d, 50.0, 30.0, &w, LandCover::Water, Surface::Ground(LandCover::Open), &p);
        let range = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
        assert!(range(&water.background_c) < 0.3 * range(&water.target_c));
    }
}
