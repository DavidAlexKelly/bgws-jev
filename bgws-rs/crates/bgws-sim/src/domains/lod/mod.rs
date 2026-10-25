//! Level of detail, aggregated combat, quiet skipping and the world's victory
//! conditions (RUST_ENGINE.md §6.3, §6.4, §8.13; research R8, P14).
//!
//! - [`formation`]: land units are grouped into formations at setup (by
//!   their scenario `parent`, else by side and proximity). A formation far
//!   from every enemy track and every entity-level enemy is **aggregated**:
//!   its members' state is folded into a `bgws_ops::aggregate::Formation`,
//!   one formation mover carries them along their routes, and the per-entity
//!   sensing, fire and people loops skip them (`Lod::Aggregated`). Two
//!   aggregated enemy formations in contact range fight by stochastic
//!   Lanchester attrition with coefficients from an [`formation::AttritionTable`]
//!   (the **[default]** unfitted rate until a fitted table is loaded). When an
//!   enemy track or entity-level enemy comes near, the formation is
//!   **disaggregated**: the keyed sample of which vehicles died is applied so
//!   killed stays killed, and stores and fatigue are sampled back. Rear
//!   logistics far from the enemy run at `Lod::Far` (no tactical sensing).
//! - [`quiet`]: quiet skipping (§6.3). `Sim::next_interesting_time` says how
//!   long nothing can be detected; `Sim::advance_quiet` runs the clock over
//!   that gap without the land sensing pass, which is provably empty there,
//!   so the outcome is exactly the stepped one.
//! - [`victory`]: air superiority, target sets, sea control, ships through,
//!   and all-of conditions, judged from truth every 10 s with hold timers.
//!
//! Aggregation and disaggregation happen at the 0.1 Hz boundary, never in the
//! middle of an engagement: a member that has seen, been shot at, fired in
//! the last `sim.lod.calm_s` or is deciding keeps its formation at entity level.

pub mod formation;
pub mod quiet;
pub mod synthetic;
pub mod victory;

use crate::clock::Rate;
use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_behaviour::{Capability, FactValue};
use bgws_core::{figure, EntityId, Figure, Figures, Side};
use std::collections::{BTreeMap, BTreeSet};

pub use formation::{AttritionTable, FormationState};

/// The LOD and quiet-skipping figures (rule 3). `sim.lod.aggregate_m` and
/// `sim.quiet.max_jump_s` are declared with the sim's own figures.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.lod.hysteresis", 0.8, "fraction", 0.3, 1.0, "default", Default, "an aggregated formation disaggregates when an enemy comes within this share of the aggregation distance"),
        figure!("sim.lod.cluster_m", 5_000.0, "m", 500.0, 30_000.0, "default", Default, "units of one side without a parent within this distance of each other form one formation"),
        figure!("sim.lod.contact_m", 3_000.0, "m", 200.0, 20_000.0, "default", Default, "aggregated enemy formations this close (edge to edge) fight by aggregate attrition"),
        figure!("sim.lod.calm_s", 120.0, "s", 0.0, 1_800.0, "default", Default, "a member that fired or was fired on this recently keeps its formation at entity level"),
        figure!("sim.lod.far_factor", 2.0, "×", 1.0, 10.0, "default", Default, "rear logistics beyond this multiple of the aggregation distance from any enemy run at the Far level"),
        figure!("sim.lod.consume_per_combat_h", 0.15, "fraction/h", 0.0, 1.0, "default", Default, "share of basic load an aggregated formation uses per hour of aggregate combat"),
        figure!("sim.quiet.reach_margin_m", 250.0, "m", 0.0, 5_000.0, "default", Default, "safety margin on sensor reach when estimating the earliest possible detection"),
        figure!("sim.quiet.closing_margin", 1.25, "×", 1.0, 3.0, "default", Default, "safety factor on the fastest closing speed when estimating the earliest possible detection"),
    ]
}

/// The LOD parameters, from the figures.
#[derive(Clone, Debug, PartialEq)]
pub struct LodParams {
    pub aggregate_m: f64,
    pub hysteresis: f64,
    pub cluster_m: f64,
    pub contact_m: f64,
    pub calm_s: f64,
    pub far_factor: f64,
    pub consume_per_combat_h: f64,
    pub reach_margin_m: f64,
    pub closing_margin: f64,
    pub max_jump_s: f64,
}

impl LodParams {
    pub fn from_figures(f: &Figures) -> Self {
        LodParams {
            aggregate_m: f.get("sim.lod.aggregate_m"),
            hysteresis: f.get("sim.lod.hysteresis"),
            cluster_m: f.get("sim.lod.cluster_m"),
            contact_m: f.get("sim.lod.contact_m"),
            calm_s: f.get("sim.lod.calm_s"),
            far_factor: f.get("sim.lod.far_factor"),
            consume_per_combat_h: f.get("sim.lod.consume_per_combat_h"),
            reach_margin_m: f.get("sim.quiet.reach_margin_m"),
            closing_margin: f.get("sim.quiet.closing_margin"),
            max_jump_s: f.get("sim.quiet.max_jump_s"),
        }
    }
}

