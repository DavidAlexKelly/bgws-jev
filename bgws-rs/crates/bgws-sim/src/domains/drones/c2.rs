//! Command and control as processes with time (§8.12, §9.2, §10.3–10.4).
//!
//! - **HQs are entities:** each scenario HQ (or one per side, in the unit
//!   with a command rating) sits in a post that can be found, jammed and
//!   destroyed. Its staff cell (`bgws_ops::c2::StaffCell`) works the reports
//!   that reach it in priority order, slower when loaded, dropping what
//!   overflows or goes stale.
//! - **Who knows what:** a worked report goes into the side's track store
//!   (the picture other domains shoot from) and the HQ's COP
//!   (`bgws_know::Cop`), fused by covariance intersection; the HQ forwards
//!   its new tracks to its parent, whose COP fuses them again. The top HQ's
//!   COP is the picture its planner gets (`Sim::hq_situation`).
//! - **Planning takes time:** a CCIR (heavy losses, a major enemy force
//!   found) or the battle rhythm starts a plan; the planner's answer takes
//!   effect only when the planning time is up, by the 1/3–2/3 rule
//!   (`bgws_ops::c2`) against the time the operation has left. The plan's
//!   orders then travel the nets, one message per unit.
//! - **Out of comms:** units that hear nothing from their HQ keep their
//!   mission's phases (the intent) and are asked what to do.
//! - **Succession:** a lost post hands the seat to the alternate after a
//!   delay; the units re-address their reports to it.
//! - **Seats:** a rules seat answers its own HQ's questions at once; a
//!   planner or a person answers through the host.

use super::comms::{send, Arrived, Payload};
use super::{alive_id, ent, pos_of, push_trigger, side_of, QCtx, C4};
use crate::components::*;
use crate::hq::{CopTrack, OrderPoint, Phase, Plan, UnitOrder};
use crate::queue::{Priority, Scheduled};
use crate::sim::{SeatKind, Sim};
use bgws_catalogue::Domain;
use bgws_core::{Enu, EntityId, Geodetic, HqId, QuestionKind, Side, SimDuration, SimTime, TrackId};
use bgws_know::{Cop, CopOutcome, Detection, TrackView};
use bgws_ops::c2::{plan_quality, Echelon, StaffCell, StaffItem};
use bgws_ops::comms::MessageKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A unit as its HQ last heard of it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitReport {
    pub unit: EntityId,
    /// Where it thinks it is (its own position error included).
    pub at: Enu,
    pub sigma_m: f64,
    pub strength: f64,
    pub fit: u32,
    pub total: u32,
    pub alive: bool,
    pub activity: String,
    pub time: SimTime,
}

/// Staff work waiting in a cell.
#[derive(Clone, Debug)]
pub enum Work {
    Contact { observer: EntityId, dets: Vec<Detection>, new: bool },
    Tracks { from: EntityId, views: Vec<TrackView> },
    Sitrep(UnitReport),
}

#[derive(Clone, Debug)]
pub struct Planning {
    pub raised: SimTime,
    pub ready: SimTime,
    pub why: String,
    pub quality: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Life {
    Up,
    /// The post is lost; the seat moves to `to` at `until`.
    Down { lost_at: SimTime, until: SimTime, to: Option<EntityId> },
}

/// One HQ.
#[derive(Clone, Debug)]
pub struct HqRec {
    pub key: String,
    pub side: Side,
    pub echelon: Echelon,
    pub post: EntityId,
    pub parent: Option<usize>,
    /// The units and sub-HQ posts it commands.
    pub subs: Vec<EntityId>,
    pub alternate: Option<String>,
    pub planner: bool,
    pub cop: Cop,
    pub staff: StaffCell,
    pub work: BTreeMap<u64, Work>,
    pub next_item: u64,
    pub life: Life,
    pub last_plan: Option<SimTime>,
    pub planning: Option<Planning>,
    pub last_forward: SimTime,
    pub forward: BTreeMap<TrackId, TrackView>,
    pub reported: BTreeMap<EntityId, UnitReport>,
    pub new_contacts: u32,
    pub marks: BTreeSet<String>,
    pub handed: BTreeMap<(TrackId, EntityId), SimTime>,
    pub relocating_until: Option<SimTime>,
}

impl HqRec {
    pub fn up(&self) -> bool {
        self.life == Life::Up
    }
}

/// How long a report took from the sensor to an HQ's COP.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopLatency {
    pub at: SimTime,
    pub hq: usize,
    pub top: bool,
    pub observer: EntityId,
    pub latency_s: f64,
}

#[derive(Clone, Debug)]
pub struct PendingPlan {
    pub hq: usize,
    pub plan: Plan,
}

#[derive(Default)]
pub struct C2State {
    pub hqs: Vec<HqRec>,
    /// Each unit's HQ (a sub-HQ's post reports to the parent).
    pub unit_hq: BTreeMap<EntityId, usize>,
    pub last_contact: BTreeMap<EntityId, SimTime>,
    pub ooc_asked: BTreeMap<EntityId, SimTime>,
    pub pending_plans: BTreeMap<u64, PendingPlan>,
    pub next_product: u64,
    pub latency: Vec<CopLatency>,
    pub ineffective: BTreeSet<EntityId>,
    /// Where each unit last knew its HQ post to be.
    pub post_seen: BTreeMap<EntityId, Enu>,
}

impl C2State {
    /// The HQ whose post this entity is, if the HQ is up.
    pub fn post_of(&self, id: EntityId) -> Option<usize> {
        self.hqs.iter().position(|h| h.post == id && h.up())
    }
    /// Where a unit sends its reports: its own post if it is one, else its HQ's.
    pub fn report_dest(&self, id: EntityId) -> Option<EntityId> {
        if self.post_of(id).is_some() {
            return Some(id);
        }
        let h = &self.hqs[*self.unit_hq.get(&id)?];
        h.up().then_some(h.post)
    }
    /// The side's top HQ (no parent).
    pub fn top(&self, side: Side) -> Option<usize> {
        self.hqs.iter().position(|h| h.side == side && h.parent.is_none())
    }
}

