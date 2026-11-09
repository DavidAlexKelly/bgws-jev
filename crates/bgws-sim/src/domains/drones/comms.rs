//! The nets (§8.5): each side's radios as a `bgws_ops::comms::CommsNetwork`.
//!
//! - **Nets** follow the command tree: an HQ's post and its subordinates
//!   share a net per bearer they both carry (VHF, UHF, HF, satcom, data
//!   link, mesh); wire and fibre run post to post. HQ posts relay between
//!   nets, so an order from a battalion reaches a platoon through its
//!   company post.
//! - **Link SINR** comes from `bgws_prop` terrain path loss (Deygout
//!   diffraction and clutter) between the radios' antennas, against the
//!   noise floor plus the J at each receiver from every jammer (§8.7), and
//!   is refreshed every 10 s. Satcom links hold while the satellite is up.
//! - **Messages:** contact reports and updates from units, situation
//!   reports, intent heartbeats, orders, target hand-offs and track batches
//!   between HQs, each with its kind's size and PACE plan. The network
//!   decides when, by what bearer and whether they arrive; this module only
//!   says what happens when they do.
//! - **Reports:** what the land loop's sensing puts in `ctx.reports` is
//!   taken from there each second and routed as reports to the observer's
//!   HQ when the scope's comms rule is radio; anything with no node keeps
//!   the land loop's fixed delay. Self-position error (§8.7 PNT) goes into
//!   each reported fix.
//! - Every transmission is an emission handed to ESM.

use super::{alive_id, ent, pos_of, side_of, C4};
use crate::components::*;
use crate::hq::UnitOrder;
use crate::sim::Sim;
use bgws_catalogue::Domain;
use bgws_core::{Enu, EntityId, Geodetic, MessageId, Seed, Side, SimDuration, SimTime, TrackId};
use bgws_know::{Detection, Measurement, TrackView};
use bgws_ops::comms::{BearerKind, BearerSpec, CommsNetwork, CommsOutcome, CryptoClass, LinkPhysics, Message, MessageKind, NetId, NodeId, PacePlan};
use bgws_scenario::v4::CommsRule;
use nalgebra::Matrix3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a message carries.
#[derive(Clone, Debug)]
pub enum Payload {
    /// Sightings from a unit (a spot report if any contact is new, else an update).
    Contact { observer: EntityId, dets: Vec<Detection>, new: bool },
    /// Tracks an HQ forwards to its parent.
    Tracks { from: EntityId, views: Vec<TrackView> },
    /// A unit's situation report.
    Sitrep(super::c2::UnitReport),
    /// The HQ's intent stands (a heartbeat).
    Intent { hq: usize },
    /// One unit's orders from a plan.
    Order { hq: usize, side: Side, order: UnitOrder, refs: BTreeMap<String, Geodetic>, plan: String, issued: SimTime },
    /// A target handed to a strike team.
    /// `cue` is the sighting that started the chain (detect-to-strike starts there).
    Target { track: TrackId, at: Enu, value: f64, seen: SimTime, cue: SimTime },
}

impl Payload {
    pub fn word(&self) -> &'static str {
        match self {
            Payload::Contact { new: true, .. } => "spotrep",
            Payload::Contact { .. } => "update",
            Payload::Tracks { .. } => "tracks",
            Payload::Sitrep(_) => "sitrep",
            Payload::Intent { .. } => "intent",
            Payload::Order { .. } => "order",
            Payload::Target { .. } => "target",
        }
    }
}

/// A message on the nets.
#[derive(Clone, Debug)]
pub struct InFlight {
    pub from: EntityId,
    pub to: EntityId,
    pub kind: MessageKind,
    pub sent: SimTime,
    pub payload: Payload,
}

/// A message that arrived, for the C2 layer to act on.
#[derive(Clone, Debug)]
pub struct Arrived {
    pub from: EntityId,
    pub to: EntityId,
    pub sent: SimTime,
    pub at: SimTime,
    pub bearer: Option<BearerKind>,
    pub payload: Payload,
}

