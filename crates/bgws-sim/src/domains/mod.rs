//! The joint domains (§8), one module each, called in a fixed order by
//! `crate::world`:
//!
//! - [`fires`]: indirect fire, counter-battery, engineers and obstacles, unit
//!   stores, logistics, maintenance and medical (§8.8, §8.9, §8.11);
//! - [`air`]: aircraft, sorties, airspace, air defence, munitions in flight
//!   (§8.1, §8.8);
//! - [`drones`]: drones and counter-drone, EW, PNT, comms nets, HQs and C2
//!   (§8.5, §8.7, §8.12);
//! - [`naval`]: ships, submarines, sonar, salvo defence, mines, ports (§8.6);
//! - [`space`]: satellites, imagery, GNSS, satcom, cyber (§8.12);
//! - [`lod`]: level of detail, aggregation, quiet skipping and the world's
//!   victory conditions (§6.3, §6.4, §8.13).

pub mod air;
pub mod drones;
pub mod fires;
pub mod lod;
pub mod naval;
pub mod space;
