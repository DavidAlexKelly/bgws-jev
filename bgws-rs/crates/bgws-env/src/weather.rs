//! Weather as fields over time (§7.3): a coarse grid (10–25 km) of
//! visibility, cloud base and cover, precipitation, wind at three levels,
//! temperature, humidity and fog, sampled bilinearly in space and linearly
//! in time.
//!
//! Sources:
//! - **Preset** — a scenario's fixed conditions (clear, overcast, rain, fog,
//!   snow, storm), uniform over the grid.
//! - **Synthetic** — a seasonal climatology for the latitude and month, a
//!   diurnal temperature and wind cycle, radiation fog that forms in calm,
//!   near-saturated pre-dawn air and burns off after sunrise, and cold
//!   fronts that sweep across the grid with a pre-frontal cloud shield, a
//!   rain band, a wind shift and a temperature drop **[default]**.
//! - **ERA5 import (stub)** — a documented JSON or CSV grid produced
//!   offline from reanalysis (see [`era5_from_json`], [`era5_from_csv`]).

use crate::light::{sun_position, UtcDateTime};
use crate::raster::{GridSpec, Raster};
use crate::synth::fbm;
use bgws_core::rng::mix64;
use bgws_core::{figure, Figure, Figures, SimTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrecipKind {
    #[default]
    None,
    Drizzle,
    Rain,
    Snow,
}

/// The weather in one grid cell.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherCell {
    pub visibility_m: f64,
    pub cloud_base_m: f64,
    /// Fraction of sky covered, 0–1.
    pub cloud_cover: f64,
    pub precip_mm_h: f64,
    pub precip: PrecipKind,
    /// (east, north) wind at 10 m, m/s.
    pub wind_surface: (f64, f64),
    /// Wind near 850 hPa (~1.5 km).
    pub wind_850: (f64, f64),
    /// Wind near 250 hPa (~10 km).
    pub wind_250: (f64, f64),
    pub temperature_c: f64,
    /// Relative humidity, 0–1.
    pub humidity: f64,
    /// Fog density, 0 none … 1 thick.
    pub fog: f64,
}

impl Default for WeatherCell {
    fn default() -> Self {
        WeatherPreset::Clear.cell()
    }
}