/// One net, as built from the command tree.
#[derive(Clone, Debug)]
pub struct NetRec {
    pub id: NetId,
    pub side: Side,
    pub kind: BearerKind,
    pub members: Vec<EntityId>,
    pub freq_hz: f64,
    pub bw_hz: f64,
    pub hq: usize,
}

/// A unit's sightings waiting for its next report.
#[derive(Clone, Debug)]
pub struct Buffer {
    pub dets: Vec<Detection>,
    pub last_sent: SimTime,
    /// Targets it has reported (from its own sightings).
    pub reported: BTreeSet<EntityId>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer { dets: Vec::new(), last_sent: SimTime::from_secs(-1e6), reported: BTreeSet::new() }
    }
}

/// Counts and delays, by message word, for the AAR and the tests.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommsStats {
    pub sent: BTreeMap<String, u32>,
    pub delivered: BTreeMap<String, u32>,
    pub lost: BTreeMap<String, u32>,
    pub latency_s: BTreeMap<String, f64>,
    /// Deliveries by the bearer that carried them, and by PACE level.
    pub by_bearer: BTreeMap<String, u32>,
}

#[derive(Default)]
pub struct CommsState {
    /// The scope's comms rule is radio and the network is built.
    pub active: bool,
    pub net: Option<CommsNetwork>,
    pub nets: Vec<NetRec>,
    pub nodes: BTreeSet<EntityId>,
    pub dead: BTreeSet<EntityId>,
    pub next_msg: u32,
    pub flying: BTreeMap<u32, InFlight>,
    pub buffers: BTreeMap<EntityId, Buffer>,
    pub last_sitrep: BTreeMap<EntityId, SimTime>,
    /// (side, LEO only) → until.
    pub satcom_down: BTreeMap<(Side, bool), Option<SimTime>>,
    /// Units whose radios are out (damage, a test, a cyber effect).
    pub cut: BTreeSet<EntityId>,
    pub stats: CommsStats,
    /// Arrivals the C2 layer has not yet taken.
    pub arrived: Vec<Arrived>,
    /// The latest messages delivered or lost, with their hops and fallbacks
    /// (§16 link-budget inspector), oldest first.
    pub recent: std::collections::VecDeque<MessageRecord>,
}

/// How many finished messages the link-budget inspector keeps.
const RECENT_MESSAGES: usize = 4_000;

/// A finished message: where it went, how, and why it was slow or lost.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub from: EntityId,
    pub to: EntityId,
    pub what: String,
    pub sent: SimTime,
    pub done: SimTime,
    pub delivered: bool,
    pub bearer: Option<String>,
    pub reason: Option<String>,
    pub hops: Vec<bgws_ops::comms::HopRecord>,
    pub fallbacks: Vec<bgws_ops::comms::BearerFailure>,
}

fn remember(c4: &mut C4, r: MessageRecord) {
    if c4.comms.recent.len() >= RECENT_MESSAGES {
        c4.comms.recent.pop_front();
    }
    c4.comms.recent.push_back(r);
}

/// A unit's latest `n` messages, sent or received, newest first.
pub fn messages_of(s: &Sim, unit: EntityId, n: usize) -> Vec<MessageRecord> {
    s.world.get_resource::<C4>().map(|c| c.comms.recent.iter().rev().filter(|m| m.from == unit || m.to == unit).take(n).cloned().collect()).unwrap_or_default()
}

fn node(id: EntityId) -> NodeId {
    NodeId(id.0)
}

fn net_bearer(kind: BearerKind, radio: &super::RadioFit, f: &bgws_core::Figures) -> BearerSpec {
    let mut b = BearerSpec::from_figures(kind, f);
    if radio.freq_hz > 0.0 && kind.radiates() {
        b.freq_hz = radio.freq_hz;
    }
    if radio.power_w > 0.0 && kind.radiates() {
        b.power_w = radio.power_w;
    }
    if kind.radiates() {
        b.antenna_gain_dbi = radio.gain_dbi;
    }
    if radio.rate_bps.is_finite() && radio.rate_bps > 1.0 && !matches!(kind, BearerKind::Courier) {
        b.rate_bps = radio.rate_bps;
    }
    b
}

