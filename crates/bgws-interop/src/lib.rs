//! Interoperability with other simulations (docs/design/INTEROP_PLAN.md).
//!
//! Phase 1: the DIS codes are in the catalogue; this crate publishes the
//! sim's truth as DIS Entity State PDUs (IEEE 1278.1-2012, DIS v7) over
//! UDP, with dead reckoning, and converts heights to the WGS-84 ellipsoid
//! at the boundary.

pub mod geo;
pub mod geoid;
pub mod model;
pub mod dr;
pub mod dis;
pub mod events;
pub mod import;
pub mod link;
