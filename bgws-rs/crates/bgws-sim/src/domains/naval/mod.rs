//! Naval and undersea (RUST_ENGINE.md §8.1 Sea, §8.2 sonar, §8.6; research
//! R6.1–R6.3, P28): surface ships, submarines, USVs and merchants on the
//! same clock as the land battle.
//!
//! - [`fleet`]: movement on tasks and lanes with the hull's sea-state limit
//!   and cubic fuel law (`bgws_domains::naval::HullParams`), the water
//!   polygons, submarines' depth and battery, ports, VLS reload and UNREP,
//!   mines (actuation per transit, sweeping);
//! - [`sense`]: ship and coastal radar against surface contacts with the
//!   4/3-Earth radar horizon and sea clutter, passive and active sonar
//!   through the sound-speed profile's transmission loss, into the sides'
//!   sea-domain tracks; the contact decision points;
//! - [`defence`]: anti-ship missiles and one-way drones as kinematic flights
//!   against ships, the ship's layered defence as a salvo against layers with
//!   magazines that deplete (`bgws_domains::air_defence::engage_raid`), hits
//!   on `ShipDamage` compartments with fire, flooding and damage control,
//!   mission kill and sinking; USVs and torpedoes;
//! - [`actions`]: the `sea.*` world actions the warship and submarine
//!   templates offer.
//!
//! Air threats aimed at ships (an air entity with a `strike` task on a ship)
//! are claimed by this domain at setup: they carry a [`NavalStrike`]
//! component and naval flies them, so the air domain should skip them.

pub mod actions;
pub mod defence;
pub mod fleet;
pub mod sense;

use crate::components::*;
use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_behaviour::{Capability, FactValue};
use bgws_catalogue::Domain;
use bgws_core::{figure, EntityId, Figure, Figures, Side, SimTime};
use bgws_domains::naval::{DamageParams, SubParams};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use bgws_scenario::v4::VictoryCondition;
use std::collections::{BTreeMap, BTreeSet};

pub use defence::{Inbound, InboundState, NavalStrike, RaidRecord};
pub use fleet::{NavTask, ShipCombat, Vessel, VesselKind};