/// Build the nodes, nets and PACE plans.
pub fn setup(s: &mut Sim, c4: &mut C4) {
    let rule = s.ctx.scenario.comms;
    let land = super::mode(s, bgws_scenario::scope::ScopeDomain::Land);
    if rule != CommsRule::Radio || land == bgws_scenario::scope::DomainMode::Off || c4.c2.hqs.is_empty() {
        return;
    }
    let seed = Seed(s.ctx.seed.0 ^ bgws_core::rng::fnv1a(b"bgws-sim.comms"));
    let mut net = CommsNetwork::new(&s.ctx.figures, seed);
    let posts: BTreeSet<EntityId> = c4.c2.hqs.iter().map(|h| h.post).collect();
    for (id, role) in &c4.roles {
        let Some(e) = ent(s, *id) else { continue };
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        if !matches!(i.domain, Domain::Land | Domain::Node | Domain::Surface) && role.radios.is_empty() {
            continue;
        }
        let pos = s.world.get::<Kin>(e).map(|k| k.pos);
        net.add_node(node(*id), pos, posts.contains(id));
        c4.comms.nodes.insert(*id);
    }
    c4.comms.net = Some(net);
    for hi in 0..c4.c2.hqs.len() {
        build_hq_nets(s, c4, hi);
    }
    let mut net = c4.comms.net.take().expect("net");
    // PACE (primary, alternate, contingency, emergency … then a runner).
    use BearerKind as B;
    net.set_pace(MessageKind::VoiceContactReport, PacePlan { levels: vec![B::Vhf, B::Uhf, B::Mesh, B::Hf, B::SatcomLeo, B::SatcomGeo, B::Courier] });
    net.set_pace(MessageKind::DataLinkTrack, PacePlan { levels: vec![B::Tdl, B::Wire, B::Fibre, B::SatcomLeo, B::SatcomGeo, B::Hf, B::Vhf, B::Courier] });
    net.set_pace(MessageKind::PlanOrder, PacePlan { levels: vec![B::Vhf, B::Tdl, B::Hf, B::SatcomLeo, B::SatcomGeo, B::Courier] });
    net.set_pace(MessageKind::Image, PacePlan { levels: vec![B::SatcomLeo, B::Mesh, B::Tdl, B::SatcomGeo, B::Courier] });
    c4.comms.net = Some(net);
    c4.comms.active = true;
    let n_nets = c4.comms.nets.len();
    refresh(s, c4);
    s.ctx.log.push(s.ctx.now, "comms.setup").text(format!("{} nodes on {} nets", c4.comms.nodes.len(), n_nets)).data(serde_json::json!({
        "nets": c4.comms.nets.iter().map(|n| serde_json::json!({ "id": n.id.0, "side": n.side, "bearer": n.kind, "members": n.members.len() })).collect::<Vec<_>>()
    }));
}

