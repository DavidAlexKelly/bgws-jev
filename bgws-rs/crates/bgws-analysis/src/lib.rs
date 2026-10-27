//! Batches, vignettes, distributions, sensitivity (Morris, Sobol) and
//! meta-models: the calibration and validation layer of the BGWS engine
//! (RUST_ENGINE.md §15, §17 "statistical", §8.13, §13 batch target, §16
//! analysis workbench).
//!
//! The crate is generic over what it runs: anything implementing
//! [`batch::Experiment`] (a pure function of figures and a seed) can be
//! batched, checked against a vignette's band, screened for sensitivity and
//! compared with a stored baseline. The adapter that plays a `bgws-sim`
//! scenario as an experiment lives with the integrator.
//!
//! - [`stats`]: summaries, t / Wilson / order-statistic intervals, histograms.
//! - [`batch`]: the parallel, thread-count-independent batch runner.
//! - [`vignettes`]: vignettes as hypotheses, the checker and the catalogue.
//! - [`compare`]: KS (one- and two-sample), chi-squared, Mann–Whitney.
//! - [`regression`]: the stored-distribution regression gate with acceptances.
//! - [`sensitivity`]: Morris, Sobol (Saltelli/Jansen), research backlog, sweeps.
//! - [`fit`]: MLE fits, goodness of fit, Lanchester coefficients, the table.
//! - [`report`]: Markdown and JSON for the CLI and the workbench.

pub mod batch;
pub mod compare;
pub mod fit;
pub mod regression;
pub mod report;
pub mod sensitivity;
pub mod stats;
pub mod vignettes;

pub use batch::{
    run_batch, run_batch_with_progress, Batch, BatchConfig, BatchProgress, BatchSummary,
    Experiment, Metric, MetricFn, Sample, Scalar,
};
pub use vignettes::{Band, CheckOptions, Statistic, Verdict, Vignette, VignetteResult};

use bgws_core::{figure, Figure};

/// The analysis crate's own figures: methodological settings, not model
/// inputs. They are `[default]` choices from standard practice, and are not
/// ranked in the research backlog unless a caller chooses to vary them.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!(
            "analysis.ci_level",
            0.95,
            "fraction",
            0.8,
            0.999,
            "default",
            Default,
            "confidence level of every reported interval"
        ),
        figure!(
            "analysis.histogram.bins",
            20,
            "count",
            2,
            200,
            "default",
            Default,
            "bins in batch histograms"
        ),
        figure!(
            "analysis.regression.ks_alpha",
            0.01,
            "fraction",
            0.0001,
            0.1,
            "default",
            Default,
            "KS significance for 'significantly shifted' (§15.8)"
        ),
        figure!(
            "analysis.morris.trajectories",
            20,
            "count",
            4,
            500,
            "default",
            Default,
            "Morris trajectories r; cost r(k+1) runs"
        ),
        figure!(
            "analysis.morris.levels",
            4,
            "count",
            2,
            16,
            "default",
            Default,
            "Morris grid levels p (even)"
        ),
        figure!(
            "analysis.sobol.base_samples",
            1024,
            "count",
            64,
            65536,
            "default",
            Default,
            "Saltelli base samples N; cost N(k+2) runs"
        ),
        figure!(
            "analysis.sobol.bootstrap",
            100,
            "count",
            0,
            2000,
            "default",
            Default,
            "bootstrap resamples for Sobol intervals"
        ),
        figure!(
            "analysis.sobol.top_k",
            5,
            "count",
            1,
            50,
            "default",
            Default,
            "figures from Morris that get Sobol indices"
        ),
        figure!(
            "analysis.gof.alpha",
            0.05,
            "fraction",
            0.001,
            0.2,
            "default",
            Default,
            "goodness-of-fit significance for meta-model fits"
        ),
        figure!(
            "analysis.gof.min_expected",
            5,
            "count",
            1,
            20,
            "default",
            Default,
            "chi-squared cells merged to at least this expected count (Cochran)"
        ),
        figure!(
            "analysis.agreement.alpha",
            0.01,
            "fraction",
            0.0001,
            0.2,
            "default",
            Default,
            "significance for aggregate-vs-entity agreement (§8.13)"
        ),
        figure!(
            "analysis.agreement.rel_tol",
            0.15,
            "fraction",
            0.01,
            0.5,
            "default",
            Default,
            "relative error of the mean counted as practically close"
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::Figures;

    #[test]
    fn figures_build() {
        let f = Figures::from_list(figures()).unwrap();
        assert_eq!(f.len(), figures().len());
        let m = sensitivity::MorrisOptions::from_figures(&f);
        assert_eq!(m.levels % 2, 0);
        let s = sensitivity::SobolOptions::from_figures(&f);
        assert!(s.base_samples >= 64);
        let _ = regression::RegressionOptions::from_figures(&f);
        let _ = fit::AgreementOptions::from_figures(&f);
        let _ = CheckOptions::from_figures(&f);
        assert!(f.get("analysis.histogram.bins") >= 2.0);
        assert!(f.get("analysis.gof.alpha") > 0.0 && f.get("analysis.gof.min_expected") >= 1.0);
        assert!(f.unused().is_empty(), "unused: {:?}", f.unused());
    }
}