impl WeatherCell {
    pub fn wind_speed(&self) -> f64 {
        self.wind_surface.0.hypot(self.wind_surface.1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherPreset {
    Clear,
    Overcast,
    Rain,
    HeavyRain,
    Fog,
    Snow,
    Storm,
}

impl WeatherPreset {
    /// Representative conditions **[default]**.
    pub fn cell(self) -> WeatherCell {
        let base = WeatherCell {
            visibility_m: 30_000.0,
            cloud_base_m: 3_000.0,
            cloud_cover: 0.1,
            precip_mm_h: 0.0,
            precip: PrecipKind::None,
            wind_surface: (3.0, 1.0),
            wind_850: (8.0, 2.0),
            wind_250: (30.0, 0.0),
            temperature_c: 15.0,
            humidity: 0.55,
            fog: 0.0,
        };
        match self {
            WeatherPreset::Clear => base,
            WeatherPreset::Overcast => WeatherCell { visibility_m: 15_000.0, cloud_base_m: 800.0, cloud_cover: 1.0, humidity: 0.75, ..base },
            WeatherPreset::Rain => WeatherCell {
                visibility_m: 6_000.0,
                cloud_base_m: 500.0,
                cloud_cover: 1.0,
                precip_mm_h: 4.0,
                precip: PrecipKind::Rain,
                humidity: 0.92,
                temperature_c: 11.0,
                ..base
            },
            WeatherPreset::HeavyRain => WeatherCell {
                visibility_m: 2_000.0,
                cloud_base_m: 300.0,
                cloud_cover: 1.0,
                precip_mm_h: 25.0,
                precip: PrecipKind::Rain,
                humidity: 0.97,
                temperature_c: 12.0,
                wind_surface: (7.0, 3.0),
                ..base
            },
            WeatherPreset::Fog => WeatherCell {
                visibility_m: 200.0,
                cloud_base_m: 30.0,
                cloud_cover: 1.0,
                humidity: 1.0,
                fog: 0.9,
                temperature_c: 6.0,
                wind_surface: (0.5, 0.3),
                ..base
            },
            WeatherPreset::Snow => WeatherCell {
                visibility_m: 1_500.0,
                cloud_base_m: 400.0,
                cloud_cover: 1.0,
                precip_mm_h: 2.0,
                precip: PrecipKind::Snow,
                humidity: 0.95,
                temperature_c: -3.0,
                ..base
            },
            WeatherPreset::Storm => WeatherCell {
                visibility_m: 3_000.0,
                cloud_base_m: 400.0,
                cloud_cover: 1.0,
                precip_mm_h: 15.0,
                precip: PrecipKind::Rain,
                humidity: 0.95,
                wind_surface: (16.0, 8.0),
                wind_850: (28.0, 10.0),
                temperature_c: 10.0,
                ..base
            },
        }
    }
}

/// Weather over a coarse grid and time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeatherField {
    pub spec: GridSpec,
    /// UTC at simulated time zero, when known.
    pub start_utc: Option<UtcDateTime>,
    pub times: Vec<SimTime>,
    /// `frames[t][k]` for grid node `k`.
    pub frames: Vec<Vec<WeatherCell>>,
}

/// Indices of the frames around `t` and the blend fraction.
pub fn bracket(times: &[SimTime], t: SimTime) -> (usize, usize, f64) {
    if times.len() < 2 || t <= times[0] {
        return (0, 0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, last, 0.0);
    }
    let b = times.partition_point(|x| *x <= t);
    let a = b - 1;
    let f = (t.0 - times[a].0) as f64 / (times[b].0 - times[a].0).max(1) as f64;
    (a, b, f)
}

fn lerp(a: f64, b: f64, f: f64) -> f64 {
    a + (b - a) * f
}

fn blend(a: &WeatherCell, b: &WeatherCell, f: f64) -> WeatherCell {
    let l2 = |x: (f64, f64), y: (f64, f64)| (lerp(x.0, y.0, f), lerp(x.1, y.1, f));
    WeatherCell {
        visibility_m: lerp(a.visibility_m, b.visibility_m, f),
        cloud_base_m: lerp(a.cloud_base_m, b.cloud_base_m, f),
        cloud_cover: lerp(a.cloud_cover, b.cloud_cover, f),
        precip_mm_h: lerp(a.precip_mm_h, b.precip_mm_h, f),
        precip: if f < 0.5 { a.precip } else { b.precip },
        wind_surface: l2(a.wind_surface, b.wind_surface),
        wind_850: l2(a.wind_850, b.wind_850),
        wind_250: l2(a.wind_250, b.wind_250),
        temperature_c: lerp(a.temperature_c, b.temperature_c, f),
        humidity: lerp(a.humidity, b.humidity, f),
        fog: lerp(a.fog, b.fog, f),
    }
}

impl WeatherField {
    /// The same cell everywhere at every time.
    pub fn uniform(spec: GridSpec, cell: WeatherCell) -> Self {
        Self { spec, start_utc: None, times: vec![SimTime::ZERO], frames: vec![vec![cell; spec.len()]] }
    }

    pub fn preset(spec: GridSpec, p: WeatherPreset) -> Self {
        Self::uniform(spec, p.cell())
    }

    fn sample_frame(&self, frame: &[WeatherCell], e: f64, n: f64) -> WeatherCell {
        let s = self.spec;
        let (gx, gy) = s.grid_coords(e, n);
        let (i0, j0) = (gx.floor() as usize, gy.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(s.width - 1), (j0 + 1).min(s.height - 1));
        let (fx, fy) = (gx - i0 as f64, gy - j0 as f64);
        let c = |i, j| &frame[s.idx(i, j)];
        let a = blend(c(i0, j0), c(i1, j0), fx);
        let b = blend(c(i0, j1), c(i1, j1), fx);
        blend(&a, &b, fy)
    }

    /// The weather at a point and time.
    pub fn sample(&self, e: f64, n: f64, t: SimTime) -> WeatherCell {
        let (a, b, f) = bracket(&self.times, t);
        let wa = self.sample_frame(&self.frames[a], e, n);
        if a == b {
            return wa;
        }
        blend(&wa, &self.sample_frame(&self.frames[b], e, n), f)
    }

    /// One variable of one frame as a raster (for overlays and fields).
    pub fn raster(&self, frame: usize, f: impl Fn(&WeatherCell) -> f64) -> Raster<f64> {
        Raster { spec: self.spec, data: self.frames[frame].iter().map(f).collect() }
    }
}

/// Seasonal climatology for the synthetic generator.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Climatology {
    pub mean_temp_c: f64,
    pub diurnal_amp_c: f64,
    /// Mean dew-point depression, °C.
    pub dewpoint_spread_c: f64,
    pub mean_wind_m_s: f64,
    pub jet_m_s: f64,
}

impl Climatology {
    /// From latitude and day of year (northern seasons mirrored south) **[default]**.
    pub fn for_place(lat: f64, day_of_year: u32, f: &Figures) -> Self {
        let a = lat.abs();
        let annual = f.get("env.weather.equator_temp_c") - f.get("env.weather.temp_lapse_per_deg") * (a - 15.0).max(0.0);
        let seasonal = f.get("env.weather.seasonal_amp_per_deg") * a;
        let phase = std::f64::consts::TAU * (day_of_year as f64 - 15.0) / 365.25;
        let sign = if lat >= 0.0 { 1.0 } else { -1.0 };
        Self {
            mean_temp_c: annual - sign * seasonal * phase.cos(),
            diurnal_amp_c: f.get("env.weather.diurnal_amp_c"),
            dewpoint_spread_c: f.get("env.weather.dewpoint_spread_c"),
            mean_wind_m_s: f.get("env.weather.mean_wind_m_s"),
            jet_m_s: f.get("env.weather.jet_m_s"),
        }
    }
}

/// Synthetic weather settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SynthWeatherParams {
    pub seed: u64,
    pub spec: GridSpec,
    pub start: UtcDateTime,
    pub lat: f64,
    pub lon: f64,
    pub duration_h: f64,
    pub step_h: f64,
    pub fronts: usize,
}

