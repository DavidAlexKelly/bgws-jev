//! The user's behaviour templates and which unit uses which (RUST_ENGINE.md
//! §10.1; DECISION_FLOW_GRAPHS.md §4.4, §6). A port of `behaviour/store.ts`,
//! with the browser's localStorage replaced by a plain serialisable value the
//! host keeps wherever it likes.
//!
//! Built-in templates (`templates/core/*.json`) are read-only. User templates
//! are always ids `custom/<slug>` and are checked with the same validation CI
//! runs on the shipped files, so what is stored always loads.
//!
//! Resolution, most specific first (§4.4): a scenario file's own per-unit
//! choice (the caller layers that on top); the side's assignment for the
//! unit's kind ("Red armour uses Stand fast"); the unit type's assignment; the
//! kind's built-in default.

use crate::archetypes::default_template_for_class;
use crate::load::{build_registry, builtin_files, LoadMode};
use crate::registry::files_with;
use crate::types::Template;
use crate::vocabulary::Vocabulary;
use bgws_core::Side;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The kinds of unit a side assignment can name, by target class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UnitKind {
    #[serde(rename = "armoured_vehicle")]
    ArmouredVehicle,
    #[serde(rename = "foot")]
    Foot,
    #[serde(rename = "soft_skin")]
    SoftSkin,
}

impl UnitKind {
    pub const ALL: [UnitKind; 3] = [UnitKind::ArmouredVehicle, UnitKind::Foot, UnitKind::SoftSkin];
    pub fn as_str(self) -> &'static str {
        match self {
            UnitKind::ArmouredVehicle => "armoured_vehicle",
            UnitKind::Foot => "foot",
            UnitKind::SoftSkin => "soft_skin",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            UnitKind::ArmouredVehicle => "armour",
            UnitKind::Foot => "infantry",
            UnitKind::SoftSkin => "soft-skinned vehicles",
        }
    }
    pub fn parse(s: &str) -> Option<UnitKind> {
        UnitKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Why a unit has its template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssignmentWhy {
    Side,
    Asset,
    Default,
}

/// A unit, as resolution needs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRef {
    pub id: String,
    /// Its platform (asset) type.
    pub platform: String,
    pub side: Option<Side>,
    pub target_class: Option<String>,
}

/// What a game should run, fixed at its start (`behaviourSnapshot`).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BehaviourSnapshot {
    /// Unit id → template, where it is not its kind's default.
    pub behaviours: BTreeMap<String, String>,
    /// A copy of the user templates, so later edits never change a game in progress.
    pub behaviour_templates: Vec<Template>,
    pub why: BTreeMap<String, AssignmentWhy>,
}

/// Is this a shipped (read-only) template id?
pub fn is_built_in(id: &str) -> bool {
    !id.starts_with("custom/")
}

/// "Stand Fast!" → "stand-fast".
pub fn slugify(name: &str) -> String {
    let mut s = String::new();
    let mut in_run = false;
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            s.push(ch);
            in_run = false;
        } else if !in_run {
            s.push('-');
            in_run = true;
        }
    }
    let out: String = s.trim_matches('-').chars().take(48).collect();
    if out.is_empty() {
        "behaviour".into()
    } else {
        out
    }
}

/// The user's templates and assignments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplateStore {
    custom: Vec<Template>,
    /// Asset (platform type) → template.
    assignments: BTreeMap<String, String>,
    /// Side → unit kind → template.
    side_assignments: BTreeMap<Side, BTreeMap<UnitKind, String>>,
    /// Bumped on every change (the UI's change notification).
    #[serde(default)]
    version: u64,
}

impl TemplateStore {
    pub fn new() -> TemplateStore {
        TemplateStore::default()
    }