fn echelon_of(e: bgws_catalogue::Echelon) -> Echelon {
    use bgws_catalogue::Echelon as E;
    match e {
        E::Team | E::Section | E::Platoon => Echelon::Platoon,
        E::Company => Echelon::Company,
        E::Battalion => Echelon::Battalion,
        E::Brigade => Echelon::Brigade,
        _ => Echelon::Division,
    }
}

fn new_hq(s: &Sim, key: String, side: Side, echelon: Echelon, post: EntityId, planner: bool, idx: usize) -> HqRec {
    HqRec {
        key,
        side,
        echelon,
        post,
        parent: None,
        subs: Vec::new(),
        alternate: None,
        planner,
        cop: Cop::new(HqId(idx as u32), side),
        staff: StaffCell::from_figures(&s.ctx.figures),
        work: BTreeMap::new(),
        next_item: 1,
        life: Life::Up,
        last_plan: None,
        planning: None,
        last_forward: SimTime::from_secs(-1e6),
        forward: BTreeMap::new(),
        reported: BTreeMap::new(),
        new_contacts: 0,
        marks: BTreeSet::new(),
        handed: BTreeMap::new(),
        relocating_until: None,
    }
}

/// The command tree: the scenario's HQs, or one per side.
pub fn setup(s: &mut Sim, c4: &mut C4) {
    let sc = &s.ctx.scenario;
    let planners = !sc.scope.planners.is_empty();
    let mut hqs: Vec<HqRec> = Vec::new();
    let mut parents: Vec<Option<String>> = Vec::new();
    let mut sub_names: Vec<Vec<String>> = Vec::new();
    for spec in &sc.command.hqs {
        let Some(&post) = s.ctx.by_name.get(&spec.post_id) else { continue };
        let idx = hqs.len();
        let mut h = new_hq(s, spec.id.clone(), spec.side, echelon_of(spec.echelon), post, planners && spec.planner.is_some(), idx);
        h.alternate = spec.alternate.clone();
        hqs.push(h);
        parents.push(spec.parent.clone());
        sub_names.push(spec.subordinates.clone());
    }
    // Parents and subordinates (a subordinate may name an HQ: its post).
    for i in 0..hqs.len() {
        hqs[i].parent = parents[i].as_ref().and_then(|p| hqs.iter().position(|h| &h.key == p));
        let mut subs = Vec::new();
        for n in &sub_names[i] {
            if let Some(h) = hqs.iter().find(|h| &h.key == n) {
                subs.push(h.post);
            } else if let Some(&id) = s.ctx.by_name.get(n) {
                subs.push(id);
            }
        }
        hqs[i].subs = subs;
    }
    // A side with no HQ gets one, in its command post or the unit with the best rating.
    for side in Side::PLAYERS {
        if hqs.iter().any(|h| h.side == side) {
            continue;
        }
        let own: Vec<(EntityId, f64, bool)> = s
            .ctx
            .scenario
            .forces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.side == side)
            .filter_map(|(i, f)| {
                let id = EntityId(i as u32);
                let e = ent(s, id)?;
                let ident = s.world.get::<Ident>(e)?;
                matches!(ident.domain, Domain::Land | Domain::Node).then_some((id, f.command_rating.unwrap_or(0.0), ident.hq))
            })
            .collect();
        if own.is_empty() {
            continue;
        }
        let post = own.iter().filter(|x| x.2).max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).or_else(|| own.iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))).map(|x| x.0).expect("non-empty");
        let idx = hqs.len();
        let echelon = echelon_of(sc.scope.echelons.highest);
        let mut h = new_hq(s, crate::hq::hq_agent(side), side, echelon, post, planners, idx);
        h.subs = own.iter().map(|x| x.0).filter(|x| *x != post).collect();
        hqs.push(h);
    }
    // Units nobody named go to their side's top HQ.
    let mut named: BTreeSet<EntityId> = hqs.iter().flat_map(|h| h.subs.iter().copied().chain([h.post])).collect();
    for (i, f) in s.ctx.scenario.forces.iter().enumerate() {
        let id = EntityId(i as u32);
        if named.contains(&id) {
            continue;
        }
        let Some(e) = ent(s, id) else { continue };
        if !s.world.get::<Ident>(e).map(|x| matches!(x.domain, Domain::Land | Domain::Node)).unwrap_or(false) {
            continue;
        }
        if let Some(top) = hqs.iter().position(|h| h.side == f.side && h.parent.is_none()) {
            hqs[top].subs.push(id);
            named.insert(id);
        }
    }
    let mut unit_hq = BTreeMap::new();
    for (i, h) in hqs.iter().enumerate() {
        for sub in &h.subs {
            unit_hq.entry(*sub).or_insert(i);
        }
    }
    for (i, h) in hqs.iter().enumerate() {
        if let Some(p) = h.parent {
            unit_hq.insert(h.post, p);
        } else {
            unit_hq.remove(&h.post);
        }
        let _ = i;
    }
    for (u, h) in &unit_hq {
        c4.c2.last_contact.insert(*u, SimTime::ZERO);
        if let Some(p) = pos_of(s, hqs[*h].post) {
            c4.c2.post_seen.insert(*u, p);
        }
    }
    c4.c2.unit_hq = unit_hq;
    c4.c2.hqs = hqs;
    for h in &c4.c2.hqs {
        let text = format!("{} HQ {} ({:?}) in {}, {} subordinates", h.side, h.key, h.echelon, crate::systems::name(s, h.post), h.subs.len());
        s.ctx.log.push(SimTime::ZERO, "c2.hq").side(h.side).entity(h.post).text(text);
    }
}

