//! Vignettes as hypotheses (RUST_ENGINE.md §15.3, §17 "statistical"): a
//! small scenario whose outcome the research gives as a figure, stated as a
//! hypothesis with an acceptance band, and checked as a distribution over
//! hundreds of seeds. The checker runs an [`Experiment`], measures it with a
//! [`Metric`], computes the band's statistic with its confidence interval,
//! and says in band or out of band, with the evidence.
//!
//! This module holds the *definitions* only: the benchmark catalogue of §15.3
//! with bands from the research ([P2], [P9], [P10], [P13], [P14], [P15],
//! [P20], [P23], [P27], [P30], [R6.2], [R7]) and the TypeScript build's W12
//! vignettes (`realtime/world/vignettes.ts`) ported as definitions. The
//! experiments that play them are wired by the integrator (`bgws-sim`).
//!
//! Bands are acceptance criteria, not model inputs, so they are data here
//! rather than figures; where the research gives a point value the band's
//! width is marked in `band_note` and its confidence is the source's.

use crate::batch::{run_batch, BatchConfig, Experiment, Metric, Sample};
use crate::stats::{
    mean, mean_ci_t, quantile_ci_sorted, quantile_sorted, sorted_finite, wilson, Interval, Summary,
};
use bgws_core::rng::{fnv1a, mix64};
use bgws_core::{Confidence, Figures, Seed};
use serde::{Deserialize, Serialize};

/// Which statistic of the outcome distribution must sit in the band.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Statistic {
    /// The mean, with a t interval.
    Mean,
    /// The median, with a distribution-free order-statistic interval.
    Median,
    /// A share: the mean of per-replicate values in [0, 1]. When every value
    /// is 0 or 1 (one trial per replicate) the interval is Wilson's;
    /// otherwise the t interval of the mean of shares, clamped to [0, 1].
    Proportion,
    /// The `p` quantile, with an order-statistic interval.
    Quantile { p: f64 },
}

impl Statistic {
    pub fn label(&self) -> String {
        match self {
            Statistic::Mean => "mean".into(),
            Statistic::Median => "median".into(),
            Statistic::Proportion => "proportion".into(),
            Statistic::Quantile { p } => format!("P{}", (p * 100.0).round()),
        }
    }

    /// The point estimate and its interval for a sample.
    pub fn estimate(&self, values: &[f64], level: f64) -> (f64, Option<Interval>) {
        let v = sorted_finite(values);
        match *self {
            Statistic::Mean => (mean(&v), mean_ci_t(&v, level)),
            Statistic::Median => (quantile_sorted(&v, 0.5), quantile_ci_sorted(&v, 0.5, level)),
            Statistic::Quantile { p } => (quantile_sorted(&v, p), quantile_ci_sorted(&v, p, level)),
            Statistic::Proportion => {
                let m = mean(&v);
                let binary = v.iter().all(|&x| x == 0.0 || x == 1.0);
                let ci = if binary {
                    let k = v.iter().filter(|&&x| x == 1.0).count() as u64;
                    wilson(k, v.len() as u64, level)
                } else {
                    mean_ci_t(&v, level).map(|i| Interval {
                        lo: i.lo.max(0.0),
                        hi: i.hi.min(1.0),
                        level,
                    })
                };
                (m, ci)
            }
        }
    }
}

/// The acceptance band: the statistic must lie in `[lo, hi]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub lo: f64,
    pub hi: f64,
    pub statistic: Statistic,
}

impl Band {
    pub fn new(lo: f64, hi: f64, statistic: Statistic) -> Self {
        Self { lo, hi, statistic }
    }
    pub fn contains(&self, x: f64) -> bool {
        x >= self.lo && x <= self.hi
    }
}

/// What is measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSpec {
    pub name: String,
    pub unit: String,
    pub description: String,
}

/// Where a vignette definition comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VignetteOrigin {
    /// The §15.3 benchmark set.
    Benchmark,
    /// Ported from the TypeScript build's W12 vignettes.
    TypeScript,
}

