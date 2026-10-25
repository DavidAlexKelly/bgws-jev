//! The declared-figures registry (design rule 3, §15.1).
//!
//! Every number a model uses is a `Figure`: a value with its unit, the range
//! it may be set within, where it came from and how far to trust it. Models
//! never hold literals: each reads its figures once, at load, through a
//! `FigureReader`, into a plain struct. The reader records what was read, so
//! a test can prove every figure in the registry is used by some model and
//! every value is inside its range.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// From a primary or official source.
    Sourced,
    /// From a secondary source, an analyst or a contested claim.
    Secondary,
    /// A chosen value, awaiting a source: an open tuning variable.
    Default,
    /// No source found; a reasoned assumption.
    Assumption,
    /// From physics or a published standard model, not from the battlefield.
    Physics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Figure {
    pub key: String,
    pub value: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub confidence: Confidence,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Figures {
    map: BTreeMap<String, Figure>,
}

#[derive(Debug, thiserror::Error)]
pub enum FigureError {
    #[error("figure file: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("figure {0} is defined twice")]
    Duplicate(String),
    #[error("figure {key}: value {value} is outside its range {min}..{max}")]
    OutOfRange { key: String, value: f64, min: f64, max: f64 },
    #[error("no figure {0}")]
    Missing(String),
}

impl Figures {
    /// Load one or more JSON arrays of figures.
    pub fn from_json_files(files: &[&str]) -> Result<Figures, FigureError> {
        let mut f = Figures::default();
        for text in files {
            let list: Vec<Figure> = serde_json::from_str(text)?;
            for fig in list {
                if f.map.contains_key(&fig.key) {
                    return Err(FigureError::Duplicate(fig.key));
                }
                f.map.insert(fig.key.clone(), fig);
            }
        }
        f.check()?;
        Ok(f)
    }
    pub fn check(&self) -> Result<(), FigureError> {
        for fig in self.map.values() {
            if !(fig.value >= fig.min && fig.value <= fig.max) {
                return Err(FigureError::OutOfRange { key: fig.key.clone(), value: fig.value, min: fig.min, max: fig.max });
            }
        }
        Ok(())
    }
    pub fn get(&self, key: &str) -> Option<&Figure> {
        self.map.get(key)
    }
    pub fn all(&self) -> impl Iterator<Item = &Figure> {
        self.map.values()
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// Set a figure (a scenario override, a sensitivity sample). Must stay in range.
    pub fn set(&mut self, key: &str, value: f64) -> Result<(), FigureError> {
        let fig = self.map.get_mut(key).ok_or_else(|| FigureError::Missing(key.into()))?;
        if !(value >= fig.min && value <= fig.max) {
            return Err(FigureError::OutOfRange { key: key.into(), value, min: fig.min, max: fig.max });
        }
        fig.value = value;
        Ok(())
    }
    pub fn reader(&self) -> FigureReader<'_> {
        FigureReader { figs: self, used: RefCell::new(BTreeSet::new()) }
    }
}

/// Reads figures for the models, and remembers which were read.
pub struct FigureReader<'a> {
    figs: &'a Figures,
    used: RefCell<BTreeSet<String>>,
}

impl<'a> FigureReader<'a> {
    /// The value of a figure. Panics if missing: a model asking for a figure
    /// that is not declared is a programming error, caught by the first test.
    pub fn f(&self, key: &str) -> f64 {
        let fig = self.figs.map.get(key).unwrap_or_else(|| panic!("figure `{key}` is not declared in data/figures"));
        self.used.borrow_mut().insert(key.to_string());
        fig.value
    }
    /// A figure as a whole number.
    pub fn n(&self, key: &str) -> usize {
        self.f(key).round().max(0.0) as usize
    }
    pub fn used(&self) -> BTreeSet<String> {
        self.used.borrow().clone()
    }
    pub fn unused(&self) -> Vec<String> {
        let used = self.used.borrow();
        self.figs.map.keys().filter(|k| !used.contains(*k)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_and_tracks() {
        let f = Figures::from_json_files(&[r#"[{"key":"a.b","value":2.0,"unit":"s","min":1,"max":3,"confidence":"default"},
            {"key":"a.c","value":5,"unit":"m","min":0,"max":10,"confidence":"physics"}]"#])
        .unwrap();
        let r = f.reader();
        assert_eq!(r.f("a.b"), 2.0);
        assert_eq!(r.unused(), vec!["a.c".to_string()]);
        let mut g = f.clone();
        assert!(g.set("a.b", 9.0).is_err());
        assert!(Figures::from_json_files(&[r#"[{"key":"x","value":20,"unit":"","min":0,"max":1,"confidence":"default"}]"#]).is_err());
    }
}