/// What a unit reports of itself: where it thinks it is, its strength.
pub fn own_report(s: &Sim, c4: &C4, id: EntityId) -> UnitReport {
    let e = ent(s, id).expect("entity");
    let pos = s.world.get::<Kin>(e).map(|k| k.pos).unwrap_or_default();
    let (bias, sigma) = c4.pnt.recs.get(&id).map(|r| {
        let sp = r.nav.self_position(s.ctx.now);
        (sp.bias, sp.sigma_m)
    }).unwrap_or((Enu::default(), c4.p.gnss_fix_sigma_m));
    let h = s.world.get::<Health>(e);
    UnitReport {
        unit: id,
        at: pos.add(bias),
        sigma_m: sigma,
        strength: h.map(|h| h.strength()).unwrap_or(0.0),
        fit: h.map(|h| h.fit).unwrap_or(0),
        total: h.map(|h| h.start).unwrap_or(0),
        alive: h.map(|h| !h.destroyed).unwrap_or(false),
        activity: s.world.get::<Act>(e).map(|a| a.activity.word().to_string()).unwrap_or_default(),
        time: s.ctx.now,
    }
}

fn hq_name(s: &Sim, c4: &C4, h: usize) -> String {
    let hq = &c4.c2.hqs[h];
    format!("{} ({})", hq.key, crate::systems::name(s, hq.post))
}

/// Deferred work that calls back into the sim.
enum After {
    Apply { side: Side, order: UnitOrder, refs: BTreeMap<String, Geodetic>, unit: EntityId, latency: f64, issued: SimTime },
    Ask { side: Side, why: String },
}

/// 1 Hz: arrivals, succession, the staffs, forwarding, heartbeats, CCIRs.
pub fn base(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    if c4.c2.hqs.is_empty() {
        super::put(s, c4);
        return;
    }
    let now = s.ctx.now;
    let mut after: Vec<After> = Vec::new();
    succession(s, &mut c4);
    // Without nets the HQs hear everything the side's picture holds.
    let arrived = std::mem::take(&mut c4.comms.arrived);
    for a in arrived {
        arrive(s, &mut c4, a, &mut after);
    }
    staff(s, &mut c4);
    forward(s, &mut c4);
    if c4.comms.active {
        heartbeats(s, &mut c4);
        if now.0 % 30_000_000 == 0 {
            out_of_comms(s, &mut c4);
        }
    }
    if now.0 % 10_000_000 == 0 {
        ineffective(s, &mut c4);
        ccir(s, &mut c4, &mut after);
    }
    super::put(s, c4);
    for a in after {
        match a {
            After::Apply { side, order, refs, unit, latency, issued } => {
                if !alive_id(s, unit) {
                    continue;
                }
                crate::decide::apply_unit_order(s, side, &order, &refs);
                let text = format!("{} receives its orders ({}) after {:.0} s", crate::systems::name(s, unit), order.task, latency);
                s.ctx.log.push(now, "c2.order_received").side(side).entity(unit).text(text).data(serde_json::json!({ "task": order.task, "latencyS": latency, "issuedS": issued.secs() }));
            }
            After::Ask { side, why } => ask_hq(s, side, &why),
        }
    }
}

/// Raise the side's HQ question; a rules seat answers it at once.
fn ask_hq(s: &mut Sim, side: Side, why: &str) {
    s.raise_hq_question(side, why);
    if s.ctx.seats.get(&side).copied().unwrap_or(SeatKind::Planner) == SeatKind::Rules {
        let q = s.ctx.pending.values().filter(|p| !p.answered && p.q.kind == QuestionKind::Hq && p.q.side == side).map(|p| p.q.clone()).next_back();
        if let Some(q) = q {
            let a = s.rules_answers(&[q]);
            let _ = s.answer(a);
        }
    }
}

fn arrive(s: &mut Sim, c4: &mut C4, a: Arrived, after: &mut Vec<After>) {
    let now = s.ctx.now;
    match a.payload {
        Payload::Contact { observer, dets, new } => {
            if let Some(h) = c4.c2.post_of(a.to) {
                let w = if new { c4.p.work_contact } else { c4.p.work_update };
                receive(c4, h, now, if new { 1 } else { 3 }, w, Work::Contact { observer, dets, new });
            }
        }
        Payload::Tracks { from, views } => {
            if let Some(h) = c4.c2.post_of(a.to) {
                let w = c4.p.work_tracks;
                receive(c4, h, now, 2, w, Work::Tracks { from, views });
            }
        }
        Payload::Sitrep(r) => {
            if let Some(h) = c4.c2.post_of(a.to) {
                let w = c4.p.work_sitrep;
                receive(c4, h, now, 4, w, Work::Sitrep(r));
            }
        }
        Payload::Intent { hq } => {
            c4.c2.last_contact.insert(a.to, now);
            if let Some(p) = c4.c2.hqs.get(hq).and_then(|h| pos_of(s, h.post)) {
                c4.c2.post_seen.insert(a.to, p);
            }
        }
        Payload::Order { hq, side, order, refs, plan: _, issued } => {
            c4.c2.last_contact.insert(a.to, now);
            if let Some(p) = c4.c2.hqs.get(hq).and_then(|h| pos_of(s, h.post)) {
                c4.c2.post_seen.insert(a.to, p);
            }
            after.push(After::Apply { side, order, refs, unit: a.to, latency: (now - issued).secs(), issued });
        }
        Payload::Target { track, at, value, seen } => {
            c4.c2.last_contact.insert(a.to, now);
            super::drones::hand_target(c4, a.to, track, at, value, seen, now);
        }
    }
}

fn receive(c4: &mut C4, h: usize, now: SimTime, priority: u8, work: f64, w: Work) {
    let hq = &mut c4.c2.hqs[h];
    let id = hq.next_item;
    hq.next_item += 1;
    hq.work.insert(id, w);
    hq.staff.receive(StaffItem { id, priority, arrived: now, work });
}

