//! The batch runner (RUST_ENGINE.md §1, §13, §15.3): N seeded replicates of
//! an experiment, run in parallel with `rayon`, with replicate seeds derived
//! deterministically from one base seed (`Seed::derive`), and results
//! collected in replicate order whatever the thread scheduling (§12).
//!
//! The runner is generic: an [`Experiment`] is anything that plays one seeded
//! run against a set of figures and returns an output (a whole game's outcome
//! from `bgws-sim`, or a cheap model call). A [`Metric`] pulls one number out
//! of that output; a replicate that produced nothing to measure yields `None`
//! (the TypeScript vignettes' `undefined`).
//!
//! Target (§13): 1,000 one-hour battalion vignettes in under ten minutes on
//! 8–16 cores. The runner adds only the collection overhead; see the test
//! `thousand_replicates_are_fast_and_thread_count_independent`.

use crate::stats::{Histogram, Summary};
use bgws_core::{Figures, Seed};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};

/// One seeded run of something worth measuring.
pub trait Experiment: Sync {
    type Output: Serialize + Send;
    /// Play one replicate. Must be a pure function of `(figures, seed)`.
    fn run(&self, figures: &Figures, seed: Seed) -> Self::Output;
}

/// Any `Fn(&Figures, Seed) -> O` is an experiment.
impl<O, F> Experiment for F
where
    F: Fn(&Figures, Seed) -> O + Sync,
    O: Serialize + Send,
{
    type Output = O;
    fn run(&self, figures: &Figures, seed: Seed) -> O {
        self(figures, seed)
    }
}

/// Pulls one number out of an experiment's output.
pub trait Metric<O>: Sync {
    /// Short name, used in reports.
    fn name(&self) -> &str;
    /// Unit symbol, used in reports.
    fn unit(&self) -> &str {
        ""
    }
    /// The measured value; `None` when this replicate produced nothing to measure.
    fn extract(&self, output: &O) -> Option<f64>;
}

/// A metric built from a name, a unit and a closure.
pub struct MetricFn<F> {
    pub name: String,
    pub unit: String,
    pub f: F,
}

impl<F> MetricFn<F> {
    pub fn new(name: impl Into<String>, unit: impl Into<String>, f: F) -> Self {
        Self {
            name: name.into(),
            unit: unit.into(),
            f,
        }
    }
}

impl<O, F: Fn(&O) -> Option<f64> + Sync> Metric<O> for MetricFn<F> {
    fn name(&self) -> &str {
        &self.name
    }
    fn unit(&self) -> &str {
        &self.unit
    }
    fn extract(&self, output: &O) -> Option<f64> {
        (self.f)(output)
    }
}

/// The identity metric for experiments that already return one number.
pub struct Scalar;

impl Metric<f64> for Scalar {
    fn name(&self) -> &str {
        "value"
    }
    fn extract(&self, output: &f64) -> Option<f64> {
        output.is_finite().then_some(*output)
    }
}

impl Metric<Option<f64>> for Scalar {
    fn name(&self) -> &str {
        "value"
    }
    fn extract(&self, output: &Option<f64>) -> Option<f64> {
        output.filter(|x| x.is_finite())
    }
}

/// How many replicates, from which base seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchConfig {
    pub replicates: usize,
    pub base_seed: Seed,
}

impl BatchConfig {
    pub fn new(replicates: usize, base_seed: Seed) -> Self {
        Self {
            replicates,
            base_seed,
        }
    }

    /// The seed of replicate `i`: `base_seed.derive(i)`. Stable across builds.
    pub fn seed_of(&self, i: usize) -> Seed {
        self.base_seed.derive(i as u64)
    }
}

/// Progress, reported after each replicate finishes (from worker threads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchProgress {
    pub done: usize,
    pub total: usize,
}

/// One replicate's seed and output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replicate<O> {
    pub index: usize,
    pub seed: Seed,
    pub output: O,
}

