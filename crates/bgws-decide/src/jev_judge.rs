//! Judgments from Jev, decisions in code (the `judge` request style).
//!
//! TypeSafe's guidance: a question asking a System One model what an agent
//! should do next is at the wrong layer; break the decision into the
//! judgments it depends on and write the decision itself in code, as a
//! truth table. Exact facts (range, reach, the chance a volley hits, orders)
//! are read from the unit's state here; Jev is asked only for what the state
//! cannot settle by itself, as yes/no questions about the state.
//!
//! The tables were fitted to play-outs (each option of a moment forced and
//! played on, `bgws lab playout`) on the engine where guided missiles hit:
//! 179 moments at D1, D2, D6 and D8, on which they cost 0.04-0.10 a
//! decision against the best option, as the doctrine default does or a
//! little less; Jev choosing the whole action cost 0.17-0.90. What they
//! found is mostly the doctrine: carry on at a new contact, answer a shooter
//! you can see, keep firing after a volley. They differ where the facts say
//! so: an identified vehicle close by that can hit the unit is engaged; a
//! unit that cannot hurt its shooter and sits in the open pulls back; a
//! halted unit whose fire cannot land moves to a better position; a unit
//! stopped short of where its orders lead resumes them.
//!
//! Every other decision point takes the doctrine default, without asking.

use bgws_core::Question;
use serde_json::{json, Map, Value};

/// A contact this close, if it is a vehicle that can reach the unit, is engaged, m.
pub const CLOSE_M: f64 = 600.0;
/// Below this chance a volley hits, the unit's fire is not landing, %.
pub const POOR_SHOT_PCT: f64 = 10.0;
/// Less cover than this is the open.
pub const OPEN_COVER: f64 = 0.3;
/// A judgment above this reads as yes, below `1 - this` as no; between, unsettled.
pub const YES: f64 = 0.7;

/// The facts a table reads, from the unit's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub posture: String,
    pub move_left: bool,
    pub cover: f64,
    /// The enemy the decision is about (else the nearest).
    pub range_m: Option<f64>,
    pub identified: bool,
    /// What it is: the type when identified, the kind when recognised.
    pub what: String,
    /// What friends have reported it as, when the unit has not identified it.
    pub reported_as: String,
    pub in_reach: bool,
    pub can_reach_us: Option<bool>,
    /// The chance a volley hits it: identified, against what it is; else as a vehicle and as a team in cover.
    pub hit_pct: Option<f64>,
    pub hit_if_vehicle: Option<f64>,
    pub hit_if_team: Option<f64>,
}

impl Facts {
    pub fn from_state(st: &Value) -> Facts {
        let s = |p: &str| st.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_f64);
        let enemies = st.get("enemiesSeen").and_then(Value::as_array).cloned().unwrap_or_default();
        let e = enemies.iter().find(|e| e.get("thisDecisionIsAboutIt").and_then(Value::as_bool) == Some(true)).or(enemies.first()).cloned().unwrap_or(Value::Null);
        Facts {
            posture: s("/unit/posture"),
            move_left: n(st, "/orders/headingFor/distanceM").is_some_and(|d| d > 150.0),
            cover: n(st, "/ground/cover").unwrap_or(0.0),
            range_m: n(&e, "/rangeM"),
            identified: e.get("identified").and_then(Value::as_bool).unwrap_or(false),
            what: e.get("what").and_then(Value::as_str).unwrap_or("").to_string(),
            reported_as: e.pointer("/reportedAs/what").and_then(Value::as_str).unwrap_or("").to_string(),
            in_reach: e.get("inOurReach").and_then(Value::as_bool).unwrap_or(false),
            can_reach_us: e.get("canReachUs").and_then(Value::as_bool),
            hit_pct: n(&e, "/ifWeFire/volleyHitPct"),
            hit_if_vehicle: n(&e, "/ifWeFire/volleyHitPctIfVehicle"),
            hit_if_team: n(&e, "/ifWeFire/volleyHitPctIfTeamInCover"),
        }
    }

    /// Whether the contact is a vehicle, when the unit knows: identified
    /// (its type), recognised (its kind: an armoured or soft-skinned vehicle,
    /// a team on foot), or reported by friends.
    fn known_vehicle(&self) -> Option<bool> {
        const ON_FOOT: [&str; 7] = ["rifle", "infantry", "anti-tank", "team", "engineer", "sniper", "on foot"];
        let foot = |w: &str| ON_FOOT.iter().any(|f| w.to_lowercase().contains(f));
        if self.identified || self.what.ends_with("vehicle") || self.what == "a team on foot" {
            return Some(!foot(&self.what));
        }
        (!self.reported_as.is_empty()).then(|| !foot(&self.reported_as))
    }

    /// The chance a volley lands, on what it is (or on the likelier reading).
    fn shot(&self, vehicle: Option<bool>) -> Option<f64> {
        match (self.identified, vehicle) {
            (true, _) => self.hit_pct,
            (false, Some(true)) => self.hit_if_vehicle,
            (false, Some(false)) => self.hit_if_team,
            // Unsettled: the worse of the two.
            (false, None) => match (self.hit_if_vehicle, self.hit_if_team) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }

    fn halted(&self) -> bool {
        matches!(self.posture.as_str(), "halted" | "in position")
    }
}

