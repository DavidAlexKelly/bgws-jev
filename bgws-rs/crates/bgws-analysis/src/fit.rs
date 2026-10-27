//! Meta-model fitting for aggregated combat (RUST_ENGINE.md §8.13, §15.5):
//! the aggregate model's coefficients are fitted to entity-level runs, not
//! hand-set. This module fits rates and distributions by maximum likelihood
//! from samples, with goodness-of-fit checks; fits stochastic Lanchester
//! attrition coefficients from engagement observations; keeps a table of
//! fitted coefficients keyed by (formation type pair, posture, terrain class)
//! that serialises to JSON; and checks that a fitted aggregate model agrees
//! with new entity-level samples (the calibration check when entity models
//! change).
//!
//! Families: exponential and Poisson (rates), lognormal and gamma (positive,
//! skewed times and quantities). Goodness of fit is KS for continuous families
//! and Pearson chi-squared for counts (see `compare` for the caveat on
//! estimated parameters). JTLS and SWORD use aggregate attrition [R8]; fitting
//! it to entity runs is this plan's addition.

use crate::compare::{chi_squared_gof, ks_one_sample, ChiSquaredResult, KsResult};
use crate::stats::{mean, sorted_finite, Interval};
use bgws_core::Figures;
use rand::Rng;
use serde::{Deserialize, Serialize};
use statrs::distribution::{
    ChiSquared, Continuous, ContinuousCDF, Discrete, DiscreteCDF, Exp, Gamma, LogNormal, Poisson,
};
use statrs::function::gamma::digamma;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum FitError {
    #[error("need at least {need} samples, got {got}")]
    TooFewSamples { need: usize, got: usize },
    #[error("the {family} family needs positive samples")]
    NonPositive { family: &'static str },
    #[error("samples have no spread; the {family} fit is degenerate")]
    Degenerate { family: &'static str },
    #[error("observations have zero total exposure")]
    NoExposure,
}

/// A fitted distribution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "lowercase")]
pub enum FittedDistribution {
    /// Rate λ (per unit of the sample's unit): mean 1/λ.
    Exponential { rate: f64 },
    /// ln X ~ N(μ, σ²).
    Lognormal { mu: f64, sigma: f64 },
    /// Shape k, scale θ: mean kθ.
    Gamma { shape: f64, scale: f64 },
    /// Counts with mean λ.
    Poisson { lambda: f64 },
}

impl FittedDistribution {
    pub fn family(&self) -> &'static str {
        match self {
            Self::Exponential { .. } => "exponential",
            Self::Lognormal { .. } => "lognormal",
            Self::Gamma { .. } => "gamma",
            Self::Poisson { .. } => "poisson",
        }
    }

    pub fn parameters(&self) -> usize {
        match self {
            Self::Exponential { .. } | Self::Poisson { .. } => 1,
            Self::Lognormal { .. } | Self::Gamma { .. } => 2,
        }
    }

    pub fn mean(&self) -> f64 {
        match *self {
            Self::Exponential { rate } => 1.0 / rate,
            Self::Lognormal { mu, sigma } => (mu + sigma * sigma / 2.0).exp(),
            Self::Gamma { shape, scale } => shape * scale,
            Self::Poisson { lambda } => lambda,
        }
    }

    pub fn variance(&self) -> f64 {
        match *self {
            Self::Exponential { rate } => 1.0 / (rate * rate),
            Self::Lognormal { mu, sigma } => {
                ((sigma * sigma).exp() - 1.0) * (2.0 * mu + sigma * sigma).exp()
            }
            Self::Gamma { shape, scale } => shape * scale * scale,
            Self::Poisson { lambda } => lambda,
        }
    }

    /// CDF at x (NaN for invalid parameters).
    pub fn cdf(&self, x: f64) -> f64 {
        match *self {
            Self::Exponential { rate } => Exp::new(rate).map(|d| d.cdf(x)).unwrap_or(f64::NAN),
            Self::Lognormal { mu, sigma } => {
                if x <= 0.0 {
                    0.0
                } else {
                    LogNormal::new(mu, sigma)
                        .map(|d| d.cdf(x))
                        .unwrap_or(f64::NAN)
                }
            }
            Self::Gamma { shape, scale } => {
                if x <= 0.0 {
                    0.0
                } else {
                    Gamma::new(shape, 1.0 / scale)
                        .map(|d| d.cdf(x))
                        .unwrap_or(f64::NAN)
                }
            }
            Self::Poisson { lambda } => {
                if x < 0.0 {
                    0.0
                } else {
                    Poisson::new(lambda)
                        .map(|d| d.cdf(x.floor() as u64))
                        .unwrap_or(f64::NAN)
                }
            }
        }
    }

    /// Log-likelihood of a sample (−∞ if any value is impossible).
    pub fn log_likelihood(&self, samples: &[f64]) -> f64 {
        let ll = |f: &dyn Fn(f64) -> f64| samples.iter().map(|&x| f(x)).sum::<f64>();
        match *self {
            Self::Exponential { rate } => Exp::new(rate)
                .map(|d| ll(&|x| d.ln_pdf(x)))
                .unwrap_or(f64::NAN),
            Self::Lognormal { mu, sigma } => LogNormal::new(mu, sigma)
                .map(|d| ll(&|x| d.ln_pdf(x)))
                .unwrap_or(f64::NAN),
            Self::Gamma { shape, scale } => Gamma::new(shape, 1.0 / scale)
                .map(|d| ll(&|x| d.ln_pdf(x)))
                .unwrap_or(f64::NAN),
            Self::Poisson { lambda } => Poisson::new(lambda)
                .map(|d| ll(&|x| d.ln_pmf(x.round().max(0.0) as u64)))
                .unwrap_or(f64::NAN),
        }
    }

    /// One draw (for the aggregate model). Callers pass a keyed stream.
    pub fn sample(&self, rng: &mut impl Rng) -> f64 {
        use rand::distributions::Distribution;
        match *self {
            Self::Exponential { rate } => Exp::new(rate).map(|d| d.sample(rng)).unwrap_or(f64::NAN),
            Self::Lognormal { mu, sigma } => LogNormal::new(mu, sigma)
                .map(|d| d.sample(rng))
                .unwrap_or(f64::NAN),
            Self::Gamma { shape, scale } => Gamma::new(shape, 1.0 / scale)
                .map(|d| d.sample(rng))
                .unwrap_or(f64::NAN),
            Self::Poisson { lambda } => Poisson::new(lambda)
                .map(|d| d.sample(rng))
                .unwrap_or(f64::NAN),
        }
    }
}

/// A fit with its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    pub distribution: FittedDistribution,
    pub n: usize,
    pub log_likelihood: f64,
    /// Akaike information criterion: 2p − 2 ln L (lower is better).
    pub aic: f64,
    /// KS against the fitted CDF (continuous families).
    pub ks: Option<KsResult>,
    /// Pearson chi-squared on binned counts (Poisson).
    pub chi_squared: Option<ChiSquaredResult>,
}