/// A vignette as a hypothesis (§15.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vignette {
    pub id: String,
    pub title: String,
    /// The claim the distribution is checked against, in words.
    pub hypothesis: String,
    pub metric: MetricSpec,
    pub band: Band,
    /// Research reference(s), e.g. "P14, R4.6".
    pub source: String,
    /// How well the band itself is sourced.
    pub band_confidence: Confidence,
    /// How the band was derived from the source.
    pub band_note: String,
    /// Seeds to run (§15.3: hundreds).
    pub replicates: usize,
    /// The figures that drive it, for the sensitivity sweep. TypeScript ports
    /// carry the TypeScript figure keys until the integrator maps them.
    #[serde(default)]
    pub driving_figures: Vec<String>,
    pub origin: VignetteOrigin,
}

/// The verdict on one vignette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    InBand,
    OutOfBand,
    /// No replicate produced anything to measure.
    NoData,
}

/// The evidence for a verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VignetteResult {
    pub id: String,
    pub title: String,
    pub hypothesis: String,
    pub source: String,
    pub band: Band,
    pub unit: String,
    pub verdict: Verdict,
    /// The band statistic's point estimate (NaN with no data).
    #[serde(deserialize_with = "crate::stats::nan_from_null")]
    pub estimate: f64,
    /// Its confidence interval, when the sample allows one.
    pub interval: Option<Interval>,
    /// The whole interval lies inside the band (a strong pass).
    pub interval_inside_band: bool,
    /// The interval touches the band (not clearly out).
    pub interval_overlaps_band: bool,
    pub summary: Summary,
    pub sample: Sample,
    /// One sentence for the report.
    pub explanation: String,
}

impl VignetteResult {
    pub fn passed(&self) -> bool {
        self.verdict == Verdict::InBand
    }
}

/// Settings for checking vignettes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CheckOptions {
    /// Confidence level of the intervals.
    pub level: f64,
    /// Override each vignette's replicate count (e.g. a quick CI run).
    pub replicates: Option<usize>,
}

impl CheckOptions {
    pub fn from_figures(f: &Figures) -> Self {
        CheckOptions {
            level: f.get("analysis.ci_level"),
            replicates: None,
        }
    }
}

impl Vignette {
    /// This vignette's base seed under a run's seed: distinct vignettes never
    /// share replicate seeds.
    pub fn seed(&self, base: Seed) -> Seed {
        Seed(mix64(base.0 ^ fnv1a(self.id.as_bytes())))
    }

    /// Judge an already measured sample.
    pub fn evaluate(&self, sample: &Sample, level: f64) -> VignetteResult {
        let summary = sample.summary(level);
        let (estimate, interval) = self.band.statistic.estimate(&sample.values, level);
        let verdict = if sample.is_empty() || !estimate.is_finite() {
            Verdict::NoData
        } else if self.band.contains(estimate) {
            Verdict::InBand
        } else {
            Verdict::OutOfBand
        };
        let inside = interval.is_some_and(|i| i.lo >= self.band.lo && i.hi <= self.band.hi);
        let overlaps = interval.is_some_and(|i| i.hi >= self.band.lo && i.lo <= self.band.hi);
        let stat = self.band.statistic.label();
        let ci = interval
            .map(|i| {
                format!(
                    " ({:.0}% CI {} – {})",
                    i.level * 100.0,
                    fmt(i.lo),
                    fmt(i.hi)
                )
            })
            .unwrap_or_default();
        let explanation = match verdict {
            Verdict::NoData => format!(
                "No replicate of {} measured `{}`.",
                sample.values.len() + sample.missing,
                self.metric.name
            ),
            Verdict::InBand => format!(
                "{stat} {}{ci} {} is inside the band {} – {} from [{}]{}.",
                fmt(estimate),
                self.metric.unit,
                fmt(self.band.lo),
                fmt(self.band.hi),
                self.source,
                if inside {
                    ""
                } else {
                    "; the interval reaches outside it, so more replicates would firm this up"
                }
            ),
            Verdict::OutOfBand => format!(
                "{stat} {}{ci} {} is {} the band {} – {} from [{}]{}.",
                fmt(estimate),
                self.metric.unit,
                if estimate < self.band.lo {
                    "below"
                } else {
                    "above"
                },
                fmt(self.band.lo),
                fmt(self.band.hi),
                self.source,
                if overlaps {
                    "; the interval still touches the band"
                } else {
                    ""
                }
            ),
        };
        VignetteResult {
            id: self.id.clone(),
            title: self.title.clone(),
            hypothesis: self.hypothesis.clone(),
            source: self.source.clone(),
            band: self.band,
            unit: self.metric.unit.clone(),
            verdict,
            estimate,
            interval,
            interval_inside_band: inside,
            interval_overlaps_band: overlaps,
            summary,
            sample: sample.clone(),
            explanation,
        }
    }

