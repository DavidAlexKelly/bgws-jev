//! Air, missiles and air defence (RUST_ENGINE.md §8.1 Air and Missiles,
//! §8.2 radar, §8.8 air defence, §18 phase 6), the sim's port of the TS
//! `world/domains/air.ts`, `world/munitions.ts` and `world/airspace.ts`:
//!
//! - [`craft`]: aircraft as point-mass 3-DoF bodies (`bgws_domains::air`)
//!   following their scenario task or the side's air tasking at 10 Hz, with
//!   fuel, bingo, sorties from bases, turnaround and the [P30] limits;
//! - [`flight`]: munitions in flight as entities on kinematic profiles
//!   (`bgws_domains::missile::Flight`) at 20 Hz: one-way drones, cruise and
//!   ballistic missiles, bombs and glide bombs, ASMs, ARMs, and the
//!   interceptors and air-to-air missiles; impacts with a CEP and damage at
//!   the aim point;
//! - [`radar`]: the radar range equation with RCS by aspect and band, the
//!   4/3-Earth horizon, terrain masking, clutter and the Doppler notch
//!   (`bgws_models::sensing::radar`), and IRST/EO, feeding each side's
//!   track store with air detections;
//! - [`defence`]: ground, ship and fighter air defence: the kill chain per
//!   track (track, classify, assign, launch, with times and odds), the
//!   side's weapon-target assignment every second (`bgws_domains::air_defence`),
//!   magazines, shoot-look-shoot, decoys that draw fire, and the cost
//!   exchange per side [R3, R4.4, P10, P11];
//! - [`airspace`]: control measures (altitude bands, corridors), fratricide
//!   risk in an active AD zone and the air-superiority helper;
//! - [`decisions`]: the `w.*` air decision points (missile warning, target in
//!   the envelope, bingo, threat ring, air track, ARM inbound) and the
//!   `air.*` actions that answer them, with facts, capabilities and gates.
//!
//! Domain state lives in the [`AirWorld`] resource and the [`Craft`],
//! [`AdUnit`] and [`InFlight`] components. Everything iterates in entity id
//! order; random draws come from keyed streams (§12).

pub mod airspace;
pub mod craft;
pub mod decisions;
pub mod defence;
pub mod flight;
pub mod params;
pub mod radar;

use crate::queue::Scheduled;
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_behaviour::{Capability, FactValue};
use bgws_catalogue::{Domain, MoverKind, Platform, WeaponKind};
use bgws_core::TrackId;
use bgws_core::{EntityId, Side, SimTime};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use airspace::{air_superiority, AirSuperiority};
pub use craft::{Craft, CraftStatus, MissionKind, Task};
pub use defence::{AdMount, AdUnit, Ledger, Policy};
pub use flight::{launch, InFlight, LaunchSpec, Purpose};
pub use params::{figures, AirParams};

/// Weapon control status (`air.holdFire`, `air.holdTight`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Wcs {
    #[default]
    Free,
    Tight,
    Hold,
}

/// Is a platform a one-way drone or a missile flown as a force (Shahed-like)?
pub fn is_munition_like(p: &Platform) -> bool {
    p.domain == Domain::Munition || (p.domain == Domain::Air && p.mover.kind == MoverKind::Missile)
}

/// Is a platform an aircraft this module flies: fixed-wing or rotary, crewed,
/// or an uncrewed one fast enough to fly like one (MALE ISR drones). Small
/// drones and loitering munitions are the drone domain's.
pub fn is_aircraft(p: &Platform) -> bool {
    p.domain == Domain::Air
        && matches!(p.mover.kind, MoverKind::FixedWing | MoverKind::Rotary)
        && (p.body.crew > 0 || p.mover.max_kmh >= 200.0)
}

/// Mount kinds that shoot at things in the air.
pub fn is_ad_mount(k: WeaponKind) -> bool {
    matches!(
        k,
        WeaponKind::SamLong
            | WeaponKind::SamMedium
            | WeaponKind::SamShort
            | WeaponKind::GunAd
            | WeaponKind::Ciws
            | WeaponKind::InterceptorDrone
    )
}