impl Fit {
    fn new(distribution: FittedDistribution, samples: &[f64]) -> Self {
        let ll = distribution.log_likelihood(samples);
        let continuous = !matches!(distribution, FittedDistribution::Poisson { .. });
        Fit {
            distribution,
            n: samples.len(),
            log_likelihood: ll,
            aic: 2.0 * distribution.parameters() as f64 - 2.0 * ll,
            ks: if continuous {
                ks_one_sample(samples, |x| distribution.cdf(x))
            } else {
                None
            },
            chi_squared: None,
        }
    }

    /// The goodness-of-fit p-value (KS or chi-squared), when there is one.
    pub fn gof_p_value(&self) -> Option<f64> {
        self.ks
            .map(|k| k.p_value)
            .or(self.chi_squared.as_ref().map(|c| c.p_value))
    }

    /// The fit is not rejected at level `alpha`.
    pub fn acceptable(&self, alpha: f64) -> bool {
        self.gof_p_value().is_none_or(|p| p >= alpha)
    }
}

fn positive(samples: &[f64], family: &'static str, need: usize) -> Result<Vec<f64>, FitError> {
    let v = sorted_finite(samples);
    if v.len() < need {
        return Err(FitError::TooFewSamples { need, got: v.len() });
    }
    if v.iter().any(|&x| x <= 0.0) {
        return Err(FitError::NonPositive { family });
    }
    Ok(v)
}

