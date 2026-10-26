//! HQ plan requests (RUST_ENGINE.md §10.3): what an HQ question carries, the
//! plan schemas, the prompts (a port of `world/command/claudePlanner.ts` and
//! the shape of `engine/orders.ts`'s `ordersPrompt`), the joint-guidance
//! checks (`parseJointGuidance`), and the heuristic planner
//! (`world/command/heuristicPlanner.ts`), which is the fallback for every
//! planner and the default for batch runs.
//!
//! An HQ question (`QuestionKind::Hq`) carries its picture as the JSON of an
//! [`HqSituation`] in `Question::situation`: only what reached that HQ (its
//! COP, its subordinates' last reports), never the truth (rule 1). If the
//! situation is plain words instead, the planners still run, on the words.

use crate::decider::Planner;
use async_trait::async_trait;
use bgws_core::{Answer, AnswerSource, Figures, Geodetic, Question};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Which planner an HQ seat runs (MULTI_DOMAIN_PLAN §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlannerKind {
    /// The joint commander: writes JointGuidance for component HQs.
    Overall,
    #[default]
    Land,
    Tactical,
    Air,
    Maritime,
    SpaceCyber,
    Sustainment,
}

/// A track in the HQ's common operating picture, as it reached the HQ.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopTrack {
    pub id: String,
    /// What the HQ thinks it is ("a tank platoon").
    pub thought: String,
    pub domain: String,
    pub seen_ago_s: f64,
    pub by: String,
    /// Report quality, 0–15.
    pub quality: u8,
    pub error_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Geodetic>,
}

/// What an HQ's unit is for, in planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnitRole {
    Manoeuvre,
    /// Guns, rockets, mortars and strike drone teams: positioned by reach.
    Fires,
    Engineer,
    Support,
    Post,
    /// Reconnaissance drone teams: positioned by reach to watch the objective.
    Isr,
}

/// One of the HQ's own units, from its last report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HqUnit {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub role: UnitRole,
    /// Combat strength (the land engine's units).
    pub strength: f64,
    pub vehicles_fit: u32,
    pub vehicles_total: u32,
    pub at: Geodetic,
    /// Longest weapon reach, for fire units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach_m: Option<f64>,
    #[serde(default = "yes")]
    pub alive: bool,
}

fn yes() -> bool {
    true
}

/// Rules of engagement and escalation in guidance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceRoe {
    pub weapons_free: bool,
    pub escalation_limit: String,
}

/// Guidance from above, or written by this HQ for those below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Guidance {
    pub by: String,
    #[serde(default)]
    pub aims: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_effort: Option<String>,
    #[serde(default)]
    pub priorities: Vec<String>,
    #[serde(default)]
    pub apportionment: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roe: Option<GuidanceRoe>,
    #[serde(default)]
    pub measures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Geodetic>,
}

/// What an HQ question carries (as JSON in `Question::situation`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HqSituation {
    pub hq: String,
    pub echelon: String,
    pub planner: PlannerKind,
    /// Its direct subordinates (units and HQs).
    pub subordinates: Vec<String>,
    /// Subordinate HQs still working.
    #[serde(default)]
    pub sub_hqs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<Geodetic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<Guidance>,
    /// Why it is planning now (a CCIR, the battle rhythm).
    pub why: String,
    #[serde(default)]
    pub cop: Vec<CopTrack>,
    #[serde(default)]
    pub units: Vec<HqUnit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Geodetic>,
    /// Named points orders may refer to ("OBJECTIVE").
    #[serde(default)]
    pub reference_points: BTreeMap<String, Geodetic>,
    /// For the overall planner: shared means by type and what exists.
    #[serde(default)]
    pub shareable_means: BTreeMap<String, f64>,
}

impl HqSituation {
    /// Read the situation an HQ question carries, if it is one.
    pub fn of(q: &Question) -> Option<HqSituation> {
        serde_json::from_str(&q.situation).ok()
    }
}

const ESCALATION: [&str; 3] = ["conventional", "noStrategicStrike", "noCrossBorder"];

