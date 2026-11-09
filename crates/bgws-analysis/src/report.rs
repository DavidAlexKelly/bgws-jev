//! Reports (RUST_ENGINE.md §15, §16): Markdown and JSON summaries of
//! batches, vignettes, sensitivity and regression checks — what the analysis
//! workbench shows and the CLI (`bgws-run batch | vignettes | sensitivity`)
//! prints. The Markdown tables follow the TypeScript `sensitivityReport`.
//!
//! Also the run-outcome shape the TypeScript headless runner reported
//! (`headless.ts` `SideOutcome`), so a sim adapter has a ready output type
//! with metrics for "who was left standing".

use crate::batch::{BatchSummary, MetricFn};
use crate::compare::SampleComparison;
use crate::regression::{RegressionStatus, RegressionVerdict};
use crate::sensitivity::{SensitivityReport, Sweep};
use crate::vignettes::{fmt, Verdict, VignetteResult};
use bgws_core::{Side, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Any report as pretty JSON.
pub fn to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}

/// How one run ended (the TypeScript `SideOutcome`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    /// Why it ended ("victory blue", "time limit", "red broke"), or None if still running.
    pub over: Option<String>,
    pub time: SimTime,
    /// Fighting strength left, as a share of the start, by side.
    pub strength: BTreeMap<Side, f64>,
}

/// The metric "strength left for `side`".
pub fn strength_metric(side: Side) -> MetricFn<impl Fn(&RunOutcome) -> Option<f64> + Sync> {
    MetricFn::new(
        format!("{side:?}_strength").to_lowercase(),
        "share",
        move |o: &RunOutcome| o.strength.get(&side).copied(),
    )
}

/// The metric "game length" in seconds.
pub fn duration_metric() -> MetricFn<impl Fn(&RunOutcome) -> Option<f64> + Sync> {
    MetricFn::new("duration", "s", |o: &RunOutcome| {
        o.over.as_ref().map(|_| o.time.0 as f64 / 1e6)
    })
}

/// A batch as Markdown.
pub fn batch_markdown(b: &BatchSummary) -> String {
    let s = &b.summary;
    let ci = s
        .mean_ci
        .map(|i| {
            format!(
                " ({:.0}% CI {} – {})",
                i.level * 100.0,
                fmt(i.lo),
                fmt(i.hi)
            )
        })
        .unwrap_or_default();
    let mut out = format!(
        "## {}\n\n`{}` over {} runs (base seed {}): {} measured, {} missing.\n\n",
        b.label, b.metric, b.runs, b.base_seed.0, s.n, s.missing
    );
    out.push_str("| Mean | SD | Min | P10 | P25 | Median | P75 | P90 | Max |\n|---|---|---|---|---|---|---|---|---|\n");
    out.push_str(&format!(
        "| {}{} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
        fmt(s.mean),
        ci,
        fmt(s.sd),
        fmt(s.min),
        fmt(s.p10),
        fmt(s.p25),
        fmt(s.p50),
        fmt(s.p75),
        fmt(s.p90),
        fmt(s.max)
    ));
    out.push_str(&format!(
        "\nUnit: {}.\n",
        if b.unit.is_empty() { "—" } else { &b.unit }
    ));
    out
}

/// A set of vignette results with its tally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VignetteSuite {
    pub results: Vec<VignetteResult>,
    pub in_band: usize,
    pub out_of_band: usize,
    pub no_data: usize,
}

impl VignetteSuite {
    pub fn new(results: Vec<VignetteResult>) -> Self {
        let count = |v: Verdict| results.iter().filter(|r| r.verdict == v).count();
        VignetteSuite {
            in_band: count(Verdict::InBand),
            out_of_band: count(Verdict::OutOfBand),
            no_data: count(Verdict::NoData),
            results,
        }
    }

    pub fn all_in_band(&self) -> bool {
        self.out_of_band == 0 && self.no_data == 0
    }