/// The kill chain of one side against one air track (§8.8) [R3].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chain {
    pub first_seen: SimTime,
    /// When the chain has run (track, classify, assign, launch); `None` if it failed.
    pub engageable_at: Option<SimTime>,
    /// What the defender thinks it is.
    pub class: bgws_domains::air_defence::ThreatClass,
    /// Recognised as a decoy: not engaged.
    pub decoy_recognised: bool,
    pub shots: u32,
    /// An interceptor is in the air at it, or its kill is being assessed, until then.
    pub look_until: Option<SimTime>,
    /// The truth behind the track (umpire's audit), for the engagement physics.
    pub truth: Option<EntityId>,
    pub classified: bool,
}

/// An emitter located by a side (ESM, RWR): a SEAD target.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fix {
    pub at: bgws_core::Enu,
    pub error_m: f64,
    pub time: SimTime,
}

/// What a pending AD or air question was about.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskContext {
    pub track: Option<TrackId>,
    pub target: Option<EntityId>,
    pub site: Option<bgws_core::Enu>,
    pub ring_m: f64,
}

/// The air domain's state (a resource in the ECS world).
#[derive(Resource, Clone, Debug)]
pub struct AirWorld {
    pub p: AirParams,
    /// Aircraft and munitions are simulated.
    pub air_on: bool,
    pub ad_on: bool,
    pub abstracted: bool,
    pub naval_simulated: bool,
    pub last_air_step: SimTime,
    pub next_tasking: SimTime,
    pub chains: BTreeMap<(Side, TrackId), Chain>,
    pub ledger: BTreeMap<Side, Ledger>,
    pub ledger_logged: BTreeMap<Side, (u32, u32)>,
    pub fixes: BTreeMap<Side, BTreeMap<EntityId, Fix>>,
    /// Structural hits taken by structures and ships.
    pub structural: BTreeMap<EntityId, f64>,
    /// Last time a question was raised about a (agent, about, kind).
    pub asked: BTreeMap<(EntityId, EntityId, u8), SimTime>,
    pub context: BTreeMap<EntityId, AskContext>,
    pub launches: u64,
    /// Own aircraft taken for hostile: (AD unit, aircraft) → when.
    pub misid: BTreeMap<(EntityId, EntityId), SimTime>,
    /// Empty launchers waiting to reload: (unit, mount) → since.
    pub reloads: BTreeMap<(EntityId, usize), SimTime>,
    /// Maintenance man-hours left today per base.
    pub mmh_left: BTreeMap<EntityId, (i64, f64, f64)>,
    /// The aircraft this module flies, in id order.
    pub crafts: Vec<EntityId>,
    /// Air-defence units, in id order.
    pub ad_units: Vec<EntityId>,
    /// Munitions in flight (alive), in id order.
    pub flights: Vec<EntityId>,
    /// Something fast is in flight (20 Hz).
    pub fast_live: bool,
    /// Something is airborne (10 Hz).
    pub air_live: bool,
}

impl AirWorld {
    fn new(s: &Sim) -> Self {
        let mode = |d| s.ctx.scenario.scope.mode(d);
        AirWorld {
            p: AirParams::from_figures(&s.ctx.figures),
            air_on: mode(ScopeDomain::Air) != DomainMode::Off,
            ad_on: mode(ScopeDomain::AirDefence) != DomainMode::Off,
            abstracted: mode(ScopeDomain::Air) == DomainMode::Abstracted,
            naval_simulated: mode(ScopeDomain::Naval) == DomainMode::Simulated,
            last_air_step: SimTime::ZERO,
            next_tasking: SimTime::from_secs(1.0),
            chains: BTreeMap::new(),
            ledger: BTreeMap::new(),
            ledger_logged: BTreeMap::new(),
            fixes: BTreeMap::new(),
            structural: BTreeMap::new(),
            asked: BTreeMap::new(),
            context: BTreeMap::new(),
            launches: 0,
            misid: BTreeMap::new(),
            reloads: BTreeMap::new(),
            mmh_left: BTreeMap::new(),
            crafts: Vec::new(),
            ad_units: Vec::new(),
            flights: Vec::new(),
            fast_live: false,
            air_live: false,
        }
    }
}

