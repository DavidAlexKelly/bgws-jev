//! Units and sane ranges for catalogue fields (§14.1: "validated: units,
//! ranges"). Every catalogue field carries its unit in its name (`roadKmh`,
//! `rangeMaxM`, `ptW`), as the TS catalogue did; this table reads the unit
//! from the name — or, for map entries like `rcs.x.front`, from the parent
//! key — and gives the range a value must sit in. The same table bounds the
//! class-default figures (`figures.rs`), so a figure override and a record
//! are held to one standard.

/// A field's unit symbol and its plausible range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldUnit {
    pub unit: &'static str,
    pub min: f64,
    pub max: f64,
}

const fn fu(unit: &'static str, min: f64, max: f64) -> FieldUnit {
    FieldUnit { unit, min, max }
}

/// Fields with their own range, by key (checked before the suffix rules).
const SPECIFIC: &[(&str, FieldUnit)] = &[
    ("costUsd", fu("USD", 0.0, 2.0e10)),
    ("ceilingM", fu("m", 0.0, 40_000.0)),
    ("maxDepthM", fu("m", 0.0, 1_200.0)),
    ("roadKmh", fu("km/h", 0.0, 150.0)),
    ("crossCountryKmh", fu("km/h", 0.0, 150.0)),
    ("orbitAltKm", fu("km", 0.0, 50_000.0)),
    ("inclinationDeg", fu("deg", 0.0, 180.0)),
    ("forDeg", fu("deg", 0.0, 360.0)),
    ("fovDeg", fu("deg", 0.0, 360.0)),
    ("arcDeg", fu("deg", 0.0, 360.0)),
    ("beamwidthDeg", fu("deg", 0.0, 360.0)),
    ("maxSlopeDeg", fu("deg", 0.0, 60.0)),
    ("dfRmsDeg", fu("deg", 0.0, 90.0)),
    ("dutyCycle", fu("fraction", 0.0, 1.0)),
    ("pk", fu("fraction", 0.0, 1.0)),
    ("hitShare", fu("fraction", 0.0, 1.0)),
    ("noiseFigureDb", fu("dB", 0.0, 30.0)),
    ("lossesDb", fu("dB", 0.0, 40.0)),
    ("snrRequiredDb", fu("dB", -10.0, 40.0)),
    ("gainDb", fu("dB", -10.0, 60.0)),
    ("diDb", fu("dB", 0.0, 50.0)),
    ("dtDb", fu("dB", -30.0, 50.0)),
    ("sourceLevelDb", fu("dB", 0.0, 260.0)),
    ("selfNoiseDb", fu("dB", 0.0, 200.0)),
    ("sidelobeDb", fu("dB", 0.0, 80.0)),
    ("sensitivityDbm", fu("dBm", -140.0, 0.0)),
    ("climbMs", fu("m/s", 0.0, 400.0)),
    ("stayingPower", fu("hits", 0.0, 100.0)),
    ("count", fu("platforms", 0.0, 100_000.0)),
    ("crew", fu("people", 0.0, 5_000.0)),
    ("salvo", fu("rounds", 0.0, 10_000.0)),
    ("magazine", fu("rounds", 0.0, 1_000_000.0)),
    ("sections", fu("sections", 0.0, 100.0)),
    ("pulsesIntegrated", fu("pulses", 1.0, 100_000.0)),
    ("medical", fu("treatments", 0.0, 1.0e7)),
    ("spares", fu("repairs", 0.0, 1.0e7)),
    ("foodRations", fu("person-days", 0.0, 1.0e8)),
    ("charges", fu("engagements", 0.0, 100.0)),
    ("reactionS", fu("s", 0.0, 86_400.0)),
    ("cryptoDelayS", fu("s", 0.0, 600.0)),
    ("speedMs", fu("m/s", 0.0, 10_000.0)),
    ("kmh", fu("km/h", 0.0, 30_000.0)),
    ("db", fu("dB", 0.0, 260.0)),
    ("mm", fu("mm", 0.0, 3_000.0)),
];