    pub fn to_markdown(&self) -> String {
        vignettes_markdown(&self.results)
    }
}

/// Vignette results as Markdown: a table, then one line of evidence each.
pub fn vignettes_markdown(results: &[VignetteResult]) -> String {
    let mut s = String::from("## Benchmark vignettes\n\n| Vignette | Statistic | Estimate | Interval | Band | Runs | Verdict |\n|---|---|---|---|---|---|---|\n");
    for r in results {
        let ci = r
            .interval
            .map(|i| format!("{} – {}", fmt(i.lo), fmt(i.hi)))
            .unwrap_or_else(|| "—".into());
        let verdict = match r.verdict {
            Verdict::InBand => "in band",
            Verdict::OutOfBand => "**out of band**",
            Verdict::NoData => "**no data**",
        };
        s.push_str(&format!(
            "| {} [{}] | {} | {} {} | {} | {} – {} | {} | {} |\n",
            r.title,
            r.source,
            r.band.statistic.label(),
            fmt(r.estimate),
            r.unit,
            ci,
            fmt(r.band.lo),
            fmt(r.band.hi),
            r.summary.n + r.summary.missing,
            verdict
        ));
    }
    s.push('\n');
    for r in results {
        s.push_str(&format!(
            "- **{}** — {} {}\n",
            r.id, r.hypothesis, r.explanation
        ));
    }
    s
}

/// A sensitivity analysis as Markdown: Morris table, Sobol table, backlog.
pub fn sensitivity_markdown(r: &SensitivityReport) -> String {
    let mut s = format!(
        "## Sensitivity: {}\n\nMorris screening: {} factors, {} trajectories, {} levels, {} evaluations. μ* is per unit fraction of each figure's range.\n\n",
        r.subject,
        r.factors.len(),
        r.morris.options.trajectories,
        r.morris.options.levels,
        r.morris.evaluations
    );
    s.push_str("| Figure | μ* | μ* (norm.) | μ | σ | Source |\n|---|---|---|---|---|---|\n");
    for e in &r.morris.effects {
        let src = r
            .factors
            .get(e.index)
            .map(|f| {
                if f.confidence == bgws_core::Confidence::Default {
                    "**[default]**".to_string()
                } else {
                    f.source.clone()
                }
            })
            .unwrap_or_default();
        s.push_str(&format!(
            "| `{}` | {} | {:.0}% | {} | {} | {} |\n",
            e.key,
            fmt(e.mu_star),
            e.mu_star_normalised * 100.0,
            fmt(e.mu),
            fmt(e.sigma),
            src
        ));
    }
    if let Some(sobol) = &r.sobol {
        s.push_str(&format!(
            "\nSobol indices for the top {} (N = {}, {} evaluations, output variance {}):\n\n| Figure | S₁ | S_T |\n|---|---|---|\n",
            sobol.indices.len(),
            sobol.base_samples,
            sobol.evaluations,
            fmt(sobol.variance)
        ));
        for i in &sobol.indices {
            let ci = |c: Option<crate::stats::Interval>| {
                c.map(|c| format!(" [{:.3}, {:.3}]", c.lo, c.hi))
                    .unwrap_or_default()
            };
            s.push_str(&format!(
                "| `{}` | {:.3}{} | {:.3}{} |\n",
                i.key,
                i.first,
                ci(i.first_ci),
                i.total,
                ci(i.total_ci)
            ));
        }
    }
    s.push('\n');
    s.push_str(&r.backlog.to_markdown());
    s
}

