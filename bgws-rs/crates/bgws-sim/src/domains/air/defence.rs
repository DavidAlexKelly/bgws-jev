//! Air and missile defence (§8.8; TS `gbad`, `adFire`, `interceptorChoice`):
//! ground, ship and fighter AD engage the side's air tracks through the
//! kill chain [R3] — detect, firm track, classify, assign, launch — each
//! stage with a time and a chance from `bgws_domains::air_defence::KillChain`.
//! Every second the side's weapon-target assignment (Kuhn–Munkres on
//! value·Pk − w·cost, `assign_optimal`) gives each ready fire unit at most
//! one track, holding an expensive shot when a cheaper layer nearly as good
//! will get its chance [R4.4, R6.2]. Interceptors fly as munitions to the
//! predicted intercept point; the unit looks before it shoots again
//! (shoot-look-shoot, or two at once where doctrine says). Launchers empty
//! and reload from the unit's stores after a drill. Decoys that are not
//! recognised draw fire [P10]. The side's cost exchange — interceptor spend
//! against the attacker's value destroyed — is kept and logged [P11].
//!
//! The unit's posture is a decision (`trackAppears`, `radarTargetedByArm`):
//! the cheapest or the surest interceptor, weapons tight, silent; until it
//! is answered the unit fights under its standing orders.

use super::flight::{self, LaunchSpec, Purpose};
use super::radar::AirSensors;
use super::{is_ad_mount, AirWorld, AskContext, Chain, Craft, MissionKind, Task, Wcs};
use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::{Sim, Trigger};
use crate::systems::{alive, name};
use bevy_ecs::prelude::*;
use bgws_catalogue::{Domain, WeaponKind};
use bgws_core::TrackId;
use bgws_core::{EntityId, Enu, Side, SimDuration, SimTime};
use bgws_domains::air_defence::{assign_greedy, assign_optimal, AdSystem, ShotOption, ThreatClass};
use bgws_domains::vec3::V3;
use bgws_know::{Hypothesis, IdLevel, TrackView};
use bgws_scenario::v4::Emcon;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a unit chooses its interceptor (`air.engageCheapest`, `air.engageSurest`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
    #[default]
    Cheapest,
    Surest,
}

/// One AD mount of a unit.
#[derive(Clone, Debug)]
pub struct AdMount {
    /// Index of the catalogue mount (and of its `MountState`).
    pub mount: usize,
    pub munition: String,
    /// The `ad.sys.*` system type its Pk table comes from.
    pub sys: AdSystem,
    pub min_range: f64,
    pub max_range: f64,
    pub speed: f64,
    pub cost: f64,
    pub salvo: u32,
    pub magazine: u32,
    /// Rounds ready on the launchers (the rest are in the unit's stores).
    pub ready_rounds: u32,
    pub ready_at: SimTime,
}

/// An air-defence fire unit.
#[derive(Component, Clone, Debug)]
pub struct AdUnit {
    pub mounts: Vec<AdMount>,
    pub policy: Policy,
    pub wcs: Wcs,
    pub last_asked: Option<SimTime>,
}

/// The side's cost-exchange account (§8.8) [P11].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    /// Interceptors and AAMs fired at air threats.
    pub shots: u32,
    pub interceptor_cost: f64,
    /// Air threats and aircraft it destroyed, and what they cost the enemy.
    pub kills: u32,
    pub value_killed: f64,
    pub decoy_shots: u32,
    pub decoys_killed: u32,
    /// Enemy munitions that got through.
    pub leakers: u32,
    /// What its own strikes cost, and what they destroyed.
    pub strike_cost: f64,
    pub value_destroyed: f64,
    pub fratricide: u32,
}

impl Ledger {
    /// Interceptor dollars spent per enemy dollar destroyed.
    pub fn cost_exchange(&self) -> f64 {
        if self.value_killed > 0.0 {
            self.interceptor_cost / self.value_killed
        } else {
            f64::INFINITY
        }
    }
}

