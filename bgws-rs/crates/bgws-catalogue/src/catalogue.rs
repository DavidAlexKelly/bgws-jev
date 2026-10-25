//! The catalogue as the engine reads it (§14.1; ports realtime/catalogue/
//! index.ts): every record by id, every munition and band, loaded from JSON
//! files and resolved.
//!
//! **Files.** Each JSON file in `data/catalogue/` is an object with any of
//! these sections: `bands` (list), `classes` (id → partial record, with an
//! optional `$extends`), `kindDefaults` (`sensors`/`emitters`/`radios`/
//! `mounts`: kind → partial element), `munitions` (list) and `platforms`
//! (list of partial records). Keys starting with `$` are comments and
//! sources. Files are read in name order, so loading is deterministic.
//!
//! **Resolution**, for each platform:
//! 1. its class's defaults, with the class's own `$extends` chain under them
//!    (class-default numbers can be overridden as figures,
//!    `catalogue.class.<class>.<path>`);
//! 2. its `parent` record's merged JSON (recursively; `id`, `aliases` and
//!    `parent` do not inherit);
//! 3. the record itself;
//! 4. each sensor, emitter, radio and mount on top of its kind's defaults;
//! 5. the derivation rules (`derive.rs`);
//! 6. typed deserialisation into `Platform`.

use crate::derive::{derive, DeriveParams};
use crate::json::{deep_merge, get_str, strip_meta, walk_numbers};
use crate::munition::Munition;
use crate::types::{Band, Domain, Mount, Platform};
use crate::units::field_unit;
use crate::validate::{catalogue_problems, CatalogueError, CatalogueErrors};
use bgws_core::{Confidence, Figure, Figures};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The shipped catalogue data, embedded at build time.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("air.json", include_str!("../../../data/catalogue/air.json")),
    ("bands.json", include_str!("../../../data/catalogue/bands.json")),
    ("classes.json", include_str!("../../../data/catalogue/classes.json")),
    ("command.json", include_str!("../../../data/catalogue/command.json")),
    ("drones.json", include_str!("../../../data/catalogue/drones.json")),
    ("land.json", include_str!("../../../data/catalogue/land.json")),
    ("munitions.json", include_str!("../../../data/catalogue/munitions.json")),
    ("nodes.json", include_str!("../../../data/catalogue/nodes.json")),
    ("sea.json", include_str!("../../../data/catalogue/sea.json")),
    ("sourced.json", include_str!("../../../data/catalogue/sourced.json")),
    ("space.json", include_str!("../../../data/catalogue/space.json")),
];

/// The resolved catalogue.
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    platforms: BTreeMap<String, Platform>,
    aliases: BTreeMap<String, String>,
    munitions: BTreeMap<String, Munition>,
    bands: BTreeMap<String, Band>,
    classes: BTreeMap<String, Value>,
}

/// The raw sections gathered from every file.
#[derive(Default)]
struct Raw {
    bands: Vec<(String, Value)>,
    classes: BTreeMap<String, Value>,
    kind_defaults: BTreeMap<String, BTreeMap<String, Value>>,
    munitions: Vec<(String, Value)>,
    platforms: BTreeMap<String, Value>,
}

const ELEMENT_LISTS: [(&str, &str); 4] = [("sensors", "kind"), ("emitters", "kind"), ("radios", "bearer"), ("mounts", "kind")];

