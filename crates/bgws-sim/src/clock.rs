//! Multi-rate stepping (§6.1). One master clock in microseconds; each system
//! group runs at its own rate. Fast sub-steps run only while something fast is
//! in flight, so an empty sky costs nothing.

use bgws_core::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rate {
    /// 20 Hz, only while munitions are in flight.
    Fast,
    /// 10 Hz: aircraft, drones, fast radar tracks.
    Air,
    /// 1 Hz: the base tick.
    Base,
    /// 0.1 Hz: ships, sonar, HQ staff work, fusion cycles.
    Slow,
    /// 1/60 Hz: logistics, maintenance, medical, fatigue, weather.
    Minute,
}

impl Rate {
    pub const ALL: [Rate; 5] = [Rate::Fast, Rate::Air, Rate::Base, Rate::Slow, Rate::Minute];

    pub fn period(self) -> SimDuration {
        match self {
            Rate::Fast => SimDuration::from_millis(50),
            Rate::Air => SimDuration::from_millis(100),
            Rate::Base => SimDuration::from_secs_i(1),
            Rate::Slow => SimDuration::from_secs_i(10),
            Rate::Minute => SimDuration::from_secs_i(60),
        }
    }

    /// Is `t` a boundary of this rate?
    pub fn due(self, t: SimTime) -> bool {
        t.0.rem_euclid(self.period().0) == 0
    }
}

/// The next time the clock must stop: the next boundary of the fastest rate
/// that is active.
pub fn next_stop(now: SimTime, fast_active: bool, air_active: bool) -> SimTime {
    let p = if fast_active {
        Rate::Fast.period()
    } else if air_active {
        Rate::Air.period()
    } else {
        Rate::Base.period()
    };
    SimTime((now.0.div_euclid(p.0) + 1) * p.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries() {
        let t = SimTime::from_secs_i(60);
        assert!(Rate::ALL.iter().all(|r| r.due(t)));
        assert!(!Rate::Minute.due(SimTime::from_secs_i(30)));
        assert_eq!(next_stop(SimTime::from_secs(1.2), false, false), SimTime::from_secs_i(2));
        assert_eq!(next_stop(SimTime::from_secs(1.2), true, false), SimTime::from_secs(1.25));
    }
}
