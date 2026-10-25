//! Cyber as processes, not packets (§8.12 Cyber; R5.5, P21): the scenario's
//! prepared accesses as `bgws_ops::cyber::AccessCampaign`s stepped daily
//! (access building, a daily hazard of discovery), the side's cyber cell
//! asked inside the authorised window (`cyberWindowOpen`: execute, hold,
//! abandon), and the effect as a time-limited outage on a named process of
//! the victim side that other domains read through [`cyber_multiplier`].

use crate::components::*;
use crate::queue::{Priority, Scheduled};
use crate::sim::Sim;
use bevy_ecs::prelude::*;
use bgws_core::{EntityId, Side, SimDuration, SimTime};
use bgws_ops::cyber::{AccessCampaign, CampaignState, CyberEffects, CyberEvent, CyberParams, EffectKind, TargetProcess};
use bgws_scenario::scope::{DomainMode, ScopeDomain};
use bgws_scenario::v4::CyberAccess;
use rand::Rng;
use std::collections::BTreeMap;

/// One prepared access and its campaign.
#[derive(Clone, Debug)]
pub struct Access {
    pub spec: CyberAccess,
    pub campaign: AccessCampaign,
    pub target: TargetProcess,
    pub executed: bool,
    pub abandoned: bool,
    /// Inside its window and waiting for a decision.
    pub open: bool,
}

/// The cyber domain's state.
#[derive(Resource, Clone, Debug)]
pub struct CyberState {
    pub mode: DomainMode,
    pub params: CyberParams,
    pub accesses: Vec<Access>,
    /// Effects in play, by the side that suffers them.
    pub effects: BTreeMap<Side, CyberEffects>,
    pub last_ask: BTreeMap<u32, SimTime>,
    /// The access each cell's question is about.
    pub asking: BTreeMap<EntityId, u32>,
}

/// The process an access's target names (`bearer`, `logistics`,
/// `airDefence`, `public`, or anything else as a named process).
pub fn target_process(target: &str, system: &str) -> TargetProcess {
    let sys = system.to_lowercase();
    match target {
        "airDefence" => TargetProcess::AdTrackSharing,
        "logistics" => TargetProcess::LogisticsPicture,
        "public" => TargetProcess::AirRaidWarning,
        "keys" | "crypto" => TargetProcess::KeyCompromise,
        "bearer" if sys.contains("satcom") || sys.contains("satellite") => TargetProcess::SatcomTerminals,
        "bearer" if sys.contains("mobile") || sys.contains("cell") || sys.contains("4g") || sys.contains("lte") => TargetProcess::MobileNetwork,
        "bearer" => TargetProcess::Custom(system.to_string()),
        other => TargetProcess::Custom(other.to_string()),
    }
}

/// The multiplier (0–1) on a side's process now: 0 under an outage, the
/// degrade fraction under a degrading effect, 1 otherwise. `side` is the
/// side whose process it is.
pub fn cyber_multiplier(s: &Sim, side: Side, target: &TargetProcess) -> f64 {
    s.world.get_resource::<CyberState>().and_then(|c| c.effects.get(&side)).map(|e| e.multiplier(target, s.ctx.now)).unwrap_or(1.0)
}

/// [`cyber_multiplier`] by the scenario's target words (`airDefence`, `logistics`…).
pub fn cyber_multiplier_named(s: &Sim, side: Side, target: &str) -> f64 {
    cyber_multiplier(s, side, &target_process(target, ""))
}

/// Is any effect on this side's process active now?
pub fn cyber_active(s: &Sim, side: Side, target: &TargetProcess) -> bool {
    s.world.get_resource::<CyberState>().and_then(|c| c.effects.get(&side)).map(|e| e.is_active(target, s.ctx.now)).unwrap_or(false)
}

pub(crate) fn setup(s: &mut Sim) {
    let mode = s.ctx.scenario.scope.mode(ScopeDomain::Cyber);
    if mode == DomainMode::Off || s.ctx.scenario.world.cyber.is_empty() {
        return;
    }
    let params = CyberParams::from_figures(&s.ctx.figures);
    let mut accesses = Vec::new();
    for (i, a) in s.ctx.scenario.world.cyber.iter().enumerate() {
        let target = target_process(&a.target, &a.system);
        let mut rng = s.ctx.rng("cyber.campaign", i as u64, 0);
        let mut c = AccessCampaign::new(i as u64, target.clone(), EffectKind::Outage, SimTime::ZERO, &params, &mut rng);
        // The scenario fixes when the access is ready (seconds from the start).
        c.required_days = (a.readiness / 86_400.0).max(0.0);
        if a.readiness <= 0.0 {
            c.state = CampaignState::Ready { since: SimTime::ZERO };
        }
        let at = a.window.as_ref().map(|w| w.from).unwrap_or(0.0).max(a.readiness).max(0.0);
        s.ctx.queue.schedule(SimTime::from_secs(at), Priority::Process, Scheduled::CyberWindow { campaign: i as u32, open: true });
        if a.readiness > 0.0 {
            // Step the campaign the moment it should be ready.
            s.ctx.queue.schedule(SimTime::from_secs(a.readiness), Priority::Process, Scheduled::Timer { system: "cyber.step".into(), key: u64::MAX });
        }
        accesses.push(Access { spec: a.clone(), campaign: c, target, executed: false, abandoned: false, open: false });
    }
    let step = s.ctx.figures.get("sim.cyber.step_s");
    s.ctx.queue.schedule(SimTime::from_secs(step), Priority::Process, Scheduled::Timer { system: "cyber.step".into(), key: 1 });
    s.world.insert_resource(CyberState { mode, params, accesses, effects: BTreeMap::new(), last_ask: BTreeMap::new(), asking: BTreeMap::new() });
}

