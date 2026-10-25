//! Command and control as processes with time (RUST_ENGINE.md §8.12; research R1, P1, P25, R8).
//!
//! - **Planning time** per echelon with the 1/3–2/3 rule **[P1]**: an HQ
//!   spends at most a third of the time it has, and at most its deliberate
//!   planning time **[P25]**, leaving the rest to subordinates; a brigade
//!   given 24 h leaves about 16 for battalion, 11 for company and 7 for
//!   platoon (R1). Plan quality grows with time spent.
//! - **Hasty plans** when a CCIR fires (a FRAGO in minutes to hours by echelon).
//! - **Battle rhythm** events on a schedule.
//! - **Staff capacity** as a queue: an overloaded staff works slower and
//!   drops information that overflows or goes stale.
//! - **HQ signatures** grow with emitters, vehicles, generators and time
//!   static; dispersal cuts them (R1: CPs targeted by drones and fires).
//! - **Succession:** losing an HQ hands the seat to its alternate after a delay.
//! - **Structured orders** (task, purpose, phases, control measures, timings,
//!   ROE, actions on contact), C2SIM-shaped **[R8]**, with serde, validation,
//!   and a hand-written JSON Schema ([`orders_json_schema`]) that Claude
//!   planners must answer in (§10.3).

use bgws_core::{figure, Figure, Figures, HqId, SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Figures for C2 (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("c2.own_fraction", 1.0 / 3.0, "fraction", 0.2, 0.6, "P1", Medium, "share of available time an HQ keeps (≤1/3); real units often overrun"),
        figure!("c2.deliberate.division_h", 48.0, "h", 24.0, 96.0, "P25", Default, "deliberate MDMP at division 24–96 h"),
        figure!("c2.deliberate.brigade_h", 18.0, "h", 12.0, 24.0, "P25", Default, "brigade 12–24 h"),
        figure!("c2.deliberate.battalion_h", 9.0, "h", 6.0, 12.0, "P25", Default, "battalion 6–12 h"),
        figure!("c2.deliberate.company_h", 4.0, "h", 2.0, 6.0, "P25", Default, "company TLP 2–6 h"),
        figure!("c2.deliberate.platoon_h", 2.0, "h", 1.0, 3.0, "P25", Default, "platoon 1–3 h"),
        figure!("c2.hasty.division_h", 18.0, "h", 12.0, 24.0, "R1", Default, "division change of scheme 12–24 h"),
        figure!("c2.hasty.brigade_h", 4.0, "h", 2.0, 6.0, "R1", Default, "brigade re-tasking 2–6 h"),
        figure!("c2.hasty.battalion_h", 1.5, "h", 1.0, 2.0, "R1", Default, "battalion FRAGO 1–2 h"),
        figure!("c2.hasty.company_h", 0.33, "h", 0.17, 0.5, "R1", Default, "company adjustment 10–30 min"),
        figure!("c2.hasty.platoon_h", 0.05, "h", 0.0, 0.17, "R1", Default, "platoon drill: seconds to minutes"),
        figure!("c2.quality_exponent", 0.5, "1", 0.2, 2.0, "default", Default, "plan quality = (time spent / deliberate time)^α"),
        figure!("c2.staff.items_per_h", 30.0, "1/h", 5.0, 200.0, "default", Default, "messages a staff cell processes per hour, unloaded"),
        figure!("c2.staff.overload_slowdown", 1.0, "1", 0.0, 5.0, "default", Default, "service time × (1 + k·queue/max_queue)"),
        figure!("c2.staff.max_queue", 40.0, "1", 5.0, 500.0, "default", Default, "items a staff cell holds before dropping"),
        figure!("c2.staff.max_age_h", 2.0, "h", 0.25, 24.0, "default", Default, "items older than this are dropped as stale"),
        figure!("c2.sig.per_emitter", 0.5, "1", 0.0, 3.0, "R1", Low, "detection multiplier per radiating emitter"),
        figure!("c2.sig.per_vehicle", 0.05, "1", 0.0, 0.5, "default", Default, "detection multiplier per vehicle"),
        figure!("c2.sig.per_generator", 0.3, "1", 0.0, 2.0, "default", Default, "thermal/acoustic multiplier per generator"),
        figure!("c2.sig.per_static_h", 0.1, "1/h", 0.0, 1.0, "R1", Low, "detection grows with time static"),
        figure!("c2.sig.dispersed_factor", 0.4, "fraction", 0.1, 1.0, "default", Default, "dispersal factor on the whole signature"),
        figure!("c2.handover_h", 0.5, "h", 0.05, 6.0, "default", Default, "delay before an alternate HQ takes the seat"),
        figure!("c2.relocation_h", 1.0, "h", 0.25, 6.0, "R1", Low, "C2 downtime when an HQ jumps"),
    ]
}

/// Echelons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Echelon {
    Division,
    Brigade,
    Battalion,
    Company,
    Platoon,
}

impl Echelon {
    fn key(self) -> &'static str {
        match self {
            Echelon::Division => "division",
            Echelon::Brigade => "brigade",
            Echelon::Battalion => "battalion",
            Echelon::Company => "company",
            Echelon::Platoon => "platoon",
        }
    }
    /// The next echelon down.
    pub fn below(self) -> Option<Echelon> {
        match self {
            Echelon::Division => Some(Echelon::Brigade),
            Echelon::Brigade => Some(Echelon::Battalion),
            Echelon::Battalion => Some(Echelon::Company),
            Echelon::Company => Some(Echelon::Platoon),
            Echelon::Platoon => None,
        }
    }
    pub fn deliberate(self, f: &Figures) -> SimDuration {
        SimDuration::from_hours(f.get(&format!("c2.deliberate.{}_h", self.key())))
    }
    pub fn hasty(self, f: &Figures) -> SimDuration {
        SimDuration::from_hours(f.get(&format!("c2.hasty.{}_h", self.key())))
    }
}

