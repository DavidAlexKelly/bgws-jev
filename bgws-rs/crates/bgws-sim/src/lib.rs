//! The simulation (RUST_ENGINE.md §3–§6, §8–§10): an ECS world built from a
//! scenario and the catalogue, stepped by a multi-rate clock, with every
//! domain's systems, decision points through behaviour templates, and the
//! sans-IO API of §3.2.
//!
//! Systems are exclusive functions over the `bevy_ecs` world that iterate
//! entities in id order; their heavy per-pair work runs on `rayon` into
//! per-pair results applied in order, so a run is deterministic for a given
//! seed, build and machine whatever the thread count (§12).

/// A `tracing` span for the rest of the enclosing block, for `tracy` (§13),
/// only with the `profile` feature: without it this expands to nothing.
macro_rules! prof_span {
    ($name:expr) => {
        #[cfg(feature = "profile")]
        let _prof_span = tracing::info_span!($name).entered();
    };
}

pub mod clock;
pub mod components;
pub mod decide;
pub mod domains;
pub mod ground;
pub mod hq;
pub mod params;
pub mod queue;
pub mod sim;
pub mod systems;
pub mod view;
pub mod world;

pub use params::{all_figures, default_figures, Params};
pub use sim::{AnswerError, CommandError, LoadError, Outcome, SeatKind, Sim, SimOptions, Snapshot, Step};
pub use view::{FrameEntity, FrameTrack, FrameView, Viewer};

/// Send the per-system spans to a running Tracy profiler (§13, phase 0:
/// "`tracy` shows the tick"). Only with the `profile` feature.
#[cfg(feature = "profile")]
pub fn init_tracy() {
    use tracing_subscriber::layer::SubscriberExt;
    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(tracing_tracy::TracyLayer::default()));
}
