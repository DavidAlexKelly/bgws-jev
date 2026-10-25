//! The regression store (RUST_ENGINE.md §15.8, §17 "statistical"): every
//! vignette's distribution is saved as JSON in a directory (one file per
//! vignette, `<id>.json`). A new run is compared with the stored one; it fails
//! when it is out of band, or when the distribution has shifted significantly
//! (two-sample KS), until the change is accepted with an explanation.
//!
//! Accepting writes the new distribution as the baseline and appends the
//! explanation to the vignette's history, with the fingerprint of the
//! distribution it accepted. A later run that reproduces exactly that
//! distribution (same seeds, same build) is reported as `Accepted`, with the
//! explanation, so CI can say why the baseline moved.
//!
//! This is the one module in the crate that touches the disk (rule 5 allows
//! loaders where the task asks for them).

use crate::compare::{ks_two_sample, mann_whitney, KsResult, MannWhitneyResult};
use crate::stats::Summary;
use crate::vignettes::{Band, Verdict, VignetteResult};
use bgws_core::Figures;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Bit-exact f64 lists through JSON: Rust's `Display` writes the shortest
/// string that round-trips, and `str::parse` is correctly rounded.
mod exact_f64s {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[f64], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(|x| x.to_string()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
        let raw = Vec::<String>::deserialize(d)?;
        raw.iter()
            .map(|s| s.parse::<f64>().map_err(serde::de::Error::custom))
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum RegressionError {
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("bad JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("vignette id `{0}` is not a safe file name")]
    BadId(String),
    #[error("an acceptance needs an explanation")]
    NoExplanation,
}

/// One accepted change to a baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Why the distribution moved (free text, required).
    pub explanation: String,
    /// Fingerprint of the distribution that was accepted.
    pub fingerprint: String,
    /// Fingerprint of the baseline it replaced, if any.
    pub replaced: Option<String>,
    /// The verdict at the time it was accepted.
    pub verdict: Option<RegressionStatus>,
}

/// A stored baseline distribution for one vignette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDistribution {
    pub id: String,
    pub metric: String,
    pub unit: String,
    pub band: Band,
    pub estimate: f64,
    pub summary: Summary,
    /// The measured values, in replicate order. Stored as decimal strings
    /// that parse back bit-exactly (JSON numbers may not, depending on the
    /// serde_json build features).
    #[serde(with = "exact_f64s")]
    pub values: Vec<f64>,
    pub missing: usize,
    pub fingerprint: String,
    /// Accepted changes, oldest first.
    #[serde(default)]
    pub history: Vec<Acceptance>,
}

impl StoredDistribution {
    pub fn from_result(r: &VignetteResult) -> Self {
        StoredDistribution {
            id: r.id.clone(),
            metric: r.sample.metric.clone(),
            unit: r.sample.unit.clone(),
            band: r.band,
            estimate: r.estimate,
            summary: r.summary.clone(),
            values: r.sample.values.clone(),
            missing: r.sample.missing,
            fingerprint: r.sample.fingerprint(),
            history: Vec::new(),
        }
    }
}

/// The outcome of comparing a run with its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionStatus {
    /// In band and not significantly shifted.
    Pass,
    /// No baseline yet: in band, to be stored.
    New,
    /// Out of band (or no data). Fails CI.
    OutOfBand,
    /// In band but significantly shifted from the baseline. Fails CI.
    Shifted,
    /// Exactly the distribution an acceptance recorded.
    Accepted,
}

impl RegressionStatus {
    pub fn fails(&self) -> bool {
        matches!(
            self,
            RegressionStatus::OutOfBand | RegressionStatus::Shifted
        )
    }
}

/// A regression comparison, with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionVerdict {
    pub id: String,
    pub status: RegressionStatus,
    pub baseline_estimate: Option<f64>,
    pub new_estimate: f64,
    pub ks: Option<KsResult>,
    pub mann_whitney: Option<MannWhitneyResult>,
    /// The KS significance level used.
    pub alpha: f64,
    pub explanation: String,
}

/// Settings for the regression check.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RegressionOptions {
    /// KS significance level for "significantly shifted".
    pub ks_alpha: f64,
}

impl RegressionOptions {
    pub fn from_figures(f: &Figures) -> Self {
        RegressionOptions {
            ks_alpha: f.get("analysis.regression.ks_alpha"),
        }
    }
}

