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
        figure!("sim.fire.team_front_m", 50.0, "m", 10.0, 200.0, "default", Default, "frontage of a dismounted section or team on the ground (8 men at 5-10 m spacing): a burst catches the share of them inside its lethal area"),
        figure!("sim.fire.team_depth_m", 20.0, "m", 5.0, 100.0, "default", Default, "depth of a dismounted section or team on the ground"),
        figure!("sim.fire.suppress_memory_s", 60.0, "s", 0.0, 600.0, "default", Default, "a unit fires to suppress where it last saw an enemy for this long after losing sight of it (0: never)"),
        figure!("sim.fire.suppress_aim_m", 15.0, "m", 2.0, 100.0, "default", Default, "least spread (1σ) of fire at a remembered spot: the spot is known only roughly"),
        figure!("sim.fire.suppress_drift_mps", 3.0, "m/s", 0.0, 15.0, "default", Default, "how fast a spot seen moving grows uncertain"),
        figure!("sim.fire.suppress_max_cost_usd", 500.0, "USD", 0.0, 100_000.0, "default", Default, "fire to suppress uses only rounds this cheap or cheaper (machine-gun bullets, cannon HE), not main-gun rounds or missiles"),
        figure!("sim.dismount.contact_m", 500.0, "m", 0.0, 3000.0, "default", Default, "an IFV's infantry get out when an enemy is seen this close"),
        figure!("sim.dismount.objective_m", 400.0, "m", 0.0, 3000.0, "default", Default, "an IFV's infantry get out this far short of where its move ends (the assault goes in on foot)"),
        figure!("sim.dismount.time_s", 30.0, "s", 5.0, 300.0, "default", Default, "time from a halt to the infantry out and ready (the dismount drill)"),
        figure!("sim.fire.support_m", 1500.0, "m", 0.0, 5000.0, "default", Default, "a halted unit fires to suppress a reported enemy position while a friend this close to it is on the move (support by fire)"),
        figure!("sim.fire.support_error_m", 200.0, "m", 10.0, 1000.0, "default", Default, "a reported enemy position is fired on only if the picture places it this well (1σ)"),
        figure!("sim.fire.danger_close_m", 150.0, "m", 25.0, 600.0, "default", Default, "no fire to suppress lands this close to a friend (or within twice the fire's spread): fire lifts and shifts"),
        figure!("sim.fire.missile_keep", 0.7, "ratio", 0.0, 1.0, "default", Default, "a gun this nearly as likely to knock the target out as a missile is used instead (missiles are few)"),
        figure!("sim.fire.near_miss_per_round", 1.0, "count", 0.2, 3.0, "default", Default, "near misses felt per round that misses close"),
        figure!("sim.fire.ineffective_volleys", 4.0, "count", 2.0, 10.0, "default", Default, "volleys with no hit before a unit asks whether its fire is working (D8)"),
        figure!("sim.decide.cooldown_s", 20.0, "s", 0.0, 120.0, "default", Default, "least time between two questions to the same unit"),
        figure!("sim.decide.idle_s", 600.0, "s", 60.0, 3_600.0, "default", Default, "a unit with nothing happening for this long asks what next (D10)"),
        figure!("sim.move.arrive_m", 25.0, "m", 5.0, 200.0, "default", Default, "distance at which a waypoint counts as reached"),
        figure!("sim.move.withdraw_m", 800.0, "m", 200.0, 3_000.0, "default", Default, "how far a unit pulls back"),
        figure!("sim.exec.reposition_m", 250.0, "m", 50.0, 1_000.0, "default", Low, "farthest a unit moves to a better firing position against a threat it knows of"),
        figure!("sim.exec.occupy_s", 30.0, "s", 5.0, 180.0, "default", Low, "time to settle hull-down in a firing position it chose against the threat"),
        figure!("sim.exec.bound_m", 250.0, "m", 50.0, 1_000.0, "default", Low, "length of one bound when bounding"),
        figure!("sim.exec.bound_halt_s", 30.0, "s", 5.0, 180.0, "default", Low, "halt between bounds, covering, before the next"),
        figure!("sim.move.cover_search_m", 400.0, "m", 100.0, 1_500.0, "default", Default, "radius searched for a better position"),
        figure!("sim.comms.report_delay_s", 30.0, "s", 0.0, 300.0, "default", Default, "contact report delay to the side picture when no nets are modelled"),
        figure!("sim.morale.isolation_m", 1_500.0, "m", 300.0, 5_000.0, "default", Default, "no friend within this range counts as isolated"),
        figure!("sim.lod.aggregate_m", 15_000.0, "m", 3_000.0, 60_000.0, "default", Default, "a formation this far from any enemy track is aggregated"),
        figure!("sim.ad.engaged_reach_m", 150_000.0, "m", 20_000.0, 400_000.0, "default", Default, "an AD unit with a hostile air track this close is engaged, not asked to go quiet"),
        figure!("sim.quiet.max_jump_s", 300.0, "s", 10.0, 3_600.0, "default", Default, "longest quiet skip in one jump"),
    ]
}