fn gather(files: &[(String, String)], errors: &mut Vec<CatalogueError>) -> Raw {
    let mut raw = Raw::default();
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut munition_ids = BTreeSet::new();
    let mut band_ids = BTreeSet::new();
    for (file, text) in sorted {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                errors.push(CatalogueError::Parse { file: file.clone(), message: e.to_string() });
                continue;
            }
        };
        let Some(obj) = v.as_object() else {
            errors.push(CatalogueError::Layout { file: file.clone(), message: "a catalogue file must be a JSON object".into() });
            continue;
        };
        for (section, body) in obj {
            if section.starts_with('$') {
                continue;
            }
            match section.as_str() {
                "bands" | "munitions" | "platforms" => {
                    let Some(list) = body.as_array() else {
                        errors.push(CatalogueError::Layout { file: file.clone(), message: format!("`{section}` must be a list") });
                        continue;
                    };
                    for item in list {
                        let Some(id) = item.get("id").and_then(Value::as_str) else {
                            errors.push(CatalogueError::Layout { file: file.clone(), message: format!("an entry in `{section}` has no string `id`") });
                            continue;
                        };
                        let what = &section[..section.len() - 1];
                        let fresh = match section.as_str() {
                            "bands" => band_ids.insert(id.to_owned()),
                            "munitions" => munition_ids.insert(id.to_owned()),
                            _ => !raw.platforms.contains_key(id),
                        };
                        if !fresh {
                            errors.push(CatalogueError::Duplicate { what: what.into(), id: id.into() });
                            continue;
                        }
                        match section.as_str() {
                            "bands" => raw.bands.push((id.to_owned(), item.clone())),
                            "munitions" => raw.munitions.push((id.to_owned(), item.clone())),
                            _ => {
                                raw.platforms.insert(id.to_owned(), item.clone());
                            }
                        }
                    }
                }
                "classes" => {
                    let Some(m) = body.as_object() else {
                        errors.push(CatalogueError::Layout { file: file.clone(), message: "`classes` must be an object".into() });
                        continue;
                    };
                    for (id, c) in m {
                        if id.starts_with('$') {
                            continue;
                        }
                        if raw.classes.insert(id.clone(), c.clone()).is_some() {
                            errors.push(CatalogueError::Duplicate { what: "class".into(), id: id.clone() });
                        }
                    }
                }
                "kindDefaults" => {
                    let Some(m) = body.as_object() else {
                        errors.push(CatalogueError::Layout { file: file.clone(), message: "`kindDefaults` must be an object".into() });
                        continue;
                    };
                    for (list, kinds) in m {
                        if list.starts_with('$') {
                            continue;
                        }
                        let Some(kinds) = kinds.as_object() else {
                            errors.push(CatalogueError::Layout { file: file.clone(), message: format!("`kindDefaults.{list}` must be an object") });
                            continue;
                        };
                        let slot = raw.kind_defaults.entry(list.clone()).or_default();
                        for (k, d) in kinds {
                            if !k.starts_with('$') && slot.insert(k.clone(), d.clone()).is_some() {
                                errors.push(CatalogueError::Duplicate { what: format!("kind default {list}"), id: k.clone() });
                            }
                        }
                    }
                }
                other => errors.push(CatalogueError::Layout { file: file.clone(), message: format!("unknown section `{other}`") }),
            }
        }
    }
    raw
}

/// A class's own numbers as figures: `catalogue.class.<class>.<path>`.
fn class_figures(class: &str, v: &Value) -> Vec<Figure> {
    let source = v.get("$source").and_then(Value::as_str).unwrap_or("default").to_owned();
    let confidence = match v.get("$confidence").and_then(Value::as_str) {
        Some("low") => Confidence::Low,
        Some("medium") => Confidence::Medium,
        Some("high") => Confidence::High,
        _ => Confidence::Default,
    };
    let note = v.get("$comment").and_then(Value::as_str).unwrap_or("class default").to_owned();
    let mut out = Vec::new();
    let stripped = strip_meta(v);
    object_leaves(&stripped, &mut Vec::new(), &mut |path, x| {
        if let Some(u) = field_unit(path) {
            if x >= u.min && x <= u.max {
                out.push(Figure {
                    key: format!("catalogue.class.{class}.{}", path.join(".")),
                    value: x,
                    unit: u.unit.to_owned(),
                    min: u.min,
                    max: u.max,
                    source: source.clone(),
                    confidence: if source == "default" { Confidence::Default } else { confidence },
                    note: note.clone(),
                });
            }
        }
    });
    out
}

/// Numbers reachable through objects only (array contents are not figures).
fn object_leaves<'a>(v: &'a Value, path: &mut Vec<&'a str>, f: &mut dyn FnMut(&[&'a str], f64)) {
    if let Value::Object(m) = v {
        for (k, x) in m {
            path.push(k.as_str());
            match x {
                Value::Number(n) => {
                    if let Some(x) = n.as_f64() {
                        f(path, x);
                    }
                }
                Value::Object(_) => object_leaves(x, path, f),
                _ => {}
            }
            path.pop();
        }
    }
}