    /// Run the experiment over this vignette's replicates and judge it.
    pub fn check<E: Experiment>(
        &self,
        experiment: &E,
        metric: &impl Metric<E::Output>,
        figures: &Figures,
        base_seed: Seed,
        options: &CheckOptions,
    ) -> VignetteResult {
        let config = BatchConfig::new(
            options.replicates.unwrap_or(self.replicates),
            self.seed(base_seed),
        );
        let batch = run_batch(experiment, figures, config);
        let mut sample = batch.sample(metric);
        sample.metric = self.metric.name.clone();
        sample.unit = self.metric.unit.clone();
        self.evaluate(&sample, options.level)
    }
}

/// Short number formatting for reports.
pub(crate) fn fmt(x: f64) -> String {
    if !x.is_finite() {
        "—".into()
    } else if x.abs() >= 100.0 {
        format!("{}", x.round())
    } else if x.abs() >= 1.0 || x == 0.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.3}")
    }
}

/// Replicates per benchmark vignette (§15.3: "hundreds of seeds").
pub const DEFAULT_REPLICATES: usize = 200;

#[allow(clippy::too_many_arguments)]
fn def(
    id: &str,
    title: &str,
    hypothesis: &str,
    metric: (&str, &str, &str),
    band: Band,
    source: &str,
    band_confidence: Confidence,
    band_note: &str,
    origin: VignetteOrigin,
    driving: &[&str],
) -> Vignette {
    Vignette {
        id: id.into(),
        title: title.into(),
        hypothesis: hypothesis.into(),
        metric: MetricSpec {
            name: metric.0.into(),
            unit: metric.1.into(),
            description: metric.2.into(),
        },
        band,
        source: source.into(),
        band_confidence,
        band_note: band_note.into(),
        replicates: DEFAULT_REPLICATES,
        driving_figures: driving.iter().map(|s| s.to_string()).collect(),
        origin,
    }
}

