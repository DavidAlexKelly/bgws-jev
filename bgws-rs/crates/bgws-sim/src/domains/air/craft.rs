//! Aircraft (§8.1 Air): flights of two to four aircraft as one entity each,
//! flown as a point mass with the energy model of `bgws_domains::air`
//! (jets) or kinematically (helicopters and slow drones), following their
//! scenario task or the side's air tasking (a light ATO, as the TS
//! `airTasking`): SEAD on located air-defence radars, CAP over the front,
//! strikes on the side's named areas of interest, AEW and tanker tracks.
//!
//! Sorties come from bases. A landed flight launches when it is ready, its
//! base still has a runway, the day's flying window is open and the base
//! has crews and maintenance hours left; it lands to a turnaround, sometimes
//! broken with a fix time drawn from the [P30] figures, so the sortie rate
//! per aircraft per day emerges from the same limits as
//! `bgws_domains::air::sortie_rate`.

use super::flight::{self, LaunchSpec, Purpose};
use super::{is_aircraft, AirWorld, AskContext, Wcs};
use crate::components::*;
use crate::sim::{Sim, Trigger};
use crate::systems::{alive, name};
use bevy_ecs::prelude::*;
use bgws_catalogue::{Domain, MoverKind, Platform, WeaponKind};
use bgws_core::{EntityId, Enu, Side, SimDuration, SimTime};
use bgws_domains::air::{
    intercept, orbit_command, samples, AirCommand, AirState, AircraftPerf, SortieLimit,
};
use bgws_domains::atmosphere::isa;
use bgws_domains::vec3::{wrap_pi, V3};
use rand::Rng;
use rand_distr::{Distribution, Exp};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CraftStatus {
    Landed,
    Airborne,
    Lost,
}

/// What a flight was tasked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MissionKind {
    /// The scenario's own task.
    Scripted,
    Cap,
    Strike,
    Sead,
    Aew,
    Tanker,
    Isr,
}

impl MissionKind {
    pub fn word(self) -> &'static str {
        match self {
            MissionKind::Scripted => "its task",
            MissionKind::Cap => "CAP",
            MissionKind::Strike => "strike",
            MissionKind::Sead => "SEAD",
            MissionKind::Aew => "AEW track",
            MissionKind::Tanker => "tanker track",
            MissionKind::Isr => "ISR orbit",
        }
    }
}

/// The flight's task in the theatre frame (the scenario `EntityTask`, resolved).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Task {
    Idle,
    Route {
        wps: Vec<Enu>,
        alt: f64,
        then: Option<Box<Task>>,
    },
    Orbit {
        at: Enu,
        radius: f64,
        alt: f64,
        until: Option<SimTime>,
    },
    Strike {
        target: Option<EntityId>,
        aim: Enu,
        alt: f64,
    },
    Intercept {
        target: EntityId,
    },
    Rtb,
    Station {
        at: Enu,
        radius: f64,
        alt: f64,
    },
    Search {
        at: Enu,
        radius: f64,
        alt: f64,
        until: SimTime,
    },
}

/// An aircraft flight's air state.
#[derive(Component, Clone, Debug)]
pub struct Craft {
    /// The energy model's record; `None` flies kinematically.
    pub perf: Option<Arc<AircraftPerf>>,
    /// Position (x east, y north, z altitude), speed, heading, flight path, fuel.
    pub st: AirState,
    pub fuel_cap: f64,
    pub status: CraftStatus,
    pub base: Option<EntityId>,
    pub task: Task,
    pub mission: Option<MissionKind>,
    pub until: Option<SimTime>,
    pub ready_at: SimTime,
    pub sorties: u32,
    pub took_off: Option<SimTime>,
    pub flight_s: f64,
    pub wcs: Wcs,
    pub defending_until: SimTime,
    pub low: bool,
    pub winchester: bool,
    pub bingo_asked: bool,
    pub ring_asked: bool,
    pub via: Option<Enu>,
    pub day: i64,
    pub day_up: bool,
    pub cruise: f64,
    pub max_speed: f64,
    pub climb: f64,
    pub turn_rate: f64,
    pub ceiling: f64,
    pub burn_lph: f64,
    pub rotary: bool,
}

impl Craft {
    pub fn fuel_share(&self) -> f64 {
        if self.fuel_cap > 0.0 {
            self.st.fuel / self.fuel_cap
        } else {
            1.0
        }
    }
    pub fn airborne(&self) -> bool {
        self.status == CraftStatus::Airborne
    }
}

fn v3(p: Enu) -> V3 {
    V3::new(p.e, p.n, p.u)
}

/// The energy model's record for a catalogue platform: the nearest sample
/// scaled to its mass, speed, ceiling and fuel (the catalogue gives the
/// envelope, not the drag polar). Helicopters and slow drones fly kinematically.
pub fn perf_for(p: &Platform) -> Option<AircraftPerf> {
    if p.mover.kind != MoverKind::FixedWing || p.mover.max_kmh < 400.0 {
        return None;
    }
    let mut a = if p.mover.max_kmh > 1_000.0 {
        samples::fighter()
    } else {
        samples::transport()
    };
    let mass = if p.body.mass_t > 0.0 {
        p.body.mass_t * 1000.0 * 0.6
    } else {
        a.empty_mass
    };
    let k = mass / a.empty_mass;
    a.name = p.id.clone();
    a.empty_mass = mass;
    a.wing_area *= k;
    a.max_thrust_sl *= k;
    if p.mover.fuel_capacity_l > 0.0 {
        a.fuel_capacity = p.mover.fuel_capacity_l * 0.8;
    }
    a.mach_max = a.mach_max.min(p.mover.max_kmh / 3.6 / 295.0).max(0.6);
    if p.mover.ceiling_m > 0.0 {
        a.ceiling = p.mover.ceiling_m;
    }
    Some(a)
}

fn ground_at(s: &Sim, e: f64, n: f64) -> f64 {
    s.ctx.ground.height(e, n)
}