/// Every replicate of a batch, in replicate order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch<O> {
    pub config: BatchConfig,
    pub replicates: Vec<Replicate<O>>,
}

impl<O> Batch<O> {
    /// The metric for every replicate, in order (`None` = nothing measured).
    pub fn metric(&self, metric: &impl Metric<O>) -> Vec<Option<f64>> {
        self.replicates
            .iter()
            .map(|r| metric.extract(&r.output).filter(|x| x.is_finite()))
            .collect()
    }

    /// The metric as a [`Sample`].
    pub fn sample(&self, metric: &impl Metric<O>) -> Sample {
        Sample::from_options(metric.name(), metric.unit(), &self.metric(metric))
    }
}

/// Run a batch with no progress reporting. Uses the current rayon pool
/// (wrap in `ThreadPool::install` to choose the thread count).
pub fn run_batch<E: Experiment>(
    exp: &E,
    figures: &Figures,
    config: BatchConfig,
) -> Batch<E::Output> {
    run_batch_with_progress(exp, figures, config, &|_| {})
}

/// Run a batch, calling `progress` after each replicate. The callback runs on
/// worker threads, in completion order; the results do not depend on it.
pub fn run_batch_with_progress<E: Experiment>(
    exp: &E,
    figures: &Figures,
    config: BatchConfig,
    progress: &(dyn Fn(BatchProgress) + Sync),
) -> Batch<E::Output> {
    let done = AtomicUsize::new(0);
    let total = config.replicates;
    // `collect` on an indexed parallel iterator keeps index order.
    let replicates: Vec<Replicate<E::Output>> = (0..total)
        .into_par_iter()
        .map(|index| {
            let seed = config.seed_of(index);
            let output = exp.run(figures, seed);
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(BatchProgress { done: d, total });
            Replicate {
                index,
                seed,
                output,
            }
        })
        .collect();
    Batch { config, replicates }
}

/// One metric's values over a batch, with the count of replicates that
/// measured nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub metric: String,
    pub unit: String,
    /// Measured values, in replicate order.
    pub values: Vec<f64>,
    pub missing: usize,
}

impl Sample {
    pub fn new(metric: impl Into<String>, unit: impl Into<String>, values: Vec<f64>) -> Self {
        let finite: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
        let missing = values.len() - finite.len();
        Sample {
            metric: metric.into(),
            unit: unit.into(),
            values: finite,
            missing,
        }
    }

