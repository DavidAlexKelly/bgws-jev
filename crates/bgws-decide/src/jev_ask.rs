//! How Jev is asked: the request style.
//!
//! TypeSafe's guidance for System One models (Jev) is to send each item in
//! its own question, keep every field of the state relevant to the question,
//! write the question plainly (no preamble, the options described in the
//! criteria), and not gate on confidence. The `legacy` style predates that:
//! one request per side with every unit's state, a templated instruction
//! carrying the balance rule and doctrine, and a confidence cut-off. `plain`
//! follows the guidance for the same question (pick the unit's next action);
//! `judge` (see [`crate::jev_judge`]) asks Jev only for judgments and decides
//! in code.

use crate::jev::Keyed;
use bgws_behaviour::Vocabulary;
use bgws_core::Question;
use serde_json::{json, Map, Value};

/// The request style (`BGWS_JEV_STYLE`: `legacy`, `plain` or `judge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    /// One request per side, templated instructions, a confidence cut-off.
    Legacy,
    /// One plain question per unit, its state cut to what the decision needs, no cut-off.
    #[default]
    Plain,
    /// Judgments from Jev, the decision in code (tables at the decision
    /// points that matter, the doctrine elsewhere).
    Judge,
}

impl Style {
    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "legacy" => Some(Style::Legacy),
            "plain" => Some(Style::Plain),
            "judge" => Some(Style::Judge),
            _ => None,
        }
    }

    /// From `BGWS_JEV_STYLE`, else the default.
    pub fn from_env() -> Style {
        std::env::var("BGWS_JEV_STYLE").ok().and_then(|s| Style::parse(&s)).unwrap_or_default()
    }

    pub fn word(self) -> &'static str {
        match self {
            Style::Legacy => "legacy",
            Style::Plain => "plain",
            Style::Judge => "judge",
        }
    }
}

/// What a unit knows, said once in its state.
pub const KNOWLEDGE: &str = "Only what this unit believes. Enemies it has not found are not listed; a lost contact is where an enemy was last seen.";

