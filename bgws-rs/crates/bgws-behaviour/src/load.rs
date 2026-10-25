//! Turn the JSON template files into a validated [`Registry`] (RUST_ENGINE.md
//! §10.1, §14.3, §20). A port of `behaviour/load.ts`.
//!
//! Files live at `templates/<kind>/<name>.json` (kind = core | custom) and a
//! template's `"id"` must equal `"<kind>/<name>"`, so the id in a trace always
//! tells you which file to open. The shipped set (`data/templates/core`) is
//! compiled in with `include_str!`; a directory can also be read from disk.
//!
//! Unknown facts always fail. Unknown actions fail in [`LoadMode::Strict`]
//! (the TypeScript behaviour); in [`LoadMode::Compatible`] the loader instead
//! lists them in an [`ActionCompatibility`] table and keeps the templates,
//! whose unsupported options are then dropped at walk time and degrade to the
//! node's fallback (§20: "templates written for the TS engine need actions
//! the Rust engine lacks").

use crate::registry::Registry;
use crate::schema::{validate_registry, validate_template, Problem, ProblemKind};
use crate::types::{GraphNode, Template};
use crate::vocabulary::Vocabulary;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// path → parsed JSON.
pub type TemplateFiles = BTreeMap<String, Value>;

/// How strictly to treat actions the vocabulary does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LoadMode {
    /// Any unknown action fails the registry (as the TypeScript build does).
    #[default]
    Strict,
    /// Unknown actions are reported in the compatibility table; templates load.
    Compatible,
}

/// Every problem in every file. Prints as the TypeScript loader's message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadError {
    pub problems: Vec<Problem>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Behaviour templates are invalid:")?;
        for p in &self.problems {
            write!(f, "\n  {p}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadError {}

/// One action named by the templates, whether this engine has it, and where it is used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatRow {
    pub action: String,
    pub supported: bool,
    /// "core/default.nodes.D1.options[3]", …
    pub used_at: Vec<String>,
}

/// The action compatibility table (§20): every action the templates name.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionCompatibility {
    pub rows: Vec<CompatRow>,
}

impl ActionCompatibility {
    /// Actions named by templates that this engine does not have.
    pub fn unsupported(&self) -> Vec<&str> {
        self.rows.iter().filter(|r| !r.supported).map(|r| r.action.as_str()).collect()
    }

    /// Built from the templates and the vocabulary.
    pub fn of(templates: &BTreeMap<String, Template>, vocab: &Vocabulary) -> ActionCompatibility {
        let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for t in templates.values() {
            for (nid, node) in t.nodes.iter().flatten() {
                match node {
                    GraphNode::Choose { options, .. } => {
                        for (i, o) in options.iter().enumerate() {
                            by.entry(o.action.clone()).or_default().push(format!("{}.nodes.{nid}.options[{i}]", t.id));
                        }
                    }
                    GraphNode::Do { action, .. } => by.entry(action.clone()).or_default().push(format!("{}.nodes.{nid}", t.id)),
                    GraphNode::Branch { .. } => {}
                }
            }
        }
        ActionCompatibility { rows: by.into_iter().map(|(action, used_at)| CompatRow { supported: vocab.has_action(&action), action, used_at }).collect() }
    }

    /// The table as Markdown, for the setup review and the AAR.
    pub fn markdown(&self) -> String {
        let mut s = String::from("| Action | In this engine | Used at |\n|---|---|---|\n");
        for r in &self.rows {
            s.push_str(&format!("| `{}` | {} | {} |\n", r.action, if r.supported { "yes" } else { "**no**: falls back" }, r.used_at.join(", ")));
        }
        s
    }
}

/// "./templates/core/tank.json" → "core/tank".
pub fn template_id_from_path(path: &str) -> Option<String> {
    let norm = path.replace('\\', "/");
    let rest = if let Some(r) = norm.strip_prefix("templates/") {
        r
    } else {
        let i = norm.find("/templates/")?;
        &norm[i + "/templates/".len()..]
    };
    let id = rest.strip_suffix(".json")?;
    (!id.is_empty()).then(|| id.to_owned())
}

/// Validate every file and build the registry. Fails once, listing every
/// problem in every file, rather than stopping at the first.
pub fn build_registry(files: &TemplateFiles, vocab: &Vocabulary, mode: LoadMode) -> Result<Registry, LoadError> {
    let mut problems = Vec::new();
    let mut templates: BTreeMap<String, Template> = BTreeMap::new();
    for (path, raw) in files {
        let expected = template_id_from_path(path);
        let found: Vec<Problem> = validate_template(raw, Some(vocab));
        let blocking: Vec<Problem> = found.into_iter().filter(|p| mode == LoadMode::Strict || !p.is_unknown_action()).collect();
        if !blocking.is_empty() {
            problems.extend(blocking.into_iter().map(|p| p.in_file(path)));
            continue;
        }
        let t: Template = match serde_json::from_value(raw.clone()) {
            Ok(t) => t,
            Err(e) => {
                problems.push(Problem { at: path.clone(), kind: ProblemKind::Parse { message: e.to_string() } });
                continue;
            }
        };
        match expected {
            Some(exp) if exp != t.id => problems.push(Problem { at: path.clone(), kind: ProblemKind::IdPathMismatch { id: t.id.clone(), expected: exp } }),
            _ if templates.contains_key(&t.id) => problems.push(Problem { at: path.clone(), kind: ProblemKind::DuplicateTemplate { id: t.id.clone() } }),
            _ => {
                templates.insert(t.id.clone(), t);
            }
        }
    }
    if problems.is_empty() {
        problems.extend(validate_registry(&templates));
    }
    if !problems.is_empty() {
        return Err(LoadError { problems });
    }
    Ok(Registry::from_validated(templates, vocab.clone()))
}

