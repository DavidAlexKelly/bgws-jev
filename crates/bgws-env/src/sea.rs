//! The sea (§7.2): bathymetry, a wind-driven sea-state field over time on the
//! Douglas scale, and sound-speed profiles per region and season from a
//! simplified climatology **[default: simplified profiles]**.
//!
//! - **Waves:** significant height from wind by the fetch-limited JONSWAP
//!   growth law `Hs = a_f·U·√(F/g)`, capped at the fully developed
//!   Pierson–Moskowitz value `Hs = a_pm·U²`; the sea relaxes toward that
//!   target with a time constant (solved exactly per step, rule 2).
//! - **Sound speed:** a temperature profile (mixed layer, exponential
//!   thermocline, cold deep water) and salinity through Mackenzie's (1981)
//!   nine-term equation. That gives the surface duct, the deep sound
//!   channel and — where the water is deep enough for the deep sound speed to
//!   exceed the surface value (depth excess) — convergence zones, whose range
//!   comes from integrating the limiting ray (§8.6).

use crate::raster::{GridSpec, Raster};
use crate::synth::fbm;
use crate::weather::WeatherField;
use bgws_core::units::G0;
use bgws_core::{figure, Figure, Figures, SimTime};
use serde::{Deserialize, Serialize};

/// Water depth (positive down, metres; ≤ 0 is land).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bathymetry {
    pub depth: Raster<f32>,
}

impl Bathymetry {
    pub fn depth_at(&self, e: f64, n: f64) -> f64 {
        self.depth.bilinear(e, n)
    }

    /// A synthetic margin: coast along the west edge, a shelf, a slope and an
    /// abyssal plain, roughened by noise.
    pub fn synthetic(spec: GridSpec, seed: u64, shelf_depth: f64, shelf_width_m: f64, abyss_depth: f64, slope_width_m: f64) -> Self {
        let depth = Raster::from_fn(spec, |i, j| {
            let (x, y) = spec.pos(i, j);
            let d = x - spec.origin_e;
            let base = if d < shelf_width_m {
                shelf_depth * d / shelf_width_m
            } else if d < shelf_width_m + slope_width_m {
                shelf_depth + (abyss_depth - shelf_depth) * (d - shelf_width_m) / slope_width_m
            } else {
                abyss_depth
            };
            let rough = 1.0 + 0.1 * fbm(seed, x / 20_000.0, y / 20_000.0, 4, 0.5);
            (base * rough - 5.0) as f32
        });
        Self { depth }
    }
}

/// Douglas sea-state code (0–9) from significant wave height.
pub fn douglas(hs_m: f64) -> u8 {
    const UPPER: [f64; 9] = [0.0, 0.1, 0.5, 1.25, 2.5, 4.0, 6.0, 9.0, 14.0];
    UPPER.iter().position(|&u| hs_m <= u).unwrap_or(9) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaveParams {
    pub a_pm: f64,
    pub a_fetch: f64,
    pub tau_h: f64,
}

impl WaveParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self { a_pm: f.get("env.sea.pm_coefficient"), a_fetch: f.get("env.sea.fetch_coefficient"), tau_h: f.get("env.sea.response_tau_h") }
    }
}

/// Equilibrium significant wave height for wind `u10` (m/s) over fetch `f` (m).
pub fn equilibrium_hs(u10: f64, fetch_m: f64, p: &WaveParams) -> f64 {
    let u = u10.max(0.0);
    (p.a_fetch * u * (fetch_m.max(0.0) / G0).sqrt()).min(p.a_pm * u * u)
}

/// Advance `hs` by `dt_h` hours toward the equilibrium for the wind.
pub fn step_hs(hs: f64, u10: f64, fetch_m: f64, dt_h: f64, p: &WaveParams) -> f64 {
    let target = equilibrium_hs(u10, fetch_m, p);
    target + (hs - target) * (-dt_h.max(0.0) / p.tau_h).exp()
}

/// Significant wave height over time on the weather grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeaStateField {
    pub spec: GridSpec,
    pub times: Vec<SimTime>,
    /// `hs[t][k]`, metres.
    pub hs: Vec<Vec<f32>>,
}