/// The `ad.sys.*` type for a mount kind.
fn sys_kind(k: WeaponKind) -> &'static str {
    match k {
        WeaponKind::SamLong => "lrsam",
        WeaponKind::SamMedium => "mrsam",
        WeaponKind::SamShort => "shorad",
        WeaponKind::GunAd | WeaponKind::Ciws => "gun",
        WeaponKind::InterceptorDrone => "interceptorDrone",
        _ => "fighter",
    }
}

fn v3(p: Enu) -> V3 {
    V3::new(p.e, p.n, p.u)
}

/// AD units get their mounts; every air-looking sensor its component.
pub fn setup(s: &mut Sim, aw: &mut AirWorld) {
    let order = s.ctx.order.clone();
    for (idx, &e) in order.iter().enumerate() {
        let Some(i) = s.world.get::<Ident>(e) else {
            continue;
        };
        let platform = i.platform.clone();
        let id = EntityId(idx as u32);
        let ship = platform.domain == Domain::Surface;
        let ad_capable = !matches!(
            platform.domain,
            Domain::Air | Domain::Space | Domain::Munition | Domain::Cyber
        ) && platform.mounts.iter().any(|m| is_ad_mount(m.kind));
        // A warship's defence against missiles is the naval domain's when it is simulated.
        let ad = aw.ad_on && ad_capable && !(ship && aw.naval_simulated);
        let emcon = s
            .ctx
            .scenario
            .forces
            .get(idx)
            .map(|f| s.ctx.scenario.emitters.emcon_of(f))
            .unwrap_or_default();
        let mut sens = super::radar::sensors_of(s, aw, &platform, ad);
        sens.emcon = emcon;
        if !sens.radars.is_empty() || !sens.passive.is_empty() || sens.esm_range > 0.0 {
            s.world.entity_mut(e).insert(sens);
        }
        if !ad {
            continue;
        }
        let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
        let mounts: Vec<AdMount> = platform
            .mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| is_ad_mount(m.kind))
            .filter_map(|(mi, m)| {
                let mun = s.ctx.catalogue.munition(m.munitions.first()?)?.clone();
                let sys = AdSystem::from_figures(
                    sys_kind(m.kind),
                    &format!("{}/{}", platform.id, m.id),
                    V3::ZERO,
                    &s.ctx.figures,
                );
                Some(AdMount {
                    mount: mi,
                    munition: mun.id.clone(),
                    min_range: mun.min_range_m,
                    max_range: mun.max_range_m,
                    speed: if mun.speed_ms > 0.0 {
                        mun.speed_ms
                    } else {
                        sys.interceptor_speed.max(1_000.0)
                    },
                    cost: if mun.cost_usd > 0.0 {
                        mun.cost_usd
                    } else {
                        sys.cost
                    },
                    salvo: if sys.doctrine
                        == bgws_domains::air_defence::Doctrine::ShootShootLookShoot
                    {
                        2
                    } else {
                        1
                    },
                    magazine: m.magazine,
                    ready_rounds: m.magazine * fit,
                    ready_at: SimTime::from_secs(m.ready_s),
                    sys,
                })
            })
            .collect();
        s.world.entity_mut(e).insert(AdUnit {
            mounts,
            policy: Policy::Cheapest,
            wcs: Wcs::Free,
            last_asked: None,
        });
        aw.ad_units.push(id);
    }
}