/// Replace a class's own numbers with the figure values, where declared.
fn apply_class_figures(class: &str, v: &mut Value, figures: &Figures) {
    fn walk(v: &mut Value, prefix: &str, figures: &Figures) {
        if let Value::Object(m) = v {
            for (k, x) in m.iter_mut() {
                if k.starts_with('$') {
                    continue;
                }
                let key = format!("{prefix}.{k}");
                match x {
                    Value::Number(_) => {
                        if let Some(f) = figures.figure(&key) {
                            if let Some(n) = serde_json::Number::from_f64(f.value) {
                                // Keep integers as integers: counts deserialise into u32.
                                *x = if f.value.fract() == 0.0 && f.value.abs() < 9.0e15 { Value::from(f.value as i64) } else { Value::Number(n) };
                            }
                        }
                    }
                    Value::Object(_) => walk(x, &key, figures),
                    _ => {}
                }
            }
        }
    }
    walk(v, &format!("catalogue.class.{class}"), figures);
}

/// Class figures for every class in a set of files.
fn figures_of_files(files: &[(String, String)]) -> Vec<Figure> {
    let mut errs = Vec::new();
    let raw = gather(files, &mut errs);
    raw.classes.iter().flat_map(|(id, v)| class_figures(id, v)).collect()
}

/// The class-default figures of the shipped data.
pub fn builtin_class_figures() -> Vec<Figure> {
    figures_of_files(&builtin_sources())
}

fn builtin_sources() -> Vec<(String, String)> {
    BUILTIN_FILES.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()
}

fn resolve_class(id: &str, raw: &BTreeMap<String, Value>, figures: &Figures, stack: &mut Vec<String>) -> Result<Value, CatalogueError> {
    if stack.iter().any(|s| s == id) {
        return Err(CatalogueError::Cycle { id: id.into(), chain: format!("{} → {id}", stack.join(" → ")) });
    }
    let Some(own) = raw.get(id) else {
        return Err(CatalogueError::UnknownClass { id: stack.last().cloned().unwrap_or_default(), class: id.into() });
    };
    let mut own = own.clone();
    apply_class_figures(id, &mut own, figures);
    let mut base = match own.get("$extends").and_then(Value::as_str) {
        Some(parent) => {
            stack.push(id.to_owned());
            let b = resolve_class(parent, raw, figures, stack);
            stack.pop();
            b?
        }
        None => Value::Object(Map::new()),
    };
    deep_merge(&mut base, &own);
    Ok(base)
}

/// A record's JSON merged down its parent chain (before class defaults).
fn merged_record(id: &str, raw: &BTreeMap<String, Value>, memo: &mut BTreeMap<String, Value>, stack: &mut Vec<String>) -> Result<Value, CatalogueError> {
    if let Some(v) = memo.get(id) {
        return Ok(v.clone());
    }
    if stack.iter().any(|s| s == id) {
        return Err(CatalogueError::Cycle { id: id.into(), chain: format!("{} → {id}", stack.join(" → ")) });
    }
    let own = raw.get(id).cloned().unwrap_or(Value::Null);
    let mut base = match own.get("parent").and_then(Value::as_str) {
        Some(parent) => {
            if !raw.contains_key(parent) {
                return Err(CatalogueError::UnknownParent { id: id.into(), parent: parent.into() });
            }
            stack.push(id.to_owned());
            let p = merged_record(parent, raw, memo, stack);
            stack.pop();
            let mut p = p?;
            if let Some(m) = p.as_object_mut() {
                for k in ["id", "aliases", "parent"] {
                    m.remove(k);
                }
            }
            p
        }
        None => Value::Object(Map::new()),
    };
    deep_merge(&mut base, &own);
    memo.insert(id.to_owned(), base.clone());
    Ok(base)
}

impl Catalogue {
    /// The shipped catalogue with its shipped figures.
    pub fn builtin() -> Result<Catalogue, CatalogueErrors> {
        Self::from_sources(&builtin_sources())
    }

    /// The shipped catalogue with figure overrides (class defaults and derivation ratios).
    pub fn builtin_with(figures: &Figures) -> Result<Catalogue, CatalogueErrors> {
        Self::from_sources_with(&builtin_sources(), figures)
    }

