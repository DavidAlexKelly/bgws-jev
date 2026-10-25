//! Light (§7.4): sun and moon positions for a real UTC date-time and place,
//! twilight phases, and ground illuminance from sun, moon, starlight and
//! cloud — what eye and image-intensifier performance reads.
//!
//! - **Sun:** the NOAA solar position algorithm (the NOAA Solar Calculator
//!   spreadsheet's equations, after Meeus), with its atmospheric refraction
//!   correction; good to well under 0.1° for 1900–2100.
//! - **Moon:** a low-precision lunar ephemeris (the largest periodic terms
//!   of Meeus, *Astronomical Algorithms* ch. 47), topocentric parallax, and
//!   the illuminated fraction from the phase angle (ch. 48). Good to a few
//!   tenths of a degree, ample for illumination.
//! - **Illuminance:** a clear-sky solar illuminance curve against elevation
//!   (twilight through zenith), the full-moon illuminance scaled by the
//!   Allen lunar phase law and airmass extinction, a starlight/airglow floor,
//!   and cloud transmission by the Kasten–Czeplak form `1 − a·N^b`.

use bgws_core::{figure, Figure, Figures, SimTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A UTC calendar date-time (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UtcDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
}

#[derive(Debug, Error, PartialEq)]
pub enum DateError {
    #[error("bad date-time `{0}` (want YYYY-MM-DDTHH:MM:SSZ)")]
    Parse(String),
}

impl UtcDateTime {
    pub fn new(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> Self {
        Self { year, month, day, hour, minute, second }
    }

    /// Parse `YYYY-MM-DDTHH:MM[:SS[.s]]Z` (the `Z` optional).
    pub fn parse(s: &str) -> Result<Self, DateError> {
        let err = || DateError::Parse(s.to_string());
        let t = s.trim().trim_end_matches('Z');
        let (d, tm) = t.split_once(['T', ' ']).ok_or_else(err)?;
        let dp: Vec<&str> = d.split('-').collect();
        let tp: Vec<&str> = tm.split(':').collect();
        if dp.len() != 3 || !(2..=3).contains(&tp.len()) {
            return Err(err());
        }
        let year: i32 = dp[0].parse().map_err(|_| err())?;
        let month: u32 = dp[1].parse().map_err(|_| err())?;
        let day: u32 = dp[2].parse().map_err(|_| err())?;
        let hour: u32 = tp[0].parse().map_err(|_| err())?;
        let minute: u32 = tp[1].parse().map_err(|_| err())?;
        let second: f64 = if tp.len() == 3 { tp[2].parse().map_err(|_| err())? } else { 0.0 };
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || !(0.0..61.0).contains(&second) {
            return Err(err());
        }
        Ok(Self { year, month, day, hour, minute, second })
    }

    /// Julian Day (Meeus ch. 7).
    pub fn julian_day(&self) -> f64 {
        let (mut y, mut m) = (self.year as f64, self.month as f64);
        if m <= 2.0 {
            y -= 1.0;
            m += 12.0;
        }
        let a = (y / 100.0).floor();
        let b = 2.0 - a + (a / 4.0).floor();
        let frac = (self.hour as f64 + self.minute as f64 / 60.0 + self.second / 3600.0) / 24.0;
        (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + self.day as f64 + frac + b - 1524.5
    }

    /// From a Julian Day.
    pub fn from_julian_day(jd: f64) -> Self {
        let z = (jd + 0.5).floor();
        let f = jd + 0.5 - z;
        let a = if z < 2_299_161.0 {
            z
        } else {
            let al = ((z - 1_867_216.25) / 36_524.25).floor();
            z + 1.0 + al - (al / 4.0).floor()
        };
        let b = a + 1524.0;
        let c = ((b - 122.1) / 365.25).floor();
        let d = (365.25 * c).floor();
        let e = ((b - d) / 30.6001).floor();
        let day = (b - d - (30.6001 * e).floor()) as u32;
        let month = if e < 14.0 { e - 1.0 } else { e - 13.0 } as u32;
        let year = if month > 2 { c - 4716.0 } else { c - 4715.0 } as i32;
        let secs = (f * 86_400.0 * 1000.0).round() / 1000.0;
        let hour = (secs / 3600.0).floor() as u32;
        let minute = ((secs - hour as f64 * 3600.0) / 60.0).floor() as u32;
        let second = secs - hour as f64 * 3600.0 - minute as f64 * 60.0;
        Self { year, month, day, hour: hour.min(23), minute: minute.min(59), second }
    }

    pub fn add_seconds(&self, s: f64) -> Self {
        Self::from_julian_day(self.julian_day() + s / 86_400.0)
    }

    /// The UTC time of simulated time `t` when the scenario starts at `self`.
    pub fn at(&self, t: SimTime) -> Self {
        self.add_seconds(t.secs())
    }

    /// Minutes past UTC midnight.
    pub fn minutes_of_day(&self) -> f64 {
        self.hour as f64 * 60.0 + self.minute as f64 + self.second / 60.0
    }

    /// Day of the year (1 = 1 January).
    pub fn day_of_year(&self) -> u32 {
        let jan1 = UtcDateTime::new(self.year, 1, 1, 0, 0, 0.0).julian_day();
        let d0 = UtcDateTime::new(self.year, self.month, self.day, 0, 0, 0.0).julian_day();
        (d0 - jan1) as u32 + 1
    }

    /// Local mean solar hour at longitude `lon` (degrees east).
    pub fn local_solar_hour(&self, lon: f64) -> f64 {
        (self.minutes_of_day() / 60.0 + lon / 15.0).rem_euclid(24.0)
    }
}

fn sind(x: f64) -> f64 {
    x.to_radians().sin()
}
fn cosd(x: f64) -> f64 {
    x.to_radians().cos()
}

/// The sun as seen from a place.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SunPosition {
    /// Apparent elevation including refraction, degrees.
    pub elevation_deg: f64,
    /// Geometric elevation, degrees.
    pub true_elevation_deg: f64,
    /// Azimuth, degrees clockwise from north.
    pub azimuth_deg: f64,
    pub declination_deg: f64,
    /// Equation of time, minutes.
    pub equation_of_time_min: f64,
    /// Apparent ecliptic longitude, degrees (for lunar phase).
    pub ecliptic_longitude_deg: f64,
    /// Sun–Earth distance, AU.
    pub distance_au: f64,
    /// Solar noon, minutes after UTC midnight on this date.
    pub solar_noon_utc_min: f64,
}

/// NOAA solar position for a UTC time at latitude/longitude (degrees, east positive).
pub fn sun_position(t: &UtcDateTime, lat: f64, lon: f64) -> SunPosition {
    let jd = t.julian_day();
    let jc = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + jc * (36_000.769_83 + jc * 0.000_303_2)).rem_euclid(360.0);
    let m = 357.52911 + jc * (35_999.050_29 - 0.000_153_7 * jc);
    let e = 0.016_708_634 - jc * (0.000_042_037 + 0.000_000_126_7 * jc);
    let c = sind(m) * (1.914_602 - jc * (0.004_817 + 0.000_014 * jc)) + sind(2.0 * m) * (0.019_993 - 0.000_101 * jc) + sind(3.0 * m) * 0.000_289;
    let true_long = l0 + c;
    let true_anom = m + c;
    let dist = (1.000_001_018 * (1.0 - e * e)) / (1.0 + e * cosd(true_anom));
    let omega = 125.04 - 1934.136 * jc;
    let app_long = true_long - 0.005_69 - 0.004_78 * sind(omega);
    let mean_obliq = 23.0 + (26.0 + (21.448 - jc * (46.815 + jc * (0.000_59 - jc * 0.001_813))) / 60.0) / 60.0;
    let obliq = mean_obliq + 0.002_56 * cosd(omega);
    let decl = (sind(obliq) * sind(app_long)).asin().to_degrees();
    let vy = (obliq / 2.0).to_radians().tan().powi(2);
    let eot = 4.0
        * (vy * sind(2.0 * l0) - 2.0 * e * sind(m) + 4.0 * e * vy * sind(m) * cosd(2.0 * l0)
            - 0.5 * vy * vy * sind(4.0 * l0)
            - 1.25 * e * e * sind(2.0 * m))
            .to_degrees();
    let tst = (t.minutes_of_day() + eot + 4.0 * lon).rem_euclid(1440.0);
    let ha = if tst / 4.0 < 0.0 { tst / 4.0 + 180.0 } else { tst / 4.0 - 180.0 };
    let cos_zen = (sind(lat) * sind(decl) + cosd(lat) * cosd(decl) * cosd(ha)).clamp(-1.0, 1.0);
    let zen = cos_zen.acos().to_degrees();
    let elev = 90.0 - zen;
    let refr = refraction_deg(elev);
    let az = {
        let denom = cosd(lat) * sind(zen);
        let a = if denom.abs() < 1e-12 { 0.0 } else { ((sind(lat) * cos_zen - sind(decl)) / denom).clamp(-1.0, 1.0).acos().to_degrees() };
        if ha > 0.0 {
            (a + 180.0).rem_euclid(360.0)
        } else {
            (540.0 - a).rem_euclid(360.0)
        }
    };
    SunPosition {
        elevation_deg: elev + refr,
        true_elevation_deg: elev,
        azimuth_deg: az,
        declination_deg: decl,
        equation_of_time_min: eot,
        ecliptic_longitude_deg: app_long.rem_euclid(360.0),
        distance_au: dist,
        solar_noon_utc_min: 720.0 - 4.0 * lon - eot,
    }
}

/// NOAA's atmospheric refraction correction, degrees, for a geometric elevation.
pub fn refraction_deg(elev: f64) -> f64 {
    let te = elev.to_radians().tan();
    let arcsec = if elev > 85.0 {
        0.0
    } else if elev > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000_086 / te.powi(5)
    } else if elev > -0.575 {
        1735.0 + elev * (-518.2 + elev * (103.4 + elev * (-12.79 + elev * 0.711)))
    } else {
        -20.772 / te
    };
    arcsec / 3600.0
}

