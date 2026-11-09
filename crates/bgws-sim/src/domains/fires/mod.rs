//! Domain systems: indirect fire and counter-battery, engineers and
//! obstacles, unit stores, the logistics network, maintenance, casualties and
//! the medical chain (RUST_ENGINE.md §8.8 "Indirect fire", §8.9, §8.10
//! casualties, §8.11; roadmap §18 phases 3 and 7).
//!
//! - [`indirect`]: calls for fire from observers, the fires cell giving each
//!   to a shooter (the `fireRequestReceived` decision), laying, ballistic time
//!   of flight from `bgws_models::indirect_fire`, delivery errors by met age
//!   and PNT, the adjust-fire loop when observed, rounds landing as scheduled
//!   events with Carleton lethality by posture and suppression by blasts,
//!   weapon-locating radars backtracking guns **[ref R4.3]**, and shoot and
//!   scoot (`counterBatteryThreat`, displacement time **[P23]**).
//! - [`obstacles`]: the scenario's minefields and ditches on the ground,
//!   encounters by `bgws_models::engineers`, halts and the obstacle decisions,
//!   breaching (about 1.5 h per 6 m lane **[P13]**), digging in.
//! - [`sustain`]: stores per unit and consumption from activity
//!   (`bgws_ops::logistics::consumption`), low-stock decisions, running dry.
//! - [`logistics`]: the supply network as a min-cost flow each cycle, convoys
//!   as entities that can be seen and shot, interdiction, culmination, and
//!   maintenance by operating hours **[ref R7, P17–P19]**.
//! - [`medical`]: casualties from losses, triage and the evacuation chain
//!   (`bgws_ops::medical::MedicalChain`) under the evacuation-delay choice
//!   **[ref R7]**.
//!
//! The domain's state is one resource ([`FiresWorld`]) and a few components
//! ([`Stores`], [`Gun`], [`Dig`], [`Tally`]); every system iterates entities
//! in id order and draws from keyed streams, so a run is deterministic (§12).

pub mod indirect;
pub mod logistics;
pub mod medical;
pub mod obstacles;
pub mod params;
pub mod sustain;

pub use params::{figures, FParams};

use crate::components::*;
use crate::queue::Scheduled;
use crate::sim::{Sim, Trigger};
use bevy_ecs::prelude::{Component, Entity, Resource};
use bgws_behaviour::{Capability, FactValue, RulesChoice};
use bgws_catalogue::{Domain, Platform, WeaponKind};
use bgws_core::{EntityId, Side};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use std::collections::{BTreeMap, BTreeSet};

/// The domain's state, kept in the ECS world as a resource.
#[derive(Resource)]
pub struct FiresWorld {
    pub p: FParams,
    pub fires_mode: DomainMode,
    pub logistics_mode: DomainMode,
    pub medical_mode: DomainMode,
    pub fires: indirect::FiresState,
    pub obstacles: obstacles::ObstacleState,
    pub logistics: logistics::LogisticsState,
    pub medical: medical::MedicalState,
}

/// A unit's stores (§8.11): fuel, ammunition (its mounts' rounds against the
/// basic load), food and water, and what running low has done to it.
#[derive(Component, Clone, Debug)]
pub struct Stores {
    /// Litres for the whole element; `fuel_max_l` 0 means it burns none.
    pub fuel_l: f64,
    pub fuel_max_l: f64,
    /// Basic load per mount index.
    pub ammo_max: BTreeMap<usize, u32>,
    pub food_kg: f64,
    pub food_max_kg: f64,
    pub water_l: f64,
    pub water_max_l: f64,
    pub people: u32,
    pub class: Option<bgws_ops::logistics::VehicleClass>,
    /// When it reported stocks low (cleared on resupply).
    pub low_since: Option<bgws_core::SimTime>,
    /// The order it was on when its fuel ran out.
    pub stalled: Option<Activity>,
    pub last_pos: bgws_core::Enu,
    /// Operating hours since the last failure roll (maintenance).
    pub op_hours: f64,
    /// It carries cargo (a convoy) rather than fighting.
    pub carrier_t: f64,
}