/// The sim's own naval figures (rule 3): thresholds and rates of the naval
/// systems that the model crates leave to the caller.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.naval.in_port_m", 3_000.0, "m", 500.0, 10_000.0, "default", Default, "a ship this close to its port is alongside"),
        figure!("sim.naval.arrive_m", 500.0, "m", 50.0, 3_000.0, "default", Default, "a waypoint at sea counts as reached"),
        figure!("sim.naval.station_speed", 0.5, "fraction", 0.1, 1.0, "default", Default, "share of cruise speed while holding station"),
        figure!("sim.naval.quiet_speed", 0.3, "fraction", 0.1, 1.0, "default", Default, "share of cruise speed while running silent"),
        figure!("sim.naval.sprint_s", 600.0, "s", 60.0, 3_600.0, "default", Default, "how long a sprint (evasion, decoy turn) lasts"),
        figure!("sim.naval.silent_s", 1_800.0, "s", 300.0, 7_200.0, "default", Default, "how long a boat stays silent once it goes quiet"),
        figure!("sim.naval.keel_m", 15.0, "m", 5.0, 50.0, "default", Default, "clearance a submarine keeps off the bottom"),
        figure!("sim.naval.water_depth_m", 120.0, "m", 10.0, 6_000.0, "default", Default, "water depth where no bathymetry is loaded"),
        figure!("sim.naval.patrol_depth_m", 100.0, "m", 20.0, 400.0, "default", Default, "a submarine's patrol depth"),
        figure!("sim.naval.fuel_low", 0.25, "fraction", 0.05, 0.5, "default", Default, "fuel share below which a ship goes to refuel"),
        figure!("sim.naval.unrep_m", 1_000.0, "m", 100.0, 5_000.0, "default", Default, "alongside distance for UNREP and tender reloads"),
        figure!("sim.naval.fetch_m", 200_000.0, "m", 5_000.0, 1_000_000.0, "default", Default, "wave fetch for a sea state grown from the wind"),
        figure!("sim.naval.mine_density_scale", 1.0, "km2*m", 0.01, 100.0, "default", Default, "sea mines per km2 per unit of the scenario's density figure"),
        figure!("sim.naval.mine_warhead", 1.5, "scale", 0.2, 4.0, "default", Default, "a mine's damage relative to an anti-ship missile (under the keel)"),
        figure!("sim.naval.degauss", 0.3, "fraction", 0.05, 1.0, "default", Default, "magnetic signature left on a degaussed warship"),
        figure!("sim.naval.mcm_assets", 1.0, "count", 0.0, 10.0, "default", Default, "sweep/hunt assets one MCM vessel counts for"),
        // Sensing.
        figure!("sim.naval.report_delay_s", 10.0, "s", 0.0, 120.0, "default", Default, "own-ship radar and sonar contacts reach the side's picture (link 11/16)"),
        figure!("sim.naval.mast_m", 25.0, "m", 3.0, 60.0, "default", Default, "radar-significant height of a large ship (for the horizon)"),
        figure!("sim.naval.radar_prf_hz", 1_000.0, "Hz", 100.0, 10_000.0, "default", Default, "surface-search radar PRF where the catalogue gives none"),
        figure!("sim.naval.radar_scan_s", 4.0, "s", 1.0, 20.0, "default", Default, "surface-search radar scan period where the catalogue gives none"),
        figure!("sim.naval.radar_sigma_m", 150.0, "m", 10.0, 2_000.0, "default", Default, "1-sigma position error of a radar surface fix"),
        figure!("sim.naval.identify_m", 15_000.0, "m", 2_000.0, 40_000.0, "default", Default, "a ship this close identifies a contact by eye and optics"),
        figure!("sim.naval.sonar_bearing_sigma", 0.03, "rad", 0.005, 0.2, "default", Default, "sonar bearing error, sets the fix's position error with range"),
        figure!("sim.naval.ts_sub_db", 10.0, "dB", 0.0, 30.0, "default", Default, "active-sonar target strength of a submarine"),
        figure!("sim.naval.ts_ship_db", 25.0, "dB", 5.0, 40.0, "default", Default, "active-sonar target strength of a surface ship"),
        figure!("sim.naval.ping_heard_factor", 2.0, "fraction", 1.0, 5.0, "default", Default, "an active sonar is heard this many times its own detection range"),
        figure!("sim.naval.contact_ask_s", 600.0, "s", 60.0, 3_600.0, "default", Default, "least time between two questions about the same contact"),
        // Defence.
        figure!("sim.naval.cic_auto_s", 15.0, "s", 2.0, 120.0, "default", Default, "an unanswered raid is engaged by the combat system's doctrine after this"),
        figure!("sim.naval.cic_hold_s", 300.0, "s", 30.0, 1_800.0, "default", Default, "a layer once chosen covers new threats for this long without asking again"),
        figure!("sim.naval.cic_defer_pk_ratio", 0.4, "fraction", 0.0, 1.0, "R6.2", Low, "a ship holds a missile for its guns if they have this share of its Pk"),
        figure!("sim.naval.threat_value_usd", 2.0e6, "USD", 1.0e4, 1.0e9, "default", Default, "value of a leaker for weapon-target assignment"),
        figure!("sim.naval.cheap_threat_usd", 2.0e5, "USD", 1.0e3, 2.0e6, "default", Default, "threats cheaper than this are left to the inner layers"),
        figure!("sim.naval.close_threat_m", 15_000.0, "m", 2_000.0, 50_000.0, "default", Default, "a raid this close is met by every layer that bears"),
        figure!("sim.naval.mutual_support_m", 8_000.0, "m", 0.0, 30_000.0, "default", Default, "consorts this close add their layers to a ship's defence"),
        figure!("sim.naval.ciws_rounds_per_burst", 50.0, "count", 10.0, 200.0, "P28", Default, "CIWS rounds per engagement burst"),
        figure!("sim.naval.gun5_range_m", 15_000.0, "m", 3_000.0, 25_000.0, "R6.2", Default, "medium-calibre gun's reach against air targets"),
        figure!("sim.naval.gun5_pk_owa", 0.35, "fraction", 0.0, 1.0, "R6.2", Low, "medium-calibre gun burst Pk against a one-way drone"),
        figure!("sim.naval.gun5_pk_missile", 0.12, "fraction", 0.0, 1.0, "default", Default, "medium-calibre gun burst Pk against a missile"),
        figure!("sim.naval.gun5_rounds_per_burst", 2.0, "count", 1.0, 10.0, "R6.2", Default, "rounds per gun engagement (R6.2: 160 rounds in 380 engagements)"),
        figure!("sim.naval.gun5_cycle_s", 4.0, "s", 1.0, 20.0, "default", Default, "time between gun bursts"),
        figure!("sim.naval.ripple_s", 2.0, "s", 0.5, 10.0, "default", Default, "interval between missiles in a salvo"),
        figure!("sim.naval.seeker_basket_m", 8_000.0, "m", 500.0, 30_000.0, "default", Default, "an anti-ship missile's seeker finds a ship this far from its aim point"),
        figure!("sim.naval.owa_basket_m", 1_500.0, "m", 100.0, 10_000.0, "default", Default, "a one-way drone's terminal seeker basket against a ship"),
        figure!("sim.naval.ref_warhead_kg", 200.0, "kg", 50.0, 1_000.0, "default", Default, "the warhead a damage scale of 1 stands for"),
        figure!("sim.naval.ref_staying", 3.0, "hits", 0.5, 10.0, "R6.1", Default, "staying power the damage model's layout is tuned for"),
        figure!("sim.naval.decoy_factor", 0.4, "fraction", 0.0, 1.0, "default", Default, "hit chance left to a seeker against a ship firing decoys and turning"),
        figure!("sim.naval.decoy_s", 120.0, "s", 20.0, 600.0, "default", Default, "how long a decoy pattern lasts"),
        figure!("sim.naval.torpedo_under_keel", 2.0, "scale", 1.0, 4.0, "default", Default, "a torpedo's damage multiplier for exploding under the keel"),
        figure!("sim.naval.evade_factor", 0.5, "fraction", 0.0, 1.0, "default", Default, "torpedo hit chance left to an evading target"),
        figure!("sim.naval.usv_hit_m", 60.0, "m", 10.0, 300.0, "R6.3", Default, "a USV this close to its target detonates"),
        figure!("sim.naval.usv_warhead", 0.8, "scale", 0.1, 3.0, "R6.3", Default, "a USV's damage relative to an anti-ship missile"),
        figure!("sim.naval.swarm_m", 12_000.0, "m", 2_000.0, 40_000.0, "R6.3", Default, "USVs this close count towards a swarm"),
        figure!("sim.naval.swarm_size", 3.0, "count", 2.0, 20.0, "R6.3", Default, "USVs that make a swarm"),
        figure!("sim.naval.gun_usv_range_m", 6_000.0, "m", 1_000.0, 20_000.0, "default", Default, "guns and helicopter reach against small boats"),
        figure!("sim.naval.gun_usv_pk", 0.3, "fraction", 0.0, 1.0, "R6.3", Low, "chance a ship's guns stop one small boat per 10 s"),
        figure!("sim.naval.ciws_usv_range_m", 1_500.0, "m", 300.0, 4_000.0, "default", Default, "close-in guns' reach against small boats without orders"),
        figure!("sim.naval.guns_s", 1_800.0, "s", 300.0, 7_200.0, "default", Default, "how long guns stay on small boats once ordered"),
        figure!("sim.naval.shadow_m", 30_000.0, "m", 5_000.0, 100_000.0, "default", Default, "distance a ship shadows a contact from"),
        figure!("sim.naval.avoid_m", 50_000.0, "m", 10_000.0, 200_000.0, "default", Default, "how far a ship opens from a contact it avoids"),
        figure!("sim.naval.evade_m", 20_000.0, "m", 2_000.0, 60_000.0, "default", Default, "how far a boat runs when it evades"),
        figure!("sim.naval.shipping_pause_s", 1_800.0, "s", 0.0, 86_400.0, "R6.3", Default, "merchants stop sailing for this long after a ship is hit near them"),
        figure!("sim.naval.shipping_pause_m", 60_000.0, "m", 5_000.0, 300_000.0, "R6.3", Default, "a ship hit this close to a merchant stops it"),
        figure!("sim.naval.dc_destroyer", 3.0, "count", 0.0, 8.0, "default", Default, "damage-control parties on a destroyer"),
        figure!("sim.naval.dc_small", 2.0, "count", 0.0, 8.0, "default", Default, "damage-control parties on a frigate, corvette or merchant"),
    ]
}

