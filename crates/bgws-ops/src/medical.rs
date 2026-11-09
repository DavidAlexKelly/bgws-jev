//! Medical evacuation and care (RUST_ENGINE.md §8.11; research R7).
//!
//! The chain runs from the point of injury to a casualty collection point
//! (CCP), an aid post (Role 1), a surgical team (Role 2) and a hospital
//! (Role 3). Ambulances and helicopters are fleets with capacity and
//! turnaround; the aid post and surgical team are servers with triage
//! queues (T1 immediate, T2 delayed, T3 minimal, T4 expectant).
//!
//! **Mortality is time-dependent by triage category:** the probability of
//! dying by "effective untreated time" τₑ is p_max·(1 − e^(−(τₑ/τ)^k)), and
//! τₑ accrues more slowly the better the care at each stage (buddy aid,
//! medic in transit, damage-control resuscitation, surgery). Each casualty
//! draws its fate once (a keyed uniform), and dies at the moment its
//! cumulative probability passes it, so the dead free their place in the
//! queues. Evacuation delays of 60–120 min under drone observation **[R7]**
//! are an input; "mortality scales with time to care" is the acceptance test.

use bgws_core::{figure, Figure, Figures, Seed, SimDuration, SimTime, StreamKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Figures for medical (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("med.t1.p_max", 0.7, "fraction", 0.3, 0.95, "default", Default, "T1: eventual death fraction if never treated"),
        figure!("med.t1.tau_h", 3.0, "h", 0.5, 12.0, "R7", Low, "T1 untreated time scale ('golden hour' regime)"),
        figure!("med.t1.k", 1.5, "1", 0.5, 3.0, "default", Default, "T1 Weibull shape"),
        figure!("med.t2.p_max", 0.3, "fraction", 0.05, 0.6, "default", Default, "T2 eventual death fraction untreated"),
        figure!("med.t2.tau_h", 12.0, "h", 2.0, 48.0, "default", Default, "T2 time scale"),
        figure!("med.t2.k", 1.5, "1", 0.5, 3.0, "default", Default, "T2 Weibull shape"),
        figure!("med.t3.p_max", 0.02, "fraction", 0.0, 0.1, "default", Default, "T3 eventual death fraction"),
        figure!("med.t3.tau_h", 72.0, "h", 12.0, 240.0, "default", Default, "T3 time scale"),
        figure!("med.t3.k", 1.0, "1", 0.5, 3.0, "default", Default, "T3 Weibull shape"),
        figure!("med.t4.p_max", 0.95, "fraction", 0.5, 1.0, "default", Default, "T4 eventual death fraction"),
        figure!("med.t4.tau_h", 1.0, "h", 0.1, 6.0, "default", Default, "T4 time scale"),
        figure!("med.t4.k", 1.0, "1", 0.5, 3.0, "default", Default, "T4 Weibull shape"),
        figure!("med.care.poi", 1.0, "1", 0.5, 1.0, "default", Default, "untreated-time rate at the point of injury (self/buddy aid)"),
        figure!("med.care.ccp", 0.85, "1", 0.3, 1.0, "default", Default, "rate at the CCP (combat medic)"),
        figure!("med.care.transit", 0.8, "1", 0.3, 1.0, "default", Default, "rate in an ambulance or helicopter"),
        figure!("med.care.aid_post", 0.4, "1", 0.1, 1.0, "default", Default, "rate at the aid post (damage-control resuscitation)"),
        figure!("med.care.post_surgery", 0.005, "1", 0.0, 0.1, "default", Default, "rate after surgery"),
        figure!("med.aid.treat_min", 20.0, "min", 5.0, 60.0, "default", Default, "aid post treatment time"),
        figure!("med.surgery.t1_min", 90.0, "min", 30.0, 240.0, "default", Default, "damage-control surgery, T1"),
        figure!("med.surgery.t2_min", 60.0, "min", 20.0, 180.0, "default", Default, "surgery, T2"),
        figure!("med.outcome_window_h", 72.0, "h", 24.0, 720.0, "default", Default, "outcome horizon after injury"),
        figure!("med.poi_delay_min", 20.0, "min", 5.0, 1_440.0, "R7", Medium, "extraction to the CCP; 60–120 min under drones, up to 24 h"),
    ]
}

/// Triage categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Triage {
    T1,
    T2,
    T3,
    T4,
}

