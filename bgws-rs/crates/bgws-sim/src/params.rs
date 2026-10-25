//! Every figure the engine runs on (rule 3): the union of every crate's
//! registry plus the sim's own, and the parameter bundles the systems use,
//! built once per run from the figures.

use bgws_core::{figure, Figure, Figures};
use bgws_know::KnowParams;
use bgws_models::direct_fire::DirectFireParams;
use bgws_models::ground::GroundParams;
use bgws_models::people::PeopleParams;
use bgws_models::sensing::acquire::AcquireParams;

/// The sim's own figures: thresholds and intervals of its systems.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.posture.settle_s", 60.0, "s", 10.0, 300.0, "default", Default, "time halted before a unit counts as settled into its position"),
        figure!("sim.posture.hulldown_s", 180.0, "s", 30.0, 900.0, "default", Default, "time settled in cover before it counts as hull-down"),
        figure!("sim.sense.search_fov_deg", 60.0, "deg", 10.0, 180.0, "default", Default, "sector a unit searches without a cue"),
        figure!("sim.sense.max_range_m", 6_000.0, "m", 1_000.0, 20_000.0, "default", Default, "longest range a ground observer searches"),
        figure!("sim.sense.forget_s", 300.0, "s", 30.0, 1_800.0, "default", Default, "a sighting not refreshed this long is dropped from the unit's own picture"),
        figure!("sim.sense.position_sigma_m", 30.0, "m", 5.0, 200.0, "default", Default, "1-sigma position error of a visual fix"),
        figure!("sim.light.eye_night_factor", 0.15, "fraction", 0.02, 0.6, "default", Default, "naked-eye resolution factor under 1 lux"),
        figure!("sim.light.night_lux", 1.0, "lux", 0.01, 50.0, "default", Default, "illuminance below which eyes and day optics degrade"),
        figure!("sim.fire.lay_min_s", 6.0, "s", 2.0, 30.0, "default", Default, "least time to lay on a new target and fire the first round"),
        figure!("sim.fire.rounds_per_volley", 1.0, "count", 1.0, 4.0, "default", Default, "main-gun rounds per vehicle per volley"),
        figure!("sim.fire.near_miss_per_round", 1.0, "count", 0.2, 3.0, "default", Default, "near misses felt per round that misses close"),
        figure!("sim.fire.ineffective_volleys", 4.0, "count", 2.0, 10.0, "default", Default, "volleys with no hit before a unit asks whether its fire is working (D8)"),
        figure!("sim.decide.cooldown_s", 20.0, "s", 0.0, 120.0, "default", Default, "least time between two questions to the same unit"),
        figure!("sim.decide.idle_s", 600.0, "s", 60.0, 3_600.0, "default", Default, "a unit with nothing happening for this long asks what next (D10)"),
        figure!("sim.move.arrive_m", 25.0, "m", 5.0, 200.0, "default", Default, "distance at which a waypoint counts as reached"),
        figure!("sim.move.withdraw_m", 800.0, "m", 200.0, 3_000.0, "default", Default, "how far a unit pulls back"),
        figure!("sim.move.cover_search_m", 400.0, "m", 100.0, 1_500.0, "default", Default, "radius searched for a better position"),
        figure!("sim.comms.report_delay_s", 30.0, "s", 0.0, 300.0, "default", Default, "contact report delay to the side picture when no nets are modelled"),
        figure!("sim.morale.isolation_m", 1_500.0, "m", 300.0, 5_000.0, "default", Default, "no friend within this range counts as isolated"),
        figure!("sim.lod.aggregate_m", 15_000.0, "m", 3_000.0, 60_000.0, "default", Default, "a formation this far from any enemy track is aggregated"),
        figure!("sim.quiet.max_jump_s", 300.0, "s", 10.0, 3_600.0, "default", Default, "longest quiet skip in one jump"),
    ]
}