/// A fresh craft component for a platform at a position.
fn new_craft(
    p: &Platform,
    pos: Enu,
    alt: f64,
    heading: f64,
    status: CraftStatus,
    base: Option<EntityId>,
) -> Craft {
    let perf = perf_for(p).map(Arc::new);
    let cruise = (p.mover.cruise_kmh.max(50.0)) / 3.6;
    let fuel_cap = perf
        .as_ref()
        .map(|a| a.fuel_capacity)
        .unwrap_or(p.mover.fuel_capacity_l.max(1.0));
    Craft {
        perf,
        st: AirState {
            pos: V3::new(pos.e, pos.n, alt),
            speed: if status == CraftStatus::Airborne {
                cruise
            } else {
                0.0
            },
            heading,
            gamma: 0.0,
            fuel: fuel_cap,
            stores: 0.0,
        },
        fuel_cap,
        status,
        base,
        task: Task::Idle,
        mission: None,
        until: None,
        ready_at: SimTime::ZERO,
        sorties: 0,
        took_off: if status == CraftStatus::Airborne {
            Some(SimTime::ZERO)
        } else {
            None
        },
        flight_s: 0.0,
        wcs: Wcs::Free,
        defending_until: SimTime::from_secs(-1.0),
        low: false,
        winchester: false,
        bingo_asked: false,
        ring_asked: false,
        via: None,
        day: -1,
        day_up: true,
        cruise,
        max_speed: p.mover.max_kmh.max(p.mover.cruise_kmh) / 3.6,
        climb: if p.mover.climb_ms > 0.0 {
            p.mover.climb_ms
        } else {
            10.0
        },
        turn_rate: if p.mover.turn_rate_dps > 0.0 {
            p.mover.turn_rate_dps.to_radians()
        } else {
            0.1
        },
        ceiling: if p.mover.ceiling_m > 0.0 {
            p.mover.ceiling_m
        } else {
            10_000.0
        },
        burn_lph: p.mover.burn_moving_lph,
        rotary: p.mover.kind == MoverKind::Rotary,
    }
}

/// Resolve a scenario task into the frame.
fn task_of(
    s: &Sim,
    t: &bgws_scenario::v4::EntityTask,
    default_alt: f64,
) -> (Task, Option<SimTime>) {
    use bgws_scenario::v4::EntityTask as T;
    let enu = |g: bgws_core::Geodetic| {
        let v = s.ctx.frame.geodetic_to_enu(g);
        Enu::new(v.e, v.n, 0.0)
    };
    match t {
        T::Idle | T::Landed { .. } | T::Deliver { .. } | T::Reload { .. } => (Task::Idle, None),
        T::Route {
            waypoints,
            alt_m,
            then,
        } => {
            let next = then
                .as_ref()
                .map(|b| Box::new(task_of(s, b, default_alt).0));
            (
                Task::Route {
                    wps: waypoints.iter().map(|g| enu(*g)).collect(),
                    alt: alt_m.unwrap_or(default_alt),
                    then: next,
                },
                None,
            )
        }
        T::Orbit {
            at,
            radius_m,
            alt_m,
            until,
        } => {
            let u = until.map(SimTime::from_secs);
            (
                Task::Orbit {
                    at: enu(*at),
                    radius: *radius_m,
                    alt: *alt_m,
                    until: u,
                },
                u,
            )
        }
        T::Strike {
            target_id,
            aim_at,
            alt_m,
        } => (
            Task::Strike {
                target: target_id
                    .as_ref()
                    .and_then(|t| s.ctx.by_name.get(t).copied()),
                aim: enu(*aim_at),
                alt: alt_m.unwrap_or(default_alt),
            },
            None,
        ),
        T::Intercept { target_id } => match s.ctx.by_name.get(target_id) {
            Some(t) => (Task::Intercept { target: *t }, None),
            None => (Task::Idle, None),
        },
        T::ReturnToBase { .. } => (Task::Rtb, None),
        T::Station { at, radius_m } => (
            Task::Station {
                at: enu(*at),
                radius: *radius_m,
                alt: default_alt,
            },
            None,
        ),
        T::LoiterSearch {
            at,
            radius_m,
            until,
        } => {
            let u = SimTime::from_secs(*until);
            (
                Task::Search {
                    at: enu(*at),
                    radius: *radius_m,
                    alt: default_alt,
                    until: u,
                },
                Some(u),
            )
        }
    }
}