/// The orders schema the plan tool must match: mission orders per unit.
pub fn orders_plan_schema() -> Value {
    let point = json!({"type": "object", "properties": {
        "ref": {"type": "string"}, "bearingDeg": {"type": "number", "minimum": 0, "maximum": 360}, "distanceM": {"type": "number", "minimum": 0},
        "lat": {"type": "number", "minimum": -90, "maximum": 90}, "lon": {"type": "number", "minimum": -180, "maximum": 180}}});
    json!({
        "type": "object",
        "required": ["plan", "orders"],
        "properties": {
            "plan": {"type": "string"},
            "orders": {"type": "array", "items": {
                "type": "object",
                "required": ["unit", "task", "intent", "phases"],
                "properties": {
                    "unit": {"type": "string"},
                    "task": {"type": "string"},
                    "intent": {"type": "string"},
                    "urgency": {"type": "string", "enum": ["whenAble", "now"]},
                    "roe": {"type": "string", "enum": ["never", "ifFiredUpon", "withinShortRange", "always"]},
                    "onContact": {"type": "string", "enum": ["engage", "observe", "avoid", "bypass"]},
                    "supports": {"type": "string"},
                    "phases": {"type": "array", "minItems": 1, "items": {
                        "type": "object", "required": ["label", "do"],
                        "properties": {
                            "label": {"type": "string"},
                            "do": {"type": "string", "enum": ["move", "withdraw", "hold", "overwatch", "observe"]},
                            "to": point,
                            "mode": {"type": "string", "enum": ["march", "tactical", "assault", "bound"]}
                        }
                    }},
                    "constraints": {"type": "array", "items": {"type": "object", "required": ["stay", "of"], "properties": {
                        "stay": {"type": "string", "enum": ["north", "south", "east", "west"]}, "of": point, "label": {"type": "string"}}}}
                }
            }}
        }
    })
}

/// The JointGuidance schema for the overall planner.
pub fn joint_guidance_schema() -> Value {
    json!({
        "type": "object",
        "required": ["aims", "priorities", "apportionment", "roe"],
        "properties": {
            "aims": {"type": "array", "items": {"type": "string"}, "minItems": 1},
            "mainEffort": {"type": "string"},
            "priorities": {"type": "array", "items": {"type": "string"}},
            "apportionment": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0}},
            "roe": {"type": "object", "required": ["weaponsFree", "escalationLimit"], "properties": {
                "weaponsFree": {"type": "boolean"}, "escalationLimit": {"type": "string", "enum": ESCALATION}}}
        }
    })
}

