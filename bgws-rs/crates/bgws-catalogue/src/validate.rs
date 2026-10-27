//! Catalogue validation (§14.1: "validated: units, ranges, references (a
//! mount's munition exists, a radio's band is in the bands table)"), and the
//! TS `catalogueProblems` checks it ports. Every problem is a typed error,
//! and all of them are returned at once, as the TS check listed every
//! problem in words.

use crate::json::walk_numbers;
use crate::types::{Domain, MoverKind, Platform, SensorKind, WeaponKind};
use crate::units::field_unit;
use crate::Catalogue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "camelCase")]
pub enum CatalogueError {
    #[error("{path}: cannot read: {message}")]
    Io { path: String, message: String },
    #[error("{file}: not valid JSON: {message}")]
    Parse { file: String, message: String },
    #[error("{file}: {message}")]
    Layout { file: String, message: String },
    #[error("{what} `{id}` is defined twice")]
    Duplicate { what: String, id: String },
    #[error("platform `{id}`: unknown class `{class}`")]
    UnknownClass { id: String, class: String },
    #[error("`{id}`: unknown parent `{parent}`")]
    UnknownParent { id: String, parent: String },
    #[error("`{id}`: inheritance cycle ({chain})")]
    Cycle { id: String, chain: String },
    #[error("{what} `{id}`: {message}")]
    Shape { what: String, id: String, message: String },
    #[error("`{id}`: {field} = {value} {unit} is outside [{min}, {max}]")]
    OutOfRange { id: String, field: String, value: f64, unit: String, min: f64, max: f64 },
    #[error("platform `{id}`: mount `{mount}` fires unknown munition `{munition}`")]
    UnknownMunition { id: String, mount: String, munition: String },
    #[error("platform `{id}`: stores hold unknown munition `{munition}`")]
    UnknownAmmo { id: String, munition: String },
    #[error("`{id}`: {what} names band `{band}`, which is not in the bands table")]
    UnknownBand { id: String, what: String, band: String },
    #[error("`{id}`: {what} at {freq_hz} Hz is outside band `{band}`")]
    FrequencyOutsideBand { id: String, what: String, band: String, freq_hz: f64 },
    #[error("platform `{id}`: carries unknown archetype `{archetype}`")]
    UnknownCarried { id: String, archetype: String },
    #[error("archetype `{archetype}` that the TS engine knows has no record")]
    MissingArchetype { archetype: String },
    #[error("alias `{alias}` of `{id}` clashes with another id or alias")]
    AliasClash { id: String, alias: String },
    #[error("`{id}`: {message}")]
    Inconsistent { id: String, message: String },
}

/// A list of catalogue errors, for `Result`s.
#[derive(Clone, Debug, PartialEq, Error, Serialize, Deserialize)]
#[error("{} catalogue problem(s): {}", .0.len(), .0.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "))]
pub struct CatalogueErrors(pub Vec<CatalogueError>);

fn inconsistent(id: &str, message: impl Into<String>) -> CatalogueError {
    CatalogueError::Inconsistent { id: id.to_owned(), message: message.into() }
}

/// Every number in a value checked against the units table.
fn range_check(id: &str, v: &serde_json::Value, out: &mut Vec<CatalogueError>) {
    walk_numbers(v, &mut Vec::new(), &mut |path, x| {
        if !x.is_finite() {
            out.push(CatalogueError::OutOfRange { id: id.into(), field: path.join("."), value: x, unit: String::new(), min: f64::NAN, max: f64::NAN });
            return;
        }
        if let Some(u) = field_unit(path) {
            if x < u.min || x > u.max {
                out.push(CatalogueError::OutOfRange { id: id.into(), field: path.join("."), value: x, unit: u.unit.into(), min: u.min, max: u.max });
            }
        }
    });
}

fn check_band(c: &Catalogue, id: &str, what: &str, band: &str, freq: f64, out: &mut Vec<CatalogueError>) {
    match c.band(band) {
        None => out.push(CatalogueError::UnknownBand { id: id.into(), what: what.into(), band: band.into() }),
        Some(b) => {
            if freq > 0.0 && !b.contains(freq) {
                out.push(CatalogueError::FrequencyOutsideBand { id: id.into(), what: what.into(), band: band.into(), freq_hz: freq });
            }
        }
    }
}