impl SeaStateField {
    /// Integrate wave growth under a weather field's surface wind.
    pub fn from_weather(w: &WeatherField, fetch_m: f64, initial_hs: f64, p: &WaveParams) -> Self {
        let mut hs = Vec::with_capacity(w.times.len());
        let mut cur: Vec<f64> = vec![initial_hs; w.spec.len()];
        for (ti, frame) in w.frames.iter().enumerate() {
            let dt_h = if ti == 0 { 0.0 } else { (w.times[ti].secs() - w.times[ti - 1].secs()) / 3600.0 };
            for (k, c) in frame.iter().enumerate() {
                let u = c.wind_surface.0.hypot(c.wind_surface.1);
                cur[k] = step_hs(cur[k], u, fetch_m, dt_h, p);
            }
            hs.push(cur.iter().map(|&v| v as f32).collect());
        }
        Self { spec: w.spec, times: w.times.clone(), hs }
    }

    /// Hs at a point and time (bilinear in space, linear in time).
    pub fn hs_at(&self, e: f64, n: f64, t: SimTime) -> f64 {
        if self.times.is_empty() {
            return 0.0;
        }
        let (a, b, f) = crate::weather::bracket(&self.times, t);
        let ra = Raster { spec: self.spec, data: self.hs[a].clone() };
        let rb = Raster { spec: self.spec, data: self.hs[b].clone() };
        ra.bilinear(e, n) * (1.0 - f) + rb.bilinear(e, n) * f
    }

    pub fn douglas_at(&self, e: f64, n: f64, t: SimTime) -> u8 {
        douglas(self.hs_at(e, n, t))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OceanRegion {
    Tropical,
    MidLatitude,
    Polar,
    /// Warm, salty, warm deep water (Mediterranean-like).
    Enclosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Winter,
    Spring,
    Summer,
    Autumn,
}

impl Season {
    /// Northern-hemisphere season of a month (1–12).
    pub fn from_month(m: u32, southern: bool) -> Season {
        let s = match m {
            12 | 1 | 2 => Season::Winter,
            3..=5 => Season::Spring,
            6..=8 => Season::Summer,
            _ => Season::Autumn,
        };
        if southern {
            match s {
                Season::Winter => Season::Summer,
                Season::Summer => Season::Winter,
                Season::Spring => Season::Autumn,
                Season::Autumn => Season::Spring,
            }
        } else {
            s
        }
    }
}

/// Mackenzie (1981) sound speed, m/s: temperature °C, salinity ppt, depth m.
pub fn mackenzie(t: f64, s: f64, d: f64) -> f64 {
    1448.96 + 4.591 * t - 5.304e-2 * t * t + 2.374e-4 * t * t * t + 1.340 * (s - 35.0) + 1.630e-2 * d + 1.675e-7 * d * d
        - 1.025e-2 * t * (s - 35.0)
        - 7.139e-13 * t * d * d * d
}

/// The climatological water column for a region and season.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterColumn {
    pub surface_temp_c: f64,
    pub mixed_layer_m: f64,
    pub thermocline_scale_m: f64,
    pub deep_temp_c: f64,
    pub salinity_ppt: f64,
}

impl WaterColumn {
    /// Simplified climatology **[default]**.
    pub fn climatology(region: OceanRegion, season: Season) -> Self {
        let (ts, ml) = match (region, season) {
            (OceanRegion::Tropical, Season::Summer | Season::Autumn) => (29.0, 40.0),
            (OceanRegion::Tropical, _) => (27.0, 70.0),
            (OceanRegion::MidLatitude, Season::Winter) => (12.0, 150.0),
            (OceanRegion::MidLatitude, Season::Spring) => (14.0, 60.0),
            (OceanRegion::MidLatitude, Season::Summer) => (20.0, 20.0),
            (OceanRegion::MidLatitude, Season::Autumn) => (17.0, 50.0),
            (OceanRegion::Polar, Season::Summer) => (3.0, 30.0),
            (OceanRegion::Polar, _) => (-1.0, 80.0),
            (OceanRegion::Enclosed, Season::Winter) => (15.0, 200.0),
            (OceanRegion::Enclosed, Season::Summer) => (25.0, 20.0),
            (OceanRegion::Enclosed, _) => (19.0, 50.0),
        };
        let (scale, deep, sal) = match region {
            OceanRegion::Tropical => (350.0, 2.0, 35.0),
            OceanRegion::MidLatitude => (500.0, 2.5, 35.0),
            OceanRegion::Polar => (800.0, 0.0, 34.5),
            OceanRegion::Enclosed => (150.0, 13.5, 38.5),
        };
        Self { surface_temp_c: ts, mixed_layer_m: ml, thermocline_scale_m: scale, deep_temp_c: deep, salinity_ppt: sal }
    }