/// A directory of baselines.
#[derive(Debug, Clone)]
pub struct RegressionStore {
    dir: PathBuf,
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> RegressionError + '_ {
    move |source| RegressionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl RegressionStore {
    /// Open (creating if needed) a store in `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, RegressionError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        Ok(RegressionStore { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Result<PathBuf, RegressionError> {
        let ok = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && !id.starts_with('.');
        if !ok {
            return Err(RegressionError::BadId(id.into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    /// The stored baseline, if any.
    pub fn load(&self, id: &str) -> Result<Option<StoredDistribution>, RegressionError> {
        let path = self.path(id)?;
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(io(&path))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| RegressionError::Json { path, source })
    }

    /// Write a baseline (overwriting).
    pub fn save(&self, stored: &StoredDistribution) -> Result<PathBuf, RegressionError> {
        let path = self.path(&stored.id)?;
        let text =
            serde_json::to_string_pretty(stored).map_err(|source| RegressionError::Json {
                path: path.clone(),
                source,
            })?;
        std::fs::write(&path, text).map_err(io(&path))?;
        Ok(path)
    }

    /// Ids of every stored baseline, sorted.
    pub fn ids(&self) -> Result<Vec<String>, RegressionError> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&self.dir).map_err(io(&self.dir))? {
            let entry = entry.map_err(io(&self.dir))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".json") {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Compare a new run with its baseline. Does not write anything.
    pub fn compare(
        &self,
        result: &VignetteResult,
        options: &RegressionOptions,
    ) -> Result<RegressionVerdict, RegressionError> {
        let stored = self.load(&result.id)?;
        Ok(compare_with(stored.as_ref(), result, options))
    }

    /// Store the run as the baseline when it is new or passes; leave the
    /// baseline alone when it fails. Returns the verdict.
    pub fn check_and_record(
        &self,
        result: &VignetteResult,
        options: &RegressionOptions,
    ) -> Result<RegressionVerdict, RegressionError> {
        let stored = self.load(&result.id)?;
        let verdict = compare_with(stored.as_ref(), result, options);
        if verdict.status == RegressionStatus::New {
            self.save(&StoredDistribution::from_result(result))?;
        }
        Ok(verdict)
    }

    /// Accept a run as the new baseline, with an explanation (§15.8).
    pub fn accept(
        &self,
        result: &VignetteResult,
        explanation: &str,
        options: &RegressionOptions,
    ) -> Result<StoredDistribution, RegressionError> {
        if explanation.trim().is_empty() {
            return Err(RegressionError::NoExplanation);
        }
        let old = self.load(&result.id)?;
        let verdict = compare_with(old.as_ref(), result, options);
        let mut new = StoredDistribution::from_result(result);
        new.history = old.as_ref().map(|o| o.history.clone()).unwrap_or_default();
        new.history.push(Acceptance {
            explanation: explanation.trim().to_string(),
            fingerprint: new.fingerprint.clone(),
            replaced: old.map(|o| o.fingerprint),
            verdict: Some(verdict.status),
        });
        self.save(&new)?;
        Ok(new)
    }
}

/// Compare a run with an optional baseline (pure; the store's logic).
pub fn compare_with(
    stored: Option<&StoredDistribution>,
    result: &VignetteResult,
    options: &RegressionOptions,
) -> RegressionVerdict {
    let ks = stored.and_then(|s| ks_two_sample(&s.values, &result.sample.values));
    let mw = stored.and_then(|s| mann_whitney(&s.values, &result.sample.values));
    let fingerprint = result.sample.fingerprint();
    let accepted = stored.and_then(|s| {
        s.history
            .iter()
            .rev()
            .find(|a| a.fingerprint == fingerprint)
    });
    let shifted = ks.is_some_and(|k| k.rejects(options.ks_alpha));
    let (status, explanation) = if result.verdict != Verdict::InBand {
        (
            RegressionStatus::OutOfBand,
            format!("Out of band: {}", result.explanation),
        )
    } else if let Some(a) =
        accepted.filter(|_| stored.is_some_and(|s| s.fingerprint == fingerprint))
    {
        (
            RegressionStatus::Accepted,
            format!("Matches the accepted baseline: {}", a.explanation),
        )
    } else if stored.is_none() {
        (
            RegressionStatus::New,
            "No baseline yet; this run becomes it.".to_string(),
        )
    } else if shifted {
        let k = ks.expect("shifted implies a KS result");
        (
            RegressionStatus::Shifted,
            format!(
                "Shifted from the baseline: KS D = {:.3}, p = {:.2e} < {}; estimate {:.4} → {:.4}. Accept with an explanation if intended.",
                k.d,
                k.p_value,
                options.ks_alpha,
                stored.map(|s| s.estimate).unwrap_or(f64::NAN),
                result.estimate
            ),
        )
    } else {
        let p = ks
            .map(|k| format!("KS p = {:.3}", k.p_value))
            .unwrap_or_else(|| "no KS (empty sample)".into());
        (
            RegressionStatus::Pass,
            format!("In band and consistent with the baseline ({p})."),
        )
    };
    RegressionVerdict {
        id: result.id.clone(),
        status,
        baseline_estimate: stored.map(|s| s.estimate),
        new_estimate: result.estimate,
        ks,
        mann_whitney: mw,
        alpha: options.ks_alpha,
        explanation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::Sample;
    use crate::vignettes::find;

    fn result(shift: f64) -> VignetteResult {
        let v = find("attack-halt").unwrap();
        // A deterministic spread of 200 values around 0.2 + shift.
        let values = (0..200)
            .map(|i| 0.2 + shift + 0.05 * (((i * 37) % 200) as f64 / 200.0 - 0.5))
            .collect();
        v.evaluate(&Sample::new("losses_at_halt", "fraction", values), 0.95)
    }

    #[test]
    fn baseline_pass_shift_accept() {
        let dir = tempfile::tempdir().unwrap();
        let store = RegressionStore::open(dir.path()).unwrap();
        let opts = RegressionOptions { ks_alpha: 0.01 };
        let base = result(0.0);
        assert_eq!(
            store.check_and_record(&base, &opts).unwrap().status,
            RegressionStatus::New
        );
        assert_eq!(store.ids().unwrap(), vec!["attack-halt".to_string()]);
        // Same distribution: passes.
        assert_eq!(
            store.compare(&base, &opts).unwrap().status,
            RegressionStatus::Pass
        );
        // A shift that stays in band but is significant.
        let moved = result(0.04);
        let v = store.compare(&moved, &opts).unwrap();
        assert_eq!(v.status, RegressionStatus::Shifted, "{}", v.explanation);
        assert!(v.status.fails());
        // Out of band.
        let out = result(0.2);
        assert_eq!(
            store.compare(&out, &opts).unwrap().status,
            RegressionStatus::OutOfBand
        );
        // Accept the shift with an explanation.
        assert!(matches!(
            store.accept(&moved, "  ", &opts),
            Err(RegressionError::NoExplanation)
        ));
        let stored = store
            .accept(&moved, "Suppression model now includes fatigue.", &opts)
            .unwrap();
        assert_eq!(stored.history.len(), 1);
        assert_eq!(stored.history[0].verdict, Some(RegressionStatus::Shifted));
        let again = store.compare(&moved, &opts).unwrap();
        assert_eq!(again.status, RegressionStatus::Accepted);
        assert!(again.explanation.contains("fatigue"));
        // The old distribution is now the shifted one.
        assert_eq!(
            store.compare(&base, &opts).unwrap().status,
            RegressionStatus::Shifted
        );
        // Round trip through disk.
        let loaded = store.load("attack-halt").unwrap().unwrap();
        assert_eq!(
            loaded.values, stored.values,
            "values round-trip bit-exactly"
        );
        assert_eq!(loaded.history, stored.history);
        assert_eq!(loaded.fingerprint, moved.sample.fingerprint());
    }

    #[test]
    fn rejects_unsafe_ids_and_bad_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = RegressionStore::open(dir.path()).unwrap();
        assert!(matches!(
            store.load("../etc/passwd"),
            Err(RegressionError::BadId(_))
        ));
        std::fs::write(dir.path().join("broken.json"), "{not json").unwrap();
        assert!(matches!(
            store.load("broken"),
            Err(RegressionError::Json { .. })
        ));
        assert!(store.load("absent").unwrap().is_none());
    }
}
