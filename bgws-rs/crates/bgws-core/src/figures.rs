//! The figures registry (rule 3, §15.1): every number a model uses is a
//! declared figure with a unit, a plausible range, a source, a confidence and
//! a sensitivity note. Models never hold literals; they ask the registry.
//!
//! Each crate declares its figures with `figure!` in a `figures()` function;
//! the simulation assembles them into one `Figures`. A run can override any
//! figure within its range (that is how sensitivity analysis varies them).
//! The registry records which figures were read, so a test can fail if a
//! declared figure is never used.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Chosen without a source, awaiting one. Shown as **[default]**.
    Default,
    /// From an open source, but indirect or old.
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Figure {
    pub key: String,
    pub value: f64,
    /// Unit symbol, e.g. "m", "s", "1/s", "dB", "fraction".
    pub unit: String,
    pub min: f64,
    pub max: f64,
    /// A reference into the research (`P14`, `R4.3`) or "default".
    pub source: String,
    pub confidence: Confidence,
    #[serde(default)]
    pub note: String,
}

impl Figure {
    pub fn is_default(&self) -> bool {
        self.confidence == Confidence::Default
    }
}

/// Declare a figure: `figure!("sensing.tau_search", 20.0, "s", 5.0, 60.0, "default", Default, "search time constant")`.
#[macro_export]
macro_rules! figure {
    ($key:expr, $value:expr, $unit:expr, $min:expr, $max:expr, $source:expr, $conf:ident, $note:expr) => {
        $crate::figures::Figure {
            key: ($key).to_string(),
            value: $value as f64,
            unit: ($unit).to_string(),
            min: $min as f64,
            max: $max as f64,
            source: ($source).to_string(),
            confidence: $crate::figures::Confidence::$conf,
            note: ($note).to_string(),
        }
    };
}

#[derive(Debug, Error, PartialEq)]
pub enum FigureError {
    #[error("figure `{0}` is declared twice")]
    Duplicate(String),
    #[error("figure `{0}` is not declared")]
    Unknown(String),
    #[error("figure `{key}` = {value} is outside its range [{min}, {max}]")]
    OutOfRange { key: String, value: f64, min: f64, max: f64 },
    #[error("figure `{key}` has unit `{declared}`, asked for `{asked}`")]
    WrongUnit { key: String, declared: String, asked: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Figures {
    figures: BTreeMap<String, Figure>,
    #[serde(skip)]
    used: Mutex<BTreeSet<String>>,
}

impl Clone for Figures {
    fn clone(&self) -> Self {
        Figures { figures: self.figures.clone(), used: Mutex::new(self.used.lock().unwrap().clone()) }
    }
}

impl Figures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_list(list: impl IntoIterator<Item = Figure>) -> Result<Self, FigureError> {
        let mut f = Self::new();
        f.extend(list)?;
        Ok(f)
    }

    pub fn extend(&mut self, list: impl IntoIterator<Item = Figure>) -> Result<(), FigureError> {
        for fig in list {
            if fig.value < fig.min || fig.value > fig.max {
                return Err(FigureError::OutOfRange { key: fig.key, value: fig.value, min: fig.min, max: fig.max });
            }
            if self.figures.contains_key(&fig.key) {
                return Err(FigureError::Duplicate(fig.key));
            }
            self.figures.insert(fig.key.clone(), fig);
        }
        Ok(())
    }

    /// The value of a declared figure. Panics on an undeclared key: that is a
    /// programming error, caught by every test that exercises the model.
    pub fn get(&self, key: &str) -> f64 {
        match self.figures.get(key) {
            Some(f) => {
                self.mark_used(key);
                f.value
            }
            None => panic!("figure `{key}` is not declared"),
        }
    }

    /// The value, checking the unit the model expects.
    pub fn get_in(&self, key: &str, unit: &str) -> Result<f64, FigureError> {
        let f = self.figures.get(key).ok_or_else(|| FigureError::Unknown(key.into()))?;
        if f.unit != unit {
            return Err(FigureError::WrongUnit { key: key.into(), declared: f.unit.clone(), asked: unit.into() });
        }
        self.mark_used(key);
        Ok(f.value)
    }

    pub fn try_get(&self, key: &str) -> Option<f64> {
        self.figures.get(key).map(|f| {
            self.mark_used(key);
            f.value
        })
    }

    fn mark_used(&self, key: &str) {
        let mut used = self.used.lock().unwrap();
        if !used.contains(key) {
            used.insert(key.to_owned());
        }
    }

    /// Override a value for this run. Must stay inside the declared range.
    pub fn set(&mut self, key: &str, value: f64) -> Result<(), FigureError> {
        let f = self.figures.get_mut(key).ok_or_else(|| FigureError::Unknown(key.into()))?;
        if value < f.min || value > f.max {
            return Err(FigureError::OutOfRange { key: key.into(), value, min: f.min, max: f.max });
        }
        f.value = value;
        Ok(())
    }

    pub fn figure(&self, key: &str) -> Option<&Figure> {
        self.figures.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Figure> {
        self.figures.values()
    }

    pub fn len(&self) -> usize {
        self.figures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
    }

    pub fn used(&self) -> BTreeSet<String> {
        self.used.lock().unwrap().clone()
    }

    pub fn unused(&self) -> Vec<String> {
        let used = self.used.lock().unwrap();
        self.figures.keys().filter(|k| !used.contains(*k)).cloned().collect()
    }

    /// The figure report (§15.1) as Markdown: every figure, defaults flagged.
    pub fn report_markdown(&self) -> String {
        let mut s = String::from("| Figure | Value | Unit | Range | Source | Confidence | Note |\n|---|---|---|---|---|---|---|\n");
        for f in self.figures.values() {
            let src = if f.is_default() { "**[default]**".to_string() } else { f.source.clone() };
            s.push_str(&format!(
                "| `{}` | {} | {} | {}–{} | {} | {:?} | {} |\n",
                f.key, f.value, f.unit, f.min, f.max, src, f.confidence, f.note
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Figures {
        Figures::from_list([
            figure!("a.x", 2.0, "s", 1.0, 3.0, "P14", Medium, ""),
            figure!("a.y", 0.2, "fraction", 0.1, 0.3, "default", Default, ""),
        ])
        .unwrap()
    }

    #[test]
    fn get_set_and_usage() {
        let mut f = sample();
        assert_eq!(f.get("a.x"), 2.0);
        assert_eq!(f.unused(), vec!["a.y".to_string()]);
        assert!(f.set("a.y", 0.5).is_err());
        f.set("a.y", 0.25).unwrap();
        assert_eq!(f.get_in("a.y", "fraction"), Ok(0.25));
        assert!(matches!(f.get_in("a.x", "m"), Err(FigureError::WrongUnit { .. })));
        assert!(f.report_markdown().contains("**[default]**"));
    }

    #[test]
    fn rejects_duplicates_and_out_of_range() {
        let mut f = sample();
        assert!(f.extend([figure!("a.x", 2.0, "s", 1.0, 3.0, "", Low, "")]).is_err());
        assert!(f.extend([figure!("b", 9.0, "s", 1.0, 3.0, "", Low, "")]).is_err());
    }
}