/// The checks on one resolved platform.
pub fn platform_problems(c: &Catalogue, r: &Platform) -> Vec<CatalogueError> {
    let mut out = Vec::new();
    let id = r.id.as_str();
    if let Ok(v) = serde_json::to_value(r) {
        range_check(id, &v, &mut out);
    }
    // The TS catalogueProblems, ported.
    if r.rationale.is_empty() {
        out.push(inconsistent(id, "no rationale"));
    }
    if !r.template.starts_with("core/") && !r.template.starts_with("custom/") {
        out.push(inconsistent(id, format!("template `{}` is not a core template", r.template)));
    }
    if r.body.count < 1 {
        out.push(inconsistent(id, "body.count must be at least 1"));
    }
    if r.cost_usd <= 0.0 {
        out.push(inconsistent(id, "costUsd must be positive"));
    }
    let m = &r.mover;
    if m.kind != MoverKind::Static && m.max_kmh <= 0.0 {
        out.push(inconsistent(id, "a mover that moves needs a speed"));
    }
    if m.cruise_kmh > m.max_kmh {
        out.push(inconsistent(id, "cruise speed above best speed"));
    }
    if m.kind.is_ground() && (m.cross_country_kmh > m.road_kmh || m.road_kmh > m.max_kmh + 1e-9) {
        out.push(inconsistent(id, "speeds must run cross-country ≤ road ≤ max"));
    }
    if r.domain == Domain::Air && m.ceiling_m <= 0.0 {
        out.push(inconsistent(id, "an aircraft needs a ceiling"));
    }
    if r.domain == Domain::Space && m.orbit_alt_km <= 100.0 {
        out.push(inconsistent(id, "a satellite needs an orbit above 100 km"));
    }
    if r.domain == Domain::Subsurface && m.max_depth_m <= 0.0 {
        out.push(inconsistent(id, "a submarine needs a maximum depth"));
    }
    if r.domain == Domain::Land && !matches!(m.kind, MoverKind::Foot | MoverKind::Static) && m.fuel_capacity_l <= 0.0 && m.battery_wh <= 0.0 {
        out.push(inconsistent(id, "a land vehicle needs fuel or a battery"));
    }
    if m.fuel_capacity_l > 0.0 && m.burn_moving_lph <= 0.0 && !matches!(m.kind, MoverKind::Static | MoverKind::Orbit) {
        out.push(inconsistent(id, "carries fuel but burns none moving"));
    }
    let a = &r.body.armour;
    if a.ke.side > a.ke.front + 1e-9 || a.ke.rear > a.ke.front + 1e-9 {
        out.push(inconsistent(id, "side or rear KE armour thicker than the front"));
    }
    if !r.signatures.rcs.contains_key("x") {
        out.push(inconsistent(id, "signatures.rcs needs an X-band entry"));
    }
    for w in r.signatures.acoustic.water_db.windows(2).chain(r.signatures.acoustic.air_dba.windows(2)) {
        if w[1].kmh < w[0].kmh {
            out.push(inconsistent(id, "acoustic table not in ascending speed"));
        }
    }
    if !(1..=8).contains(&r.crew.quality) {
        out.push(inconsistent(id, "crew.quality must be 1–8"));
    }
    // Sensors.
    let mut seen = BTreeSet::new();
    for s in &r.sensors {
        if !seen.insert(s.id.as_str()) {
            out.push(CatalogueError::Duplicate { what: format!("sensor on `{id}`"), id: s.id.clone() });
        }
        if s.range_max_m <= 0.0 || s.range_max_m <= s.range_min_m {
            out.push(inconsistent(id, format!("sensor {}: no usable range", s.id)));
        }
        if s.for_deg <= 0.0 || s.for_deg > 360.0 {
            out.push(inconsistent(id, format!("sensor {}: field of regard outside 0–360", s.id)));
        }
        check_band(c, id, &format!("sensor {}", s.id), &s.band, s.radar.map(|x| x.freq_hz).unwrap_or(0.0), &mut out);
        if matches!(s.kind, SensorKind::Radar | SensorKind::Cbr) && s.radar.is_none() {
            out.push(inconsistent(id, format!("sensor {}: a radar needs radar parameters", s.id)));
        }
        if matches!(s.kind, SensorKind::SonarActive | SensorKind::SonarPassive) && s.sonar.is_none() {
            out.push(inconsistent(id, format!("sensor {}: a sonar needs sonar parameters", s.id)));
        }
        if matches!(s.kind, SensorKind::Esm | SensorKind::Rwr) && s.esm.is_none() {
            out.push(inconsistent(id, format!("sensor {}: an ESM receiver needs ESM parameters", s.id)));
        }
        if s.kind.is_imaging_eo() && s.optics.is_none() {
            out.push(inconsistent(id, format!("sensor {}: an imager needs optics", s.id)));
        }
        if let Some(o) = &s.optics {
            if !(o.n50.detect < o.n50.recognise && o.n50.recognise < o.n50.identify) {
                out.push(inconsistent(id, format!("sensor {}: N50 must rise detect < recognise < identify", s.id)));
            }
        }
        if let Some(e) = &s.esm {
            if e.freq_max_hz <= e.freq_min_hz {
                out.push(inconsistent(id, format!("sensor {}: ESM frequency range empty", s.id)));
            }
        }
    }
    // Mounts and their munitions.
    for mt in &r.mounts {
        if mt.munitions.is_empty() {
            out.push(inconsistent(id, format!("mount {}: fires nothing", mt.id)));
        }
        for mu in &mt.munitions {
            if c.munition(mu).is_none() {
                out.push(CatalogueError::UnknownMunition { id: id.into(), mount: mt.id.clone(), munition: mu.clone() });
            }
        }
        if mt.kind == WeaponKind::Vls {
            let loaded: f64 = mt.munitions.iter().filter_map(|m| r.stores.ammo.get(m)).sum();
            if loaded > mt.magazine as f64 {
                out.push(inconsistent(id, format!("mount {}: {loaded} rounds loaded in {} cells", mt.id, mt.magazine)));
            }
        }
    }
    for mu in r.stores.ammo.keys() {
        if c.munition(mu).is_none() {
            out.push(CatalogueError::UnknownAmmo { id: id.into(), munition: mu.clone() });
        }
    }
    // Radios and emitters: their bands.
    for ra in &r.radios {
        check_band(c, id, &format!("radio {}", ra.id), &ra.band, ra.freq_hz, &mut out);
        if ra.power_w < 0.0 || ra.data_rate_bps <= 0.0 {
            out.push(inconsistent(id, format!("radio {}: needs power and a data rate", ra.id)));
        }
    }
    for e in &r.emitters {
        check_band(c, id, &format!("emitter {}", e.id), &e.band, e.freq_hz, &mut out);
        for j in &e.jams {
            check_band(c, id, &format!("emitter {} jams", e.id), j, 0.0, &mut out);
        }
        for j in e.jam_radii_m.keys() {
            check_band(c, id, &format!("emitter {} jam radius", e.id), j, 0.0, &mut out);
        }
    }
    for b in r.signatures.rcs.keys() {
        check_band(c, id, "signatures.rcs", b, 0.0, &mut out);
    }
    for sub in r.carried.archetypes() {
        if !c.has(sub) {
            out.push(CatalogueError::UnknownCarried { id: id.into(), archetype: sub.into() });
        }
    }
    out
}