/// Posts lost: the seat moves to the alternate (or the senior surviving
/// subordinate post) after the handover delay.
fn succession(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    for h in 0..c4.c2.hqs.len() {
        if !c4.c2.hqs[h].up() || alive_id(s, c4.c2.hqs[h].post) {
            continue;
        }
        let hq = &c4.c2.hqs[h];
        let alt_post = hq.alternate.as_ref().and_then(|a| c4.c2.hqs.iter().find(|x| &x.key == a)).filter(|x| x.up() && alive_id(s, x.post)).map(|x| x.post);
        let to = alt_post.or_else(|| {
            let mut cands: Vec<(EntityId, f64, bool)> = hq
                .subs
                .iter()
                .filter(|x| alive_id(s, **x))
                .map(|x| (*x, s.ctx.scenario.forces.get(x.index()).and_then(|f| f.command_rating).unwrap_or(0.0), c4.c2.hqs.iter().any(|y| y.post == *x)))
                .collect();
            cands.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.total_cmp(&a.1)).then(a.0.cmp(&b.0)));
            cands.first().map(|x| x.0)
        });
        let until = now + SimDuration::from_secs(c4.p.handover_s);
        let lost = c4.c2.hqs[h].post;
        c4.c2.hqs[h].life = Life::Down { lost_at: now, until, to };
        s.ctx.queue.schedule(until, Priority::Process, Scheduled::Timer { system: "c4.handover".into(), key: h as u64 });
        let text = format!("{} lost with its post {}; {} takes the seat at {}", c4.c2.hqs[h].key, crate::systems::name(s, lost), to.map(|t| crate::systems::name(s, t)).unwrap_or_else(|| "nobody".into()), until);
        s.ctx.log.push(now, "c2.hq_lost").side(c4.c2.hqs[h].side).entity(lost).text(text).data(serde_json::json!({ "hq": c4.c2.hqs[h].key, "until": until.secs(), "to": to.map(|t| t.0) }));
    }
}

/// The handover is done: the new post takes the seat and joins the nets.
pub fn handover_due(s: &mut Sim, h: usize) {
    let Some(mut c4) = super::take(s) else { return };
    let now = s.ctx.now;
    if let Some(Life::Down { to, .. }) = c4.c2.hqs.get(h).map(|x| x.life.clone()) {
        match to.filter(|t| alive_id(s, *t)) {
            Some(to) => {
                let old = c4.c2.hqs[h].post;
                // The seat inherits the picture of the HQ that was in that post, if any.
                let inherited = c4.c2.hqs.iter().find(|x| x.post == to && x.up()).map(|x| x.cop.clone());
                let hq = &mut c4.c2.hqs[h];
                hq.post = to;
                hq.life = Life::Up;
                hq.cop = inherited.unwrap_or_else(|| Cop::new(HqId(h as u32), hq.side));
                hq.subs.retain(|x| *x != to && *x != old);
                hq.work.clear();
                hq.staff = StaffCell::from_figures(&s.ctx.figures);
                for u in hq.subs.clone() {
                    c4.c2.unit_hq.insert(u, h);
                }
                c4.c2.unit_hq.remove(&to);
                super::comms::join_nets(s, &mut c4, h);
                let text = format!("{} re-established in {}", c4.c2.hqs[h].key, crate::systems::name(s, to));
                s.ctx.log.push(now, "c2.handover").side(c4.c2.hqs[h].side).entity(to).text(text).data(serde_json::json!({ "hq": c4.c2.hqs[h].key }));
            }
            None => {
                let text = format!("{} has no one left to take the seat", c4.c2.hqs[h].key);
                s.ctx.log.push(now, "c2.hq_gone").side(c4.c2.hqs[h].side).text(text);
            }
        }
    }
    super::put(s, c4);
}

/// The staffs work their queues.
fn staff(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let know = s.ctx.p.know.clone();
    for h in 0..c4.c2.hqs.len() {
        if !c4.c2.hqs[h].up() || c4.c2.hqs[h].relocating_until.map(|u| u > now).unwrap_or(false) {
            continue;
        }
        let rep = c4.c2.hqs[h].staff.process(now);
        for d in &rep.dropped {
            c4.c2.hqs[h].work.remove(&d.id);
            let text = format!("{} staff drop a message ({:?})", c4.c2.hqs[h].key, d.reason);
            s.ctx.log.push(now, "c2.staff_drop").side(c4.c2.hqs[h].side).entity(c4.c2.hqs[h].post).text(text).data(serde_json::json!({ "reason": d.reason }));
        }
        let is_top = c4.c2.hqs[h].parent.is_none();
        let side = c4.c2.hqs[h].side;
        for p in rep.processed {
            let Some(w) = c4.c2.hqs[h].work.remove(&p.id) else { continue };
            match w {
                Work::Contact { observer, dets, new: _ } => {
                    let oldest = dets.iter().map(|d| d.time).min().unwrap_or(now);
                    let store = s.ctx.tracks.get_mut(&side).expect("side");
                    let scans = store.ingest(dets, &know);
                    let mut ids: BTreeSet<TrackId> = BTreeSet::new();
                    for sc in &scans {
                        ids.extend(sc.tracks_seen_by(observer));
                    }
                    let views: Vec<TrackView> = ids.iter().filter_map(|t| store.view(*t, &know)).collect();
                    fuse(s, c4, h, observer, views);
                    c4.c2.latency.push(CopLatency { at: now, hq: h, top: is_top, observer, latency_s: (now - oldest).secs() });
                }
                Work::Tracks { from, views } => {
                    let oldest = views.iter().map(|v| v.last_update).min().unwrap_or(now);
                    fuse(s, c4, h, from, views);
                    c4.c2.latency.push(CopLatency { at: now, hq: h, top: is_top, observer: from, latency_s: (now - oldest).secs() });
                }
                Work::Sitrep(r) => {
                    c4.c2.hqs[h].reported.insert(r.unit, r);
                }
            }
        }
    }
}

