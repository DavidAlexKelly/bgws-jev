//! Sensitivity analysis (RUST_ENGINE.md §15.1, §15.4): Morris
//! elementary-effects screening across the figures, each varied within its
//! declared `[min, max]`, then Sobol first-order and total indices for the
//! top-k (Saltelli sampling, Jansen estimators). The result is the research
//! backlog: unsourced (`Confidence::Default`) figures ranked by how much they
//! move the outcome, so the ones that matter are researched first.
//!
//! Also the TypeScript build's one-at-a-time sweep (`batch.ts`
//! `sensitivity`): each driving figure at both ends of its range, and whether
//! the vignette's conclusion changes.
//!
//! The samplers work in the unit hypercube and are validated on analytic
//! functions (a linear model's Morris μ*, the Ishigami function's Sobol
//! indices); the figure-level wrappers map unit coordinates to each figure's
//! range. Every design is drawn from keyed streams, and every model evaluation
//! is collected in design order, so results do not depend on threads (§12).
//!
//! References: Morris (1991); Campolongo, Cariboni & Saltelli (2007) for μ*;
//! Saltelli et al. (2010) and Jansen (1999) for the estimators.

use crate::batch::{run_batch, BatchConfig, Experiment, Metric};
use crate::stats::{quantile, Interval};
use crate::vignettes::{fmt, CheckOptions, Statistic, Verdict, Vignette};
use bgws_core::{Confidence, Figure, Figures, Seed, StreamKey};
use rand::seq::SliceRandom;
use rand::Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// A figure varied by the analysis, with its declared range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Factor {
    pub key: String,
    pub base: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub source: String,
    pub confidence: Confidence,
}

impl Factor {
    pub fn from_figure(f: &Figure) -> Self {
        Factor {
            key: f.key.clone(),
            base: f.value,
            min: f.min,
            max: f.max,
            unit: f.unit.clone(),
            source: f.source.clone(),
            confidence: f.confidence,
        }
    }

    /// Unit coordinate → value in range (clamped against rounding).
    pub fn value_at(&self, x: f64) -> f64 {
        (self.min + x * (self.max - self.min)).clamp(self.min, self.max)
    }
}

/// The figures that can be varied (a non-empty range) and pass `include`,
/// in key order.
pub fn factors(figures: &Figures, include: impl Fn(&Figure) -> bool) -> Vec<Factor> {
    figures.iter().filter(|f| f.max > f.min && include(f)).map(Factor::from_figure).collect()
}

/// A scalar response of the whole model to a set of figures.
pub trait Response: Sync {
    fn eval(&self, figures: &Figures) -> f64;
}

impl<F: Fn(&Figures) -> f64 + Sync> Response for F {
    fn eval(&self, figures: &Figures) -> f64 {
        self(figures)
    }
}

/// A batch of an experiment, reduced to one statistic: the response used for
/// sensitivity of a vignette. Every design point runs the same replicate
/// seeds (common random numbers), so noise mostly cancels in the differences.
pub struct BatchResponse<'a, E, M> {
    pub experiment: &'a E,
    pub metric: &'a M,
    pub config: BatchConfig,
    pub statistic: Statistic,
}

impl<E, M> Response for BatchResponse<'_, E, M>
where
    E: Experiment,
    M: Metric<E::Output>,
{
    fn eval(&self, figures: &Figures) -> f64 {
        let batch = run_batch(self.experiment, figures, self.config);
        let sample = batch.sample(self.metric);
        self.statistic.estimate(&sample.values, 0.95).0
    }
}

/// Evaluate a response at unit-hypercube points, mapping each to figures.
fn eval_points(figures: &Figures, factors: &[Factor], response: &impl Response, points: &[Vec<f64>]) -> Vec<f64> {
    points
        .par_iter()
        .map(|x| {
            let mut f = figures.clone();
            for (fac, &xi) in factors.iter().zip(x) {
                // In range by construction; an unknown key is a caller error
                // that `factors` cannot produce.
                f.set(&fac.key, fac.value_at(xi)).expect("factor values stay in their declared range");
            }
            response.eval(&f)
        })
        .collect()
}

// ───────────────────────────── Morris ─────────────────────────────

/// Morris screening settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MorrisOptions {
    /// Trajectories r (each costs k + 1 evaluations).
    pub trajectories: usize,
    /// Grid levels p (even); the step is Δ = p / (2(p − 1)).
    pub levels: usize,
}

