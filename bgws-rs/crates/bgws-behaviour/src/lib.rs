//! Behaviour templates, inheritance, the graph walk, facts, the action library
//! and the hard gates (RUST_ENGINE.md §3.1, §10.1, §10.6, §19, §20;
//! DECISION_FLOW_GRAPHS.md).
//!
//! A faithful port of the TypeScript `realtime/behaviour/` with the same JSON:
//! the shipped templates in `bgws-rs/data/templates` load unchanged. On top of
//! the TypeScript walker it adds the §10.1 hard gates (ROE, weapon release
//! authority, EMCON, capability, cohesion) applied before options are offered,
//! an action compatibility table for engines that lack some actions (§20),
//! the §10.6 safeguards, and [`question::build_question`], which turns a walk
//! into a `bgws_core::Question` for the deciders.
//!
//! Everything here is pure: no network, no clock, no randomness. The walker
//! reads facts through the [`facts::Facts`] trait, which the sim fills from a
//! `SideView` (rule 1), never from the world's truth.
//!
//! Typical use by the sim:
//! ```
//! use bgws_behaviour::{facts::NoFacts, gates::Gates, registry::Registry};
//! let reg = Registry::builtin().unwrap();
//! let (template, walk) = reg.decide(&["core/armour"], "D1", &NoFacts, &Gates::open()).unwrap().unwrap();
//! assert_eq!(template.id, "core/armour");
//! assert!(walk.option_ids().contains(&"pullBack"));
//! ```

pub mod archetypes;
pub mod bench;
pub mod facts;
pub mod gates;
pub mod library;
pub mod load;
pub mod patch;
pub mod question;
pub mod registry;
pub mod safeguards;
pub mod schema;
pub mod shared;
pub mod store;
pub mod types;
pub mod vocabulary;
pub mod walk;

pub use facts::{FactInfo, Facts, NoFacts};
pub use gates::{Cohesion, Emcon, Gates, Roe, WeaponRelease};
pub use library::{ActionInfo, ActionTraits, Capability};
pub use load::{build_registry, builtin_files, ActionCompatibility, LoadError, LoadMode};
pub use question::{build_question, needs_asking, QuestionContext, QuestionLimits, RulesChoice};
pub use registry::Registry;
pub use types::{Condition, DropReason, FactValue, GraphNode, ResolvedTemplate, Template, WalkResult};
pub use vocabulary::Vocabulary;
pub use walk::{evaluate, resolve_template, walk, walk_gated, Masks, ResolveError, WalkError};

use bgws_core::{figure, Figure};

/// Every figure this crate's models read (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![figure!(
        "behaviour.safeguards.max_options",
        16.0,
        "count",
        2.0,
        255.0,
        "R8",
        Low,
        "Most options offered in one decision; bounded option lists are an R8 safeguard (Jev's own hard limit is 255)."
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Figures;

    #[test]
    fn figures_register() {
        let f = Figures::from_list(figures()).unwrap();
        assert_eq!(QuestionLimits::from_figures(&f).max_options, 16);
        assert!(f.unused().is_empty());
    }
}
