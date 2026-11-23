//! Records written by the asset-catalogue importer (`data/catalogue/imported.json`,
//! from `data/catalogue/csv/`; see docs/research/catalogue-csv-assessment.md).
//!
//! The importer writes the CSVs' own vocabulary, and the file is generated, so
//! it is read as it is and adapted here rather than edited by hand. A catalogue
//! file is an import file when it has a `summary` section (the importer's
//! counts).
//!
//! **Platforms** are overlays on a parent record. Each is adapted to the
//! engine's schema:
//!
//! - armour by `front / turret / side / rear / roof` becomes the engine's
//!   `front / side / rear / top`, with the turret front as the `turret` zone;
//! - `eraFit` (a named kit) becomes `era` through [`ERA_FITS`];
//! - `apsFit` becomes an `aps` through [`APS_FITS`];
//! - `figures` (each figure's row-level source and grade) becomes `sources`;
//! - when the catalogue already has a record for the same CSV row (the id
//!   without the importer's prefix, from an earlier import), that record is the
//!   parent, so the overlay keeps its weapons and only updates what the
//!   new row sources.
//!
//! A fit name not in the tables is an error, so a new kit in the CSV is a
//! decision, not a silent default.
//!
//! **Munitions** are not engine munitions yet: the importer's `kind` is coarse
//! (`he` covers mortar bombs, glide bombs and cruise missiles; `sam` covers
//! air-to-air missiles), and the rows have no flight, guidance or range. They
//! are kept as typed [`ImportedMunition`]s (penetration against range, warhead,
//! what it does against ERA, with source and grade) for the weapon fits to
//! draw on.

use crate::munition::PenPoint;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// An ERA or armour kit named by the CSV's `era_fit`: whether it is explosive
/// reactive armour (what the engine's `body.era` means), and why.
/// `Some("heavy")` (against long rods too: `eraKind` heavy), `Some("light")`
/// (against shaped charges only) or `None` (not ERA).
pub const ERA_FITS: &[(&str, Option<&str>, &str)] = &[
    ("kontakt1", Some("light"), "Kontakt-1: light ERA, against shaped charges"),
    ("kontakt5", Some("heavy"), "Kontakt-5: heavy ERA, against long rods too"),
    ("relikt", Some("heavy"), "Relikt: heavy ERA, against long rods too"),
    ("malachit", Some("heavy"), "Malachit: heavy ERA, against long rods too"),
    ("rafael_era", Some("light"), "Rafael ERA kit: light, against shaped charges"),
    ("romor_lfp", Some("light"), "ROMOR-A on the lower front plate: light ERA, against shaped charges"),
    ("brat_optional", None, "BRAT ERA is an optional kit: off unless a scenario fits it"),
    ("slat", None, "slat armour: a cage, not ERA (not modelled)"),
    ("bar_cage", None, "bar armour: a cage, not ERA (not modelled)"),
    ("appliqué", None, "passive appliqué armour, not ERA"),
    ("epsom", None, "Epsom: Challenger 3's passive modular armour, not ERA"),
];

/// An active protection system named by the CSV's `aps_fit`: the kind it
/// becomes, or `None` when it is not fitted as standard. The engine's
/// intercept chance is the figure `direct.aps.p_intercept`; `pk` and `charges`
/// here are [default] and not read by the simulation.
pub const APS_FITS: &[(&str, Option<&str>, &str)] = &[
    ("trophy", Some("hard"), "Trophy hard-kill APS"),
    ("afganit", Some("hard"), "Afganit hard-kill APS"),
    ("strikeshield", Some("hard"), "StrikeShield hard-kill APS"),
    ("trophy_optional", None, "Trophy is an optional kit: off unless a scenario fits it"),
];

const APS_DEFAULT_PK: f64 = 0.8;
const APS_DEFAULT_CHARGES: u32 = 4;

/// Whether a catalogue file is the importer's output.
pub fn is_import_file(obj: &Map<String, Value>) -> bool {
    obj.contains_key("summary")
}

