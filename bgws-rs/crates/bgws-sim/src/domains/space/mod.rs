//! Space and cyber (RUST_ENGINE.md §8.1 Space, §8.2 SAR/EO satellites,
//! §8.12; research R5.3–R5.5, P21): enabling layers the other domains feel.
//!
//! - [`orbit`]: satellites from their scenario placement or the catalogue's
//!   orbit (a circular-orbit TLE is synthesised and flown with SGP4 by
//!   `bgws_domains::space::Satellite`); access windows over each side's
//!   areas of interest scheduled as `Scheduled::SatellitePass` events; ISR
//!   tasking queues per side (`spaceTasking`), imagery served per pass with
//!   cloud blocking EO, identification by GSD against N₅₀
//!   (`bgws_models::sensing::imagery`), delivered to the side's tracks after
//!   downlink and analysis; GNSS geometry (DOP) and SATCOM beams for the
//!   other domains; the scripted counterspace and provider events.
//! - [`cyber`]: access campaigns stepped daily (`bgws_ops::cyber`), the
//!   `cyberWindowOpen` decision, effects as time-limited multipliers and
//!   outages on named processes that other domains query with
//!   [`cyber_multiplier`].

pub mod cyber;
pub mod orbit;

use crate::queue::Scheduled;
use crate::sim::{Sim, Trigger};
use bevy_ecs::prelude::Entity;
use bgws_behaviour::{Capability, FactValue, RulesChoice};
use bgws_core::{figure, EntityId, Figure, Side};
use std::collections::{BTreeMap, BTreeSet};

pub use cyber::{cyber_active, cyber_multiplier, cyber_multiplier_named, target_process, CyberState};
pub use orbit::{gnss_geometry, gnss_quality, imagery_available, missile_warning, satcom_capacity_bps, SpaceState};

/// The sim's own space and cyber figures (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.space.first_pass_s", 900.0, "s", 0.0, 86_400.0, "default", Default, "a satellite placed by the engine first passes over its area this long after the start"),
        figure!("sim.space.pass_spacing_s", 1_800.0, "s", 0.0, 43_200.0, "default", Default, "first passes of further placed satellites are spaced by this"),
        figure!("sim.space.isr_min_el_deg", 30.0, "deg", 5.0, 80.0, "default", Default, "least elevation for a useful image (look angle limit)"),
        figure!("sim.space.pass_step_s", 30.0, "s", 5.0, 300.0, "default", Default, "coarse scan step for access windows"),
        figure!("sim.space.horizon_h", 48.0, "h", 1.0, 240.0, "default", Default, "passes are scheduled this far ahead (and again when it runs out)"),
        figure!("sim.space.image_quality", 0.8, "fraction", 0.1, 1.0, "default", Default, "image MTF, look angle and analyst skill folded into one"),
        figure!("sim.space.fix_sigma_m", 30.0, "m", 1.0, 500.0, "default", Default, "1-sigma position error of an imagery detection"),
        figure!("sim.space.footprint_factor", 1.0, "fraction", 0.1, 5.0, "default", Default, "imaged radius as a share of half the swath"),
        figure!("sim.space.eo_skip_cloud", 0.85, "fraction", 0.0, 1.0, "default", Default, "an EO pass under more cloud than this is not worth tasking"),
        figure!("sim.space.gnss_alt_km", 20_200.0, "km", 19_000.0, 24_000.0, "default", Default, "GNSS constellation altitude (GPS-like)"),
        figure!("sim.space.gnss_planes", 6.0, "count", 3.0, 8.0, "default", Default, "GNSS orbital planes"),
        figure!("sim.space.gnss_per_plane", 4.0, "count", 2.0, 12.0, "default", Default, "GNSS satellites per plane"),
        figure!("sim.space.gnss_incl_deg", 55.0, "deg", 45.0, 65.0, "default", Default, "GNSS inclination"),
        figure!("sim.space.gnss_hdop_nominal", 1.2, "ratio", 0.5, 5.0, "default", Default, "HDOP assumed when space is abstracted"),
        figure!("sim.space.jam_gnss_quality", 0.3, "fraction", 0.0, 1.0, "R5.3", Low, "GNSS quality left under counterspace jamming when the event gives none"),
        figure!("sim.space.satcom_mil_bps", 2.0e6, "bit/s", 1.0e4, 1.0e9, "default", Default, "military SATCOM capacity in a theatre beam"),
        figure!("sim.space.satcom_leo_bps", 5.0e7, "bit/s", 1.0e5, 1.0e10, "R5.4", Default, "commercial LEO SATCOM capacity in the theatre"),
        figure!("sim.space.beam_radius_km", 800.0, "km", 50.0, 5_000.0, "default", Default, "radius of a theatre SATCOM beam"),
        figure!("sim.cyber.ask_s", 600.0, "s", 60.0, 7_200.0, "default", Default, "least time between two questions about the same access"),
        figure!("sim.cyber.remediated_share", 0.5, "fraction", 0.0, 1.0, "R5.5", Low, "share of the effect left when the victim detects the intrusion"),
        figure!("sim.cyber.step_s", 86_400.0, "s", 3_600.0, 604_800.0, "P21", Default, "access campaigns are stepped this often"),
    ]
}