/// Suffix rules, longest first.
const SUFFIX: &[(&str, FieldUnit)] = &[
    ("CycPerMrad", fu("cyc/mrad", 0.0, 2_000.0)),
    ("HpT", fu("hp/t", 0.0, 200.0)),
    ("Kmh", fu("km/h", 0.0, 30_000.0)),
    ("Lph", fu("L/h", 0.0, 50_000.0)),
    ("Dps", fu("deg/s", 0.0, 720.0)),
    ("Kpa", fu("kPa", 0.0, 2_000.0)),
    ("Usd", fu("USD", 0.0, 2.0e10)),
    ("Bps", fu("bit/s", 0.0, 1.0e11)),
    ("Dbm", fu("dBm", -200.0, 100.0)),
    ("Dbi", fu("dBi", -30.0, 60.0)),
    ("Dba", fu("dB(A)", 0.0, 200.0)),
    ("Db", fu("dB", -50.0, 260.0)),
    ("Hz", fu("Hz", 0.0, 1.0e16)),
    ("Wh", fu("Wh", 0.0, 1.0e9)),
    ("Km", fu("km", 0.0, 50_000.0)),
    ("Mm", fu("mm", 0.0, 3_000.0)),
    ("M2", fu("m²", 0.0, 1.0e6)),
    ("Ms", fu("m/s", 0.0, 10_000.0)),
    ("Kg", fu("kg", 0.0, 1.0e6)),
    ("Deg", fu("deg", -360.0, 360.0)),
    ("T", fu("t", 0.0, 1.0e6)),
    ("L", fu("L", 0.0, 1.0e9)),
    ("W", fu("W", 0.0, 1.0e9)),
    ("H", fu("h", 0.0, 1.0e6)),
    ("S", fu("s", 0.0, 1.0e7)),
    ("M", fu("m", -12_000.0, 5.0e7)),
];

/// The unit and range of a field, from its key path (array indices left out).
/// `None` for a key with no unit in its name and no known context.
pub fn field_unit(path: &[&str]) -> Option<FieldUnit> {
    let last = *path.last()?;
    let parent = if path.len() >= 2 { path[path.len() - 2] } else { "" };
    let grand = if path.len() >= 3 { path[path.len() - 3] } else { "" };
    // Context from the parent key: map entries and aspect tables.
    match parent {
        "contrast" => return Some(fu("fraction", 0.0, 1.0)),
        "deltaK" => return Some(fu("K", -50.0, 2_000.0)),
        "n50" => return Some(fu("cycles", 0.05, 50.0)),
        "ke" | "ce" => return Some(fu("mm", 0.0, 3_000.0)),
        "ammo" => return Some(fu("rounds", 0.0, 1.0e8)),
        "jamRadiiM" => return Some(fu("m", 0.0, 1.0e7)),
        "index" => return Some(fu("multiplier", 0.0, 100.0)),
        "crew" if last == "quality" => return Some(fu("quality", 1.0, 8.0)),
        "fireControl" if last == "quality" => return Some(fu("fraction", 0.0, 1.0)),
        _ => {}
    }
    if grand == "rcs" {
        return Some(fu("m²", 0.0, 1.0e6));
    }
    if let Some((_, u)) = SPECIFIC.iter().find(|(k, _)| *k == last) {
        return Some(*u);
    }
    for (suffix, u) in SUFFIX {
        if let Some(stem) = last.strip_suffix(suffix) {
            // A suffix counts only after a lowercase letter or digit (camelCase boundary).
            if stem.chars().last().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
                return Some(*u);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_units_from_names_and_context() {
        assert_eq!(field_unit(&["mover", "roadKmh"]).unwrap().unit, "km/h");
        assert_eq!(field_unit(&["sensors", "rangeMaxM"]).unwrap().unit, "m");
        assert_eq!(field_unit(&["body", "armour", "ke", "front"]).unwrap().unit, "mm");
        assert_eq!(field_unit(&["signatures", "rcs", "x", "front"]).unwrap().unit, "m²");
        assert_eq!(field_unit(&["signatures", "ir", "deltaK", "moved"]).unwrap().unit, "K");
        assert_eq!(field_unit(&["crew", "quality"]).unwrap().max, 8.0);
        assert_eq!(field_unit(&["mounts", "fireControl", "quality"]).unwrap().max, 1.0);
        assert_eq!(field_unit(&["body", "massT"]).unwrap().unit, "t");
        assert_eq!(field_unit(&["sensors", "radar", "ptW"]).unwrap().unit, "W");
        assert_eq!(field_unit(&["optics", "resolutionCycPerMrad"]).unwrap().unit, "cyc/mrad");
        assert!(field_unit(&["name"]).is_none());
        assert!(field_unit(&["front"]).is_none());
    }
}