/// Is this entity's radar on? EMCON silent, a shutdown in force, or limited
/// with nothing to look at: off.
pub(crate) fn radiating(s: &Sim, e: Entity) -> bool {
    let Some(sens) = s.world.get::<AirSensors>(e) else {
        return false;
    };
    if sens.radars.is_empty() || sens.emcon == Emcon::Silent {
        return false;
    }
    if sens.silent_until.map(|t| s.ctx.now < t).unwrap_or(false) {
        return false;
    }
    if sens.emcon == Emcon::Limited {
        // Radiate only when cued: the side holds an air track inside the radar's reach.
        let Some(i) = s.world.get::<Ident>(e) else {
            return false;
        };
        let Some(k) = s.world.get::<Kin>(e) else {
            return false;
        };
        let reach = sens.radars.iter().map(|r| r.range_max).fold(0.0, f64::max);
        return s
            .ctx
            .tracks
            .get(&i.side)
            .map(|st| {
                st.confirmed(&s.ctx.p.know).iter().any(|t| {
                    t.domain == bgws_know::Domain::Air
                        && t.position.sub(k.pos).horizontal() <= reach
                })
            })
            .unwrap_or(false);
    }
    true
}

/// A scheduled end of a radar shutdown.
pub fn radar_on(s: &mut Sim, id: EntityId) {
    let e = s.ctx.entity(id);
    let side = s
        .world
        .get::<Ident>(e)
        .map(|i| i.side)
        .unwrap_or(Side::Neutral);
    if let Some(mut sens) = s.world.get_mut::<AirSensors>(e) {
        if sens.silent_until.map(|t| t <= s.ctx.now).unwrap_or(false) {
            sens.silent_until = None;
            let text = format!("{} brings its radar back up", name(s, id));
            s.ctx
                .log
                .push(s.ctx.now, "ad.radarOn")
                .side(side)
                .entity(id)
                .text(text);
        }
    }
}

/// Shut a radar down for a while.
pub(crate) fn go_silent(s: &mut Sim, aw: &AirWorld, id: EntityId, why: &str) {
    let e = s.ctx.entity(id);
    let until = s.ctx.now + SimDuration::from_secs(aw.p.silent_s);
    let side = s
        .world
        .get::<Ident>(e)
        .map(|i| i.side)
        .unwrap_or(Side::Neutral);
    if let Some(mut sens) = s.world.get_mut::<AirSensors>(e) {
        sens.silent_until = Some(until);
    } else {
        return;
    }
    s.ctx.queue.schedule(
        until,
        Priority::Process,
        Scheduled::Timer {
            system: "air.radarOn".into(),
            key: id.0 as u64,
        },
    );
    let text = format!("{} shuts its radar down: {why}", name(s, id));
    s.ctx
        .log
        .push(s.ctx.now, "ad.silent")
        .side(side)
        .entity(id)
        .text(text);
}

/// What the defender takes a track for, from its class belief and kinematics.
pub(crate) fn apparent_class(t: &TrackView) -> ThreatClass {
    let speed = t.velocity.norm();
    let top = t
        .class_top
        .first()
        .filter(|(_, p)| *p >= 0.4)
        .map(|(h, _)| *h);
    match top {
        Some(Hypothesis::Drone) | Some(Hypothesis::AirDecoy) => ThreatClass::OneWayDrone,
        Some(Hypothesis::Missile) => {
            if speed > 1_000.0 {
                ThreatClass::Ballistic
            } else if speed > 500.0 && t.position.u > 5_000.0 {
                ThreatClass::Rocket
            } else {
                ThreatClass::Cruise
            }
        }
        Some(Hypothesis::Fighter | Hypothesis::Bomber | Hypothesis::Helicopter) => {
            ThreatClass::Aircraft
        }
        _ => {
            if speed < 90.0 {
                ThreatClass::OneWayDrone
            } else if speed > 1_000.0 {
                ThreatClass::Ballistic
            } else if speed < 320.0 && t.position.u < 1_500.0 {
                ThreatClass::Cruise
            } else {
                ThreatClass::Aircraft
            }
        }
    }
}

fn value_of(aw: &AirWorld, c: ThreatClass) -> f64 {
    let p = &aw.p;
    match c {
        ThreatClass::OneWayDrone | ThreatClass::Decoy => p.value_owa,
        ThreatClass::Cruise => p.value_cruise,
        ThreatClass::SeaSkimmer => p.value_asm,
        ThreatClass::Ballistic => p.value_ballistic,
        ThreatClass::Rocket => p.value_rocket,
        ThreatClass::Aircraft => p.value_aircraft,
    }
}