    pub fn from_options(metric: &str, unit: &str, values: &[Option<f64>]) -> Self {
        let measured: Vec<f64> = values.iter().flatten().copied().collect();
        Sample {
            metric: metric.into(),
            unit: unit.into(),
            missing: values.len() - measured.len(),
            values: measured,
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Summary statistics at confidence `level`.
    pub fn summary(&self, level: f64) -> Summary {
        let mut s = Summary::of(&self.values, level);
        s.missing += self.missing;
        s
    }

    /// A stable fingerprint of the values (FNV-1a over their bit patterns),
    /// used by the regression store to recognise an accepted distribution.
    pub fn fingerprint(&self) -> String {
        let mut bytes = Vec::with_capacity(self.values.len() * 8 + 8);
        for v in &self.values {
            bytes.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        bytes.extend_from_slice(&(self.missing as u64).to_le_bytes());
        format!("{:016x}", bgws_core::rng::fnv1a(&bytes))
    }
}

/// What the workbench and the CLI print for a batch (§16): the TypeScript
/// `BatchResult` shape, generalised to any metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchSummary {
    pub label: String,
    pub metric: String,
    pub unit: String,
    pub runs: usize,
    pub base_seed: Seed,
    pub summary: Summary,
    pub histogram: Histogram,
}

impl BatchSummary {
    pub fn new(
        label: impl Into<String>,
        config: BatchConfig,
        sample: &Sample,
        level: f64,
        bins: usize,
    ) -> Self {
        BatchSummary {
            label: label.into(),
            metric: sample.metric.clone(),
            unit: sample.unit.clone(),
            runs: config.replicates,
            base_seed: config.base_seed,
            summary: sample.summary(level),
            histogram: Histogram::of(&sample.values, bins),
        }
    }

    /// With the confidence level and bin count from the figures.
    pub fn from_figures(
        label: impl Into<String>,
        config: BatchConfig,
        sample: &Sample,
        figures: &Figures,
    ) -> Self {
        let bins = figures.get("analysis.histogram.bins").round() as usize;
        Self::new(
            label,
            config,
            sample,
            figures.get("analysis.ci_level"),
            bins,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{figure, StreamKey};
    use rand::Rng;
    use std::sync::Mutex;

    fn figs() -> Figures {
        Figures::from_list([figure!(
            "test.p", 0.3, "fraction", 0.0, 1.0, "default", Default, ""
        )])
        .unwrap()
    }

    /// A cheap stochastic experiment: 50 Bernoulli trials and a sum of draws.
    fn cheap(figures: &Figures, seed: Seed) -> (u32, f64) {
        let p = figures.get("test.p");
        let mut rng = seed.stream(StreamKey::new("test.cheap", 0, 0, 0));
        let hits = (0..50).filter(|_| rng.gen::<f64>() < p).count() as u32;
        let s: f64 = (0..200).map(|_| rng.gen::<f64>()).sum();
        (hits, s)
    }

    #[test]
    fn thousand_replicates_are_fast_and_thread_count_independent() {
        let f = figs();
        let cfg = BatchConfig::new(1000, Seed(7));
        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let four = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let t0 = std::time::Instant::now();
        let a = one.install(|| run_batch(&cheap, &f, cfg));
        let b = four.install(|| run_batch(&cheap, &f, cfg));
        assert!(t0.elapsed().as_secs_f64() < 5.0);
        assert_eq!(a.replicates.len(), 1000);
        for (x, y) in a.replicates.iter().zip(&b.replicates) {
            assert_eq!(x.index, y.index);
            assert_eq!(x.seed, y.seed);
            assert_eq!(x.output.0, y.output.0);
            assert_eq!(x.output.1.to_bits(), y.output.1.to_bits());
        }
        let m = MetricFn::new("hits", "count", |o: &(u32, f64)| Some(o.0 as f64 / 50.0));
        let s = a.sample(&m);
        assert_eq!(s.fingerprint(), b.sample(&m).fingerprint());
        let summary = s.summary(0.95);
        assert!((summary.mean - 0.3).abs() < 0.01, "{summary:?}");
        let ci = summary.mean_ci.unwrap();
        assert!(ci.contains(0.3));
    }

    #[test]
    fn progress_reaches_total_and_missing_values_are_counted() {
        let f = figs();
        let seen = Mutex::new(Vec::new());
        let exp = |_: &Figures, seed: Seed| {
            if seed.0.is_multiple_of(3) {
                None
            } else {
                Some((seed.0 % 100) as f64)
            }
        };
        let batch = run_batch_with_progress(&exp, &f, BatchConfig::new(60, Seed(1)), &|p| {
            seen.lock().unwrap().push(p.done)
        });
        let mut seen = seen.into_inner().unwrap();
        seen.sort();
        assert_eq!(seen, (1..=60).collect::<Vec<_>>());
        let s = batch.sample(&Scalar);
        assert_eq!(s.len() + s.missing, 60);
        let bs = BatchSummary::new("t", batch.config, &s, 0.95, 10);
        assert_eq!(bs.summary.n + bs.summary.missing, 60);
        assert_eq!(bs.histogram.total() as usize, s.len());
    }

    #[test]
    fn seeds_are_derived() {
        let cfg = BatchConfig::new(3, Seed(99));
        assert_eq!(cfg.seed_of(2), Seed(99).derive(2));
        assert_ne!(cfg.seed_of(1), cfg.seed_of(2));
    }
}