impl MorrisOptions {
    pub fn from_figures(f: &Figures) -> Self {
        MorrisOptions {
            trajectories: f.get("analysis.morris.trajectories").round() as usize,
            levels: f.get("analysis.morris.levels").round() as usize,
        }
    }

    pub fn delta(&self) -> f64 {
        let p = self.levels.max(2) as f64;
        p / (2.0 * (p - 1.0))
    }
}

/// One trajectory: k + 1 points, each differing from the previous in one factor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    pub points: Vec<Vec<f64>>,
    /// Which factor changes at step t (between points t and t + 1).
    pub order: Vec<usize>,
    /// The signed step at step t (±Δ).
    pub steps: Vec<f64>,
}

/// The Morris design: `r` random trajectories on the p-level grid.
pub fn morris_design(k: usize, options: &MorrisOptions, seed: Seed) -> Vec<Trajectory> {
    let p = (options.levels.max(2) / 2 * 2).max(2);
    let delta = options.delta();
    (0..options.trajectories)
        .map(|t| {
            let mut rng = seed.stream(StreamKey::new("analysis.morris", t as u64, k as u64, 0));
            let levels: Vec<usize> = (0..k).map(|_| rng.gen_range(0..p)).collect();
            let mut x: Vec<f64> = levels.iter().map(|&j| j as f64 / (p - 1) as f64).collect();
            let mut order: Vec<usize> = (0..k).collect();
            order.shuffle(&mut rng);
            let mut points = vec![x.clone()];
            let mut steps = Vec::with_capacity(k);
            for &i in &order {
                // Low half of the grid steps up, high half steps down: always in [0, 1].
                let step = if levels[i] < p / 2 { delta } else { -delta };
                x[i] = (x[i] + step).clamp(0.0, 1.0);
                steps.push(step);
                points.push(x.clone());
            }
            Trajectory { points, order, steps }
        })
        .collect()
}

/// Elementary-effect statistics for one factor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorrisEffect {
    pub index: usize,
    pub key: String,
    /// Mean elementary effect (signed; effects of opposite sign cancel).
    pub mu: f64,
    /// Mean absolute elementary effect: the ranking measure.
    pub mu_star: f64,
    /// Standard deviation of the effects: non-linearity or interaction.
    pub sigma: f64,
    /// μ* as a share of the largest μ*.
    pub mu_star_normalised: f64,
}

/// Morris results, ranked by μ* (largest first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorrisResult {
    pub options: MorrisOptions,
    pub evaluations: usize,
    pub effects: Vec<MorrisEffect>,
}

fn morris_effects(k: usize, design: &[Trajectory], y: &[f64], names: &[String], options: MorrisOptions) -> MorrisResult {
    let mut ee: Vec<Vec<f64>> = vec![Vec::new(); k];
    let mut at = 0;
    for tr in design {
        for (t, (&i, &step)) in tr.order.iter().zip(&tr.steps).enumerate() {
            let d = (y[at + t + 1] - y[at + t]) / step;
            if d.is_finite() {
                ee[i].push(d);
            }
        }
        at += tr.points.len();
    }
    let mut effects: Vec<MorrisEffect> = ee
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let n = e.len().max(1) as f64;
            let mu = e.iter().sum::<f64>() / n;
            let mu_star = e.iter().map(|x| x.abs()).sum::<f64>() / n;
            let sigma = if e.len() > 1 { (e.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / (n - 1.0)).sqrt() } else { 0.0 };
            MorrisEffect { index: i, key: names[i].clone(), mu, mu_star, sigma, mu_star_normalised: 0.0 }
        })
        .collect();
    let top = effects.iter().map(|e| e.mu_star).fold(0.0, f64::max);
    for e in &mut effects {
        e.mu_star_normalised = if top > 0.0 { e.mu_star / top } else { 0.0 };
    }
    effects.sort_by(|a, b| b.mu_star.total_cmp(&a.mu_star).then(a.index.cmp(&b.index)));
    MorrisResult { options, evaluations: y.len(), effects }
}

/// Morris screening of a function on the unit hypercube [0, 1]^k.
pub fn morris_unit(k: usize, f: &(dyn Fn(&[f64]) -> f64 + Sync), options: &MorrisOptions, seed: Seed) -> MorrisResult {
    let design = morris_design(k, options, seed);
    let points: Vec<&Vec<f64>> = design.iter().flat_map(|t| t.points.iter()).collect();
    let y: Vec<f64> = points.par_iter().map(|x| f(x)).collect();
    let names: Vec<String> = (0..k).map(|i| format!("x{}", i + 1)).collect();
    morris_effects(k, &design, &y, &names, *options)
}