    /// Every `*.json` file in a directory (not recursive), in name order.
    pub fn load_dir(dir: &Path) -> Result<Catalogue, CatalogueErrors> {
        let rd = std::fs::read_dir(dir).map_err(|e| CatalogueErrors(vec![CatalogueError::Io { path: dir.display().to_string(), message: e.to_string() }]))?;
        let mut files = Vec::new();
        let mut errors = Vec::new();
        for entry in rd {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    errors.push(CatalogueError::Io { path: dir.display().to_string(), message: e.to_string() });
                    continue;
                }
            };
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&p) {
                Ok(t) => files.push((p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), t)),
                Err(e) => errors.push(CatalogueError::Io { path: p.display().to_string(), message: e.to_string() }),
            }
        }
        if !errors.is_empty() {
            return Err(CatalogueErrors(errors));
        }
        Self::from_sources(&files)
    }

    /// From (file name, JSON text) pairs, with the default figures.
    pub fn from_sources(files: &[(String, String)]) -> Result<Catalogue, CatalogueErrors> {
        let mut list = crate::derive::figures();
        list.extend(figures_of_files(files));
        let figures = Figures::from_list(list).map_err(|e| CatalogueErrors(vec![CatalogueError::Layout { file: "(figures)".into(), message: e.to_string() }]))?;
        Self::from_sources_with(files, &figures)
    }

    /// From (file name, JSON text) pairs, reading class defaults and
    /// derivation ratios from `figures` where it declares them.
    pub fn from_sources_with(files: &[(String, String)], figures: &Figures) -> Result<Catalogue, CatalogueErrors> {
        let mut errors = Vec::new();
        let raw = gather(files, &mut errors);
        let mut cat = Catalogue::default();
        for (id, v) in &raw.bands {
            match serde_json::from_value::<Band>(v.clone()) {
                Ok(b) => {
                    cat.bands.insert(id.clone(), b);
                }
                Err(e) => errors.push(CatalogueError::Shape { what: "band".into(), id: id.clone(), message: e.to_string() }),
            }
        }
        for (id, v) in &raw.munitions {
            match serde_json::from_value::<Munition>(strip_meta(v)) {
                Ok(m) => {
                    cat.munitions.insert(id.clone(), m);
                }
                Err(e) => errors.push(CatalogueError::Shape { what: "munition".into(), id: id.clone(), message: e.to_string() }),
            }
        }
        for id in raw.classes.keys() {
            match resolve_class(id, &raw.classes, figures, &mut Vec::new()) {
                Ok(v) => {
                    cat.classes.insert(id.clone(), v);
                }
                Err(e) => errors.push(e),
            }
        }
        let params = DeriveParams::from_figures(figures);
        let mut memo = BTreeMap::new();
        for id in raw.platforms.keys() {
            match cat.resolve_platform(id, &raw, &mut memo, &params) {
                Ok(p) => {
                    cat.platforms.insert(id.clone(), p);
                }
                Err(e) => errors.push(e),
            }
        }
        // Aliases: other ids naming a record.
        for p in cat.platforms.values() {
            for a in &p.aliases {
                if cat.platforms.contains_key(a) || cat.aliases.contains_key(a) {
                    errors.push(CatalogueError::AliasClash { id: p.id.clone(), alias: a.clone() });
                } else {
                    cat.aliases.insert(a.clone(), p.id.clone());
                }
            }
        }
        if errors.is_empty() {
            Ok(cat)
        } else {
            Err(CatalogueErrors(errors))
        }
    }

    fn resolve_platform(&self, id: &str, raw: &Raw, memo: &mut BTreeMap<String, Value>, params: &DeriveParams) -> Result<Platform, CatalogueError> {
        let merged = merged_record(id, &raw.platforms, memo, &mut Vec::new())?;
        let class = get_str(&merged, &["class"]).unwrap_or("").to_owned();
        let Some(class_v) = self.classes.get(&class) else {
            return Err(CatalogueError::UnknownClass { id: id.into(), class });
        };
        let mut v = class_v.clone();
        deep_merge(&mut v, &merged);
        for (list, key) in ELEMENT_LISTS {
            let defaults = raw.kind_defaults.get(list);
            if let Some(arr) = v.get_mut(list).and_then(Value::as_array_mut) {
                for el in arr.iter_mut() {
                    let kind = el.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
                    if let Some(d) = defaults.and_then(|d| d.get(&kind)) {
                        let mut base = strip_meta(d);
                        deep_merge(&mut base, el);
                        *el = base;
                    }
                }
            }
        }
        if let Some(m) = v.as_object_mut() {
            m.insert("id".into(), Value::String(id.to_owned()));
            m.insert("class".into(), Value::String(class));
        }
        derive(&mut v, params, &self.bands);
        serde_json::from_value::<Platform>(v).map_err(|e| CatalogueError::Shape { what: "platform".into(), id: id.into(), message: e.to_string() })
    }

    /// The record with this id or alias.
    pub fn platform(&self, id: &str) -> Option<&Platform> {
        self.platforms.get(id).or_else(|| self.aliases.get(id).and_then(|real| self.platforms.get(real)))
    }

    pub fn has(&self, id: &str) -> bool {
        self.platform(id).is_some()
    }

    /// Every record, in id order.
    pub fn platforms(&self) -> impl Iterator<Item = &Platform> {
        self.platforms.values()
    }

    pub fn platforms_in(&self, domain: Domain) -> Vec<&Platform> {
        self.platforms.values().filter(|p| p.domain == domain).collect()
    }

    pub fn munition(&self, id: &str) -> Option<&Munition> {
        self.munitions.get(id)
    }

    pub fn munitions(&self) -> impl Iterator<Item = &Munition> {
        self.munitions.values()
    }

    pub fn band(&self, id: &str) -> Option<&Band> {
        self.bands.get(id)
    }

    pub fn bands(&self) -> impl Iterator<Item = &Band> {
        self.bands.values()
    }

    /// A class's resolved defaults, as JSON.
    pub fn class(&self, id: &str) -> Option<&Value> {
        self.classes.get(id)
    }

    pub fn class_ids(&self) -> impl Iterator<Item = &str> {
        self.classes.keys().map(String::as_str)
    }

    /// A record's mount by id (index.ts `mountOf`).
    pub fn mount_of(&self, platform: &str, mount: &str) -> Option<&Mount> {
        self.platform(platform)?.mount(mount)
    }

    /// The default munition a record's mount fires.
    pub fn munition_of(&self, platform: &str, mount: &str) -> Option<&Munition> {
        self.munition(self.mount_of(platform, mount)?.munitions.first()?)
    }

    /// The generic archetype a record is: itself if it is one, else the
    /// nearest one up its parent chain.
    pub fn archetype_of(&self, id: &str) -> Option<&str> {
        let mut cur = self.platform(id)?;
        for _ in 0..64 {
            if crate::archetypes::TS_ARCHETYPES.contains(&cur.id.as_str()) || cur.parent.is_none() {
                return Some(cur.id.as_str());
            }
            cur = self.platform(cur.parent.as_deref()?)?;
        }
        None
    }

    /// Every validation problem (units and ranges, references, bands,
    /// consistency, and the TS archetypes' coverage).
    pub fn validate(&self) -> Vec<CatalogueError> {
        catalogue_problems(self)
    }

    /// Every number of every record in canonical form, for a stable hash in a game record.
    pub fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0;
        for p in self.platforms.values() {
            if let Ok(b) = serde_json::to_vec(p) {
                h = bgws_core::rng::mix64(h ^ bgws_core::rng::fnv1a(&b));
            }
        }
        for m in self.munitions.values() {
            if let Ok(b) = serde_json::to_vec(m) {
                h = bgws_core::rng::mix64(h ^ bgws_core::rng::fnv1a(&b));
            }
        }
        h
    }

    /// Count of numbers in every record (used to sanity-check coverage in tests).
    pub fn number_count(&self) -> usize {
        let mut n = 0;
        for p in self.platforms.values() {
            if let Ok(v) = serde_json::to_value(p) {
                walk_numbers(&v, &mut Vec::new(), &mut |_, _| n += 1);
            }
        }
        n
    }
}