/// Relative humidity from temperature and dew point (Magnus).
pub fn relative_humidity(t: f64, td: f64) -> f64 {
    let g = |x: f64| 17.62 * x / (243.12 + x);
    (g(td) - g(t)).exp().clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug)]
struct Front {
    /// Unit normal of motion (east, north).
    dir: (f64, f64),
    speed_m_s: f64,
    /// Signed distance of the front from the grid origin at t = 0.
    s0: f64,
}

/// Generate a synthetic weather field.
pub fn synthetic(p: &SynthWeatherParams, f: &Figures) -> WeatherField {
    let clim = Climatology::for_place(p.lat, p.start.day_of_year(), f);
    let n_steps = (p.duration_h / p.step_h).ceil().max(0.0) as usize + 1;
    let s = p.spec;
    let (e0, n0, e1, n1) = s.bounds();
    let diag = (e1 - e0).hypot(n1 - n0).max(1.0);
    let fronts: Vec<Front> = (0..p.fronts)
        .map(|k| {
            let h = mix64(p.seed ^ mix64(k as u64 + 101));
            let ang = std::f64::consts::FRAC_PI_4 * ((h & 0xff) as f64 / 255.0 - 0.5) + std::f64::consts::FRAC_PI_2; // mostly eastward
            let speed = f.get("env.weather.front_speed_m_s") * (0.8 + 0.4 * ((h >> 8) & 0xff) as f64 / 255.0);
            // Arrival spread over the run.
            let arrive_s = p.duration_h * 3600.0 * (k as f64 + 0.5) / p.fronts as f64;
            Front { dir: (ang.sin(), ang.cos()), speed_m_s: speed, s0: -speed * arrive_s + 0.5 * diag }
        })
        .collect();
    let shield = f.get("env.weather.front_cloud_shield_m");
    let band = f.get("env.weather.front_rain_band_m");
    let peak_rain = f.get("env.weather.front_peak_rain_mm_h");
    let drop = f.get("env.weather.front_temp_drop_c");
    let fog_spread = f.get("env.weather.fog_spread_c");
    let fog_wind = f.get("env.weather.fog_max_wind_m_s");
    let fog_vis = f.get("env.weather.fog_visibility_m");
    let clear_vis = f.get("env.weather.clear_visibility_m");
    let wind_dir = std::f64::consts::TAU * ((mix64(p.seed ^ 77) >> 11) as f64 / (1u64 << 53) as f64);

    let mut times = Vec::with_capacity(n_steps);
    let mut frames = Vec::with_capacity(n_steps);
    for step in 0..n_steps {
        let secs = step as f64 * p.step_h * 3600.0;
        times.push(SimTime::from_secs(secs));
        let utc = p.start.add_seconds(secs);
        let g = utc.local_solar_hour(p.lon);
        let sun = sun_position(&utc, p.lat, p.lon);
        let frame = (0..s.len())
            .map(|k| {
                let (i, j) = s.ij(k);
                let (x, y) = s.pos(i, j);
                let noise = fbm(p.seed, x / 200_000.0 + secs / 400_000.0, y / 200_000.0, 3, 0.5);
                let mut cloud = (0.35 + 0.35 * noise).clamp(0.0, 1.0);
                let mut rain: f64 = 0.0;
                let mut temp_off = 0.0;
                let mut wind_boost = 0.0;
                for fr in &fronts {
                    // Positive ahead of the front.
                    let sd = fr.s0 + fr.speed_m_s * secs - (x * fr.dir.0 + y * fr.dir.1);
                    let ahead = -sd;
                    if ahead > 0.0 && ahead < shield {
                        cloud = cloud.max(1.0 - ahead / shield);
                    }
                    if ahead.abs() < band {
                        let w = 1.0 - ahead.abs() / band;
                        rain = rain.max(peak_rain * w);
                        cloud = 1.0;
                        wind_boost = f64::max(wind_boost, 6.0 * w);
                    }
                    if ahead < 0.0 {
                        temp_off -= drop * (1.0 - (-(-ahead) / (3.0 * band)).exp());
                    }
                }
                let diurnal = clim.diurnal_amp_c * (1.0 - 0.6 * cloud) * (std::f64::consts::TAU * (g - 15.0) / 24.0).cos();
                let temp = clim.mean_temp_c + temp_off + diurnal;
                let dew = clim.mean_temp_c + temp_off - clim.dewpoint_spread_c + if rain > 0.0 { clim.dewpoint_spread_c * 0.8 } else { 0.0 };
                let dew = dew.min(temp);
                let rh = relative_humidity(temp, dew);
                let day_wind = if sun.elevation_deg > 0.0 { 1.3 } else { 0.6 };
                let ws = clim.mean_wind_m_s * day_wind * (1.0 + 0.3 * noise) + wind_boost;
                let wsurf = (ws * wind_dir.sin(), ws * wind_dir.cos());
                let veer = wind_dir + 0.35;
                let w850 = (2.0 * ws * veer.sin(), 2.0 * ws * veer.cos());
                let w250 = (clim.jet_m_s, 0.2 * clim.jet_m_s * noise);
                let fog = if temp - dew < fog_spread && ws < fog_wind && rain == 0.0 { ((fog_spread - (temp - dew)) / fog_spread).clamp(0.0, 1.0) } else { 0.0 };
                let precip = if rain <= 0.05 {
                    PrecipKind::None
                } else if temp < 0.5 {
                    PrecipKind::Snow
                } else if rain < 0.5 {
                    PrecipKind::Drizzle
                } else {
                    PrecipKind::Rain
                };
                // Visibility: haze by humidity, rain, then fog.
                let mut vis = clear_vis * (1.0 - 0.85 * rh.powi(4));
                if rain > 0.05 {
                    vis = vis.min(f.get("env.weather.rain_vis_coeff_m") * rain.powf(-0.6));
                }
                if fog > 0.0 {
                    vis = vis.min(fog_vis / fog.max(0.05));
                }
                WeatherCell {
                    visibility_m: vis.max(30.0),
                    cloud_base_m: if fog > 0.5 { 30.0 } else { (125.0 * (temp - dew)).max(100.0) },
                    cloud_cover: cloud,
                    precip_mm_h: rain,
                    precip,
                    wind_surface: wsurf,
                    wind_850: w850,
                    wind_250: w250,
                    temperature_c: temp,
                    humidity: rh,
                    fog,
                }
            })
            .collect();
        frames.push(frame);
    }
    WeatherField { spec: s, start_utc: Some(p.start), times, frames }
}