/// The parts of a unit's state a decision point needs; the rest is left out,
/// as a field sent for one judgment can sway another.
pub fn trim_state(trigger: &str, st: &Value) -> Value {
    let Some(src) = st.as_object() else { return st.clone() };
    let mut out = Map::new();
    let copy = |out: &mut Map<String, Value>, k: &str| {
        if let Some(v) = src.get(k) {
            out.insert(k.into(), v.clone());
        }
    };
    copy(&mut out, "unit");
    copy(&mut out, "orders");
    let enemies = !matches!(trigger, "D0" | "D3" | "D11");
    if enemies {
        // The enemy the decision is about, then the two nearest others.
        if let Some(list) = src.get("enemiesSeen").and_then(Value::as_array) {
            let about = list.iter().filter(|e| e.get("thisDecisionIsAboutIt").and_then(Value::as_bool) == Some(true));
            let others = list.iter().filter(|e| e.get("thisDecisionIsAboutIt").and_then(Value::as_bool) != Some(true)).take(2);
            let kept: Vec<Value> = about.chain(others).cloned().collect();
            if !kept.is_empty() {
                out.insert("enemiesSeen".into(), Value::Array(kept));
            }
        }
        copy(&mut out, "weaponReachM");
    }
    if matches!(trigger, "D3" | "D2") {
        copy(&mut out, "firedOnByUnseen");
    }
    if trigger == "D8" {
        copy(&mut out, "weapons");
    }
    if matches!(trigger, "D2" | "D3" | "D9" | "D11" | "D12") {
        copy(&mut out, "friendsWithin3km");
    }
    if let Some(g) = src.get("ground").and_then(Value::as_object) {
        let keep: &[&str] = match trigger {
            "D10" => &["cover", "concealment", "land", "onRoad", "deadGround", "keyTerrainNear"],
            "D1" | "D2" => &["cover", "concealment", "land", "deadGround", "openToView", "inTownBlock", "keyTerrainNear"],
            _ => &["cover", "concealment", "land", "deadGround", "openToView", "inTownBlock"],
        };
        let ground: Map<String, Value> = g.iter().filter(|(k, _)| keep.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        if !ground.is_empty() {
            out.insert("ground".into(), Value::Object(ground));
        }
    }
    Value::Object(out)
}

/// An option as Jev reads it: the action's one-sentence description, and
/// the doctrine's lean as a short tag.
pub fn option_words(vocab: &Vocabulary, action: &str, summary: &str, bias: i8) -> String {
    let base = vocab.action(action).map(|a| a.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| summary.to_string());
    let tag = match bias {
        b if b >= 2 => " Its doctrine strongly prefers this.",
        b if b > 0 => " Its doctrine prefers this.",
        b if b <= -2 => " Its doctrine strongly advises against this.",
        b if b < 0 => " Its doctrine advises against this.",
        _ => "",
    };
    format!("{base}{tag}")
}

/// One land unit's question as its own request: its trimmed state, a plain
/// question, and the options described in the criteria.
pub fn plain_request(model: &str, q: &Question, max_choices: usize, vocab: &Vocabulary) -> (Value, Keyed) {
    let mut options: Vec<(String, String)> = Vec::new();
    for (i, o) in q.options.iter().take(max_choices).enumerate() {
        let taken: Vec<String> = options.iter().map(|(k, _)| k.clone()).collect();
        options.push((crate::jev::option_key(&o.id, i, &taken), o.id.clone()));
    }
    let mut criteria = Map::new();
    for ((k, _), o) in options.iter().zip(&q.options) {
        criteria.insert(k.clone(), Value::String(option_words(vocab, &o.action, &o.summary, o.bias)));
    }
    let mut state = match &q.state {
        Some(st) => trim_state(&q.trigger, st),
        None => json!({ "situation": q.situation }),
    };
    if let Some(m) = state.as_object_mut() {
        m.insert("knowledge".into(), Value::String(KNOWLEDGE.into()));
    }
    let about = if q.state.as_ref().and_then(|s| s.pointer("/enemiesSeen")).is_some() { " The enemy this is about is marked in `enemiesSeen`." } else { "" };
    let instructions = format!("{} What should `unit` do now, given its `orders`?{about}", q.question.trim());
    let key = "next".to_string();
    let mut qs = Map::new();
    qs.insert(key.clone(), json!({ "type": "choice", "instructions": instructions, "criteria": criteria }));
    (json!({ "model": model, "state": state, "questions": qs }), Keyed { key, options })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trimmed_state_keeps_what_the_point_needs() {
        let st = json!({
            "unit": {"name": "u"}, "orders": {"doing": "moving"}, "weapons": [1], "weaponReachM": 3000.0,
            "enemiesSeen": [{"rangeM": 900.0}, {"rangeM": 2000.0, "thisDecisionIsAboutIt": true}, {"rangeM": 2100.0}, {"rangeM": 2200.0}],
            "firedOnByUnseen": [], "friendsWithin3km": [{"unit": "f"}],
            "ground": {"cover": 0.1, "keyTerrainNear": [], "onRoad": true}
        });
        let d1 = trim_state("D1", &st);
        assert!(d1.get("weapons").is_none() && d1.get("friendsWithin3km").is_none() && d1.get("firedOnByUnseen").is_none());
        let e = d1["enemiesSeen"].as_array().unwrap();
        assert_eq!(e.len(), 3, "the one it is about, then two others");
        assert_eq!(e[0]["thisDecisionIsAboutIt"], true);
        assert!(d1["ground"].get("onRoad").is_none());
        let d8 = trim_state("D8", &st);
        assert!(d8.get("weapons").is_some());
        let d3 = trim_state("D3", &st);
        assert!(d3.get("enemiesSeen").is_none() && d3.get("firedOnByUnseen").is_some() && d3.get("friendsWithin3km").is_some());
    }

    #[test]
    fn styles_read_from_words() {
        assert_eq!(Style::parse("Judge"), Some(Style::Judge));
        assert_eq!(Style::parse("legacy"), Some(Style::Legacy));
        assert_eq!(Style::parse("x"), None);
    }
}
