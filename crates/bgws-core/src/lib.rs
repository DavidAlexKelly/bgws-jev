//! The bottom of the BGWS Rust engine (RUST_ENGINE.md §3.1): identifiers,
//! simulated time, units of measure, WGS84 geodesy, the figures registry,
//! keyed random streams, the event log with its fingerprint, and the types
//! that cross the sans-IO boundary between the simulation and its deciders.
//!
//! Nothing here touches the network, the disk or a clock (rule 7).

pub mod decision;
pub mod events;
pub mod figures;
pub mod geo;
pub mod hazard;
pub mod ids;
pub mod rng;
pub mod time;
pub mod units;

pub use decision::{Answer, AnswerSource, DecisionOption, Question, QuestionId, QuestionKind};
pub use events::{EventLog, Fingerprint, LogEvent};
pub use figures::{Confidence, Figure, FigureError, Figures};
pub use geo::{Ecef, Enu, EnuFrame, Geodetic};
pub use hazard::{hazard_probability, rate_from_probability};
pub use ids::{EntityId, HqId, Interner, MessageId, SeatId, Side, TrackId};
pub use rng::{Seed, StreamKey};
pub use time::{SimDuration, SimTime};
pub use units::{Decibels, Hertz, Metres, MetresPerSecond, Radians, Seconds, Watts};