/// Aircraft forces get their craft state; bases spawn their flights.
pub fn setup(s: &mut Sim, aw: &mut AirWorld) {
    if !aw.air_on {
        return;
    }
    // Bases with carried aircraft and no flights of their own in the scenario.
    let forces = s.ctx.scenario.forces.clone();
    for f in &forces {
        let Some(&base_id) = s.ctx.by_name.get(&f.id) else {
            continue;
        };
        let platform = s
            .world
            .get::<Ident>(s.ctx.entity(base_id))
            .map(|i| i.platform.clone());
        let Some(platform) = platform else { continue };
        let Some(carried) = platform.carried.aircraft.clone() else {
            continue;
        };
        if !matches!(platform.domain, Domain::Node | Domain::Surface) {
            continue;
        }
        if forces.iter().any(|o| {
            o.parent.as_deref() == Some(f.id.as_str())
                && s.ctx
                    .catalogue
                    .platform(&o.platform)
                    .map(is_aircraft)
                    .unwrap_or(false)
        }) {
            continue;
        }
        let Some(rec) = s.ctx.catalogue.platform(&carried.archetype).cloned() else {
            continue;
        };
        let per = rec.body.count.max(1);
        let n = carried.count / per;
        for i in 0..n {
            let force = bgws_scenario::v4::Force {
                id: format!("{}-flight-{}", f.id, i + 1),
                label: format!("{} flight {}", f.label, i + 1),
                side: f.side,
                platform: rec.id.clone(),
                count: per,
                quality: f.quality,
                position: bgws_core::Geodetic::new(f.position.lat, f.position.lon, 0.0),
                heading_deg: f.heading_deg,
                layer: bgws_scenario::v4::Layer::Entity,
                echelon: None,
                concealed: false,
                command_rating: None,
                dummy: false,
                parent: Some(f.id.clone()),
                task: None,
                orbit: None,
                provider: None,
                lane: vec![],
                status: Some("landed".into()),
            };
            let id = EntityId(s.ctx.order.len() as u32);
            let e = crate::sim::spawn_force(
                &mut s.world,
                &mut s.ctx,
                id,
                &force,
                Arc::new(rec.clone()),
            );
            s.ctx.order.push(e);
            s.ctx.by_name.insert(force.id.clone(), id);
        }
        let text = format!("{} holds {} flights of {}", f.label, n, rec.name);
        s.ctx
            .log
            .push(SimTime::ZERO, "air.base")
            .side(f.side)
            .entity(base_id)
            .text(text)
            .data(serde_json::json!({ "flights": n, "perFlight": per, "archetype": rec.id }));
    }
    // Every aircraft entity: its craft state.
    let order = s.ctx.order.clone();
    for (idx, &e) in order.iter().enumerate() {
        let Some(i) = s.world.get::<Ident>(e) else {
            continue;
        };
        if !is_aircraft(&i.platform) {
            continue;
        }
        let platform = i.platform.clone();
        let id = EntityId(idx as u32);
        let force = s.ctx.scenario.forces.get(idx).cloned();
        let spawned_parent = s
            .ctx
            .by_name
            .iter()
            .find(|(_, v)| **v == id)
            .and_then(|(k, _)| k.rsplit_once("-flight-").map(|(b, _)| b.to_string()));
        let parent = force
            .as_ref()
            .and_then(|f| f.parent.clone())
            .or(spawned_parent);
        let mut base = parent.as_ref().and_then(|p| s.ctx.by_name.get(p).copied());
        let task = force.as_ref().and_then(|f| f.task.clone());
        if let Some(
            bgws_scenario::v4::EntityTask::Landed { base_id, .. }
            | bgws_scenario::v4::EntityTask::ReturnToBase { base_id },
        ) = &task
        {
            base = s.ctx.by_name.get(base_id).copied().or(base);
        }
        let landed = force
            .as_ref()
            .map(|f| f.status.as_deref() == Some("landed"))
            .unwrap_or(true)
            || matches!(task, Some(bgws_scenario::v4::EntityTask::Landed { .. }))
            || (task.is_none()
                && base.is_some()
                && force
                    .as_ref()
                    .map(|f| f.status.as_deref() != Some("active"))
                    .unwrap_or(true));
        let k = *s.world.get::<Kin>(e).expect("kin");
        let (pos, alt, status) = if landed {
            let bp = base
                .and_then(|b| s.world.get::<Kin>(s.ctx.entity(b)).map(|k| k.pos))
                .unwrap_or(k.pos);
            let g = ground_at(s, bp.e, bp.n);
            (Enu::new(bp.e, bp.n, g), g, CraftStatus::Landed)
        } else {
            let g = ground_at(s, k.pos.e, k.pos.n);
            let alt = g + k.agl.max(if platform.mover.kind == MoverKind::Rotary {
                aw.p.helo_alt
            } else {
                1_000.0
            });
            (Enu::new(k.pos.e, k.pos.n, alt), alt, CraftStatus::Airborne)
        };
        let mut c = new_craft(&platform, pos, alt, k.heading, status, base);
        let default_alt = if c.rotary {
            aw.p.helo_alt
        } else {
            aw.p.strike_alt
        };
        if let Some(t) = &task {
            let (tk, until) = task_of(s, t, default_alt);
            if tk != Task::Idle {
                c.mission = Some(MissionKind::Scripted);
                c.until = until;
            }
            c.task = tk;
            if let bgws_scenario::v4::EntityTask::Landed { ready_at, .. } = t {
                c.ready_at = SimTime::from_secs(*ready_at);
            }
        }
        {
            let mut km = s.world.get_mut::<Kin>(e).expect("kin");
            km.pos = Enu::new(pos.e, pos.n, alt);
            km.agl = alt - ground_at_pos(&s.ctx.ground, pos);
        }
        s.world.get_mut::<Act>(e).expect("act").activity = Activity::Hold;
        s.world.entity_mut(e).insert(c);
        aw.crafts.push(id);
    }
}

fn ground_at_pos(g: &crate::ground::Ground, p: Enu) -> f64 {
    g.height(p.e, p.n)
}

/// Home and the enemy as briefed: own base (or forces) and the enemy's start centroid.
fn home_and_front(s: &Sim, side: Side, base: Option<EntityId>) -> (Enu, Enu) {
    let centroid = |sd: Side| {
        let v: Vec<Enu> = s
            .ctx
            .scenario
            .forces_of(sd)
            .map(|f| s.ctx.frame.geodetic_to_enu(f.position))
            .collect();
        if v.is_empty() {
            Enu::default()
        } else {
            let n = v.len() as f64;
            let sum = v.iter().fold(Enu::default(), |a, b| a.add(*b));
            Enu::new(sum.e / n, sum.n / n, 0.0)
        }
    };
    let home = base
        .and_then(|b| {
            s.world
                .get::<Kin>(s.ctx.entity(b))
                .map(|k| Enu::new(k.pos.e, k.pos.n, 0.0))
        })
        .unwrap_or_else(|| centroid(side));
    (home, centroid(side.enemy()))
}

/// Best munition of these kinds a flight still carries: (mount index, munition).
pub(crate) fn mount_of(
    s: &Sim,
    e: Entity,
    kinds: &[WeaponKind],
) -> Option<(usize, bgws_catalogue::Munition, u32)> {
    let i = s.world.get::<Ident>(e)?;
    let arms = s.world.get::<Arms>(e)?;
    let mut best: Option<(usize, bgws_catalogue::Munition, u32)> = None;
    for m in &arms.mounts {
        let cm = i.platform.mounts.get(m.index)?;
        if m.rounds == 0 || !kinds.contains(&cm.kind) {
            continue;
        }
        let Some(mun) = s.ctx.catalogue.munition(&m.munition) else {
            continue;
        };
        if best
            .as_ref()
            .map(|b| mun.max_range_m > b.1.max_range_m)
            .unwrap_or(true)
        {
            best = Some((m.index, mun.clone(), m.rounds));
        }
    }
    best
}

fn has_kind(s: &Sim, e: Entity, kinds: &[WeaponKind]) -> bool {
    mount_of(s, e, kinds).is_some()
}

const STRIKE_KINDS: [WeaponKind; 6] = [
    WeaponKind::GlideBomb,
    WeaponKind::Bomb,
    WeaponKind::Agm,
    WeaponKind::Cruise,
    WeaponKind::Ascm,
    WeaponKind::Arm,
];