/// UTC minutes after midnight of sunrise and sunset (or twilight start/end)
/// for a solar depression `depression_deg` (0.833 sunrise, 6 civil, 12, 18).
/// `None` in polar day or night.
pub fn sun_event_times(date: &UtcDateTime, lat: f64, lon: f64, depression_deg: f64) -> Option<(f64, f64)> {
    // Iterate twice around solar noon for accuracy.
    let mut noon = UtcDateTime::new(date.year, date.month, date.day, 12, 0, 0.0);
    let mut out = None;
    for _ in 0..2 {
        let s = sun_position(&noon, lat, lon);
        let x = (cosd(90.0 + depression_deg) / (cosd(lat) * cosd(s.declination_deg)) - (lat.to_radians().tan() * s.declination_deg.to_radians().tan()))
            .clamp(-2.0, 2.0);
        if !(-1.0..=1.0).contains(&x) {
            return None;
        }
        let ha = x.acos().to_degrees();
        out = Some((s.solar_noon_utc_min - 4.0 * ha, s.solar_noon_utc_min + 4.0 * ha));
        let day0 = UtcDateTime::new(date.year, date.month, date.day, 0, 0, 0.0);
        noon = day0.add_seconds(s.solar_noon_utc_min * 60.0);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TwilightPhase {
    Day,
    Civil,
    Nautical,
    Astronomical,
    Night,
}

/// Twilight phase from the sun's apparent elevation.
pub fn twilight_phase(sun_elev_deg: f64) -> TwilightPhase {
    if sun_elev_deg > -0.833 {
        TwilightPhase::Day
    } else if sun_elev_deg > -6.0 {
        TwilightPhase::Civil
    } else if sun_elev_deg > -12.0 {
        TwilightPhase::Nautical
    } else if sun_elev_deg > -18.0 {
        TwilightPhase::Astronomical
    } else {
        TwilightPhase::Night
    }
}

/// The moon as seen from a place.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoonPosition {
    /// Topocentric elevation (parallax-corrected, with refraction), degrees.
    pub elevation_deg: f64,
    pub azimuth_deg: f64,
    pub distance_km: f64,
    /// Phase angle (sun–moon–earth), degrees; 0 full, 180 new.
    pub phase_angle_deg: f64,
    /// Illuminated fraction of the disc, 0–1.
    pub illuminated_fraction: f64,
    pub waxing: bool,
    pub ecliptic_longitude_deg: f64,
    pub ecliptic_latitude_deg: f64,
}

/// Low-precision lunar position and phase.
pub fn moon_position(t: &UtcDateTime, lat: f64, lon: f64) -> MoonPosition {
    let jd = t.julian_day();
    let tc = (jd - 2_451_545.0) / 36_525.0;
    let lp = 218.316_447_7 + 481_267.881_234_21 * tc;
    let d = 297.850_192_1 + 445_267.111_403_4 * tc;
    let m = 357.529_109_2 + 35_999.050_290_9 * tc;
    let mp = 134.963_396_4 + 477_198.867_505_5 * tc;
    let f = 93.272_095_0 + 483_202.017_523_3 * tc;
    let lambda = lp + 6.288_774 * sind(mp) + 1.274_027 * sind(2.0 * d - mp) + 0.658_314 * sind(2.0 * d) + 0.213_618 * sind(2.0 * mp)
        - 0.185_116 * sind(m)
        - 0.114_332 * sind(2.0 * f)
        + 0.058_793 * sind(2.0 * d - 2.0 * mp)
        + 0.057_066 * sind(2.0 * d - m - mp)
        + 0.053_322 * sind(2.0 * d + mp)
        + 0.045_758 * sind(2.0 * d - m)
        - 0.040_923 * sind(m - mp)
        - 0.034_720 * sind(d)
        - 0.030_383 * sind(m + mp);
    let beta = 5.128_122 * sind(f) + 0.280_602 * sind(mp + f) + 0.277_693 * sind(mp - f) + 0.173_237 * sind(2.0 * d - f)
        + 0.055_413 * sind(2.0 * d - mp + f)
        + 0.046_271 * sind(2.0 * d - mp - f);
    let dist = 385_000.56 - 20_905.355 * cosd(mp) - 3_699.111 * cosd(2.0 * d - mp) - 2_955.968 * cosd(2.0 * d) - 569.925 * cosd(2.0 * mp);
    let eps = 23.439_291 - 0.013_004_2 * tc;
    let lam = lambda.rem_euclid(360.0);
    let ra = (sind(lam) * cosd(eps) - beta.to_radians().tan() * sind(eps)).atan2(cosd(lam)).to_degrees();
    let dec = (sind(beta) * cosd(eps) + cosd(beta) * sind(eps) * sind(lam)).clamp(-1.0, 1.0).asin().to_degrees();
    let gmst = 280.460_618_37 + 360.985_647_366_29 * (jd - 2_451_545.0) + 0.000_387_933 * tc * tc - tc * tc * tc / 38_710_000.0;
    let h = (gmst + lon - ra).rem_euclid(360.0);
    let alt_geo = (sind(lat) * sind(dec) + cosd(lat) * cosd(dec) * cosd(h)).clamp(-1.0, 1.0).asin().to_degrees();
    let az = (sind(h)).atan2(cosd(h) * sind(lat) - dec.to_radians().tan() * cosd(lat)).to_degrees();
    let az = (az + 180.0).rem_euclid(360.0);
    let parallax = (6_378.14 / dist).asin().to_degrees() * cosd(alt_geo);
    let alt = alt_geo - parallax;
    let sun = sun_position(t, lat, lon);
    let r_km = sun.distance_au * 149_597_870.7;
    let cos_psi = cosd(beta) * cosd(lam - sun.ecliptic_longitude_deg);
    let psi = cos_psi.clamp(-1.0, 1.0).acos();
    let i = (r_km * psi.sin()).atan2(dist - r_km * cos_psi).to_degrees();
    MoonPosition {
        elevation_deg: alt + refraction_deg(alt),
        azimuth_deg: az,
        distance_km: dist,
        phase_angle_deg: i,
        illuminated_fraction: (1.0 + cosd(i)) / 2.0,
        waxing: (lam - sun.ecliptic_longitude_deg).rem_euclid(360.0) < 180.0,
        ecliptic_longitude_deg: lam,
        ecliptic_latitude_deg: beta,
    }
}

/// Illuminance parameters, from figures.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightParams {
    pub sun_zenith_lux: f64,
    pub full_moon_lux: f64,
    pub starlight_lux: f64,
    pub extinction_mag: f64,
    pub cloud_a: f64,
    pub cloud_b: f64,
}

