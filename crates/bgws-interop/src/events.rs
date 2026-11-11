//! Fire and detonation, protocol-free: what the sim's fire events say other
//! simulations should hear. DIS sends these as Fire and Detonation PDUs (and
//! HLA will send them as RPR WeaponFire and MunitionDetonation).
//!
//! - A gun volley (`fire.hit`, `fire.kill`, `fire.miss`) is a fire and its
//!   detonation at once, sharing an event.
//! - A guided missile is a fire at launch (`fire.launch`) and a detonation
//!   when it lands (the `fire.*` event at arrival), under the launch's event.
//! - Artillery and mortars: a fire per salvo (`fires.fire`, `fires.adjust`),
//!   a detonation at the impact centre (`fires.impact`) for every simulation
//!   to judge its own entities near it.
//! - Fire to suppress (`fire.suppress`): a fire, and a detonation on the
//!   ground at the spot.
//!
//! Only our own entities' fire is sent: a remote entity's owner sends its own.

use bgws_core::{Enu, EntityId, LogEvent};
use bgws_sim::Sim;
use std::collections::BTreeMap;

/// What a detonation did, in SISO-REF-010 Detonation Result terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Result {
    EntityImpact = 1,
    EntityProximate = 2,
    GroundImpact = 3,
}

/// A weapon fired.
#[derive(Clone, Debug, PartialEq)]
pub struct FireOut {
    pub event: u16,
    pub firer: EntityId,
    pub target: Option<EntityId>,
    pub munition: String,
    pub rounds: u32,
    /// The firer's position (east, north, height above mean sea level).
    pub from: Enu,
    pub range_m: f64,
}

/// A round (or rounds) went off.
#[derive(Clone, Debug, PartialEq)]
pub struct DetonationOut {
    pub event: u16,
    pub firer: Option<EntityId>,
    pub target: Option<EntityId>,
    pub munition: String,
    pub rounds: u32,
    pub at: Enu,
    pub result: Result,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Fire(FireOut),
    Detonation(DetonationOut),
}

/// Numbers fire events and pairs each detonation with its fire.
#[derive(Clone, Debug, Default)]
pub struct FireBook {
    next: u16,
    /// Missiles in flight: (firer, target, munition) → event, oldest first.
    missiles: BTreeMap<(EntityId, EntityId, String), Vec<u16>>,
    /// Artillery missions → the event of their last salvo.
    missions: BTreeMap<i64, u16>,
}

fn pos(sim: &Sim, id: EntityId) -> Option<Enu> {
    let e = *sim.ctx.order.get(id.index())?;
    sim.world.get::<bgws_sim::components::Kin>(e).map(|k| k.pos)
}

fn num(e: &LogEvent, k: &str) -> f64 {
    e.data.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0)
}

fn text(e: &LogEvent, k: &str) -> String {
    e.data.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string()
}

impl FireBook {
    fn event(&mut self) -> u16 {
        // DIS event numbers run 1..=65535 and wrap.
        self.next = if self.next == u16::MAX { 1 } else { self.next + 1 };
        self.next
    }

    fn guided(sim: &Sim, munition: &str) -> bool {
        sim.ctx.catalogue.munition(munition).is_some_and(|m| m.guidance != bgws_catalogue::Guidance::None && !(m.kind == bgws_catalogue::MunitionKind::Shell && m.guidance == bgws_catalogue::Guidance::Radar))
    }

    /// The fires and detonations a tick's events call for.
    pub fn from_events(&mut self, sim: &Sim, events: &[LogEvent]) -> Vec<Outgoing> {
        let mut out = Vec::new();
        for e in events {
            let Some(firer) = e.entity else { continue };
            let ours = sim.ctx.order.get(firer.index()).is_some_and(|&x| !bgws_sim::remote::is_remote(sim, x));
            if !ours {
                continue;
            }
            let from = pos(sim, firer).unwrap_or_default();
            let munition = text(e, "munition");
            let rounds = num(e, "rounds").max(1.0) as u32;
            match e.kind.as_str() {
                "fire.launch" => {
                    let Some(t) = e.target else { continue };
                    let ev = self.event();
                    self.missiles.entry((firer, t, munition.clone())).or_default().push(ev);
                    out.push(Outgoing::Fire(FireOut { event: ev, firer, target: Some(t), munition, rounds, from, range_m: num(e, "rangeM") }));
                }
                "fire.hit" | "fire.kill" | "fire.miss" => {
                    let Some(t) = e.target else { continue };
                    let landed = if Self::guided(sim, &munition) {
                        // A missile arriving: the event it was launched under.
                        self.missiles.get_mut(&(firer, t, munition.clone())).and_then(|v| (!v.is_empty()).then(|| v.remove(0)))
                    } else {
                        None
                    };
                    let ev = match landed {
                        Some(ev) => ev,
                        None => {
                            let ev = self.event();
                            out.push(Outgoing::Fire(FireOut { event: ev, firer, target: Some(t), munition: munition.clone(), rounds, from, range_m: num(e, "rangeM") }));
                            ev
                        }
                    };
                    let result = if num(e, "hits") > 0.0 || num(e, "kills") > 0.0 {
                        Result::EntityImpact
                    } else if num(e, "blasts") > 0.0 {
                        Result::EntityProximate
                    } else {
                        Result::GroundImpact
                    };
                    let at = pos(sim, t).unwrap_or(from);
                    out.push(Outgoing::Detonation(DetonationOut { event: ev, firer: Some(firer), target: Some(t), munition, rounds, at, result }));
                }
                "fires.fire" | "fires.adjust" => {
                    let ev = self.event();
                    self.missions.insert(num(e, "mission") as i64, ev);
                    let range_m = num(e, "rangeM");
                    out.push(Outgoing::Fire(FireOut { event: ev, firer, target: e.target, munition, rounds, from, range_m }));
                }
                "fires.impact" => {
                    let ev = self.missions.get(&(num(e, "mission") as i64)).copied().unwrap_or_else(|| self.event());
                    let at = Enu::new(num(e, "centreE"), num(e, "centreN"), 0.0);
                    let at = Enu::new(at.e, at.n, sim.ctx.ground.height(at.e, at.n));
                    out.push(Outgoing::Detonation(DetonationOut { event: ev, firer: Some(firer), target: None, munition, rounds, at, result: Result::GroundImpact }));
                }
                "fire.suppress" => {
                    let ev = self.event();
                    let spot = Enu::new(num(e, "spotE"), num(e, "spotN"), 0.0);
                    let spot = Enu::new(spot.e, spot.n, sim.ctx.ground.height(spot.e, spot.n));
                    out.push(Outgoing::Fire(FireOut { event: ev, firer, target: e.target, munition: munition.clone(), rounds, from, range_m: num(e, "rangeM") }));
                    out.push(Outgoing::Detonation(DetonationOut { event: ev, firer: Some(firer), target: None, munition, rounds, at: spot, result: Result::GroundImpact }));
                }
                _ => {}
            }
        }
        out
    }
}
