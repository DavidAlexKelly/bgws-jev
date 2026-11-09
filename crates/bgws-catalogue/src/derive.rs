//! Derivation rules (§14.1 "defaults per class, so a new platform can be
//! added with a few numbers"): after a record is merged onto its class and
//! parent, what it still lacks is derived from what it has — side and rear
//! armour from the front, CE protection from KE, cross-country speed from
//! road speed, RCS at other aspects from the front, endurance from fuel and
//! burn, and a radar's transmit power from the range it is quoted at, by the
//! radar equation against the standard target. Every ratio is a declared
//! figure (`catalogue.derive.*`), so sensitivity analysis can vary them.
//!
//! The rules work on the merged JSON, before typed deserialisation, so the
//! resolved types can hold plain numbers.

use crate::json::{get, get_f64, get_str, has_num, obj_mut, set_default};
use crate::types::{Band, Sidc};
use bgws_core::units::{BOLTZMANN, SPEED_OF_LIGHT, STANDARD_TEMPERATURE_K};
use bgws_core::{figure, Figure, Figures};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The figures the derivation rules read.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("catalogue.derive.armour_side_of_front", 0.3, "fraction", 0.05, 1.0, "default", Default, "side KE armour as a share of frontal, when a record gives only the front"),
        figure!("catalogue.derive.armour_rear_of_front", 0.12, "fraction", 0.02, 1.0, "default", Default, "rear KE armour as a share of frontal"),
        figure!("catalogue.derive.armour_top_of_front", 0.06, "fraction", 0.01, 1.0, "default", Default, "roof KE armour as a share of frontal"),
        figure!("catalogue.derive.ce_over_ke_composite", 1.4, "ratio", 1.0, 3.0, "default", Default, "CE protection over KE for composite armour (frontal KE at or above the threshold)"),
        figure!("catalogue.derive.composite_threshold_mm", 300.0, "mm", 50.0, 1000.0, "default", Default, "frontal KE armour from which a record is taken to be composite"),
        figure!("catalogue.derive.xc_ratio_tracked", 0.6, "fraction", 0.2, 1.0, "default", Default, "cross-country over road speed, tracked"),
        figure!("catalogue.derive.xc_ratio_wheeled", 0.45, "fraction", 0.1, 1.0, "default", Default, "cross-country over road speed, wheeled"),
        figure!("catalogue.derive.xc_ratio_foot", 0.6, "fraction", 0.2, 1.0, "P26", Low, "cross-country over road speed on foot: 2–3 against 4 km/h"),
        figure!("catalogue.derive.rcs_side_of_front", 3.0, "ratio", 0.1, 50.0, "default", Default, "side RCS over front RCS"),
        figure!("catalogue.derive.rcs_rear_of_front", 1.5, "ratio", 0.1, 50.0, "default", Default, "rear RCS over front RCS"),
        figure!("catalogue.derive.rcs_top_of_front", 5.0, "ratio", 0.1, 50.0, "default", Default, "top RCS over front RCS"),
        figure!("catalogue.derive.radar_standard_target_m2", 5.0, "m²", 0.1, 100.0, "default", Default, "the target a radar's quoted range is against (TS SensorSpec: 5 m²)"),
        figure!("catalogue.derive.fuel_reserve", 0.1, "fraction", 0.0, 0.5, "default", Default, "fuel kept in reserve when endurance is derived from fuel and burn"),
    ]
}

/// The derivation figures, read once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeriveParams {
    pub armour_side: f64,
    pub armour_rear: f64,
    pub armour_top: f64,
    pub ce_over_ke: f64,
    pub composite_mm: f64,
    pub xc_tracked: f64,
    pub xc_wheeled: f64,
    pub xc_foot: f64,
    pub rcs_side: f64,
    pub rcs_rear: f64,
    pub rcs_top: f64,
    pub radar_sigma: f64,
    pub fuel_reserve: f64,
}

impl DeriveParams {
    pub fn from_figures(f: &Figures) -> Self {
        let g = |k: &str| f.try_get(k).unwrap_or_else(|| figures().into_iter().find(|x| x.key == k).map(|x| x.value).unwrap_or(0.0));
        DeriveParams {
            armour_side: g("catalogue.derive.armour_side_of_front"),
            armour_rear: g("catalogue.derive.armour_rear_of_front"),
            armour_top: g("catalogue.derive.armour_top_of_front"),
            ce_over_ke: g("catalogue.derive.ce_over_ke_composite"),
            composite_mm: g("catalogue.derive.composite_threshold_mm"),
            xc_tracked: g("catalogue.derive.xc_ratio_tracked"),
            xc_wheeled: g("catalogue.derive.xc_ratio_wheeled"),
            xc_foot: g("catalogue.derive.xc_ratio_foot"),
            rcs_side: g("catalogue.derive.rcs_side_of_front"),
            rcs_rear: g("catalogue.derive.rcs_rear_of_front"),
            rcs_top: g("catalogue.derive.rcs_top_of_front"),
            radar_sigma: g("catalogue.derive.radar_standard_target_m2"),
            fuel_reserve: g("catalogue.derive.fuel_reserve"),
        }
    }
}