impl LightParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            sun_zenith_lux: f.get("env.light.sun_zenith_lux"),
            full_moon_lux: f.get("env.light.full_moon_lux"),
            starlight_lux: f.get("env.light.starlight_lux"),
            extinction_mag: f.get("env.light.extinction_mag_per_airmass"),
            cloud_a: f.get("env.light.cloud_a"),
            cloud_b: f.get("env.light.cloud_b"),
        }
    }
}

/// Clear-sky horizontal illuminance from the sun relative to the zenith
/// value, by sun elevation: log10 table through the twilights
/// (typical published values: ~400 lux at sunset, ~3 lux at the end of civil
/// twilight, ~0.008 at the end of nautical).
const SUN_CURVE: [(f64, f64); 15] = [
    (-18.0, -8.2),
    (-12.0, -7.18),
    (-9.0, -5.9),
    (-6.0, -4.55),
    (-3.0, -3.48),
    (-0.833, -2.48),
    (0.0, -2.23),
    (5.0, -1.48),
    (10.0, -1.08),
    (20.0, -0.63),
    (30.0, -0.38),
    (45.0, -0.18),
    (60.0, -0.08),
    (75.0, -0.02),
    (90.0, 0.0),
];

fn sun_relative(elev: f64) -> f64 {
    if elev <= SUN_CURVE[0].0 {
        return 0.0;
    }
    let k = SUN_CURVE.partition_point(|p| p.0 < elev).min(SUN_CURVE.len() - 1);
    let (a, b) = (SUN_CURVE[k - 1], SUN_CURVE[k]);
    let lg = a.1 + (b.1 - a.1) * (elev - a.0) / (b.0 - a.0);
    10f64.powf(lg)
}

