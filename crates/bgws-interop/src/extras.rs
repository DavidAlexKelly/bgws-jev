//! What a game publishes beyond entities and fire (docs/design/INTEROP_PLAN.md
//! phase 4), protocol-free: radios and the messages they carry, radars and
//! jammers, minefields, and aggregated formations. Read from the sim's own
//! state; only our entities' (a remote entity's owner publishes its own).

use crate::model::ExternalId;
use bgws_core::{Enu, EntityId, Side, SimTime};
use bgws_scenario::v4::InteropSpec;
use bgws_sim::components::{Health, Ident, Kin};
use bgws_sim::domains::drones::C4;
use bgws_sim::Sim;

/// A radio's state (SISO-REF-010 Transmit State).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RadioState {
    Off = 0,
    On = 1,
    Transmitting = 2,
}

/// One of an entity's radios.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioTruth {
    pub id: EntityId,
    /// 1, 2, … in the entity's fit.
    pub number: u16,
    pub bearer: bgws_ops::comms::BearerKind,
    pub freq_hz: f64,
    pub bw_hz: f64,
    pub power_w: f64,
    pub state: RadioState,
}

/// A radar or jammer on the air this second.
#[derive(Clone, Debug, PartialEq)]
pub struct EmitterTruth {
    pub id: EntityId,
    /// 1, 2, … among the entity's emitters.
    pub number: u8,
    pub jammer: bool,
    pub freq_hz: f64,
    pub bw_hz: f64,
    pub erp_w: f64,
    pub duty: f64,
}

/// A message our comms model carried (or lost).
#[derive(Clone, Debug, PartialEq)]
pub struct MessageTruth {
    pub from: EntityId,
    pub to: EntityId,
    pub what: String,
    pub sent: SimTime,
    pub done: SimTime,
    pub delivered: bool,
    pub bearer: Option<String>,
}

/// A land minefield.
#[derive(Clone, Debug, PartialEq)]
pub struct MinefieldTruth {
    /// 1, 2, … among the scenario's minefields.
    pub number: u16,
    pub name: String,
    pub owner: Side,
    pub centre: Enu,
    /// Across its front, and its depth, m.
    pub width_m: f64,
    pub depth_m: f64,
    /// Radians clockwise from north: the direction of its depth.
    pub bearing: f64,
    pub density_m2: f64,
    /// Lanes breached through it.
    pub lanes: usize,
}

/// A formation the level of detail has aggregated.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateTruth {
    /// Its formation's index (stable for the game).
    pub number: u16,
    pub side: Side,
    pub name: String,
    /// armour, mech, infantry, fires or support.
    pub kind: String,
    pub members: Vec<EntityId>,
    pub centre: Enu,
    pub radius_m: f64,
    pub velocity: Enu,
    pub aggregated: bool,
}

fn ours(sim: &Sim, spec: &InteropSpec, id: EntityId) -> bool {
    sim.ctx.order.get(id.index()).is_some_and(|&e| !bgws_sim::remote::is_remote(sim, e)) && ExternalId::ours(spec, id).is_some()
}

fn alive(sim: &Sim, id: EntityId) -> bool {
    sim.ctx.order.get(id.index()).and_then(|&e| sim.world.get::<Health>(e)).is_some_and(|h| !h.destroyed)
}

/// Every radio our entities carry that radiates, and whether it is
/// transmitting (a transmission in the last second).
pub fn radios(sim: &Sim, spec: &InteropSpec) -> Vec<RadioTruth> {
    let Some(c4) = sim.world.get_resource::<C4>() else { return vec![] };
    let now = sim.ctx.now;
    let mut out = vec![];
    for (&id, role) in &c4.roles {
        if !ours(sim, spec, id) {
            continue;
        }
        let up = alive(sim, id);
        let last = c4.ew.last_radio.get(&id).filter(|(t, _)| (now - *t).secs() <= 1.0);
        for (k, fit) in role.radios.iter().enumerate().filter(|(_, f)| f.kind.radiates()) {
            let tx = last.filter(|(_, x)| (x.freq_hz - fit.freq_hz).abs() < 1.0 || role.radios.len() == 1);
            let state = match (up, tx) {
                (false, _) => RadioState::Off,
                (true, Some(_)) => RadioState::Transmitting,
                (true, None) => RadioState::On,
            };
            out.push(RadioTruth { id, number: k as u16 + 1, bearer: fit.kind, freq_hz: fit.freq_hz, bw_hz: tx.map(|(_, x)| x.bw_hz).unwrap_or(25e3), power_w: fit.power_w, state });
        }
    }
    out
}

/// Our radars and jammers on the air this second.
pub fn emitters(sim: &Sim, spec: &InteropSpec) -> Vec<EmitterTruth> {
    let Some(c4) = sim.world.get_resource::<C4>() else { return vec![] };
    let mut count: std::collections::BTreeMap<EntityId, u8> = Default::default();
    let mut out = vec![];
    for t in c4.ew.last_tx.iter().filter(|t| t.what == "radar" || t.what == "jammer") {
        if !ours(sim, spec, t.entity) {
            continue;
        }
        let n = count.entry(t.entity).or_insert(0);
        *n = n.saturating_add(1);
        out.push(EmitterTruth { id: t.entity, number: *n, jammer: t.what == "jammer", freq_hz: t.freq_hz, bw_hz: t.bw_hz, erp_w: t.erp_w, duty: t.duty });
    }
    out
}