/// Run `f` with the domain state taken out of the world (so systems can
/// borrow components freely), then put it back. Does nothing before setup.
pub(crate) fn with<R: Default>(s: &mut Sim, f: impl FnOnce(&mut Sim, &mut AirWorld) -> R) -> R {
    let Some(mut aw) = s.world.remove_resource::<AirWorld>() else {
        return R::default();
    };
    let r = f(s, &mut aw);
    s.world.insert_resource(aw);
    r
}

/// The domain state, read-only.
pub fn state(s: &Sim) -> Option<&AirWorld> {
    s.world.get_resource::<AirWorld>()
}

pub fn setup(s: &mut Sim) {
    let aw = AirWorld::new(s);
    if !aw.air_on && !aw.ad_on {
        return;
    }
    s.world.insert_resource(aw);
    with(s, |s, aw| {
        craft::setup(s, aw);
        defence::setup(s, aw);
        flight::setup(s, aw);
        refresh(s, aw);
    });
}

/// Munitions or interceptors in flight: the 20 Hz rate.
pub fn fast_active(s: &Sim) -> bool {
    state(s).map(|a| a.fast_live).unwrap_or(false)
}

/// Aircraft airborne or anything of ours flying: the 10 Hz rate.
pub fn air_active(s: &Sim) -> bool {
    state(s).map(|a| a.air_live || a.fast_live).unwrap_or(false)
}

/// Recompute the rate flags after a step.
pub(crate) fn refresh(s: &Sim, aw: &mut AirWorld) {
    aw.flights.retain(|id| {
        s.world
            .get::<InFlight>(s.ctx.entity(*id))
            .map(|f| f.alive)
            .unwrap_or(false)
    });
    aw.fast_live = aw.flights.iter().any(|id| {
        s.world
            .get::<InFlight>(s.ctx.entity(*id))
            .map(|f| f.fast)
            .unwrap_or(false)
    });
    aw.air_live = !aw.flights.is_empty()
        || aw.crafts.iter().any(|id| {
            s.world
                .get::<Craft>(s.ctx.entity(*id))
                .map(|c| c.status == CraftStatus::Airborne)
                .unwrap_or(false)
        });
}

pub fn fast(s: &mut Sim, _dt: f64) {
    with(s, |s, aw| {
        flight::step(s, aw);
        if crate::clock::Rate::Air.due(s.ctx.now) {
            let dt_air = (s.ctx.now - aw.last_air_step).secs().clamp(0.0, 1.0);
            aw.last_air_step = s.ctx.now;
            if dt_air > 0.0 {
                craft::step(s, aw, dt_air);
            }
        }
        refresh(s, aw);
    })
}

pub fn base(s: &mut Sim, _dt: f64) {
    with(s, |s, aw| {
        if aw.air_on {
            craft::tasking(s, aw);
            craft::launches(s, aw);
            craft::checks(s, aw);
        }
        radar::sense(s, aw);
        if aw.ad_on {
            defence::cycle(s, aw);
        }
        if aw.air_on {
            decisions::air_to_air(s, aw);
        }
        airspace::control(s, aw);
        refresh(s, aw);
    })
}

pub fn slow(s: &mut Sim, _dt: f64) {
    with(s, |s, aw| {
        radar::esm_fixes(s, aw);
        defence::reload(s, aw);
    })
}

pub fn minute(s: &mut Sim, _dt: f64) {
    with(s, defence::log_ledger)
}

pub fn script_event(_s: &mut Sim, _e: &bgws_scenario::v4::ScriptedEvent) {}

/// Handle a scheduled event; return it back if it is not this domain's.
pub fn scheduled(s: &mut Sim, ev: Scheduled) -> Option<Scheduled> {
    match ev {
        Scheduled::Timer { ref system, key } if system == "air.radarOn" => {
            with(s, |s, _| defence::radar_on(s, EntityId(key as u32)));
            None
        }
        other => Some(other),
    }
}

pub fn facts(s: &Sim, e: Entity, m: &mut BTreeMap<String, FactValue>) {
    decisions::facts(s, e, m)
}