/// One fire unit (or fighter) for this cycle's assignment.
struct Shooter {
    unit: EntityId,
    /// Index into the unit's `AdUnit.mounts`, or the catalogue mount for a fighter.
    mount: usize,
    fighter: bool,
    pos: Enu,
    sys: AdSystem,
    munition: String,
    min_range: f64,
    max_range: f64,
    speed: f64,
    cost: f64,
    salvo: u32,
    rounds: u32,
    policy: Policy,
    wcs: Wcs,
}

/// Every engageable fire unit of a side now.
fn shooters(s: &Sim, aw: &AirWorld, side: Side) -> Vec<Shooter> {
    let now = s.ctx.now;
    let mut out = Vec::new();
    for &id in &aw.ad_units {
        let e = s.ctx.entity(id);
        let Some(i) = s.world.get::<Ident>(e) else {
            continue;
        };
        if i.side != side
            || !alive(s, e)
            || !s
                .world
                .get::<Health>(e)
                .map(|h| h.can_fire())
                .unwrap_or(false)
        {
            continue;
        }
        let Some(u) = s.world.get::<AdUnit>(e) else {
            continue;
        };
        if u.wcs == Wcs::Hold {
            continue;
        }
        let on = radiating(s, e);
        let k = s.world.get::<Kin>(e).expect("k");
        let h = s
            .world
            .get::<AirSensors>(e)
            .and_then(|x| x.radars.first().map(|r| r.height))
            .unwrap_or(3.0);
        for (mi, m) in u.mounts.iter().enumerate() {
            // A radar-guided system needs its radar; guns and IR missiles can fight on the side's picture.
            let needs_radar = matches!(
                m.sys.layer,
                bgws_domains::air_defence::Layer::Long | bgws_domains::air_defence::Layer::Medium
            );
            if (needs_radar && !on) || m.ready_rounds == 0 || m.ready_at > now {
                continue;
            }
            out.push(Shooter {
                unit: id,
                mount: mi,
                fighter: false,
                pos: Enu::new(k.pos.e, k.pos.n, k.pos.u + h),
                sys: m.sys.clone(),
                munition: m.munition.clone(),
                min_range: m.min_range,
                max_range: m.max_range,
                speed: m.speed,
                cost: m.cost,
                salvo: m.salvo,
                rounds: m.ready_rounds,
                policy: u.policy,
                wcs: u.wcs,
            });
        }
    }
    // Fighters on CAP: a layer against drones and cruise missiles [R6.2].
    for &id in &aw.crafts {
        let e = s.ctx.entity(id);
        let Some(c) = s.world.get::<Craft>(e) else {
            continue;
        };
        let i = s.world.get::<Ident>(e).expect("i");
        if i.side != side || !c.airborne() || !alive(s, e) || c.wcs == Wcs::Hold {
            continue;
        }
        let on_cap = matches!(c.mission, Some(MissionKind::Cap))
            || (c.mission == Some(MissionKind::Scripted)
                && matches!(
                    c.task,
                    Task::Orbit { .. } | Task::Station { .. } | Task::Intercept { .. }
                ));
        if !on_cap {
            continue;
        }
        let Some((mount, mun, rounds)) = super::craft::mount_of(s, e, &[WeaponKind::Aam]) else {
            continue;
        };
        let ready = s
            .world
            .get::<Arms>(e)
            .and_then(|a| {
                a.mounts
                    .iter()
                    .find(|m| m.index == mount)
                    .map(|m| m.ready_at <= now)
            })
            .unwrap_or(false);
        if !ready {
            continue;
        }
        let sys = AdSystem::from_figures("fighter", &i.name, V3::ZERO, &s.ctx.figures);
        out.push(Shooter {
            unit: id,
            mount,
            fighter: true,
            pos: s.world.get::<Kin>(e).expect("k").pos,
            sys,
            munition: mun.id.clone(),
            min_range: mun.min_range_m,
            max_range: mun.max_range_m,
            speed: mun.speed_ms.max(300.0),
            cost: mun.cost_usd,
            salvo: 1,
            rounds,
            policy: Policy::Cheapest,
            wcs: c.wcs,
        });
    }
    out
}

