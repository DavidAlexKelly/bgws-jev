//! Cyber as processes, not packets (RUST_ENGINE.md §8.12; research R5.5, P21).
//!
//! An **access campaign** against a target system builds access over months
//! (Kyivstar took 6–8 months **[P21]**) with a daily hazard of being
//! discovered; once ready it is triggered at a chosen time (often H-hour)
//! and gives a **one-shot effect window of days** (2–3 days for Kyivstar),
//! after which the access is burned.
//!
//! **Effects** are time-limited multipliers or outages on named processes
//! that other domains own: air-defence track sharing, the logistics picture,
//! the mobile network, satcom terminals, air-raid warning, and crypto keys
//! (which [`crate::comms::CommsNetwork::compromise_key`] then honours). No
//! packet-level simulation **[ref Conclusion: modifiers, not mini-games]**.

use bgws_core::{figure, hazard_probability, Figure, Figures, SimDuration, SimTime};
use rand::Rng;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Figures for cyber (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("cyber.build_days_mean", 210.0, "day", 30.0, 400.0, "P21", Medium, "access building ~6–8 months (Kyivstar)"),
        figure!("cyber.build_days_sd", 30.0, "day", 0.0, 120.0, "default", Default, "spread of access-building time"),
        figure!("cyber.build_days_min", 21.0, "day", 1.0, 120.0, "default", Default, "fastest plausible access build ('weeks to months')"),
        figure!("cyber.discovery_per_day", 0.002, "1/day", 0.0, 0.05, "R5.5", Low, "daily hazard the defender finds the intrusion"),
        figure!("cyber.effect_days", 2.5, "day", 0.1, 30.0, "P21", Medium, "effect window, 2–3 days for Kyivstar; weeks for Viasat"),
        figure!("cyber.degrade_multiplier", 0.3, "fraction", 0.0, 1.0, "default", Default, "throughput/quality left under a degrading (not outage) effect"),
    ]
}

/// Processes that cyber effects act on (owned by other domains).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetProcess {
    AdTrackSharing,
    LogisticsPicture,
    MobileNetwork,
    SatcomTerminals,
    AirRaidWarning,
    KeyCompromise,
    Custom(String),
}

/// What the effect does to its process.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectKind {
    /// The process stops (multiplier 0).
    Outage,
    /// The process runs at this fraction.
    Degrade { multiplier: f64 },
}

/// Campaign parameters from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CyberParams {
    pub build_days_mean: f64,
    pub build_days_sd: f64,
    pub build_days_min: f64,
    pub discovery_per_day: f64,
    pub effect_days: f64,
    pub degrade_multiplier: f64,
}

impl CyberParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            build_days_mean: f.get("cyber.build_days_mean"),
            build_days_sd: f.get("cyber.build_days_sd"),
            build_days_min: f.get("cyber.build_days_min"),
            discovery_per_day: f.get("cyber.discovery_per_day"),
            effect_days: f.get("cyber.effect_days"),
            degrade_multiplier: f.get("cyber.degrade_multiplier"),
        }
    }
}

/// Where a campaign stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CampaignState {
    Building { progress_days: f64 },
    Ready { since: SimTime },
    Active { start: SimTime, end: SimTime },
    Spent { at: SimTime },
    Discovered { at: SimTime },
}

/// What happened this step (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CyberEvent {
    AccessReady { campaign: u64, at: SimTime },
    Discovered { campaign: u64, at: SimTime, p_step: f64 },
    EffectEnded { campaign: u64, at: SimTime },
}

#[derive(Debug, Error, PartialEq)]
pub enum CyberError {
    #[error("campaign {0} is not ready to trigger")]
    NotReady(u64),
}

/// An access campaign.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccessCampaign {
    pub id: u64,
    pub target: TargetProcess,
    pub effect: EffectKind,
    pub started: SimTime,
    pub required_days: f64,
    pub state: CampaignState,
    last: SimTime,
}

/// A triggered effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CyberEffect {
    pub campaign: u64,
    pub target: TargetProcess,
    pub kind: EffectKind,
    pub start: SimTime,
    pub end: SimTime,
}

