//! Units of measure (§5.3): newtypes with explicit conversions. Models take
//! and return these at their boundaries; inside a formula they unwrap to f64.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Mul, Sub};

macro_rules! unit {
    ($(#[$m:meta])* $name:ident, $sym:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
        #[serde(transparent)]
        pub struct $name(pub f64);
        impl $name {
            pub const SYMBOL: &'static str = $sym;
            pub fn get(self) -> f64 { self.0 }
        }
        impl Add for $name { type Output = $name; fn add(self, o: $name) -> $name { $name(self.0 + o.0) } }
        impl Sub for $name { type Output = $name; fn sub(self, o: $name) -> $name { $name(self.0 - o.0) } }
        impl Mul<f64> for $name { type Output = $name; fn mul(self, k: f64) -> $name { $name(self.0 * k) } }
        impl Div<f64> for $name { type Output = $name; fn div(self, k: f64) -> $name { $name(self.0 / k) } }
    };
}

unit!(/// Length.
    Metres, "m");
unit!(/// Duration as a float, for physics. (Clock time is `SimTime`.)
    Seconds, "s");
unit!(/// Angle.
    Radians, "rad");
unit!(/// Power.
    Watts, "W");
unit!(/// A ratio or level in decibels (dB, dBW, dBm, dBi by context).
    Decibels, "dB");
unit!(/// Frequency.
    Hertz, "Hz");
unit!(/// Speed.
    MetresPerSecond, "m/s");

impl Metres {
    pub fn km(km: f64) -> Self { Metres(km * 1000.0) }
    pub fn as_km(self) -> f64 { self.0 / 1000.0 }
}
impl Radians {
    pub fn deg(d: f64) -> Self { Radians(d.to_radians()) }
    pub fn as_deg(self) -> f64 { self.0.to_degrees() }
}
impl Watts {
    pub fn to_dbw(self) -> Decibels { Decibels(10.0 * self.0.log10()) }
    pub fn to_dbm(self) -> Decibels { Decibels(10.0 * self.0.log10() + 30.0) }
}
impl Decibels {
    pub fn to_ratio(self) -> f64 { 10f64.powf(self.0 / 10.0) }
    pub fn from_ratio(r: f64) -> Self { Decibels(10.0 * r.log10()) }
    pub fn dbw_to_watts(self) -> Watts { Watts(self.to_ratio()) }
}
impl Hertz {
    pub fn mhz(m: f64) -> Self { Hertz(m * 1e6) }
    pub fn ghz(g: f64) -> Self { Hertz(g * 1e9) }
    /// Free-space wavelength.
    pub fn wavelength(self) -> Metres { Metres(SPEED_OF_LIGHT / self.0) }
}
impl MetresPerSecond {
    pub fn kmh(k: f64) -> Self { MetresPerSecond(k / 3.6) }
    pub fn knots(k: f64) -> Self { MetresPerSecond(k * 0.514_444) }
    pub fn as_kmh(self) -> f64 { self.0 * 3.6 }
}

pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;
pub const BOLTZMANN: f64 = 1.380_649e-23;
pub const STANDARD_TEMPERATURE_K: f64 = 290.0;
pub const G0: f64 = 9.806_65;

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    #[test]
    fn conversions() {
        assert_relative_eq!(Watts(1.0).to_dbm().0, 30.0);
        assert_relative_eq!(Decibels(3.0).to_ratio(), 1.995, epsilon = 1e-3);
        assert_relative_eq!(Hertz::ghz(10.0).wavelength().0, 0.02998, epsilon = 1e-4);
        assert_relative_eq!(MetresPerSecond::kmh(36.0).0, 10.0);
    }
}