/// The light ATO: idle flights get missions on the tasking cycle.
pub fn tasking(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    if now < aw.next_tasking {
        return;
    }
    aw.next_tasking = now + SimDuration::from_secs(aw.p.tasking_s);
    for side in Side::PLAYERS {
        let mut idle: Vec<(EntityId, Entity)> = aw
            .crafts
            .iter()
            .map(|id| (*id, s.ctx.entity(*id)))
            .filter(|(_, e)| alive(s, *e))
            .filter(|(_, e)| {
                let i = s.world.get::<Ident>(*e).expect("i");
                let c = s.world.get::<Craft>(*e).expect("c");
                i.side == side && c.status != CraftStatus::Lost && c.task == Task::Idle
            })
            .collect();
        if idle.is_empty() {
            continue;
        }
        let base0 = idle
            .first()
            .and_then(|(_, e)| s.world.get::<Craft>(*e).and_then(|c| c.base));
        let (home, front) = home_and_front(s, side, base0);
        let toward = front.sub(home);
        let assign =
            |s: &mut Sim, id: EntityId, e: Entity, kind: MissionKind, task: Task, why: String| {
                let label = name(s, id);
                let side = s.world.get::<Ident>(e).expect("i").side;
                let mut c = s.world.get_mut::<Craft>(e).expect("c");
                c.task = task;
                c.mission = Some(kind);
                c.until = None;
                c.winchester = false;
                c.ring_asked = false;
                c.bingo_asked = false;
                c.via = None;
                s.ctx
                    .log
                    .push(now, "air.tasked")
                    .side(side)
                    .entity(id)
                    .text(format!("{label} tasked: {} {why}", kind.word()))
                    .data(serde_json::json!({ "mission": kind }));
            };
        let take = |s: &Sim,
                    idle: &mut Vec<(EntityId, Entity)>,
                    pred: &dyn Fn(&Sim, Entity) -> bool|
         -> Option<(EntityId, Entity)> {
            let i = idle.iter().position(|(_, e)| pred(s, *e))?;
            Some(idle.remove(i))
        };
        let on_target = |s: &Sim, t: EntityId| {
            aw.crafts.iter().any(|c| {
                s.world
                    .get::<Craft>(s.ctx.entity(*c))
                    .map(|c| matches!(c.task, Task::Strike { target: Some(x), .. } if x == t))
                    .unwrap_or(false)
            })
        };
        // SEAD on located air-defence radars first.
        let fixes: Vec<(EntityId, Enu)> = aw
            .fixes
            .get(&side)
            .map(|m| m.iter().map(|(k, v)| (*k, v.at)).collect())
            .unwrap_or_default();
        for (site, at) in fixes.into_iter().take(2) {
            if on_target(s, site) {
                continue;
            }
            let Some((id, e)) = take(s, &mut idle, &|s, e| has_kind(s, e, &[WeaponKind::Arm]))
            else {
                break;
            };
            let alt = aw.p.strike_alt;
            assign(
                s,
                id,
                e,
                MissionKind::Sead,
                Task::Strike {
                    target: Some(site),
                    aim: at,
                    alt,
                },
                format!(
                    "on the radar at {:.0} km",
                    at.sub(home).horizontal() / 1000.0
                ),
            );
        }
        // CAP over the front when the enemy has anything that flies.
        let enemy_flies = s.ctx.scenario.forces_of(side.enemy()).any(|f| {
            s.ctx
                .catalogue
                .platform(&f.platform)
                .map(|p| p.domain == Domain::Air || p.carried.aircraft.is_some())
                .unwrap_or(false)
        });
        if enemy_flies {
            if let Some((id, e)) = take(s, &mut idle, &|s, e| has_kind(s, e, &[WeaponKind::Aam])) {
                let at = home.add(toward.scale(aw.p.cap_depth));
                let task = Task::Orbit {
                    at,
                    radius: aw.p.cap_radius,
                    alt: aw.p.cap_alt,
                    until: None,
                };
                assign(s, id, e, MissionKind::Cap, task, "over the front".into());
            }
        }
        // Strikes on the side's named areas of interest, highest priority first.
        let mut nais: Vec<(f64, String, Enu)> = s
            .ctx
            .scenario
            .world
            .nais
            .iter()
            .filter(|n| n.side == side)
            .map(|n| {
                (
                    n.priority,
                    n.label.clone(),
                    s.ctx.frame.geodetic_to_enu(n.at),
                )
            })
            .collect();
        nais.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, label, at) in nais.into_iter().take(2) {
            let busy = aw.crafts.iter().any(|c| s.world.get::<Craft>(s.ctx.entity(*c)).map(|c| matches!(c.task, Task::Strike { target: None, aim, .. } if aim.sub(at).horizontal() < 100.0)).unwrap_or(false));
            if busy {
                continue;
            }
            let Some((id, e)) = take(s, &mut idle, &|s, e| {
                has_kind(
                    s,
                    e,
                    &[
                        WeaponKind::GlideBomb,
                        WeaponKind::Bomb,
                        WeaponKind::Agm,
                        WeaponKind::Cruise,
                    ],
                )
            }) else {
                break;
            };
            let alt = aw.p.strike_alt;
            assign(
                s,
                id,
                e,
                MissionKind::Strike,
                Task::Strike {
                    target: None,
                    aim: Enu::new(at.e, at.n, 0.0),
                    alt,
                },
                format!("on {label}"),
            );
        }
        // AEW and tankers to their tracks.
        for (role, kind) in [("aew", MissionKind::Aew), ("tanker", MissionKind::Tanker)] {
            if let Some((id, e)) = take(s, &mut idle, &|s, e| {
                s.world
                    .get::<Ident>(e)
                    .map(|i| i.platform.id.contains(role))
                    .unwrap_or(false)
            }) {
                let at = home.add(toward.scale(aw.p.station_share));
                assign(
                    s,
                    id,
                    e,
                    kind,
                    Task::Station {
                        at,
                        radius: 20_000.0,
                        alt: 9_000.0,
                    },
                    "on its track".into(),
                );
            }
        }
    }
}