/// Step every campaign: access building, discovery, effects ending (daily).
pub(crate) fn step(s: &mut Sim, key: u64) {
    let Some(mut st) = s.world.remove_resource::<CyberState>() else { return };
    let now = s.ctx.now;
    let p = st.params.clone();
    for (i, a) in st.accesses.iter_mut().enumerate() {
        let mut rng = s.ctx.rng("cyber.step", i as u64, key);
        for ev in a.campaign.step(now, &p, &mut rng) {
            let (kind, text) = match ev {
                CyberEvent::AccessReady { .. } => ("cyber.ready", format!("cyber access {} into {}'s {} is ready", a.spec.id, a.spec.side.enemy(), a.spec.system)),
                CyberEvent::Discovered { p_step, .. } => ("cyber.discovered", format!("{} finds the intrusion behind access {} (p {:.3})", a.spec.side.enemy(), a.spec.id, p_step)),
                CyberEvent::EffectEnded { .. } => continue,
            };
            s.ctx.log.push(now, kind).side(a.spec.side).text(text).data(serde_json::json!({ "access": a.spec.id }));
        }
    }
    if key != u64::MAX {
        let step = s.ctx.figures.get("sim.cyber.step_s");
        s.ctx.queue.schedule(now + SimDuration::from_secs(step), Priority::Process, Scheduled::Timer { system: "cyber.step".into(), key: key + 1 });
    }
    s.world.insert_resource(st);
}

/// Who decides a side's cyber operations: its cyber cell, else an HQ.
fn cell_of(s: &Sim, side: Side) -> Option<EntityId> {
    let mut hq = None;
    for &e in &s.ctx.order {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        if i.side != side || !crate::systems::alive(s, e) {
            continue;
        }
        if i.platform.id.to_lowercase().contains("cyber") {
            return Some(i.id);
        }
        if i.hq && hq.is_none() {
            hq = Some(i.id);
        }
    }
    hq
}

/// A window opens (ask the cell) or an effect ends.
pub(crate) fn window(s: &mut Sim, campaign: u32, open: bool) {
    let now = s.ctx.now;
    let Some(mut st) = s.world.remove_resource::<CyberState>() else { return };
    let Some(a) = st.accesses.get_mut(campaign as usize) else {
        s.world.insert_resource(st);
        return;
    };
    if open {
        if !a.executed && !a.abandoned {
            a.open = true;
            let text = format!("the window for cyber access {} opens", a.spec.id);
            s.ctx.log.push(now, "cyber.windowOpen").side(a.spec.side).text(text).data(serde_json::json!({ "access": a.spec.id }));
        }
        s.world.insert_resource(st);
        ask(s, campaign);
    } else {
        let mut rng = s.ctx.rng("cyber.end", campaign as u64, 0);
        let p = st.params.clone();
        let a = &mut st.accesses[campaign as usize];
        a.campaign.step(now, &p, &mut rng);
        let victim = a.spec.side.enemy();
        let text = format!("the effect of cyber access {} on {victim}'s {} ends", a.spec.id, a.spec.system);
        s.ctx.log.push(now, "cyber.effectEnds").side(victim).text(text).data(serde_json::json!({ "access": a.spec.id }));
        s.world.insert_resource(st);
    }
}

/// Ask the cell about an open access (cyberWindowOpen) with the TS rule:
/// abandon a risky, unlikely access; execute when the window is closing or
/// the risk is low; else hold.
fn ask(s: &mut Sim, campaign: u32) {
    let now = s.ctx.now;
    let gap = s.ctx.figures.get("sim.cyber.ask_s");
    let Some(st) = s.world.get_resource::<CyberState>() else { return };
    let Some(a) = st.accesses.get(campaign as usize) else { return };
    if !a.open || a.executed || a.abandoned || st.last_ask.get(&campaign).map(|t| (now - *t).secs() < gap).unwrap_or(false) {
        return;
    }
    let left = a.spec.window.as_ref().map(|w| w.until - now.secs()).unwrap_or(f64::INFINITY);
    let (risk, pe, side) = (a.spec.detection_risk, a.spec.p_effect, a.spec.side);
    let Some(cell) = cell_of(s, side) else { return };
    let (opt, action) = if risk > 0.6 && pe < 0.3 {
        ("abandon", "cyber.abandon")
    } else if left < gap * 2.0 || risk < 0.5 {
        ("execute", "cyber.executeNow")
    } else {
        ("hold", "cyber.holdForMoment")
    };
    if let Some(mut st) = s.world.get_resource_mut::<CyberState>() {
        st.last_ask.insert(campaign, now);
        st.asking.insert(cell, campaign);
    }
    super::ask_or_do(s, cell, "cyberWindowOpen", opt, action);
}