/// The §15.3 benchmark vignettes, bands from the research parameter sheet.
pub fn benchmark_vignettes() -> Vec<Vignette> {
    use Confidence::*;
    use Statistic::*;
    use VignetteOrigin::Benchmark as B;
    vec![
        def("mbt-engagement", "MBT engagement at 2 km",
            "A main battle tank's first round against a stationary tank-sized target at 2 km hits more than 70% of the time.",
            ("first_round_hit", "share", "1 if the first aimed round hit, else 0"),
            Band::new(0.70, 0.95, Proportion), "P27, R4.2", Default,
            "P27: Ph > 70% at 2 km, effective range 2–3 km [default]; upper bound 0.95 [default].", B, &[]),
        def("fpv-effectiveness", "FPV effectiveness",
            "Most FPV strikes fail to destroy their target: 60–80% fail.",
            ("fpv_fail_share", "share", "share of FPV strikes launched that did not destroy the target"),
            Band::new(0.60, 0.80, Proportion), "P9, R5.1", Medium,
            "P9: hit rate 30–50% (10% novice), 60–80% fail to destroy.", B, &[]),
        def("shahed-raid", "Shahed raid intercept",
            "Against a mixed one-way-attack raid (about 60% strike, 40% decoy) a layered air defence intercepts about 85%.",
            ("raid_intercepted", "share", "share of the raid's drones destroyed before impact"),
            Band::new(0.75, 0.95, Proportion), "P10, R5.2", Medium,
            "P10: ~85% intercepted; band ±10 points for a synthetic catalogue.", B, &[]),
        def("ballistic-missile-defence", "Ballistic missile defence",
            "A raid-level intercept rate near 85% emerges from 1.5–2 interceptors per threat at a per-shot Pk of 0.6–0.7.",
            ("bm_intercepted", "share", "share of ballistic missiles engaged that were intercepted"),
            Band::new(0.80, 0.90, Proportion), "P20, P29, R6.6", Medium,
            "P20: 273/322 = 0.848 (Wilson 95% 0.805–0.883), widened to 0.80–0.90.", B, &[]),
        def("breach-time", "Minefield breach time",
            "Breaching one lane through a ~500 m mine belt takes about 1.5 h, longer when observed.",
            ("breach_time", "s", "time from the first breach action to an open lane"),
            Band::new(3600.0, 7200.0, Median), "P13, R4.5", Medium,
            "P13: ~1.5 h per ~500 m belt, 6 m lanes; band 1–2 h for the median.", B, &[]),
        def("attack-halt", "Attack halt threshold",
            "An attacking battalion halts at about 20% casualties; the probability of halting rises steeply from 10–20%.",
            ("losses_at_halt", "fraction", "share of the attacking force lost when the attack halted"),
            Band::new(0.10, 0.30, Median), "P14, R4.6", Medium,
            "P14: attackers near 20% casualties almost always halted (Dupuy).", B, &[]),
        def("precision-fires-clear", "Precision fires, GNSS clear",
            "Without jamming, more than half of GNSS-guided rounds are effective.",
            ("round_effective", "share", "1 if the round landed within its lethal radius of the aim point"),
            Band::new(0.50, 1.0, Proportion), "P15, R3", Low,
            "P15: Excalibur > 50% effective before jamming (thin data).", B, &[]),
        def("precision-fires-jammed", "Precision fires under GNSS jamming",
            "Under GNSS jamming, guided-round effectiveness falls below about 10%.",
            ("round_effective", "share", "1 if the round landed within its lethal radius of the aim point"),
            Band::new(0.0, 0.10, Proportion), "P15, P16, R3", Low,
            "P15: > 50% → < 10% within months of jamming (thin data).", B, &[]),
        def("detect-to-strike-networked", "Detect-to-strike, networked",
            "With a networked kill chain, detect-to-strike takes 2–5 minutes.",
            ("detect_to_strike", "min", "time from first detection to the first round or munition fired"),
            Band::new(2.0, 5.0, Median), "P2, R3", Medium,
            "P2: 2–5 min networked (official claims).", B, &[]),
        def("detect-to-strike-voice", "Detect-to-strike, pre-network",
            "Without the network (voice nets and staff), detect-to-strike takes about 20 minutes.",
            ("detect_to_strike", "min", "time from first detection to the first round or munition fired"),
            Band::new(10.0, 30.0, Median), "P2, R3", Low,
            "P2: pre-network ~20 min; band ×0.5–×1.5.", B, &[]),
        def("counter-battery-survival", "Counter-battery survival",
            "A self-propelled gun that displaces within 2–3 minutes of firing survives responsive counter-battery fire (2–5 min).",
            ("gun_survived", "share", "1 if a gun that fired and displaced within its window survived the counter-battery response"),
            Band::new(0.70, 1.0, Proportion), "P23, R4.3", Default,
            "P23: displace within 2–3 min [default]; survival band [default].", B, &[]),
        def("sortie-surge", "Sortie generation, surge",
            "A fighter squadron surging generates 2–3 sorties per aircraft per day.",
            ("sorties_per_aircraft_day", "1/day", "sorties flown per available aircraft per day"),
            Band::new(2.0, 3.0, Mean), "P30, R6.5", Default,
            "P30: surge 2–3 [default].", B, &[]),
        def("sortie-sustained", "Sortie generation, sustained",
            "Sustained operations generate 1–1.5 sorties per aircraft per day.",
            ("sorties_per_aircraft_day", "1/day", "sorties flown per available aircraft per day"),
            Band::new(1.0, 1.5, Mean), "P30, R6.5", Default,
            "P30: sustained 1–1.5 [default].", B, &[]),
        def("casevac-delay", "Casualty evacuation delay",
            "Under drone observation the golden hour is lost: evacuation typically takes 60–120 minutes.",
            ("evacuation_delay", "min", "time from wounding to arrival at the first surgical care"),
            Band::new(60.0, 120.0, Median), "R7", Low,
            "R7: 60–120 min common; point-of-injury evacuation can exceed 24 h (tail, not checked here).", B, &[]),
        def("red-sea-magazine", "Red Sea magazine depth",
            "Escorts under a sustained drone and missile campaign fire about 0.6 missiles per engagement, guns, fighters and EW carrying the rest.",
            ("missiles_per_engagement", "missiles", "interceptor missiles fired per threat engaged"),
            Band::new(0.40, 0.80, Mean), "R6.2", Medium,
            "R6.2: ~220 missiles over ~380 engagements = 0.58; band ±0.2.", B, &[]),
    ]
}

