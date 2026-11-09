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
pub mod editor;
pub mod eventlog;
pub mod workbench;

pub use host::{load_scenario, router_for, DeciderChoice, Host, HostOptions, PlayReport};

/// Where theatre packs live: `$BGWS_PACKS`, else `packs/` at the repository root.
pub fn packs_dir() -> std::path::PathBuf {
    std::env::var_os("BGWS_PACKS").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs"))
}

/// A scenario's `theatre.pack`: a path, or a folder name under `packs_dir()`.
pub fn resolve_pack(name: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(name);
    if p.is_absolute() || p.exists() {
        p
    } else {
        packs_dir().join(name)
    }
}

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