/// Parameter bundle built once per run.
#[derive(Clone, Debug)]
pub struct NavalParams {
    pub f: BTreeMap<String, f64>,
    pub damage: DamageParams,
    pub sub: SubParams,
    pub withdraw_fraction: f64,
}

impl NavalParams {
    pub fn from_figures(f: &Figures) -> Self {
        let mut m = BTreeMap::new();
        for fig in figures() {
            m.insert(fig.key.trim_start_matches("sim.naval.").to_string(), f.get(&fig.key));
        }
        NavalParams { f: m, damage: DamageParams::from_figures(f), sub: SubParams::from_figures(f), withdraw_fraction: f.get("naval.reload.withdraw_fraction") }
    }
    /// A `sim.naval.*` figure by its short key.
    pub fn g(&self, k: &str) -> f64 {
        *self.f.get(k).unwrap_or_else(|| panic!("naval figure {k} is declared"))
    }
}

/// One minefield in the theatre's frame.
#[derive(Clone, Debug)]
pub struct Mines {
    pub id: String,
    pub owner: Side,
    pub field: bgws_domains::naval::Minefield,
    pub centre: bgws_core::Enu,
    pub half_w: f64,
    pub half_d: f64,
    pub bearing: f64,
    pub area_km2: f64,
    pub known_to: BTreeSet<Side>,
}