#[derive(Debug, Error)]
pub enum WeatherImportError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("csv line {line}: {msg}")]
    Csv { line: usize, msg: String },
    #[error("bad grid: {0}")]
    Grid(String),
    #[error("date: {0}")]
    Date(#[from] crate::light::DateError),
}

/// The JSON layout for an offline ERA5 extract:
///
/// ```json
/// { "start_utc": "2022-03-01T00:00:00Z", "step_hours": 6,
///   "grid": { "width": 4, "height": 3, "cell": 25000, "origin_e": 0, "origin_n": 0 },
///   "frames": [ [ { "visibility_m": 20000, "temperature_c": 3.5, ... }, ... ], ... ] }
/// ```
///
/// Each frame lists `width × height` cells row-major (rows north); any
/// `WeatherCell` field may be omitted and takes the clear-preset value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Era5Json {
    pub start_utc: String,
    pub step_hours: f64,
    pub grid: GridSpec,
    pub frames: Vec<Vec<WeatherCell>>,
}

/// Import an ERA5 extract in the JSON layout above.
pub fn era5_from_json(text: &str) -> Result<WeatherField, WeatherImportError> {
    let j: Era5Json = serde_json::from_str(text)?;
    if j.grid.width == 0 || j.grid.height == 0 || j.grid.cell.is_nan() || j.grid.cell <= 0.0 {
        return Err(WeatherImportError::Grid("empty grid".into()));
    }
    if j.step_hours.is_nan() || j.step_hours <= 0.0 || j.frames.is_empty() {
        return Err(WeatherImportError::Grid("need frames and a positive step".into()));
    }
    for (i, fr) in j.frames.iter().enumerate() {
        if fr.len() != j.grid.len() {
            return Err(WeatherImportError::Grid(format!("frame {i} has {} cells, want {}", fr.len(), j.grid.len())));
        }
    }
    Ok(WeatherField {
        spec: j.grid,
        start_utc: Some(UtcDateTime::parse(&j.start_utc)?),
        times: (0..j.frames.len()).map(|k| SimTime::from_secs(k as f64 * j.step_hours * 3600.0)).collect(),
        frames: j.frames,
    })
}