impl AccessCampaign {
    /// Start building access at `t`; the build time is drawn now.
    pub fn new(id: u64, target: TargetProcess, effect: EffectKind, t: SimTime, p: &CyberParams, rng: &mut impl Rng) -> Self {
        let required = if p.build_days_sd > 0.0 {
            Normal::new(p.build_days_mean, p.build_days_sd).map(|d| d.sample(rng)).unwrap_or(p.build_days_mean)
        } else {
            p.build_days_mean
        }
        .max(p.build_days_min);
        Self { id, target, effect, started: t, required_days: required, state: CampaignState::Building { progress_days: 0.0 }, last: t }
    }

    /// Advance to `t`: build access, risk discovery (a hazard, so step size
    /// doesn't matter), and end an active window.
    pub fn step(&mut self, t: SimTime, p: &CyberParams, rng: &mut impl Rng) -> Vec<CyberEvent> {
        let mut out = Vec::new();
        let dt_days = (t - self.last).secs().max(0.0) / 86_400.0;
        self.last = t;
        match self.state.clone() {
            CampaignState::Building { progress_days } => {
                // Discovery is checked against the time actually spent building.
                let remaining = self.required_days - progress_days;
                let exposed = dt_days.min(remaining.max(0.0));
                let ps = hazard_probability(p.discovery_per_day, exposed);
                if rng.gen::<f64>() < ps {
                    self.state = CampaignState::Discovered { at: t };
                    out.push(CyberEvent::Discovered { campaign: self.id, at: t, p_step: ps });
                } else if progress_days + dt_days >= self.required_days {
                    let ready_at = SimTime(t.0 - SimDuration::from_secs((progress_days + dt_days - self.required_days) * 86_400.0).0);
                    self.state = CampaignState::Ready { since: ready_at };
                    out.push(CyberEvent::AccessReady { campaign: self.id, at: ready_at });
                    // Exposure while waiting ready, for the rest of this step.
                    let rest = (t - ready_at).secs() / 86_400.0;
                    let pr = hazard_probability(p.discovery_per_day, rest);
                    if rng.gen::<f64>() < pr {
                        self.state = CampaignState::Discovered { at: t };
                        out.push(CyberEvent::Discovered { campaign: self.id, at: t, p_step: pr });
                    }
                } else {
                    self.state = CampaignState::Building { progress_days: progress_days + dt_days };
                }
            }
            CampaignState::Ready { .. } => {
                let ps = hazard_probability(p.discovery_per_day, dt_days);
                if rng.gen::<f64>() < ps {
                    self.state = CampaignState::Discovered { at: t };
                    out.push(CyberEvent::Discovered { campaign: self.id, at: t, p_step: ps });
                }
            }
            CampaignState::Active { end, .. } if t >= end => {
                self.state = CampaignState::Spent { at: end };
                out.push(CyberEvent::EffectEnded { campaign: self.id, at: end });
            }
            _ => {}
        }
        out
    }

    /// Fire the access at `t`: a one-shot effect window; the access burns.
    pub fn trigger(&mut self, t: SimTime, p: &CyberParams) -> Result<CyberEffect, CyberError> {
        if !matches!(self.state, CampaignState::Ready { .. }) {
            return Err(CyberError::NotReady(self.id));
        }
        let end = t + SimDuration::from_secs(p.effect_days * 86_400.0);
        self.state = CampaignState::Active { start: t, end };
        Ok(CyberEffect { campaign: self.id, target: self.target.clone(), kind: self.effect, start: t, end })
    }
}

/// Every effect in play; other domains ask it for their multiplier.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CyberEffects {
    pub effects: Vec<CyberEffect>,
}

impl CyberEffects {
    pub fn add(&mut self, e: CyberEffect) {
        self.effects.push(e);
    }

    /// Multiplier on a process at `t`: the product of active effects (0 for an outage).
    pub fn multiplier(&self, target: &TargetProcess, t: SimTime) -> f64 {
        self.effects
            .iter()
            .filter(|e| &e.target == target && e.start <= t && t < e.end)
            .map(|e| match e.kind {
                EffectKind::Outage => 0.0,
                EffectKind::Degrade { multiplier } => multiplier.clamp(0.0, 1.0),
            })
            .product()
    }

    pub fn is_active(&self, target: &TargetProcess, t: SimTime) -> bool {
        self.effects.iter().any(|e| &e.target == target && e.start <= t && t < e.end)
    }

