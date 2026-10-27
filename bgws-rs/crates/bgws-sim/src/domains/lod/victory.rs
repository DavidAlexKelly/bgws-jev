//! The world's victory conditions (scenario `victory`, the TS
//! `realtime/world/victory.ts`): air superiority over an area, a target set
//! destroyed, sea control, ships through, and several aims together. Each is
//! judged from the truth (the umpire's view) every 10 s; one that must be held
//! (air superiority, sea control) starts a clock when first met and resets if
//! lost. The first side to meet one of its conditions wins. Land breakpoints
//! and the time limit stay with `systems::victory`.

use super::LodState;
use crate::components::*;
use crate::sim::Sim;
use crate::systems::alive;
use bgws_catalogue::Domain;
use bgws_core::{Geodetic, Side, SimTime};
use bgws_scenario::v4::VictoryCondition;
use std::collections::BTreeMap;

/// Hold clocks, through-memos and the verdict.
#[derive(Clone, Debug, Default)]
pub struct VictoryState {
    /// Condition index → when it was first met (and held since).
    pub since: BTreeMap<usize, SimTime>,
    /// Through conditions: (key, unit) → when it reached the area.
    pub through: BTreeMap<String, SimTime>,
    pub verdict: Option<(Option<Side>, String)>,
}

/// How far a condition has got (rule 10).
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    pub met: bool,
    pub text: String,
}

fn label(v: &VictoryCondition) -> String {
    use VictoryCondition as V;
    match v {
        V::AirSuperiority { label, .. } => label.clone().unwrap_or_else(|| "air superiority".into()),
        V::TargetSet { label, .. } => label.clone().unwrap_or_else(|| "target set".into()),
        V::SeaControl { label, .. } => label.clone().unwrap_or_else(|| "sea control".into()),
        V::Through { label, .. } => label.clone().unwrap_or_else(|| "through".into()),
        V::All { label, .. } => label.clone().unwrap_or_else(|| "all aims".into()),
        V::Breakpoint { .. } => "breakpoint".into(),
    }
}

fn side_of(v: &VictoryCondition) -> Option<Side> {
    use VictoryCondition as V;
    match v {
        V::AirSuperiority { side, .. } | V::TargetSet { side, .. } | V::SeaControl { side, .. } | V::Through { side, .. } | V::All { side, .. } => Some(*side),
        V::Breakpoint { .. } => None,
    }
}

/// Seconds a condition must be held (as the TS engine: only the area ones).
pub fn hold_s(v: &VictoryCondition) -> f64 {
    match v {
        VictoryCondition::AirSuperiority { hold_s, .. } | VictoryCondition::SeaControl { hold_s, .. } => *hold_s,
        _ => 0.0,
    }
}

/// Living entities of `side` within `radius` of `at`, filtered.
fn count_in(s: &Sim, side: Side, at: Geodetic, radius: f64, keep: impl Fn(&Ident, &Kin, bevy_ecs::prelude::Entity) -> bool) -> usize {
    let c = s.ctx.frame.geodetic_to_enu(at);
    s.ctx
        .order
        .iter()
        .filter(|&&e| alive(s, e))
        .filter_map(|&e| Some((e, s.world.get::<Ident>(e)?, s.world.get::<Kin>(e)?)))
        .filter(|(e, i, k)| i.side == side && !i.dummy && k.pos.sub(c).horizontal() <= radius && keep(i, k, *e))
        .count()
}

fn air_defence(i: &Ident) -> bool {
    let t = i.platform.id.to_lowercase();
    t.contains("sam") || t.contains("shorad") || t.contains("gbad")
}

