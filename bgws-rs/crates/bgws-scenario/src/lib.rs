//! Scenario, scope and game-record formats (RUST_ENGINE.md §14.3, §19 "port
//! scope.ts, scenario.ts, presets, plus a v3 → v4 converter").
//!
//! - `scope`: the scenario scope (types, domains simulated/abstracted/off,
//!   echelons, planners, detail, time) and what the engine can run.
//! - `v3`: the TypeScript scenario file (versions 1–3), parsed and checked
//!   as the TS did.
//! - `v4`: scenario version 4 — forces with stable ids at WGS84 positions,
//!   the theatre, UTC start, weather source, emitter (EMCON) settings and
//!   deciders per side.
//! - `convert`: v3 → v4, recovering authored metres and placing them through
//!   an ENU frame (the assumption is documented there).
//! - `validate`: loading any version, never panicking, and checking a
//!   scenario against the catalogue with typed errors.
//! - `record`: the game record — scenario, seed, build, machine, figure
//!   overrides, every command and answer with its time, snapshots.
//! - `time`: UTC date-times without a clock.
//!
//! Behaviour templates are carried as opaque JSON (`bgws-behaviour` reads
//! them); template ids are checked, the template graphs are not.

pub mod convert;
pub mod record;
pub mod scope;
pub mod time;
pub mod v3;
pub mod v4;
pub mod validate;

pub use convert::{convert_v3, ConvertError, ConvertOptions, Placement, TS_DEFAULT_ORIGIN};
pub use record::{BuildInfo, CommandKind, GameRecord, MachineInfo, RecordError, RecordedAnswer, RecordedCommand, RecordedSnapshot, RunSetup, SnapshotData};
pub use scope::{effective_scope, playable, scope_errors, scope_for, type_info, DomainMode, ScenarioType, Scope, ScopeCheck, ScopeDomain, ScopeError, ScopeTime, Weather};
pub use v3::{parse_v3, ScenarioV3};
pub use v4::*;
pub use validate::{load_any, load_path, load_value, validate, LoadError, ScenarioError};

/// The shipped v4 scenarios (`data/scenarios/`), embedded: (file name, JSON).
pub const BUILTIN_SCENARIOS: &[(&str, &str)] = &[
    ("m2-mech-company-hasty-attack.json", include_str!("../../../data/scenarios/m2-mech-company-hasty-attack.json")),
    ("m3-battalion-attack.json", include_str!("../../../data/scenarios/m3-battalion-attack.json")),
    ("m4-drone-line.json", include_str!("../../../data/scenarios/m4-drone-line.json")),
    ("m5-air-over-the-battle.json", include_str!("../../../data/scenarios/m5-air-over-the-battle.json")),
    ("m6-sustained-offensive.json", include_str!("../../../data/scenarios/m6-sustained-offensive.json")),
    ("m7-littoral-holding-the-strait.json", include_str!("../../../data/scenarios/m7-littoral-holding-the-strait.json")),
    ("m8-joint-strike.json", include_str!("../../../data/scenarios/m8-joint-strike.json")),
    ("skirmish-advance-to-contact-v1.json", include_str!("../../../data/scenarios/skirmish-advance-to-contact-v1.json")),
    ("skirmish-catalogue-armour-v1.json", include_str!("../../../data/scenarios/skirmish-catalogue-armour-v1.json")),
    ("skirmish-combined-arms-v1.json", include_str!("../../../data/scenarios/skirmish-combined-arms-v1.json")),
    ("skirmish-generation-probe-v1.json", include_str!("../../../data/scenarios/skirmish-generation-probe-v1.json")),
    ("skirmish-meeting-engagement-v1.json", include_str!("../../../data/scenarios/skirmish-meeting-engagement-v1.json")),
    ("skirmish-quality-probe-v1.json", include_str!("../../../data/scenarios/skirmish-quality-probe-v1.json")),
    ("skirmish-symmetric-control-v1.json", include_str!("../../../data/scenarios/skirmish-symmetric-control-v1.json")),
];

/// The shipped scenarios, parsed.
pub fn builtin_scenarios() -> Result<Vec<ScenarioV4>, LoadError> {
    BUILTIN_SCENARIOS.iter().map(|(_, t)| ScenarioV4::from_json(t)).collect()
}

/// This crate declares no model figures: formats only.
pub fn figures() -> Vec<bgws_core::Figure> {
    Vec::new()
}