impl Triage {
    fn key(self) -> &'static str {
        match self {
            Triage::T1 => "t1",
            Triage::T2 => "t2",
            Triage::T3 => "t3",
            Triage::T4 => "t4",
        }
    }
    /// Queue priority: T1, then T2, then T3, then T4.
    fn priority(self) -> u8 {
        self as u8
    }
}

/// Where a casualty is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    PointOfInjury,
    Ccp,
    ForwardTransit,
    AidPost,
    RearTransit,
    Surgical,
    Hospital,
    ReturnedToDuty,
}

/// Mortality curve for one triage category.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mortality {
    pub p_max: f64,
    pub tau_h: f64,
    pub k: f64,
}

impl Mortality {
    pub fn from_figures(t: Triage, f: &Figures) -> Self {
        let k = t.key();
        Self { p_max: f.get(&format!("med.{k}.p_max")), tau_h: f.get(&format!("med.{k}.tau_h")), k: f.get(&format!("med.{k}.k")) }
    }
    /// P(dead by effective untreated time `te_h`).
    pub fn cdf(&self, te_h: f64) -> f64 {
        self.p_max * (1.0 - (-(te_h.max(0.0) / self.tau_h).powf(self.k)).exp())
    }
    /// The effective time at which the cdf reaches `p` (None if never).
    pub fn inverse(&self, p: f64) -> Option<f64> {
        if p >= self.p_max {
            return None;
        }
        Some(self.tau_h * (-(1.0 - p / self.p_max).ln()).powf(1.0 / self.k))
    }
}

/// A vehicle fleet on one leg.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fleet {
    pub name: String,
    /// When each vehicle is next free at the leg's origin.
    pub free_at: Vec<SimTime>,
    pub capacity: usize,
    pub one_way: SimDuration,
    /// Which categories this fleet carries (e.g. helicopters: T1 only).
    pub carries: Vec<Triage>,
}

impl Fleet {
    pub fn new(name: &str, vehicles: usize, capacity: usize, one_way_min: f64, carries: &[Triage]) -> Self {
        Self { name: name.into(), free_at: vec![SimTime::ZERO; vehicles], capacity, one_way: SimDuration::from_mins(one_way_min), carries: carries.to_vec() }
    }
}

/// The evacuation chain's resources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Extraction from the point of injury to the CCP (60–120 min under drones [R7]).
    pub poi_delay: SimDuration,
    /// CCP → aid post.
    pub forward: Vec<Fleet>,
    /// Aid post → surgical team, tried in order (helicopters first if flying).
    pub rear: Vec<Fleet>,
    pub aid_teams: usize,
    pub surgical_tables: usize,
    /// Surgical team → hospital transfer.
    pub to_hospital: SimDuration,
}

/// Waits and times per casualty (rule 10).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageTimes {
    pub reached_ccp: Option<SimTime>,
    pub reached_aid_post: Option<SimTime>,
    pub treated: Option<SimTime>,
    pub reached_surgical: Option<SimTime>,
    pub surgery_start: Option<SimTime>,
    pub reached_hospital: Option<SimTime>,
    pub ccp_wait_min: f64,
    pub aid_wait_min: f64,
    pub rear_wait_min: f64,
    pub surgery_wait_min: f64,
}