/// Time for an interceptor launched now to meet the track, and where.
fn intercept_point(sh: &Shooter, t: &TrackView) -> (f64, Enu) {
    let sol = bgws_domains::air::intercept(v3(sh.pos), sh.speed, v3(t.position), v3(t.velocity));
    let tgo = if sol.time_to_go.is_finite() {
        sol.time_to_go
    } else {
        t.position.sub(sh.pos).norm() / sh.speed.max(1.0)
    };
    let p = t.position.add(t.velocity.scale(tgo));
    (tgo, p)
}

/// Closest the track's straight path comes to `p` in the next ten minutes.
fn closest_approach(t: &TrackView, p: Enu) -> f64 {
    let r = t.position.sub(p);
    let v = t.velocity;
    let vv = v.dot(v);
    let tau = if vv > 0.0 {
        (-r.dot(v) / vv).clamp(0.0, 600.0)
    } else {
        0.0
    };
    r.add(v.scale(tau)).norm()
}

/// The once-a-second engagement cycle for both sides.
#[allow(clippy::map_entry)]
pub fn cycle(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    let chain_fig = aw.p.chain.clone();
    for side in Side::PLAYERS {
        let views: Vec<TrackView> = match s.ctx.tracks.get(&side) {
            Some(st) => st
                .confirmed(&s.ctx.p.know)
                .into_iter()
                .filter(|t| t.domain == bgws_know::Domain::Air)
                .collect(),
            None => continue,
        };
        // Kill chains: open one per new track, classify on time, close dead ones.
        let live: std::collections::BTreeSet<TrackId> = views.iter().map(|t| t.id).collect();
        aw.chains
            .retain(|(sd, tid), _| *sd != side || live.contains(tid));
        for t in &views {
            let key = (side, t.id);
            if !aw.chains.contains_key(&key) {
                let truth = s.ctx.tracks[&side].audit().truth_of(t.id);
                // Own aircraft and munitions are not threats (IFF answers).
                let own = truth
                    .map(|x| entity_side(s, x) == Some(side))
                    .unwrap_or(false);
                let mut rng = s.ctx.rng("ad.chain", side.index() as u64, t.id.0 as u64);
                let ok = !own && rng.gen::<f64>() < chain_fig.p_track * chain_fig.p_classify;
                let engageable_at = if ok {
                    Some(t.first_seen + SimDuration::from_secs(chain_fig.reaction()))
                } else {
                    None
                };
                aw.chains.insert(
                    key,
                    Chain {
                        first_seen: t.first_seen,
                        engageable_at,
                        class: apparent_class(t),
                        decoy_recognised: false,
                        shots: 0,
                        look_until: None,
                        truth,
                        classified: false,
                    },
                );
                if !ok && !own {
                    s.ctx
                        .log
                        .push(now, "ad.chainFailed")
                        .side(side)
                        .text(format!(
                            "{side}'s kill chain loses track {} before it is classified",
                            t.id.0
                        ))
                        .data(serde_json::json!({ "track": t.id.0 }));
                }
            }
            let ch = aw.chains.get_mut(&key).expect("chain");
            let classify_at = ch.first_seen
                + SimDuration::from_secs(chain_fig.detect_to_track + chain_fig.track_to_classify);
            if !ch.classified && now >= classify_at && ch.engageable_at.is_some() {
                ch.classified = true;
                ch.class = apparent_class(t);
                let decoy_truth = ch.truth.map(|x| is_decoy(s, x)).unwrap_or(false);
                let mut rng = s
                    .ctx
                    .rng("ad.discriminate", side.index() as u64, t.id.0 as u64);
                let seen_as_decoy = t
                    .class_top
                    .first()
                    .map(|(h, p)| *h == Hypothesis::AirDecoy && *p > 0.5)
                    .unwrap_or(false);
                if decoy_truth && (seen_as_decoy || rng.gen::<f64>() < chain_fig.p_discriminate) {
                    ch.decoy_recognised = true;
                    s.ctx
                        .log
                        .push(now, "ad.decoyRecognised")
                        .side(side)
                        .text(format!("{side} recognises track {} as a decoy", t.id.0))
                        .data(serde_json::json!({ "track": t.id.0 }));
                } else {
                    let class = ch.class;
                    s.ctx
                        .log
                        .push(now, "ad.classified")
                        .side(side)
                        .text(format!("{side} classifies track {} as {:?}", t.id.0, class))
                        .data(serde_json::json!({ "track": t.id.0, "class": class }));
                }
            }
        }
        if !aw.ad_on {
            continue;
        }
        // Engageable tracks: the chain has run, fresh, not being looked at.
        let tracks: Vec<&TrackView> = views
            .iter()
            .filter(|t| {
                let ch = &aw.chains[&(side, t.id)];
                ch.engageable_at.map(|x| x <= now).unwrap_or(false)
                    && !ch.decoy_recognised
                    && ch.look_until.map(|x| x <= now).unwrap_or(true)
                    && t.age.secs() < 5.0
            })
            .collect();
        if tracks.is_empty() {
            continue;
        }
        let sh = shooters(s, aw, side);
        if sh.is_empty() {
            continue;
        }
        raise_track_questions(s, aw, side, &sh, &tracks);
        let mut options = Vec::new();
        let mut aim: BTreeMap<(usize, usize), (f64, Enu)> = BTreeMap::new();
        for (si, x) in sh.iter().enumerate() {
            for (ti, t) in tracks.iter().enumerate() {
                let class = aw.chains[&(side, t.id)].class;
                if x.fighter && class == ThreatClass::Aircraft {
                    continue; // air-to-air is the pilots' decision (targetInWez)
                }
                if x.wcs == Wcs::Tight && t.best_level < IdLevel::Recognise {
                    continue;
                }
                let pk = x.sys.pk_against(class);
                if pk <= 0.0 {
                    continue;
                }
                let (tgo, p) = intercept_point(x, t);
                let r = p.sub(x.pos).norm();
                if r < x.min_range || r > x.max_range || !tgo.is_finite() {
                    continue;
                }
                // Hold the shot for a cheaper layer nearly as good that will get its chance.
                let defer = sh.iter().enumerate().any(|(oi, o)| {
                    oi != si
                        && o.cost <= x.cost * chain_fig.defer_cost_ratio
                        && o.sys.pk_against(class) >= chain_fig.defer_pk_ratio * pk
                        && closest_approach(t, o.pos) <= o.max_range * 0.9
                        && t.position.sub(o.pos).norm() > o.max_range * 0.5
                });
                if defer {
                    continue;
                }
                let mult = match x.policy {
                    Policy::Cheapest => aw.p.cheapest_mult,
                    Policy::Surest => aw.p.surest_mult,
                };
                options.push(ShotOption {
                    system: si,
                    threat: ti,
                    pk,
                    cost: x.cost * mult,
                });
                aim.insert((si, ti), (tgo, p));
            }
        }
        if options.is_empty() {
            continue;
        }
        let values: Vec<f64> = tracks
            .iter()
            .map(|t| value_of(aw, aw.chains[&(side, t.id)].class))
            .collect();
        let picks = if aw.abstracted {
            assign_greedy(&values, &options, chain_fig.cost_weight)
        } else {
            assign_optimal(&values, &options, chain_fig.cost_weight)
        };
        for o in picks {
            let x = &sh[o.system];
            let t = tracks[o.threat];
            let (tgo, p) = aim[&(o.system, o.threat)];
            fire(s, aw, side, x, t, o.pk, tgo, p);
        }
    }
}

