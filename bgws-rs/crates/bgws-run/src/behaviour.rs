//! The Behaviour Manager page (§16): every template with its parent chain
//! merged in, drawn as decision graphs per trigger, and the action library.
//! Custom templates (variants of any other) live as files in a library
//! folder (`data/templates/custom/`): the page makes, changes and deletes
//! them through the server, each checked strictly against the shipped ones
//! before it is written; the scenario editor assigns them to forces.

use anyhow::{anyhow, bail, Result};
use bgws_behaviour::{LoadMode, Registry, Template, Vocabulary};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("../../../web/behaviour/index.html");

/// The page's data: resolved templates and the action library.
pub fn data(registry: &Registry) -> Value {
    let templates: Vec<Value> = registry
        .templates()
        .iter()
        .filter_map(|(id, own)| {
            let r = registry.resolved(id)?;
            Some(json!({ "id": id, "name": own.name, "archetype": own.archetype, "own": own, "resolved": r, "custom": id.starts_with("custom/") }))
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

/// The shipped custom-template library: `data/templates/custom/`.
pub fn library_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/templates/custom")
}

/// Every template in a library folder (none if it does not exist yet).
pub fn library(dir: &Path) -> Result<Vec<Template>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    paths.iter().map(|p| serde_json::from_str(&std::fs::read_to_string(p)?).map_err(|e| anyhow!("{}: {e}", p.display()))).collect()
}

/// The shipped templates and a library, checked strictly.
pub fn registry_with(dir: &Path) -> Result<Registry> {
    Registry::with_custom(&library(dir)?, &Vocabulary::builtin(), LoadMode::Strict).map_err(|e| anyhow!("{e}"))
}

/// The page with a library's templates as well.
pub fn library_html(dir: &Path) -> Result<String> {
    Ok(html(&registry_with(dir)?))
}

/// The library's file name for a custom template id (`custom/<name>`).
fn file_for(dir: &Path, id: &str) -> Result<PathBuf> {
    let name = id.strip_prefix("custom/").ok_or_else(|| anyhow!("a custom template's id starts with `custom/` (got `{id}`)"))?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("a custom template's name is letters, digits, `-` and `_` (got `{name}`)");
    }
    Ok(dir.join(format!("{name}.json")))
}

/// Add or replace a custom template, if the library still loads strictly
/// with it; the problems otherwise.
pub fn save(dir: &Path, t: &Template) -> std::result::Result<(), Vec<String>> {
    let path = file_for(dir, &t.id).map_err(|e| vec![e.to_string()])?;
    let mut lib: Vec<Template> = library(dir).map_err(|e| vec![e.to_string()])?.into_iter().filter(|x| x.id != t.id).collect();
    lib.push(t.clone());
    Registry::with_custom(&lib, &Vocabulary::builtin(), LoadMode::Strict).map_err(|e| e.to_string().lines().map(str::to_owned).collect::<Vec<_>>())?;
    std::fs::create_dir_all(dir).map_err(|e| vec![e.to_string()])?;
    let text = serde_json::to_string_pretty(t).map_err(|e| vec![e.to_string()])?;
    std::fs::write(&path, text + "\n").map_err(|e| vec![e.to_string()])
}

/// Remove a custom template, unless another one in the library inherits from it.
pub fn delete(dir: &Path, id: &str) -> Result<()> {
    let path = file_for(dir, id)?;
    if let Some(child) = library(dir)?.iter().find(|t| t.parent.as_deref() == Some(id)) {
        bail!("`{}` inherits from `{id}`: delete it first", child.id);
    }
    std::fs::remove_file(&path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// `ids` and every custom ancestor they need, from a library, for a
/// scenario's `behaviours.templates`.
pub fn with_ancestors(dir: &Path, ids: &[String]) -> Result<Vec<Template>> {
    let lib = library(dir)?;
    let mut out: Vec<Template> = Vec::new();
    let mut todo: Vec<String> = ids.to_vec();
    while let Some(id) = todo.pop() {
        if out.iter().any(|t| t.id == id) {
            continue;
        }
        if let Some(t) = lib.iter().find(|t| t.id == id) {
            if let Some(p) = &t.parent {
                todo.push(p.clone());
            }
            out.push(t.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_checked_saved_used_and_deleted() {
        let dir = std::env::temp_dir().join(format!("bgws-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut t = Template::variant("custom/armour-stand-fast", "core/armour");
        t.doctrine = Some("Holds its ground.".into());
        t.remove = Some(vec!["pullBack".into()]);
        save(&dir, &t).unwrap();
        let reg = registry_with(&dir).unwrap();
        assert!(reg.resolved("custom/armour-stand-fast").unwrap().remove.contains("pullBack"));
        // A variant of a variant, and what a scenario needs to carry for it.
        save(&dir, &Template::variant("custom/armour-stand-faster", "custom/armour-stand-fast")).unwrap();
        let need = with_ancestors(&dir, &["custom/armour-stand-faster".into()]).unwrap();
        assert_eq!(need.len(), 2);
        assert!(delete(&dir, "custom/armour-stand-fast").is_err(), "a parent in use stays");
        // Invalid templates are refused and not written.
        let bad = Template::variant("custom/orphan", "core/no-such-parent");
        assert!(!save(&dir, &bad).unwrap_err().is_empty());
        assert!(save(&dir, &Template::variant("core/sneaky", "core/armour")).is_err());
        assert!(save(&dir, &Template::variant("custom/../x", "core/armour")).is_err());
        delete(&dir, "custom/armour-stand-faster").unwrap();
        delete(&dir, "custom/armour-stand-fast").unwrap();
        assert!(library(&dir).unwrap().is_empty());
    }

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
