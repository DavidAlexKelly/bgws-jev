//! Archetypes (MULTI_DOMAIN_PLAN.md W2; ports `behaviour/archetypes.ts` and
//! `model/archetypes.ts`): from what a platform is to the generic record it
//! inherits from, and the behaviour template a unit uses when nothing else
//! is assigned. The lists below are every archetype and force-list platform
//! key the TS engine can field; validation fails if any lacks a record.

use crate::Catalogue;

/// Used when nothing else is assigned (archetypes.ts `DEFAULT_TEMPLATE_ID`).
pub const DEFAULT_TEMPLATE_ID: &str = "core/default";

/// The default template for a unit of this target class (archetypes.ts).
pub fn default_template_for_class(target_class: Option<&str>) -> &'static str {
    match target_class {
        Some("armoured_vehicle") => "core/armour",
        Some("foot") => "core/infantry",
        _ => DEFAULT_TEMPLATE_ID,
    }
}

/// Every generic archetype the TS engine knows (realtime/catalogue/generic.ts).
pub const TS_ARCHETYPES: &[&str] = &[
    "generic/mbt",
    "generic/ifv",
    "generic/infantrySection",
    "generic/atgmTeam",
    "generic/recce",
    "generic/engineerSection",
    "generic/mortar120",
    "generic/sph155",
    "generic/mlrs",
    "generic/cbRadar",
    "generic/shorad",
    "generic/samMedium",
    "generic/samLong",
    "generic/ewJammer",
    "generic/esmTeam",
    "generic/droneIsrTeam",
    "generic/fpvTeamRf",
    "generic/fpvTeamFibre",
    "generic/loiterTeam",
    "generic/interceptorTeam",
    "generic/ugv",
    "generic/convoyTruck",
    "generic/medevac",
    "generic/coastalAsm",
    "generic/cp-company",
    "generic/cp-battalion",
    "generic/cp-brigade",
    "generic/cp-division",
    "generic/quadIsr",
    "generic/fpvRf",
    "generic/fpvFibre",
    "generic/loiter",
    "generic/owaDrone",
    "generic/owaDecoy",
    "generic/male",
    "generic/fighter4",
    "generic/fighter5",
    "generic/attackHelo",
    "generic/transportHelo",
    "generic/tanker",
    "generic/aew",
    "generic/mpa",
    "generic/destroyer",
    "generic/frigate",
    "generic/corvette",
    "generic/ssk",
    "generic/ssn",
    "generic/usv",
    "generic/merchant",
    "generic/satEo",
    "generic/satSar",
    "generic/satcomGeo",
    "generic/gnss",
    "generic/missileWarning",
    "generic/depot",
    "generic/railhead",
    "generic/port",
    "generic/airbase",
    "generic/cyberCell",
];

/// Every platform key a TS force list can name (rules/forceList.ts
/// `PLATFORM_SNAPSHOT`: the generated catalogue slice and the pinned rows).
pub const TS_PLATFORM_KEYS: &[&str] = &[
    "tankmodels/cn_m1a2t",
    "tankmodels/cn_ztz_96",
    "tankmodels/fr_leopard_2a5nl",
    "tankmodels/fr_leopard_2a6nl",
    "tankmodels/germ_leopard_2a5",
    "tankmodels/germ_leopard_2a5_pso",
    "tankmodels/il_merkava_mk_3_raam_segol",
    "tankmodels/il_merkava_mk_3b",
    "tankmodels/it_leopard_2a4",
    "tankmodels/it_leopard_2a7_hungary",
    "tankmodels/jp_leopard_2ri",
    "tankmodels/jp_tkx_prot",
    "tankmodels/sw_leopard_2a6nl",
    "tankmodels/sw_pvrbv_551",
    "tankmodels/uk_m1a1_aim_abrams",
    "tankmodels/us_m1a1_abrams",
    "infantry/rifle_infantry",
    "declared/mortar_section",
    "var_11_default",
    "var_27_default",
    "var_1_default",
    "var_20_default",
    "var_22_1",
    "var_23_1",
    "sec_uk_armd_inf",
    "sec_uk_javelin",
    "sec_uk_gpmg_sf",
    "sec_uk_mortar",
    "sec_ru_mr_bmp",
    "sec_ru_mr_btr",
    "sec_ru_at",
    "sec_ru_mortar",
    "sec_us_mech_sqd",
    "sec_us_stryker_sqd",
    "sec_generic_light",
];

