//! The Behaviour Manager page (§16): every template with its parent chain
//! merged in, drawn as decision graphs per trigger, and the action library.
//! Read-only: templates are edited as files (`data/templates/`) and checked
//! on load.

use bgws_behaviour::Registry;
use serde_json::{json, Value};

const TEMPLATE: &str = include_str!("../../../web/behaviour/index.html");

/// The page's data: resolved templates and the action library.
pub fn data(registry: &Registry) -> Value {
    let templates: Vec<Value> = registry
        .templates()
        .iter()
        .filter_map(|(id, own)| {
            let r = registry.resolved(id)?;
            Some(json!({ "id": id, "name": own.name, "archetype": own.archetype, "own": own, "resolved": r }))
        })
        .collect();
    let mut actions = bgws_behaviour::library::land_actions();
    actions.extend(bgws_behaviour::library::world_actions());
    json!({ "templates": templates, "actions": actions })
}

/// The page for the built-in templates.
pub fn html(registry: &Registry) -> String {
    TEMPLATE.replace("__BGWS_DATA__", &data(registry).to_string().replace("</", "<\\/"))
}

/// The page for the shipped templates.
pub fn builtin_html() -> anyhow::Result<String> {
    let reg = Registry::builtin().map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(html(&reg))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_builtin_template_is_on_the_page_resolved() {
        let reg = bgws_behaviour::Registry::builtin().unwrap();
        let d = super::data(&reg);
        let ts = d["templates"].as_array().unwrap();
        assert_eq!(ts.len(), reg.templates().len());
        let armour = ts.iter().find(|t| t["id"] == "core/armour").unwrap();
        // Armour states only its doctrine; the triggers come from core/default.
        assert!(armour["own"]["triggers"].is_null());
        assert!(armour["resolved"]["triggers"].as_object().unwrap().contains_key("D1"));
        assert!(!d["actions"].as_array().unwrap().is_empty());
        assert!(!super::html(&reg).contains("__BGWS_DATA__"));
    }
}