/// Is the day's flying possible for this flight now? Rolls the day's
/// serviceability and refreshes the base's crew and maintenance budgets.
fn may_launch(
    s: &Sim,
    aw: &mut AirWorld,
    id: EntityId,
    c: &mut Craft,
    count: u32,
) -> Result<(), &'static str> {
    let now = s.ctx.now.secs();
    let day = (now / 86_400.0).floor() as i64;
    let sp = aw.p.sortie;
    if c.day != day {
        c.day = day;
        let mut rng = s.ctx.rng("air.day", id.0 as u64, day as u64);
        c.day_up = rng.gen::<f64>() < sp.mission_capable;
    }
    if !c.day_up {
        return Err("not mission capable today");
    }
    let into_day = now - day as f64 * 86_400.0;
    if (into_day + sp.sortie_h * 3600.0) > sp.ops_window_h * 3600.0 {
        return Err("outside the flying window");
    }
    let Some(base) = c.base else { return Ok(()) };
    // Base budgets per day: crews and maintenance man-hours for its aircraft.
    let at_base: u32 = aw
        .crafts
        .iter()
        .filter_map(|x| {
            s.world
                .get::<Craft>(s.ctx.entity(*x))
                .filter(|o| o.base == Some(base))
                .and_then(|_| s.world.get::<Health>(s.ctx.entity(*x)).map(|h| h.fit))
        })
        .sum::<u32>()
        + if s.world.get::<Craft>(s.ctx.entity(id)).is_none() {
            count
        } else {
            0
        };
    let rate = bgws_domains::air::sortie_rate(&sp);
    let mmh_per = sp.mmh_per_fh * sp.sortie_h * (1.0 - sp.maintenance_deferred);
    let entry = aw.mmh_left.entry(base).or_insert((-1, 0.0, 0.0));
    if entry.0 != day {
        let n = at_base.max(count) as f64;
        *entry = (
            day,
            sp.maintainers_per_aircraft
                * sp.maintenance_shift_cover_h
                * sp.maintenance_productive
                * n,
            (rate.aircrew_limit * n).floor(),
        );
    }
    if entry.2 < count as f64 {
        return Err(match rate.binding {
            SortieLimit::Aircrew => "no rested crews left today",
            _ => "no crews left today",
        });
    }
    if entry.1 < mmh_per * count as f64 {
        return Err("no maintenance hours left today");
    }
    entry.1 -= mmh_per * count as f64;
    entry.2 -= count as f64;
    Ok(())
}

/// Tasked flights on the ground take off when they can.
pub fn launches(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    for id in aw.crafts.clone() {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let mut c = s.world.get::<Craft>(e).expect("c").clone();
        if c.status != CraftStatus::Landed || c.task == Task::Idle || c.ready_at > now {
            continue;
        }
        if let Some(b) = c.base {
            if !alive(s, s.ctx.entity(b)) {
                continue;
            }
        }
        let count = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
        if may_launch(s, aw, id, &mut c, count).is_err() {
            // Not today, or not yet: look again in ten minutes.
            c.ready_at = now + SimDuration::from_secs(600.0);
            *s.world.get_mut::<Craft>(e).expect("c") = c;
            continue;
        }
        let g = ground_at(s, c.st.pos.x, c.st.pos.y);
        c.status = CraftStatus::Airborne;
        c.st.pos.z = g + 50.0;
        c.st.speed = c
            .cruise
            .min(c.perf.as_ref().map(|_| 150.0).unwrap_or(c.cruise));
        c.st.gamma = 0.0;
        c.took_off = Some(now);
        if c.mission != Some(MissionKind::Scripted) {
            c.until = Some(now + SimDuration::from_secs(aw.p.sortie.sortie_h * 3600.0));
        }
        let what = c.mission.map(|m| m.word()).unwrap_or("its task");
        let side = s.world.get::<Ident>(e).expect("i").side;
        *s.world.get_mut::<Craft>(e).expect("c") = c;
        let text = format!("{} takes off: {}", name(s, id), what);
        s.ctx
            .log
            .push(now, "air.takeoff")
            .side(side)
            .entity(id)
            .text(text);
    }
}

/// The 1 Hz checks: mission time up, bingo fuel, a threat ring ahead.
pub fn checks(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    for id in aw.crafts.clone() {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let c = s.world.get::<Craft>(e).expect("c").clone();
        if !c.airborne() {
            continue;
        }
        let side = s.world.get::<Ident>(e).expect("i").side;
        if c.task != Task::Rtb && c.until.map(|u| now >= u).unwrap_or(false) {
            rtb(s, id, "its time on task is up");
            continue;
        }
        if c.task != Task::Rtb && !c.bingo_asked && c.fuel_share() < aw.p.bingo {
            s.world.get_mut::<Craft>(e).expect("c").bingo_asked = true;
            s.ctx.triggers.push(Trigger {
                entity: id,
                point: "bingoFuel",
                about: None,
            });
        }
        if c.task != Task::Rtb
            && c.mission == Some(MissionKind::Cap)
            && !c.bingo_asked
            && !has_kind(s, e, &[WeaponKind::Aam])
        {
            s.world.get_mut::<Craft>(e).expect("c").bingo_asked = true;
            s.ctx.triggers.push(Trigger {
                entity: id,
                point: "bingoFuel",
                about: None,
            });
        }
        if !c.ring_asked
            && matches!(
                c.mission,
                Some(MissionKind::Strike | MissionKind::Sead | MissionKind::Cap)
            )
            && c.task != Task::Rtb
        {
            if let Some((site, at, ring)) = threat_ring(s, aw, side, &c) {
                s.world.get_mut::<Craft>(e).expect("c").ring_asked = true;
                aw.context.insert(
                    id,
                    AskContext {
                        track: None,
                        target: Some(site),
                        site: Some(at),
                        ring_m: ring,
                    },
                );
                s.ctx.triggers.push(Trigger {
                    entity: id,
                    point: "threatRingAhead",
                    about: Some(site),
                });
            }
        }
    }
}

/// The ring a side believes a located AD site has: its radar type tells the
/// system (ELINT), so its longest missile.
pub(crate) fn believed_ring(s: &Sim, site: EntityId) -> f64 {
    let e = s.ctx.entity(site);
    let Some(i) = s.world.get::<Ident>(e) else {
        return 8_000.0;
    };
    i.platform
        .mounts
        .iter()
        .filter(|m| super::is_ad_mount(m.kind))
        .filter_map(|m| m.munitions.first())
        .filter_map(|m| s.ctx.catalogue.munition(m))
        .map(|m| m.max_range_m)
        .fold(8_000.0, f64::max)
}

/// The first located enemy AD ring the flight's path to its goal crosses.
fn threat_ring(s: &Sim, aw: &AirWorld, side: Side, c: &Craft) -> Option<(EntityId, Enu, f64)> {
    let goal = match &c.task {
        Task::Strike { aim, .. } => *aim,
        Task::Orbit { at, .. } | Task::Station { at, .. } | Task::Search { at, .. } => *at,
        _ => return None,
    };
    let own_target = match &c.task {
        Task::Strike { target, .. } => *target,
        _ => None,
    };
    let from = Enu::new(c.st.pos.x, c.st.pos.y, 0.0);
    for (site, fix) in aw.fixes.get(&side)? {
        if Some(*site) == own_target {
            continue;
        }
        let ring = believed_ring(s, *site);
        let d = seg_dist(fix.at, from, goal);
        if d <= ring {
            return Some((*site, fix.at, ring));
        }
    }
    None
}