/// The transmit power a radar needs to see `sigma` m² at `range` m with
/// single-look SNR `snr_db` after integrating `n` pulses:
/// Pt = SNR·(4π)³·k·T₀·B·F·L·R⁴ / (G²·λ²·σ·n).
#[allow(clippy::too_many_arguments)]
pub fn radar_power_for_range(range: f64, sigma: f64, gain_db: f64, freq_hz: f64, bandwidth_hz: f64, nf_db: f64, losses_db: f64, snr_db: f64, n: f64) -> f64 {
    let db = |x: f64| 10f64.powf(x / 10.0);
    let lambda = SPEED_OF_LIGHT / freq_hz;
    let g = db(gain_db);
    let num = db(snr_db) * (4.0 * std::f64::consts::PI).powi(3) * BOLTZMANN * STANDARD_TEMPERATURE_K * bandwidth_hz * db(nf_db) * db(losses_db) * range.powi(4);
    num / (g * g * lambda * lambda * sigma * n.max(1.0))
}

/// The range at which a radar sees `sigma` (the inverse of `radar_power_for_range`).
#[allow(clippy::too_many_arguments)]
pub fn radar_range_for(pt: f64, sigma: f64, gain_db: f64, freq_hz: f64, bandwidth_hz: f64, nf_db: f64, losses_db: f64, snr_db: f64, n: f64) -> f64 {
    let at_1m = radar_power_for_range(1.0, sigma, gain_db, freq_hz, bandwidth_hz, nf_db, losses_db, snr_db, n);
    (pt / at_1m).powf(0.25)
}

fn band_centre(bands: &BTreeMap<String, Band>, id: &str) -> f64 {
    bands.get(id).map(Band::centre_hz).unwrap_or(0.0)
}

/// Fill what a merged record still lacks. Never overwrites a value it has.
pub fn derive(v: &mut Value, p: &DeriveParams, bands: &BTreeMap<String, Band>) {
    // SIDC: a friendly code as a string expands to both sides.
    if let Some(s) = get_str(v, &["sidc"]).map(str::to_owned) {
        let sidc = Sidc::from_friendly(&s);
        if let Some(m) = v.as_object_mut() {
            m.insert("sidc".into(), json!({"blue": sidc.blue, "red": sidc.red}));
        }
    }
    derive_mover(v, p);
    derive_body(v, p);
    derive_signatures(v, p);
    derive_sensors(v, p, bands);
    derive_emitters(v, bands);
    derive_radios(v, bands);
}

fn derive_mover(v: &mut Value, p: &DeriveParams) {
    let kind = get_str(v, &["mover", "kind"]).unwrap_or("static").to_owned();
    let max = get_f64(v, &["mover", "maxKmh"]).unwrap_or(0.0);
    let ground = matches!(kind.as_str(), "foot" | "wheeled" | "tracked");
    if ground {
        set_default(v, &["mover"], "roadKmh", json!(max));
        let road = get_f64(v, &["mover", "roadKmh"]).unwrap_or(max);
        let ratio = match kind.as_str() {
            "tracked" => p.xc_tracked,
            "wheeled" => p.xc_wheeled,
            _ => p.xc_foot,
        };
        set_default(v, &["mover"], "crossCountryKmh", json!(road * ratio));
    }
    if !has_num(v, &["mover", "enduranceH"]) {
        let fuel = get_f64(v, &["mover", "fuelCapacityL"]).unwrap_or(0.0);
        let burn = get_f64(v, &["mover", "burnMovingLph"]).unwrap_or(0.0);
        let batt = get_f64(v, &["mover", "batteryWh"]).unwrap_or(0.0);
        let draw = get_f64(v, &["mover", "batteryDrawW"]).unwrap_or(0.0);
        let h = if fuel > 0.0 && burn > 0.0 {
            fuel * (1.0 - p.fuel_reserve) / burn
        } else if batt > 0.0 && draw > 0.0 {
            batt / draw
        } else {
            0.0
        };
        if h > 0.0 {
            if let Some(m) = obj_mut(v, &["mover"]) {
                m.insert("enduranceH".into(), json!(h));
            }
        }
    }
}