/// Every figure the engine declares, across all crates.
pub fn all_figures() -> Vec<Figure> {
    let mut v = figures();
    v.extend(crate::domains::fires::figures());
    v.extend(crate::domains::air::figures());
    v.extend(crate::domains::naval::figures());
    v.extend(crate::domains::space::figures());
    v.extend(crate::domains::lod::figures());
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
    v.extend(crate::domains::drones::figures());
    v.extend(crate::routing::figures());
    v.extend(crate::formation::figures());
    v.extend(crate::urban::figures());
    v.extend(crate::appreciation::figures());
    v
}

/// The default registry.
pub fn default_figures() -> Figures {
    Figures::from_list(all_figures()).expect("the shipped figures are consistent")
}

/// When an IFV's infantry get out ([`crate::dismount`]).
#[derive(Clone, Debug)]
pub struct DismountParams {
    pub contact_m: f64,
    pub objective_m: f64,
    pub time_s: f64,
}

/// The parameter bundles, built once per run.
#[derive(Clone, Debug)]
pub struct Params {
    pub ground: GroundParams,
    pub acquire: AcquireParams,
    pub direct: DirectFireParams,
    pub people: PeopleParams,
    pub dismount: DismountParams,
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
    pub missile_keep: f64,
    pub team_front_m: f64,
    pub suppress_memory_s: f64,
    pub support_m: f64,
    pub support_error_m: f64,
    pub danger_close_m: f64,
    pub suppress_max_cost_usd: f64,
    pub suppress_aim_m: f64,
    pub suppress_drift_mps: f64,
    pub team_depth_m: f64,
    pub near_miss_per_round: f64,
    pub ineffective_volleys: u32,
    pub cooldown_s: f64,
    pub idle_s: f64,
    pub arrive_m: f64,
    pub withdraw_m: f64,
    pub cover_search_m: f64,
    pub reposition_m: f64,
    pub occupy_s: f64,
    pub bound_m: f64,
    pub bound_halt_s: f64,
    pub report_delay_s: f64,
    pub isolation_m: f64,
    pub aggregate_m: f64,
    pub quiet_max_jump_s: f64,
    pub ad_quiet_reach_m: f64,
    pub formation: crate::formation::FormationParams,
}

impl Params {
    pub fn from_figures(f: &Figures) -> Self {
        Params {
            ground: GroundParams::from_figures(f),
            acquire: AcquireParams::from_figures(f),
            direct: DirectFireParams::from_figures(f),
            people: PeopleParams::from_figures(f),
            dismount: DismountParams { contact_m: f.get("sim.dismount.contact_m"), objective_m: f.get("sim.dismount.objective_m"), time_s: f.get("sim.dismount.time_s") },
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
            missile_keep: f.get("sim.fire.missile_keep"),
            team_front_m: f.get("sim.fire.team_front_m"),
            suppress_memory_s: f.get("sim.fire.suppress_memory_s"),
            support_m: f.get("sim.fire.support_m"),
            support_error_m: f.get("sim.fire.support_error_m"),
            danger_close_m: f.get("sim.fire.danger_close_m"),
            suppress_max_cost_usd: f.get("sim.fire.suppress_max_cost_usd"),
            suppress_aim_m: f.get("sim.fire.suppress_aim_m"),
            suppress_drift_mps: f.get("sim.fire.suppress_drift_mps"),
            team_depth_m: f.get("sim.fire.team_depth_m"),
            near_miss_per_round: f.get("sim.fire.near_miss_per_round"),
            ineffective_volleys: f.get("sim.fire.ineffective_volleys") as u32,
            cooldown_s: f.get("sim.decide.cooldown_s"),
            idle_s: f.get("sim.decide.idle_s"),
            arrive_m: f.get("sim.move.arrive_m"),
            withdraw_m: f.get("sim.move.withdraw_m"),
            cover_search_m: f.get("sim.move.cover_search_m"),
            reposition_m: f.get("sim.exec.reposition_m"),
            occupy_s: f.get("sim.exec.occupy_s"),
            bound_m: f.get("sim.exec.bound_m"),
            bound_halt_s: f.get("sim.exec.bound_halt_s"),
            report_delay_s: f.get("sim.comms.report_delay_s"),
            isolation_m: f.get("sim.morale.isolation_m"),
            aggregate_m: f.get("sim.lod.aggregate_m"),
            quiet_max_jump_s: f.get("sim.quiet.max_jump_s"),
            ad_quiet_reach_m: f.get("sim.ad.engaged_reach_m"),
            formation: crate::formation::FormationParams::from_figures(f),
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