/// Horizontal distance from `p` to the segment a→b.
pub(crate) fn seg_dist(p: Enu, a: Enu, b: Enu) -> f64 {
    let ab = Enu::new(b.e - a.e, b.n - a.n, 0.0);
    let ap = Enu::new(p.e - a.e, p.n - a.n, 0.0);
    let len2 = ab.e * ab.e + ab.n * ab.n;
    let t = if len2 > 0.0 {
        ((ap.e * ab.e + ap.n * ab.n) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Enu::new(ap.e - ab.e * t, ap.n - ab.n * t, 0.0).horizontal()
}

/// Send a flight home.
pub(crate) fn rtb(s: &mut Sim, id: EntityId, why: &str) {
    let e = s.ctx.entity(id);
    let side = s
        .world
        .get::<Ident>(e)
        .map(|i| i.side)
        .unwrap_or(Side::Neutral);
    let Some(mut c) = s.world.get_mut::<Craft>(e) else {
        return;
    };
    if c.task == Task::Rtb {
        return;
    }
    c.task = Task::Rtb;
    c.via = None;
    let text = format!("{} returns to base: {why}", name(s, id));
    s.ctx
        .log
        .push(s.ctx.now, "air.rtb")
        .side(side)
        .entity(id)
        .text(text);
}

/// Where the flight flies now: the goal point and altitude for its task.
/// Goal point, altitude, speed, and the orbit (centre, radius) if orbiting.
type Goal = (Enu, f64, f64, Option<(Enu, f64)>);

fn goal(s: &Sim, aw: &AirWorld, id: EntityId, c: &Craft, side: Side) -> Option<Goal> {
    let ground = |p: Enu| ground_at(s, p.e, p.n);
    let alt_or = |a: f64| {
        if c.rotary {
            a.min(aw.p.helo_alt * 2.0)
        } else {
            a
        }
    };
    match &c.task {
        Task::Idle => None,
        Task::Route { wps, alt, .. } => wps
            .first()
            .map(|w| (*w, ground(*w) + alt_or(*alt).max(100.0), c.cruise, None)),
        Task::Orbit {
            at, radius, alt, ..
        }
        | Task::Station { at, radius, alt }
        | Task::Search {
            at, radius, alt, ..
        } => Some((
            *at,
            ground(*at) + alt_or(*alt),
            c.cruise,
            Some((*at, *radius)),
        )),
        Task::Strike { target, aim, alt } => {
            if c.winchester {
                return None;
            }
            let aim = target.and_then(|t| track_of(s, side, t)).unwrap_or(*aim);
            let reach = strike_reach(s, s.ctx.entity(id), c.mission == Some(MissionKind::Sead))
                .unwrap_or(5_000.0)
                * aw.p.release_share;
            let here = Enu::new(c.st.pos.x, c.st.pos.y, 0.0);
            let back = here.sub(aim);
            let d = back.horizontal().max(1.0);
            let release = if d <= reach {
                here
            } else {
                aim.add(back.scale(reach / d))
            };
            let to = c.via.unwrap_or(release);
            Some((
                to,
                ground(to) + alt_or(*alt),
                c.max_speed.min(c.cruise * 1.2),
                None,
            ))
        }
        Task::Intercept { target } => {
            let t = track_of(s, side, *target)?;
            Some((t, t.u.max(ground(t) + 300.0), c.max_speed * 0.9, None))
        }
        Task::Rtb => {
            let b = c
                .base
                .and_then(|b| s.world.get::<Kin>(s.ctx.entity(b)).map(|k| k.pos))?;
            Some((
                b,
                ground(b) + if c.rotary { aw.p.helo_alt } else { 1_500.0 },
                c.cruise,
                None,
            ))
        }
    }
}

/// Where the side believes a target is: its freshest confirmed track whose
/// detections came from that entity (the umpire's audit ties track to
/// truth; the position is the track's, not the truth's).
pub(crate) fn track_of(s: &Sim, side: Side, target: EntityId) -> Option<Enu> {
    let store = s.ctx.tracks.get(&side)?;
    let audit = store.audit();
    store
        .confirmed(&s.ctx.p.know)
        .into_iter()
        .filter(|t| audit.truth_of(t.id) == Some(target))
        .min_by(|a, b| a.age.cmp(&b.age).then(a.id.cmp(&b.id)))
        .map(|t| t.position)
}

/// The longest reach of what this flight would release on a strike or SEAD.
pub(crate) fn strike_reach(s: &Sim, e: Entity, sead: bool) -> Option<f64> {
    let kinds: &[WeaponKind] = if sead {
        &[WeaponKind::Arm]
    } else {
        &STRIKE_KINDS[..5]
    };
    mount_of(s, e, kinds).map(|m| m.1.max_range_m)
}

/// Fly every airborne flight one air step (10 Hz).
pub fn step(s: &mut Sim, aw: &mut AirWorld, dt: f64) {
    let now = s.ctx.now;
    for id in aw.crafts.clone() {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let side = s.world.get::<Ident>(e).expect("i").side;
        let mut c = s.world.get::<Craft>(e).expect("c").clone();
        if !c.airborne() {
            continue;
        }
        let g_here = ground_at(s, c.st.pos.x, c.st.pos.y);
        let (to, mut alt, speed, orbit) = match goal(s, aw, id, &c, side) {
            Some(x) => x,
            None => {
                // Nothing to do in the air: home, or circle here.
                if c.base.is_some() {
                    c.task = Task::Rtb;
                    *s.world.get_mut::<Craft>(e).expect("c") = c;
                    continue;
                }
                let here = Enu::new(c.st.pos.x, c.st.pos.y, 0.0);
                (
                    here,
                    c.st.pos.z.max(g_here + 500.0),
                    c.cruise,
                    Some((here, 5_000.0)),
                )
            }
        };
        if c.low {
            alt = alt.min(g_here + aw.p.low_level);
        }
        alt = alt.min(c.ceiling);
        let cmd = match orbit {
            Some((centre, radius)) => {
                orbit_command(&aw.p.paths, &c.st, v3(centre), radius, alt, speed, true)
            }
            None => {
                let d = V3::new(to.e - c.st.pos.x, to.n - c.st.pos.y, 0.0);
                let heading = if let Task::Intercept { target } = c.task {
                    let tv = s
                        .world
                        .get::<Kin>(s.ctx.entity(target))
                        .map(|k| k.vel)
                        .unwrap_or_default();
                    intercept(
                        c.st.pos,
                        speed,
                        V3::new(to.e, to.n, alt),
                        V3::new(tv.e, tv.n, 0.0),
                    )
                    .heading
                } else {
                    d.heading()
                };
                AirCommand {
                    heading,
                    altitude: alt,
                    speed,
                }
            }
        };
        // Defending: break away from the threat for a while.
        let cmd = if c.defending_until > now {
            AirCommand {
                heading: (c.st.heading + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU),
                ..cmd
            }
        } else {
            cmd
        };
        match &c.perf {
            Some(perf) => {
                perf.step(&aw.p.air, &mut c.st, &cmd, V3::ZERO, dt);
            }
            None => kinematic_step(&mut c, &cmd, dt),
        }
        if c.perf.is_none() {
            c.st.fuel = (c.st.fuel - c.burn_lph * dt / 3600.0).max(0.0);
        }
        // Never into the ground.
        let g = ground_at(s, c.st.pos.x, c.st.pos.y);
        if c.st.pos.z < g + 20.0 {
            c.st.pos.z = g + 20.0;
            c.st.gamma = c.st.gamma.max(0.0);
        }
        c.flight_s += dt;
        let pos = Enu::new(c.st.pos.x, c.st.pos.y, c.st.pos.z);
        let vel = c.st.air_velocity();
        {
            let mut k = s.world.get_mut::<Kin>(e).expect("kin");
            k.pos = pos;
            k.vel = Enu::new(vel.x, vel.y, vel.z);
            k.heading = c.st.heading;
            k.agl = c.st.pos.z - g;
        }
        if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
            sig.last_moved = Some(now);
            sig.engine_on = true;
        }
        let out_of_fuel = c.st.fuel <= 0.0;
        let arrived = orbit.is_none()
            && Enu::new(to.e - pos.e, to.n - pos.n, 0.0).horizontal()
                <= aw.p.paths.arrival_radius.max(speed * dt * 2.0);
        *s.world.get_mut::<Craft>(e).expect("c") = c.clone();
        if out_of_fuel {
            lose(s, aw, id, "out of fuel");
            continue;
        }
        if arrived {
            arrive(s, aw, id);
        } else if let Task::Rtb = c.task {
            let b = c
                .base
                .and_then(|b| s.world.get::<Kin>(s.ctx.entity(b)).map(|k| k.pos));
            if let Some(b) = b {
                if b.sub(pos).horizontal() <= aw.p.land_m {
                    land(s, aw, id);
                }
            }
        }
    }
}

fn kinematic_step(c: &mut Craft, cmd: &AirCommand, dt: f64) {
    let err = wrap_pi(cmd.heading - c.st.heading);
    let turn = err.clamp(-c.turn_rate * dt, c.turn_rate * dt);
    c.st.heading = (c.st.heading + turn).rem_euclid(std::f64::consts::TAU);
    let dz = (cmd.altitude - c.st.pos.z).clamp(-c.climb * 2.0 * dt, c.climb * dt);
    let v = cmd.speed.min(c.max_speed).max(1.0);
    let accel = 5.0 * dt;
    c.st.speed += (v - c.st.speed).clamp(-accel, accel);
    c.st.pos.z += dz;
    c.st.gamma = if dt > 0.0 {
        (dz / dt / c.st.speed.max(1.0)).clamp(-1.0, 1.0).asin()
    } else {
        0.0
    };
    c.st.pos.x += c.st.heading.sin() * c.st.speed * dt;
    c.st.pos.y += c.st.heading.cos() * c.st.speed * dt;
}

/// Reached the goal point: next waypoint, the release point, or done.
fn arrive(s: &mut Sim, aw: &mut AirWorld, id: EntityId) {
    let e = s.ctx.entity(id);
    let mut c = s.world.get::<Craft>(e).expect("c").clone();
    if c.via.take().is_some() {
        *s.world.get_mut::<Craft>(e).expect("c") = c;
        return;
    }
    match c.task.clone() {
        Task::Route { mut wps, alt, then } => {
            if !wps.is_empty() {
                wps.remove(0);
            }
            c.task = if wps.is_empty() {
                then.map(|t| *t).unwrap_or(if c.base.is_some() {
                    Task::Rtb
                } else {
                    Task::Orbit {
                        at: Enu::new(c.st.pos.x, c.st.pos.y, 0.0),
                        radius: 5_000.0,
                        alt,
                        until: None,
                    }
                })
            } else {
                Task::Route { wps, alt, then }
            };
            *s.world.get_mut::<Craft>(e).expect("c") = c;
        }
        Task::Strike { target, aim, .. } => {
            *s.world.get_mut::<Craft>(e).expect("c") = c.clone();
            let side = s.world.get::<Ident>(e).expect("i").side;
            let aim = target.and_then(|t| track_of(s, side, t)).unwrap_or(aim);
            release(s, aw, id, target, aim, c.mission == Some(MissionKind::Sead));
        }
        _ => {}
    }
}

/// Release stand-off weapons at the target (`world/domains/air.ts release$`).
pub(crate) fn release(
    s: &mut Sim,
    aw: &mut AirWorld,
    id: EntityId,
    target: Option<EntityId>,
    aim: Enu,
    sead: bool,
) {
    let e = s.ctx.entity(id);
    let kinds: &[WeaponKind] = if sead {
        &[WeaponKind::Arm]
    } else {
        &STRIKE_KINDS
    };
    let pos = s.world.get::<Kin>(e).expect("k").pos;
    let range = aim.sub(pos).horizontal();
    let usable = mount_of(s, e, kinds).filter(|(_, m, _)| range <= m.max_range_m);
    let Some((mount, mun, rounds)) = usable else {
        s.world.get_mut::<Craft>(e).expect("c").winchester = true;
        rtb(s, id, "no weapon reaches the target");
        return;
    };
    let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
    let per = if mun.kind == bgws_catalogue::MunitionKind::Arm {
        1
    } else {
        2
    };
    let shots = rounds.min(fit.max(1) * per);
    let side = s.world.get::<Ident>(e).expect("i").side;
    for k in 0..shots {
        let mut rng = s.ctx.rng("air.release", id.0 as u64, k as u64);
        // A stick across the aim point: each weapon its own DMPI.
        let off = Enu::new(
            (rng.gen::<f64>() - 0.5) * 60.0,
            (rng.gen::<f64>() - 0.5) * 60.0,
            0.0,
        );
        flight::launch(
            s,
            aw,
            LaunchSpec {
                side,
                launcher: Some(id),
                munition: mun.id.clone(),
                from: pos,
                to: aim.add(off),
                target,
                purpose: Purpose::Strike,
                track: None,
                pk: mun.pk,
            },
        );
    }
    {
        let mut arms = s.world.get_mut::<Arms>(e).expect("arms");
        if let Some(m) = arms.mounts.iter_mut().find(|m| m.index == mount) {
            m.rounds -= shots;
        }
    }
    if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
        sig.last_fired = Some(s.ctx.now);
    }
    s.world.get_mut::<Craft>(e).expect("c").winchester = true;
    let text = format!(
        "{} releases {} {} at {} from {:.0} km",
        name(s, id),
        shots,
        mun.name,
        target
            .map(|t| name(s, t))
            .unwrap_or_else(|| "the aim point".into()),
        range / 1000.0
    );
    s.ctx.log.push(s.ctx.now, "air.release").side(side).entity(id).text(text).data(serde_json::json!({ "munition": mun.id, "shots": shots, "rangeM": range, "sead": sead }));
    // An ARM homing on a radar: its crew may see it coming.
    if mun.kind == bgws_catalogue::MunitionKind::Arm {
        if let Some(t) = target {
            let mut rng = s.ctx.rng("air.seeArm", id.0 as u64, t.0 as u64);
            if alive(s, s.ctx.entity(t)) && rng.gen::<f64>() < aw.p.see_arm {
                aw.context.insert(
                    t,
                    AskContext {
                        track: None,
                        target: Some(id),
                        site: None,
                        ring_m: 0.0,
                    },
                );
                s.ctx.triggers.push(Trigger {
                    entity: t,
                    point: "radarTargetedByArm",
                    about: Some(id),
                });
            }
        }
    }
    rtb(s, id, "weapons away");
}