/// The TypeScript build's W12 vignettes (`realtime/world/vignettes.ts`),
/// ported as definitions with their bands. TypeScript checked the median of
/// 3–12 runs; here they run [`DEFAULT_REPLICATES`] seeds.
pub fn typescript_vignettes() -> Vec<Vignette> {
    use Confidence::*;
    use Statistic::Median;
    use VignetteOrigin::TypeScript as T;
    vec![
        def(
            "ts.breach",
            "Minefield breach",
            "About 1.5 h per lane.",
            (
                "breach_s",
                "s",
                "time to breach the minefield (world.stats.breachS[0])",
            ),
            Band::new(0.5 * 5400.0, 1.5 * 5400.0, Median),
            "R4.5, P13",
            Medium,
            "TypeScript band 0.5–1.5 × 5400 s.",
            T,
            &[
                "world/domains/landExt.BREACH_S",
                "world/domains/landExt.UNDER_FIRE_SLOW",
            ],
        ),
        def(
            "ts.battalion",
            "Attacking battalion",
            "Halts near 20% casualties.",
            (
                "share_lost",
                "share lost",
                "losses when the HQ decided to halt (CCIR), else at the end",
            ),
            Band::new(0.05, 0.6, Median),
            "P14",
            Medium,
            "TypeScript band 0.05–0.6.",
            T,
            &["world/command/heuristicPlanner.CULMINATION_LOSS"],
        ),
        def(
            "ts.fpv",
            "FPV effectiveness",
            "60–80% of strikes fail to destroy.",
            (
                "share_failing",
                "share failing",
                "1 − kills / fired for FPV munitions",
            ),
            Band::new(0.5, 0.92, Median),
            "P8, P9",
            Medium,
            "TypeScript band 0.5–0.92.",
            T,
            &[
                "world/domains/drones.MOVING_HIT",
                "world/domains/drones.ACQUIRE_IN_COVER",
                "world/domains/drones.JAMMED_HIT",
            ],
        ),
        def(
            "ts.shahed",
            "Shahed raid",
            "About 85% intercepted.",
            (
                "share_intercepted",
                "share intercepted",
                "destroyed one-way-attack drones / raid size",
            ),
            Band::new(0.65, 0.97, Median),
            "P10",
            Medium,
            "TypeScript band 0.65–0.97.",
            T,
            &["world/domains/air.AD_ASK_S", "world/domains/air.LOW_FACTOR"],
        ),
        def(
            "ts.pnt-clear",
            "Precision rounds, GNSS clear",
            "More than 50% effective.",
            (
                "share_within_20m",
                "share within 20 m",
                "share of 100 guided rounds landing within 20 m",
            ),
            Band::new(0.5, 1.0, Median),
            "R3, P15",
            Low,
            "TypeScript band 0.5–1.",
            T,
            &["world/pnt.INERTIAL_CEP_M"],
        ),
        def(
            "ts.pnt-jammed",
            "Precision rounds under GNSS jamming",
            "Down toward 10%.",
            (
                "share_within_20m",
                "share within 20 m",
                "share of 100 guided rounds landing within 20 m",
            ),
            Band::new(0.0, 0.15, Median),
            "R3, P15",
            Low,
            "TypeScript band 0–0.15.",
            T,
            &["world/pnt.JAM_DEPTH", "world/pnt.INERTIAL_CEP_M"],
        ),
        def(
            "ts.redsea",
            "Red Sea exchange",
            "Interceptors used per threat downed.",
            (
                "interceptors_per_kill",
                "interceptors per kill",
                "SAM rounds fired / threats destroyed",
            ),
            Band::new(1.0, 6.0, Median),
            "R6.2",
            Medium,
            "TypeScript band 1–6.",
            T,
            &[
                "world/domains/naval.VLS_PER_S",
                "world/domains/air.ENERGY_FALLOFF",
            ],
        ),
        def(
            "ts.killweb-datalink",
            "Kill chain, data link",
            "Minutes.",
            (
                "detect_to_fire",
                "min",
                "fastest blue mission, found (or requested) to fired",
            ),
            Band::new(1.0, 20.0, Median),
            "R3, P2",
            Medium,
            "TypeScript band 1–20 min.",
            T,
            &["world/domains/killweb.DATALINK_STAFF_S"],
        ),
        def(
            "ts.killweb-voice",
            "Kill chain, voice nets",
            "Hours.",
            (
                "detect_to_fire",
                "min",
                "fastest blue mission, found (or requested) to fired",
            ),
            Band::new(15.0, 240.0, Median),
            "R3, P2",
            Medium,
            "TypeScript band 15–240 min.",
            T,
            &["world/domains/killweb.VOICE_STAFF_S"],
        ),
    ]
}