pub fn setup(s: &mut Sim) {
    orbit::setup(s);
    cyber::setup(s);
}
pub fn fast_active(_s: &Sim) -> bool {
    false
}
pub fn air_active(_s: &Sim) -> bool {
    false
}
pub fn fast(_s: &mut Sim, _dt: f64) {}
pub fn base(_s: &mut Sim, _dt: f64) {}
pub fn slow(_s: &mut Sim, _dt: f64) {}
pub fn minute(s: &mut Sim, _dt: f64) {
    orbit::minute(s);
    cyber::minute(s);
}
pub fn script_event(s: &mut Sim, e: &bgws_scenario::v4::ScriptedEvent) {
    orbit::script_event(s, e);
}

/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    match ev {
        Scheduled::SatellitePass { satellite, rising } => {
            orbit::pass(s, satellite, rising);
            None
        }
        Scheduled::CyberWindow { campaign, open } => {
            cyber::window(s, campaign, open);
            None
        }
        Scheduled::Timer { ref system, key } if system == "space.deliver" => {
            orbit::deliver(s, key);
            None
        }
        Scheduled::Timer { ref system, key } if system == "space.passes" => {
            orbit::schedule_passes(s, key as f64);
            None
        }
        Scheduled::Timer { ref system, key } if system == "cyber.step" => {
            cyber::step(s, key);
            None
        }
        other => Some(other),
    }
}

pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    orbit::facts(s, e, m);
}
pub fn capabilities(_s: &Sim, _e: Entity, _c: &mut BTreeSet<Capability>) {}

pub fn apply_action(s: &mut Sim, e: Entity, id: EntityId, action: &str, _params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    if action.starts_with("space.") {
        orbit::apply(s, e, id, action, about)
    } else if action.starts_with("cyber.") {
        cyber::apply(s, e, id, action)
    } else {
        false
    }
}
pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}

/// Ask a world question with the domain's rules choice; if no template
/// answers it, carry out the rule's action now. Returns whether one was asked.
pub(crate) fn ask_or_do(s: &mut Sim, id: EntityId, point: &'static str, option: &str, action: &str) -> bool {
    let e = s.ctx.entity(id);
    if !crate::systems::alive(s, e) {
        return false;
    }
    let deciding = s.world.get::<crate::components::Act>(e).map(|a| a.deciding).unwrap_or(false);
    if deciding {
        return false;
    }
    if crate::decide::ask_with_rule(s, &Trigger { entity: id, point, about: None }, RulesChoice::Option(option.to_string())) {
        return true;
    }
    crate::world::apply_action(s, e, id, action, &BTreeMap::new(), None);
    false
}