/// Each minute: re-ask open accesses; close lapsed windows.
pub(crate) fn minute(s: &mut Sim) {
    let now = s.ctx.now.secs();
    let Some(st) = s.world.get_resource::<CyberState>() else { return };
    let mut lapsed = Vec::new();
    let mut open = Vec::new();
    for (i, a) in st.accesses.iter().enumerate() {
        if !a.open || a.executed || a.abandoned {
            continue;
        }
        if a.spec.window.as_ref().map(|w| now > w.until).unwrap_or(false) {
            lapsed.push(i);
        } else {
            open.push(i as u32);
        }
    }
    for i in lapsed {
        let text = {
            let mut st = s.world.get_resource_mut::<CyberState>().expect("state");
            st.accesses[i].open = false;
            (format!("the window for cyber access {} closes unused", st.accesses[i].spec.id), st.accesses[i].spec.side)
        };
        s.ctx.log.push(s.ctx.now, "cyber.windowLapsed").side(text.1).text(text.0);
    }
    for c in open {
        ask(s, c);
    }
}

/// Execute an access: the effect works with its probability, may be
/// detected (shortening it as the victim remediates), and burns the access.
fn execute(s: &mut Sim, idx: u32) {
    let now = s.ctx.now;
    let Some(mut st) = s.world.remove_resource::<CyberState>() else { return };
    let remediated = s.ctx.figures.get("sim.cyber.remediated_share");
    let mut rng = s.ctx.rng("cyber.execute", idx as u64, 0);
    let p = st.params.clone();
    let a = &mut st.accesses[idx as usize];
    a.campaign.step(now, &p, &mut rng);
    let side = a.spec.side;
    let victim = side.enemy();
    if !matches!(a.campaign.state, CampaignState::Ready { .. }) {
        let text = format!("cyber access {} cannot be executed ({:?})", a.spec.id, a.campaign.state);
        a.open = false;
        s.ctx.log.push(now, "cyber.notReady").side(side).text(text);
        s.world.insert_resource(st);
        return;
    }
    let worked = rng.gen::<f64>() < a.spec.p_effect;
    let detected = rng.gen::<f64>() < a.spec.detection_risk;
    let length = a.spec.duration_s * if detected { remediated } else { 1.0 };
    let mut ep = p.clone();
    ep.effect_days = length / 86_400.0;
    a.executed = true;
    a.open = false;
    let effect = a.campaign.trigger(now, &ep).ok();
    let (id, system, target) = (a.spec.id.clone(), a.spec.system.clone(), a.target.clone());
    if worked {
        if let Some(e) = effect {
            let end = e.end;
            st.effects.entry(victim).or_default().add(e);
            s.ctx.queue.schedule(end, Priority::Process, Scheduled::CyberWindow { campaign: idx, open: false });
        }
        let text = format!("cyber access {id} takes down {victim}'s {system} for {:.1} h", length / 3600.0);
        s.ctx.log.push(now, "cyber.effect").side(side).text(text).data(serde_json::json!({ "access": id, "target": target, "victim": victim, "durationS": length, "detected": detected }));
    } else {
        let text = format!("cyber access {id} fails to take effect");
        s.ctx.log.push(now, "cyber.failed").side(side).text(text).data(serde_json::json!({ "access": id }));
    }
    if detected {
        let text = format!("{victim} finds an intrusion in its {system} and starts to remediate");
        s.ctx.log.push(now, "cyber.detected").side(victim).text(text).data(serde_json::json!({ "access": id }));
    }
    s.world.insert_resource(st);
}

/// The `cyber.*` actions.
pub(crate) fn apply(s: &mut Sim, _e: Entity, id: EntityId, action: &str) -> bool {
    let Some(idx) = s.world.get_resource::<CyberState>().and_then(|st| st.asking.get(&id).copied()) else { return action.starts_with("cyber.") };
    match action {
        "cyber.executeNow" => execute(s, idx),
        "cyber.holdForMoment" => {}
        "cyber.abandon" => {
            let (text, side) = {
                let mut st = s.world.get_resource_mut::<CyberState>().expect("state");
                st.accesses[idx as usize].abandoned = true;
                st.accesses[idx as usize].open = false;
                (format!("cyber access {} is abandoned", st.accesses[idx as usize].spec.id), st.accesses[idx as usize].spec.side)
            };
            s.ctx.log.push(s.ctx.now, "cyber.abandoned").side(side).text(text);
        }
        _ => return false,
    }
    true
}