/// Settings for fits and their goodness-of-fit checks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FitOptions {
    /// A fit is acceptable when its goodness-of-fit p-value is at least this.
    pub alpha: f64,
    /// Chi-squared cells are merged to at least this expected count.
    pub min_expected: f64,
    /// Confidence level of rate intervals.
    pub level: f64,
}

impl FitOptions {
    pub fn from_figures(f: &Figures) -> Self {
        FitOptions {
            alpha: f.get("analysis.gof.alpha"),
            min_expected: f.get("analysis.gof.min_expected"),
            level: f.get("analysis.ci_level"),
        }
    }
}

/// Exponential MLE: λ = 1 / mean.
pub fn fit_exponential(samples: &[f64]) -> Result<Fit, FitError> {
    let v = positive(samples, "exponential", 1)?;
    Ok(Fit::new(
        FittedDistribution::Exponential {
            rate: 1.0 / mean(&v),
        },
        &v,
    ))
}

/// Lognormal MLE: μ, σ of ln x (σ with 1/n).
pub fn fit_lognormal(samples: &[f64]) -> Result<Fit, FitError> {
    let v = positive(samples, "lognormal", 2)?;
    let logs: Vec<f64> = v.iter().map(|x| x.ln()).collect();
    let mu = mean(&logs);
    let sigma = (logs.iter().map(|l| (l - mu).powi(2)).sum::<f64>() / logs.len() as f64).sqrt();
    if sigma <= 0.0 {
        return Err(FitError::Degenerate {
            family: "lognormal",
        });
    }
    Ok(Fit::new(FittedDistribution::Lognormal { mu, sigma }, &v))
}

/// Trigamma ψ₁(x) for x > 0 (recurrence then asymptotic series).
pub fn trigamma(mut x: f64) -> f64 {
    let mut acc = 0.0;
    while x < 10.0 {
        acc += 1.0 / (x * x);
        x += 1.0;
    }
    let x2 = 1.0 / (x * x);
    acc + 1.0 / x
        + x2 / 2.0
        + (1.0 / x) * x2 * (1.0 / 6.0 - x2 * (1.0 / 30.0 - x2 * (1.0 / 42.0 - x2 / 30.0)))
}

/// Gamma MLE: solve ln k − ψ(k) = ln x̄ − mean(ln x) by Newton from the
/// Minka/Choi–Wette start; θ = x̄ / k.
pub fn fit_gamma(samples: &[f64]) -> Result<Fit, FitError> {
    let v = positive(samples, "gamma", 2)?;
    let m = mean(&v);
    let s = m.ln() - v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64;
    if s <= 1e-12 {
        return Err(FitError::Degenerate { family: "gamma" });
    }
    let mut k = (3.0 - s + ((s - 3.0).powi(2) + 24.0 * s).sqrt()) / (12.0 * s);
    for _ in 0..50 {
        let g = k.ln() - digamma(k) - s;
        let dg = 1.0 / k - trigamma(k);
        let next = k - g / dg;
        let next = if next > 0.0 { next } else { k / 2.0 };
        if (next - k).abs() < 1e-12 * k {
            k = next;
            break;
        }
        k = next;
    }
    Ok(Fit::new(
        FittedDistribution::Gamma {
            shape: k,
            scale: m / k,
        },
        &v,
    ))
}

/// Poisson MLE for counts in equal windows: λ = mean, with a chi-squared
/// goodness-of-fit over count classes (cells merged to expected ≥ `min_expected`).
pub fn fit_poisson(counts: &[u64], min_expected: f64) -> Result<Fit, FitError> {
    if counts.is_empty() {
        return Err(FitError::TooFewSamples { need: 1, got: 0 });
    }
    let v: Vec<f64> = counts.iter().map(|&c| c as f64).collect();
    let lambda = mean(&v);
    let dist = FittedDistribution::Poisson { lambda };
    let mut fit = Fit::new(dist, &v);
    if lambda > 0.0 {
        let max = *counts.iter().max().unwrap_or(&0) as usize;
        let n = counts.len() as f64;
        let mut observed = vec![0.0; max + 1];
        for &c in counts {
            observed[c as usize] += 1.0;
        }
        let p = Poisson::new(lambda).map_err(|_| FitError::Degenerate { family: "poisson" })?;
        let mut expected: Vec<f64> = (0..=max).map(|k| n * p.pmf(k as u64)).collect();
        // The last class is "max or more".
        expected[max] = if max == 0 {
            n
        } else {
            n * (1.0 - p.cdf(max as u64 - 1))
        };
        fit.chi_squared = chi_squared_gof(&observed, &expected, 1, min_expected);
    }
    Ok(fit)
}