/// The one-at-a-time sweep as Markdown (the TypeScript report's table).
pub fn sweep_markdown(sweep: &Sweep) -> String {
    let mut s = format!(
        "## Sweep: {}\n\nBase estimate {}; {}.\n\n| Figure | Base | Low → estimate | High → estimate | Swing | Conclusion |\n|---|---|---|---|---|---|\n",
        sweep.vignette,
        fmt(sweep.base_estimate),
        if sweep.base_in_band { "in band" } else { "**out of band**" }
    );
    for r in &sweep.rows {
        s.push_str(&format!(
            "| {} | {} | {} → {} | {} → {} | {:.0}% | {} |\n",
            r.figure,
            fmt(r.base),
            fmt(r.low.value),
            fmt(r.low.estimate),
            fmt(r.high.value),
            fmt(r.high.estimate),
            r.swing * 100.0,
            if r.changes_conclusion {
                "**changes**"
            } else {
                "holds"
            }
        ));
    }
    s
}

/// Regression verdicts as Markdown.
pub fn regression_markdown(verdicts: &[RegressionVerdict]) -> String {
    let mut s = String::from("## Regression\n\n| Vignette | Status | Baseline | New | KS D | KS p |\n|---|---|---|---|---|---|\n");
    for v in verdicts {
        let status = match v.status {
            RegressionStatus::Pass => "pass",
            RegressionStatus::New => "new baseline",
            RegressionStatus::Accepted => "accepted",
            RegressionStatus::Shifted => "**shifted**",
            RegressionStatus::OutOfBand => "**out of band**",
        };
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            v.id,
            status,
            v.baseline_estimate.map(fmt).unwrap_or_else(|| "—".into()),
            fmt(v.new_estimate),
            v.ks.map(|k| format!("{:.3}", k.d))
                .unwrap_or_else(|| "—".into()),
            v.ks.map(|k| format!("{:.3}", k.p_value))
                .unwrap_or_else(|| "—".into()),
        ));
    }
    s.push('\n');
    for v in verdicts
        .iter()
        .filter(|v| v.status != RegressionStatus::Pass)
    {
        s.push_str(&format!("- **{}** — {}\n", v.id, v.explanation));
    }
    s
}

/// Two samples compared, as Markdown.
pub fn comparison_markdown(c: &SampleComparison) -> String {
    let row = |label: &str, s: &crate::stats::Summary| {
        format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            label,
            s.n,
            fmt(s.mean),
            fmt(s.sd),
            fmt(s.p10),
            fmt(s.p50),
            fmt(s.p90)
        )
    };
    let mut out = format!(
        "## {} vs {}\n\n| | n | Mean | SD | P10 | Median | P90 |\n|---|---|---|---|---|---|---|\n",
        c.label_a, c.label_b
    );
    out.push_str(&row(&c.label_a, &c.a));
    out.push_str(&row(&c.label_b, &c.b));
    out.push('\n');
    if let Some(d) = c.mean_difference {
        out.push_str(&format!(
            "- Mean difference ({} − {}): {:.0}% CI {} – {}\n",
            c.label_b,
            c.label_a,
            d.level * 100.0,
            fmt(d.lo),
            fmt(d.hi)
        ));
    }
    if let Some(k) = c.ks {
        out.push_str(&format!(
            "- Kolmogorov–Smirnov: D = {:.3}, p = {:.3}\n",
            k.d, k.p_value
        ));
    }
    if let Some(m) = c.mann_whitney {
        out.push_str(&format!(
            "- Mann–Whitney: U = {}, p = {:.3}, P({} > {}) = {:.2}\n",
            m.u, m.p_value, c.label_a, c.label_b, m.effect
        ));
    }
    out
}

/// Everything the analysis workbench (§16) loads in one document.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkbenchData {
    #[serde(default)]
    pub batches: Vec<BatchSummary>,
    #[serde(default)]
    pub vignettes: Vec<VignetteResult>,
    #[serde(default)]
    pub sensitivity: Vec<SensitivityReport>,
    #[serde(default)]
    pub sweeps: Vec<Sweep>,
    #[serde(default)]
    pub comparisons: Vec<SampleComparison>,
    #[serde(default)]
    pub regression: Vec<RegressionVerdict>,
}