fn derive_body(v: &mut Value, p: &DeriveParams) {
    let front = get_f64(v, &["body", "armour", "ke", "front"]).unwrap_or(0.0);
    if front > 0.0 {
        set_default(v, &["body", "armour", "ke"], "side", json!(front * p.armour_side));
        set_default(v, &["body", "armour", "ke"], "rear", json!(front * p.armour_rear));
        set_default(v, &["body", "armour", "ke"], "top", json!(front * p.armour_top));
        let factor = if front >= p.composite_mm { p.ce_over_ke } else { 1.0 };
        for aspect in ["front", "side", "rear", "top"] {
            let ke = get_f64(v, &["body", "armour", "ke", aspect]).unwrap_or(0.0);
            // Only the frontal arc is composite; side, rear and roof are near-homogeneous.
            let f = if aspect == "front" { factor } else { 1.0 };
            set_default(v, &["body", "armour", "ce"], aspect, json!(ke * f));
        }
    }
    let empty = get(v, &["body", "subPlatforms"]).and_then(Value::as_array).is_none_or(|a| a.is_empty());
    if empty {
        let name = get_str(v, &["name"]).unwrap_or("platform").to_owned();
        let count = get_f64(v, &["body", "count"]).unwrap_or(1.0);
        let crew = get_f64(v, &["body", "crew"]).unwrap_or(0.0);
        let comps = match get_str(v, &["body", "vulnerability"]) {
            Some("personnel") => json!(["crew", "firepower", "comms"]),
            Some("structure") => json!(["firepower", "sensors", "comms"]),
            _ => json!(["mobility", "firepower", "sensors", "comms", "crew"]),
        };
        if let Some(m) = obj_mut(v, &["body"]) {
            m.insert("subPlatforms".into(), json!([{"name": name, "count": count.round().max(0.0) as u64, "crew": crew.round().max(0.0) as u64, "components": comps}]));
        }
    }
}

fn derive_signatures(v: &mut Value, p: &DeriveParams) {
    // A scalar `rcsM2` (the TS form) is the X-band front.
    if let Some(r) = get_f64(v, &["signatures", "rcsM2"]) {
        set_default(v, &["signatures", "rcs", "x"], "front", json!(r));
        if let Some(m) = obj_mut(v, &["signatures"]) {
            m.remove("rcsM2");
        }
    }
    let bands: Vec<String> = get(v, &["signatures", "rcs"]).and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for b in bands {
        if let Some(front) = get_f64(v, &["signatures", "rcs", &b, "front"]) {
            set_default(v, &["signatures", "rcs", &b], "side", json!(front * p.rcs_side));
            set_default(v, &["signatures", "rcs", &b], "rear", json!(front * p.rcs_rear));
            set_default(v, &["signatures", "rcs", &b], "top", json!(front * p.rcs_top));
        }
    }
    if !has_num(v, &["signatures", "visual", "criticalDimM"]) {
        let h = get_f64(v, &["body", "heightM"]).unwrap_or(0.0);
        let w = get_f64(v, &["body", "widthM"]).unwrap_or(0.0);
        if h > 0.0 && w > 0.0 {
            if let Some(m) = obj_mut(v, &["signatures", "visual"]) {
                m.insert("criticalDimM".into(), json!((h * w).sqrt()));
            }
        }
    }
}

fn derive_sensors(v: &mut Value, p: &DeriveParams, bands: &BTreeMap<String, Band>) {
    let Some(list) = v.get_mut("sensors").and_then(Value::as_array_mut) else { return };
    for (i, s) in list.iter_mut().enumerate() {
        let kind = get_str(s, &["kind"]).unwrap_or("sensor").to_owned();
        set_default(s, &[], "id", json!(format!("{kind}-{i}")));
        let active = matches!(kind.as_str(), "radar" | "sonarActive" | "sar" | "cbr");
        set_default(s, &[], "active", json!(active));
        if s.get("radar").is_some_and(Value::is_object) {
            let band = get_str(s, &["band"]).unwrap_or("x").to_owned();
            if !has_num(s, &["radar", "freqHz"]) {
                let f = band_centre(bands, &band);
                if f > 0.0 {
                    if let Some(m) = obj_mut(s, &["radar"]) {
                        m.insert("freqHz".into(), json!(f));
                    }
                }
            }
            if !has_num(s, &["radar", "ptW"]) {
                let r = get_f64(s, &["rangeMaxM"]).unwrap_or(0.0);
                let gp = |k: &str, d: f64| get_f64(s, &["radar", k]).unwrap_or(d);
                let (g, f, b, nf, l, snr, n) =
                    (gp("gainDb", 30.0), gp("freqHz", 1e10), gp("bandwidthHz", 1e6), gp("noiseFigureDb", 4.0), gp("lossesDb", 6.0), gp("snrRequiredDb", 13.0), gp("pulsesIntegrated", 1.0));
                if r > 0.0 && f > 0.0 {
                    let pt = radar_power_for_range(r, p.radar_sigma, g, f, b, nf, l, snr, n);
                    if let Some(m) = obj_mut(s, &["radar"]) {
                        m.insert("ptW".into(), json!(pt));
                    }
                }
            }
        }
    }
}