/// CSV columns for an ERA5 extract, one row per (time, cell):
/// `time_h,i,j,visibility_m,cloud_base_m,cloud_cover,precip_mm_h,u10,v10,u850,v850,u250,v250,temp_c,rh,fog`.
/// `time_h` is hours since `start`; every (time, i, j) must appear once.
pub const ERA5_CSV_HEADER: &str = "time_h,i,j,visibility_m,cloud_base_m,cloud_cover,precip_mm_h,u10,v10,u850,v850,u250,v250,temp_c,rh,fog";

/// Import an ERA5 extract in the CSV layout above.
pub fn era5_from_csv(text: &str, grid: GridSpec, start: UtcDateTime) -> Result<WeatherField, WeatherImportError> {
    let mut rows: Vec<(f64, usize, usize, WeatherCell)> = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("time_h") {
            continue;
        }
        let v: Vec<f64> = line
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|e| WeatherImportError::Csv { line: ln + 1, msg: e.to_string() })?;
        if v.len() != 16 {
            return Err(WeatherImportError::Csv { line: ln + 1, msg: format!("{} columns, want 16", v.len()) });
        }
        if v[1] < 0.0 || v[2] < 0.0 || v[1] as usize >= grid.width || v[2] as usize >= grid.height {
            return Err(WeatherImportError::Csv { line: ln + 1, msg: "cell outside grid".into() });
        }
        let precip = if v[6] <= 0.05 {
            PrecipKind::None
        } else if v[13] < 0.5 {
            PrecipKind::Snow
        } else {
            PrecipKind::Rain
        };
        rows.push((
            v[0],
            v[1] as usize,
            v[2] as usize,
            WeatherCell {
                visibility_m: v[3],
                cloud_base_m: v[4],
                cloud_cover: v[5].clamp(0.0, 1.0),
                precip_mm_h: v[6].max(0.0),
                precip,
                wind_surface: (v[7], v[8]),
                wind_850: (v[9], v[10]),
                wind_250: (v[11], v[12]),
                temperature_c: v[13],
                humidity: v[14].clamp(0.0, 1.0),
                fog: v[15].clamp(0.0, 1.0),
            },
        ));
    }
    let mut hours: Vec<f64> = rows.iter().map(|r| r.0).collect();
    hours.sort_by(f64::total_cmp);
    hours.dedup();
    let mut frames = vec![vec![None; grid.len()]; hours.len()];
    for (h, i, j, c) in rows {
        let t = hours.partition_point(|x| *x < h);
        frames[t][grid.idx(i, j)] = Some(c);
    }
    let frames = frames
        .into_iter()
        .enumerate()
        .map(|(t, fr)| fr.into_iter().collect::<Option<Vec<_>>>().ok_or_else(|| WeatherImportError::Grid(format!("frame at {} h is incomplete", hours[t]))))
        .collect::<Result<Vec<_>, _>>()?;
    if frames.is_empty() {
        return Err(WeatherImportError::Grid("no rows".into()));
    }
    Ok(WeatherField { spec: grid, start_utc: Some(start), times: hours.iter().map(|h| SimTime::from_secs(h * 3600.0)).collect(), frames })
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("env.weather.equator_temp_c", 27.0, "degC", 20.0, 32.0, "default", Default, "annual mean temperature equatorward of 15°"),
        figure!("env.weather.temp_lapse_per_deg", 0.45, "degC/deg", 0.2, 0.8, "default", Default, "fall of annual mean temperature per degree of latitude"),
        figure!("env.weather.seasonal_amp_per_deg", 0.2, "degC/deg", 0.05, 0.5, "default", Default, "seasonal half-range per degree of latitude"),
        figure!("env.weather.diurnal_amp_c", 5.0, "degC", 1.0, 12.0, "default", Default, "clear-sky diurnal half-range of air temperature"),
        figure!("env.weather.dewpoint_spread_c", 4.0, "degC", 0.5, 20.0, "default", Default, "mean dew-point depression"),
        figure!("env.weather.mean_wind_m_s", 4.0, "m/s", 0.5, 15.0, "default", Default, "mean surface wind"),
        figure!("env.weather.jet_m_s", 30.0, "m/s", 5.0, 80.0, "default", Default, "upper (250 hPa) westerly"),
        figure!("env.weather.front_speed_m_s", 10.0, "m/s", 2.0, 25.0, "default", Default, "speed of a cold front"),
        figure!("env.weather.front_cloud_shield_m", 300_000.0, "m", 50_000.0, 800_000.0, "default", Default, "pre-frontal cloud shield depth"),
        figure!("env.weather.front_rain_band_m", 40_000.0, "m", 5_000.0, 200_000.0, "default", Default, "half-width of the frontal rain band"),
        figure!("env.weather.front_peak_rain_mm_h", 6.0, "mm/h", 0.5, 50.0, "default", Default, "peak frontal rain rate"),
        figure!("env.weather.front_temp_drop_c", 5.0, "degC", 0.0, 15.0, "default", Default, "temperature drop behind a cold front"),
        figure!("env.weather.fog_spread_c", 1.0, "degC", 0.2, 3.0, "default", Default, "dew-point depression below which radiation fog forms"),
        figure!("env.weather.fog_max_wind_m_s", 3.0, "m/s", 0.5, 6.0, "default", Default, "wind above which fog does not form"),
        figure!("env.weather.fog_visibility_m", 300.0, "m", 30.0, 1000.0, "default", Default, "visibility in moderate fog"),
        figure!("env.weather.clear_visibility_m", 40_000.0, "m", 10_000.0, 100_000.0, "default", Default, "visibility in dry clear air"),
        figure!("env.weather.rain_vis_coeff_m", 12_000.0, "m", 2_000.0, 40_000.0, "default", Default, "visibility in rain V = c·R^-0.6"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GridSpec {
        GridSpec::new(12, 10, 25_000.0, 0.0, 0.0)
    }

    #[test]
    fn presets_and_sampling() {
        let w = WeatherField::preset(spec(), WeatherPreset::Fog);
        let c = w.sample(1e5, 1e5, SimTime::from_secs_i(3600));
        assert!(c.visibility_m < 500.0 && c.fog > 0.5);
        let mut two = WeatherField::preset(spec(), WeatherPreset::Clear);
        two.times.push(SimTime::from_secs_i(3600));
        two.frames.push(vec![WeatherPreset::HeavyRain.cell(); spec().len()]);
        let mid = two.sample(0.0, 0.0, SimTime::from_secs_i(1800));
        assert!((mid.precip_mm_h - 12.5).abs() < 1e-9);
    }

    #[test]
    fn synthetic_front_passes_and_fog_forms_at_dawn() {
        let f = crate::default_figures();
        let p = SynthWeatherParams {
            seed: 3,
            spec: spec(),
            start: UtcDateTime::new(2024, 10, 10, 0, 0, 0.0),
            lat: 50.0,
            lon: 30.0,
            duration_h: 72.0,
            step_h: 1.0,
            fronts: 1,
        };
        let w = synthetic(&p, &f);
        assert_eq!(w.frames.len(), 73);
        assert_eq!(w, synthetic(&p, &f), "deterministic");
        // Somewhere, sometime, it rains under the front; temperatures drop behind it.
        let centre = (150_000.0, 112_500.0);
        let series: Vec<WeatherCell> = w.times.iter().map(|&t| w.sample(centre.0, centre.1, t)).collect();
        let rain_peak = series.iter().map(|c| c.precip_mm_h).fold(0.0, f64::max);
        assert!(rain_peak > 2.0, "{rain_peak}");
        let first = series[0].temperature_c;
        let last_mean = series[48..].iter().map(|c| c.temperature_c).sum::<f64>() / 25.0;
        assert!(last_mean < first + 3.0);
        // Diurnal cycle: afternoon warmer than pre-dawn on day 1 (local solar ≈ UTC+2).
        assert!(series[13].temperature_c > series[3].temperature_c);
    }

    #[test]
    fn calm_saturated_dawn_makes_fog() {
        let mut f = crate::default_figures();
        f.set("env.weather.dewpoint_spread_c", 4.5).unwrap();
        f.set("env.weather.diurnal_amp_c", 5.0).unwrap();
        f.set("env.weather.mean_wind_m_s", 1.0).unwrap();
        let p = SynthWeatherParams {
            seed: 4,
            spec: GridSpec::new(3, 3, 25_000.0, 0.0, 0.0),
            start: UtcDateTime::new(2024, 10, 10, 0, 0, 0.0),
            lat: 50.0,
            lon: 30.0,
            duration_h: 24.0,
            step_h: 1.0,
            fronts: 0,
        };
        let w = synthetic(&p, &f);
        let at = |h: i64| w.sample(25_000.0, 25_000.0, SimTime::from_secs_i(h * 3600));
        assert!(at(2).fog > 0.0 && at(2).visibility_m < 2_000.0, "pre-dawn fog: {:?}", at(2));
        assert_eq!(at(12).fog, 0.0, "burnt off by afternoon");
    }

    #[test]
    fn era5_imports() {
        let json = r#"{"start_utc":"2022-03-01T00:00:00Z","step_hours":6,
            "grid":{"width":2,"height":1,"cell":25000,"origin_e":0,"origin_n":0},
            "frames":[[{"temperature_c":1.0},{"temperature_c":3.0,"precip_mm_h":2.0,"precip":"rain"}],
                      [{"temperature_c":2.0},{"temperature_c":4.0}]]}"#;
        let w = era5_from_json(json).unwrap();
        assert_eq!(w.times[1], SimTime::from_secs_i(6 * 3600));
        assert!((w.sample(12_500.0, 0.0, SimTime::from_secs_i(3 * 3600)).temperature_c - 2.5).abs() < 1e-9);
        assert!(era5_from_json(r#"{"start_utc":"x","step_hours":6,"grid":{"width":2,"height":1,"cell":1,"origin_e":0,"origin_n":0},"frames":[[{}]]}"#).is_err());
        let csv = format!("{ERA5_CSV_HEADER}\n0,0,0,20000,900,0.5,0,3,1,8,2,30,0,5,0.7,0\n0,1,0,20000,900,0.5,1.5,3,1,8,2,30,0,-2,0.9,0\n");
        let g = GridSpec::new(2, 1, 25_000.0, 0.0, 0.0);
        let w = era5_from_csv(&csv, g, UtcDateTime::new(2022, 3, 1, 0, 0, 0.0)).unwrap();
        assert_eq!(w.frames[0][1].precip, PrecipKind::Snow);
        assert!(era5_from_csv("0,0,0,1", g, UtcDateTime::new(2022, 3, 1, 0, 0, 0.0)).is_err());
        let missing = format!("{ERA5_CSV_HEADER}\n0,0,0,20000,900,0.5,0,3,1,8,2,30,0,5,0.7,0\n");
        assert!(era5_from_csv(&missing, g, UtcDateTime::new(2022, 3, 1, 0, 0, 0.0)).is_err());
    }
}