    /// Whether keys are compromised at `t` (honoured by comms).
    pub fn keys_compromised(&self, t: SimTime) -> bool {
        self.is_active(&TargetProcess::KeyCompromise, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use bgws_core::{Seed, StreamKey};

    fn params() -> CyberParams {
        CyberParams::from_figures(&crate::default_figures())
    }

    fn day(d: f64) -> SimTime {
        SimTime::from_secs(d * 86_400.0)
    }

    #[test]
    fn campaign_builds_for_months_then_fires_once() {
        let p = params();
        let mut ready = Vec::new();
        let mut discovered = 0;
        let n = 2_000;
        for i in 0..n {
            let mut r = Seed(5).stream(StreamKey::new("cyber", i, 0, 0));
            let mut c = AccessCampaign::new(i, TargetProcess::MobileNetwork, EffectKind::Outage, SimTime::ZERO, &p, &mut r);
            for d in 1..=400 {
                c.step(day(d as f64), &p, &mut r);
                if !matches!(c.state, CampaignState::Building { .. }) {
                    break;
                }
            }
            match c.state {
                CampaignState::Ready { since } => ready.push(since.secs() / 86_400.0),
                CampaignState::Discovered { .. } => discovered += 1,
                _ => {}
            }
        }
        let mean = ready.iter().sum::<f64>() / ready.len() as f64;
        // 6–8 months [P21].
        assert!((180.0..240.0).contains(&mean), "{mean}");
        // Discovery before ready ≈ 1 − e^(−0.002·210) ≈ 34%.
        let frac = discovered as f64 / n as f64;
        assert!((frac - (1.0 - (-0.002f64 * 210.0).exp())).abs() < 0.05, "{frac}");
    }

    #[test]
    fn discovery_is_step_size_independent() {
        let p = CyberParams { build_days_mean: 1_000.0, build_days_sd: 0.0, ..params() };
        let run = |step: f64| {
            let mut found = 0;
            for i in 0..4_000u64 {
                let mut r = Seed(9).stream(StreamKey::new("cyber.step", i, step as u64, 0));
                let mut c = AccessCampaign::new(i, TargetProcess::LogisticsPicture, EffectKind::Outage, SimTime::ZERO, &p, &mut r);
                let mut t = 0.0;
                while t < 100.0 {
                    t += step;
                    c.step(day(t), &p, &mut r);
                }
                if matches!(c.state, CampaignState::Discovered { .. }) {
                    found += 1;
                }
            }
            found as f64 / 4_000.0
        };
        let expect = 1.0 - (-0.2f64).exp();
        assert!((run(1.0) - expect).abs() < 0.025);
        assert!((run(10.0) - expect).abs() < 0.025);
    }

    #[test]
    fn effects_are_time_limited_multipliers() {
        let p = params();
        let mut r = Seed(1).stream(StreamKey::new("t", 0, 0, 0));
        let mut c = AccessCampaign::new(1, TargetProcess::AdTrackSharing, EffectKind::Degrade { multiplier: 0.3 }, SimTime::ZERO, &CyberParams { discovery_per_day: 0.0, ..p.clone() }, &mut r);
        assert_eq!(c.trigger(day(1.0), &p), Err(CyberError::NotReady(1)));
        c.step(day(400.0), &CyberParams { discovery_per_day: 0.0, ..p.clone() }, &mut r);
        let e = c.trigger(day(400.0), &p).unwrap();
        assert_relative_eq!((e.end - e.start).secs() / 86_400.0, 2.5);
        let mut fx = CyberEffects::default();
        fx.add(e);
        fx.add(CyberEffect { campaign: 2, target: TargetProcess::KeyCompromise, kind: EffectKind::Outage, start: day(401.0), end: day(402.0) });
        assert_relative_eq!(fx.multiplier(&TargetProcess::AdTrackSharing, day(401.0)), 0.3);
        assert_relative_eq!(fx.multiplier(&TargetProcess::AdTrackSharing, day(403.0)), 1.0);
        assert_relative_eq!(fx.multiplier(&TargetProcess::MobileNetwork, day(401.0)), 1.0);
        assert!(fx.keys_compromised(day(401.5)) && !fx.keys_compromised(day(402.0)));
        // Burned after use.
        assert!(c.step(day(404.0), &p, &mut r).iter().any(|e| matches!(e, CyberEvent::EffectEnded { .. })));
        assert!(c.trigger(day(405.0), &p).is_err());
    }
}
