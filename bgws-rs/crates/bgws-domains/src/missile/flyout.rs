//! The three-degree-of-freedom tactical flyout (§8.1 Missiles and munitions,
//! §8.8 guided munitions): a point mass with thrust along the body (velocity)
//! axis, drag from a piecewise-linear Cd(Mach) table and the ISA density at
//! its height, gravity, and a lateral acceleration commanded by the guidance
//! law and capped by both the structure and the dynamic pressure
//! (`a_max = min(n_struct·g, q·S·CN_max / m)`), so a missile that is slow or
//! high cannot pull the g its guidance asks for.
//!
//! Guidance laws ([`Guidance`]):
//!
//! - **true proportional navigation**: `a = N'·Vc·(Ω × r̂)` with the
//!   line-of-sight rate `Ω = r × ṙ / |r|²`, plus gravity bias; midcourse on
//!   the launcher's track (with its error) until the seeker locks;
//! - **command to line of sight (CLOS)**: a second-order loop that nulls the
//!   missile's offset from the tracker–target line;
//! - **inertial plus GNSS cruise**: steer the *believed* position to the aim
//!   point, where belief = truth + a navigation bias + INS drift that grows
//!   while GNSS is denied (so CEP follows from the navigation state, §8.7);
//!   optional terminal seeker;
//! - **ballistic**: gravity and drag only.
//!
//! A terminal seeker ([`SeekerSpec`]) acquires inside its range and field of
//! view, one look per interval, with a caller-supplied detection probability
//! hook ([`SeekerDetection`]) so the sensing models (§8.2) can decide.
//!
//! Every sub-step computes the closest point of approach of the linear
//! relative motion ([`cpa_linear`]), so a 1,000 m/s missile never passes
//! through its target between samples.

use super::MissileParams;
use crate::atmosphere::{isa, G0};
use crate::vec3::{cpa_linear, V3};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// One rocket motor phase.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Motor {
    /// Average thrust, N.
    pub thrust: f64,
    /// Burn time, s.
    pub burn_time: f64,
    /// Propellant burned, kg.
    pub propellant: f64,
}

/// What kind of terminal seeker