/// One echelon's slice of the planning time (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanningSlot {
    pub echelon: Echelon,
    /// Time this echelon had when it received the order.
    pub available: SimDuration,
    /// Time it spends planning.
    pub own: SimDuration,
    /// Time left for its subordinates.
    pub left: SimDuration,
    /// Plan quality, 0–1.
    pub quality: f64,
}

/// Plan quality from time spent: (spent / deliberate)^α, capped at 1.
pub fn plan_quality(spent: SimDuration, e: Echelon, f: &Figures) -> f64 {
    (spent.secs() / e.deliberate(f).secs()).clamp(0.0, 1.0).powf(f.get("c2.quality_exponent"))
}

/// The 1/3–2/3 cascade **[P1]** from `top` down, given `available` time.
pub fn planning_cascade(available: SimDuration, top: Echelon, f: &Figures) -> Vec<PlanningSlot> {
    let frac = f.get("c2.own_fraction");
    let mut out = Vec::new();
    let mut avail = available;
    let mut e = Some(top);
    while let Some(ech) = e {
        let own = SimDuration::from_secs((avail.secs() * frac).min(ech.deliberate(f).secs()));
        let left = SimDuration(avail.0 - own.0);
        out.push(PlanningSlot { echelon: ech, available: avail, own, left, quality: plan_quality(own, ech, f) });
        avail = left;
        e = ech.below();
    }
    out
}

/// Commander's critical information requirements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CcirKind {
    /// Priority intelligence requirement.
    Pir,
    /// Friendly force information requirement.
    Ffir,
    /// Essential element of friendly information.
    Eefi,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ccir {
    pub id: String,
    pub kind: CcirKind,
    pub question: String,
    /// Answering it triggers a hasty plan.
    #[serde(default)]
    pub triggers_replan: bool,
}

/// A hasty plan triggered by a CCIR (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HastyPlan {
    pub ccir: String,
    pub echelon: Echelon,
    pub triggered_at: SimTime,
    pub ready_at: SimTime,
    pub quality: f64,
}

/// A CCIR fires at `t`: the HQ produces a FRAGO after its hasty planning time.
pub fn on_ccir(ccir: &Ccir, e: Echelon, t: SimTime, f: &Figures) -> Option<HastyPlan> {
    ccir.triggers_replan.then(|| {
        let d = e.hasty(f);
        HastyPlan { ccir: ccir.id.clone(), echelon: e, triggered_at: t, ready_at: t + d, quality: plan_quality(d, e, f) }
    })
}

/// A recurring battle-rhythm event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RhythmEvent {
    pub name: String,
    pub period: SimDuration,
    pub offset: SimDuration,
    pub duration: SimDuration,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RhythmOccurrence {
    pub name: String,
    pub start: SimTime,
    pub end: SimTime,
}

/// Every occurrence starting in [from, to), ordered by start then name.
pub fn battle_rhythm(events: &[RhythmEvent], from: SimTime, to: SimTime) -> Vec<RhythmOccurrence> {
    let mut out = Vec::new();
    for e in events {
        if e.period.0 <= 0 {
            continue;
        }
        let k0 = ((from.0 - e.offset.0) as f64 / e.period.0 as f64).ceil().max(0.0) as i64;
        let mut k = k0;
        loop {
            let start = SimTime(e.offset.0 + k * e.period.0);
            if start >= to {
                break;
            }
            if start >= from {
                out.push(RhythmOccurrence { name: e.name.clone(), start, end: start + e.duration });
            }
            k += 1;
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.name.cmp(&b.name)));
    out
}

/// An item of staff work (a report, a request).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaffItem {
    pub id: u64,
    /// Lower is more urgent.
    pub priority: u8,
    pub arrived: SimTime,
    /// Work units (1 = an ordinary report).
    pub work: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Processed {
    pub id: u64,
    pub start: SimTime,
    pub done: SimTime,
    pub wait_min: f64,
    /// Service-time multiplier from overload.
    pub slowdown: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    Overflow,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dropped {
    pub id: u64,
    pub at: SimTime,
    pub reason: DropReason,
}

/// An HQ staff cell as a single-server priority queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaffCell {
    pub items_per_h: f64,
    pub slowdown: f64,
    pub max_queue: usize,
    pub max_age: SimDuration,
    queue: Vec<StaffItem>,
    busy_until: SimTime,
    dropped: Vec<Dropped>,
}

/// What the staff did (rule 10).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StaffReport {
    pub processed: Vec<Processed>,
    pub dropped: Vec<Dropped>,
}