    pub fn temp_at(&self, z: f64) -> f64 {
        if z <= self.mixed_layer_m {
            self.surface_temp_c
        } else {
            self.deep_temp_c + (self.surface_temp_c - self.deep_temp_c) * (-(z - self.mixed_layer_m) / self.thermocline_scale_m).exp()
        }
    }
}

/// A sound-speed profile: (depth m, speed m/s), depth increasing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundSpeedProfile {
    pub points: Vec<(f64, f64)>,
}

/// What a profile supports, for the sonar model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub surface_c: f64,
    /// Depth of the near-surface speed maximum (the sonic layer depth), m.
    pub layer_depth_m: f64,
    /// Depth of the speed minimum (deep sound channel axis), m.
    pub channel_axis_m: f64,
    /// Deep speed minus the near-surface maximum, m/s (> 0 means depth excess).
    pub depth_excess_m_s: f64,
    /// First convergence-zone range, if the profile supports one.
    pub cz_range_m: Option<f64>,
}

impl SoundSpeedProfile {
    /// Sample a water column every `step_m` down to `water_depth`.
    pub fn from_column(col: &WaterColumn, water_depth: f64, step_m: f64) -> Self {
        let n = (water_depth / step_m).ceil().max(1.0) as usize;
        let points = (0..=n)
            .map(|k| {
                let z = (k as f64 * step_m).min(water_depth);
                (z, mackenzie(col.temp_at(z), col.salinity_ppt, z))
            })
            .collect();
        Self { points }
    }

    /// Climatological profile for a region, season and water depth.
    pub fn climatology(region: OceanRegion, season: Season, water_depth: f64) -> Self {
        Self::from_column(&WaterColumn::climatology(region, season), water_depth, 5.0)
    }

    pub fn max_depth(&self) -> f64 {
        self.points.last().map(|p| p.0).unwrap_or(0.0)
    }

    /// Speed at depth (linear, clamped).
    pub fn c_at(&self, z: f64) -> f64 {
        let p = &self.points;
        if p.is_empty() {
            return 1500.0;
        }
        if z <= p[0].0 {
            return p[0].1;
        }
        let k = p.partition_point(|q| q.0 < z);
        if k >= p.len() {
            return p[p.len() - 1].1;
        }
        let (a, b) = (p[k - 1], p[k]);
        a.1 + (b.1 - a.1) * (z - a.0) / (b.0 - a.0).max(1e-9)
    }

    /// Sonic layer depth: the bottom of the near-surface layer in which the
    /// speed keeps rising with depth (0 if it falls from the surface).
    pub fn layer_depth(&self) -> f64 {
        let p = &self.points;
        let mut k = 0;
        while k + 1 < p.len() && p[k + 1].1 >= p[k].1 {
            k += 1;
        }
        p.get(k).map(|q| q.0).unwrap_or(0.0)
    }

    /// Depth of the minimum speed below the sonic layer (the deep sound
    /// channel axis); equal to the layer depth when the speed never falls.
    pub fn channel_axis(&self) -> f64 {
        let layer = self.layer_depth();
        self.points.iter().filter(|p| p.0 >= layer).fold((layer, f64::MAX), |acc, p| if p.1 < acc.1 { (p.0, p.1) } else { acc }).0
    }