/// Ground illuminance and its parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Illuminance {
    pub sun_lux: f64,
    pub moon_lux: f64,
    pub sky_lux: f64,
    /// Transmission of the cloud layer, 0–1.
    pub cloud_factor: f64,
    pub total_lux: f64,
    pub phase: TwilightPhase,
}

/// Illuminance from the sun and moon geometry and fractional cloud cover.
pub fn illuminance(sun_elev_deg: f64, moon: Option<&MoonPosition>, cloud_cover: f64, p: &LightParams) -> Illuminance {
    let cloud_factor = (1.0 - p.cloud_a * cloud_cover.clamp(0.0, 1.0).powf(p.cloud_b)).clamp(0.0, 1.0);
    let sun_lux = p.sun_zenith_lux * sun_relative(sun_elev_deg);
    let moon_lux = match moon {
        Some(m) if m.elevation_deg > 0.0 => {
            let i = m.phase_angle_deg.abs();
            let phase = 10f64.powf(-0.4 * (0.026 * i + 4e-9 * i.powi(4)));
            let s = sind(m.elevation_deg);
            let airmass = (1.0 / s.max(0.025)).min(40.0);
            p.full_moon_lux * phase * s * 10f64.powf(-0.4 * p.extinction_mag * (airmass - 1.0))
        }
        _ => 0.0,
    };
    let sky_lux = p.starlight_lux;
    Illuminance {
        sun_lux,
        moon_lux,
        sky_lux,
        cloud_factor,
        total_lux: (sun_lux + moon_lux + sky_lux) * cloud_factor,
        phase: twilight_phase(sun_elev_deg),
    }
}

