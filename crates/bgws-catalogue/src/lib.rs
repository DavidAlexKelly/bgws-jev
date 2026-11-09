//! Platform records (RUST_ENGINE.md §5.1, §14.1, §19 "port and extend
//! realtime/catalogue/"): movers, bodies, signatures, sensors, weapon mounts
//! and munitions, radios, emitters, stores, carried loads and crew defaults;
//! class defaults so a record can be written with a few numbers; loading
//! from `data/catalogue/*.json`; validation; and the archetype table.
//!
//! Everything here is synthetic and open-source-anchored (§1.2, §14.1):
//! each record says how its guesses were made (`rationale`, `source`), and
//! class defaults are declared figures (`catalogue.class.*`) that a run can
//! override within range.
//!
//! ```no_run
//! let cat = bgws_catalogue::Catalogue::builtin().expect("shipped data loads");
//! assert!(cat.validate().is_empty());
//! let mbt = cat.platform("generic/mbt").unwrap();
//! let gun = cat.munition_of("generic/mbt", "tankGun").unwrap();
//! let margin = gun.penetration_margin(2000.0, mbt.body.armour.ke.front);
//! # let _ = margin;
//! ```

pub mod archetypes;
pub mod catalogue;
pub mod derive;
pub mod json;
pub mod munition;
pub mod types;
pub mod units;
pub mod validate;

pub use archetypes::{archetype_for, archetype_of_snapshot, default_template_for_class, SnapshotHint, DEFAULT_TEMPLATE_ID, TS_ARCHETYPES, TS_PLATFORM_KEYS};
pub use catalogue::{builtin_class_figures, Catalogue, BUILTIN_FILES};
pub use derive::{radar_power_for_range, radar_range_for, DeriveParams};
pub use munition::{Carleton, FlightProfile, Guidance, Munition, MunitionKind, PenPoint, Seeker, TargetDomain};
pub use types::*;
pub use validate::{CatalogueError, CatalogueErrors};

/// Every figure this crate declares: the derivation ratios and every class
/// default (`catalogue.class.<class>.<path>`).
pub fn figures() -> Vec<bgws_core::Figure> {
    let mut v = derive::figures();
    v.extend(builtin_class_figures());
    v
}
