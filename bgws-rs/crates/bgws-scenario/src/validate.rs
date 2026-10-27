//! Loading and validating version-4 scenarios (§14.3, §17 "loader
//! fuzzing"): every loader returns typed errors and never panics; a loaded
//! scenario is checked against the catalogue — unknown platforms, duplicate
//! ids, dangling references, positions outside the theatre, a scope the
//! engine cannot run — and every problem is returned at once.

use crate::convert::{convert_v3, ConvertError, ConvertOptions};
use crate::scope::{effective_scope, ScopeError};
use crate::v3::{is_builtin_template, parse_v3, V3_FORMAT};
use crate::v4::*;
use bgws_catalogue::Catalogue;
use bgws_core::Side;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

/// Why a file could not be read as a scenario.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum LoadError {
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("not JSON: {0}")]
    Json(String),
    #[error("not a scenario file (format is `{0}`)")]
    Format(String),
    #[error("scenario version {found} is not supported (this reader takes {V4_FORMAT} 4, and {V3_FORMAT} 1–3)")]
    Version { found: String },
    #[error("the v3 file is invalid: {}", .0.join("; "))]
    V3(Vec<String>),
    #[error("the scenario does not match the v4 format: {0}")]
    Shape(String),
    #[error("converting from v3: {0}")]
    Convert(#[from] ConvertError),
}

/// A problem in a loaded scenario.
#[derive(Clone, Debug, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "camelCase")]
pub enum ScenarioError {
    #[error("{what} id `{id}` is used twice")]
    DuplicateId { what: String, id: String },
    #[error("force `{id}`: unknown platform `{platform}`")]
    UnknownPlatform { id: String, platform: String },
    #[error("`{id}`: position ({lat}, {lon}) is outside the theatre")]
    OutOfTheatre { id: String, lat: f64, lon: f64 },
    #[error("`{id}`: position ({lat}, {lon}, {h}) is not a valid WGS84 position")]
    BadPosition { id: String, lat: f64, lon: f64, h: f64 },
    #[error("{what} refers to unknown force or HQ `{target}`")]
    UnknownReference { what: String, target: String },
    #[error("behaviours: force `{id}` uses unknown template `{template}`")]
    UnknownTemplate { id: String, template: String },
    #[error("scope: {0}")]
    Scope(ScopeError),
    #[error("`{field}`: {message}")]
    Invalid { field: String, message: String },
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> ScenarioError {
    ScenarioError::Invalid { field: field.into(), message: message.into() }
}

impl ScenarioV4 {
    /// Read a v4 document (no conversion).
    pub fn from_json(text: &str) -> Result<ScenarioV4, LoadError> {
        let v: Value = serde_json::from_str(text).map_err(|e| LoadError::Json(e.to_string()))?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<ScenarioV4, LoadError> {
        let format = v.get("format").and_then(Value::as_str).unwrap_or("").to_owned();
        if format != V4_FORMAT {
            return Err(LoadError::Format(format));
        }
        match v.get("version").and_then(Value::as_u64) {
            Some(4) => {}
            _ => return Err(LoadError::Version { found: v.get("version").map(|x| x.to_string()).unwrap_or_else(|| "none".into()) }),
        }
        serde_json::from_value(v).map_err(|e| LoadError::Shape(e.to_string()))
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), LoadError> {
        std::fs::write(path, self.to_json_pretty() + "\n").map_err(|e| LoadError::Io { path: path.display().to_string(), message: e.to_string() })
    }
}

/// Read any scenario file — v4, or TS v1–v3 converted with the default
/// options — from JSON text. Never panics.
pub fn load_any(text: &str, catalogue: Option<&Catalogue>) -> Result<ScenarioV4, LoadError> {
    let v: Value = serde_json::from_str(text).map_err(|e| LoadError::Json(e.to_string()))?;
    load_value(v, catalogue, &ConvertOptions::default())
}

/// As `load_any`, from a parsed value, with conversion options.
pub fn load_value(v: Value, catalogue: Option<&Catalogue>, opts: &ConvertOptions) -> Result<ScenarioV4, LoadError> {
    match v.get("format").and_then(Value::as_str) {
        Some(V4_FORMAT) => ScenarioV4::from_value(v),
        Some(V3_FORMAT) => {
            let v3 = parse_v3(&v).map_err(LoadError::V3)?;
            Ok(convert_v3(&v3, catalogue, opts)?)
        }
        Some(other) => Err(LoadError::Format(other.to_owned())),
        None => Err(LoadError::Format(String::new())),
    }
}

/// Read a scenario file from disk (any version).
pub fn load_path(path: &Path, catalogue: Option<&Catalogue>) -> Result<ScenarioV4, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Io { path: path.display().to_string(), message: e.to_string() })?;
    load_any(&text, catalogue)
}

fn finite_position(g: bgws_core::Geodetic) -> bool {
    g.lat.is_finite() && g.lon.is_finite() && g.h.is_finite() && g.lat.abs() <= 90.0 && g.lon.abs() <= 180.0 && g.h > -12_000.0 && g.h < 5.0e7
}