/// Land at the base: refuel, rearm, turnaround (and a fix if it broke).
pub(crate) fn land(s: &mut Sim, aw: &mut AirWorld, id: EntityId) {
    let now = s.ctx.now;
    let e = s.ctx.entity(id);
    let platform = s.world.get::<Ident>(e).expect("i").platform.clone();
    let side = s.world.get::<Ident>(e).expect("i").side;
    let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
    let mut c = s.world.get::<Craft>(e).expect("c").clone();
    let Some(base) = c.base else { return };
    let be = s.ctx.entity(base);
    let bp = s.world.get::<Kin>(be).map(|k| k.pos).unwrap_or_default();
    // A damaged runway turns aircraft round slower.
    let sp = s
        .world
        .get::<Ident>(be)
        .map(|i| i.platform.body.staying_power)
        .unwrap_or(1.0)
        .max(1.0);
    let hits = aw.structural.get(&base).copied().unwrap_or(0.0);
    let damage = (1.0 - hits / sp).max(0.05);
    let mut turnaround = aw.p.sortie.turnaround_h
        * 3600.0
        * if c.rotary {
            aw.p.helo_turnaround_factor
        } else {
            1.0
        }
        / damage;
    let mut rng = s.ctx.rng("air.land", id.0 as u64, c.sorties as u64);
    let broke = rng.gen::<f64>() < aw.p.sortie.break_rate;
    if broke {
        let fix = Exp::new(1.0 / aw.p.sortie.fix_h.max(1e-6))
            .map(|d| d.sample(&mut rng))
            .unwrap_or(aw.p.sortie.fix_h);
        turnaround += fix * 3600.0;
    }
    let block = c.took_off.map(|t| (now - t).secs()).unwrap_or(0.0);
    c.status = CraftStatus::Landed;
    c.st.pos = V3::new(bp.e, bp.n, ground_at(s, bp.e, bp.n));
    c.st.speed = 0.0;
    c.st.gamma = 0.0;
    c.st.fuel = c.fuel_cap;
    c.ready_at = now + SimDuration::from_secs(turnaround);
    c.sorties += 1;
    c.task = Task::Idle;
    c.mission = None;
    c.until = None;
    c.winchester = false;
    c.bingo_asked = false;
    c.ring_asked = false;
    c.low = false;
    c.via = None;
    c.took_off = None;
    let sorties = c.sorties;
    *s.world.get_mut::<Craft>(e).expect("c") = c;
    {
        let mut k = s.world.get_mut::<Kin>(e).expect("k");
        k.pos = Enu::new(bp.e, bp.n, bp.u);
        k.vel = Enu::default();
        k.agl = 0.0;
    }
    // Rearm from the base.
    {
        let mut arms = s.world.get_mut::<Arms>(e).expect("arms");
        for m in arms.mounts.iter_mut() {
            if let Some(cm) = platform.mounts.get(m.index) {
                m.rounds = cm.magazine * fit;
            }
        }
    }
    let text = format!(
        "{} lands; ready again in {:.0} min{}",
        name(s, id),
        turnaround / 60.0,
        if broke { " (broken: a fix first)" } else { "" }
    );
    s.ctx.log.push(now, "air.land").side(side).entity(id).text(text).data(serde_json::json!({ "sorties": sorties, "blockS": block, "turnaroundS": turnaround, "broke": broke, "runway": damage }));
}

/// A flight is lost (out of fuel, no base to land at).
pub(crate) fn lose(s: &mut Sim, _aw: &mut AirWorld, id: EntityId, why: &str) {
    let e = s.ctx.entity(id);
    let side = s
        .world
        .get::<Ident>(e)
        .map(|i| i.side)
        .unwrap_or(Side::Neutral);
    if let Some(mut c) = s.world.get_mut::<Craft>(e) {
        c.status = CraftStatus::Lost;
    }
    if let Some(mut h) = s.world.get_mut::<Health>(e) {
        let now = s.ctx.now;
        for _ in 0..h.fit {
            h.losses.push(now);
        }
        h.fit = 0;
        h.destroyed = true;
    }
    let text = format!("{} is lost: {why}", name(s, id));
    s.ctx
        .log
        .push(s.ctx.now, "air.lost")
        .side(side)
        .entity(id)
        .text(text);
}

/// The speed of sound at an altitude (for words in the AAR).
pub fn mach(alt: f64, v: f64) -> f64 {
    isa(alt).mach(v)
}