/// The nets of one HQ: its post and its subordinates on each bearer they
/// share; wire and fibre post to post.
fn build_hq_nets(s: &Sim, c4: &mut C4, hi: usize) {
    let kinds = [BearerKind::Vhf, BearerKind::Uhf, BearerKind::Hf, BearerKind::SatcomGeo, BearerKind::SatcomLeo, BearerKind::Tdl, BearerKind::Mesh];
    let posts: BTreeSet<EntityId> = c4.c2.hqs.iter().map(|h| h.post).collect();
    let h = &c4.c2.hqs[hi];
    let (side, post) = (h.side, h.post);
    let mut members: Vec<EntityId> = vec![post];
    members.extend(h.subs.iter().copied().filter(|x| *x != post));
    members.dedup();
    let Some(post_role) = c4.roles.get(&post).cloned() else { return };
    let mut next = c4.comms.nets.iter().map(|n| n.id.0 + 1).max().unwrap_or(0);
    let mut add: Vec<(NetRec, BearerSpec)> = Vec::new();
    for k in kinds {
        let Some(radio) = post_role.radio(k) else { continue };
        let m: Vec<EntityId> = members.iter().copied().filter(|x| c4.roles.get(x).map(|r| r.has(k)).unwrap_or(false) && c4.comms.nodes.contains(x) && !c4.comms.dead.contains(x)).collect();
        if m.len() < 2 {
            continue;
        }
        let spec = net_bearer(k, radio, &s.ctx.figures);
        add.push((NetRec { id: NetId(next), side, kind: k, members: m, freq_hz: spec.freq_hz, bw_hz: spec.waveform.bandwidth_hz, hq: hi }, spec));
        next += 1;
    }
    for k in [BearerKind::Wire, BearerKind::Fibre] {
        let Some(radio) = post_role.radio(k) else { continue };
        for other in members.iter().skip(1) {
            if !posts.contains(other) || !c4.roles.get(other).map(|r| r.has(k)).unwrap_or(false) {
                continue;
            }
            let spec = net_bearer(k, radio, &s.ctx.figures);
            add.push((NetRec { id: NetId(next), side, kind: k, members: vec![post, *other], freq_hz: 0.0, bw_hz: 0.0, hq: hi }, spec));
            next += 1;
        }
    }
    let Some(net) = c4.comms.net.as_mut() else { return };
    for (rec, spec) in add {
        if net.add_net(rec.id, spec, rec.members.iter().map(|x| node(*x)), &s.ctx.figures).is_ok() {
            c4.comms.nets.push(rec);
        }
    }
}

/// A new post takes an HQ's seat: it relays, and gets nets to the HQ's units.
pub fn join_nets(s: &mut Sim, c4: &mut C4, h: usize) {
    if !c4.comms.active {
        return;
    }
    let post = c4.c2.hqs[h].post;
    if let Some(net) = c4.comms.net.as_mut() {
        net.add_node(node(post), pos_of(s, post), true);
    }
    build_hq_nets(s, c4, h);
    refresh(s, c4);
}

/// Propagation for the network: terrain path loss and the J at each receiver.
struct Phys {
    loss: BTreeMap<(u32, u32, u64), f64>,
    interf: BTreeMap<(u32, u64), f64>,
}

fn fkey(f: f64) -> u64 {
    (f / 1e3).round().max(0.0) as u64
}

impl LinkPhysics for Phys {
    fn path_loss_db(&self, from: NodeId, to: NodeId, freq_hz: f64) -> f64 {
        let k = (from.0.min(to.0), from.0.max(to.0), fkey(freq_hz));
        self.loss.get(&k).copied().unwrap_or(250.0)
    }
    fn interference_dbw(&self, rx: NodeId, freq_hz: f64, _bw: f64) -> Option<f64> {
        self.interf.get(&(rx.0, fkey(freq_hz))).copied().filter(|j| j.is_finite())
    }
}