pub fn capabilities(s: &Sim, e: Entity, c: &mut BTreeSet<Capability>) {
    decisions::capabilities(s, e, c)
}

pub fn apply_action(
    s: &mut Sim,
    e: Entity,
    id: EntityId,
    action: &str,
    params: &BTreeMap<String, serde_json::Value>,
    about: Option<EntityId>,
) -> bool {
    if !action.starts_with("air.") {
        return false;
    }
    with(s, |s, aw| {
        decisions::apply(s, aw, e, id, action, params, about)
    })
}

/// World victory conditions involving air are the LOD module's; see
/// [`air_superiority`] for the helper it can call.
pub fn victory(_s: &Sim) -> Option<(Option<Side>, String)> {
    None
}

/// The gates a decision about an air or AD agent passes through: the
/// weapon-release status and EMCON of the agent (rule 5).
pub fn gates(s: &Sim, e: Entity, about: Option<EntityId>, g: &mut bgws_behaviour::Gates) {
    decisions::gates(s, e, about, g)
}

/// Does this entity count towards a side's combat strength? One-way drones
/// and missiles flown as forces do not: they are expended by design.
pub fn counts_for_strength(p: &Platform) -> bool {
    !is_munition_like(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Figures;

    fn catalogue() -> bgws_catalogue::Catalogue {
        bgws_catalogue::Catalogue::builtin_with(&crate::default_figures()).unwrap()
    }

    #[test]
    fn figures_register_and_build_params() {
        let f = Figures::from_list(crate::all_figures()).unwrap();
        let p = AirParams::from_figures(&f);
        assert!(p.chain.reaction() > 0.0 && p.tasking_s > 0.0);
        let sr = bgws_domains::air::sortie_rate(&p.sortie);
        assert!((2.0..=3.0).contains(&sr.per_aircraft_per_day), "{sr:?}");
    }

    #[test]
    fn who_flies_what() {
        let c = catalogue();
        let p = |id: &str| c.platform(id).unwrap().clone();
        assert!(
            is_aircraft(&p("generic/fighter4"))
                && is_aircraft(&p("generic/attackHelo"))
                && is_aircraft(&p("generic/male"))
        );
        assert!(
            !is_aircraft(&p("generic/quadIsr"))
                && !is_aircraft(&p("generic/loiter"))
                && !is_aircraft(&p("generic/owaDrone"))
        );
        assert!(
            is_munition_like(&p("generic/owaDrone")) && is_munition_like(&p("generic/owaDecoy"))
        );
        assert!(
            !counts_for_strength(&p("generic/owaDrone"))
                && counts_for_strength(&p("generic/samMedium"))
        );
        assert!(p("generic/samMedium")
            .mounts
            .iter()
            .any(|m| is_ad_mount(m.kind)));
    }

    #[test]
    fn energy_model_records_from_the_catalogue() {
        let c = catalogue();
        let f = craft::perf_for(c.platform("generic/fighter4").unwrap()).unwrap();
        let ap = bgws_domains::air::AirParams::from_figures(&crate::default_figures());
        let m = f.empty_mass + 0.5 * f.fuel_capacity;
        assert!(f.max_level_speed(&ap, 11_000.0, m) > 400.0);
        assert!((50.0..120.0).contains(&f.stall_speed(0.0, m)));
        // Helicopters and slow drones fly kinematically.
        assert!(craft::perf_for(c.platform("generic/attackHelo").unwrap()).is_none());
        assert!(craft::perf_for(c.platform("generic/male").unwrap()).is_none());
    }

    #[test]
    fn cost_exchange_and_segments() {
        let l = Ledger {
            interceptor_cost: 4.0e6,
            value_killed: 2.0e5,
            ..Default::default()
        };
        assert!((l.cost_exchange() - 20.0).abs() < 1e-9);
        assert!(Ledger::default().cost_exchange().is_infinite());
        let d = craft::seg_dist(
            bgws_core::Enu::new(5.0, 3.0, 0.0),
            bgws_core::Enu::new(0.0, 0.0, 0.0),
            bgws_core::Enu::new(10.0, 0.0, 0.0),
        );
        assert!((d - 3.0).abs() < 1e-9);
    }
}