/// Every problem in a catalogue.
pub fn catalogue_problems(c: &Catalogue) -> Vec<CatalogueError> {
    let mut out = Vec::new();
    for b in c.bands() {
        if b.f_max_hz < b.f_min_hz {
            out.push(inconsistent(&b.id, "band upper edge below its lower edge"));
        }
    }
    for m in c.munitions() {
        if let Ok(v) = serde_json::to_value(m) {
            range_check(&m.id, &v, &mut out);
        }
        if m.max_range_m <= m.min_range_m {
            out.push(inconsistent(&m.id, "max range not above min range"));
        }
        if m.flight != crate::munition::FlightProfile::None && m.speed_ms <= 0.0 {
            out.push(inconsistent(&m.id, "flies but has no speed"));
        }
        if m.cost_usd <= 0.0 {
            out.push(inconsistent(&m.id, "no cost"));
        }
        if m.targets.is_empty() {
            out.push(inconsistent(&m.id, "no target domain"));
        }
        if m.penetration.windows(2).any(|w| w[1].range_m < w[0].range_m) {
            out.push(inconsistent(&m.id, "penetration table not in ascending range"));
        }
    }
    for p in c.platforms() {
        out.extend(platform_problems(c, p));
    }
    for a in crate::archetypes::TS_ARCHETYPES.iter().chain(crate::archetypes::TS_PLATFORM_KEYS) {
        if !c.has(a) {
            out.push(CatalogueError::MissingArchetype { archetype: (*a).to_owned() });
        }
    }
    out
}