/// The HQ's picture, in words: what reached it, and how old (`pictureWords`).
pub fn picture_words(cop: &[CopTrack]) -> String {
    if cop.is_empty() {
        return "Nothing about the enemy has reached this HQ.".into();
    }
    let mut tracks: Vec<&CopTrack> = cop.iter().collect();
    tracks.sort_by(|a, b| a.seen_ago_s.total_cmp(&b.seen_ago_s).then_with(|| a.id.cmp(&b.id)));
    tracks
        .iter()
        .take(25)
        .map(|t| {
            let off = if t.error_m > 200.0 { format!(", may be {} m off", t.error_m.round()) } else { String::new() };
            format!("{}: {} ({}), seen {} min ago by {}, quality {}/15{off}", t.id, t.thought, t.domain, (t.seen_ago_s / 60.0).round(), t.by, t.quality)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prompt for a land or tactical planner (`claudeLandPlanner` + `ordersPrompt`).
pub fn land_prompt(s: &HqSituation, side: &str) -> String {
    let mut lines = vec![format!("You are the {} HQ {}. You plan for your own subordinates only: {}.", s.echelon, s.hq, s.subordinates.join(", "))];
    if let Some(g) = &s.guidance {
        lines.push(format!("Guidance from {}: {}. Priorities: {}.", g.by, g.aims.join("; "), g.priorities.join(", ")));
    }
    lines.push(format!("Why you are planning now: {}.", s.why));
    lines.push(format!("What has reached your HQ:\n{}", picture_words(&s.cop)));
    let units: Vec<Value> = s
        .units
        .iter()
        .filter(|u| u.alive)
        .map(|u| json!({"id": u.id, "unit": u.label, "role": u.role, "vehicles": format!("{} of {}", u.vehicles_fit, u.vehicles_total), "at": u.at}))
        .collect();
    let refs: Vec<&str> = s.reference_points.keys().map(String::as_str).collect();
    lines.push(String::new());
    lines.push(format!(
        "You command the {side} side's {} at this HQ. The clock is stopped. Write mission orders for your units; each unit's leader carries them out and makes the local calls within them, weighing every one against your intent.",
        s.echelon
    ));
    lines.push(format!("Your units: {}", Value::Array(units)));
    lines.push(String::new());
    lines.push("Answer by calling the submit_plan tool exactly once. Rules:".into());
    lines.push("- unit: one of your units' ids. Units you leave out keep their current orders.".into());
    lines.push("- phases, in order. do: \"move\" | \"withdraw\" | \"hold\" | \"overwatch\" | \"observe\". A move or withdraw needs \"to\"; hold, overwatch and observe are where a task ends, so put them last. mode (moves only): \"march\" (fastest, does not fire), \"tactical\" (fires within its ROE; the default), \"assault\" (fires at anything, closes to point-blank: how ground is taken), \"bound\" (slow, pairs cover each other).".into());
    lines.push(format!("- to: {{\"ref\": name, \"bearingDeg\": 0-359, \"distanceM\": metres}} from a reference point, or {{\"lat\", \"lon\"}}. Reference points: {}.", if refs.is_empty() { "none".into() } else { refs.join(", ") }));
    lines.push("- urgency: \"whenAble\" (the default: a unit in a fight may finish it first) or \"now\" (it breaks off at once; use it only when the timing is the point).".into());
    lines.push("- roe: \"never\" (weapons hold), \"ifFiredUpon\", \"withinShortRange\", \"always\". onContact: \"engage\", \"observe\" (report, do not fire), \"avoid\" (pull back), \"bypass\" (carry on).".into());
    lines.push("- constraints: fixed lines on the ground not to cross, {\"stay\": \"north\"|\"south\"|\"east\"|\"west\", \"of\": <point>, \"label\": \"a few words\"}. Leave the list empty if there are none.".into());
    lines.join("\n")
}

/// The prompt for the overall (joint) planner (`jointGuidancePrompt`).
pub fn joint_prompt(s: &HqSituation, side: &str) -> String {
    let means: Vec<String> = s.shareable_means.iter().map(|(k, v)| format!("{k} {v}")).collect();
    [
        format!("You are the overall (joint) commander for {side}, at {}. Write JointGuidance for your component commanders: {}.", s.hq, s.subordinates.join(", ")),
        format!("Shared means you can apportion: {}.", means.join(", ")),
        format!("What has reached your HQ:\n{}", picture_words(&s.cop)),
        "Answer by calling the submit_plan tool exactly once, with aims, mainEffort (a component HQ id), priorities, apportionment (means → number) and roe {weaponsFree, escalationLimit: conventional | noStrategicStrike | noCrossBorder}.".into(),
        "Escalation is yours to limit: choose the lowest level that achieves the aims.".into(),
    ]
    .join("\n")
}

/// Read and check JointGuidance (`parseJointGuidance`): an apportionment
/// above what exists is cut back and the overage becomes a request to the
/// joint HQ; an unknown escalation level becomes conventional; a main effort
/// that is not a component HQ is dropped. Returns (guidance, warnings, requests).
pub fn parse_joint_guidance(raw: &Value, s: &HqSituation) -> (Guidance, Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut requests = Vec::new();
    let strings = |v: Option<&Value>| -> Vec<String> { v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default() };
    let aims = strings(raw.get("aims"));
    if aims.is_empty() {
        warnings.push("no aims given".into());
    }
    let mut apportionment = BTreeMap::new();
    for (k, v) in raw.get("apportionment").and_then(Value::as_object).into_iter().flatten() {
        match (v.as_f64(), s.shareable_means.get(k)) {
            (Some(x), Some(&have)) => {
                apportionment.insert(k.clone(), x.min(have));
                if x > have {
                    requests.push(format!("{k}: asked for {x}, only {have} exist — the overage goes to the joint HQ as a request"));
                }
            }
            _ => warnings.push(format!("apportionment \"{k}\" ignored")),
        }
    }
    let roe = raw.get("roe");
    let asked = roe.and_then(|r| r.get("escalationLimit")).and_then(Value::as_str);
    let escalation = asked.filter(|e| ESCALATION.contains(e)).unwrap_or("conventional");
    if let Some(a) = asked.filter(|a| !ESCALATION.contains(a)) {
        warnings.push(format!("unknown escalation limit \"{a}\": conventional"));
    }
    let main_effort = raw.get("mainEffort").and_then(Value::as_str).filter(|m| s.subordinates.iter().any(|x| x == m)).map(str::to_owned);
    if raw.get("mainEffort").is_some_and(|m| !m.is_null()) && main_effort.is_none() {
        warnings.push(format!("main effort {} is not a component HQ", raw["mainEffort"]));
    }
    let guidance = Guidance {
        by: s.hq.clone(),
        aims,
        main_effort,
        priorities: strings(raw.get("priorities")),
        apportionment,
        roe: Some(GuidanceRoe { weapons_free: roe.and_then(|r| r.get("weaponsFree")).and_then(Value::as_bool) != Some(false), escalation_limit: escalation.into() }),
        measures: s.guidance.as_ref().map(|g| g.measures.clone()).unwrap_or_default(),
        objective: None,
    };
    (guidance, warnings, requests)
}

/// The heuristic planner's numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct HeuristicParams {
    pub culmination_loss: f64,
    pub enemy_radius_m: f64,
    pub attack_ratio: f64,
    pub strength_per_track: f64,
    pub frontage_m: f64,
    pub reserve_back_m: f64,
    pub engineer_back_m: f64,
    pub post_back_m: f64,
    pub post_follow_beyond_m: f64,
    pub fires_move_up: f64,
    pub fires_standoff: f64,
    pub enemy_close_m: f64,
    pub culmination_min_distance_m: f64,
    pub min_units_for_reserve: usize,
}

impl HeuristicParams {
    pub fn from_figures(f: &Figures) -> HeuristicParams {
        HeuristicParams {
            culmination_loss: f.get("decide.heuristic.culmination_loss"),
            enemy_radius_m: f.get("decide.heuristic.enemy_radius"),
            attack_ratio: f.get("decide.heuristic.attack_ratio"),
            strength_per_track: f.get("decide.heuristic.strength_per_track"),
            frontage_m: f.get("decide.heuristic.frontage"),
            reserve_back_m: f.get("decide.heuristic.reserve_back"),
            engineer_back_m: f.get("decide.heuristic.engineer_back"),
            post_back_m: f.get("decide.heuristic.post_back"),
            post_follow_beyond_m: f.get("decide.heuristic.post_follow_beyond"),
            fires_move_up: f.get("decide.heuristic.fires_move_up"),
            fires_standoff: f.get("decide.heuristic.fires_standoff"),
            enemy_close_m: f.get("decide.heuristic.enemy_close"),
            culmination_min_distance_m: f.get("decide.heuristic.culmination_min_distance"),
            min_units_for_reserve: f.get("decide.heuristic.min_units_for_reserve") as usize,
        }
    }
}

fn deg(b: f64) -> f64 {
    b.to_radians()
}

fn point(g: Geodetic) -> Value {
    json!({"lat": g.lat, "lon": g.lon})
}

fn order(unit: &str, phases: Vec<Value>, task: &str, intent: &str, on_contact: &str, roe: &str) -> Value {
    json!({"unit": unit, "task": task, "intent": intent, "urgency": "whenAble", "roe": roe, "onContact": on_contact, "phases": phases})
}

fn phase(label: &str, what: &str, to: Option<Geodetic>, mode: Option<&str>) -> Value {
    let mut p = json!({"label": label, "do": what});
    if let Some(t) = to {
        p["to"] = point(t);
    }
    if let Some(m) = mode {
        p["mode"] = json!(m);
    }
    p
}

/// The heuristic plan (`heuristicPlan`), in the orders-plan shape, plus
/// guidance for the HQs below. Works only from the HQ's own picture.
pub fn heuristic_plan(s: &HqSituation, p: &HeuristicParams) -> Value {
    let objective = s.guidance.as_ref().and_then(|g| g.objective).or(s.objective);
    let units: Vec<&HqUnit> = s.units.iter().filter(|u| u.alive && u.strength > 0.0).collect();
    let anchor = s.post.or_else(|| units.first().map(|u| u.at)).or(objective);
    let (Some(objective), Some(anchor)) = (objective, anchor) else {
        return json!({"by": "heuristic", "plan": "hold: no objective", "orders": []});
    };
    let own: f64 = units.iter().map(|u| u.strength).sum();
    let per = if units.is_empty() { p.strength_per_track } else { own / units.len() as f64 };
    let enemy = s.cop.iter().filter(|t| t.domain == "land" && t.at.is_some_and(|a| a.surface_distance(objective) <= p.enemy_radius_m)).count() as f64 * per;
    let ratio = own / enemy.max(1.0);
    let total: u32 = s.units.iter().map(|u| u.vehicles_total).sum();
    let fit: u32 = s.units.iter().map(|u| u.vehicles_fit).sum();
    let lost = if total > 0 { 1.0 - fit as f64 / total as f64 } else { 0.0 };
    let culminated = lost >= p.culmination_loss && anchor.surface_distance(objective) > p.culmination_min_distance_m;
    let attack = !culminated && (enemy == 0.0 || ratio >= p.attack_ratio);
    let r1 = (ratio * 10.0).round() / 10.0;
    let intent = if culminated {
        format!("the attack has culminated at {}% losses: hold and consolidate", (lost * 100.0).round())
    } else if attack {
        format!("take the objective and hold it ({} attack, {r1}:1)", s.echelon)
    } else {
        format!("hold where we stand until the odds improve ({r1}:1)")
    };
    let mut guidance = json!({
        "by": s.hq, "aims": [intent],
        "priorities": if attack { json!(["the objective", "fire support to the main effort"]) } else { json!(["hold", "counter-battery"]) },
        "apportionment": {},
        "roe": s.guidance.as_ref().and_then(|g| g.roe.clone()).map(|r| json!(r)).unwrap_or(json!({"weaponsFree": true, "escalationLimit": "conventional"})),
        "measures": s.guidance.as_ref().map(|g| g.measures.clone()).unwrap_or_default(),
    });
    if attack {
        guidance["objective"] = point(objective);
    }
    if let Some(me) = s.sub_hqs.first() {
        guidance["mainEffort"] = json!(me);
    }

    let mut manoeuvre: Vec<&HqUnit> = units.iter().copied().filter(|u| u.role == UnitRole::Manoeuvre).collect();
    manoeuvre.sort_by(|a, b| a.id.cmp(&b.id));
    let reserve = (attack && manoeuvre.len() >= p.min_units_for_reserve).then(|| manoeuvre[manoeuvre.len() - 1].id.clone());
    let lead: Vec<&HqUnit> = manoeuvre.iter().copied().filter(|u| Some(&u.id) != reserve.as_ref()).collect();
    let axis = anchor.bearing_to(objective).to_degrees();
    let mut orders = Vec::new();
    for (i, u) in lead.iter().enumerate() {
        if !attack {
            orders.push(order(&u.id, vec![phase("hold this ground on overwatch", "overwatch", None, None)], "hold", &intent, "engage", "withinShortRange"));
            continue;
        }
        let offset = (i as f64 - (lead.len() as f64 - 1.0) / 2.0) * p.frontage_m;
        let target = objective.destination(deg((axis + 90.0).rem_euclid(360.0)), offset);
        let close = s.cop.iter().any(|t| t.domain == "land" && t.at.is_some_and(|a| a.surface_distance(u.at) < p.enemy_close_m));
        orders.push(order(
            &u.id,
            vec![phase("advance on the objective", "move", Some(target), Some(if close { "bound" } else { "tactical" })), phase("hold the objective on overwatch", "overwatch", None, None)],
            "attack: take the objective",
            &intent,
            "engage",
            "withinShortRange",
        ));
    }
    let behind = |d: f64| objective.destination(deg((axis + 180.0).rem_euclid(360.0)), d);
    if let Some(r) = &reserve {
        orders.push(order(
            r,
            vec![phase("follow as the reserve", "move", Some(behind(p.reserve_back_m)), Some("tactical")), phase("wait as the reserve", "hold", None, None)],
            "reserve",
            &format!("{intent}; be ready to reinforce"),
            "observe",
            "withinShortRange",
        ));
    }
    for u in &units {
        match u.role {
            UnitRole::Manoeuvre => {}
            UnitRole::Fires => {
                let reach = u.reach_m.unwrap_or(0.0);
                let d = u.at.surface_distance(objective);
                let phases = if attack && d > reach * p.fires_move_up {
                    vec![phase("move up into range", "move", Some(behind(reach * p.fires_standoff)), Some("tactical")), phase("in position: answer calls for fire", "hold", None, None)]
                } else {
                    vec![phase("in position: answer calls for fire", "hold", None, None)]
                };
                orders.push(order(&u.id, phases, "fire support", &intent, "avoid", "withinShortRange"));
            }
            UnitRole::Engineer => {
                let phases = if attack {
                    vec![phase("follow the lead, ready to breach", "move", Some(behind(p.engineer_back_m)), Some("tactical")), phase("ready to breach", "hold", None, None)]
                } else {
                    vec![phase("prepare obstacles", "hold", None, None)]
                };
                orders.push(order(&u.id, phases, "mobility support", &intent, "avoid", "withinShortRange"));
            }
            UnitRole::Post => {
                let phases = if attack && u.at.surface_distance(objective) > p.post_follow_beyond_m {
                    vec![phase("follow the attack", "move", Some(behind(p.post_back_m)), Some("tactical")), phase("command", "hold", None, None)]
                } else {
                    vec![phase("command", "hold", None, None)]
                };
                orders.push(order(&u.id, phases, "command", &intent, "avoid", "ifFiredUpon"));
            }
            UnitRole::Isr => {
                let reach = u.reach_m.unwrap_or(0.0);
                let phases = if reach > 0.0 && u.at.surface_distance(objective) > reach * p.fires_move_up {
                    vec![phase("move up to watch the objective", "move", Some(behind(reach * p.fires_standoff)), Some("tactical")), phase("watch the objective and report", "hold", None, None)]
                } else {
                    vec![phase("watch the objective and report", "hold", None, None)]
                };
                orders.push(order(&u.id, phases, "reconnaissance", &intent, "avoid", "withinShortRange"));
            }
            UnitRole::Support => orders.push(order(&u.id, vec![phase("cover the force", "hold", None, None)], "support the force", &intent, "avoid", "withinShortRange")),
        }
    }
    let mut plan = json!({
        "by": "heuristic",
        "plan": format!("{}: {intent}{}", s.hq, reserve.as_ref().map(|r| format!("; reserve {r}")).unwrap_or_default()),
        "orders": orders,
        "guidance": guidance,
    });
    if let Some(r) = reserve {
        plan["reserve"] = json!(r);
    }
    plan
}

/// The heuristic planner as a [`Planner`]: the fallback for every HQ seat and
/// the default for batch runs. Deterministic and free.
#[derive(Clone, Debug)]
pub struct HeuristicPlanner {
    pub params: HeuristicParams,
}

impl HeuristicPlanner {
    pub fn from_figures(f: &Figures) -> HeuristicPlanner {
        HeuristicPlanner { params: HeuristicParams::from_figures(f) }
    }

    /// The heuristic's answer, with this source (Rules, or a Fallback with its reason).
    pub fn answer(&self, q: &Question, source: AnswerSource) -> Answer {
        let plan = match HqSituation::of(q) {
            Some(s) => heuristic_plan(&s, &self.params),
            None => json!({"by": "heuristic", "plan": "hold: the HQ's situation could not be read", "orders": []}),
        };
        let words = plan["plan"].as_str().unwrap_or_default().to_owned();
        Answer { plan: Some(plan), rationale: Some(words), ..Answer::simple(q, q.fallback.clone(), source) }
    }
}

impl Default for HeuristicPlanner {
    fn default() -> Self {
        HeuristicPlanner::from_figures(&crate::default_figures())
    }
}

#[async_trait]
impl Planner for HeuristicPlanner {
    async fn plan(&self, question: Question) -> Answer {
        self.answer(&question, AnswerSource::Rules)
    }
    fn name(&self) -> &str {
        "heuristic"
    }
}