/// Fit every continuous family and rank by AIC (best first). Families that
/// cannot fit the data are left out.
pub fn fit_best(samples: &[f64]) -> Vec<Fit> {
    let mut fits: Vec<Fit> = [
        fit_exponential(samples),
        fit_lognormal(samples),
        fit_gamma(samples),
    ]
    .into_iter()
    .flatten()
    .collect();
    fits.sort_by(|a, b| a.aic.total_cmp(&b.aic));
    fits
}

/// A Poisson rate per unit exposure (events per hour, per shooter-hour…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateFit {
    /// MLE: total events / total exposure.
    pub rate: f64,
    /// Exact (Garwood) interval of the rate.
    pub interval: Interval,
    pub events: u64,
    pub exposure: f64,
    pub n: usize,
    /// Pearson dispersion Σ(k − λt)²/(λt) / (n − 1); ≫ 1 means overdispersed.
    pub dispersion: f64,
    /// Chi-squared p-value of the dispersion statistic (n − 1 dof).
    pub dispersion_p: f64,
}

/// Fit a Poisson rate from counts over exposures (e.g. losses over
/// shooter-hours). Zero-exposure observations are ignored.
pub fn fit_poisson_rate(
    counts: &[u64],
    exposures: &[f64],
    level: f64,
) -> Result<RateFit, FitError> {
    let obs: Vec<(u64, f64)> = counts
        .iter()
        .zip(exposures)
        .filter(|(_, &t)| t > 0.0 && t.is_finite())
        .map(|(&k, &t)| (k, t))
        .collect();
    if obs.is_empty() {
        return Err(FitError::NoExposure);
    }
    let events: u64 = obs.iter().map(|o| o.0).sum();
    let exposure: f64 = obs.iter().map(|o| o.1).sum();
    let rate = events as f64 / exposure;
    let a = 1.0 - level;
    let chi = |q: f64, dof: f64| {
        ChiSquared::new(dof)
            .map(|c| c.inverse_cdf(q))
            .unwrap_or(f64::NAN)
    };
    let lo = if events == 0 {
        0.0
    } else {
        chi(a / 2.0, 2.0 * events as f64) / 2.0 / exposure
    };
    let hi = chi(1.0 - a / 2.0, 2.0 * events as f64 + 2.0) / 2.0 / exposure;
    let (dispersion, dispersion_p) = if obs.len() > 1 && rate > 0.0 {
        let x2: f64 = obs
            .iter()
            .map(|&(k, t)| (k as f64 - rate * t).powi(2) / (rate * t))
            .sum();
        let dof = (obs.len() - 1) as f64;
        (
            x2 / dof,
            ChiSquared::new(dof).map(|c| c.sf(x2)).unwrap_or(f64::NAN),
        )
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(RateFit {
        rate,
        interval: Interval { lo, hi, level },
        events,
        exposure,
        n: obs.len(),
        dispersion,
        dispersion_p,
    })
}

// ───────────────────────── Stochastic Lanchester ─────────────────────────

/// How the loss rate of one side scales with the two strengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttritionLaw {
    /// Aimed fire: losses ∝ shooters (Lanchester square law).
    Square,
    /// Area fire: losses ∝ shooters × targets (linear law).
    Linear,
    /// Losses ∝ own targets (Peterson's logarithmic law; operational losses).
    Logarithmic,
}

impl AttritionLaw {
    pub fn exposure(&self, shooters: f64, targets: f64, duration: f64) -> f64 {
        match self {
            AttritionLaw::Square => shooters * duration,
            AttritionLaw::Linear => shooters * targets * duration,
            AttritionLaw::Logarithmic => targets * duration,
        }
    }
}

/// One entity-level engagement, reduced to what the aggregate model sees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EngagementObservation {
    /// Mean strength of the firing side over the interval (e.g. vehicles).
    pub shooters: f64,
    /// Mean strength of the side taking losses.
    pub targets: f64,
    /// Interval length (hours, by convention of the table).
    pub duration: f64,
    /// Losses suffered by the target side over the interval.
    pub losses: u64,
}