/// Node positions, dead and cut nodes, and every radio link's SINR.
pub fn refresh(s: &Sim, c4: &mut C4) {
    if !c4.comms.active {
        return;
    }
    let now = s.ctx.now;
    // Positions and life.
    let posts: BTreeSet<EntityId> = c4.c2.hqs.iter().filter(|h| h.up()).map(|h| h.post).collect();
    let mut gone = Vec::new();
    {
        let net = c4.comms.net.as_mut().expect("active");
        for id in &c4.comms.nodes {
            if c4.comms.dead.contains(id) {
                continue;
            }
            if !alive_id(s, *id) || c4.comms.cut.contains(id) {
                let _ = net.destroy_node(node(*id));
                gone.push(*id);
                continue;
            }
            net.add_node(node(*id), pos_of(s, *id), posts.contains(id));
        }
    }
    c4.comms.dead.extend(gone);
    // Path losses on every radio net, in one parallel batch.
    let mut queries = Vec::new();
    let mut owners = Vec::new();
    for (ni, n) in c4.comms.nets.iter().enumerate() {
        if !n.kind.radiates() || matches!(n.kind, BearerKind::SatcomGeo | BearerKind::SatcomLeo) {
            continue;
        }
        for (i, a) in n.members.iter().enumerate() {
            for b in &n.members[i + 1..] {
                let (Some(pa), Some(pb)) = (pos_of(s, *a), pos_of(s, *b)) else { continue };
                let ha = c4.roles.get(a).and_then(|r| r.radio(n.kind)).map(|r| r.agl).unwrap_or(c4.p.ground_antenna_m);
                let hb = c4.roles.get(b).and_then(|r| r.radio(n.kind)).map(|r| r.agl).unwrap_or(c4.p.ground_antenna_m);
                queries.push(super::LossQuery { a: a.0 as u64, b: b.0 as u64, f_hz: n.freq_hz, pa, agl_a: ha, pb, agl_b: hb });
                owners.push(ni);
            }
        }
    }
    let losses = c4.losses.resolve(&s.ctx.ground.terrain, &c4.radio, c4.p.move_tolerance_m, now, &queries);
    let mut phys = Phys { loss: BTreeMap::new(), interf: BTreeMap::new() };
    for (q, l) in queries.iter().zip(&losses) {
        phys.loss.insert(((q.a.min(q.b)) as u32, (q.a.max(q.b)) as u32, fkey(q.f_hz)), *l);
    }
    // The J at every receiver of every radio net.
    let mut victims = Vec::new();
    for n in &c4.comms.nets {
        if !n.kind.radiates() || matches!(n.kind, BearerKind::SatcomGeo | BearerKind::SatcomLeo) {
            continue;
        }
        for m in &n.members {
            let (Some(p), Some(side)) = (pos_of(s, *m), side_of(s, *m)) else { continue };
            let agl = c4.roles.get(m).and_then(|r| r.radio(n.kind)).map(|r| r.agl).unwrap_or(c4.p.ground_antenna_m);
            let gain = c4.roles.get(m).and_then(|r| r.radio(n.kind)).map(|r| r.gain_dbi).unwrap_or(0.0);
            victims.push(super::ew::Victim { key: m.0 as u64, pos: p, agl, side, freq_hz: n.freq_hz, bw_hz: n.bw_hz, gnss: false, rx_gain_dbi: gain });
        }
    }
    let js = super::ew::jamming_dbw(s, c4, &victims);
    // Combat-net radios hop: a jammer that cannot follow loses the processing gain.
    let fhss = c4.ew.ew.as_ref().map(|p| p.fhss_gain_db).unwrap_or(0.0);
    for (v, j) in victims.iter().zip(js) {
        let hopping = c4.comms.nets.iter().any(|n| n.freq_hz == v.freq_hz && matches!(n.kind, BearerKind::Vhf | BearerKind::Uhf));
        let j = if hopping { j - fhss } else { j };
        if j.is_finite() {
            phys.interf.insert((v.key as u32, fkey(v.freq_hz)), j);
        }
    }
    let net = c4.comms.net.as_mut().expect("active");
    net.refresh_links(&phys);
    apply_satcom_links(s, c4);
}

fn apply_satcom_links(s: &Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let sinr = c4.p.satcom_sinr_db;
    let down = c4.comms.satcom_down.clone();
    let Some(net) = c4.comms.net.as_mut() else { return };
    for n in &c4.comms.nets {
        if !matches!(n.kind, BearerKind::SatcomGeo | BearerKind::SatcomLeo) {
            continue;
        }
        let is_down = down.iter().any(|((side, leo_only), until)| *side == n.side && (!leo_only || n.kind == BearerKind::SatcomLeo) && until.map(|u| u > now).unwrap_or(true));
        let _ = net.set_net_up(n.id, !is_down);
        for (i, a) in n.members.iter().enumerate() {
            for b in &n.members[i + 1..] {
                net.set_link_sinr(n.id, node(*a), node(*b), sinr);
            }
        }
    }
}