/// Morris screening of a response over figures, each varied in its range.
/// Elementary effects are per unit fraction of each figure's range.
pub fn morris_figures(figures: &Figures, factors: &[Factor], response: &impl Response, options: &MorrisOptions, seed: Seed) -> MorrisResult {
    let k = factors.len();
    let design = morris_design(k, options, seed);
    let points: Vec<Vec<f64>> = design.iter().flat_map(|t| t.points.iter().cloned()).collect();
    let y = eval_points(figures, factors, response, &points);
    let names: Vec<String> = factors.iter().map(|f| f.key.clone()).collect();
    morris_effects(k, &design, &y, &names, *options)
}

// ───────────────────────────── Sobol ─────────────────────────────

/// Sobol settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SobolOptions {
    /// Base samples N (cost N·(k + 2) evaluations).
    pub base_samples: usize,
    /// Bootstrap resamples for the intervals (0 = none).
    pub bootstrap: usize,
    /// Confidence level of the bootstrap intervals.
    pub level: f64,
    /// How many of the Morris top factors get Sobol indices.
    pub top_k: usize,
}

impl SobolOptions {
    pub fn from_figures(f: &Figures) -> Self {
        SobolOptions {
            base_samples: f.get("analysis.sobol.base_samples").round() as usize,
            bootstrap: f.get("analysis.sobol.bootstrap").round() as usize,
            level: f.get("analysis.ci_level"),
            top_k: f.get("analysis.sobol.top_k").round() as usize,
        }
    }
}

/// Sobol indices for one factor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SobolIndex {
    pub index: usize,
    pub key: String,
    /// First-order index S_i: the share of variance from this factor alone.
    pub first: f64,
    /// Total index S_Ti: including every interaction it takes part in.
    pub total: f64,
    pub first_ci: Option<Interval>,
    pub total_ci: Option<Interval>,
}

/// Sobol results, ranked by total index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SobolResult {
    pub base_samples: usize,
    pub evaluations: usize,
    /// Output variance.
    pub variance: f64,
    pub indices: Vec<SobolIndex>,
}

/// The Saltelli design: rows of A, B and each A_B^(i), as unit points in the
/// order [A; B; A_B^1; …; A_B^k].
fn saltelli_points(k: usize, n: usize, seed: Seed) -> Vec<Vec<f64>> {
    let row = |r: usize| -> (Vec<f64>, Vec<f64>) {
        let mut rng = seed.stream(StreamKey::new("analysis.sobol", r as u64, k as u64, 0));
        let a: Vec<f64> = (0..k).map(|_| rng.gen()).collect();
        let b: Vec<f64> = (0..k).map(|_| rng.gen()).collect();
        (a, b)
    };
    let rows: Vec<(Vec<f64>, Vec<f64>)> = (0..n).map(row).collect();
    let mut pts = Vec::with_capacity(n * (k + 2));
    pts.extend(rows.iter().map(|r| r.0.clone()));
    pts.extend(rows.iter().map(|r| r.1.clone()));
    for i in 0..k {
        pts.extend(rows.iter().map(|(a, b)| {
            let mut x = a.clone();
            x[i] = b[i];
            x
        }));
    }
    pts
}

/// Jansen estimators over the rows `idx` of the evaluated design.
fn jansen(k: usize, n: usize, y: &[f64], idx: &[usize]) -> (Vec<f64>, Vec<f64>, f64) {
    let fa = |r: usize| y[r];
    let fb = |r: usize| y[n + r];
    let fab = |i: usize, r: usize| y[(2 + i) * n + r];
    let m = idx.len() as f64;
    let mean = idx.iter().map(|&r| fa(r) + fb(r)).sum::<f64>() / (2.0 * m);
    let var = idx.iter().map(|&r| (fa(r) - mean).powi(2) + (fb(r) - mean).powi(2)).sum::<f64>() / (2.0 * m);
    let mut first = Vec::with_capacity(k);
    let mut total = Vec::with_capacity(k);
    for i in 0..k {
        let s1 = idx.iter().map(|&r| (fb(r) - fab(i, r)).powi(2)).sum::<f64>() / (2.0 * m);
        let st = idx.iter().map(|&r| (fa(r) - fab(i, r)).powi(2)).sum::<f64>() / (2.0 * m);
        if var > 0.0 {
            first.push((var - s1) / var);
            total.push(st / var);
        } else {
            first.push(0.0);
            total.push(0.0);
        }
    }
    (first, total, var)
}