/// What a platform's data says it is, for matching an unknown platform to an
/// archetype (model/archetypes.ts `archetypeOfSnapshot`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapshotHint {
    pub key: String,
    pub display_name: String,
    /// "armoured_vehicle", "soft_skin", "foot".
    pub target_class: String,
    /// "T", "W", "F".
    pub move_type: String,
    /// Capability kinds: "atk", "atm", "apers", "idf", "aa", "smoke".
    pub capability_kinds: Vec<String>,
    pub armour_mm: Option<f64>,
    pub best_penetration_mm: f64,
}

/// The generic archetype for a platform described only by its data.
pub fn archetype_of_snapshot(s: &SnapshotHint) -> &'static str {
    let name = format!("{} {}", s.key, s.display_name).to_lowercase();
    let has = |k: &str| s.capability_kinds.iter().any(|c| c == k);
    if has("idf") || name.contains("mortar") {
        return "generic/mortar120";
    }
    if has("aa") {
        return "generic/shorad";
    }
    if s.target_class == "foot" {
        let at = name.contains("javelin") || name.contains("atgm") || name.contains("anti-tank") || name.split(|c: char| !c.is_alphanumeric() && c != '_').any(|w| w.ends_with("_at"));
        return if at { "generic/atgmTeam" } else { "generic/infantrySection" };
    }
    if s.target_class == "armoured_vehicle" {
        let heavy = s.armour_mm.unwrap_or(0.0) >= 400.0 || (has("atk") && s.best_penetration_mm >= 450.0 && s.move_type == "T");
        let ifv = ["bmp", "btr", "warrior", "bradley", "stryker", "ifv", "apc", "cv90", "marder", "puma"].iter().any(|w| name.contains(w));
        if ifv {
            return "generic/ifv";
        }
        return if heavy { "generic/mbt" } else { "generic/ifv" };
    }
    "generic/recce"
}

/// The archetype for a platform key (model/archetypes.ts `archetypeFor`): an
/// HQ is a company command post; a record is the nearest generic archetype
/// up its parent chain; anything else is a rifle section.
pub fn archetype_for<'a>(c: &'a Catalogue, platform: &str, command_rating: Option<f64>) -> &'a str {
    if command_rating.unwrap_or(0.0) > 0.0 {
        if let Some(p) = c.platform("generic/cp-company") {
            return p.id.as_str();
        }
    }
    c.archetype_of(platform).or_else(|| c.platform("generic/infantrySection").map(|p| p.id.as_str())).unwrap_or("generic/infantrySection")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(key: &str, name: &str, tc: &str, mt: &str, kinds: &[&str], armour: Option<f64>, pen: f64) -> SnapshotHint {
        SnapshotHint {
            key: key.into(),
            display_name: name.into(),
            target_class: tc.into(),
            move_type: mt.into(),
            capability_kinds: kinds.iter().map(|s| s.to_string()).collect(),
            armour_mm: armour,
            best_penetration_mm: pen,
        }
    }

    #[test]
    fn matches_the_ts_rules() {
        assert_eq!(archetype_of_snapshot(&hint("var_11_default", "Challenger 2", "armoured_vehicle", "T", &["atk"], Some(700.0), 657.0)), "generic/mbt");
        assert_eq!(archetype_of_snapshot(&hint("var_27_default", "FV510 Warrior", "armoured_vehicle", "T", &["atk"], Some(40.0), 50.0)), "generic/ifv");
        assert_eq!(archetype_of_snapshot(&hint("sec_uk_javelin", "UK Javelin Detachment", "foot", "F", &["atm"], None, 750.0)), "generic/atgmTeam");
        assert_eq!(archetype_of_snapshot(&hint("sec_ru_at", "RU Anti-Tank Section (Kornet)", "foot", "F", &["atm"], None, 1200.0)), "generic/atgmTeam");
        assert_eq!(archetype_of_snapshot(&hint("sec_uk_mortar", "UK 81mm Mortar Section", "foot", "F", &["idf"], None, 0.0)), "generic/mortar120");
        assert_eq!(archetype_of_snapshot(&hint("x", "Truck", "soft_skin", "W", &[], None, 0.0)), "generic/recce");
        assert_eq!(default_template_for_class(Some("foot")), "core/infantry");
        assert_eq!(default_template_for_class(None), DEFAULT_TEMPLATE_ID);
    }
}