/// Messages our entities' comms finished (delivered or lost) after `after`.
pub fn messages(sim: &Sim, spec: &InteropSpec, after: SimTime) -> Vec<MessageTruth> {
    let Some(c4) = sim.world.get_resource::<C4>() else { return vec![] };
    c4.comms.recent.iter().filter(|m| m.done > after && ours(sim, spec, m.from)).map(|m| MessageTruth { from: m.from, to: m.to, what: m.what.clone(), sent: m.sent, done: m.done, delivered: m.delivered, bearer: m.bearer.clone() }).collect()
}

/// The land minefields (any side's: a minefield is ground, not an entity).
pub fn minefields(sim: &Sim) -> Vec<MinefieldTruth> {
    let Some(f) = sim.world.get_resource::<bgws_sim::domains::fires::FiresWorld>() else { return vec![] };
    f.obstacles
        .obstacles
        .iter()
        .filter(|o| o.kind == bgws_scenario::v4::ObstacleKind::Minefield)
        .enumerate()
        .map(|(k, o)| MinefieldTruth { number: k as u16 + 1, name: o.id.clone(), owner: o.owner, centre: o.centre, width_m: o.width_m, depth_m: o.depth_m, bearing: o.bearing, density_m2: o.density_m2, lanes: o.lanes.len() })
        .collect()
}

/// Our formations: where they are and whether aggregated.
pub fn aggregates(sim: &Sim, spec: &InteropSpec) -> Vec<AggregateTruth> {
    let Some(lod) = sim.world.get_resource::<bgws_sim::domains::lod::LodState>() else { return vec![] };
    let mut out = vec![];
    for (k, f) in lod.formations.iter().enumerate() {
        let members: Vec<EntityId> = f.members.iter().copied().filter(|&m| ours(sim, spec, m) && alive(sim, m)).collect();
        let kin: Vec<Kin> = members.iter().filter_map(|&m| sim.world.get::<Kin>(sim.ctx.entity(m)).copied()).collect();
        if kin.is_empty() {
            continue;
        }
        let n = kin.len() as f64;
        let centre = Enu::new(kin.iter().map(|k| k.pos.e).sum::<f64>() / n, kin.iter().map(|k| k.pos.n).sum::<f64>() / n, kin.iter().map(|k| k.pos.u).sum::<f64>() / n);
        let velocity = Enu::new(kin.iter().map(|k| k.vel.e).sum::<f64>() / n, kin.iter().map(|k| k.vel.n).sum::<f64>() / n, 0.0);
        let radius_m = kin.iter().map(|k| k.pos.sub(centre).horizontal()).fold(0.0, f64::max);
        let side = members.first().and_then(|&m| sim.world.get::<Ident>(sim.ctx.entity(m))).map(|i| i.side).unwrap_or(f.side);
        out.push(AggregateTruth { number: k as u16 + 1, side, name: f.name.clone(), kind: f.kind.clone(), members, centre, radius_m, velocity, aggregated: f.aggregated() });
    }
    out
}

/// A laser designation one of ours is keeping.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignationTruth {
    pub id: u64,
    pub designator: EntityId,
    pub target: Option<EntityId>,
    /// East, north, height above mean sea level.
    pub spot: Enu,
    pub code: u16,
    pub lasing: bool,
}

/// Our designations now.
pub fn designations(sim: &Sim, spec: &InteropSpec) -> Vec<DesignationTruth> {
    let Some(d) = sim.world.get_resource::<bgws_sim::designation::Designations>() else { return vec![] };
    d.active.iter().filter(|x| ours(sim, spec, x.designator)).map(|x| DesignationTruth { id: x.id, designator: x.designator, target: x.target, spot: x.spot, code: x.code, lasing: x.lasing }).collect()
}

/// A smoke screen in the world (each is ours: we released it).
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenTruth {
    pub id: u64,
    pub side: Side,
    /// Its centre now (drifted), on the ground.
    pub centre: Enu,
    /// How far it reaches, m.
    pub radius_m: f64,
    pub mass_g: f64,
    pub kind: bgws_models::signature::SmokeKind,
}

pub fn screens(sim: &Sim) -> Vec<ScreenTruth> {
    let Some(st) = sim.world.get_resource::<bgws_sim::smoke::SmokeState>() else { return vec![] };
    st.screens
        .iter()
        .map(|s| {
            let (c, r) = s.extent(sim.ctx.now, &st.sig);
            ScreenTruth { id: s.id, side: s.side, centre: Enu::new(c.e, c.n, sim.ctx.ground.height(c.e, c.n)), radius_m: r, mass_g: s.mass_g, kind: s.kind }
        })
        .collect()
}
