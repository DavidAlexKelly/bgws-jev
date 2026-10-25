//! Physical and process models for the joint domains (RUST_ENGINE.md §8.1
//! Air / Missiles and munitions / Sea / Space, §8.6, §8.8 air defence and
//! guided munitions, §8.12 space):
//!
//! - [`air`]: point-mass aircraft, steering, drones, fibre tethers, sortie generation;
//! - [`missile`]: munitions in flight on kinematic profiles, radar-horizon exposure, CEP at the aim point;
//! - [`air_defence`]: kill chains, layered defence with magazines, weapon-target assignment, cost exchange;
//! - [`naval`]: ships, submarines, small craft, salvo defence, damage control, mines, ports and replenishment;
//! - [`space`]: SGP4 satellites, ground tracks, access windows, footprints, manoeuvres, imagery, GNSS, satcom.
//!
//! Shared physics lives in [`vec3`], [`atmosphere`] and [`orbit`]. Every
//! tunable number is a declared figure (rule 3); [`figures`] returns them all.
//! Models are pure: random draws take a caller's keyed stream (§12).

pub mod air;
pub mod air_defence;
pub mod atmosphere;
pub mod missile;
pub mod naval;
pub mod orbit;
pub mod space;
pub mod vec3;

pub use vec3::V3;

use bgws_core::Figure;
use thiserror::Error;

/// Errors from the domain models (bad inputs, not model outcomes).
#[derive(Debug, Error, PartialEq)]
pub enum DomainError {
    #[error("orbital elements could not be parsed: {0}")]
    Elements(String),
    #[error("SGP4 propagation failed: {0}")]
    Propagation(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// Every figure this crate declares, keyed by domain (`air.`, `missile.`,
/// `ad.`, `naval.`, `space.`).
pub fn figures() -> Vec<Figure> {
    let mut v = air::figures();
    v.extend(missile::figures());
    v.extend(air_defence::figures());
    v.extend(naval::figures());
    v.extend(space::figures());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Figures;

    #[test]
    fn figures_register_cleanly() {
        let list = figures();
        let n = list.len();
        let f = Figures::from_list(list).expect("no duplicates, all in range");
        assert_eq!(f.len(), n);
        for fig in f.iter() {
            assert!(!fig.source.is_empty() && !fig.note.is_empty(), "{}", fig.key);
            assert!(["air.", "missile.", "ad.", "naval.", "space."].iter().any(|p| fig.key.starts_with(p)), "{}", fig.key);
        }
    }
}