/// Satcom up or down after a scripted event.
pub fn apply_satcom_state(s: &Sim, c4: &mut C4) {
    apply_satcom_links(s, c4);
}

/// Hand a message to the network now. False if comms are off or an end has no node.
#[allow(clippy::too_many_arguments)]
pub fn send(now: SimTime, c4: &mut C4, from: EntityId, to: EntityId, kind: MessageKind, priority: u8, size_bits: Option<f64>, payload: Payload) -> bool {
    if !c4.comms.active || !c4.comms.nodes.contains(&from) || !c4.comms.nodes.contains(&to) {
        return false;
    }
    let word = payload.word().to_string();
    *c4.comms.stats.sent.entry(word.clone()).or_insert(0) += 1;
    if from == to {
        c4.comms.arrived.push(Arrived { from, to, sent: now, at: now, bearer: None, payload });
        *c4.comms.stats.delivered.entry(word).or_insert(0) += 1;
        return true;
    }
    let id = c4.comms.next_msg;
    c4.comms.next_msg += 1;
    let crypto = if kind == MessageKind::VoiceContactReport { CryptoClass::ComsecVoice } else { CryptoClass::Type1Data };
    let msg = Message { id: MessageId(id), kind, from: node(from), to: node(to), priority, crypto, size_bits };
    let net = c4.comms.net.as_mut().expect("active");
    if net.send(msg, now).is_err() {
        return false;
    }
    c4.comms.flying.insert(id, InFlight { from, to, kind, sent: now, payload });
    true
}

/// Queue a detection in a unit's next report (ESM fixes, drone camera fixes).
/// Without nets it goes to the side picture after the land loop's delay.
pub fn queue_detection(s: &mut Sim, c4: &mut C4, observer: EntityId, side: Side, det: Detection) {
    if c4.comms.active && c4.comms.nodes.contains(&observer) && c4.c2.unit_hq.contains_key(&observer) {
        push_buffer(c4, observer, det);
    } else {
        let delay = SimDuration::from_secs(s.ctx.p.report_delay_s);
        s.ctx.reports.push((s.ctx.now + delay, side, det));
    }
}

fn det_pos(d: &Detection) -> Option<Enu> {
    match &d.measurement {
        Measurement::Position { pos, .. } => Some(*pos),
        Measurement::RangeBearing { origin, range, bearing, .. } => Some(origin.add(Enu::new(bearing.sin() * range, bearing.cos() * range, 0.0))),
        Measurement::Bearing { .. } => None,
    }
}

fn push_buffer(c4: &mut C4, observer: EntityId, det: Detection) {
    let merge = c4.p.merge_m;
    let max = c4.p.buffer_max;
    let b = c4.comms.buffers.entry(observer).or_default();
    let p = det_pos(&det);
    if let Some(p) = p {
        if let Some(i) = b.dets.iter().position(|x| x.sensor == det.sensor && det_pos(x).map(|q| q.sub(p).horizontal() <= merge).unwrap_or(false)) {
            b.dets[i] = det;
            return;
        }
    }
    b.dets.push(det);
    if b.dets.len() > max {
        b.dets.remove(0);
    }
}

/// Self-position error in a reported fix: the observer's bias, and its σ added.
fn with_nav_error(c4: &C4, now: SimTime, mut d: Detection) -> Detection {
    let Some(rec) = c4.pnt.recs.get(&d.observer) else { return d };
    let sp = rec.nav.self_position(now);
    if let Measurement::Position { pos, cov } = &mut d.measurement {
        *pos = pos.add(sp.bias);
        *cov += Matrix3::from_diagonal(&nalgebra::Vector3::new(sp.sigma_m * sp.sigma_m, sp.sigma_m * sp.sigma_m, 0.0));
    }
    d
}