/// Parse JSON text files into a [`TemplateFiles`] map.
pub fn parse_files<'a>(texts: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<TemplateFiles, LoadError> {
    let mut files = TemplateFiles::new();
    let mut problems = Vec::new();
    for (path, text) in texts {
        match serde_json::from_str(text) {
            Ok(v) => {
                files.insert(path.to_owned(), v);
            }
            Err(e) => problems.push(Problem { at: path.to_owned(), kind: ProblemKind::Parse { message: e.to_string() } }),
        }
    }
    if problems.is_empty() {
        Ok(files)
    } else {
        Err(LoadError { problems })
    }
}

/// Every template shipped with this build, compiled in.
pub fn builtin_files() -> TemplateFiles {
    // The shipped files are checked by the tests, so a parse failure here is a build defect.
    parse_files(BUILTIN.iter().copied()).unwrap_or_else(|e| panic!("shipped behaviour templates do not parse: {e}"))
}

/// Read every `*.json` under `dir` (a `templates` folder), keyed as `templates/<kind>/<name>.json`.
pub fn load_dir(dir: &Path) -> Result<TemplateFiles, LoadError> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Result<String, String>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            out.push((dir.display().to_string(), Err("cannot read directory".into())));
            return;
        };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(base, &p, out);
            } else if p.extension().is_some_and(|e| e == "json") {
                let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                out.push((format!("templates/{rel}"), std::fs::read_to_string(&p).map_err(|e| e.to_string())));
            }
        }
    }
    let mut raw = Vec::new();
    walk(dir, dir, &mut raw);
    let mut problems = Vec::new();
    let mut texts = Vec::new();
    for (path, r) in raw {
        match r {
            Ok(t) => texts.push((path, t)),
            Err(message) => problems.push(Problem { at: path, kind: ProblemKind::Parse { message } }),
        }
    }
    if !problems.is_empty() {
        return Err(LoadError { problems });
    }
    parse_files(texts.iter().map(|(p, t)| (p.as_str(), t.as_str())))
}

/// The shipped templates (`bgws-rs/data/templates/core`), copied unchanged from the TypeScript build.
const BUILTIN: &[(&str, &str)] = &[
    ("./templates/core/aircraft.json", include_str!("../../../data/templates/core/aircraft.json")),
    ("./templates/core/armour.json", include_str!("../../../data/templates/core/armour.json")),
    ("./templates/core/artillery.json", include_str!("../../../data/templates/core/artillery.json")),
    ("./templates/core/atgmTeam.json", include_str!("../../../data/templates/core/atgmTeam.json")),
    ("./templates/core/coastal.json", include_str!("../../../data/templates/core/coastal.json")),
    ("./templates/core/convoy.json", include_str!("../../../data/templates/core/convoy.json")),
    ("./templates/core/cyber.json", include_str!("../../../data/templates/core/cyber.json")),
    ("./templates/core/default.json", include_str!("../../../data/templates/core/default.json")),
    ("./templates/core/drone.json", include_str!("../../../data/templates/core/drone.json")),
    ("./templates/core/droneTeam.json", include_str!("../../../data/templates/core/droneTeam.json")),
    ("./templates/core/engineer.json", include_str!("../../../data/templates/core/engineer.json")),
    ("./templates/core/ewEsm.json", include_str!("../../../data/templates/core/ewEsm.json")),
    ("./templates/core/ewJammer.json", include_str!("../../../data/templates/core/ewJammer.json")),
    ("./templates/core/gbad.json", include_str!("../../../data/templates/core/gbad.json")),
    ("./templates/core/hq.json", include_str!("../../../data/templates/core/hq.json")),
    ("./templates/core/ifv.json", include_str!("../../../data/templates/core/ifv.json")),
    ("./templates/core/infantry.json", include_str!("../../../data/templates/core/infantry.json")),
    ("./templates/core/mortar.json", include_str!("../../../data/templates/core/mortar.json")),
    ("./templates/core/neutral.json", include_str!("../../../data/templates/core/neutral.json")),
    ("./templates/core/node.json", include_str!("../../../data/templates/core/node.json")),
    ("./templates/core/recce.json", include_str!("../../../data/templates/core/recce.json")),
    ("./templates/core/satellite.json", include_str!("../../../data/templates/core/satellite.json")),
    ("./templates/core/sensor.json", include_str!("../../../data/templates/core/sensor.json")),
    ("./templates/core/submarine.json", include_str!("../../../data/templates/core/submarine.json")),
    ("./templates/core/warship.json", include_str!("../../../data/templates/core/warship.json")),
    ("./templates/core/world.json", include_str!("../../../data/templates/core/world.json")),
];
