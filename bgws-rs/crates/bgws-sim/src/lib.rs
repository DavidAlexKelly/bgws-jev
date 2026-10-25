//! The simulation (RUST_ENGINE.md §3–§6, §8–§10): an ECS world built from a
//! scenario and the catalogue, stepped by a multi-rate clock, with every
//! domain's systems, decision points through behaviour templates, and the
//! sans-IO API of §3.2.
//!
//! Systems are exclusive functions over the `bevy_ecs` world that iterate
//! entities in id order; their heavy per-pair work runs on `rayon` into
//! per-pair results applied in order, so a run is deterministic for a given
//! seed, build and machine whatever the thread count (§12).

pub mod clock;
pub mod components;
pub mod decide;
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
