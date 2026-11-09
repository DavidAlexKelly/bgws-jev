//! Casualties and the medical chain (§8.10 casualties, §8.11 medical; the
//! TypeScript `logistics.ts` medical half; research R7).
//!
//! Every tick the losses of every entity, from any cause (direct fire,
//! indirect fire, mines, drones), are seen by comparing it with what it had
//! ([`super::Tally`]). A lost vehicle's crew are partly casualties, a lost
//! soldier is one; a share are killed outright and the rest triaged
//! T1–T4. The wounded wait at their unit, which is asked how to get them
//! out (`casualtiesWaiting`: now, wait for dark, a robot); the choice sets
//! the extraction delay to the collection point, 60–120 min under drones and
//! fire **[ref R7]**. From there `bgws_ops::medical::MedicalChain` runs the
//! ambulances, aid post and surgical team with triage queues and
//! time-dependent mortality, and each casualty's outcome is logged
//! (`medical.died` / `medical.survived`) with its evacuation times.
//! Abstracted medical draws each casualty's evacuation time instead.

use super::{alive, name, raise, secs_to_dark, FiresWorld, FParams, Tally};
use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_catalogue::{Domain, Vulnerability};
use bgws_core::{EntityId, Seed, Side, SimDuration, SimTime};
use bgws_ops::medical::{ChainConfig, Fleet, MedicalChain, Mortality, Triage};
use bgws_scenario::scope::DomainMode;
use rand::Rng;
use std::collections::BTreeMap;