/// Judge one condition now. `through` keeps who has got through (it latches).
pub fn judge(s: &Sim, v: &VictoryCondition, through: &mut BTreeMap<String, SimTime>) -> Judgement {
    use VictoryCondition as V;
    match v {
        V::AirSuperiority { side, at, radius_m, .. } => {
            let enemy = side.enemy();
            let air = count_in(s, enemy, *at, *radius_m, |i, k, _| i.domain == Domain::Air && i.platform.body.crew > 0 && k.agl > 10.0);
            let ad = count_in(s, enemy, *at, *radius_m, |i, _, e| i.domain == Domain::Land && air_defence(i) && !matches!(s.world.get::<Act>(e).map(|a| &a.activity), Some(Activity::Quiet)));
            Judgement { met: air == 0 && ad == 0, text: format!("{air} enemy aircraft and {ad} radiating air-defence sites over the area") }
        }
        V::TargetSet { targets, share, .. } => {
            let gone = targets.iter().filter(|t| s.ctx.by_name.get(*t).map(|&id| !alive(s, s.ctx.entity(id))).unwrap_or(false)).count();
            let need = (targets.len() as f64 * share).ceil() as usize;
            Judgement { met: !targets.is_empty() && gone >= need, text: format!("{gone} of {} targets destroyed", targets.len()) }
        }
        V::SeaControl { side, at, radius_m, .. } => {
            let ships = count_in(s, side.enemy(), *at, *radius_m, |i, _, _| matches!(i.domain, Domain::Surface | Domain::Subsurface));
            let own = count_in(s, *side, *at, *radius_m, |i, _, _| i.domain == Domain::Surface);
            Judgement { met: ships == 0 && own > 0, text: format!("{own} own and {ships} enemy vessels in the area") }
        }
        V::Through { side, ids, at, radius_m, count, label } => {
            let key = format!("through:{}:{}", side.as_str(), label.clone().unwrap_or_else(|| ids.join(",")));
            let c = s.ctx.frame.geodetic_to_enu(*at);
            for id in ids {
                let Some(&eid) = s.ctx.by_name.get(id) else { continue };
                let e = s.ctx.entity(eid);
                if !alive(s, e) {
                    continue;
                }
                let inside = s.world.get::<Kin>(e).map(|k| k.pos.sub(c).horizontal() <= *radius_m).unwrap_or(false);
                if inside {
                    through.entry(format!("{key}:{id}")).or_insert(s.ctx.now);
                }
            }
            let n = ids.iter().filter(|id| through.contains_key(&format!("{key}:{id}"))).count();
            Judgement { met: n as u32 >= *count, text: format!("{n} of {count} through") }
        }
        V::All { of, .. } => {
            let parts: Vec<Judgement> = of.iter().map(|p| judge(s, p, through)).collect();
            Judgement { met: !parts.is_empty() && parts.iter().all(|p| p.met), text: parts.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("; ") }
        }
        V::Breakpoint { .. } => Judgement { met: false, text: "judged by the land breakpoint".into() },
    }
}

/// Every 10 s: judge each side's conditions, run the hold clocks, log, and
/// set the verdict when one is met and held.
pub(crate) fn evaluate(s: &mut Sim, st: &mut LodState) {
    if st.victory.verdict.is_some() || s.ctx.over.is_some() {
        return;
    }
    let now = s.ctx.now;
    let conds = s.ctx.scenario.victory.clone();
    for side in Side::PLAYERS {
        for (i, v) in conds.iter().enumerate() {
            if side_of(v) != Some(side) || st.victory.verdict.is_some() {
                continue;
            }
            let j = judge(s, v, &mut st.victory.through);
            let name = label(v);
            if !j.met {
                if st.victory.since.remove(&i).is_some() {
                    s.ctx.log.push(now, "victory.lost").side(side).text(format!("{name}: no longer met ({})", j.text)).data(serde_json::json!({ "condition": i }));
                }
                continue;
            }
            let hold = hold_s(v);
            let since = *st.victory.since.entry(i).or_insert_with(|| {
                if hold > 0.0 {
                    s.ctx.log.push(now, "victory.met").side(side).text(format!("{name}: met ({}); held for {:.0} min it wins", j.text, hold / 60.0)).data(serde_json::json!({ "condition": i, "holdS": hold }));
                }
                now
            });
            if (now - since).secs() >= hold {
                let reason = format!("{name} ({})", j.text);
                s.ctx.log.push(now, "victory.won").side(side).text(format!("{side} wins: {reason}")).data(serde_json::json!({ "condition": i, "heldS": (now - since).secs() }));
                st.victory.verdict = Some((Some(side), reason));
            }
        }
    }
}