impl StaffCell {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            items_per_h: f.get("c2.staff.items_per_h"),
            slowdown: f.get("c2.staff.overload_slowdown"),
            max_queue: f.get("c2.staff.max_queue").round() as usize,
            max_age: SimDuration::from_hours(f.get("c2.staff.max_age_h")),
            queue: Vec::new(),
            busy_until: SimTime::ZERO,
            dropped: Vec::new(),
        }
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// An item arrives. If the queue is full, the least urgent (newest on ties) is dropped.
    pub fn receive(&mut self, item: StaffItem) {
        let at = item.arrived;
        self.queue.push(item);
        if self.queue.len() > self.max_queue {
            let (i, _) = self.queue.iter().enumerate().max_by(|a, b| a.1.priority.cmp(&b.1.priority).then(a.1.arrived.cmp(&b.1.arrived)).then(a.1.id.cmp(&b.1.id))).expect("non-empty");
            let d = self.queue.remove(i);
            self.dropped.push(Dropped { id: d.id, at, reason: DropReason::Overflow });
        }
    }

    /// Work until `until`.
    pub fn process(&mut self, until: SimTime) -> StaffReport {
        let mut rep = StaffReport { processed: Vec::new(), dropped: std::mem::take(&mut self.dropped) };
        loop {
            // The next item starts when the cell is free and something has arrived.
            let Some(earliest) = self.queue.iter().map(|q| q.arrived).min() else { break };
            let start = self.busy_until.max(earliest);
            if start > until {
                break;
            }
            let (i, _) = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, q)| q.arrived <= start)
                .min_by(|a, b| a.1.priority.cmp(&b.1.priority).then(a.1.arrived.cmp(&b.1.arrived)).then(a.1.id.cmp(&b.1.id)))
                .expect("an item has arrived by start");
            let load = self.queue.iter().filter(|q| q.arrived <= start).count() as f64;
            let item = self.queue.remove(i);
            if start - item.arrived > self.max_age {
                rep.dropped.push(Dropped { id: item.id, at: start, reason: DropReason::Stale });
                self.busy_until = start;
                continue;
            }
            let slow = 1.0 + self.slowdown * load / self.max_queue as f64;
            let done = start + SimDuration::from_hours(item.work / self.items_per_h * slow);
            self.busy_until = done;
            rep.processed.push(Processed { id: item.id, start, done, wait_min: (start - item.arrived).secs() / 60.0, slowdown: slow });
        }
        rep
    }
}

/// What an HQ shows (R1: CP detection rises with RF emissions and time static).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HqPosture {
    pub emitters_on: u32,
    pub vehicles: u32,
    pub generators: u32,
    pub static_h: f64,
    pub dispersed: bool,
}

/// An HQ's signature (rule 10): factors on detection hazard by channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HqSignature {
    pub rf: f64,
    pub visual: f64,
    pub thermal: f64,
    /// Overall multiplier on detection hazard.
    pub detection_multiplier: f64,
}

pub fn hq_signature(p: &HqPosture, f: &Figures) -> HqSignature {
    let disp = if p.dispersed { f.get("c2.sig.dispersed_factor") } else { 1.0 };
    let rf = 1.0 + f.get("c2.sig.per_emitter") * p.emitters_on as f64;
    let visual = (1.0 + f.get("c2.sig.per_vehicle") * p.vehicles as f64) * disp;
    let thermal = (1.0 + f.get("c2.sig.per_generator") * p.generators as f64) * disp;
    let time = 1.0 + f.get("c2.sig.per_static_h") * p.static_h.max(0.0);
    HqSignature { rf, visual, thermal, detection_multiplier: rf * visual * thermal * time }
}

/// Who takes the seat when an HQ is lost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Succession {
    pub primary: HqId,
    pub alternates: Vec<HqId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Handover {
    pub from: HqId,
    pub to: HqId,
    pub lost_at: SimTime,
    pub effective_at: SimTime,
}

impl Succession {
    /// `lost` is destroyed at `t`: the first surviving alternate takes the
    /// seat after the handover delay. `None` if nobody is left.
    pub fn on_loss(&mut self, lost: HqId, t: SimTime, alive: &BTreeSet<HqId>, f: &Figures) -> Option<Handover> {
        if lost != self.primary {
            self.alternates.retain(|a| *a != lost);
            return None;
        }
        let pos = self.alternates.iter().position(|a| alive.contains(a))?;
        let to = self.alternates.remove(pos);
        self.primary = to;
        Some(Handover { from: lost, to, lost_at: t, effective_at: t + SimDuration::from_hours(f.get("c2.handover_h")) })
    }
}

/// C2 downtime when an HQ relocates.
pub fn relocation_downtime(f: &Figures) -> SimDuration {
    SimDuration::from_hours(f.get("c2.relocation_h"))
}

// ───────────────────────────── structured orders ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Opord,
    Warno,
    Frago,
}

/// Task verbs (C2SIM-like who-what-when-where-why).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerb {
    Attack,
    Defend,
    Seize,
    Secure,
    Clear,
    Destroy,
    Fix,
    Block,
    Screen,
    Guard,
    Delay,
    Withdraw,
    Move,
    Recon,
    SupportByFire,
    Observe,
    Reserve,
    Resupply,
}

