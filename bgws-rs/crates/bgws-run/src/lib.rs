//! The host side of the engine (§3.3): the loop that advances a `Sim`,
//! gathers its questions, sends them to the deciders (Jev, Claude, a local
//! model, a person or the rules) and hands the answers back; the game
//! record; and the experiments that run the simulation for the analysis
//! crate (batches, vignettes, sensitivity).

pub mod aar;
pub mod experiments;
pub mod geotiff;
pub mod host;
pub mod behaviour;
pub mod workbench;

pub use host::{load_scenario, router_for, DeciderChoice, Host, HostOptions, PlayReport};

/// Every figure the whole engine declares: the sim's crates plus the
/// deciders and the analysis settings.
pub fn all_figures() -> Vec<bgws_core::Figure> {
    let mut v = bgws_sim::all_figures();
    v.extend(bgws_decide::figures());
    v.extend(bgws_analysis::figures());
    v
}

/// The default registry for a host.
pub fn default_figures() -> bgws_core::Figures {
    bgws_core::Figures::from_list(all_figures()).expect("the shipped figures are consistent")
}