/// Fuse track reports into an HQ's COP; hand new land targets to its strike teams.
fn fuse(s: &mut Sim, c4: &mut C4, h: usize, from: EntityId, views: Vec<TrackView>) {
    let now = s.ctx.now;
    let know = &s.ctx.p.know;
    let has_parent = c4.c2.hqs[h].parent.is_some();
    let mut fresh: Vec<(TrackId, TrackView)> = Vec::new();
    for v in views {
        let up = c4.c2.hqs[h].cop.receive(&v, from, now, know);
        if v.domain == bgws_know::Domain::Land && up.outcome == CopOutcome::New {
            c4.c2.hqs[h].new_contacts += 1;
        }
        if v.domain == bgws_know::Domain::Land && up.outcome != CopOutcome::Ignored {
            if let Some(e) = c4.c2.hqs[h].cop.entries.get(&up.entry) {
                fresh.push((up.entry, e.fused.clone()));
            }
        }
        if has_parent {
            c4.c2.hqs[h].forward.insert(v.id, v);
        }
    }
    if c4.comms.active {
        for (entry, v) in fresh {
            super::drones::offer_target(s, c4, h, entry, &v);
        }
    }
}

/// HQs pass their new tracks up.
fn forward(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    for h in 0..c4.c2.hqs.len() {
        let hq = &c4.c2.hqs[h];
        let Some(p) = hq.parent else { continue };
        if !hq.up() || hq.forward.is_empty() || (now - hq.last_forward).secs() < c4.p.forward_gap_s {
            continue;
        }
        let parent = &c4.c2.hqs[p];
        if !parent.up() {
            continue;
        }
        let (from, to) = (hq.post, parent.post);
        let views: Vec<TrackView> = std::mem::take(&mut c4.c2.hqs[h].forward).into_values().collect();
        c4.c2.hqs[h].last_forward = now;
        if c4.comms.active {
            let size = Some(s.ctx.figures.get("comms.msg.track_bits") * views.len() as f64);
            send(now, c4, from, to, MessageKind::DataLinkTrack, 2, size, Payload::Tracks { from, views });
        } else {
            let w = c4.p.work_tracks;
            receive(c4, p, now, 2, w, Work::Tracks { from, views });
        }
    }
}

/// Intent repeated down the nets; units in comms hear it.
fn heartbeats(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let every = c4.p.heartbeat_s.max(1.0) as i64;
    for h in 0..c4.c2.hqs.len() {
        let hq = &c4.c2.hqs[h];
        if !hq.up() || !c4.may_transmit(hq.post) {
            continue;
        }
        let phase = (hq.post.0 as i64 * 37) % every;
        if (now.0 / 1_000_000) % every != phase {
            continue;
        }
        let (post, subs) = (hq.post, hq.subs.clone());
        for u in subs {
            if u != post && alive_id(s, u) {
                send(now, c4, post, u, MessageKind::PlanOrder, 3, Some(2_000.0), Payload::Intent { hq: h });
            }
        }
    }
}

fn out_of_comms(s: &mut Sim, c4: &mut C4) {
    let now = s.ctx.now;
    let units: Vec<(EntityId, usize)> = c4.c2.unit_hq.iter().map(|(u, h)| (*u, *h)).collect();
    for (u, h) in units {
        if c4.c2.post_of(u).is_some() || !alive_id(s, u) {
            continue;
        }
        let Some(e) = ent(s, u) else { continue };
        let steady = s.world.get::<Crew>(e).map(|c| c.morale.state == bgws_models::people::MoraleState::Steady).unwrap_or(false);
        let has_mission = s.world.get::<Act>(e).map(|a| !a.mission.phases.is_empty()).unwrap_or(false);
        if !steady || !has_mission {
            continue;
        }
        let last = c4.c2.last_contact.get(&u).copied().unwrap_or(SimTime::ZERO);
        let silent = (now - last).secs();
        if silent < c4.p.out_of_comms_s {
            continue;
        }
        if c4.c2.ooc_asked.get(&u).map(|t| (now - *t).secs() < c4.p.out_of_comms_s).unwrap_or(false) {
            continue;
        }
        c4.c2.ooc_asked.insert(u, now);
        let text = format!("{} has heard nothing from {} for {:.0} min", crate::systems::name(s, u), hq_name(s, c4, h), silent / 60.0);
        s.ctx.log.push(now, "c2.out_of_comms").side(c4.c2.hqs[h].side).entity(u).text(text).data(serde_json::json!({ "silentS": silent }));
        push_trigger(s, c4, u, "outOfComms", None, QCtx::default());
    }
}

fn strength_of(s: &Sim, c4: &C4, h: usize, u: EntityId) -> (f64, bool) {
    if c4.comms.active {
        if let Some(r) = c4.c2.hqs[h].reported.get(&u) {
            return (r.strength, r.alive);
        }
        return (1.0, true);
    }
    let e = ent(s, u);
    let st = e.and_then(|e| s.world.get::<Health>(e)).map(|h| h.strength()).unwrap_or(0.0);
    (st, st > 0.0)
}

fn ineffective(s: &mut Sim, c4: &mut C4) {
    for h in 0..c4.c2.hqs.len() {
        let hq = &c4.c2.hqs[h];
        if !hq.up() || !alive_id(s, hq.post) {
            continue;
        }
        let post = hq.post;
        for u in hq.subs.clone() {
            if c4.c2.ineffective.contains(&u) || !alive_id(s, u) {
                continue;
            }
            let (st, _) = strength_of(s, c4, h, u);
            let broken = ent(s, u).and_then(|e| s.world.get::<Crew>(e)).map(|c| matches!(c.morale.state, bgws_models::people::MoraleState::Broken | bgws_models::people::MoraleState::Routed)).unwrap_or(false);
            if st >= c4.p.ineffective_strength && !broken {
                continue;
            }
            c4.c2.ineffective.insert(u);
            let at = c4.c2.hqs[h].reported.get(&u).map(|r| r.at).or_else(|| pos_of(s, u));
            push_trigger(s, c4, post, "subordinateIneffective", Some(u), QCtx { sub: Some(u), at, ..Default::default() });
            break;
        }
    }
}