/// A fitted attrition coefficient: losses ~ Poisson(α · exposure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttritionFit {
    pub law: AttritionLaw,
    /// α, with its interval and dispersion check.
    pub rate: RateFit,
    pub log_likelihood: f64,
}

impl AttritionFit {
    /// Expected losses over an interval.
    pub fn expected_losses(&self, shooters: f64, targets: f64, duration: f64) -> f64 {
        self.rate.rate * self.law.exposure(shooters, targets, duration)
    }

    /// Draw losses for one aggregate step (capped at the targets present).
    pub fn sample_losses(
        &self,
        shooters: f64,
        targets: f64,
        duration: f64,
        rng: &mut impl Rng,
    ) -> u64 {
        use rand::distributions::Distribution;
        let lambda = self.expected_losses(shooters, targets, duration);
        if lambda <= 0.0 || !lambda.is_finite() {
            return 0;
        }
        let k = Poisson::new(lambda).map(|d| d.sample(rng)).unwrap_or(0.0) as u64;
        k.min(targets.max(0.0).floor() as u64)
    }
}

/// Fit an attrition coefficient under one law by Poisson MLE.
pub fn fit_attrition(
    obs: &[EngagementObservation],
    law: AttritionLaw,
    level: f64,
) -> Result<AttritionFit, FitError> {
    let counts: Vec<u64> = obs.iter().map(|o| o.losses).collect();
    let exposures: Vec<f64> = obs
        .iter()
        .map(|o| law.exposure(o.shooters, o.targets, o.duration))
        .collect();
    let rate = fit_poisson_rate(&counts, &exposures, level)?;
    let ll = counts
        .iter()
        .zip(&exposures)
        .filter(|(_, &t)| t > 0.0)
        .map(|(&k, &t)| {
            let mu = rate.rate * t;
            if mu > 0.0 {
                k as f64 * mu.ln() - mu - statrs::function::gamma::ln_gamma(k as f64 + 1.0)
            } else if k == 0 {
                0.0
            } else {
                f64::NEG_INFINITY
            }
        })
        .sum();
    Ok(AttritionFit {
        law,
        rate,
        log_likelihood: ll,
    })
}

/// Fit every law and return them best first (by log-likelihood; all have one parameter).
pub fn fit_attrition_best(obs: &[EngagementObservation], level: f64) -> Vec<AttritionFit> {
    let mut fits: Vec<AttritionFit> = [
        AttritionLaw::Square,
        AttritionLaw::Linear,
        AttritionLaw::Logarithmic,
    ]
    .into_iter()
    .filter_map(|law| fit_attrition(obs, law, level).ok())
    .collect();
    fits.sort_by(|a, b| {
        b.log_likelihood
            .total_cmp(&a.log_likelihood)
            .then(a.law.cmp(&b.law))
    });
    fits
}

// ───────────────────────── The coefficient table ─────────────────────────

/// What a coefficient set is keyed by (§8.13).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CoefficientKey {
    /// Formation type of the side whose behaviour is fitted (e.g. "mech-bn").
    pub own: String,
    /// Formation type of the opponent.
    pub opponent: String,
    /// Posture of `own` (e.g. "attack", "defend-prepared", "move").
    pub posture: String,
    /// Terrain class (e.g. "open", "mixed", "urban", "forest").
    pub terrain: String,
}

impl CoefficientKey {
    pub fn new(own: &str, opponent: &str, posture: &str, terrain: &str) -> Self {
        Self {
            own: own.into(),
            opponent: opponent.into(),
            posture: posture.into(),
            terrain: terrain.into(),
        }
    }
}

/// The fitted coefficients for one key.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CoefficientSet {
    /// Attrition fits by name, e.g. "own_losses", "opponent_losses".
    #[serde(default)]
    pub attrition: BTreeMap<String, AttritionFit>,
    /// Distribution fits by name, e.g. "advance_km_per_h", "fuel_l_per_h".
    #[serde(default)]
    pub distributions: BTreeMap<String, Fit>,
    /// Which entity-level runs it came from (build, figures fingerprint, seeds).
    #[serde(default)]
    pub provenance: String,
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoefficientEntry {
    pub key: CoefficientKey,
    pub set: CoefficientSet,
}