/// Every vignette definition: benchmarks first, then the TypeScript ports.
pub fn catalogue() -> Vec<Vignette> {
    let mut v = benchmark_vignettes();
    v.extend(typescript_vignettes());
    v
}

/// Look a vignette up by id.
pub fn find(id: &str) -> Option<Vignette> {
    catalogue().into_iter().find(|v| v.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::StreamKey;
    use rand::Rng;
    use std::collections::BTreeSet;

    fn figs() -> Figures {
        Figures::from_list(crate::figures()).unwrap()
    }

    #[test]
    fn catalogue_is_well_formed() {
        let all = catalogue();
        let ids: BTreeSet<_> = all.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids.len(), all.len(), "duplicate vignette ids");
        for refs in [
            "P27", "P9", "P10", "P20", "P13", "P14", "P15", "P2", "P23", "P30", "R7", "R6.2",
        ] {
            assert!(
                benchmark_vignettes()
                    .iter()
                    .any(|v| v.source.split(", ").any(|s| s == refs)),
                "no benchmark vignette cites {refs}"
            );
        }
        for v in &all {
            assert!(v.band.lo <= v.band.hi, "{}", v.id);
            if v.band.statistic == Statistic::Proportion {
                assert!(v.band.lo >= 0.0 && v.band.hi <= 1.0, "{}", v.id);
            }
            assert!(v.replicates >= 100);
        }
        assert_eq!(typescript_vignettes().len(), 9);
        let json = serde_json::to_string(&all).unwrap();
        let back: Vec<Vignette> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, all);
        assert!(find("attack-halt").is_some());
    }

    #[test]
    fn bernoulli_experiment_in_and_out_of_band() {
        let f = figs();
        let v = find("ballistic-missile-defence").unwrap();
        let shot = |p: f64| {
            move |_: &Figures, seed: Seed| {
                let x: f64 = seed.stream(StreamKey::new("test.bmd", 0, 0, 0)).gen();
                if x < p {
                    1.0
                } else {
                    0.0
                }
            }
        };
        let opts = CheckOptions {
            level: 0.95,
            replicates: Some(2000),
        };
        let good = v.check(&shot(0.85), &crate::batch::Scalar, &f, Seed(1), &opts);
        assert_eq!(good.verdict, Verdict::InBand, "{}", good.explanation);
        assert!(good.interval.unwrap().contains(0.85));
        assert!(good.interval_inside_band);
        let bad = v.check(&shot(0.6), &crate::batch::Scalar, &f, Seed(1), &opts);
        assert_eq!(bad.verdict, Verdict::OutOfBand);
        assert!(bad.explanation.contains("below"));
        assert!(!bad.interval_overlaps_band);
    }

    #[test]
    fn median_and_no_data() {
        let v = find("breach-time").unwrap();
        let s = Sample::new(
            "breach_time",
            "s",
            (0..101).map(|i| 4000.0 + i as f64 * 20.0).collect(),
        );
        let r = v.evaluate(&s, 0.95);
        assert_eq!(r.verdict, Verdict::InBand);
        assert_eq!(r.estimate, 5000.0);
        let empty = Sample::from_options("breach_time", "s", &[None, None]);
        let r = v.evaluate(&empty, 0.95);
        assert_eq!(r.verdict, Verdict::NoData);
        assert_eq!(r.summary.missing, 2);
    }

    #[test]
    fn options_from_figures() {
        let o = CheckOptions::from_figures(&figs());
        assert_eq!(o.level, 0.95);
        let q = Statistic::Quantile { p: 0.9 };
        assert_eq!(q.label(), "P90");
        let (e, ci) = q.estimate(&(0..=100).map(|i| i as f64).collect::<Vec<_>>(), 0.95);
        assert_eq!(e, 90.0);
        assert!(ci.unwrap().contains(90.0));
    }
}