    /// First convergence-zone range for a near-surface source: the range at
    /// which the ray that just leaves the surface layer (vertex speed =
    /// layer maximum + `vertex_margin`) refracts back to the surface.
    /// `None` without depth excess.
    pub fn cz_range(&self, vertex_margin_m_s: f64) -> Option<f64> {
        let layer = self.layer_depth();
        let cv = self.c_at(layer) + vertex_margin_m_s;
        // Turning depth: first depth below the axis where c reaches cv.
        let axis = self.channel_axis();
        if axis <= layer {
            return None;
        }
        let zt = self.points.iter().find(|p| p.0 > axis && p.1 >= cv).map(|p| p.0)?;
        // R = 2 ∫ c / sqrt(cv² − c²) dz, midpoint rule (integrable end singularities).
        let dz = 1.0;
        let n = (zt / dz) as usize;
        let mut r = 0.0;
        for k in 0..n {
            let z = (k as f64 + 0.5) * dz;
            let c = self.c_at(z);
            let d = cv * cv - c * c;
            if d > 0.0 {
                r += c / d.sqrt() * dz;
            }
        }
        Some(2.0 * r)
    }

    pub fn summary(&self, vertex_margin_m_s: f64) -> ProfileSummary {
        let layer = self.layer_depth();
        let deep = self.points.last().map(|p| p.1).unwrap_or(0.0);
        ProfileSummary {
            surface_c: self.c_at(0.0),
            layer_depth_m: layer,
            channel_axis_m: self.channel_axis(),
            depth_excess_m_s: deep - self.c_at(layer),
            cz_range_m: self.cz_range(vertex_margin_m_s),
        }
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("env.sea.pm_coefficient", 0.0246, "s2/m", 0.015, 0.035, "default", Low,
            "fully developed Hs = a·U10² (Pierson–Moskowitz, ≈0.21 U19.5²/g)"),
        figure!("env.sea.fetch_coefficient", 0.0016, "1", 0.0008, 0.003, "default", Low, "fetch-limited Hs = a·U·sqrt(F/g) (JONSWAP)"),
        figure!("env.sea.response_tau_h", 4.0, "h", 0.5, 24.0, "default", Default, "time constant of sea response to wind changes"),
        figure!("env.sea.cz_vertex_margin_m_s", 1.0, "m/s", 0.1, 10.0, "default", Default,
            "vertex speed above the layer maximum of the limiting CZ ray"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn mackenzie_reference_value() {
        // Mackenzie's own check value: T=25 °C, S=35, D=1000 m → 1550.744 m/s.
        assert_relative_eq!(mackenzie(25.0, 35.0, 1000.0), 1550.744, epsilon = 0.01);
    }

    #[test]
    fn douglas_scale_and_wave_growth() {
        assert_eq!(douglas(0.0), 0);
        assert_eq!(douglas(0.3), 2);
        assert_eq!(douglas(1.0), 3);
        assert_eq!(douglas(3.0), 5);
        assert_eq!(douglas(20.0), 9);
        let p = WaveParams::from_figures(&Figures::from_list(figures()).unwrap());
        // 15 m/s fully developed ≈ 5.5 m; short fetch much less.
        assert_relative_eq!(equilibrium_hs(15.0, 1e9, &p), 5.535, epsilon = 0.01);
        assert!(equilibrium_hs(15.0, 10_000.0, &p) < 1.0);
        let one = step_hs(0.5, 15.0, 1e9, 6.0, &p);
        let mut h = 0.5;
        for _ in 0..6 {
            h = step_hs(h, 15.0, 1e9, 1.0, &p);
        }
        assert_relative_eq!(one, h, epsilon = 1e-12);
    }

    #[test]
    fn deep_mid_latitude_has_a_cz_shallow_does_not() {
        let deep = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Summer, 5000.0);
        let s = deep.summary(1.0);
        assert!(s.channel_axis_m > 600.0 && s.channel_axis_m < 1500.0, "{s:?}");
        let r = s.cz_range_m.expect("deep water supports a CZ");
        assert!((30_000.0..=65_000.0).contains(&r), "CZ at {r} m");
        let shallow = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Summer, 1500.0);
        assert!(shallow.cz_range(1.0).is_none());
        let shelf = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Winter, 200.0);
        assert!(shelf.cz_range(1.0).is_none());
        // Winter mixed layer gives a surface duct: speed rises through the layer.
        let w = SoundSpeedProfile::climatology(OceanRegion::MidLatitude, Season::Winter, 4000.0);
        assert!(w.layer_depth() >= 140.0, "{}", w.layer_depth());
    }
}