/// The meta-model's coefficient table, sorted by key. Serialises as a JSON
/// list of entries (struct keys cannot be JSON object keys).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CoefficientTable {
    pub entries: Vec<CoefficientEntry>,
}

impl CoefficientTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace, keeping key order.
    pub fn insert(&mut self, key: CoefficientKey, set: CoefficientSet) {
        match self.entries.binary_search_by(|e| e.key.cmp(&key)) {
            Ok(i) => self.entries[i].set = set,
            Err(i) => self.entries.insert(i, CoefficientEntry { key, set }),
        }
    }

    pub fn get(&self, key: &CoefficientKey) -> Option<&CoefficientSet> {
        self.entries
            .binary_search_by(|e| e.key.cmp(key))
            .ok()
            .map(|i| &self.entries[i].set)
    }

    pub fn get_mut(&mut self, key: &CoefficientKey) -> Option<&mut CoefficientSet> {
        self.entries
            .binary_search_by(|e| e.key.cmp(key))
            .ok()
            .map(|i| &mut self.entries[i].set)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Read a table; entries are re-sorted and later duplicates win.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let raw: CoefficientTable = serde_json::from_str(text)?;
        let mut t = CoefficientTable::new();
        for e in raw.entries {
            t.insert(e.key, e.set);
        }
        Ok(t)
    }
}

// ───────────────────────── Agreement checks ─────────────────────────

/// Settings for agreement between a fitted aggregate model and new samples.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AgreementOptions {
    /// Significance level: the fit agrees when the test does not reject.
    pub alpha: f64,
    /// Relative error of the mean considered practically close.
    pub rel_tol: f64,
}

impl AgreementOptions {
    pub fn from_figures(f: &Figures) -> Self {
        AgreementOptions {
            alpha: f.get("analysis.agreement.alpha"),
            rel_tol: f.get("analysis.agreement.rel_tol"),
        }
    }
}

/// Does the fitted model still describe the entity model?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agreement {
    pub quantity: String,
    pub n: usize,
    pub predicted_mean: f64,
    pub observed_mean: f64,
    pub relative_error: f64,
    /// The test's p-value (KS for distributions, Poisson for attrition).
    pub p_value: f64,
    /// Not rejected at `alpha`.
    pub agrees: bool,
    /// Mean within `rel_tol`, whatever the test says.
    pub practically_close: bool,
    pub explanation: String,
}

fn rel_err(pred: f64, obs: f64) -> f64 {
    (obs - pred).abs() / pred.abs().max(1e-12)
}

/// Check a fitted distribution against new entity-level samples (KS).
pub fn distribution_agreement(
    quantity: &str,
    fit: &FittedDistribution,
    samples: &[f64],
    options: &AgreementOptions,
) -> Agreement {
    let v = sorted_finite(samples);
    let ks = ks_one_sample(&v, |x| fit.cdf(x));
    let (pm, om) = (fit.mean(), mean(&v));
    let re = rel_err(pm, om);
    let p = ks.map_or(f64::NAN, |k| k.p_value);
    let agrees = p >= options.alpha;
    Agreement {
        quantity: quantity.into(),
        n: v.len(),
        predicted_mean: pm,
        observed_mean: om,
        relative_error: re,
        p_value: p,
        agrees,
        practically_close: re <= options.rel_tol,
        explanation: format!(
            "{} fit ({}) vs {} new samples: KS p = {:.3} ({}); mean {:.4} predicted, {:.4} observed ({:.1}% off).",
            quantity,
            fit.family(),
            v.len(),
            p,
            if agrees { "agrees" } else { "rejected — refit" },
            pm,
            om,
            re * 100.0
        ),
    }
}