fn entity_side(s: &Sim, id: EntityId) -> Option<Side> {
    let e = s.ctx.entity(id);
    s.world
        .get::<Ident>(e)
        .map(|i| i.side)
        .or_else(|| s.world.get::<super::InFlight>(e).map(|f| f.side))
}

fn is_decoy(s: &Sim, id: EntityId) -> bool {
    let e = s.ctx.entity(id);
    s.world
        .get::<super::InFlight>(e)
        .map(|f| f.decoy)
        .unwrap_or(false)
        || s.world.get::<Ident>(e).map(|i| i.dummy).unwrap_or(false)
}

/// Launch the assigned interceptors.
#[allow(clippy::too_many_arguments)]
fn fire(
    s: &mut Sim,
    aw: &mut AirWorld,
    side: Side,
    x: &Shooter,
    t: &TrackView,
    pk: f64,
    tgo: f64,
    p: Enu,
) {
    let now = s.ctx.now;
    let key = (side, t.id);
    let class = aw.chains[&key].class;
    let truth = aw.chains[&key].truth;
    let salvo = if matches!(class, ThreatClass::OneWayDrone | ThreatClass::Decoy) {
        1
    } else {
        x.salvo
    }
    .min(x.rounds)
    .max(1);
    let e = s.ctx.entity(x.unit);
    for _ in 0..salvo {
        flight::launch(
            s,
            aw,
            LaunchSpec {
                side,
                launcher: Some(x.unit),
                munition: x.munition.clone(),
                from: x.pos,
                to: p,
                target: truth,
                purpose: Purpose::Intercept,
                track: Some(t.id),
                pk,
            },
        );
    }
    // Rounds: the launcher's ready rounds and the unit's total.
    let reload = if x.fighter {
        None
    } else {
        let mut u = s.world.get_mut::<AdUnit>(e).expect("ad");
        let m = &mut u.mounts[x.mount];
        m.ready_rounds -= salvo;
        m.ready_at = now + SimDuration::from_secs(m.sys.cycle);
        Some(m.mount)
    };
    let mount_index = reload.unwrap_or(x.mount);
    if let Some(mut arms) = s.world.get_mut::<Arms>(e) {
        if let Some(m) = arms.mounts.iter_mut().find(|m| m.index == mount_index) {
            m.rounds = m.rounds.saturating_sub(salvo);
            if x.fighter {
                m.ready_at = now + SimDuration::from_secs(2.0);
            }
        }
    }
    if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
        sig.last_fired = Some(now);
    }
    let decoy = truth.map(|x| is_decoy(s, x)).unwrap_or(false);
    {
        let ch = aw.chains.get_mut(&key).expect("chain");
        ch.shots += salvo;
        ch.look_until = Some(now + SimDuration::from_secs(tgo + aw.p.chain.assess));
    }
    {
        let l = aw.ledger.entry(side).or_default();
        l.shots += salvo;
        l.interceptor_cost += x.cost * salvo as f64;
        if decoy {
            l.decoy_shots += salvo;
        }
    }
    let range = p.sub(x.pos).norm();
    let text = format!(
        "{} fires {} {} at track {} ({:?}, {:.1} km, Pk {:.2})",
        name(s, x.unit),
        salvo,
        x.munition,
        t.id.0,
        class,
        range / 1000.0,
        pk
    );
    s.ctx.log.push(now, "ad.engage").side(side).entity(x.unit).text(text).data(serde_json::json!({
        "track": t.id.0, "class": class, "munition": x.munition, "salvo": salvo, "pk": pk, "costUsd": x.cost * salvo as f64,
        "rangeM": range, "timeToGoS": tgo, "policy": x.policy, "system": x.sys.name,
    }));
}

