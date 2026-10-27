//! Simulated time (§6.1): one master clock in integer microseconds.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Sub};

pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// A point in simulated time, in microseconds since the scenario's start.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "number"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct SimTime(pub i64);

/// A length of simulated time in microseconds.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "number"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct SimDuration(pub i64);

impl SimTime {
    pub const ZERO: SimTime = SimTime(0);
    pub const MAX: SimTime = SimTime(i64::MAX);

    pub fn from_secs(s: f64) -> Self {
        SimTime((s * MICROS_PER_SECOND as f64).round() as i64)
    }
    pub fn from_secs_i(s: i64) -> Self {
        SimTime(s * MICROS_PER_SECOND)
    }
    pub fn secs(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND as f64
    }
    pub fn micros(self) -> i64 {
        self.0
    }
    /// Whole simulated seconds elapsed (the base-rate tick number).
    pub fn tick(self) -> i64 {
        self.0.div_euclid(MICROS_PER_SECOND)
    }
    pub fn since(self, earlier: SimTime) -> SimDuration {
        SimDuration(self.0 - earlier.0)
    }
}

impl SimDuration {
    pub const ZERO: SimDuration = SimDuration(0);
    pub fn from_secs(s: f64) -> Self {
        SimDuration((s * MICROS_PER_SECOND as f64).round() as i64)
    }
    pub fn from_secs_i(s: i64) -> Self {
        SimDuration(s * MICROS_PER_SECOND)
    }
    pub fn from_millis(ms: i64) -> Self {
        SimDuration(ms * 1000)
    }
    pub fn from_mins(m: f64) -> Self {
        Self::from_secs(m * 60.0)
    }
    pub fn from_hours(h: f64) -> Self {
        Self::from_secs(h * 3600.0)
    }
    pub fn secs(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND as f64
    }
}

impl Add<SimDuration> for SimTime {
    type Output = SimTime;
    fn add(self, d: SimDuration) -> SimTime {
        SimTime(self.0.saturating_add(d.0))
    }
}
impl AddAssign<SimDuration> for SimTime {
    fn add_assign(&mut self, d: SimDuration) {
        self.0 = self.0.saturating_add(d.0);
    }
}
impl Sub for SimTime {
    type Output = SimDuration;
    fn sub(self, o: SimTime) -> SimDuration {
        SimDuration(self.0 - o.0)
    }
}
impl Add for SimDuration {
    type Output = SimDuration;
    fn add(self, o: SimDuration) -> SimDuration {
        SimDuration(self.0 + o.0)
    }
}

impl fmt::Display for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.0.div_euclid(MICROS_PER_SECOND);
        let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
        let ms = self.0.rem_euclid(MICROS_PER_SECOND) / 1000;
        write!(f, "{h:02}:{m:02}:{s:02}.{ms:03}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_and_display() {
        let t = SimTime::from_secs(3725.25);
        assert_eq!(t.tick(), 3725);
        assert_eq!(format!("{t}"), "01:02:05.250");
        assert_eq!((t + SimDuration::from_secs(0.75)).tick(), 3726);
        assert_eq!((SimTime::from_secs_i(10) - SimTime::from_secs_i(4)).secs(), 6.0);
    }
}