impl WorkbenchData {
    pub fn to_markdown(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.batches.iter().map(batch_markdown));
        if !self.vignettes.is_empty() {
            parts.push(vignettes_markdown(&self.vignettes));
        }
        parts.extend(self.sensitivity.iter().map(sensitivity_markdown));
        parts.extend(self.sweeps.iter().map(sweep_markdown));
        parts.extend(self.comparisons.iter().map(comparison_markdown));
        if !self.regression.is_empty() {
            parts.push(regression_markdown(&self.regression));
        }
        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{run_batch, BatchConfig, Metric, Sample};
    use crate::compare::compare_samples;
    use crate::regression::{compare_with, RegressionOptions};
    use crate::sensitivity::{analyse, factors, MorrisOptions, SobolOptions};
    use crate::vignettes::find;
    use bgws_core::{figure, Figures, Seed, StreamKey};
    use rand::Rng;

    #[test]
    fn reports_render_and_round_trip() {
        let figs = Figures::from_list([figure!(
            "x.k", 0.5, "fraction", 0.1, 0.9, "default", Default, ""
        )])
        .unwrap();
        let exp = |f: &Figures, seed: Seed| {
            let mut rng = seed.stream(StreamKey::new("t", 0, 0, 0));
            let k = f.get("x.k");
            RunOutcome {
                over: Some("time limit".into()),
                time: SimTime(3_600_000_000),
                strength: [(Side::Blue, k * rng.gen::<f64>()), (Side::Red, 0.5)]
                    .into_iter()
                    .collect(),
            }
        };
        let cfg = BatchConfig::new(50, Seed(3));
        let batch = run_batch(&exp, &figs, cfg);
        let m = strength_metric(Side::Blue);
        assert_eq!(m.name(), "blue_strength");
        let sample = batch.sample(&m);
        let bs = BatchSummary::new("blue left", cfg, &sample, 0.95, 10);
        let md = batch_markdown(&bs);
        assert!(md.contains("| Mean | SD |"));
        assert_eq!(batch.sample(&duration_metric()).values[0], 3600.0);

        let v = find("attack-halt").unwrap();
        let vr = v.evaluate(
            &Sample::new("losses", "fraction", sample.values.clone()),
            0.95,
        );
        let suite = VignetteSuite::new(vec![vr.clone()]);
        assert_eq!(suite.in_band + suite.out_of_band + suite.no_data, 1);
        assert!(suite.to_markdown().contains("Attack halt threshold"));

        let resp = |f: &Figures| f.get("x.k") * 2.0;
        let fac = factors(&figs, |_| true);
        let sr = analyse(
            "t",
            &figs,
            &fac,
            &resp,
            &MorrisOptions {
                trajectories: 4,
                levels: 4,
            },
            &SobolOptions {
                base_samples: 64,
                bootstrap: 10,
                level: 0.95,
                top_k: 1,
            },
            Seed(1),
        );
        let smd = sensitivity_markdown(&sr);
        assert!(
            smd.contains("Sobol indices")
                && smd.contains("Research backlog")
                && smd.contains("**[default]**")
        );

        let rv = compare_with(None, &vr, &RegressionOptions { ks_alpha: 0.01 });
        let cmp = compare_samples("A", &sample.values, "B", &sample.values, 0.95, 8);
        let wb = WorkbenchData {
            batches: vec![bs],
            vignettes: vec![vr],
            sensitivity: vec![sr],
            sweeps: vec![],
            comparisons: vec![cmp],
            regression: vec![rv],
        };
        let all = wb.to_markdown();
        assert!(all.contains("## Regression") && all.contains("## A vs B"));
        let json = to_json(&wb).unwrap();
        let back: WorkbenchData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.batches[0].label, wb.batches[0].label);
        assert_eq!(back.batches[0].summary.n, 50);
        assert_eq!(back.vignettes.len(), 1);
    }
}
