//! bgws-core: the shared foundations of the Rust engine (docs/plan/RUST_ENGINE.md §3.1).
//!
//! - `time`: the master clock, integer microseconds.
//! - `geo`: WGS84 geodesy, ECEF, local ENU frames, fast local distances.
//! - `ids`: sides and interned entity keys.
//! - `rng`: keyed dice — every draw from a stream keyed by what it is for (§12).
//! - `figures`: the declared-figures registry (design rule 3).
//! - `log`: the typed event log and its fingerprint.
//! - `math`: normal distributions, hazards, decibels, small helpers.

pub mod figures;
pub mod geo;
pub mod ids;
pub mod log;
pub mod math;
pub mod rng;
pub mod time;

pub use figures::{Confidence, Figure, FigureReader, Figures};
pub use geo::{Geo, LatLon};
pub use ids::{Owner, Side};
pub use log::{Domain, Fingerprint, LogEvent};
pub use rng::{purpose, Dice};
pub use time::SimTime;