/// Every problem in a scenario, checked against a catalogue.
pub fn validate(s: &ScenarioV4, catalogue: &Catalogue) -> Vec<ScenarioError> {
    let mut out = Vec::new();
    if s.format != V4_FORMAT || s.version != V4_VERSION {
        out.push(invalid("format", format!("expected {V4_FORMAT} version {V4_VERSION}")));
    }
    if s.name.trim().is_empty() {
        out.push(invalid("name", "a scenario needs a name"));
    }
    out.extend(effective_scope(&s.scope).errors.into_iter().map(ScenarioError::Scope));
    if let Err(e) = s.start_unix_s() {
        out.push(invalid("startUtc", e.to_string()));
    }
    // Forces: ids, platforms, counts, positions.
    let mut ids = BTreeSet::new();
    for f in &s.forces {
        if f.id.is_empty() {
            out.push(invalid("forces", "a force has an empty id"));
        } else if !ids.insert(f.id.as_str()) {
            out.push(ScenarioError::DuplicateId { what: "force".into(), id: f.id.clone() });
        }
        if !catalogue.has(&f.platform) {
            out.push(ScenarioError::UnknownPlatform { id: f.id.clone(), platform: f.platform.clone() });
        }
        if f.count == 0 {
            out.push(invalid(format!("forces.{}.count", f.id), "must be at least 1"));
        }
        if f.side == Side::Neutral && f.layer == Layer::Unit {
            out.push(invalid(format!("forces.{}.side", f.id), "a land unit must be blue or red"));
        }
    }
    let mut hq_ids = BTreeSet::new();
    for h in &s.command.hqs {
        if !hq_ids.insert(h.id.as_str()) || ids.contains(h.id.as_str()) {
            out.push(ScenarioError::DuplicateId { what: "HQ".into(), id: h.id.clone() });
        }
    }
    let known = |id: &str| ids.contains(id) || hq_ids.contains(id);
    let refer = |what: String, target: &str, out: &mut Vec<ScenarioError>| {
        if !known(target) {
            out.push(ScenarioError::UnknownReference { what, target: target.to_owned() });
        }
    };
    for f in &s.forces {
        if let Some(p) = &f.parent {
            refer(format!("force `{}` parent", f.id), p, &mut out);
        }
        if let Some(t) = &f.task {
            for r in t.references() {
                refer(format!("force `{}` task", f.id), r, &mut out);
            }
        }
    }
    for h in &s.command.hqs {
        refer(format!("HQ `{}` post", h.id), &h.post_id, &mut out);
        for sub in &h.subordinates {
            refer(format!("HQ `{}` subordinate", h.id), sub, &mut out);
        }
        for r in h.parent.iter().chain(h.alternate.iter()) {
            refer(format!("HQ `{}`", h.id), r, &mut out);
        }
    }
    for (i, v) in s.victory.iter().enumerate() {
        for r in v.references() {
            refer(format!("victory[{i}]"), r, &mut out);
        }
        if let VictoryCondition::Breakpoint { pct, .. } = v {
            if !(10.0..=100.0).contains(pct) {
                out.push(invalid(format!("victory[{i}].pct"), "breakpoint must be 10–100"));
            }
        }
    }
    for id in s.emitters.forces.keys() {
        refer("emitters.forces".into(), id, &mut out);
    }
    // Behaviours: assigned forces exist; templates are built in or bundled.
    let bundled: BTreeSet<&str> = s.behaviours.templates.iter().filter_map(|t| t.get("id")?.as_str()).collect();
    for (id, t) in &s.behaviours.assign {
        refer("behaviours".into(), id, &mut out);
        if !is_builtin_template(t) && !bundled.contains(t.as_str()) {
            out.push(ScenarioError::UnknownTemplate { id: id.clone(), template: t.clone() });
        }
    }
    // Positions: valid, and inside the theatre.
    if !finite_position(s.theatre.origin) {
        let o = s.theatre.origin;
        out.push(ScenarioError::BadPosition { id: "theatre.origin".into(), lat: o.lat, lon: o.lon, h: o.h });
    }
    for (id, p) in s.positions() {
        if !finite_position(p) {
            out.push(ScenarioError::BadPosition { id, lat: p.lat, lon: p.lon, h: p.h });
        } else if let Some(b) = s.theatre.bounds {
            if !b.contains(p) {
                out.push(ScenarioError::OutOfTheatre { id, lat: p.lat, lon: p.lon });
            }
        }
    }
    for (i, o) in s.world.obstacles.iter().enumerate() {
        if !(o.width_m > 0.0 && o.depth_m > 0.0) {
            out.push(invalid(format!("world.obstacles[{i}]"), "widthM and depthM must be positive"));
        }
    }
    for (i, e) in s.events.iter().enumerate() {
        if e.at.is_nan() || e.at < 0.0 {
            out.push(invalid(format!("events[{i}].at"), "must be at or after the start"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage_with_typed_errors() {
        assert!(matches!(load_any("{", None), Err(LoadError::Json(_))));
        assert!(matches!(load_any("[]", None), Err(LoadError::Format(_))));
        assert!(matches!(load_any(r#"{"format": "x"}"#, None), Err(LoadError::Format(_))));
        assert!(matches!(load_any(r#"{"format": "bgws.scenario", "version": 9}"#, None), Err(LoadError::Version { .. })));
        assert!(matches!(load_any(r#"{"format": "bgws.scenario", "version": 4}"#, None), Err(LoadError::Shape(_))));
        assert!(matches!(load_any(r#"{"format": "bgws.realtime.scenario", "version": 3}"#, None), Err(LoadError::V3(_))));
    }
}