/// What an entity lost since the last tick.
#[derive(Clone, Debug)]
pub struct Loss {
    pub id: EntityId,
    pub side: Side,
    pub start: u32,
    pub lost: u32,
    pub destroyed: bool,
    pub mobility_new: u32,
    pub firepower_new: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evac {
    Now,
    Dark,
    Ugv,
}

#[derive(Clone, Debug)]
pub struct Waiting {
    pub cas: Vec<(u64, Triage, SimTime)>,
    pub since: SimTime,
    pub asked: bool,
    pub mode: Option<Evac>,
}

#[derive(Clone, Debug)]
pub struct CasInfo {
    pub side: Side,
    pub unit: EntityId,
    pub delay_min: f64,
}

#[derive(Clone, Debug, Default)]
pub struct MedStats {
    pub killed: u32,
    pub wounded: u32,
    pub died: u32,
    pub survived: u32,
}

#[derive(Default)]
pub struct MedicalState {
    pub chains: BTreeMap<(Side, u32), MedicalChain>,
    pub waiting: BTreeMap<EntityId, Waiting>,
    pub next: u64,
    pub info: BTreeMap<u64, CasInfo>,
    pub stats: BTreeMap<Side, MedStats>,
    pub ambulances: BTreeMap<Side, usize>,
    pub seed: Option<Seed>,
}

/// The evacuation chain's resources for a given extraction delay.
pub fn chain_config(p: &FParams, ambulances: usize, delay_min: f64) -> ChainConfig {
    let all = [Triage::T1, Triage::T2, Triage::T3, Triage::T4];
    ChainConfig {
        poi_delay: SimDuration::from_mins(delay_min),
        forward: vec![Fleet::new("ambulance", ambulances.max(1), p.ambulance_capacity, p.forward_min, &all)],
        rear: vec![Fleet::new("rear ambulance", ambulances.max(1), p.ambulance_capacity, p.rear_min, &[Triage::T1, Triage::T2])],
        aid_teams: p.aid_teams,
        surgical_tables: p.tables,
        to_hospital: SimDuration::from_mins(p.hospital_min),
    }
}

/// Triage of one wounded from a uniform draw.
pub fn triage(p: &FParams, u: f64) -> Triage {
    if u < p.t1 {
        Triage::T1
    } else if u < p.t1 + p.t2 {
        Triage::T2
    } else if u < p.t1 + p.t2 + p.t4 {
        Triage::T4
    } else {
        Triage::T3
    }
}

pub fn setup(s: &mut Sim, st: &mut FiresWorld) {
    st.medical.seed = Some(s.ctx.seed.derive(0x006d_6564_6963_616c));
    for side in Side::PLAYERS {
        let n: u32 = s
            .ctx
            .order
            .iter()
            .filter_map(|&e| s.world.get::<Ident>(e))
            .filter(|i| i.side == side && {
                let t = format!("{} {}", i.platform.id, i.platform.role).to_lowercase();
                t.contains("medevac") || t.contains("evacuat") || t.contains("ambulance")
            })
            .filter_map(|i| s.world.get::<Health>(s.ctx.entity(i.id)).map(|h| h.start))
            .sum();
        st.medical.ambulances.insert(side, if n > 0 { n as usize } else { st.p.ambulances });
    }
}

/// See every entity's losses since last tick; casualties and damage follow.
pub fn losses(s: &mut Sim, st: &mut FiresWorld) {
    let mut out = Vec::new();
    for &e in &s.ctx.order.clone() {
        let (Some(h), Some(t)) = (s.world.get::<Health>(e), s.world.get::<Tally>(e)) else { continue };
        if t.destroyed {
            continue;
        }
        let lost = t.fit.saturating_sub(h.fit);
        let mob = h.mobility_kills.saturating_sub(t.mobility);
        let fp = h.firepower_kills.saturating_sub(t.firepower);
        let changed = h.fit != t.fit || h.mobility_kills != t.mobility || h.firepower_kills != t.firepower || h.destroyed != t.destroyed;
        if !changed {
            continue;
        }
        let i = s.world.get::<Ident>(e).expect("i");
        let loss = Loss { id: i.id, side: i.side, start: h.start, lost, destroyed: h.destroyed, mobility_new: mob, firepower_new: fp };
        let nt = Tally { fit: h.fit, mobility: h.mobility_kills, firepower: h.firepower_kills, destroyed: h.destroyed };
        *s.world.get_mut::<Tally>(e).expect("tally") = nt;
        if lost > 0 || mob > 0 || fp > 0 || loss.destroyed {
            out.push(loss);
        }
    }
    if out.is_empty() {
        return;
    }
    if st.medical_mode != DomainMode::Off {
        for l in &out {
            if l.lost > 0 {
                casualties(s, st, l);
            }
        }
    }
    super::logistics::on_losses(s, st, &out);
}

fn under_fire(s: &Sim, st: &FiresWorld, e: Entity) -> bool {
    s.world.get::<Crew>(e).and_then(|c| c.last_incoming).map(|(t, _)| (s.ctx.now - t).secs() <= st.p.under_fire_s).unwrap_or(false)
}

fn casualties(s: &mut Sim, st: &mut FiresWorld, l: &Loss) {
    let now = s.ctx.now;
    let e = s.ctx.entity(l.id);
    let (domain, platform) = {
        let i = s.world.get::<Ident>(e).expect("i");
        (i.domain, i.platform.clone())
    };
    if domain != Domain::Land || super::is_node(&platform) {
        return;
    }
    let per = (platform.people() as f64 / platform.body.count.max(1) as f64).max(1.0);
    let share = if platform.body.vulnerability == Vulnerability::Personnel { 1.0 } else { st.p.crew_hurt };
    let x = l.lost as f64 * per * share;
    let mut rng = s.ctx.rng("med.casualties", l.id.0 as u64, 0);
    let n = x.floor() as u32 + (rng.gen::<f64>() < x.fract()) as u32;
    if n == 0 {
        return;
    }
    let mut killed = 0;
    let mut wounded: Vec<(u64, Triage)> = Vec::new();
    for _ in 0..n {
        if rng.gen::<f64>() < st.p.killed {
            killed += 1;
        } else {
            let id = st.medical.next;
            st.medical.next += 1;
            wounded.push((id, triage(&st.p, rng.gen())));
        }
    }
    let stats = st.medical.stats.entry(l.side).or_default();
    stats.killed += killed;
    stats.wounded += wounded.len() as u32;
    let mix: BTreeMap<String, usize> = wounded.iter().fold(BTreeMap::new(), |mut m, (_, t)| {
        *m.entry(format!("{t:?}")).or_insert(0) += 1;
        m
    });
    let text = format!("{}: {} casualties ({} killed, {} wounded)", name(s, l.id), n, killed, wounded.len());
    s.ctx.log.push(now, "medical.casualties").side(l.side).entity(l.id).text(text).data(serde_json::json!({ "killed": killed, "wounded": mix }));
    match st.medical_mode {
        DomainMode::Simulated => {
            let w = st.medical.waiting.entry(l.id).or_insert(Waiting { cas: vec![], since: now, asked: false, mode: None });
            if w.cas.is_empty() {
                w.since = now;
                w.asked = false;
                w.mode = None;
            }
            w.cas.extend(wounded.into_iter().map(|(id, t)| (id, t, now)));
        }
        DomainMode::Abstracted => {
            let threat = under_fire(s, st, e);
            for (id, t) in wounded {
                abstract_outcome(s, st, l.side, l.id, id, t, threat);
            }
        }
        DomainMode::Off => {}
    }
}

/// Abstracted medical: a drawn evacuation time and the mortality curve.
fn abstract_outcome(s: &mut Sim, st: &mut FiresWorld, side: Side, unit: EntityId, id: u64, t: Triage, threat: bool) {
    let mut rng = s.ctx.rng("med.abstract", id, 0);
    let median = if threat { st.p.evac_threat_min } else { st.p.evac_now_min };
    let z = {
        let (u1, u2): (f64, f64) = (rng.gen::<f64>().max(1e-12), rng.gen());
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    };
    let evac_min = median * (st.p.abstract_sigma * z).exp();
    let p = Mortality::from_figures(t, &s.ctx.figures).cdf(evac_min / 60.0);
    let died = rng.gen::<f64>() < p;
    outcome_log(s, st, side, unit, died, serde_json::json!({ "triage": format!("{t:?}"), "evacMin": evac_min, "pDeath": p, "abstracted": true }));
}

fn outcome_log(s: &mut Sim, st: &mut FiresWorld, side: Side, unit: EntityId, died: bool, data: serde_json::Value) {
    let stats = st.medical.stats.entry(side).or_default();
    if died {
        stats.died += 1;
    } else {
        stats.survived += 1;
    }
    let text = format!("a casualty from {} {}", name(s, unit), if died { "dies of wounds" } else { "survives" });
    s.ctx.log.push(s.ctx.now, if died { "medical.died" } else { "medical.survived" }).side(side).entity(unit).text(text).data(data);
}

/// Wounded waiting at their units: ask, then evacuate by the choice (or the rules).
pub fn base(s: &mut Sim, st: &mut FiresWorld) {
    if st.medical_mode != DomainMode::Simulated {
        return;
    }
    let now = s.ctx.now;
    let units: Vec<EntityId> = st.medical.waiting.keys().copied().collect();
    for u in units {
        let w = st.medical.waiting[&u].clone();
        if w.cas.is_empty() {
            st.medical.waiting.remove(&u);
            continue;
        }
        let e = s.ctx.entity(u);
        let threat = under_fire(s, st, e);
        let rule_mode = if threat && !super::is_night(s, now) { Evac::Ugv } else { Evac::Now };
        if !alive(s, e) {
            evacuate(s, st, u, Evac::Now, true);
            continue;
        }
        if !w.asked {
            st.medical.waiting.get_mut(&u).expect("w").asked = true;
            raise(s, u, "casualtiesWaiting", None, Some(if rule_mode == Evac::Ugv { "ugv" } else { "now" }));
            continue;
        }
        if let Some(m) = w.mode {
            evacuate(s, st, u, m, threat);
        } else if (now - w.since).secs() >= st.p.medical_decide_wait_s {
            evacuate(s, st, u, rule_mode, threat);
        }
    }
}

/// Into the chain: the extraction delay by the means chosen.
fn evacuate(s: &mut Sim, st: &mut FiresWorld, unit: EntityId, mode: Evac, threat: bool) {
    let Some(w) = st.medical.waiting.remove(&unit) else { return };
    let side = s.world.get::<Ident>(s.ctx.entity(unit)).expect("i").side;
    let delay = match mode {
        Evac::Now => {
            if threat {
                st.p.evac_threat_min
            } else {
                st.p.evac_now_min
            }
        }
        Evac::Dark => secs_to_dark(s) / 60.0 + st.p.evac_now_min,
        Evac::Ugv => st.p.evac_ugv_min,
    };
    // Chains by extraction delay, in 5-minute buckets.
    let bucket = ((delay / 5.0).round().max(1.0) * 5.0) as u32;
    let amb = st.medical.ambulances.get(&side).copied().unwrap_or(st.p.ambulances);
    let seed = st.medical.seed.unwrap_or(s.ctx.seed);
    let p = st.p.clone();
    let figs = s.ctx.figures.clone();
    let chain = st.medical.chains.entry((side, bucket)).or_insert_with(|| MedicalChain::new(chain_config(&p, amb, bucket as f64), &figs, seed));
    for (id, t, at) in &w.cas {
        chain.injure(*id, *t, *at);
        st.medical.info.insert(*id, CasInfo { side, unit, delay_min: bucket as f64 });
    }
    let text = format!("{}: {} wounded on their way ({:?}, {} min to the collection point)", name(s, unit), w.cas.len(), mode, bucket);
    s.ctx.log.push(s.ctx.now, "medical.evacuation").side(side).entity(unit).text(text).data(serde_json::json!({ "count": w.cas.len(), "mode": format!("{mode:?}"), "delayMin": bucket, "underFire": threat }));
}

/// Run the chains; log each closed outcome.
pub fn minute(s: &mut Sim, st: &mut FiresWorld) {
    if st.medical_mode != DomainMode::Simulated {
        return;
    }
    let now = s.ctx.now;
    let keys: Vec<(Side, u32)> = st.medical.chains.keys().copied().collect();
    for k in keys {
        let outs = st.medical.chains.get_mut(&k).expect("chain").run(now);
        for o in outs {
            let info = st.medical.info.remove(&o.id).unwrap_or(CasInfo { side: k.0, unit: EntityId(0), delay_min: k.1 as f64 });
            let mins = |t: Option<SimTime>| t.map(|t| (t - o.injured_at).secs() / 60.0);
            outcome_log(
                s,
                st,
                info.side,
                info.unit,
                !o.survived,
                serde_json::json!({
                    "triage": format!("{:?}", o.triage), "delayMin": info.delay_min, "ccpMin": mins(o.times.reached_ccp), "aidPostMin": mins(o.times.reached_aid_post),
                    "surgeryMin": mins(o.times.surgery_start), "effectiveH": o.effective_h, "pDeath": o.p_death, "stage": format!("{:?}", o.stage_reached),
                }),
            );
        }
    }
}

/// `medical.evacuateNow`, `medical.waitForDark`, `medical.requestUgv`.
pub fn apply(_s: &mut Sim, st: &mut FiresWorld, _e: Entity, id: EntityId, action: &str) -> bool {
    let mode = match action {
        "medical.evacuateNow" => Evac::Now,
        "medical.waitForDark" => Evac::Dark,
        "medical.requestUgv" => Evac::Ugv,
        _ => return false,
    };
    if let Some(w) = st.medical.waiting.get_mut(&id) {
        w.mode = Some(mode);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mortality rises with the evacuation delay (R7: 60–120 min under drones).
    #[test]
    fn longer_extraction_kills_more() {
        let f = crate::default_figures();
        let p = FParams::from_figures(&f);
        let deaths = |delay: f64| {
            let mut c = MedicalChain::new(chain_config(&p, 2, delay), &f, Seed(5));
            let mut died = 0;
            let mut n = 0;
            for i in 0..400u64 {
                let u = (i as f64 * 0.618_033_988_75).fract();
                c.injure(i, triage(&p, u), SimTime::from_secs(i as f64 * 1200.0));
            }
            for o in c.run(SimTime::from_secs(400.0 * 1200.0 + 80.0 * 3600.0)) {
                n += 1;
                died += (!o.survived) as u32;
            }
            assert_eq!(n, 400);
            died as f64 / n as f64
        };
        let (fast, slow) = (deaths(30.0), deaths(120.0));
        assert!(slow > fast + 0.02, "30 min: {fast:.3}, 120 min: {slow:.3}");
    }

    #[test]
    fn triage_shares_cover_the_unit_interval() {
        let p = FParams::from_figures(&crate::default_figures());
        let mut n = BTreeMap::new();
        for k in 0..1000 {
            *n.entry(triage(&p, k as f64 / 1000.0)).or_insert(0) += 1;
        }
        assert_eq!(n.values().sum::<i32>(), 1000);
        assert!(n[&Triage::T1] > 200 && n[&Triage::T3] > 300);
    }
}