/// CCIRs and the battle rhythm start plans at the side's top HQ.
fn ccir(s: &mut Sim, c4: &mut C4, after: &mut Vec<After>) {
    let now = s.ctx.now;
    if s.ctx.scenario.scope.planners.is_empty() {
        return;
    }
    for side in Side::PLAYERS {
        let Some(h) = c4.c2.top(side) else { continue };
        if !c4.c2.hqs[h].up() || !c4.c2.hqs[h].planner {
            continue;
        }
        if c4.c2.hqs[h].last_plan.is_none() {
            // The opening plan was made before H-hour (`Sim::new`).
            c4.c2.hqs[h].last_plan = Some(SimTime::ZERO);
        }
        if c4.c2.hqs[h].planning.as_ref().map(|p| p.ready > now).unwrap_or(false) {
            continue;
        }
        if s.ctx.pending.values().any(|p| !p.answered && p.q.kind == QuestionKind::Hq && p.q.side == side) {
            continue;
        }
        let last = c4.c2.hqs[h].last_plan.unwrap_or(SimTime::ZERO);
        let since = (now - last).secs();
        // Heavy losses among its units: a hasty plan at once, once.
        let subs = all_units_under(c4, h);
        let (mut fit, mut total) = (0.0, 0.0);
        for u in &subs {
            let start = ent(s, *u).and_then(|e| s.world.get::<Health>(e)).map(|x| x.start as f64).unwrap_or(0.0);
            let (st, _) = strength_of(s, c4, c4.c2.unit_hq.get(u).copied().unwrap_or(h), *u);
            fit += st * start;
            total += start;
        }
        let lost = if total > 0.0 { 1.0 - fit / total } else { 0.0 };
        let mut why: Option<(String, bool)> = None;
        if lost >= c4.p.culmination_loss && !c4.c2.hqs[h].marks.contains("culmination") {
            c4.c2.hqs[h].marks.insert("culmination".into());
            why = Some((format!("CCIR: {:.0}% losses", lost * 100.0), true));
        } else if c4.c2.hqs[h].new_contacts >= c4.p.ccir_new_contacts && since >= c4.p.ccir_min_gap_s {
            why = Some((format!("CCIR: {} new enemy contacts", c4.c2.hqs[h].new_contacts), true));
        } else if since >= c4.p.rhythm_s {
            why = Some(("battle rhythm".into(), false));
        }
        let Some((why, hasty)) = why else { continue };
        let left = (s.ctx.duration_limit() - now).secs().max(0.0);
        let e = c4.c2.hqs[h].echelon;
        let cap = if hasty { e.hasty(&s.ctx.figures) } else { e.deliberate(&s.ctx.figures) };
        let dur = SimDuration::from_secs((left * c4.p.own_fraction).min(cap.secs()));
        let quality = plan_quality(dur, e, &s.ctx.figures);
        let hq = &mut c4.c2.hqs[h];
        hq.planning = Some(Planning { raised: now, ready: now + dur, why: why.clone(), quality });
        hq.last_plan = Some(now);
        hq.new_contacts = 0;
        let text = format!("{} starts planning ({}); ready in {:.0} min, quality {:.2}", hq.key, why, dur.secs() / 60.0, quality);
        s.ctx.log.push(now, "c2.planning").side(side).entity(hq.post).text(text).data(serde_json::json!({ "why": why, "readyS": (now + dur).secs(), "quality": quality, "hasty": hasty }));
        after.push(After::Ask { side, why });
    }
}