fn sobol_from(k: usize, n: usize, y: &[f64], names: &[String], options: &SobolOptions, seed: Seed) -> SobolResult {
    let all: Vec<usize> = (0..n).collect();
    let (first, total, variance) = jansen(k, n, y, &all);
    let mut cis: Vec<(Option<Interval>, Option<Interval>)> = vec![(None, None); k];
    if options.bootstrap > 0 && n > 1 {
        let boots: Vec<(Vec<f64>, Vec<f64>, f64)> = (0..options.bootstrap)
            .into_par_iter()
            .map(|b| {
                let mut rng = seed.stream(StreamKey::new("analysis.sobol.bootstrap", b as u64, 0, 0));
                let idx: Vec<usize> = (0..n).map(|_| rng.gen_range(0..n)).collect();
                jansen(k, n, y, &idx)
            })
            .collect();
        let lo = (1.0 - options.level) / 2.0;
        for (i, ci) in cis.iter_mut().enumerate() {
            let s: Vec<f64> = boots.iter().map(|b| b.0[i]).collect();
            let t: Vec<f64> = boots.iter().map(|b| b.1[i]).collect();
            *ci = (
                Some(Interval { lo: quantile(&s, lo), hi: quantile(&s, 1.0 - lo), level: options.level }),
                Some(Interval { lo: quantile(&t, lo), hi: quantile(&t, 1.0 - lo), level: options.level }),
            );
        }
    }
    let mut indices: Vec<SobolIndex> = (0..k)
        .map(|i| SobolIndex { index: i, key: names[i].clone(), first: first[i], total: total[i], first_ci: cis[i].0, total_ci: cis[i].1 })
        .collect();
    indices.sort_by(|a, b| b.total.total_cmp(&a.total).then(a.index.cmp(&b.index)));
    SobolResult { base_samples: n, evaluations: y.len(), variance, indices }
}

/// Sobol indices of a function on the unit hypercube [0, 1]^k.
pub fn sobol_unit(k: usize, f: &(dyn Fn(&[f64]) -> f64 + Sync), options: &SobolOptions, seed: Seed) -> SobolResult {
    let n = options.base_samples.max(2);
    let pts = saltelli_points(k, n, seed);
    let y: Vec<f64> = pts.par_iter().map(|x| f(x)).collect();
    let names: Vec<String> = (0..k).map(|i| format!("x{}", i + 1)).collect();
    sobol_from(k, n, &y, &names, options, seed)
}

/// Sobol indices of a response over figures, the others held at base value.
pub fn sobol_figures(figures: &Figures, factors: &[Factor], response: &impl Response, options: &SobolOptions, seed: Seed) -> SobolResult {
    let k = factors.len();
    let n = options.base_samples.max(2);
    let pts = saltelli_points(k, n, seed);
    let y = eval_points(figures, factors, response, &pts);
    let names: Vec<String> = factors.iter().map(|f| f.key.clone()).collect();
    sobol_from(k, n, &y, &names, options, seed)
}

// ───────────────────────── The full analysis ─────────────────────────

/// Morris across every factor, then Sobol for the top-k, then the backlog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensitivityReport {
    /// What was analysed (a vignette id, or a label).
    pub subject: String,
    pub factors: Vec<Factor>,
    pub morris: MorrisResult,
    pub sobol: Option<SobolResult>,
    pub backlog: ResearchBacklog,
}

/// Screen `factors` with Morris, then compute Sobol indices for the top
/// `sobol.top_k` by μ* (skipped when `top_k` is 0), and rank the backlog.
pub fn analyse(
    subject: &str,
    figures: &Figures,
    factors: &[Factor],
    response: &impl Response,
    morris: &MorrisOptions,
    sobol: &SobolOptions,
    seed: Seed,
) -> SensitivityReport {
    let m = morris_figures(figures, factors, response, morris, seed);
    let top: Vec<Factor> = m.effects.iter().take(sobol.top_k).filter(|e| e.mu_star > 0.0).map(|e| factors[e.index].clone()).collect();
    let s = (!top.is_empty()).then(|| sobol_figures(figures, &top, response, sobol, seed));
    let backlog = ResearchBacklog::rank(factors, &m, s.as_ref());
    SensitivityReport { subject: subject.into(), factors: factors.to_vec(), morris: m, sobol: s, backlog }
}