/// A raid in a unit's zone: ask it how to fight (once a while per unit).
fn raise_track_questions(
    s: &mut Sim,
    aw: &mut AirWorld,
    side: Side,
    sh: &[Shooter],
    tracks: &[&TrackView],
) {
    let now = s.ctx.now;
    let mut asked = std::collections::BTreeSet::new();
    for x in sh {
        if x.fighter || asked.contains(&x.unit) {
            continue;
        }
        let e = s.ctx.entity(x.unit);
        let last = s.world.get::<AdUnit>(e).and_then(|u| u.last_asked);
        if last
            .map(|l| (now - l).secs() < aw.p.ad_ask_s)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(t) = tracks
            .iter()
            .filter(|t| t.position.sub(x.pos).norm() <= x.max_range)
            .min_by(|a, b| {
                a.position
                    .sub(x.pos)
                    .norm()
                    .total_cmp(&b.position.sub(x.pos).norm())
                    .then(a.id.cmp(&b.id))
            })
        else {
            continue;
        };
        asked.insert(x.unit);
        s.world.get_mut::<AdUnit>(e).expect("ad").last_asked = Some(now);
        let truth = aw.chains.get(&(side, t.id)).and_then(|c| c.truth);
        aw.context.insert(
            x.unit,
            AskContext {
                track: Some(t.id),
                target: truth,
                site: None,
                ring_m: 0.0,
            },
        );
        s.ctx.triggers.push(Trigger {
            entity: x.unit,
            point: "trackAppears",
            about: None,
        });
    }
}