    /// Bumped on every change.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn custom_templates(&self) -> &[Template] {
        &self.custom
    }

    /// Every template: the shipped ones, then the user's.
    pub fn all_templates(&self) -> Vec<Template> {
        let mut out: Vec<Template> = builtin_files().into_values().filter_map(|v| serde_json::from_value(v).ok()).collect();
        out.extend(self.custom.iter().cloned());
        out
    }

    fn exists(&self, id: &str) -> bool {
        self.custom.iter().any(|t| t.id == id) || builtin_files().keys().any(|p| crate::load::template_id_from_path(p).as_deref() == Some(id))
    }

    /// Validate the whole set as it would be with `custom` saved. Empty = valid.
    pub fn check_templates(custom: &[Template]) -> Vec<String> {
        match build_registry(&files_with(custom), &Vocabulary::builtin(), LoadMode::Strict) {
            Ok(_) => Vec::new(),
            Err(e) => e.problems.iter().map(|p| p.to_string()).collect(),
        }
    }

    /// A free "custom/<slug>" id for this name.
    pub fn free_custom_id(&self, name: &str, except: Option<&str>) -> String {
        let taken = |id: &str| Some(id) != except && self.exists(id);
        let base = format!("custom/{}", slugify(name));
        if !taken(&base) {
            return base;
        }
        (2..).map(|i| format!("{base}-{i}")).find(|id| !taken(id)).unwrap_or(base)
    }

    /// Save a user template. Refuses (returning the problems) if the set
    /// would not validate. `previous_id` handles renames: assignments and
    /// children follow the new id.
    pub fn save_custom_template(&mut self, t: Template, previous_id: Option<&str>) -> Vec<String> {
        if is_built_in(&t.id) {
            return vec![format!("\"{}\" is a built-in template and cannot be changed", t.id)];
        }
        let mut custom: Vec<Template> = self.custom.iter().filter(|c| c.id != t.id && Some(c.id.as_str()) != previous_id).cloned().collect();
        if let Some(prev) = previous_id.filter(|p| *p != t.id) {
            for c in &mut custom {
                if c.parent.as_deref() == Some(prev) {
                    c.parent = Some(t.id.clone());
                }
            }
        }
        let new_id = t.id.clone();
        custom.push(t);
        custom.sort_by(|a, b| a.id.cmp(&b.id));
        let problems = Self::check_templates(&custom);
        if !problems.is_empty() {
            return problems;
        }
        self.custom = custom;
        if let Some(prev) = previous_id.filter(|p| *p != new_id) {
            for v in self.assignments.values_mut() {
                if v == prev {
                    *v = new_id.clone();
                }
            }
        }
        self.version += 1;
        Vec::new()
    }

    /// Replace the user templates with this set (a pull from the team's
    /// shared templates), if the whole set validates.
    pub fn replace_custom_templates(&mut self, templates: Vec<Template>) -> Vec<String> {
        let mut custom: Vec<Template> = templates.into_iter().filter(|t| !is_built_in(&t.id)).collect();
        custom.sort_by(|a, b| a.id.cmp(&b.id));
        let problems = Self::check_templates(&custom);
        if !problems.is_empty() {
            return problems;
        }
        self.custom = custom;
        self.version += 1;
        Vec::new()
    }

    /// User templates that inherit from this one directly.
    pub fn children_of(&self, id: &str) -> Vec<&Template> {
        self.custom.iter().filter(|t| t.parent.as_deref() == Some(id)).collect()
    }

    /// Delete a user template; assets using it go back to their default.
    /// Refuses if others inherit from it.
    pub fn delete_custom_template(&mut self, id: &str) -> Vec<String> {
        if is_built_in(id) {
            return vec![format!("\"{id}\" is a built-in template and cannot be deleted")];
        }
        let kids = self.children_of(id);
        if !kids.is_empty() {
            let names: Vec<String> = kids.iter().map(|k| k.name.clone().unwrap_or_else(|| k.id.clone())).collect();
            return vec![format!("{} build on it — delete or re-base those first", names.join(", "))];
        }
        self.custom.retain(|t| t.id != id);
        self.assignments.retain(|_, v| v != id);
        self.version += 1;
        Vec::new()
    }

    /// The template explicitly assigned to an asset, if it still exists.
    pub fn assignment_for(&self, asset: &str) -> Option<&str> {
        self.assignments.get(asset).map(String::as_str).filter(|id| self.exists(id))
    }

    /// The template an asset uses: its assignment, else its class's default.
    pub fn template_for(&self, asset: &str, target_class: Option<&str>) -> String {
        self.assignment_for(asset).map(str::to_owned).unwrap_or_else(|| default_template_for_class(target_class).to_owned())
    }

    pub fn is_assigned(&self, asset: &str, target_class: Option<&str>) -> bool {
        self.template_for(asset, target_class) != default_template_for_class(target_class)
    }

    /// Assign a template to an asset; `None` (or its default) clears it.
    pub fn assign_template(&mut self, asset: &str, template: Option<&str>, target_class: Option<&str>) {
        match template.filter(|t| *t != default_template_for_class(target_class)) {
            Some(t) => {
                self.assignments.insert(asset.into(), t.into());
            }
            None => {
                self.assignments.remove(asset);
            }
        }
        self.version += 1;
    }

    /// The template a side's units of this kind use, if assigned and still there.
    pub fn side_assignment_for(&self, side: Side, kind: Option<&str>) -> Option<&str> {
        let kind = UnitKind::parse(kind?)?;
        self.side_assignments.get(&side)?.get(&kind).map(String::as_str).filter(|id| self.exists(id))
    }

    /// Make a side's units of this kind use a template; `None` clears it.
    pub fn assign_side(&mut self, side: Side, kind: UnitKind, template: Option<&str>) {
        let m = self.side_assignments.entry(side).or_default();
        match template {
            Some(t) => {
                m.insert(kind, t.into());
            }
            None => {
                m.remove(&kind);
            }
        }
        self.version += 1;
    }

    /// Which side assignments name this template.
    pub fn side_uses_of(&self, template: &str) -> Vec<(Side, UnitKind)> {
        let mut out = Vec::new();
        for side in Side::PLAYERS {
            for kind in UnitKind::ALL {
                if self.side_assignments.get(&side).and_then(|m| m.get(&kind)).map(String::as_str) == Some(template) {
                    out.push((side, kind));
                }
            }
        }
        out
    }

    /// The template a unit uses and why, most specific first.
    pub fn resolve_behaviour(&self, unit: &UnitRef) -> (String, AssignmentWhy) {
        if let Some(t) = unit.side.and_then(|s| self.side_assignment_for(s, unit.target_class.as_deref())) {
            return (t.to_owned(), AssignmentWhy::Side);
        }
        if let Some(t) = self.assignment_for(&unit.platform) {
            return (t.to_owned(), AssignmentWhy::Asset);
        }
        (default_template_for_class(unit.target_class.as_deref()).to_owned(), AssignmentWhy::Default)
    }

    /// What a game should run, fixed at its start.
    pub fn behaviour_snapshot(&self, units: &[UnitRef]) -> BehaviourSnapshot {
        let mut snap = BehaviourSnapshot { behaviour_templates: self.custom.clone(), ..Default::default() };
        for u in units {
            let (t, why) = self.resolve_behaviour(u);
            if why == AssignmentWhy::Default {
                continue;
            }
            snap.behaviours.insert(u.id.clone(), t);
            snap.why.insert(u.id.clone(), why);
        }
        snap
    }

    /// How many assets have this template assigned (beyond defaults).
    pub fn assigned_count(&self, template: &str) -> usize {
        self.assignments.values().filter(|t| *t == template).count()
    }
}