/// Illuminance at a place and time.
pub fn illuminance_at(t: &UtcDateTime, lat: f64, lon: f64, cloud_cover: f64, p: &LightParams) -> Illuminance {
    let s = sun_position(t, lat, lon);
    let m = moon_position(t, lat, lon);
    illuminance(s.elevation_deg, Some(&m), cloud_cover, p)
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("env.light.sun_zenith_lux", 120_000.0, "lx", 80_000.0, 140_000.0, "default", Low, "clear-sky horizontal illuminance, sun at zenith"),
        figure!("env.light.full_moon_lux", 0.27, "lx", 0.1, 0.5, "default", Low, "horizontal illuminance of a full moon at zenith"),
        figure!("env.light.starlight_lux", 0.001, "lx", 0.0001, 0.005, "default", Low, "moonless clear-night floor (starlight and airglow)"),
        figure!("env.light.extinction_mag_per_airmass", 0.2, "mag", 0.05, 1.0, "default", Default, "atmospheric extinction of moonlight per airmass"),
        figure!("env.light.cloud_a", 0.75, "1", 0.3, 0.95, "default", Low, "Kasten–Czeplak cloud transmission 1 − a·N^b: a"),
        figure!("env.light.cloud_b", 3.4, "1", 1.0, 5.0, "default", Low, "Kasten–Czeplak cloud transmission: b"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn lp() -> LightParams {
        LightParams::from_figures(&Figures::from_list(figures()).unwrap())
    }

    #[test]
    fn julian_day_round_trip() {
        // Meeus example 7.a: 1957 Oct 4.81 = JD 2436116.31.
        let t = UtcDateTime::new(1957, 10, 4, 19, 26, 24.0);
        assert_relative_eq!(t.julian_day(), 2_436_116.31, epsilon = 1e-6);
        assert_relative_eq!(UtcDateTime::new(2000, 1, 1, 12, 0, 0.0).julian_day(), 2_451_545.0);
        let back = UtcDateTime::from_julian_day(t.julian_day());
        assert_eq!((back.year, back.month, back.day, back.hour, back.minute), (1957, 10, 4, 19, 26));
        assert_eq!(UtcDateTime::parse("2024-03-20T03:06:00Z").unwrap(), UtcDateTime::new(2024, 3, 20, 3, 6, 0.0));
        assert!(UtcDateTime::parse("2024-13-01T00:00Z").is_err());
        assert_eq!(UtcDateTime::new(2024, 3, 1, 0, 0, 0.0).day_of_year(), 61);
    }

    #[test]
    fn equinox_noon_elevation_is_ninety_minus_latitude() {
        // March equinox 2024-03-20 03:06 UTC; at solar noon that day δ ≈ +0.2°.
        let lat = 50.0;
        let lon = 30.0;
        let d = UtcDateTime::new(2024, 3, 20, 0, 0, 0.0);
        let s0 = sun_position(&d, lat, lon);
        let noon = d.add_seconds(s0.solar_noon_utc_min * 60.0);
        let s = sun_position(&noon, lat, lon);
        assert!(s.declination_deg.abs() < 0.5, "{}", s.declination_deg);
        assert_relative_eq!(s.true_elevation_deg, 90.0 - lat + s.declination_deg, epsilon = 0.02);
        assert!((s.azimuth_deg - 180.0).abs() < 0.5);
        // Solar noon at 30°E is near 10:00 UTC (EoT ≈ −7.5 min in late March).
        assert!((s.solar_noon_utc_min - (600.0 + 7.5)).abs() < 1.5, "{}", s.solar_noon_utc_min);
        // Equinox day length a little over 12 h (refraction + disc).
        let (rise, set) = sun_event_times(&d, lat, lon, 0.833).unwrap();
        assert!((set - rise - 12.0 * 60.0 - 10.0).abs() < 6.0, "{}", set - rise);
    }

    #[test]
    fn solstice_noon_elevation() {
        // 2024-06-20 20:51 UTC solstice: δ ≈ 23.44°; Denver-ish 40°N 105°W.
        let d = UtcDateTime::new(2024, 6, 21, 0, 0, 0.0);
        let s0 = sun_position(&d, 40.0, -105.0);
        let s = sun_position(&d.add_seconds(s0.solar_noon_utc_min * 60.0), 40.0, -105.0);
        assert_relative_eq!(s.declination_deg, 23.44, epsilon = 0.03);
        assert_relative_eq!(s.true_elevation_deg, 73.44, epsilon = 0.05);
        assert!(sun_event_times(&d, 80.0, 0.0, 0.833).is_none(), "polar day");
    }

    #[test]
    fn lunar_phase_at_known_full_and_new_moons() {
        let full = [UtcDateTime::new(2024, 4, 23, 23, 49, 0.0), UtcDateTime::new(2000, 1, 21, 4, 40, 0.0), UtcDateTime::new(2025, 9, 7, 18, 9, 0.0)];
        let new = [UtcDateTime::new(2024, 4, 8, 18, 21, 0.0), UtcDateTime::new(2000, 1, 6, 18, 14, 0.0), UtcDateTime::new(2025, 1, 29, 12, 36, 0.0)];
        for t in full {
            let m = moon_position(&t, 0.0, 0.0);
            assert!(m.illuminated_fraction > 0.995, "{t:?} {m:?}");
        }
        for t in new {
            let m = moon_position(&t, 0.0, 0.0);
            assert!(m.illuminated_fraction < 0.005, "{t:?} {m:?}");
        }
        // First quarter 2024-04-15 19:13 UTC: half lit and waxing.
        let q = moon_position(&UtcDateTime::new(2024, 4, 15, 19, 13, 0.0), 0.0, 0.0);
        assert!((q.illuminated_fraction - 0.5).abs() < 0.03 && q.waxing, "{q:?}");
        // Total solar eclipse 2024-04-08 ~18:18 UTC near 25.3°N 104.1°W: the moon sits on the sun.
        let t = UtcDateTime::new(2024, 4, 8, 18, 18, 0.0);
        let (s, m) = (sun_position(&t, 25.29, -104.14), moon_position(&t, 25.29, -104.14));
        let sep = (sind(s.elevation_deg) * sind(m.elevation_deg)
            + cosd(s.elevation_deg) * cosd(m.elevation_deg) * cosd(s.azimuth_deg - m.azimuth_deg))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
        assert!(sep < 0.5, "separation {sep}: {s:?} {m:?}");
    }

    #[test]
    fn illuminance_orders_of_magnitude() {
        let p = lp();
        assert!(illuminance(60.0, None, 0.0, &p).total_lux > 90_000.0);
        let sunset = illuminance(-0.833, None, 0.0, &p).total_lux;
        assert!((200.0..1000.0).contains(&sunset));
        let civil = illuminance(-6.0, None, 0.0, &p).total_lux;
        assert!((1.0..10.0).contains(&civil));
        let full = MoonPosition { elevation_deg: 60.0, azimuth_deg: 0.0, distance_km: 384_400.0, phase_angle_deg: 0.0, illuminated_fraction: 1.0, waxing: true, ecliptic_longitude_deg: 0.0, ecliptic_latitude_deg: 0.0 };
        let quarter = MoonPosition { phase_angle_deg: 90.0, ..full };
        let fm = illuminance(-30.0, Some(&full), 0.0, &p).total_lux;
        let qm = illuminance(-30.0, Some(&quarter), 0.0, &p).total_lux;
        let none = illuminance(-30.0, None, 0.0, &p).total_lux;
        assert!(fm > 0.15 && fm < 0.3, "{fm}");
        assert!(qm < fm / 5.0 && qm > none);
        assert!(illuminance(-30.0, Some(&full), 1.0, &p).total_lux < fm * 0.3, "overcast darkens");
        assert_eq!(twilight_phase(-7.0), TwilightPhase::Nautical);
    }
}
