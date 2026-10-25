//! Physical and human models for land combat (RUST_ENGINE.md §8.1 ground,
//! §8.2 sensing, §8.3 signatures, §8.8 weapons and terminal effects, §8.9
//! engineers, §8.10 people).
//!
//! Every model here is a pure function or a small state struct with a pure
//! step. Nothing reads the world, the disk or a clock; random draws take a
//! caller's keyed stream (`&mut impl rand::Rng`, §12). Every number a model
//! uses is a declared figure (rule 3): each module has a `figures()` and the
//! crate's [`figures`] is their union. Models take `&Figures`, usually through
//! a small `…Params::from_figures` struct built once per run.
//!
//! Inputs that other crates compute (terrain cells, transmission, path and
//! transmission loss, platform records) are taken as plain parameters so this
//! crate depends only on `bgws-core`; the integrator adapts.

pub mod direct_fire;
pub mod engineers;
pub mod ground;
pub mod guided;
pub mod indirect_fire;
pub mod math;
pub mod people;
pub mod sensing;
pub mod signature;

use bgws_core::Figure;
use thiserror::Error;

/// Errors a model can return for inputs it cannot work with.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ModelError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("no solution: {0}")]
    NoSolution(String),
}

/// Every figure this crate declares: the union of each module's.
pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    v.extend(ground::figures());
    v.extend(sensing::figures());
    v.extend(signature::figures());
    v.extend(direct_fire::figures());
    v.extend(indirect_fire::figures());
    v.extend(guided::figures());
    v.extend(engineers::figures());
    v.extend(people::figures());
    v
}

#[cfg(test)]
pub(crate) mod testutil {
    use bgws_core::Figures;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// The crate's default registry.
    pub fn figs() -> Figures {
        Figures::from_list(super::figures()).expect("figures register")
    }

    pub fn rng(n: u64) -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_register_and_are_sourced() {
        let f = bgws_core::Figures::from_list(figures()).expect("no duplicates, all in range");
        assert!(f.len() > 100);
        for fig in f.iter() {
            assert!(!fig.source.is_empty(), "{} has no source", fig.key);
            assert!(!fig.unit.is_empty(), "{} has no unit", fig.key);
            assert!(fig.min <= fig.max, "{}", fig.key);
            let domain = fig.key.split('.').next().unwrap_or("");
            assert!(
                ["ground", "sensing", "signature", "direct", "indirect", "guided", "engineers", "people"].contains(&domain),
                "{} has an unexpected prefix",
                fig.key
            );
        }
    }
}
