//! HQ plan requests and the orders that come back (§10.3, §8.12). The sim
//! writes the HQ's picture as JSON in the question's `situation` and reads a
//! plan matching the orders schema. The shapes mirror `bgws_decide::hq`
//! (the JSON is the contract; the sim does not depend on the decider crate).

use bgws_core::{Geodetic, Side};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopTrack {
    pub id: String,
    pub thought: String,
    pub domain: String,
    pub seen_ago_s: f64,
    pub by: String,
    pub quality: u8,
    pub error_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Geodetic>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HqUnit {
    pub id: String,
    pub label: String,
    pub role: String,
    pub strength: f64,
    pub vehicles_fit: u32,
    pub vehicles_total: u32,
    pub at: Geodetic,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach_m: Option<f64>,
    pub alive: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HqSituation {
    pub hq: String,
    pub echelon: String,
    pub planner: String,
    pub subordinates: Vec<String>,
    #[serde(default)]
    pub sub_hqs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<Geodetic>,
    pub why: String,
    pub cop: Vec<CopTrack>,
    pub units: Vec<HqUnit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Geodetic>,
    #[serde(default)]
    pub reference_points: BTreeMap<String, Geodetic>,
    #[serde(default)]
    pub shareable_means: BTreeMap<String, f64>,
}

/// A point in an order: a named reference, a bearing and distance from one,
/// or a position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrderPoint {
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    #[serde(default)]
    pub bearing_deg: Option<f64>,
    #[serde(default)]
    pub distance_m: Option<f64>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub label: String,
    #[serde(rename = "do")]
    pub action: String,
    #[serde(default)]
    pub to: Option<OrderPoint>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitOrder {
    pub unit: String,
    pub task: String,
    pub intent: String,
    #[serde(default)]
    pub urgency: Option<String>,
    #[serde(default)]
    pub roe: Option<String>,
    #[serde(default)]
    pub on_contact: Option<String>,
    pub phases: Vec<Phase>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub plan: String,
    pub orders: Vec<UnitOrder>,
}

impl OrderPoint {
    /// Resolve against the reference points and a unit's own position.
    pub fn resolve(&self, refs: &BTreeMap<String, Geodetic>, own: Geodetic) -> Option<Geodetic> {
        if let (Some(lat), Some(lon)) = (self.lat, self.lon) {
            return Some(Geodetic::new(lat, lon, 0.0));
        }
        let base = match &self.reference {
            Some(r) if r.eq_ignore_ascii_case("self") || r.eq_ignore_ascii_case("here") => own,
            Some(r) => *refs.get(r).or_else(|| refs.iter().find(|(k, _)| k.eq_ignore_ascii_case(r)).map(|(_, v)| v))?,
            None => own,
        };
        match (self.bearing_deg, self.distance_m) {
            (Some(b), Some(d)) => Some(base.destination(b.to_radians(), d)),
            _ => Some(base),
        }
    }
}

/// The orders schema (identical to `bgws_decide::hq::orders_plan_schema`).
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

/// The HQ's agent id for a side's top planner.
pub fn hq_agent(side: Side) -> String {
    format!("{}-hq", side.as_str())
}