/// A yes/no question for Jev about the unit's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgment {
    pub key: &'static str,
    pub instructions: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
}

/// Is the (unidentified) contact a vehicle?
pub const VEHICLE: Judgment = Judgment {
    key: "vehicle",
    instructions: "Is the enemy in `enemiesSeen` that this decision is about a vehicle?",
    yes: "It is a vehicle, such as a tank or an armoured carrier.",
    no: "It is a team on foot, such as infantry or an anti-tank team.",
};

/// The judgments a decision point needs from Jev, given the facts: only
/// what the facts leave open and the table turns on.
pub fn judgments(trigger: &str, f: &Facts) -> Vec<Judgment> {
    match trigger {
        // Whether a close, unidentified contact is a vehicle decides whether to engage it.
        "D1" if f.known_vehicle().is_none() && f.in_reach && f.range_m.is_some_and(|r| r <= CLOSE_M) => vec![VEHICLE],
        _ => Vec::new(),
    }
}

/// A judgment's answer read as yes, no or unsettled.
pub fn read(p: Option<f64>) -> Option<bool> {
    match p {
        Some(p) if p >= YES => Some(true),
        Some(p) if p <= 1.0 - YES => Some(false),
        _ => None,
    }
}

/// The table's choice: option ids in order of preference (the first one
/// offered is taken), and the row that chose them. `None`: this point has no
/// table, and the doctrine default stands.
pub fn decide(trigger: &str, f: &Facts, judged: &dyn Fn(&str) -> Option<bool>) -> Option<(Vec<&'static str>, &'static str)> {
    let vehicle = f.known_vehicle().or_else(|| judged(VEHICLE.key));
    Some(match trigger {
        "D1" => {
            let close = f.range_m.is_some_and(|r| r <= CLOSE_M);
            let threat = f.can_reach_us.unwrap_or(!f.identified);
            if f.in_reach && close && vehicle == Some(true) && threat {
                (vec!["engage", "keep"], "a vehicle close by that can hit it: engage")
            } else {
                (vec!["keep"], "otherwise carry on")
            }
        }
        "D2" => {
            let poor = !f.in_reach || f.shot(vehicle).is_some_and(|s| s < POOR_SHOT_PCT);
            if poor && f.cover < OPEN_COVER {
                (vec!["pullBack", "hullDown", "returnFire"], "cannot hurt the shooter, in the open: pull back")
            } else {
                (vec!["returnFire"], "answer the shooter")
            }
        }
        "D6" => (vec!["keep"], "keep firing"),
        "D8" => {
            if f.halted() && f.shot(vehicle).is_some_and(|s| s < POOR_SHOT_PCT) {
                (vec!["better", "keep"], "halted and its fire cannot land: a better position")
            } else {
                (vec!["keep"], "keep at it")
            }
        }
        "D10" if f.move_left => (vec!["resume", "nextPhase"], "its orders still lead on: resume"),
        _ => return None,
    })
}

/// One unit's judgments as a request: its state cut to what the point needs,
/// one Noul question per judgment.
pub fn judge_request(model: &str, q: &Question, js: &[Judgment]) -> Value {
    let mut state = q.state.as_ref().map(|st| crate::jev_ask::trim_state(&q.trigger, st)).unwrap_or_else(|| json!({ "situation": q.situation }));
    if let Some(m) = state.as_object_mut() {
        m.insert("knowledge".into(), Value::String(crate::jev_ask::KNOWLEDGE.into()));
    }
    let mut qs = Map::new();
    for j in js {
        qs.insert(j.key.into(), json!({ "type": "noul", "instructions": j.instructions, "criteria": { "true": j.yes, "false": j.no } }));
    }
    json!({ "model": model, "state": state, "questions": qs })
}

