//! The scenario editor page (§16): start from a shipped scenario, set the
//! theatre, time, weather and duration, place forces from the catalogue,
//! check it, save it as v4 JSON, or load it into a running server game.

use anyhow::{anyhow, Result};
use bgws_scenario::scope::Weather;
use serde_json::{json, Value};
use std::path::Path;

const TEMPLATE: &str = include_str!("../../../web/editor/index.html");

/// The page's data: the shipped scenarios to start from, the catalogue's
/// platforms and the weather presets.
pub fn data(scenarios_dir: &Path) -> Result<Value> {
    let cat = bgws_catalogue::Catalogue::builtin().map_err(|e| anyhow!("catalogue: {e:?}"))?;
    let mut presets = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(scenarios_dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(&p)?;
        if let Ok(sc) = bgws_scenario::validate::load_any(&text, Some(&cat)) {
            presets.push(json!({ "id": p.file_stem().and_then(|s| s.to_str()).unwrap_or(""), "scenario": sc }));
        }
    }
    let platforms: Vec<Value> = cat.platforms().map(|p| json!({ "id": p.id, "name": p.name, "domain": p.domain, "role": p.role, "class": p.class })).collect();
    let weather: Vec<Value> = [Weather::Clear, Weather::Overcast, Weather::Rain, Weather::Fog, Weather::Snow, Weather::Storm].iter().map(|w| serde_json::to_value(w).unwrap_or_default()).collect();
    Ok(json!({ "presets": presets, "platforms": platforms, "weather": weather }))
}

pub fn html(scenarios_dir: &Path) -> Result<String> {
    Ok(TEMPLATE.replace("__BGWS_DATA__", &data(scenarios_dir)?.to_string().replace("</", "<\\/")))
}

/// Load and check an edited scenario: the scenario, or every problem found.
pub fn check(v: Value) -> std::result::Result<bgws_scenario::v4::ScenarioV4, Vec<String>> {
    let cat = bgws_catalogue::Catalogue::builtin().map_err(|e| vec![format!("catalogue: {e:?}")])?;
    let sc = bgws_scenario::validate::load_value(v, Some(&cat), &bgws_scenario::ConvertOptions::default()).map_err(|e| vec![e.to_string()])?;
    let errors: Vec<String> = bgws_scenario::validate::validate(&sc, &cat).iter().map(|e| e.to_string()).collect();
    if errors.is_empty() {
        Ok(sc)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios")
    }

    #[test]
    fn presets_load_and_check_and_edits_are_caught() {
        let d = super::data(&dir()).unwrap();
        let presets = d["presets"].as_array().unwrap();
        assert!(presets.len() >= 10);
        assert!(d["platforms"].as_array().unwrap().iter().any(|p| p["id"] == "generic/mbt"));
        let mut sc = presets.iter().find(|p| p["id"] == "skirmish-advance-to-contact-v1").unwrap()["scenario"].clone();
        assert!(super::check(sc.clone()).is_ok());
        sc["forces"][0]["platform"] = serde_json::json!("generic/no-such-thing");
        let errs = super::check(sc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("unknown platform")), "{errs:?}");
        assert!(!super::html(&dir()).unwrap().contains("__BGWS_DATA__"));
    }
}
