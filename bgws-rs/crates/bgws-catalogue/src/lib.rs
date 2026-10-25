//! bgws-catalogue: platform records, weapons and munitions (§5.1, §14.1).
//!
//! `Catalogue::builtin()` loads the synthetic catalogue in `data/catalogue/`;
//! a scenario may add or override records. `validate` checks every reference
//! (a mount's weapon exists, a weapon's munitions exist, a munition that flies
//! names a record that can fly) and every physical range.

pub mod schema;

pub use schema::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    pub records: BTreeMap<String, Record>,
    pub weapons: BTreeMap<String, WeaponSpec>,
    pub munitions: BTreeMap<String, MunitionSpec>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("{file}: {err}")]
    Parse { file: String, err: serde_json::Error },
    #[error("duplicate {kind} id {id}")]
    Duplicate { kind: &'static str, id: String },
    #[error("catalogue problems:\n{0}")]
    Invalid(String),
}

pub const BUILTIN_RECORDS: &[(&str, &str)] = &[
    ("land.json", include_str!("../../../data/catalogue/land.json")),
    ("air.json", include_str!("../../../data/catalogue/air.json")),
    ("sea.json", include_str!("../../../data/catalogue/sea.json")),
    ("space.json", include_str!("../../../data/catalogue/space.json")),
    ("munitions-flying.json", include_str!("../../../data/catalogue/munitions-flying.json")),
];
pub const BUILTIN_WEAPONS: &str = include_str!("../../../data/catalogue/weapons.json");
pub const BUILTIN_MUNITIONS: &str = include_str!("../../../data/catalogue/munitions.json");

impl Catalogue {
    pub fn builtin() -> Catalogue {
        let mut c = Catalogue::default();
        for (file, text) in BUILTIN_RECORDS {
            c.add_records(file, text).expect("built-in records parse");
        }
        c.add_weapons("weapons.json", BUILTIN_WEAPONS).expect("built-in weapons parse");
        c.add_munitions("munitions.json", BUILTIN_MUNITIONS).expect("built-in munitions parse");
        c
    }

    pub fn add_records(&mut self, file: &str, text: &str) -> Result<(), CatalogueError> {
        let list: Vec<Record> = serde_json::from_str(text).map_err(|err| CatalogueError::Parse { file: file.into(), err })?;
        for r in list {
            if self.records.insert(r.id.clone(), r.clone()).is_some() {
                return Err(CatalogueError::Duplicate { kind: "record", id: r.id });
            }
        }
        Ok(())
    }
    pub fn add_weapons(&mut self, file: &str, text: &str) -> Result<(), CatalogueError> {
        let list: Vec<WeaponSpec> = serde_json::from_str(text).map_err(|err| CatalogueError::Parse { file: file.into(), err })?;
        for w in list {
            if self.weapons.insert(w.id.clone(), w.clone()).is_some() {
                return Err(CatalogueError::Duplicate { kind: "weapon", id: w.id });
            }
        }
        Ok(())
    }
    pub fn add_munitions(&mut self, file: &str, text: &str) -> Result<(), CatalogueError> {
        let list: Vec<MunitionSpec> = serde_json::from_str(text).map_err(|err| CatalogueError::Parse { file: file.into(), err })?;
        for m in list {
            if self.munitions.insert(m.id.clone(), m.clone()).is_some() {
                return Err(CatalogueError::Duplicate { kind: "munition", id: m.id });
            }
        }
        Ok(())
    }
    /// Replace or add records (a scenario's own).
    pub fn overlay(&mut self, records: Vec<Record>) {
        for r in records {
            self.records.insert(r.id.clone(), r);
        }
    }

    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }
    pub fn weapon(&self, id: &str) -> Option<&WeaponSpec> {
        self.weapons.get(id)
    }
    pub fn munition(&self, id: &str) -> Option<&MunitionSpec> {
        self.munitions.get(id)
    }

    /// Every problem, in words. Empty when the catalogue is sound.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        for r in self.records.values() {
            let who = format!("record {}", r.id);
            for m in &r.mounts {
                match self.weapons.get(&m.weapon) {
                    None => p.push(format!("{who}: mount names unknown weapon {}", m.weapon)),
                    Some(w) => {
                        for mu in m.reserve.keys() {
                            if !w.munitions.contains(mu) {
                                p.push(format!("{who}: reserve of {mu}, which {} cannot fire", w.id));
                            }
                        }
                    }
                }
            }
            for c in &r.carries {
                if !self.records.contains_key(&c.record) {
                    p.push(format!("{who}: carries unknown record {}", c.record));
                }
            }
            if r.body.count == 0 {
                p.push(format!("{who}: body.count must be at least 1"));
            }
            let k = r.mover_kind();
            if k.is_ground() && r.mover.max_ms <= 0.0 {
                p.push(format!("{who}: a ground mover needs maxMs"));
            }
            if matches!(k, MoverKind::FixedWing | MoverKind::Rotary | MoverKind::Multirotor) && r.mover.cruise_ms <= 0.0 {
                p.push(format!("{who}: an aircraft needs cruiseMs"));
            }
            if matches!(k, MoverKind::Missile | MoverKind::Ballistic) && (r.mover.mass_kg <= 0.0 || r.mover.burn_s <= 0.0) {
                p.push(format!("{who}: a missile needs massKg and burnS"));
            }
            for s in &r.sensors {
                if s.kind == SensorKind::Radar || s.kind == SensorKind::Wlr {
                    if s.peak_w <= 0.0 || s.gain_db <= 0.0 {
                        p.push(format!("{who}: a radar needs peakW and gainDb"));
                    }
                }
                if s.max_range_m <= 0.0 {
                    p.push(format!("{who}: sensor {:?} needs maxRangeM", s.kind));
                }
            }
            for (i, v) in r.signature.rcs_m2.iter().enumerate() {
                if *v <= 0.0 {
                    p.push(format!("{who}: rcsM2[{i}] must be positive"));
                }
            }
        }
        for w in self.weapons.values() {
            if w.munitions.is_empty() {
                p.push(format!("weapon {}: no munitions", w.id));
            }
            for m in &w.munitions {
                if !self.munitions.contains_key(m) {
                    p.push(format!("weapon {}: unknown munition {m}", w.id));
                }
            }
            if w.max_range_m <= w.min_range_m {
                p.push(format!("weapon {}: maxRangeM must exceed minRangeM", w.id));
            }
        }
        for m in self.munitions.values() {
            if let Some(f) = &m.flies_as {
                match self.records.get(f) {
                    None => p.push(format!("munition {}: flies as unknown record {f}", m.id)),
                    Some(r) if r.domain != PlatformDomain::Munition && r.domain != PlatformDomain::Air => {
                        p.push(format!("munition {}: flies as {f}, which is not a munition or aircraft", m.id))
                    }
                    _ => {}
                }
            }
            if !(0.0..=1.0).contains(&m.reliability) {
                p.push(format!("munition {}: reliability must be 0–1", m.id));
            }
        }
        p
    }

    pub fn validate(&self) -> Result<(), CatalogueError> {
        let p = self.problems();
        if p.is_empty() {
            Ok(())
        } else {
            Err(CatalogueError::Invalid(p.join("\n")))
        }
    }

    /// Records by archetype.
    pub fn by_archetype(&self, archetype: &str) -> Vec<&Record> {
        self.records.values().filter(|r| r.archetype == archetype).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_catalogue_is_sound() {
        let c = Catalogue::builtin();
        let p = c.problems();
        assert!(p.is_empty(), "{}", p.join("\n"));
        assert!(c.records.len() >= 40, "{} records", c.records.len());
    }
}