fn derive_emitters(v: &mut Value, bands: &BTreeMap<String, Band>) {
    let Some(list) = v.get_mut("emitters").and_then(Value::as_array_mut) else { return };
    for (i, e) in list.iter_mut().enumerate() {
        let kind = get_str(e, &["kind"]).unwrap_or("emitter").to_owned();
        set_default(e, &[], "id", json!(format!("{kind}-{i}")));
        if !has_num(e, &["freqHz"]) {
            let band = get_str(e, &["band"]).unwrap_or("").to_owned();
            let f = band_centre(bands, &band);
            if let Some(m) = e.as_object_mut() {
                m.insert("freqHz".into(), json!(f));
            }
        }
    }
}

fn derive_radios(v: &mut Value, bands: &BTreeMap<String, Band>) {
    let Some(list) = v.get_mut("radios").and_then(Value::as_array_mut) else { return };
    for (i, r) in list.iter_mut().enumerate() {
        let bearer = get_str(r, &["bearer"]).unwrap_or("radio").to_owned();
        set_default(r, &[], "id", json!(format!("{bearer}-{i}")));
        if !has_num(r, &["freqHz"]) {
            let band = get_str(r, &["band"]).unwrap_or("").to_owned();
            let kind = bands.get(&band).map(|b| b.kind.as_str()).unwrap_or("none");
            let f = if kind == "radio" || kind == "radar" { band_centre(bands, &band) } else { 0.0 };
            if let Some(m) = r.as_object_mut() {
                m.insert("freqHz".into(), json!(f));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn params() -> DeriveParams {
        DeriveParams::from_figures(&Figures::from_list(figures()).unwrap())
    }

    #[test]
    fn radar_equation_round_trips() {
        let pt = radar_power_for_range(25_000.0, 5.0, 35.0, 1e10, 1e6, 4.0, 6.0, 13.0, 10.0);
        assert!(pt > 100.0 && pt < 100_000.0, "{pt}");
        let r = radar_range_for(pt, 5.0, 35.0, 1e10, 1e6, 4.0, 6.0, 13.0, 10.0);
        assert_relative_eq!(r, 25_000.0, max_relative = 1e-9);
        // Range scales with RCS^¼ [R6.4]: 5 m² to 0.001 m² is about 12%.
        let small = radar_range_for(pt, 0.001, 35.0, 1e10, 1e6, 4.0, 6.0, 13.0, 10.0);
        assert_relative_eq!(small / r, (0.001f64 / 5.0).powf(0.25), max_relative = 1e-9);
        assert!((small / r - 0.12).abs() < 0.01);
    }

    #[test]
    fn fills_from_a_few_numbers() {
        let mut v = json!({
            "name": "T", "sidc": "SFGPUCA",
            "mover": {"kind": "tracked", "maxKmh": 60, "fuelCapacityL": 1000, "burnMovingLph": 100},
            "body": {"count": 4, "crew": 3, "vulnerability": "armoured", "armour": {"ke": {"front": 600}}},
            "signatures": {"rcsM2": 10},
            "sensors": [{"kind": "radar", "band": "x", "rangeMaxM": 20000, "radar": {"gainDb": 30}}]
        });
        let mut bands = BTreeMap::new();
        bands.insert("x".to_string(), Band { id: "x".into(), name: "X".into(), kind: "radar".into(), f_min_hz: 8e9, f_max_hz: 12e9, source: String::new() });
        derive(&mut v, &params(), &bands);
        assert_eq!(get_str(&v, &["sidc", "red"]), Some("SHGPUCA--------"));
        assert_relative_eq!(get_f64(&v, &["mover", "crossCountryKmh"]).unwrap(), 36.0);
        assert_relative_eq!(get_f64(&v, &["mover", "enduranceH"]).unwrap(), 9.0);
        assert_relative_eq!(get_f64(&v, &["body", "armour", "ke", "side"]).unwrap(), 180.0);
        assert_relative_eq!(get_f64(&v, &["body", "armour", "ce", "front"]).unwrap(), 840.0);
        assert_relative_eq!(get_f64(&v, &["signatures", "rcs", "x", "side"]).unwrap(), 30.0);
        assert!(get_f64(&v, &["sensors"]).is_none());
        let s = &v["sensors"][0];
        assert_eq!(s["id"], json!("radar-0"));
        assert_eq!(s["active"], json!(true));
        assert!(s["radar"]["ptW"].as_f64().unwrap() > 0.0);
        assert_eq!(v["body"]["subPlatforms"][0]["count"], json!(4));
    }
}