/// The domain's state, kept as a world resource.
#[derive(Resource, Debug)]
pub struct LodState {
    pub p: LodParams,
    /// The scope aggregates away from contact.
    pub enabled: bool,
    pub formations: Vec<FormationState>,
    /// Member → formation index.
    pub of: BTreeMap<EntityId, usize>,
    /// Attrition coefficients by formation-type pair.
    pub table: AttritionTable,
    /// Bumped on every aggregation; keys the disaggregation draws.
    pub epoch: u64,
    /// Rounds per vehicle per mount at the start (basic load).
    pub basic: BTreeMap<EntityId, Vec<f64>>,
    /// Rear logistics units that may run at the Far level.
    pub logistics: Vec<EntityId>,
    pub victory: victory::VictoryState,
    pub quiet: quiet::QuietState,
}

/// Run `f` with the state taken out of the world (so `f` may borrow the sim).
pub(crate) fn with_state<R>(s: &mut Sim, f: impl FnOnce(&mut Sim, &mut LodState) -> R) -> Option<R> {
    let mut st = s.world.remove_resource::<LodState>()?;
    let r = f(s, &mut st);
    s.world.insert_resource(st);
    Some(r)
}

pub fn setup(s: &mut Sim) {
    let p = LodParams::from_figures(&s.ctx.figures);
    let enabled = s.ctx.scenario.scope.detail.aggregate_away_from_contact;
    let mut st = LodState {
        p,
        enabled,
        formations: Vec::new(),
        of: BTreeMap::new(),
        table: AttritionTable::unfitted(&s.ctx.figures),
        epoch: 0,
        basic: BTreeMap::new(),
        logistics: Vec::new(),
        victory: victory::VictoryState::default(),
        quiet: quiet::QuietState::default(),
    };
    formation::build(s, &mut st);
    s.world.insert_resource(st);
}

pub fn fast_active(_s: &Sim) -> bool {
    false
}
pub fn air_active(_s: &Sim) -> bool {
    false
}
pub fn fast(_s: &mut Sim, _dt: f64) {}

/// The formation mover each second, and the victory judgement every 10 s.
pub fn base(s: &mut Sim, dt: f64) {
    with_state(s, |s, st| {
        if st.enabled {
            formation::move_aggregated(s, st, dt);
        }
        if Rate::Slow.due(s.ctx.now) && !s.ctx.scenario.victory.is_empty() {
            victory::evaluate(s, st);
        }
    });
}

/// Aggregation, disaggregation, aggregate attrition and the Far level, at 0.1 Hz.
pub fn slow(s: &mut Sim, dt: f64) {
    with_state(s, |s, st| {
        if st.enabled {
            formation::pass(s, st, dt);
        }
    });
}
pub fn minute(_s: &mut Sim, _dt: f64) {}
pub fn script_event(_s: &mut Sim, _e: &bgws_scenario::v4::ScriptedEvent) {}
/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(_s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    Some(ev)
}
pub fn facts(_s: &Sim, _e: Entity, _m: &mut BTreeMap<String, FactValue>) {}
pub fn capabilities(_s: &Sim, _e: Entity, _c: &mut BTreeSet<Capability>) {}
pub fn apply_action(_s: &mut Sim, _e: Entity, _id: EntityId, _action: &str, _params: &BTreeMap<String, serde_json::Value>, _about: Option<EntityId>) -> bool {
    false
}

/// A world victory condition met and held (judged in `base`).
pub fn victory(s: &Sim) -> Option<(Option<Side>, String)> {
    s.world.get_resource::<LodState>().and_then(|st| st.victory.verdict.clone())
}

/// Load a fitted attrition table (JSON of [`AttritionTable`]) in place of the defaults.
pub fn load_attrition_table(s: &mut Sim, json: &str) -> Result<(), serde_json::Error> {
    let t: AttritionTable = serde_json::from_str(json)?;
    if let Some(mut st) = s.world.get_resource_mut::<LodState>() {
        st.table = t;
    }
    Ok(())
}

/// Bring a formation back to entity level now (a player or planner focuses
/// on it, §6.4). False when it is not aggregated. It may aggregate again at
/// a later 0.1 Hz pass if it is still far from every enemy.
pub fn disaggregate(s: &mut Sim, formation: u64, why: &str) -> bool {
    with_state(s, |s, st| {
        let Some(fi) = st.formations.iter().position(|f| f.id == formation && f.aggregated()) else { return false };
        formation::disaggregate_formation(s, st, fi, why);
        true
    })
    .unwrap_or(false)
}

/// The formations and their state (for views, tests and the AAR).
pub fn formations(s: &Sim) -> Vec<FormationState> {
    s.world.get_resource::<LodState>().map(|st| st.formations.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn figures_register() {
        let f = bgws_core::Figures::from_list(crate::all_figures()).expect("figures");
        let p = super::LodParams::from_figures(&f);
        assert!(p.aggregate_m > p.contact_m);
        assert!(p.hysteresis < 1.0);
    }
}