/// Every figure the engine declares, across all crates.
pub fn all_figures() -> Vec<Figure> {
    let mut v = figures();
    v.extend(crate::domains::fires::figures());
    v.extend(bgws_catalogue::figures());
    v.extend(bgws_env::figures());
    v.extend(bgws_prop::figures());
    v.extend(bgws_gpu::figures());
    v.extend(bgws_know::figures());
    v.extend(bgws_behaviour::figures());
    v.extend(bgws_scenario::figures());
    v.extend(bgws_models::figures());
    v.extend(bgws_domains::figures());
    v.extend(bgws_ops::figures());
    v
}

/// The default registry.
pub fn default_figures() -> Figures {
    Figures::from_list(all_figures()).expect("the shipped figures are consistent")
}

/// The parameter bundles, built once per run.
#[derive(Clone, Debug)]
pub struct Params {
    pub ground: GroundParams,
    pub acquire: AcquireParams,
    pub direct: DirectFireParams,
    pub people: PeopleParams,
    pub know: KnowParams,
    pub observer: bgws_models::sensing::observer::ObserverParams,
    pub atmos: bgws_env::AtmosParams,
    pub light: bgws_env::LightParams,
    pub settle_s: f64,
    pub hulldown_s: f64,
    pub search_fov_deg: f64,
    pub sense_max_m: f64,
    pub forget_s: f64,
    pub fix_sigma_m: f64,
    pub eye_night_factor: f64,
    pub night_lux: f64,
    pub lay_min_s: f64,
    pub rounds_per_volley: f64,
    pub near_miss_per_round: f64,
    pub ineffective_volleys: u32,
    pub cooldown_s: f64,
    pub idle_s: f64,
    pub arrive_m: f64,
    pub withdraw_m: f64,
    pub cover_search_m: f64,
    pub report_delay_s: f64,
    pub isolation_m: f64,
    pub aggregate_m: f64,
    pub quiet_max_jump_s: f64,
}

impl Params {
    pub fn from_figures(f: &Figures) -> Self {
        Params {
            ground: GroundParams::from_figures(f),
            acquire: AcquireParams::from_figures(f),
            direct: DirectFireParams::from_figures(f),
            people: PeopleParams::from_figures(f),
            know: KnowParams::from_figures(f),
            observer: bgws_models::sensing::observer::ObserverParams::from_figures(f),
            atmos: bgws_env::AtmosParams::from_figures(f),
            light: bgws_env::LightParams::from_figures(f),
            settle_s: f.get("sim.posture.settle_s"),
            hulldown_s: f.get("sim.posture.hulldown_s"),
            search_fov_deg: f.get("sim.sense.search_fov_deg"),
            sense_max_m: f.get("sim.sense.max_range_m"),
            forget_s: f.get("sim.sense.forget_s"),
            fix_sigma_m: f.get("sim.sense.position_sigma_m"),
            eye_night_factor: f.get("sim.light.eye_night_factor"),
            night_lux: f.get("sim.light.night_lux"),
            lay_min_s: f.get("sim.fire.lay_min_s"),
            rounds_per_volley: f.get("sim.fire.rounds_per_volley"),
            near_miss_per_round: f.get("sim.fire.near_miss_per_round"),
            ineffective_volleys: f.get("sim.fire.ineffective_volleys") as u32,
            cooldown_s: f.get("sim.decide.cooldown_s"),
            idle_s: f.get("sim.decide.idle_s"),
            arrive_m: f.get("sim.move.arrive_m"),
            withdraw_m: f.get("sim.move.withdraw_m"),
            cover_search_m: f.get("sim.move.cover_search_m"),
            report_delay_s: f.get("sim.comms.report_delay_s"),
            isolation_m: f.get("sim.morale.isolation_m"),
            aggregate_m: f.get("sim.lod.aggregate_m"),
            quiet_max_jump_s: f.get("sim.quiet.max_jump_s"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_crate_registers_cleanly_together() {
        let f = super::default_figures();
        assert!(f.len() > 1000, "{}", f.len());
        let _ = super::Params::from_figures(&f);
    }
}