/// Adapt one imported platform record in place. The error says what could not
/// be mapped.
pub fn adapt_platform(v: &mut Value) -> Result<(), String> {
    let Some(rec) = v.as_object_mut() else { return Err("a platform must be an object".into()) };
    rec.insert("provenance".into(), json!("imported"));
    let mut notes: BTreeMap<String, String> = BTreeMap::new();
    if let Some(Value::Object(figs)) = rec.remove("figures") {
        for (path, f) in figs {
            let src = f.get("source").and_then(Value::as_str).unwrap_or("");
            let grade = f.get("grade").and_then(Value::as_str).unwrap_or("?");
            let path = match path.as_str() {
                "body.eraFit" => "body.era".to_owned(),
                "body.apsFit" => "body.aps".to_owned(),
                _ => path,
            };
            notes.insert(path, format!("{src} (grade {grade})"));
        }
    }
    if let Some(body) = rec.get_mut("body").and_then(Value::as_object_mut) {
        if let Some(armour) = body.get_mut("armour").and_then(Value::as_object_mut) {
            adapt_armour(armour);
        }
        if let Some(fit) = body.remove("eraFit") {
            let name = fit.as_str().ok_or("`eraFit` must be a string")?;
            let (_, kind, why) = ERA_FITS.iter().find(|(n, _, _)| *n == name).ok_or_else(|| format!("unknown ERA fit `{name}`: add it to imported::ERA_FITS"))?;
            body.insert("era".into(), json!(kind.is_some()));
            if let Some(k) = kind {
                body.insert("eraKind".into(), json!(k));
            }
            let e = notes.entry("body.era".into()).or_default();
            *e = if e.is_empty() { format!("{name}: {why}") } else { format!("{name}: {why}; {e}") };
        }
        if let Some(fit) = body.remove("apsFit") {
            let name = fit.as_str().ok_or("`apsFit` must be a string")?;
            let (_, kind, why) = APS_FITS.iter().find(|(n, _, _)| *n == name).ok_or_else(|| format!("unknown APS fit `{name}`: add it to imported::APS_FITS"))?;
            // Null removes an APS a parent has: the row says what is fitted.
            body.insert("aps".into(), kind.map_or(Value::Null, |k| json!({ "kind": k, "pk": APS_DEFAULT_PK, "charges": APS_DEFAULT_CHARGES })));
            let e = notes.entry("body.aps".into()).or_default();
            let pk = if kind.is_some() { "; pk and charges [default]" } else { "" };
            *e = if e.is_empty() { format!("{name}: {why}{pk}") } else { format!("{name}: {why}{pk}; {e}") };
        }
    }
    let sources: Map<String, Value> = notes.into_iter().map(|(path, r)| (path, json!({ "kind": "imported", "ref": r }))).collect();
    if !sources.is_empty() {
        rec.insert("sources".into(), Value::Object(sources));
    }
    rec.insert(
        "rationale".into(),
        json!("Imported from the asset-catalogue CSV (data/catalogue/csv). Only the figures in `sources` come from its row, each with a row-level source; everything else, the weapons included, is the parent record's."),
    );
    Ok(())
}

/// `front / turret / side / rear / roof` → `front / side / rear / top`, the
/// turret front as the `turret` zone.
fn adapt_armour(armour: &mut Map<String, Value>) {
    for kind in ["ke", "ce"] {
        let Some(asp) = armour.get_mut(kind).and_then(Value::as_object_mut) else { continue };
        if let Some(roof) = asp.remove("roof") {
            asp.insert("top".into(), roof);
        }
        let Some(turret) = asp.remove("turret") else { continue };
        let zones = armour.entry("zones").or_insert_with(|| json!({}));
        if let Some(z) = zones.as_object_mut() {
            let t = z.entry("turret").or_insert_with(|| json!({}));
            if let Some(t) = t.as_object_mut() {
                let a = t.entry(kind).or_insert_with(|| json!({}));
                if let Some(a) = a.as_object_mut() {
                    a.insert("front".into(), turret);
                }
            }
        }
    }
}