fn all_units_under(c4: &C4, h: usize) -> Vec<EntityId> {
    let mut out = Vec::new();
    let mut stack = vec![h];
    while let Some(x) = stack.pop() {
        for u in &c4.c2.hqs[x].subs {
            if let Some(sub) = c4.c2.hqs.iter().position(|y| y.post == *u && y.parent == Some(x)) {
                stack.push(sub);
            } else {
                out.push(*u);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 0.1 Hz: COPs age; without nets the HQs are fed from the side picture.
pub fn slow(s: &mut Sim) {
    let Some(mut c4) = super::take(s) else { return };
    let now = s.ctx.now;
    let know = s.ctx.p.know.clone();
    if !c4.comms.active {
        for h in 0..c4.c2.hqs.len() {
            if !c4.c2.hqs[h].up() {
                continue;
            }
            let side = c4.c2.hqs[h].side;
            let views = s.ctx.tracks[&side].confirmed(&know);
            for v in views {
                let from = v.sources.first().copied().unwrap_or(c4.c2.hqs[h].post);
                c4.c2.hqs[h].cop.receive(&v, from, now, &know);
            }
        }
    }
    for h in &mut c4.c2.hqs {
        h.cop.prune(now, &know);
    }
    super::put(s, c4);
}

// ── Plans and orders ────────────────────────────────────────────────────

/// An HQ's plan, answered by its planner. With nets, it takes effect when
/// its planning time is up and then travels as one order per unit; returns
/// false when there are no nets (the caller applies it at once).
pub fn route_plan(s: &mut Sim, side: Side, plan: &Plan, raised_at: SimTime) -> bool {
    let Some(mut c4) = super::take(s) else { return false };
    if !c4.comms.active {
        super::put(s, c4);
        return false;
    }
    let now = s.ctx.now;
    let Some(h) = c4.c2.top(side) else {
        super::put(s, c4);
        return false;
    };
    if !c4.c2.hqs[h].up() {
        let text = format!("{} cannot issue its plan: its post is lost", c4.c2.hqs[h].key);
        s.ctx.log.push(now, "c2.plan_lost").side(side).text(text);
        super::put(s, c4);
        return true;
    }
    let ready = c4.c2.hqs[h].planning.as_ref().filter(|p| p.raised == raised_at).map(|p| p.ready).unwrap_or(now);
    if ready > now {
        let id = c4.c2.next_product;
        c4.c2.next_product += 1;
        c4.c2.pending_plans.insert(id, PendingPlan { hq: h, plan: plan.clone() });
        let post = c4.c2.hqs[h].post;
        s.ctx.queue.schedule(ready, Priority::Process, Scheduled::HqProduct { hq: post, product: id });
        super::put(s, c4);
        return true;
    }
    super::put(s, c4);
    issue(s, h, plan);
    true
}

/// A plan's planning time is up.
pub fn product_due(s: &mut Sim, _post: EntityId, product: u64) {
    let Some(mut c4) = super::take(s) else { return };
    let p = c4.c2.pending_plans.remove(&product);
    super::put(s, c4);
    if let Some(p) = p {
        issue(s, p.hq, &p.plan);
    }
}

/// Send a plan's orders over the nets.
fn issue(s: &mut Sim, h: usize, plan: &Plan) {
    let now = s.ctx.now;
    let side = match s.world.get_resource::<C4>() {
        Some(c4) => c4.c2.hqs[h].side,
        None => return,
    };
    let refs = s.hq_situation(side, "orders").reference_points;
    let Some(mut c4) = super::take(s) else { return };
    if !c4.c2.hqs[h].up() {
        let text = format!("{}'s plan dies with its post", c4.c2.hqs[h].key);
        s.ctx.log.push(now, "c2.plan_lost").side(side).text(text);
        super::put(s, c4);
        return;
    }
    let post = c4.c2.hqs[h].post;
    let mut local = Vec::new();
    let (mut sent, mut failed) = (0, 0);
    for o in &plan.orders {
        let Some(&u) = s.ctx.by_name.get(&o.unit) else { continue };
        if side_of(s, u) != Some(side) || !alive_id(s, u) {
            continue;
        }
        if u == post || !c4.comms.nodes.contains(&u) {
            local.push(o.clone());
            continue;
        }
        let payload = Payload::Order { hq: h, side, order: o.clone(), refs: refs.clone(), plan: plan.plan.clone(), issued: now };
        if send(now, &mut c4, post, u, MessageKind::PlanOrder, 1, None, payload) {
            sent += 1;
        } else {
            failed += 1;
        }
    }
    let quality = c4.c2.hqs[h].planning.as_ref().map(|p| p.quality);
    let key = c4.c2.hqs[h].key.clone();
    super::put(s, c4);
    for o in &local {
        crate::decide::apply_unit_order(s, side, o, &refs);
    }
    s.ctx.last_plan.insert(side, plan.clone());
    let text = format!("{} issues its plan: {} ({} orders on the nets, {} by hand)", key, plan.plan, sent, local.len() + failed);
    s.ctx.log.push(now, "c2.plan_issued").side(side).entity(post).text(text).data(serde_json::json!({ "hq": key, "orders": sent, "local": local.len(), "quality": quality, "plan": plan }));
}

/// One order from an HQ to one unit, over the nets.
pub(crate) fn send_order(s: &mut Sim, c4: &mut C4, h: usize, unit: EntityId, order: UnitOrder) -> bool {
    let now = s.ctx.now;
    let side = c4.c2.hqs[h].side;
    if !c4.c2.hqs[h].up() {
        return false;
    }
    let refs = BTreeMap::new();
    let post = c4.c2.hqs[h].post;
    let payload = Payload::Order { hq: h, side, order, refs, plan: "FRAGO".into(), issued: now };
    send(now, c4, post, unit, MessageKind::PlanOrder, 1, None, payload)
}

fn point(s: &Sim, p: Enu) -> OrderPoint {
    let g = s.ctx.frame.enu_to_geodetic(p);
    OrderPoint { lat: Some(g.lat), lon: Some(g.lon), ..Default::default() }
}

// ── What the planner sees (Sim::hq_situation) ───────────────────────────

/// The side's top HQ's COP, as the planner's tracks; `None` with no HQs.
pub fn hq_cop_tracks(s: &Sim, side: Side) -> Option<Vec<CopTrack>> {
    let c4 = s.world.get_resource::<C4>()?;
    let h = c4.c2.top(side)?;
    let hq = &c4.c2.hqs[h];
    if !hq.up() {
        return Some(Vec::new());
    }
    let now = s.ctx.now;
    let know = &s.ctx.p.know;
    Some(
        hq.cop
            .picture(now, know)
            .into_iter()
            .map(|t| CopTrack {
                id: format!("{}", t.track),
                thought: t.class_top.first().map(|(h, p)| format!("{h:?} ({:.0}%)", p * 100.0)).unwrap_or_else(|| "unknown".into()),
                domain: format!("{:?}", t.domain).to_lowercase(),
                seen_ago_s: (now - t.seen_at).secs(),
                by: t.sources.first().and_then(|x| s.ctx.scenario.forces.get(x.index())).map(|f| f.id.clone()).unwrap_or_default(),
                quality: t.quality,
                error_m: t.ellipse.semi_major,
                at: Some(s.ctx.frame.enu_to_geodetic(t.estimate.position())),
            })
            .collect(),
    )
}

/// A unit as its side's HQs last heard of it (the newest report), with nets.
pub fn hq_unit_report(s: &Sim, side: Side, unit: EntityId) -> Option<UnitReport> {
    let c4 = s.world.get_resource::<C4>()?;
    if !c4.comms.active {
        return None;
    }
    c4.c2.hqs.iter().filter(|h| h.side == side).filter_map(|h| h.reported.get(&unit)).max_by(|a, b| a.time.cmp(&b.time)).cloned()
}

/// The side's top HQ post.
pub fn hq_post(s: &Sim, side: Side) -> Option<EntityId> {
    let c4 = s.world.get_resource::<C4>()?;
    let h = c4.c2.top(side)?;
    c4.c2.hqs[h].up().then_some(c4.c2.hqs[h].post)
}

/// Report latencies into the HQs' COPs (AAR, tests).
pub fn cop_latencies(s: &Sim) -> Vec<CopLatency> {
    s.world.get_resource::<C4>().map(|c| c.c2.latency.clone()).unwrap_or_default()
}

/// The HQ commanding a unit, by key.
pub fn hq_of(s: &Sim, unit: EntityId) -> Option<String> {
    let c4 = s.world.get_resource::<C4>()?;
    c4.c2.unit_hq.get(&unit).map(|h| c4.c2.hqs[*h].key.clone())
}

// ── Actions (c2.*) ──────────────────────────────────────────────────────

pub fn apply_action(s: &mut Sim, e: bevy_ecs::prelude::Entity, id: EntityId, action: &str, _about: Option<EntityId>) -> bool {
    let Some(mut c4) = super::take(s) else { return false };
    let now = s.ctx.now;
    let q = c4.qctx.get(&id).cloned().unwrap_or_default();
    let handled = match action {
        "carryOnPhase" | "carryOn" | "acceptRisk" | "holdReserve" | "commitReserve" | "fireFirstRequest" => true,
        "actOnIntent" => {
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                if a.mission.phase + 1 < a.mission.phases.len() {
                    a.mission.phase += 1;
                    a.activity = a.mission.phases[a.mission.phase].clone();
                } else if let Some(p) = a.mission.phases.get(a.mission.phase).cloned() {
                    a.activity = p;
                }
            }
            true
        }
        "holdAndReestablish" => {
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Hold;
            }
            c4.comms.last_sitrep.remove(&id);
            true
        }
        "moveToRegainComms" => {
            let post = c4.c2.post_seen.get(&id).copied();
            let me = pos_of(s, id);
            if let (Some(p), Some(me)) = (post, me) {
                let d = p.sub(me);
                let dist = d.horizontal();
                if dist > c4.p.regain_m {
                    let to = me.add(d.scale((dist - c4.p.regain_m * 0.8) / dist));
                    super::put(s, c4);
                    let route = crate::decide::route_for(s, e, to);
                    if let Some(mut a) = s.world.get_mut::<Act>(e) {
                        a.activity = Activity::Move { route, mode: MoveModeS::Tactical, then: Some(Box::new(Activity::Overwatch)) };
                    }
                    return true;
                }
            }
            true
        }
        "displacePost" => {
            let away = super::ew::point_away(s, id, c4.p.relocate_m);
            if let Some(h) = c4.c2.post_of(id) {
                c4.c2.hqs[h].relocating_until = Some(now + SimDuration::from_secs(c4.p.relocation_s));
            }
            c4.on_air.insert(id, 0.0);
            super::put(s, c4);
            let route = crate::decide::route_for(s, e, away);
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Move { route, mode: MoveModeS::March, then: Some(Box::new(Activity::Hold)) };
            }
            let text = format!("{} displaces its command post", crate::systems::name(s, id));
            s.ctx.log.push(now, "c2.displace").entity(id).text(text);
            return true;
        }
        "alternateBearer" => {
            c4.emcon.insert(id, bgws_scenario::v4::Emcon::Limited);
            c4.on_air.insert(id, 0.0);
            true
        }
        "reinforce" | "swapWithReserve" | "withdrawIt" => {
            let (Some(h), Some(sub)) = (c4.c2.post_of(id), q.sub) else {
                super::put(s, c4);
                return true;
            };
            let at = q.at.or_else(|| pos_of(s, sub)).unwrap_or_default();
            let sub_name = s.ctx.scenario.forces.get(sub.index()).map(|f| f.id.clone()).unwrap_or_default();
            if action == "withdrawIt" {
                let post_at = pos_of(s, id).unwrap_or(at);
                let back = at.add(post_at.sub(at).scale(0.5));
                let order = UnitOrder {
                    unit: sub_name,
                    task: "withdraw".into(),
                    intent: "pull back, reconstitute and hold".into(),
                    urgency: Some("now".into()),
                    roe: Some("ifFiredUpon".into()),
                    on_contact: None,
                    phases: vec![Phase { label: "withdraw".into(), action: "withdraw".into(), to: Some(point(s, back)), mode: None }, Phase { label: "hold".into(), action: "hold".into(), to: None, mode: None }],
                };
                send_order(s, &mut c4, h, sub, order);
            } else {
                // The healthiest other unit goes to its aid.
                let helper = c4.c2.hqs[h].subs.iter().copied().filter(|u| *u != sub && alive_id(s, *u) && c4.c2.post_of(*u).is_none()).max_by(|a, b| strength_of(s, &c4, h, *a).0.total_cmp(&strength_of(s, &c4, h, *b).0).then(b.cmp(a)));
                if let Some(hp) = helper {
                    let name = s.ctx.scenario.forces.get(hp.index()).map(|f| f.id.clone()).unwrap_or_default();
                    let order = UnitOrder {
                        unit: name,
                        task: "reinforce".into(),
                        intent: format!("go to the aid of {sub_name}"),
                        urgency: Some("now".into()),
                        roe: None,
                        on_contact: None,
                        phases: vec![Phase { label: "reinforce".into(), action: "move".into(), to: Some(point(s, at)), mode: Some("tactical".into()) }, Phase { label: "hold".into(), action: "overwatch".into(), to: None, mode: None }],
                    };
                    send_order(s, &mut c4, h, hp, order);
                }
            }
            true
        }
        _ => false,
    };
    super::put(s, c4);
    handled
}