/// Read Noul answers (`{"type": "noul", "noul": p}`) from a reply.
pub fn parse_nouls(body: &Value) -> std::collections::BTreeMap<String, f64> {
    body.get("answers")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| v.get("noul").and_then(Value::as_f64).filter(|p| p.is_finite()).map(|p| (k.clone(), p))).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(enemy: Value, extra: Value) -> Value {
        let mut s = json!({ "unit": { "posture": "moving" }, "orders": { "headingFor": { "distanceM": 900.0 } }, "ground": { "cover": 0.1 }, "enemiesSeen": [enemy] });
        if let (Some(a), Some(b)) = (s.as_object_mut(), extra.as_object()) {
            for (k, v) in b {
                a.insert(k.clone(), v.clone());
            }
        }
        s
    }
    fn none(_: &str) -> Option<bool> {
        None
    }

    #[test]
    fn a_new_contact_is_carried_past_unless_a_close_vehicle_can_hit_it() {
        let far = Facts::from_state(&st(json!({"rangeM": 2300.0, "identified": false, "inOurReach": true, "thisDecisionIsAboutIt": true, "ifWeFire": {"volleyHitPctIfVehicle": 85.0, "volleyHitPctIfTeamInCover": 4.0}}), json!({})));
        assert!(judgments("D1", &far).is_empty(), "far: nothing to ask");
        assert_eq!(decide("D1", &far, &none).unwrap().0, ["keep"]);
        let tank = Facts::from_state(&st(json!({"rangeM": 400.0, "identified": true, "what": "Challenger 2", "inOurReach": true, "canReachUs": true, "thisDecisionIsAboutIt": true}), json!({})));
        assert_eq!(decide("D1", &tank, &none).unwrap().0, ["engage", "keep"]);
        let squad = Facts::from_state(&st(json!({"rangeM": 400.0, "identified": true, "what": "Rifle section", "inOurReach": true, "canReachUs": true, "thisDecisionIsAboutIt": true}), json!({})));
        assert_eq!(decide("D1", &squad, &none).unwrap().0, ["keep"], "a team on foot is not engaged by this row");
        // Close and unidentified: Jev is asked whether it is a vehicle.
        let unknown = Facts::from_state(&st(json!({"rangeM": 450.0, "identified": false, "inOurReach": true, "thisDecisionIsAboutIt": true}), json!({})));
        assert_eq!(judgments("D1", &unknown), vec![VEHICLE]);
        assert_eq!(decide("D1", &unknown, &|_| Some(true)).unwrap().0, ["engage", "keep"]);
        assert_eq!(decide("D1", &unknown, &|_| None).unwrap().0, ["keep"], "unsettled: carry on");
    }

    #[test]
    fn the_other_tables() {
        let blind = Facts::from_state(&st(json!({"rangeM": 1400.0, "identified": true, "what": "Anti-tank team", "inOurReach": true, "ifWeFire": {"volleyHitPct": 4.0}}), json!({"ground": {"cover": 0.0}})));
        assert_eq!(decide("D2", &blind, &none).unwrap().0[0], "pullBack");
        let fair = Facts::from_state(&st(json!({"rangeM": 1400.0, "identified": true, "what": "T-72", "inOurReach": true, "ifWeFire": {"volleyHitPct": 80.0}}), json!({})));
        assert_eq!(decide("D2", &fair, &none).unwrap().0, ["returnFire"]);
        assert_eq!(decide("D6", &fair, &none).unwrap().0, ["keep"]);
        let halted = Facts::from_state(&st(json!({"rangeM": 1400.0, "identified": true, "what": "Rifle section", "inOurReach": true, "ifWeFire": {"volleyHitPct": 6.0}}), json!({"unit": {"posture": "halted"}})));
        assert_eq!(decide("D8", &halted, &none).unwrap().0[0], "better");
        assert_eq!(decide("D10", &fair, &none).unwrap().0[0], "resume");
        let done = Facts::from_state(&json!({"orders": {}}));
        assert!(decide("D10", &done, &none).is_none(), "orders done: the doctrine's answer");
        assert!(decide("D3", &fair, &none).is_none());
    }

    #[test]
    fn judgments_read_with_an_unsettled_band() {
        assert_eq!(read(Some(0.9)), Some(true));
        assert_eq!(read(Some(0.1)), Some(false));
        assert_eq!(read(Some(0.5)), None);
        assert_eq!(read(None), None);
    }
}