/// Check a fitted attrition coefficient against new engagements: is the
/// observed total of losses plausible under Poisson(Σ α·exposure)? (two-sided).
pub fn attrition_agreement(
    quantity: &str,
    fit: &AttritionFit,
    obs: &[EngagementObservation],
    options: &AgreementOptions,
) -> Agreement {
    let predicted: f64 = obs
        .iter()
        .map(|o| fit.expected_losses(o.shooters, o.targets, o.duration))
        .sum();
    let observed: u64 = obs.iter().map(|o| o.losses).sum();
    let p = match Poisson::new(predicted.max(1e-12)) {
        Ok(d) => {
            let lower = d.cdf(observed);
            let upper = if observed == 0 {
                1.0
            } else {
                1.0 - d.cdf(observed - 1)
            };
            (2.0 * lower.min(upper)).min(1.0)
        }
        Err(_) => f64::NAN,
    };
    let re = rel_err(predicted, observed as f64);
    let agrees = p >= options.alpha;
    Agreement {
        quantity: quantity.into(),
        n: obs.len(),
        predicted_mean: predicted,
        observed_mean: observed as f64,
        relative_error: re,
        p_value: p,
        agrees,
        practically_close: re <= options.rel_tol,
        explanation: format!(
            "{} ({:?} law, α = {:.4}): {:.1} losses predicted over {} engagements, {} observed; Poisson p = {:.3} ({}).",
            quantity,
            fit.law,
            fit.rate.rate,
            predicted,
            obs.len(),
            observed,
            p,
            if agrees { "agrees" } else { "rejected — refit" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use bgws_core::{Seed, StreamKey};
    use rand::distributions::Distribution;

    fn draws(d: &FittedDistribution, n: usize, seed: u64) -> Vec<f64> {
        let mut rng = Seed(seed).stream(StreamKey::new("test.fit", 0, 0, 0));
        (0..n).map(|_| d.sample(&mut rng)).collect()
    }

    #[test]
    fn recovers_continuous_parameters() {
        let e = fit_exponential(&draws(
            &FittedDistribution::Exponential { rate: 0.25 },
            4000,
            1,
        ))
        .unwrap();
        let FittedDistribution::Exponential { rate } = e.distribution else {
            panic!()
        };
        assert_relative_eq!(rate, 0.25, max_relative = 0.05);
        assert!(e.acceptable(0.01));

        let l = fit_lognormal(&draws(
            &FittedDistribution::Lognormal {
                mu: 1.0,
                sigma: 0.5,
            },
            4000,
            2,
        ))
        .unwrap();
        let FittedDistribution::Lognormal { mu, sigma } = l.distribution else {
            panic!()
        };
        assert_relative_eq!(mu, 1.0, epsilon = 0.03);
        assert_relative_eq!(sigma, 0.5, epsilon = 0.03);

        let g = fit_gamma(&draws(
            &FittedDistribution::Gamma {
                shape: 3.0,
                scale: 2.0,
            },
            4000,
            3,
        ))
        .unwrap();
        let FittedDistribution::Gamma { shape, scale } = g.distribution else {
            panic!()
        };
        assert_relative_eq!(shape, 3.0, max_relative = 0.08);
        assert_relative_eq!(scale, 2.0, max_relative = 0.08);
        assert!(g.acceptable(0.01));
    }

    #[test]
    fn trigamma_known_values() {
        assert_relative_eq!(
            trigamma(1.0),
            std::f64::consts::PI.powi(2) / 6.0,
            epsilon = 1e-10
        );
        assert_relative_eq!(
            trigamma(0.5),
            std::f64::consts::PI.powi(2) / 2.0,
            epsilon = 1e-10
        );
    }

    #[test]
    fn aic_prefers_the_true_family_and_ks_rejects_the_wrong_one() {
        let x = draws(
            &FittedDistribution::Lognormal {
                mu: 0.0,
                sigma: 1.0,
            },
            3000,
            4,
        );
        let best = fit_best(&x);
        assert_eq!(best.len(), 3);
        assert_eq!(best[0].distribution.family(), "lognormal");
        let exp = best
            .iter()
            .find(|f| f.distribution.family() == "exponential")
            .unwrap();
        assert!(!exp.acceptable(0.01));
        assert!(matches!(
            fit_lognormal(&[1.0, -1.0]),
            Err(FitError::NonPositive { .. })
        ));
        assert!(matches!(
            fit_gamma(&[2.0, 2.0, 2.0]),
            Err(FitError::Degenerate { .. })
        ));
    }

    #[test]
    fn poisson_counts_and_rates() {
        let d = Poisson::new(3.0).unwrap();
        let mut rng = Seed(5).stream(StreamKey::new("test.pois", 0, 0, 0));
        let counts: Vec<u64> = (0..2000).map(|_| d.sample(&mut rng) as u64).collect();
        let f = fit_poisson(&counts, 5.0).unwrap();
        assert_relative_eq!(f.distribution.mean(), 3.0, epsilon = 0.1);
        let chi = f.chi_squared.clone().unwrap();
        assert!(chi.p_value > 0.01, "{chi:?}");
        // Rates with exposure: 30 events over 10 h, Garwood 95% ≈ [2.024, 4.283].
        let r = fit_poisson_rate(&[10, 20], &[5.0, 5.0], 0.95).unwrap();
        assert_relative_eq!(r.rate, 3.0);
        assert_relative_eq!(r.interval.lo, 2.024, epsilon = 2e-3);
        assert_relative_eq!(r.interval.hi, 4.283, epsilon = 2e-3);
        assert!(matches!(
            fit_poisson_rate(&[1], &[0.0], 0.95),
            Err(FitError::NoExposure)
        ));
    }

    /// Synthetic "entity-level" engagements generated under the square law.
    fn engagements(alpha: f64, n: usize, seed: u64) -> Vec<EngagementObservation> {
        let mut rng = Seed(seed).stream(StreamKey::new("test.lanchester", 0, 0, 0));
        (0..n)
            .map(|i| {
                let shooters = 4.0 + (i % 9) as f64;
                let targets = 30.0 - (i % 7) as f64 * 3.0;
                let duration = 0.5 + (i % 4) as f64 * 0.5;
                let lambda = alpha * shooters * duration;
                let losses = Poisson::new(lambda).unwrap().sample(&mut rng) as u64;
                EngagementObservation {
                    shooters,
                    targets,
                    duration,
                    losses,
                }
            })
            .collect()
    }

    #[test]
    fn lanchester_fit_table_and_agreement() {
        let obs = engagements(0.15, 600, 6);
        let fits = fit_attrition_best(&obs, 0.95);
        assert_eq!(fits[0].law, AttritionLaw::Square);
        let sq = &fits[0];
        assert!(sq.rate.interval.contains(0.15), "{:?}", sq.rate);
        assert!(sq.rate.dispersion_p > 0.01);

        let mut table = CoefficientTable::new();
        let key = CoefficientKey::new("armour-coy", "mech-inf-coy", "attack", "open");
        let mut set = CoefficientSet {
            provenance: "synthetic test".into(),
            ..Default::default()
        };
        set.attrition.insert("opponent_losses".into(), sq.clone());
        set.distributions.insert(
            "advance_km_per_h".into(),
            fit_gamma(&draws(
                &FittedDistribution::Gamma {
                    shape: 4.0,
                    scale: 0.5,
                },
                500,
                7,
            ))
            .unwrap(),
        );
        table.insert(key.clone(), set);
        table.insert(
            CoefficientKey::new("armour-coy", "mech-inf-coy", "attack", "forest"),
            CoefficientSet::default(),
        );
        assert_eq!(table.entries[0].key.terrain, "forest");
        let back = CoefficientTable::from_json(&table.to_json().unwrap()).unwrap();
        assert_eq!(back, table);

        let opts = AgreementOptions {
            alpha: 0.01,
            rel_tol: 0.15,
        };
        let fit = &back.get(&key).unwrap().attrition["opponent_losses"];
        let same = attrition_agreement("opponent_losses", fit, &engagements(0.15, 300, 8), &opts);
        assert!(same.agrees, "{}", same.explanation);
        let changed =
            attrition_agreement("opponent_losses", fit, &engagements(0.25, 300, 9), &opts);
        assert!(
            !changed.agrees && !changed.practically_close,
            "{}",
            changed.explanation
        );

        let adv = back.get(&key).unwrap().distributions["advance_km_per_h"].distribution;
        let ok = distribution_agreement(
            "advance",
            &adv,
            &draws(
                &FittedDistribution::Gamma {
                    shape: 4.0,
                    scale: 0.5,
                },
                400,
                10,
            ),
            &opts,
        );
        assert!(ok.agrees, "{}", ok.explanation);
        let bad = distribution_agreement(
            "advance",
            &adv,
            &draws(
                &FittedDistribution::Gamma {
                    shape: 4.0,
                    scale: 0.8,
                },
                400,
                11,
            ),
            &opts,
        );
        assert!(!bad.agrees);

        let mut rng = Seed(1).stream(StreamKey::new("t", 0, 0, 0));
        assert!(fit.sample_losses(10.0, 3.0, 10.0, &mut rng) <= 3);
    }
}