impl TaskVerb {
    pub const ALL: [TaskVerb; 18] = [
        TaskVerb::Attack,
        TaskVerb::Defend,
        TaskVerb::Seize,
        TaskVerb::Secure,
        TaskVerb::Clear,
        TaskVerb::Destroy,
        TaskVerb::Fix,
        TaskVerb::Block,
        TaskVerb::Screen,
        TaskVerb::Guard,
        TaskVerb::Delay,
        TaskVerb::Withdraw,
        TaskVerb::Move,
        TaskVerb::Recon,
        TaskVerb::SupportByFire,
        TaskVerb::Observe,
        TaskVerb::Reserve,
        TaskVerb::Resupply,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Roe {
    WeaponsFree,
    WeaponsTight,
    WeaponsHold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionsOnContact {
    Engage,
    Bypass,
    ReportAndHold,
    Withdraw,
    Fix,
}

/// A place: absolute, or relative to a named reference point (how the
/// planners are told positions, never raw coordinates of their own).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Location {
    Geo {
        lat: f64,
        lon: f64,
    },
    Relative {
        #[serde(rename = "ref")]
        reference: String,
        bearing_deg: f64,
        distance_m: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Point { at: Location },
    Line { points: Vec<Location> },
    Polygon { points: Vec<Location> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlMeasureKind {
    PhaseLine,
    Boundary,
    Objective,
    AssemblyArea,
    AxisOfAdvance,
    Route,
    Checkpoint,
    NoFireArea,
    RestrictiveFireArea,
    FreeFireArea,
    Fscl,
    KillBox,
    RestrictedOperationsZone,
}

impl ControlMeasureKind {
    pub const ALL: [ControlMeasureKind; 13] = [
        ControlMeasureKind::PhaseLine,
        ControlMeasureKind::Boundary,
        ControlMeasureKind::Objective,
        ControlMeasureKind::AssemblyArea,
        ControlMeasureKind::AxisOfAdvance,
        ControlMeasureKind::Route,
        ControlMeasureKind::Checkpoint,
        ControlMeasureKind::NoFireArea,
        ControlMeasureKind::RestrictiveFireArea,
        ControlMeasureKind::FreeFireArea,
        ControlMeasureKind::Fscl,
        ControlMeasureKind::KillBox,
        ControlMeasureKind::RestrictedOperationsZone,
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlMeasure {
    pub name: String,
    pub kind: ControlMeasureKind,
    pub geometry: Geometry,
    /// Minutes after the order's H-hour; absent = always.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_from_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_until_min: Option<f64>,
}

/// When a phase ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhaseEnd {
    ObjectiveReached,
    AtTime { min: f64 },
    OnEvent { event: String },
    OnOrder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub name: String,
    pub task: TaskVerb,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Location>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<Location>,
    pub ends_when: PhaseEnd,
}

/// Minutes relative to H-hour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_later_than_min: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitTask {
    pub unit: String,
    pub task: TaskVerb,
    pub purpose: String,
    pub phases: Vec<Phase>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_measures: Vec<ControlMeasure>,
    #[serde(default)]
    pub timings: Timings,
    pub roe: Roe,
    pub actions_on_contact: ActionsOnContact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub purpose: String,
    #[serde(default)]
    pub key_tasks: Vec<String>,
    pub end_state: String,
}

/// A structured order (OPORD, WARNO or FRAGO), C2SIM-shaped **[R8]**.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Orders {
    pub order_id: String,
    pub order_type: OrderType,
    pub issued_by: String,
    /// H-hour, minutes after the order is issued.
    #[serde(default)]
    pub h_hour_min: f64,
    pub intent: Intent,
    pub tasks: Vec<UnitTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_measures: Vec<ControlMeasure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ccir: Vec<Ccir>,
    /// A FRAGO names the order it changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifies: Option<String>,
}

/// What the validator found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// What the validator checks names against.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OrderContext {
    pub units: BTreeSet<String>,
    pub references: BTreeSet<String>,
}

fn check_location(l: &Location, ctx: &OrderContext, at: &str, r: &mut ValidationReport) {
    match l {
        Location::Geo { lat, lon } => {
            if !(-90.0..=90.0).contains(lat) || !(-180.0..=180.0).contains(lon) {
                r.errors.push(format!("{at}: lat/lon out of range ({lat}, {lon})"));
            }
        }
        Location::Relative { reference, bearing_deg, distance_m } => {
            if !ctx.references.contains(reference) && !ctx.units.contains(reference) {
                r.errors.push(format!("{at}: unknown reference `{reference}`"));
            }
            if !(0.0..360.0).contains(bearing_deg) {
                r.errors.push(format!("{at}: bearing {bearing_deg} not in [0, 360)"));
            }
            if !distance_m.is_finite() || *distance_m < 0.0 {
                r.errors.push(format!("{at}: negative distance {distance_m}"));
            }
        }
    }
}

fn check_measure(m: &ControlMeasure, ctx: &OrderContext, at: &str, r: &mut ValidationReport) {
    let at = format!("{at} control measure `{}`", m.name);
    match &m.geometry {
        Geometry::Point { at: p } => check_location(p, ctx, &at, r),
        Geometry::Line { points } => {
            if points.len() < 2 {
                r.errors.push(format!("{at}: a line needs at least 2 points"));
            }
            points.iter().for_each(|p| check_location(p, ctx, &at, r));
        }
        Geometry::Polygon { points } => {
            if points.len() < 3 {
                r.errors.push(format!("{at}: a polygon needs at least 3 points"));
            }
            points.iter().for_each(|p| check_location(p, ctx, &at, r));
        }
    }
    let line_kinds = [ControlMeasureKind::PhaseLine, ControlMeasureKind::Boundary, ControlMeasureKind::Fscl, ControlMeasureKind::Route, ControlMeasureKind::AxisOfAdvance];
    if line_kinds.contains(&m.kind) && !matches!(m.geometry, Geometry::Line { .. }) {
        r.errors.push(format!("{at}: {:?} must be a line", m.kind));
    }
    if let (Some(a), Some(b)) = (m.active_from_min, m.active_until_min) {
        if b < a {
            r.errors.push(format!("{at}: active window ends before it starts"));
        }
    }
}

impl Orders {
    /// Parse and validate in one go.
    pub fn from_json(v: &Value, ctx: &OrderContext) -> Result<(Orders, ValidationReport), String> {
        let o: Orders = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        let r = o.validate(ctx);
        Ok((o, r))
    }

    /// Check names, geometry, timings and consistency.
    pub fn validate(&self, ctx: &OrderContext) -> ValidationReport {
        let mut r = ValidationReport::default();
        if self.order_id.trim().is_empty() {
            r.errors.push("order_id is empty".into());
        }
        if self.intent.purpose.trim().is_empty() || self.intent.end_state.trim().is_empty() {
            r.errors.push("intent needs a purpose and an end state".into());
        }
        if self.order_type == OrderType::Frago && self.modifies.is_none() {
            r.warnings.push("a FRAGO should name the order it modifies".into());
        }
        if self.order_type != OrderType::Warno && self.tasks.is_empty() {
            r.errors.push("no tasks".into());
        }
        for m in &self.control_measures {
            check_measure(m, ctx, "order", &mut r);
        }
        let mut seen = BTreeSet::new();
        for t in &self.tasks {
            let at = format!("task for `{}`", t.unit);
            if !ctx.units.contains(&t.unit) {
                r.errors.push(format!("{at}: unknown unit"));
            }
            if !seen.insert(t.unit.clone()) {
                r.errors.push(format!("{at}: unit tasked twice"));
            }
            if t.purpose.trim().is_empty() {
                r.errors.push(format!("{at}: no purpose"));
            }
            if t.phases.is_empty() {
                r.errors.push(format!("{at}: no phases"));
            }
            for (i, p) in t.phases.iter().enumerate() {
                if let Some(o) = &p.objective {
                    check_location(o, ctx, &format!("{at} phase {}", p.name), &mut r);
                }
                for l in &p.route {
                    check_location(l, ctx, &format!("{at} phase {} route", p.name), &mut r);
                }
                if p.ends_when == PhaseEnd::ObjectiveReached && p.objective.is_none() {
                    r.errors.push(format!("{at} phase {}: ends on reaching an objective it doesn't have", p.name));
                }
                if p.ends_when == PhaseEnd::OnOrder && i + 1 < t.phases.len() {
                    r.warnings.push(format!("{at} phase {}: later phases wait for an order", p.name));
                }
            }
            if let (Some(a), Some(b)) = (t.timings.not_before_min, t.timings.not_later_than_min) {
                if b < a {
                    r.errors.push(format!("{at}: not-later-than before not-before"));
                }
            }
            for m in &t.control_measures {
                check_measure(m, ctx, &at, &mut r);
            }
            if let Some(s) = &t.supports {
                if s == &t.unit {
                    r.errors.push(format!("{at}: supports itself"));
                } else if !ctx.units.contains(s) {
                    r.errors.push(format!("{at}: supports unknown unit `{s}`"));
                }
            }
            if t.roe == Roe::WeaponsHold && t.actions_on_contact == ActionsOnContact::Engage {
                r.warnings.push(format!("{at}: weapons hold but actions on contact = engage"));
            }
        }
        r
    }
}

/// The JSON Schema (draft 2020-12) of [`Orders`], written by hand to match
/// the serde shapes above (a test checks they agree).
pub fn orders_json_schema() -> Value {
    let verbs: Vec<Value> = TaskVerb::ALL.iter().map(|v| serde_json::to_value(v).expect("enum")).collect();
    let kinds: Vec<Value> = ControlMeasureKind::ALL.iter().map(|v| serde_json::to_value(v).expect("enum")).collect();
    let location = json!({
        "oneOf": [
            { "type": "object", "required": ["lat", "lon"], "additionalProperties": false,
              "properties": { "lat": { "type": "number", "minimum": -90, "maximum": 90 }, "lon": { "type": "number", "minimum": -180, "maximum": 180 } } },
            { "type": "object", "required": ["ref", "bearing_deg", "distance_m"], "additionalProperties": false,
              "properties": { "ref": { "type": "string" }, "bearing_deg": { "type": "number", "minimum": 0, "exclusiveMaximum": 360 }, "distance_m": { "type": "number", "minimum": 0 } } }
        ]
    });
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Orders",
        "description": "Structured orders (OPORD/WARNO/FRAGO), C2SIM-shaped: who, what, when, where, why.",
        "type": "object",
        "required": ["order_id", "order_type", "issued_by", "intent", "tasks"],
        "additionalProperties": false,
        "properties": {
            "order_id": { "type": "string", "minLength": 1 },
            "order_type": { "enum": ["opord", "warno", "frago"] },
            "issued_by": { "type": "string" },
            "h_hour_min": { "type": "number" },
            "modifies": { "type": "string" },
            "intent": {
                "type": "object", "required": ["purpose", "end_state"], "additionalProperties": false,
                "properties": { "purpose": { "type": "string" }, "key_tasks": { "type": "array", "items": { "type": "string" } }, "end_state": { "type": "string" } }
            },
            "tasks": { "type": "array", "items": { "$ref": "#/$defs/unit_task" } },
            "control_measures": { "type": "array", "items": { "$ref": "#/$defs/control_measure" } },
            "ccir": { "type": "array", "items": { "$ref": "#/$defs/ccir" } }
        },
        "$defs": {
            "location": location,
            "task_verb": { "enum": verbs },
            "geometry": {
                "oneOf": [
                    { "type": "object", "required": ["type", "at"], "additionalProperties": false,
                      "properties": { "type": { "const": "point" }, "at": { "$ref": "#/$defs/location" } } },
                    { "type": "object", "required": ["type", "points"], "additionalProperties": false,
                      "properties": { "type": { "const": "line" }, "points": { "type": "array", "minItems": 2, "items": { "$ref": "#/$defs/location" } } } },
                    { "type": "object", "required": ["type", "points"], "additionalProperties": false,
                      "properties": { "type": { "const": "polygon" }, "points": { "type": "array", "minItems": 3, "items": { "$ref": "#/$defs/location" } } } }
                ]
            },
            "control_measure": {
                "type": "object", "required": ["name", "kind", "geometry"], "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "enum": kinds },
                    "geometry": { "$ref": "#/$defs/geometry" },
                    "active_from_min": { "type": "number" },
                    "active_until_min": { "type": "number" }
                }
            },
            "phase_end": {
                "oneOf": [
                    { "type": "object", "required": ["kind"], "additionalProperties": false, "properties": { "kind": { "const": "objective_reached" } } },
                    { "type": "object", "required": ["kind", "min"], "additionalProperties": false, "properties": { "kind": { "const": "at_time" }, "min": { "type": "number" } } },
                    { "type": "object", "required": ["kind", "event"], "additionalProperties": false, "properties": { "kind": { "const": "on_event" }, "event": { "type": "string" } } },
                    { "type": "object", "required": ["kind"], "additionalProperties": false, "properties": { "kind": { "const": "on_order" } } }
                ]
            },
            "phase": {
                "type": "object", "required": ["name", "task", "ends_when"], "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "task": { "$ref": "#/$defs/task_verb" },
                    "objective": { "$ref": "#/$defs/location" },
                    "route": { "type": "array", "items": { "$ref": "#/$defs/location" } },
                    "ends_when": { "$ref": "#/$defs/phase_end" }
                }
            },
            "timings": {
                "type": "object", "additionalProperties": false,
                "properties": { "not_before_min": { "type": "number" }, "not_later_than_min": { "type": "number" } }
            },
            "unit_task": {
                "type": "object", "required": ["unit", "task", "purpose", "phases", "roe", "actions_on_contact"], "additionalProperties": false,
                "properties": {
                    "unit": { "type": "string" },
                    "task": { "$ref": "#/$defs/task_verb" },
                    "purpose": { "type": "string", "minLength": 1 },
                    "phases": { "type": "array", "minItems": 1, "items": { "$ref": "#/$defs/phase" } },
                    "control_measures": { "type": "array", "items": { "$ref": "#/$defs/control_measure" } },
                    "timings": { "$ref": "#/$defs/timings" },
                    "roe": { "enum": ["weapons_free", "weapons_tight", "weapons_hold"] },
                    "actions_on_contact": { "enum": ["engage", "bypass", "report_and_hold", "withdraw", "fix"] },
                    "supports": { "type": "string" }
                }
            },
            "ccir": {
                "type": "object", "required": ["id", "kind", "question"], "additionalProperties": false,
                "properties": {
                    "id": { "type": "string" },
                    "kind": { "enum": ["pir", "ffir", "eefi"] },
                    "question": { "type": "string" },
                    "triggers_replan": { "type": "boolean" }
                }
            }
        }
    })
}