impl Mines {
    pub fn contains(&self, p: bgws_core::Enu) -> bool {
        let d = p.sub(self.centre);
        // Rotate into the field's axes: width across the bearing, depth along it.
        let (s, c) = self.bearing.sin_cos();
        let along = d.e * s + d.n * c;
        let across = d.e * c - d.n * s;
        across.abs() <= self.half_w && along.abs() <= self.half_d
    }
}

/// Counters the AAR and the vignettes read.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct NavalStats {
    /// Interceptors (SAMs) fired by ships.
    pub missiles: u32,
    /// Threats engaged by anything (shots > 0).
    pub engagements: u32,
    pub gun_rounds: u32,
    pub leakers: u32,
    pub ship_hits: u32,
    pub kills_by_system: BTreeMap<String, u32>,
    pub interceptor_cost: f64,
    pub salvos: u32,
    pub mine_strikes: u32,
    pub transits: BTreeMap<EntityId, u32>,
    pub sonar_contacts: u32,
    pub radar_contacts: u32,
    pub neutral_hits: u32,
}

/// The domain's state outside the components.
#[derive(Resource)]
pub struct NavalState {
    pub p: NavalParams,
    pub mode: DomainMode,
    /// Douglas sea state where the scenario fixes it; else the field below.
    pub sea_state_fixed: Option<f64>,
    pub sea: Option<bgws_env::SeaStateField>,
    pub profile: bgws_env::SoundSpeedProfile,
    pub water_depth: f64,
    /// Water polygons in ENU (none: all water).
    pub waters: Vec<Vec<(f64, f64)>>,
    pub mines: Vec<Mines>,
    pub inbound: BTreeMap<u64, Inbound>,
    pub torpedoes: BTreeMap<u64, defence::Torpedo>,
    pub next_id: u64,
    pub raids: Vec<RaidRecord>,
    pub stats: NavalStats,
    /// (asker, about) → when last asked.
    pub asked: BTreeMap<(EntityId, &'static str, Option<EntityId>), SimTime>,
    /// Ships hit recently, and where: merchants near them stop sailing.
    pub recent_hits: Vec<(SimTime, bgws_core::Enu)>,
    /// Pairs (observer, target) already reported once, for the first-contact log.
    pub seen: BTreeSet<(EntityId, EntityId)>,
    /// Sonar contacts this slow tick (hunter, target), for the questions.
    pub fresh_sonar: Vec<(EntityId, EntityId)>,
    /// Questions to raise once the state is back in the world:
    /// (asker, point, about, rules option, rules action).
    pub deferred: Vec<(EntityId, &'static str, Option<EntityId>, &'static str, &'static str)>,
    /// Port stores of munitions and fuel.
    pub ports: BTreeMap<EntityId, BTreeMap<String, f64>>,
}

impl NavalState {
    pub fn next_key(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    /// Douglas sea state at a point now.
    pub fn sea_state(&self, e: f64, n: f64, t: SimTime) -> f64 {
        match (&self.sea_state_fixed, &self.sea) {
            (Some(ss), _) => *ss,
            (None, Some(f)) => f.douglas_at(e, n, t) as f64,
            _ => 3.0,
        }
    }
    pub fn is_water(&self, e: f64, n: f64) -> bool {
        self.waters.is_empty() || self.waters.iter().any(|poly| point_in_poly(poly, e, n))
    }
    pub fn recently_asked(&self, key: (EntityId, &'static str, Option<EntityId>), now: SimTime, gap_s: f64) -> bool {
        self.asked.get(&key).map(|t| (now - *t).secs() < gap_s).unwrap_or(false)
    }
}

fn point_in_poly(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Take the domain state out of the world for a system (put it back after).
pub(crate) fn take(s: &mut Sim) -> Option<NavalState> {
    s.world.remove_resource::<NavalState>()
}
pub(crate) fn put(s: &mut Sim, st: NavalState) {
    s.world.insert_resource(st);
}

pub(crate) fn on(s: &Sim) -> bool {
    s.world.get_resource::<NavalState>().map(|st| st.mode != DomainMode::Off).unwrap_or(false)
}

pub fn setup(s: &mut Sim) {
    let mode = s.ctx.scenario.scope.mode(ScopeDomain::Naval);
    if mode == DomainMode::Off {
        return;
    }
    let st = fleet::build_state(s, mode);
    put(s, st);
    fleet::setup_vessels(s);
    defence::claim_air_threats(s);
}

pub fn fast_active(_s: &Sim) -> bool {
    false
}
pub fn air_active(_s: &Sim) -> bool {
    false
}
pub fn fast(_s: &mut Sim, _dt: f64) {}

pub fn base(s: &mut Sim, dt: f64) {
    if !on(s) {
        return;
    }
    fleet::movement(s, dt);
    defence::fly_threats(s);
    defence::ship_defence(s);
    defence::usv_runs(s);
    flush_asks(s);
}

/// Raise the questions systems and actions queued while they held the state.
pub(crate) fn flush_asks(s: &mut Sim) {
    let Some(mut st) = take(s) else { return };
    let asks = std::mem::take(&mut st.deferred);
    put(s, st);
    for (id, point, about, opt, action) in asks {
        fleet::ask_or_do(s, id, point, about, opt, action);
    }
}

pub fn slow(s: &mut Sim, dt: f64) {
    if !on(s) {
        return;
    }
    sense::radar(s, dt);
    sense::sonar(s, dt);
    sense::contacts(s);
    defence::usv_guns(s);
    fleet::mines(s, dt);
    fleet::magazines(s);
    flush_asks(s);
}

pub fn minute(s: &mut Sim, dt: f64) {
    if !on(s) {
        return;
    }
    defence::damage_control(s, dt);
    fleet::ports_and_fuel(s, dt);
}

pub fn script_event(_s: &mut Sim, _e: &bgws_scenario::v4::ScriptedEvent) {}

/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    match &ev {
        Scheduled::Timer { system, key } if system.starts_with("naval.") => {
            let key = *key;
            match system.as_str() {
                "naval.torpedo" => defence::torpedo_arrives(s, key),
                "naval.reloaded" => fleet::reloaded(s, EntityId(key as u32)),
                "naval.drift" => fleet::drift(s, EntityId(key as u32)),
                _ => {}
            }
            None
        }
        _ => Some(ev),
    }
}

/// The naval facts for a ship's, submarine's or coastal battery's template walk.
pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    let Some(st) = s.world.get_resource::<NavalState>() else { return };
    let Some(i) = s.world.get::<Ident>(e) else { return };
    let Some(v) = s.world.get::<Vessel>(e) else { return };
    let domain = match i.domain {
        Domain::Subsurface => "subsurface",
        _ => "surface",
    };
    m.insert("agent.domain".into(), FactValue::Text(domain.into()));
    m.insert("agent.archetype".into(), FactValue::Text(i.platform.id.clone()));
    m.insert("agent.fuelPct".into(), FactValue::Num((100.0 * v.fuel_t / v.hull.fuel_capacity.max(1e-9)).round()));
    if let Some(c) = s.world.get::<ShipCombat>(e) {
        let (left, full) = c.sams();
        if full > 0 {
            m.insert("agent.magazinePct".into(), FactValue::Num((100.0 * left as f64 / full as f64).round()));
        }
        m.insert("agent.armed".into(), FactValue::Bool(c.magazine.values().any(|r| *r > 0)));
        let lead = c.pending.first().and_then(|id| st.inbound.get(id));
        if let Some(t) = lead {
            m.insert("target.costUsd".into(), FactValue::Num(t.cost));
            let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
            let at = t.flight.position(s.ctx.now.secs());
            m.insert("target.rangeKm".into(), FactValue::Num(((at.x - pos.e).hypot(at.y - pos.n) / 1000.0).round()));
        }
    }
    if let Some(a) = s.world.get::<Act>(e) {
        m.insert("roe.weaponsFree".into(), FactValue::Bool(a.roe == RoeS::Always));
    }
}

pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    if let Some(sc) = s.world.get::<ShipCombat>(e) {
        if sc.magazine.values().any(|r| *r > 0) && !sc.state.mission_kill {
            c.insert(Capability::Weapon);
        }
        if s.world.get::<Vessel>(e).map(|v| !v.silent).unwrap_or(false) {
            c.insert(Capability::Radar);
            c.insert(Capability::Emitter);
        }
    }
}

pub fn apply_action(s: &mut Sim, e: Entity, id: EntityId, action: &str, params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    if !action.starts_with("sea.") || !on(s) {
        return false;
    }
    actions::apply(s, e, id, action, params, about)
}

/// Shipping through a strait and sea control: the naval victory conditions.
pub fn victory(s: &Sim) -> Option<(Option<Side>, String)> {
    let st = s.world.get_resource::<NavalState>()?;
    for c in &s.ctx.scenario.victory {
        if let VictoryCondition::Through { side, ids, count, label, .. } = c {
            let through: u32 = ids.iter().filter_map(|i| s.ctx.by_name.get(i)).map(|id| st.stats.transits.get(id).copied().unwrap_or(0)).sum();
            if through >= *count && *count > 0 {
                return Some((Some(*side), format!("{}: {through} through", label.clone().unwrap_or_else(|| "shipping through".into()))));
            }
        }
    }
    None
}

/// Short helper: the label of an entity.
pub(crate) fn label(s: &Sim, id: EntityId) -> String {
    crate::systems::name(s, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_polygons_and_minefield_extent() {
        let sq = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(point_in_poly(&sq, 5.0, 5.0));
        assert!(!point_in_poly(&sq, 15.0, 5.0));
        let m = Mines {
            id: "f".into(),
            owner: Side::Red,
            field: bgws_domains::naval::Minefield { kind: bgws_domains::naval::MineKind::Magnetic, density: 1.0, cleared: 0.0 },
            centre: bgws_core::Enu::new(0.0, 0.0, 0.0),
            half_w: 1_000.0,
            half_d: 100.0,
            bearing: 0.0,
            area_km2: 0.4,
            known_to: BTreeSet::new(),
        };
        // Bearing north: the width runs east-west, the depth north-south.
        assert!(m.contains(bgws_core::Enu::new(900.0, 50.0, 0.0)));
        assert!(!m.contains(bgws_core::Enu::new(50.0, 900.0, 0.0)));
    }

    #[test]
    fn naval_figures_are_declared() {
        let f = crate::params::default_figures();
        let p = NavalParams::from_figures(&f);
        assert!(p.g("cic_defer_pk_ratio") > 0.0 && p.withdraw_fraction > 0.0);
    }
}