/// One unsourced figure in the research backlog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacklogEntry {
    pub rank: usize,
    pub key: String,
    pub value: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub source: String,
    pub mu_star: f64,
    pub mu_star_normalised: f64,
    pub sigma: f64,
    /// Sobol total index, when the figure made the top-k.
    pub sobol_total: Option<f64>,
    pub sobol_first: Option<f64>,
}

/// Unsourced figures ranked by sensitivity: research these first (§15.1, §15.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchBacklog {
    pub entries: Vec<BacklogEntry>,
}

impl ResearchBacklog {
    /// Rank the `Confidence::Default` factors: Sobol total index first (for
    /// the figures that have one), then normalised μ*, then key.
    pub fn rank(factors: &[Factor], morris: &MorrisResult, sobol: Option<&SobolResult>) -> Self {
        let mut entries: Vec<BacklogEntry> = factors
            .iter()
            .filter(|f| f.confidence == Confidence::Default)
            .map(|f| {
                let m = morris.effects.iter().find(|e| e.key == f.key);
                let s = sobol.and_then(|s| s.indices.iter().find(|i| i.key == f.key));
                BacklogEntry {
                    rank: 0,
                    key: f.key.clone(),
                    value: f.base,
                    unit: f.unit.clone(),
                    min: f.min,
                    max: f.max,
                    source: f.source.clone(),
                    mu_star: m.map_or(0.0, |e| e.mu_star),
                    mu_star_normalised: m.map_or(0.0, |e| e.mu_star_normalised),
                    sigma: m.map_or(0.0, |e| e.sigma),
                    sobol_total: s.map(|i| i.total),
                    sobol_first: s.map(|i| i.first),
                }
            })
            .collect();
        entries.sort_by(|a, b| {
            let sa = a.sobol_total.unwrap_or(-1.0);
            let sb = b.sobol_total.unwrap_or(-1.0);
            sb.total_cmp(&sa).then(b.mu_star_normalised.total_cmp(&a.mu_star_normalised)).then(a.key.cmp(&b.key))
        });
        for (i, e) in entries.iter_mut().enumerate() {
            e.rank = i + 1;
        }
        ResearchBacklog { entries }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn to_markdown(&self) -> String {
        let mut s = String::from("## Research backlog\n\nUnsourced figures (**[default]**), ranked by how much they move the outcome. Research these first.\n\n");
        s.push_str("| # | Figure | Value | Range | μ* (norm.) | σ | Sobol S_T | Sobol S_1 |\n|---|---|---|---|---|---|---|---|\n");
        for e in &self.entries {
            s.push_str(&format!(
                "| {} | `{}` | {} {} | {}–{} | {} ({:.0}%) | {} | {} | {} |\n",
                e.rank,
                e.key,
                fmt(e.value),
                e.unit,
                fmt(e.min),
                fmt(e.max),
                fmt(e.mu_star),
                e.mu_star_normalised * 100.0,
                fmt(e.sigma),
                e.sobol_total.map(|x| format!("{x:.3}")).unwrap_or_else(|| "—".into()),
                e.sobol_first.map(|x| format!("{x:.3}")).unwrap_or_else(|| "—".into()),
            ));
        }
        s
    }
}

// ───────────────────── One-at-a-time sweep (TypeScript port) ─────────────────────

/// One end of a figure's range in the one-at-a-time sweep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SweepEnd {
    pub value: f64,
    pub estimate: f64,
    pub in_band: bool,
}

/// One driving figure in the sweep (TypeScript `SensitivityRow`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SweepRow {
    pub figure: String,
    pub base: f64,
    pub low: SweepEnd,
    pub high: SweepEnd,
    /// The verdict (in band or not) differs at one end of the range.
    pub changes_conclusion: bool,
    /// |high − low| as a share of the base estimate.
    pub swing: f64,
}

/// The one-at-a-time sweep of a vignette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sweep {
    pub vignette: String,
    pub base_estimate: f64,
    pub base_in_band: bool,
    pub rows: Vec<SweepRow>,
}