/// A minimal JSON Schema checker for the subset [`orders_json_schema`] uses
/// (type, required, properties, additionalProperties: false, enum, const,
/// oneOf, items, minItems, minLength, minimum, maximum, exclusiveMaximum,
/// local `$ref`). Returns the paths that fail. Enough to keep the schema and
/// the serde types honest, and to pre-check a planner's answer.
pub fn schema_errors(schema: &Value, v: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    check(schema, schema, v, "$", &mut errs);
    errs
}

fn resolve<'a>(root: &'a Value, s: &'a Value) -> &'a Value {
    match s.get("$ref").and_then(Value::as_str) {
        Some(r) => r.strip_prefix("#/").map(|p| p.split('/').fold(root, |acc, k| &acc[k])).unwrap_or(s),
        None => s,
    }
}

fn check(root: &Value, schema: &Value, v: &Value, path: &str, errs: &mut Vec<String>) {
    let s = resolve(root, schema);
    if let Some(alts) = s.get("oneOf").and_then(Value::as_array) {
        let ok = alts.iter().filter(|a| {
            let mut e = Vec::new();
            check(root, a, v, path, &mut e);
            e.is_empty()
        });
        if ok.count() != 1 {
            errs.push(format!("{path}: matches none (or several) of oneOf"));
        }
        return;
    }
    if let Some(c) = s.get("const") {
        if c != v {
            errs.push(format!("{path}: expected {c}"));
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            errs.push(format!("{path}: {v} not in enum"));
        }
    }
    if let Some(t) = s.get("type").and_then(Value::as_str) {
        let ok = match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            _ => true,
        };
        if !ok {
            errs.push(format!("{path}: expected {t}"));
            return;
        }
    }
    if let (Some(n), Some(x)) = (s.get("minimum").and_then(Value::as_f64), v.as_f64()) {
        if x < n {
            errs.push(format!("{path}: {x} < minimum {n}"));
        }
    }
    if let (Some(n), Some(x)) = (s.get("maximum").and_then(Value::as_f64), v.as_f64()) {
        if x > n {
            errs.push(format!("{path}: {x} > maximum {n}"));
        }
    }
    if let (Some(n), Some(x)) = (s.get("exclusiveMaximum").and_then(Value::as_f64), v.as_f64()) {
        if x >= n {
            errs.push(format!("{path}: {x} ≥ exclusiveMaximum {n}"));
        }
    }
    if let (Some(n), Some(x)) = (s.get("minLength").and_then(Value::as_u64), v.as_str()) {
        if (x.chars().count() as u64) < n {
            errs.push(format!("{path}: shorter than {n}"));
        }
    }
    if let Some(obj) = v.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        if let Some(req) = s.get("required").and_then(Value::as_array) {
            for r in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    errs.push(format!("{path}: missing `{r}`"));
                }
            }
        }
        let closed = s.get("additionalProperties") == Some(&Value::Bool(false));
        let sorted: BTreeMap<&String, &Value> = obj.iter().collect();
        for (k, val) in sorted {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => check(root, ps, val, &format!("{path}.{k}"), errs),
                None if closed => errs.push(format!("{path}: unexpected `{k}`")),
                None => {}
            }
        }
    }
    if let Some(arr) = v.as_array() {
        if let Some(n) = s.get("minItems").and_then(Value::as_u64) {
            if (arr.len() as u64) < n {
                errs.push(format!("{path}: fewer than {n} items"));
            }
        }
        if let Some(items) = s.get("items") {
            for (i, x) in arr.iter().enumerate() {
                check(root, items, x, &format!("{path}[{i}]"), errs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn figs() -> Figures {
        crate::default_figures()
    }

    #[test]
    fn one_third_rule_cascade() {
        let c = planning_cascade(SimDuration::from_hours(24.0), Echelon::Brigade, &figs());
        let h = |d: SimDuration| d.secs() / 3600.0;
        assert_eq!(c.len(), 4);
        assert_relative_eq!(h(c[0].own), 8.0, epsilon = 1e-6);
        assert_relative_eq!(h(c[1].available), 16.0, epsilon = 1e-6);
        assert!((h(c[2].available) - 10.67).abs() < 0.05);
        assert!((h(c[3].available) - 7.1).abs() < 0.05);
        assert!(c[0].quality < 1.0 && c[0].quality > 0.5);
        // With plenty of time, an HQ caps at its deliberate time.
        let long = planning_cascade(SimDuration::from_hours(200.0), Echelon::Battalion, &figs());
        assert_relative_eq!(h(long[0].own), 9.0, epsilon = 1e-6);
        assert_relative_eq!(long[0].quality, 1.0);
    }

    #[test]
    fn ccir_triggers_hasty_plan() {
        let f = figs();
        let c = Ccir { id: "PIR1".into(), kind: CcirKind::Pir, question: "Enemy reserve moves?".into(), triggers_replan: true };
        let p = on_ccir(&c, Echelon::Battalion, SimTime::from_secs_i(100), &f).unwrap();
        assert_eq!(p.ready_at, SimTime::from_secs_i(100) + SimDuration::from_hours(1.5));
        assert!(p.quality < 0.5);
        assert!(on_ccir(&Ccir { triggers_replan: false, ..c }, Echelon::Battalion, SimTime::ZERO, &f).is_none());
    }

    #[test]
    fn battle_rhythm_schedule() {
        let ev = vec![
            RhythmEvent { name: "CUB".into(), period: SimDuration::from_hours(12.0), offset: SimDuration::from_hours(6.0), duration: SimDuration::from_mins(30.0) },
            RhythmEvent { name: "Targeting".into(), period: SimDuration::from_hours(24.0), offset: SimDuration::from_hours(6.0), duration: SimDuration::from_hours(1.0) },
        ];
        let occ = battle_rhythm(&ev, SimTime::from_secs_i(3600), SimTime::from_secs_i(48 * 3600));
        let names: Vec<&str> = occ.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["CUB", "Targeting", "CUB", "CUB", "Targeting", "CUB"]);
        assert_eq!(occ[0].start, SimTime::from_secs_i(6 * 3600));
    }

    #[test]
    fn staff_overload_slows_and_drops() {
        let f = figs();
        let mut s = StaffCell::from_figures(&f);
        for i in 0..60 {
            s.receive(StaffItem { id: i, priority: if i % 10 == 0 { 0 } else { 2 }, arrived: SimTime::ZERO, work: 1.5 });
        }
        assert_eq!(s.queue_len(), 40);
        let r = s.process(SimTime::from_secs_i(10 * 3600));
        let overflow = r.dropped.iter().filter(|d| d.reason == DropReason::Overflow).count();
        let stale = r.dropped.iter().filter(|d| d.reason == DropReason::Stale).count();
        assert_eq!(overflow, 20);
        assert!(stale > 0, "{r:?}");
        // Urgent items went first, and the first ones were slowed by the backlog.
        assert_eq!(r.processed[0].id, 0);
        assert!(r.processed[0].slowdown > 1.5);
        assert!(r.processed.iter().all(|p| p.wait_min <= 120.0 + 1e-9));
        // A light load is quick.
        let mut light = StaffCell::from_figures(&f);
        light.receive(StaffItem { id: 1, priority: 1, arrived: SimTime::from_secs_i(50), work: 1.0 });
        let r = light.process(SimTime::from_secs_i(3600));
        assert_eq!(r.processed[0].start, SimTime::from_secs_i(50));
        assert!((r.processed[0].done - r.processed[0].start).secs() < 130.0);
    }

    #[test]
    fn hq_signature_and_succession() {
        let f = figs();
        let quiet = hq_signature(&HqPosture { emitters_on: 0, vehicles: 4, generators: 0, static_h: 1.0, dispersed: true }, &f);
        let loud = hq_signature(&HqPosture { emitters_on: 4, vehicles: 20, generators: 2, static_h: 48.0, dispersed: false }, &f);
        assert!(loud.detection_multiplier > 20.0 * quiet.detection_multiplier);
        let mut s = Succession { primary: HqId(1), alternates: vec![HqId(2), HqId(3)] };
        let alive: BTreeSet<HqId> = [HqId(3)].into();
        let h = s.on_loss(HqId(1), SimTime::from_secs_i(100), &alive, &f).unwrap();
        assert_eq!(h.to, HqId(3));
        assert_eq!(h.effective_at, SimTime::from_secs_i(100) + SimDuration::from_hours(0.5));
        assert_eq!(s.primary, HqId(3));
        assert!(s.on_loss(HqId(3), SimTime::from_secs_i(200), &BTreeSet::new(), &f).is_none());
        assert!(relocation_downtime(&f).secs() > 0.0);
    }

    fn sample() -> Value {
        json!({
            "order_id": "OPORD-7",
            "order_type": "opord",
            "issued_by": "HQ1",
            "h_hour_min": 60,
            "intent": { "purpose": "Seize the crossing to let the brigade pass", "key_tasks": ["seize OBJ RED"], "end_state": "Crossing held, enemy pushed beyond PL GOLD" },
            "control_measures": [
                { "name": "PL GOLD", "kind": "phase_line", "geometry": { "type": "line", "points": [ { "ref": "bridge", "bearing_deg": 90, "distance_m": 2000 }, { "lat": 50.1, "lon": 30.2 } ] } },
                { "name": "NFA church", "kind": "no_fire_area", "geometry": { "type": "polygon", "points": [ { "lat": 50.0, "lon": 30.0 }, { "lat": 50.01, "lon": 30.0 }, { "lat": 50.0, "lon": 30.01 } ] }, "active_from_min": 0, "active_until_min": 600 }
            ],
            "ccir": [ { "id": "PIR1", "kind": "pir", "question": "Does the enemy reserve move south?", "triggers_replan": true } ],
            "tasks": [
                {
                    "unit": "A1", "task": "seize", "purpose": "to secure the crossing",
                    "phases": [
                        { "name": "move", "task": "move", "route": [ { "ref": "bridge", "bearing_deg": 270, "distance_m": 3000 } ], "ends_when": { "kind": "at_time", "min": 90 } },
                        { "name": "assault", "task": "attack", "objective": { "ref": "bridge", "bearing_deg": 0, "distance_m": 0 }, "ends_when": { "kind": "objective_reached" } }
                    ],
                    "timings": { "not_before_min": 0, "not_later_than_min": 240 },
                    "roe": "weapons_free", "actions_on_contact": "engage"
                },
                {
                    "unit": "B1", "task": "support_by_fire", "purpose": "to fix the defenders", "supports": "A1",
                    "phases": [ { "name": "overwatch", "task": "support_by_fire", "ends_when": { "kind": "on_order" } } ],
                    "roe": "weapons_tight", "actions_on_contact": "fix"
                }
            ]
        })
    }

    fn ctx() -> OrderContext {
        OrderContext { units: ["A1", "B1", "C1"].iter().map(|s| s.to_string()).collect(), references: ["bridge"].iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn orders_parse_validate_and_match_the_schema() {
        let schema = orders_json_schema();
        let v = sample();
        assert_eq!(schema_errors(&schema, &v), Vec::<String>::new());
        let (o, r) = Orders::from_json(&v, &ctx()).unwrap();
        assert!(r.is_ok(), "{r:?}");
        // Round trip: what we serialise also matches the schema.
        let back = serde_json::to_value(&o).unwrap();
        assert_eq!(schema_errors(&schema, &back), Vec::<String>::new());
        assert_eq!(serde_json::from_value::<Orders>(back).unwrap(), o);
        // Every enum variant serde produces is in the schema.
        for verb in TaskVerb::ALL {
            let mut t = o.clone();
            t.tasks[0].task = verb;
            assert!(schema_errors(&schema, &serde_json::to_value(&t).unwrap()).is_empty());
        }
        for kind in ControlMeasureKind::ALL {
            let mut t = o.clone();
            t.control_measures[1].kind = kind;
            assert!(schema_errors(&schema, &serde_json::to_value(&t).unwrap()).is_empty());
        }
        for roe in [Roe::WeaponsFree, Roe::WeaponsTight, Roe::WeaponsHold] {
            for aoc in [ActionsOnContact::Engage, ActionsOnContact::Bypass, ActionsOnContact::ReportAndHold, ActionsOnContact::Withdraw, ActionsOnContact::Fix] {
                let mut t = o.clone();
                t.tasks[0].roe = roe;
                t.tasks[0].actions_on_contact = aoc;
                assert!(schema_errors(&schema, &serde_json::to_value(&t).unwrap()).is_empty());
            }
        }
    }

    #[test]
    fn bad_orders_are_caught() {
        let schema = orders_json_schema();
        let mut v = sample();
        v["tasks"][0]["roe"] = json!("shoot_everything");
        assert!(!schema_errors(&schema, &v).is_empty());
        assert!(serde_json::from_value::<Orders>(v).is_err());
        let mut v = sample();
        v["tasks"][0]["colour"] = json!("blue");
        assert!(!schema_errors(&schema, &v).is_empty());
        assert!(serde_json::from_value::<Orders>(v).is_err());
        // Schema-valid but semantically wrong.
        let mut v = sample();
        v["tasks"][1]["unit"] = json!("Z9");
        v["tasks"][1]["supports"] = json!("Z9");
        v["tasks"][0]["timings"] = json!({ "not_before_min": 100, "not_later_than_min": 50 });
        v["control_measures"][0]["geometry"] = json!({ "type": "point", "at": { "ref": "nowhere", "bearing_deg": 10, "distance_m": 5 } });
        assert!(schema_errors(&schema, &v).is_empty());
        let (_, r) = Orders::from_json(&v, &ctx()).unwrap();
        let all = r.errors.join("\n");
        for needle in ["unknown unit", "supports itself", "not-later-than", "unknown reference", "must be a line"] {
            assert!(all.contains(needle), "missing `{needle}` in\n{all}");
        }
    }
}
