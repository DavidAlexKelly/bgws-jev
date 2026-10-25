//! The master clock: integer microseconds since the start of the game (§6.1).
//! Integers so that adding steps never drifts and ordering is exact.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Sub};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimTime(pub i64);

impl SimTime {
    pub const ZERO: SimTime = SimTime(0);
    pub const MAX: SimTime = SimTime(i64::MAX);
    pub const US_PER_S: i64 = 1_000_000;

    pub fn from_secs(s: f64) -> SimTime {
        SimTime((s * Self::US_PER_S as f64).round() as i64)
    }
    pub fn from_secs_i(s: i64) -> SimTime {
        SimTime(s * Self::US_PER_S)
    }
    pub fn from_mins(m: f64) -> SimTime {
        Self::from_secs(m * 60.0)
    }
    pub fn from_hours(h: f64) -> SimTime {
        Self::from_secs(h * 3600.0)
    }
    pub fn secs(self) -> f64 {
        self.0 as f64 / Self::US_PER_S as f64
    }
    pub fn whole_secs(self) -> i64 {
        self.0.div_euclid(Self::US_PER_S)
    }
    pub fn mins(self) -> f64 {
        self.secs() / 60.0
    }
    pub fn hours(self) -> f64 {
        self.secs() / 3600.0
    }
    /// The duration from `earlier` to `self`, in seconds (negative if `earlier` is later).
    pub fn since(self, earlier: SimTime) -> f64 {
        (self.0 - earlier.0) as f64 / Self::US_PER_S as f64
    }
    /// "1h 05m 30s" style, for the feed and the console.
    pub fn clock(self) -> String {
        let s = self.whole_secs();
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    }
}

impl Add<f64> for SimTime {
    type Output = SimTime;
    fn add(self, s: f64) -> SimTime {
        SimTime(self.0 + (s * Self::US_PER_S as f64).round() as i64)
    }
}
impl Add<SimTime> for SimTime {
    type Output = SimTime;
    fn add(self, o: SimTime) -> SimTime {
        SimTime(self.0.saturating_add(o.0))
    }
}
impl AddAssign<f64> for SimTime {
    fn add_assign(&mut self, s: f64) {
        *self = *self + s;
    }
}
impl Sub<SimTime> for SimTime {
    type Output = f64;
    fn sub(self, o: SimTime) -> f64 {
        self.since(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steps_do_not_drift() {
        let mut t = SimTime::ZERO;
        for _ in 0..1_000_000 {
            t += 0.1;
        }
        assert_eq!(t, SimTime::from_secs_i(100_000));
        assert_eq!(SimTime::from_secs(3725.0).clock(), "1:02:05");
    }
}