/// Make each imported record's parent the earlier record for the same CSV
/// row, where there is one (`l7/var_11_default` → `var_11_default`).
pub fn reparent(platforms: &mut BTreeMap<String, Value>) {
    let ids: Vec<(String, String)> = platforms
        .iter()
        .filter(|(_, v)| v.get("provenance").and_then(Value::as_str) == Some("imported"))
        .filter_map(|(id, _)| {
            let bare = id.split_once('/')?.1;
            let earlier = platforms.get(bare)?;
            (earlier.get("provenance").and_then(Value::as_str) != Some("imported")).then(|| (id.clone(), bare.to_owned()))
        })
        .collect();
    for (id, bare) in ids {
        if let Some(rec) = platforms.get_mut(&id).and_then(Value::as_object_mut) {
            rec.insert("parent".into(), Value::String(bare));
        }
    }
}

pub use crate::munition::PenBasis;

/// What ERA does to the round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EraEffect {
    PrecursorDefeatsEra,
    EraDegrades,
    DefeatsEra,
}

/// A munition row from the importer: the round's own figures, not yet an
/// engine munition (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedMunition {
    pub id: String,
    pub name: String,
    /// The CSV's coarse kind: `ke`, `heat`, `he`, `atgm`, `sam`.
    pub kind: String,
    /// The weapon it is fired from (the CSV's `weapon_id`).
    pub family: String,
    #[serde(default)]
    pub tandem: bool,
    #[serde(default)]
    pub top_attack: bool,
    #[serde(default)]
    pub lethal_radius_m: Option<f64>,
    #[serde(default)]
    pub warhead_kg: Option<f64>,
    /// Penetration against range, mm on [`Self::pen_basis`].
    #[serde(default)]
    pub penetration: Vec<PenPoint>,
    #[serde(default)]
    pub pen_basis: Option<PenBasis>,
    #[serde(default)]
    pub era_effect: Option<EraEffect>,
    pub source: String,
    /// A–D, as in the CSV assessment.
    pub grade: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armour_fits_and_sources_adapt() {
        let mut v = json!({
            "id": "l7/x", "parent": "generic/mbt", "provenance": "imported",
            "body": {
                "armour": { "ke": { "front": 400, "turret": 450, "side": 80, "rear": 40, "roof": 25 }, "ce": { "front": 600, "turret": 700, "roof": 25 } },
                "eraFit": "relikt", "apsFit": "trophy_optional"
            },
            "figures": { "body.armour": { "source": "a document", "grade": "C" }, "body.eraFit": { "source": "another", "grade": "B" } }
        });
        adapt_platform(&mut v).unwrap();
        let a = &v["body"]["armour"];
        assert_eq!(a["ke"], json!({ "front": 400, "side": 80, "rear": 40, "top": 25 }));
        assert_eq!(a["zones"]["turret"]["ke"]["front"], 450);
        assert_eq!(a["zones"]["turret"]["ce"]["front"], 700);
        assert_eq!(v["body"]["era"], true);
        assert_eq!(v["body"]["eraKind"], "heavy");
        assert!(v["body"]["aps"].is_null() && v["body"].get("apsFit").is_none());
        assert!(v.get("figures").is_none());
        assert_eq!(v["sources"]["body.armour"]["ref"], "a document (grade C)");
        assert!(v["sources"]["body.era"]["ref"].as_str().unwrap().starts_with("relikt: "));
    }

    #[test]
    fn an_unknown_fit_is_an_error() {
        let mut v = json!({ "id": "l7/x", "body": { "eraFit": "mystery" } });
        assert!(adapt_platform(&mut v).unwrap_err().contains("mystery"));
    }

    #[test]
    fn reparents_onto_an_earlier_record_for_the_same_row() {
        let mut p: BTreeMap<String, Value> = BTreeMap::new();
        p.insert("var_1_default".into(), json!({ "parent": "generic/mbt" }));
        p.insert("l7/var_1_default".into(), json!({ "parent": "generic/mbt", "provenance": "imported" }));
        p.insert("l7/var_2_default".into(), json!({ "parent": "generic/ifv", "provenance": "imported" }));
        reparent(&mut p);
        assert_eq!(p["l7/var_1_default"]["parent"], "var_1_default");
        assert_eq!(p["l7/var_2_default"]["parent"], "generic/ifv");
    }
}