impl Stores {
    pub fn fuel_pct(&self) -> f64 {
        if self.fuel_max_l > 0.0 {
            (self.fuel_l / self.fuel_max_l).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
    /// Ammunition left as a share of the basic load (1 if it carries none).
    pub fn ammo_pct(&self, arms: &Arms) -> f64 {
        let max: u32 = self.ammo_max.values().sum();
        if max == 0 {
            return 1.0;
        }
        let have: u32 = arms.mounts.iter().filter(|m| self.ammo_max.contains_key(&m.index)).map(|m| m.rounds).sum();
        (have as f64 / max as f64).clamp(0.0, 1.0)
    }
}

/// An indirect-fire unit's state (shoot and scoot).
#[derive(Component, Clone, Debug)]
pub struct Gun {
    pub system: bgws_models::indirect_fire::FiringSystem,
    /// Missions fired since it last moved.
    pub fired_here: u32,
    /// It has been told (rounds close by) that it was found.
    pub located: bool,
    /// Coming out of action: it moves off at this time.
    pub displace_at: Option<bgws_core::SimTime>,
}

/// Digging in (§8.9 field fortifications).
#[derive(Component, Clone, Debug, Default)]
pub struct Dig {
    pub active: bool,
    pub done: bool,
    pub worked_s: f64,
    pub need_s: f64,
    pub asked: Option<bgws_core::SimTime>,
}

/// What it had last tick, to see losses and damage from any cause.
#[derive(Component, Clone, Debug, Default)]
pub struct Tally {
    pub fit: u32,
    pub mobility: u32,
    pub firepower: u32,
    pub destroyed: bool,
}

// ── Shared helpers ───────────────────────────────────────────────────────

pub(crate) fn mode(s: &Sim, d: ScopeDomain) -> DomainMode {
    s.ctx.scenario.scope.mode(d)
}

/// Is this platform mount an indirect-fire weapon (mortar, howitzer, rocket)?
pub fn indirect_mount(p: &Platform, index: usize) -> bool {
    p.mounts.get(index).map(|m| matches!(m.kind, WeaponKind::Mortar | WeaponKind::Howitzer | WeaponKind::Rocket)).unwrap_or(false)
}

pub(crate) fn alive(s: &Sim, e: Entity) -> bool {
    crate::systems::alive(s, e)
}

pub(crate) fn name(s: &Sim, id: EntityId) -> String {
    crate::systems::name(s, id)
}

pub(crate) fn is_engineer(p: &Platform) -> bool {
    p.template.contains("engineer") || p.role.to_lowercase().contains("breach")
}

pub(crate) fn is_node(p: &Platform) -> bool {
    p.domain == Domain::Node
}

/// Raise a decision for an entity now, with the engine's rule for the rules
/// decider. False if it cannot be asked (dead, already deciding, no template).
pub(crate) fn raise(s: &mut Sim, id: EntityId, point: &'static str, about: Option<EntityId>, rule: Option<&str>) -> bool {
    let e = s.ctx.entity(id);
    if !alive(s, e) || s.world.get::<Act>(e).map(|a| a.deciding).unwrap_or(true) {
        return false;
    }
    let rule = rule.map(|r| RulesChoice::Option(r.to_string())).unwrap_or(RulesChoice::Fallback);
    crate::decide::ask_with_rule(s, &Trigger { entity: id, point, about }, rule)
}

/// Is it night where the theatre is?
pub(crate) fn is_night(s: &Sim, t: bgws_core::SimTime) -> bool {
    let o = s.ctx.frame.origin;
    let cloud = s.ctx.weather.sample(0.0, 0.0, t).cloud_cover;
    bgws_env::illuminance_at(&s.ctx.start.at(t), o.lat, o.lon, cloud, &s.ctx.p.light).total_lux < s.ctx.p.night_lux
}

/// Seconds from `now` until dark (0 if dark), searched in 15-minute steps up to a day.
pub(crate) fn secs_to_dark(s: &Sim) -> f64 {
    for k in 0..96 {
        let t = s.ctx.now + bgws_core::SimDuration::from_mins(15.0 * k as f64);
        if is_night(s, t) {
            return 900.0 * k as f64;
        }
    }
    86_400.0
}

/// Run `f` with the domain's resource taken out of the world (so `f` may use
/// the whole `Sim`), then put it back.
fn with<R>(s: &mut Sim, f: impl FnOnce(&mut Sim, &mut FiresWorld) -> R) -> Option<R> {
    let mut st = s.world.remove_resource::<FiresWorld>()?;
    let r = f(s, &mut st);
    s.world.insert_resource(st);
    Some(r)
}

// ── Hooks ────────────────────────────────────────────────────────────────

pub fn setup(s: &mut Sim) {
    let p = FParams::from_figures(&s.ctx.figures);
    let scope = &s.ctx.scenario.scope;
    let mut st = FiresWorld {
        fires_mode: scope.mode(ScopeDomain::Fires),
        logistics_mode: scope.mode(ScopeDomain::Logistics),
        medical_mode: scope.mode(ScopeDomain::Medical),
        fires: indirect::FiresState::new(&p),
        obstacles: obstacles::ObstacleState::new(&p),
        logistics: logistics::LogisticsState::default(),
        medical: medical::MedicalState::default(),
        p,
    };
    sustain::setup(s, &st);
    indirect::setup(s, &mut st);
    obstacles::setup(s, &mut st);
    logistics::setup(s, &mut st);
    medical::setup(s, &mut st);
    s.world.insert_resource(st);
}

pub fn fast_active(_s: &Sim) -> bool {
    // Rounds in flight land as scheduled events (§6.2), so the clock need
    // not run fast for them.
    false
}
pub fn air_active(_s: &Sim) -> bool {
    false
}
pub fn fast(_s: &mut Sim, _dt: f64) {}

pub fn base(s: &mut Sim, dt: f64) {
    with(s, |s, st| {
        medical::losses(s, st);
        obstacles::base(s, st, dt);
        sustain::base(s, st, dt);
        indirect::base(s, st);
        medical::base(s, st);
        logistics::base(s, st);
    });
}

pub fn slow(_s: &mut Sim, _dt: f64) {}

pub fn minute(s: &mut Sim, dt: f64) {
    with(s, |s, st| {
        sustain::minute(s, st, dt);
        logistics::minute(s, st, dt);
        medical::minute(s, st);
    });
}

pub fn script_event(s: &mut Sim, e: &bgws_scenario::v4::ScriptedEvent) {
    with(s, |s, st| logistics::script_event(s, st, e));
}

/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    let mine = match &ev {
        Scheduled::RoundsLand { mission } => indirect::owns(*mission),
        Scheduled::Delivery { flow } => logistics::owns(*flow),
        Scheduled::Timer { system, .. } => system.starts_with("fires."),
        _ => false,
    };
    if !mine {
        return Some(ev);
    }
    with(s, |s, st| match ev {
        Scheduled::RoundsLand { mission } => indirect::rounds_land(s, st, mission),
        Scheduled::Delivery { flow } => logistics::delivery(s, st, flow),
        Scheduled::Timer { system, key } if system == "fires.cb" => indirect::cb_fix(s, st, key),
        _ => {}
    });
    None
}

pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    if let (Some(st), Some(arms)) = (s.world.get::<Stores>(e), s.world.get::<Arms>(e)) {
        m.insert("agent.ammoPct".into(), FactValue::Num((st.ammo_pct(arms) * 100.0).round()));
        m.insert("agent.fuelPct".into(), FactValue::Num((st.fuel_pct() * 100.0).round()));
    }
    if let Some(g) = s.world.get::<Gun>(e) {
        let located = g.located || s.world.get_resource::<FiresWorld>().map(|w| g.fired_here >= w.p.located_after).unwrap_or(false);
        m.insert("threat.located".into(), FactValue::Bool(located));
    }
    if s.world.get::<Dig>(e).map(|d| d.done).unwrap_or(false) {
        m.insert("agent.inCover".into(), FactValue::Bool(true));
    }
    if let (Some(w), Some(i)) = (s.world.get_resource::<FiresWorld>(), s.world.get::<Ident>(e)) {
        indirect::mission_facts(s, w, i.id, m);
    }
}

pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    if let Some(i) = s.world.get::<Ident>(e) {
        if indirect::is_wlr(&i.platform) {
            c.insert(Capability::Radar);
            c.insert(Capability::Emitter);
        }
    }
}

pub fn apply_action(s: &mut Sim, e: Entity, id: EntityId, action: &str, params: &BTreeMap<String, serde_json::Value>, about: Option<EntityId>) -> bool {
    let _ = params;
    let domain = action.split('.').next().unwrap_or("");
    let handled = with(s, |s, st| match domain {
        "fires" => indirect::apply(s, st, e, id, action),
        "land" => obstacles::apply(s, st, e, id, action) || sustain::apply(s, st, e, id, action),
        "logistics" => logistics::apply(s, st, e, id, action),
        "medical" => medical::apply(s, st, e, id, action),
        _ => false,
    });
    let _ = about;
    handled.unwrap_or(false)
}

pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}