/// Play the vignette with each figure in `keys` at its minimum and maximum
/// (others at base), on the same seeds, and rank by swing. Unknown keys are
/// skipped.
pub fn one_at_a_time<E: Experiment>(
    vignette: &Vignette,
    experiment: &E,
    metric: &impl Metric<E::Output>,
    figures: &Figures,
    keys: &[String],
    base_seed: Seed,
    options: &CheckOptions,
) -> Sweep {
    let base = vignette.check(experiment, metric, figures, base_seed, options);
    let end = |key: &str, value: f64| {
        let mut f = figures.clone();
        f.set(key, value).ok()?;
        let r = vignette.check(experiment, metric, &f, base_seed, options);
        Some(SweepEnd { value, estimate: r.estimate, in_band: r.verdict == Verdict::InBand })
    };
    let base_in = base.verdict == Verdict::InBand;
    let mut rows: Vec<SweepRow> = keys
        .iter()
        .filter_map(|key| {
            let fig = figures.figure(key)?.clone();
            let low = end(key, fig.min)?;
            let high = end(key, fig.max)?;
            Some(SweepRow {
                figure: key.clone(),
                base: fig.value,
                changes_conclusion: low.in_band != base_in || high.in_band != base_in,
                swing: (high.estimate - low.estimate).abs() / base.estimate.abs().max(1e-9),
                low,
                high,
            })
        })
        .collect();
    rows.sort_by(|a, b| b.swing.total_cmp(&a.swing).then(a.figure.cmp(&b.figure)));
    Sweep { vignette: vignette.id.clone(), base_estimate: base.estimate, base_in_band: base_in, rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;
    use bgws_core::figure;
    use std::f64::consts::PI;

    fn ishigami(x: &[f64]) -> f64 {
        let (a, b) = (7.0, 0.1);
        let v: Vec<f64> = x.iter().map(|u| -PI + 2.0 * PI * u).collect();
        v[0].sin() + a * v[1].sin().powi(2) + b * v[2].powi(4) * v[0].sin()
    }

    #[test]
    fn morris_linear_model_mu_star_is_the_coefficient() {
        let c = [3.0, -2.0, 0.5, 0.0];
        let f = move |x: &[f64]| x.iter().zip(&c).map(|(a, b)| a * b).sum::<f64>();
        let r = morris_unit(4, &f, &MorrisOptions { trajectories: 10, levels: 4 }, Seed(1));
        assert_eq!(r.evaluations, 10 * 5);
        for e in &r.effects {
            assert_abs_diff_eq!(e.mu_star, c[e.index].abs(), epsilon = 1e-9);
            assert_abs_diff_eq!(e.mu, c[e.index], epsilon = 1e-9);
            assert_abs_diff_eq!(e.sigma, 0.0, epsilon = 1e-9);
        }
        assert_eq!(r.effects[0].key, "x1");
        assert_eq!(r.effects[3].mu_star, 0.0);
    }

    #[test]
    fn morris_design_stays_on_grid() {
        let d = morris_design(6, &MorrisOptions { trajectories: 30, levels: 6 }, Seed(9));
        for tr in &d {
            assert_eq!(tr.points.len(), 7);
            let mut seen = tr.order.clone();
            seen.sort();
            assert_eq!(seen, (0..6).collect::<Vec<_>>());
            for x in tr.points.iter().flatten() {
                assert!((0.0..=1.0).contains(x));
            }
        }
    }

    #[test]
    fn morris_flags_ishigami_interaction() {
        let r = morris_unit(3, &ishigami, &MorrisOptions { trajectories: 50, levels: 4 }, Seed(2));
        let x3 = r.effects.iter().find(|e| e.index == 2).unwrap();
        // x3 acts only through its interaction with x1: μ ≈ 0 relative to μ*, σ > 0.
        assert!(x3.sigma > 0.0 && x3.mu_star > 0.0);
    }

    #[test]
    fn sobol_ishigami_matches_analytic_indices() {
        let opts = SobolOptions { base_samples: 8192, bootstrap: 50, level: 0.95, top_k: 3 };
        let r = sobol_unit(3, &ishigami, &opts, Seed(3));
        let by = |i: usize| r.indices.iter().find(|s| s.index == i).unwrap().clone();
        // Analytic (a = 7, b = 0.1): S = (0.3139, 0.4424, 0), S_T = (0.5576, 0.4424, 0.2437).
        let (s1, s2, s3) = (by(0), by(1), by(2));
        assert_abs_diff_eq!(s1.first, 0.3139, epsilon = 0.04);
        assert_abs_diff_eq!(s2.first, 0.4424, epsilon = 0.04);
        assert_abs_diff_eq!(s3.first, 0.0, epsilon = 0.04);
        assert_abs_diff_eq!(s1.total, 0.5576, epsilon = 0.04);
        assert_abs_diff_eq!(s2.total, 0.4424, epsilon = 0.04);
        assert_abs_diff_eq!(s3.total, 0.2437, epsilon = 0.04);
        assert_abs_diff_eq!(r.variance, 13.8446, epsilon = 0.5);
        assert!(s1.total_ci.unwrap().width() > 0.0);
        assert_eq!(r.indices[0].index, 0);
    }

    #[test]
    fn figures_analysis_and_backlog() {
        let figs = Figures::from_list([
            figure!("m.a", 1.0, "s", 0.0, 2.0, "default", Default, ""),
            figure!("m.b", 1.0, "s", 0.0, 10.0, "P14", Medium, ""),
            figure!("m.c", 1.0, "s", 0.0, 1.0, "default", Default, ""),
            figure!("m.d", 5.0, "s", 5.0, 5.0, "default", Default, "fixed"),
        ])
        .unwrap();
        // y = 2a + b + 0.1c: per unit of range, μ* = 4, 10, 0.1.
        let resp = |f: &Figures| 2.0 * f.get("m.a") + f.get("m.b") + 0.1 * f.get("m.c");
        let fac = factors(&figs, |_| true);
        assert_eq!(fac.len(), 3, "a fixed figure cannot be varied");
        let m = MorrisOptions { trajectories: 8, levels: 4 };
        let s = SobolOptions { base_samples: 512, bootstrap: 0, level: 0.95, top_k: 2 };
        let rep = analyse("linear", &figs, &fac, &resp, &m, &s, Seed(4));
        let mu = |k: &str| rep.morris.effects.iter().find(|e| e.key == k).unwrap().mu_star;
        assert_abs_diff_eq!(mu("m.a"), 4.0, epsilon = 1e-9);
        assert_abs_diff_eq!(mu("m.b"), 10.0, epsilon = 1e-9);
        assert_abs_diff_eq!(mu("m.c"), 0.1, epsilon = 1e-9);
        let sobol = rep.sobol.as_ref().unwrap();
        assert_eq!(sobol.indices.len(), 2);
        assert_eq!(sobol.indices[0].key, "m.b");
        // Backlog: only the defaults, sobol-ranked first.
        let keys: Vec<_> = rep.backlog.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["m.a", "m.c"]);
        assert!(rep.backlog.entries[0].sobol_total.is_some());
        assert!(rep.backlog.to_markdown().contains("| 1 | `m.a`"));
        let back: ResearchBacklog = serde_json::from_str(&rep.backlog.to_json()).unwrap();
        assert_eq!(back, rep.backlog);
    }

    #[test]
    fn thread_count_does_not_change_results() {
        let opts = SobolOptions { base_samples: 256, bootstrap: 20, level: 0.95, top_k: 3 };
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let four = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let a = one.install(|| sobol_unit(3, &ishigami, &opts, Seed(8)));
        let b = four.install(|| sobol_unit(3, &ishigami, &opts, Seed(8)));
        assert_eq!(a, b);
    }

    #[test]
    fn one_at_a_time_sweep_reports_both_ends() {
        let figs = Figures::from_list(
            crate::figures().into_iter().chain([figure!("t.p", 0.85, "fraction", 0.5, 0.95, "default", Default, "")]),
        )
        .unwrap();
        let v = crate::vignettes::find("ballistic-missile-defence").unwrap();
        let exp = |f: &Figures, seed: Seed| {
            let x: f64 = seed.stream(StreamKey::new("t", 0, 0, 0)).gen();
            if x < f.get("t.p") { 1.0 } else { 0.0 }
        };
        let opts = CheckOptions { level: 0.95, replicates: Some(400) };
        let sweep = one_at_a_time(&v, &exp, &crate::batch::Scalar, &figs, &["t.p".into(), "nope".into()], Seed(1), &opts);
        assert_eq!(sweep.rows.len(), 1);
        let row = &sweep.rows[0];
        assert!(row.low.value < row.high.value);
        assert!(row.changes_conclusion);
        assert!(row.swing > 0.3);
    }
}
