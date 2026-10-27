//! Knowledge: tracks, Kalman filters, association, classification, fusion and
//! the side views (RUST_ENGINE.md §9, rule 1 of §2, the truth property of §17).
//!
//! The pipeline a side runs:
//!
//! 1. The simulation turns each sensor look into a [`Detection`] (a position
//!    fix, range-bearing or bearing with its covariance, class evidence from
//!    the confusion model, and the truth inside an opaque [`TruthTag`]).
//! 2. The side's [`TrackStore`] gates and assigns detections to Kalman-filtered
//!    tracks ([`filter`], [`assoc`]), starts, confirms, merges and drops them,
//!    and updates each track's class belief ([`classify`]).
//! 3. Units hold their own sightings and what they were told
//!    ([`AgentKnowledge`]); HQs fuse what reaches them by covariance
//!    intersection ([`Cop`], [`fusion`]).
//! 4. Deciders read a [`SideView`] built from those, never the store's
//!    [`AuditLog`] (umpire and AAR only).
//!
//! Deception needs no special path (§9.3): a spoofed detection is a detection
//! whose truth is `TruthTag::none()`, and a decoy is a target whose true class
//! is a decoy hypothesis — it is classified as what it imitates until a
//! good enough look.

pub mod assoc;
pub mod classify;
pub mod df;
pub mod filter;
pub mod fusion;
pub mod knowledge;
pub mod params;
pub mod stats;
pub mod store;
pub mod view;

pub use classify::{ClassBelief, ClassEvidence, ConfusionModel, Domain, Hypothesis, IdLevel};
pub use filter::{Kf, KfInit, Measurement, MotionModel, ProcessNoise, StateEstimate};
pub use fusion::{ci_estimates, covariance_intersection, naive_fusion, CiCriterion, CiResult};
pub use knowledge::{AgentKnowledge, Cop, CopEntry, CopOutcome, CopUpdate, Heard, KnowledgeOrigin, KnownTrack};
pub use params::{DomainParams, KnowParams};
pub use stats::Ellipse;
pub use store::{
    AssocOutcome, AssocRecord, AuditContribution, AuditLog, Detection, DropReason, MergeRecord, MotionHint, ScanReport,
    SensorKind, TrackAudit, TrackStatus, TrackStore, TrackView, TruthTag,
};
pub use view::{MessageView, OwnForceView, SideView, Viewer};

/// Every figure this crate declares (tracking, association, fusion, classification).
pub fn figures() -> Vec<bgws_core::Figure> {
    let mut v = params::figures();
    v.extend(classify::figures());
    v
}

#[cfg(test)]
mod tests;