/// Empty launchers reload from the unit's stores after the drill.
pub fn reload(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    for &id in &aw.ad_units.clone() {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let Some(u) = s.world.get::<AdUnit>(e).cloned() else {
            continue;
        };
        let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(1);
        for (mi, m) in u.mounts.iter().enumerate() {
            if m.ready_rounds > 0 {
                aw.reloads.remove(&(id, mi));
                continue;
            }
            let total = s
                .world
                .get::<Arms>(e)
                .and_then(|a| {
                    a.mounts
                        .iter()
                        .find(|x| x.index == m.mount)
                        .map(|x| x.rounds)
                })
                .unwrap_or(0);
            if total == 0 {
                continue;
            }
            let since = *aw.reloads.entry((id, mi)).or_insert(now);
            if (now - since).secs() < aw.p.reload_s {
                continue;
            }
            aw.reloads.remove(&(id, mi));
            let n = (m.magazine * fit).min(total);
            s.world.get_mut::<AdUnit>(e).expect("ad").mounts[mi].ready_rounds = n;
            let side = s.world.get::<Ident>(e).expect("i").side;
            let text = format!(
                "{} reloads its {} launchers ({} ready, {} left in all)",
                name(s, id),
                m.munition,
                n,
                total
            );
            s.ctx
                .log
                .push(now, "ad.reload")
                .side(side)
                .entity(id)
                .text(text);
        }
    }
}

/// Each minute: the side's cost exchange when it changed.
pub fn log_ledger(s: &mut Sim, aw: &mut AirWorld) {
    let now = s.ctx.now;
    for (side, l) in aw.ledger.clone() {
        let key = (l.shots, l.kills + l.leakers);
        if aw.ledger_logged.get(&side) == Some(&key) {
            continue;
        }
        aw.ledger_logged.insert(side, key);
        let cx = l.cost_exchange();
        s.ctx.log.push(now, "ad.costExchange").side(side).text(format!("{side}: {} interceptors (${:.1}M) for {} kills (${:.1}M), {} leakers, {} shots at decoys", l.shots, l.interceptor_cost / 1e6, l.kills, l.value_killed / 1e6, l.leakers, l.decoy_shots)).data(serde_json::json!({ "ledger": l, "costExchange": if cx.is_finite() { serde_json::json!(cx) } else { serde_json::Value::Null } }));
    }
}

/// The side's account so far.
pub fn ledger(s: &Sim, side: Side) -> Ledger {
    super::state(s)
        .and_then(|a| a.ledger.get(&side).cloned())
        .unwrap_or_default()
}