/// A casualty's outcome (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CasualtyOutcome {
    pub id: u64,
    pub triage: Triage,
    pub injured_at: SimTime,
    pub survived: bool,
    pub died_at: Option<SimTime>,
    pub stage_reached: Stage,
    /// Effective untreated hours accrued by the end.
    pub effective_h: f64,
    /// Analytic probability of death given the path taken.
    pub p_death: f64,
    pub times: StageTimes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Casualty {
    id: u64,
    triage: Triage,
    injured_at: SimTime,
    u: f64,
    stage: Stage,
    care_rate: f64,
    last: SimTime,
    eff_h: f64,
    dead_at: Option<SimTime>,
    times: StageTimes,
    queued_at: SimTime,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Ev {
    ReachCcp(u64),
    ReachAid(u64),
    Treated(u64, usize),
    ReachSurgical(u64),
    SurgeryDone(u64, usize),
    ReachHospital(u64),
    Retry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Params {
    mort: BTreeMap<Triage, Mortality>,
    care_poi: f64,
    care_ccp: f64,
    care_transit: f64,
    care_aid: f64,
    care_post: f64,
    treat: SimDuration,
    surg_t1: SimDuration,
    surg_t2: SimDuration,
    window: SimDuration,
}

/// The medical chain as a discrete-event simulation.
#[derive(Clone, Debug)]
pub struct MedicalChain {
    cfg: ChainConfig,
    p: Params,
    seed: Seed,
    now: SimTime,
    cas: BTreeMap<u64, Casualty>,
    ccp_q: Vec<u64>,
    aid_q: Vec<u64>,
    rear_q: Vec<u64>,
    surg_q: Vec<u64>,
    aid_free: Vec<SimTime>,
    tables_free: Vec<SimTime>,
    events: BTreeMap<(SimTime, u64), Ev>,
    retries: std::collections::BTreeSet<SimTime>,
    seq: u64,
}

impl MedicalChain {
    pub fn new(cfg: ChainConfig, f: &Figures, seed: Seed) -> Self {
        let mort = [Triage::T1, Triage::T2, Triage::T3, Triage::T4].into_iter().map(|t| (t, Mortality::from_figures(t, f))).collect();
        let p = Params {
            mort,
            care_poi: f.get("med.care.poi"),
            care_ccp: f.get("med.care.ccp"),
            care_transit: f.get("med.care.transit"),
            care_aid: f.get("med.care.aid_post"),
            care_post: f.get("med.care.post_surgery"),
            treat: SimDuration::from_mins(f.get("med.aid.treat_min")),
            surg_t1: SimDuration::from_mins(f.get("med.surgery.t1_min")),
            surg_t2: SimDuration::from_mins(f.get("med.surgery.t2_min")),
            window: SimDuration::from_hours(f.get("med.outcome_window_h")),
        };
        let aid_free = vec![SimTime::ZERO; cfg.aid_teams];
        let tables_free = vec![SimTime::ZERO; cfg.surgical_tables];
        Self { cfg, p, seed, now: SimTime::ZERO, cas: BTreeMap::new(), ccp_q: vec![], aid_q: vec![], rear_q: vec![], surg_q: vec![], aid_free, tables_free, events: BTreeMap::new(), retries: Default::default(), seq: 0 }
    }

    /// The default extraction delay from figures.
    pub fn default_poi_delay(f: &Figures) -> SimDuration {
        SimDuration::from_mins(f.get("med.poi_delay_min"))
    }

    fn schedule(&mut self, t: SimTime, ev: Ev) {
        if ev == Ev::Retry && !self.retries.insert(t) {
            return;
        }
        self.events.insert((t, self.seq), ev);
        self.seq += 1;
    }

    /// A casualty at time `t`.
    pub fn injure(&mut self, id: u64, triage: Triage, t: SimTime) {
        let u: f64 = self.seed.stream(StreamKey::new("med.fate", id, 0, 0)).gen();
        let c = Casualty {
            id,
            triage,
            injured_at: t,
            u,
            stage: Stage::PointOfInjury,
            care_rate: self.p.care_poi,
            last: t,
            eff_h: 0.0,
            dead_at: None,
            times: StageTimes::default(),
            queued_at: t,
            finished: false,
        };
        self.cas.insert(id, c);
        self.schedule(t + self.cfg.poi_delay, Ev::ReachCcp(id));
    }

    /// Advance a casualty's clock to `t`; returns false if it died on the way.
    fn advance(&mut self, id: u64, t: SimTime) -> bool {
        let m = self.p.mort[&self.cas[&id].triage];
        let c = self.cas.get_mut(&id).expect("casualty");
        if c.dead_at.is_some() {
            return false;
        }
        let dt_h = (t - c.last).secs().max(0.0) / 3600.0;
        let new_eff = c.eff_h + dt_h * c.care_rate;
        if m.cdf(new_eff) > c.u {
            let te = m.inverse(c.u).unwrap_or(new_eff);
            let frac = if c.care_rate > 0.0 { (te - c.eff_h) / c.care_rate } else { dt_h };
            c.dead_at = Some(c.last + SimDuration::from_hours(frac.clamp(0.0, dt_h)));
            c.eff_h = te;
            c.last = t;
            return false;
        }
        c.eff_h = new_eff;
        c.last = t;
        true
    }

    fn set_stage(&mut self, id: u64, stage: Stage, care: f64, t: SimTime) {
        let c = self.cas.get_mut(&id).expect("casualty");
        c.stage = stage;
        c.care_rate = care;
        c.queued_at = t;
    }

    /// Run the chain to `until`. Returns outcomes for casualties whose
    /// outcome window has closed by then (and anyone who died).
    pub fn run(&mut self, until: SimTime) -> Vec<CasualtyOutcome> {
        while let Some((&(t, s), _)) = self.events.iter().next() {
            if t > until {
                break;
            }
            let ev = self.events.remove(&(t, s)).expect("event");
            self.now = t;
            self.handle(t, ev);
        }
        self.now = until;
        self.collect(until)
    }

    fn by_priority(&self, q: &mut [u64]) {
        q.sort_by(|a, b| {
            let (ca, cb) = (&self.cas[a], &self.cas[b]);
            ca.triage.priority().cmp(&cb.triage.priority()).then(ca.queued_at.cmp(&cb.queued_at)).then(a.cmp(b))
        });
    }

    fn handle(&mut self, t: SimTime, ev: Ev) {
        match ev {
            Ev::ReachCcp(id) => {
                if self.advance(id, t) {
                    self.set_stage(id, Stage::Ccp, self.p.care_ccp, t);
                    self.cas.get_mut(&id).expect("c").times.reached_ccp = Some(t);
                    self.ccp_q.push(id);
                }
            }
            Ev::ReachAid(id) => {
                if self.advance(id, t) {
                    self.set_stage(id, Stage::AidPost, self.p.care_aid, t);
                    self.cas.get_mut(&id).expect("c").times.reached_aid_post = Some(t);
                    self.aid_q.push(id);
                }
            }
            Ev::Treated(id, _team) => {
                if self.advance(id, t) {
                    let c = self.cas.get_mut(&id).expect("c");
                    c.times.treated = Some(t);
                    match c.triage {
                        Triage::T3 => {
                            c.stage = Stage::ReturnedToDuty;
                            c.care_rate = 0.0;
                        }
                        Triage::T4 => {} // expectant: comfort care at the aid post
                        _ => {
                            c.queued_at = t;
                            self.rear_q.push(id);
                        }
                    }
                }
            }
            Ev::ReachSurgical(id) => {
                if self.advance(id, t) {
                    self.set_stage(id, Stage::Surgical, self.p.care_aid, t);
                    self.cas.get_mut(&id).expect("c").times.reached_surgical = Some(t);
                    self.surg_q.push(id);
                }
            }
            Ev::SurgeryDone(id, _table) => {
                if self.advance(id, t) {
                    let c = self.cas.get_mut(&id).expect("c");
                    c.care_rate = self.p.care_post;
                    let at = t + self.cfg.to_hospital;
                    self.schedule(at, Ev::ReachHospital(id));
                }
            }
            Ev::ReachHospital(id) => {
                if self.advance(id, t) {
                    let post = self.p.care_post;
                    self.set_stage(id, Stage::Hospital, post, t);
                    self.cas.get_mut(&id).expect("c").times.reached_hospital = Some(t);
                }
            }
            Ev::Retry => {
                self.retries.remove(&t);
            }
        }
        self.dispatch(t);
    }

    /// Start whatever can start at `t`: transport legs, treatment, surgery.
    fn dispatch(&mut self, t: SimTime) {
        // Forward evacuation CCP → aid post.
        let mut q = std::mem::take(&mut self.ccp_q);
        q.retain(|id| self.cas[id].dead_at.is_none());
        let mut fleets = std::mem::take(&mut self.cfg.forward);
        self.load_fleets(t, &mut q, &mut fleets, Stage::ForwardTransit, true);
        self.cfg.forward = fleets;
        self.ccp_q = q;
        // Aid post treatment.
        let mut q = std::mem::take(&mut self.aid_q);
        q.retain(|id| self.advance_peek(*id, t));
        self.by_priority(&mut q);
        let mut rest = Vec::new();
        for id in q {
            let free = self.aid_free.iter().enumerate().filter(|(_, f)| **f <= t).map(|(i, _)| i).next();
            match free {
                Some(i) => {
                    let done = t + self.p.treat;
                    self.aid_free[i] = done;
                    let c = self.cas.get_mut(&id).expect("c");
                    c.times.aid_wait_min += (t - c.queued_at).secs() / 60.0;
                    self.schedule(done, Ev::Treated(id, i));
                }
                None => rest.push(id),
            }
        }
        if !rest.is_empty() {
            if let Some(next) = self.aid_free.iter().copied().min() {
                self.schedule(next, Ev::Retry);
            }
        }
        self.aid_q = rest;
        // Rear evacuation aid post → surgical.
        let mut q = std::mem::take(&mut self.rear_q);
        q.retain(|id| self.cas[id].dead_at.is_none());
        let mut fleets = std::mem::take(&mut self.cfg.rear);
        self.load_fleets(t, &mut q, &mut fleets, Stage::RearTransit, false);
        self.cfg.rear = fleets;
        self.rear_q = q;
        // Surgery.
        let mut q = std::mem::take(&mut self.surg_q);
        q.retain(|id| self.advance_peek(*id, t));
        self.by_priority(&mut q);
        let mut rest = Vec::new();
        for id in q {
            let free = self.tables_free.iter().enumerate().filter(|(_, f)| **f <= t).map(|(i, _)| i).next();
            match free {
                Some(i) => {
                    let dur = if self.cas[&id].triage == Triage::T1 { self.p.surg_t1 } else { self.p.surg_t2 };
                    self.tables_free[i] = t + dur;
                    let c = self.cas.get_mut(&id).expect("c");
                    c.times.surgery_wait_min += (t - c.queued_at).secs() / 60.0;
                    c.times.surgery_start = Some(t);
                    self.schedule(t + dur, Ev::SurgeryDone(id, i));
                }
                None => rest.push(id),
            }
        }
        if !rest.is_empty() {
            if let Some(next) = self.tables_free.iter().copied().min() {
                self.schedule(next, Ev::Retry);
            }
        }
        self.surg_q = rest;
    }

    /// Advance and report whether still alive (removing the dead from a queue).
    fn advance_peek(&mut self, id: u64, t: SimTime) -> bool {
        self.advance(id, t)
    }

    fn load_fleets(&mut self, t: SimTime, q: &mut Vec<u64>, fleets: &mut [Fleet], transit: Stage, forward: bool) {
        // Kill anyone who died while waiting.
        let ids: Vec<u64> = q.clone();
        q.clear();
        for id in ids {
            if self.advance(id, t) {
                q.push(id);
            }
        }
        self.by_priority(q);
        for fleet in fleets.iter_mut() {
            for v in 0..fleet.free_at.len() {
                if fleet.free_at[v] > t {
                    continue;
                }
                let load: Vec<u64> = q.iter().copied().filter(|id| fleet.carries.contains(&self.cas[id].triage)).take(fleet.capacity).collect();
                if load.is_empty() {
                    break;
                }
                q.retain(|id| !load.contains(id));
                fleet.free_at[v] = t + fleet.one_way + fleet.one_way;
                let arrive = t + fleet.one_way;
                for id in load {
                    let care = self.p.care_transit;
                    let c = self.cas.get_mut(&id).expect("c");
                    let wait = (t - c.queued_at).secs() / 60.0;
                    if forward {
                        c.times.ccp_wait_min += wait;
                    } else {
                        c.times.rear_wait_min += wait;
                    }
                    c.stage = transit;
                    c.care_rate = care;
                    self.schedule(arrive, if forward { Ev::ReachAid(id) } else { Ev::ReachSurgical(id) });
                }
                let back = fleet.free_at[v];
                self.schedule(back, Ev::Retry);
            }
        }
    }

    fn collect(&mut self, until: SimTime) -> Vec<CasualtyOutcome> {
        let ids: Vec<u64> = self.cas.keys().copied().collect();
        let mut out = Vec::new();
        for id in ids {
            let c = &self.cas[&id];
            if c.finished {
                continue;
            }
            let end = c.injured_at + self.p.window;
            let close = c.dead_at.is_some() || end <= until;
            if !close {
                continue;
            }
            if c.dead_at.is_none() {
                self.advance(id, end);
            }
            let m = self.p.mort[&self.cas[&id].triage];
            let c = self.cas.get_mut(&id).expect("c");
            c.finished = true;
            out.push(CasualtyOutcome {
                id,
                triage: c.triage,
                injured_at: c.injured_at,
                survived: c.dead_at.is_none(),
                died_at: c.dead_at,
                stage_reached: c.stage,
                effective_h: c.eff_h,
                p_death: m.cdf(c.eff_h),
                times: c.times.clone(),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn figs() -> Figures {
        crate::default_figures()
    }

    fn config(poi_min: f64, helicopters: bool) -> ChainConfig {
        let mut rear = Vec::new();
        if helicopters {
            rear.push(Fleet::new("helicopter", 1, 2, 20.0, &[Triage::T1]));
        }
        rear.push(Fleet::new("ground ambulance", 3, 4, 60.0, &[Triage::T1, Triage::T2]));
        ChainConfig {
            poi_delay: SimDuration::from_mins(poi_min),
            forward: vec![Fleet::new("forward ambulance", 3, 4, 20.0, &[Triage::T1, Triage::T2, Triage::T3, Triage::T4])],
            rear,
            aid_teams: 2,
            surgical_tables: 4,
            to_hospital: SimDuration::from_hours(6.0),
        }
    }

    fn run(poi_min: f64, helicopters: bool, n: u64, seed: u64) -> (f64, Vec<CasualtyOutcome>) {
        let f = figs();
        let mut chain = MedicalChain::new(config(poi_min, helicopters), &f, Seed(seed));
        for i in 0..n {
            let tri = match i % 10 {
                0..=2 => Triage::T1,
                3..=5 => Triage::T2,
                6..=8 => Triage::T3,
                _ => Triage::T4,
            };
            // A casualty every 20 minutes.
            chain.injure(i, tri, SimTime::from_secs_i(i as i64 * 1_200));
        }
        let out = chain.run(SimTime::from_secs_i(30 * 86_400));
        assert_eq!(out.len() as u64, n);
        let t1: Vec<&CasualtyOutcome> = out.iter().filter(|o| o.triage == Triage::T1).collect();
        let survival = t1.iter().filter(|o| o.survived).count() as f64 / t1.len() as f64;
        (survival, out)
    }

    #[test]
    fn mortality_curve_and_inverse() {
        let m = Mortality::from_figures(Triage::T1, &figs());
        assert_eq!(m.cdf(0.0), 0.0);
        assert!(m.cdf(1.0) < m.cdf(2.0));
        let te = m.inverse(0.3).unwrap();
        assert_relative_eq!(m.cdf(te), 0.3, epsilon = 1e-12);
        assert!(m.inverse(0.99).is_none());
    }

    /// Acceptance [R7]: T1 survival falls as extraction under drones
    /// stretches from 15 min to 60 and 120 min.
    #[test]
    fn survival_falls_with_evacuation_delay() {
        let (s15, out15) = run(15.0, true, 600, 11);
        let (s60, _) = run(60.0, false, 600, 11);
        let (s120, out120) = run(120.0, false, 600, 11);
        assert!(s15 > s60 + 0.05 && s60 > s120 + 0.05, "{s15} {s60} {s120}");
        // Every death happens after injury, at the moment its probability passed the draw.
        assert!(out120.iter().filter(|o| !o.survived).all(|o| o.p_death > 0.0 && o.died_at.is_some()));
        // T3 mostly returns to duty, and nobody dies before injury.
        assert!(out15.iter().filter(|o| o.triage == Triage::T3).all(|o| o.stage_reached == Stage::ReturnedToDuty || !o.survived));
        assert!(out15.iter().all(|o| o.died_at.is_none_or(|d| d >= o.injured_at)));
        let surgery = out15.iter().filter(|o| o.times.surgery_start.is_some()).count();
        assert!(surgery > 0);
        // Same seed, same result.
        let (again, _) = run(15.0, true, 600, 11);
        assert_eq!(again, s15);
    }

    #[test]
    fn overload_builds_queues() {
        let f = figs();
        let mut cfg = config(15.0, false);
        cfg.aid_teams = 1;
        let mut chain = MedicalChain::new(cfg, &f, Seed(1));
        for i in 0..20 {
            chain.injure(i, Triage::T2, SimTime::ZERO);
        }
        let out = chain.run(SimTime::from_secs_i(10 * 86_400));
        let max_wait = out.iter().map(|o| o.times.aid_wait_min).fold(0.0, f64::max);
        assert!(max_wait > 200.0, "{max_wait}");
    }
}