/// 1 Hz: take the land loop's reports, send what is due, run the network.
pub fn base(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    if !c4.comms.active {
        super::put(s, c4);
        return;
    }
    let now = s.ctx.now;
    // Dead and cut nodes leave the network at once.
    let mut newly_dead = Vec::new();
    for id in &c4.comms.nodes {
        if !c4.comms.dead.contains(id) && (!alive_id(s, *id) || c4.comms.cut.contains(id)) {
            newly_dead.push(*id);
        }
    }
    for id in newly_dead {
        if let Some(net) = c4.comms.net.as_mut() {
            let _ = net.destroy_node(node(id));
        }
        c4.comms.dead.insert(id);
    }
    // The land loop's reports: route those whose observer is on the nets.
    let reports = std::mem::take(&mut s.ctx.reports);
    let mut keep = Vec::new();
    for (due, side, d) in reports {
        let o = d.observer;
        if c4.comms.nodes.contains(&o) && c4.c2.unit_hq.contains_key(&o) {
            let d = with_nav_error(&c4, now, d);
            push_buffer(&mut c4, o, d);
        } else {
            keep.push((due, side, d));
        }
    }
    s.ctx.reports = keep;
    // Reports due out.
    let ids: Vec<EntityId> = c4.comms.buffers.iter().filter(|(_, b)| !b.dets.is_empty()).map(|(k, _)| *k).collect();
    for id in ids {
        if !alive_id(s, id) || !c4.may_transmit(id) {
            continue;
        }
        let Some(dest) = c4.c2.report_dest(id) else { continue };
        let seen_new: Vec<EntityId> = {
            let b = &c4.comms.buffers[&id];
            ent(s, id).and_then(|e| s.world.get::<Aware>(e)).map(|a| a.seen.keys().filter(|t| !b.reported.contains(t)).copied().collect()).unwrap_or_default()
        };
        let b = c4.comms.buffers.get(&id).expect("buffer");
        let since = (now - b.last_sent).secs();
        let is_new = !seen_new.is_empty();
        let due = if is_new { since >= c4.p.new_contact_gap_s } else { since >= c4.p.update_interval_s };
        if !due {
            continue;
        }
        let b = c4.comms.buffers.get_mut(&id).expect("buffer");
        let dets = std::mem::take(&mut b.dets);
        b.last_sent = now;
        b.reported.extend(seen_new);
        let n = dets.len() as f64;
        let size = Some(s.ctx.figures.get("comms.msg.voice_as_data_bits") * n.max(1.0));
        let payload = Payload::Contact { observer: id, dets, new: is_new };
        send(now, &mut c4, id, dest, MessageKind::VoiceContactReport, if is_new { 1 } else { 3 }, size, payload);
    }
    // Situation reports.
    let units: Vec<EntityId> = c4.c2.unit_hq.keys().copied().collect();
    for id in units {
        if !alive_id(s, id) || !c4.may_transmit(id) || c4.c2.post_of(id).is_some() {
            continue;
        }
        let last = c4.comms.last_sitrep.get(&id).copied().unwrap_or(SimTime::from_secs(-1e6));
        let phase = (id.0 as f64 * 37.0) % c4.p.sitrep_s;
        if (now - last).secs() < c4.p.sitrep_s || now.secs() < phase {
            continue;
        }
        if let Some(dest) = c4.c2.report_dest(id) {
            let rep = super::c2::own_report(s, &c4, id);
            c4.comms.last_sitrep.insert(id, now);
            send(now, &mut c4, id, dest, MessageKind::VoiceContactReport, 4, None, Payload::Sitrep(rep));
        }
    }
    // Run the network to now.
    let outcomes = c4.comms.net.as_mut().expect("active").step(now);
    for o in outcomes {
        let id = o.msg().0;
        let Some(m) = c4.comms.flying.remove(&id) else { continue };
        let word = m.payload.word().to_string();
        match o {
            CommsOutcome::Delivered(d) => {
                remember(&mut c4, MessageRecord { from: m.from, to: m.to, what: word.clone(), sent: m.sent, done: d.delivered_at, delivered: true, bearer: Some(format!("{:?}", d.bearer)), reason: None, hops: d.hops.clone(), fallbacks: d.fallbacks.clone() });
                *c4.comms.stats.delivered.entry(word.clone()).or_insert(0) += 1;
                *c4.comms.stats.latency_s.entry(word).or_insert(0.0) += d.latency_s();
                *c4.comms.stats.by_bearer.entry(format!("{:?}/{}", d.bearer, d.pace_level)).or_insert(0) += 1;
                c4.comms.arrived.push(Arrived { from: m.from, to: m.to, sent: m.sent, at: d.delivered_at, bearer: Some(d.bearer), payload: m.payload });
            }
            CommsOutcome::Lost(l) => {
                remember(&mut c4, MessageRecord { from: m.from, to: m.to, what: word.clone(), sent: m.sent, done: l.lost_at, delivered: false, bearer: None, reason: Some(format!("{:?}", l.reason)), hops: l.hops.clone(), fallbacks: l.fallbacks.clone() });
                *c4.comms.stats.lost.entry(word.clone()).or_insert(0) += 1;
                if matches!(m.payload, Payload::Order { .. } | Payload::Contact { new: true, .. } | Payload::Target { .. }) {
                    let text = format!("{} from {} to {} lost: {:?}", word, crate::systems::name(s, m.from), crate::systems::name(s, m.to), l.reason);
                    let fs = side_of(s, m.from).unwrap_or(Side::Neutral);
                    s.ctx.log.push(now, "comms.lost").side(fs).entity(m.from).target(m.to).text(text).data(serde_json::json!({ "kind": word, "reason": l.reason, "fallbacks": l.fallbacks.len() }));
                }
            }
        }
    }
    // Every transmission is an emission.
    let ems = c4.comms.net.as_mut().expect("active").drain_emissions();
    for e in ems {
        let id = EntityId(e.node.0);
        let Some(side) = side_of(s, id) else { continue };
        c4.add_on_air(id, e.duration_s);
        let gain = c4.roles.get(&id).and_then(|r| r.radio(e.bearer)).map(|r| r.gain_dbi).unwrap_or(0.0);
        c4.ew.tx.push(super::ew::TxEmission { entity: id, side, freq_hz: e.freq_hz, bw_hz: e.bandwidth_hz, erp_w: e.power_w * 10f64.powf(gain / 10.0), on_air_s: e.duration_s, duty: 1.0, what: "radio" });
    }
    super::put(s, c4);
}

/// 0.1 Hz: links and satcom.
pub fn slow(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    if c4.comms.active {
        let now = s.ctx.now;
        c4.comms.satcom_down.retain(|_, until| until.map(|u| u > now).unwrap_or(true));
        refresh(s, &mut c4);
    }
    super::put(s, c4);
}

/// Put a unit's radios out of action (damage, a test, a cyber effect): it
/// can neither send nor receive until restored.
pub fn cut_off(s: &mut Sim, unit: EntityId) {
    let Some(mut c4) = super::take(s) else { return };
    c4.comms.cut.insert(unit);
    if let Some(net) = c4.comms.net.as_mut() {
        let _ = net.destroy_node(node(unit));
    }
    c4.comms.dead.insert(unit);
    let text = format!("{}'s radios are out", crate::systems::name(s, unit));
    s.ctx.log.push(s.ctx.now, "comms.cut").entity(unit).text(text);
    super::put(s, c4);
}

/// The comms counters (AAR, tests).
pub fn stats(s: &Sim) -> Option<CommsStats> {
    s.world.get_resource::<C4>().map(|c| c.comms.stats.clone())
}
