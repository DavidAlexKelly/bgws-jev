//! The tracker's figures (rule 3) and the parameter struct built from them.
//!
//! Track staleness follows the research's COP mechanic: uncertainty grows
//! with time since the last update and a track drops after minutes for air,
//! tens of minutes to hours for ground **[ref R2]**. Every other value is
//! **[default]** pending calibration (§15).

use crate::classify::{ConfusionModel, Domain};
use crate::filter::{KfInit, ProcessNoise};
use crate::fusion::CiCriterion;
use bgws_core::{figure, Figure, Figures, SimDuration};
use serde::{Deserialize, Serialize};

/// Per-domain tracker settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainParams {
    /// White-noise acceleration PSD, m²/s³.
    pub accel_psd: f64,
    /// Initial velocity σ per axis for a new track, m/s.
    pub init_speed_sigma: f64,
    /// Drop when the 1σ horizontal semi-major axis exceeds this, metres.
    pub drop_sigma: f64,
    /// Drop after this long without an update.
    pub max_coast: SimDuration,
}

/// Everything the knowledge models read from the figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowParams {
    pub land: DomainParams,
    pub air: DomainParams,
    pub sea: DomainParams,
    /// Position random-walk PSD for static tracks, m²/s.
    pub static_psd: f64,
    /// Altitude random-walk PSD for surface and turning tracks, m²/s.
    pub alt_psd: f64,
    /// Turn-rate PSD for the CT model, rad²/s³.
    pub turn_psd: f64,
    /// Initial turn-rate σ, rad/s.
    pub init_turn_sigma: f64,
    /// Velocity σ assumed for a static track (m/s), when expressed in 6D.
    pub static_speed_sigma: f64,
    /// Vertical speed σ for surface tracks (m/s), when expressed in 6D.
    pub vertical_speed_sigma: f64,
    /// Altitude σ for a fix that carries none (range-bearing, DF), metres.
    pub unknown_alt_sigma: f64,
    /// Probability mass inside the association gate.
    pub gate_prob: f64,
    /// M-of-N confirmation.
    pub confirm_m: u32,
    pub confirm_n: u32,
    /// Two tracks merge when their difference is inside this chi-squared mass.
    pub merge_prob: f64,
    /// Quality 15 at or below this 1σ, metres; one step per factor √2 above it.
    pub quality_ref_sigma: f64,
    /// DF: minimum crossing angle to fix, radians.
    pub df_min_crossing: f64,
    /// DF: how long an unassociated bearing waits for a partner.
    pub df_max_age: SimDuration,
    /// DF: furthest credible fix, metres.
    pub df_max_range: f64,
    pub ci_criterion: CiCriterion,
    /// COP: gate for matching reports from different sources.
    pub cop_gate_prob: f64,
    /// How long a unit keeps a report it heard or a sighting it made.
    pub agent_memory: SimDuration,
    /// How many hypotheses a view shows.
    pub view_top_k: usize,
    pub confusion: ConfusionModel,
}

/// The tracker figures (classification figures are in [`crate::classify::figures`]).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("know.kf.accel_psd.land", 0.5, "m2/s3", 0.01, 10.0, "default", Default,
            "white-noise acceleration PSD for ground vehicles"),
        figure!("know.kf.accel_psd.air", 25.0, "m2/s3", 1.0, 1000.0, "default", Default,
            "white-noise acceleration PSD for aircraft and missiles"),
        figure!("know.kf.accel_psd.sea", 0.05, "m2/s3", 0.001, 5.0, "default", Default,
            "white-noise acceleration PSD for ships"),
        figure!("know.kf.init_speed_sigma.land", 8.0, "m/s", 1.0, 40.0, "default", Default,
            "velocity σ per axis of a new ground track"),
        figure!("know.kf.init_speed_sigma.air", 150.0, "m/s", 20.0, 800.0, "default", Default,
            "velocity σ per axis of a new air track"),
        figure!("know.kf.init_speed_sigma.sea", 8.0, "m/s", 1.0, 30.0, "default", Default,
            "velocity σ per axis of a new surface track"),
        figure!("know.kf.static_psd", 0.01, "m2/s", 0.0, 10.0, "default", Default,
            "position random walk of a near-static ground position"),
        figure!("know.kf.alt_psd", 1.0, "m2/s", 0.0, 1000.0, "default", Default,
            "altitude random walk beside a 2D filter"),
        figure!("know.kf.turn_psd", 1e-5, "rad2/s3", 0.0, 1e-2, "default", Default,
            "turn-rate random walk of the coordinated-turn filter"),
        figure!("know.kf.init_turn_sigma", 0.05, "rad/s", 0.001, 0.5, "default", Default,
            "initial turn-rate σ of the coordinated-turn filter"),
        figure!("know.kf.static_speed_sigma", 0.5, "m/s", 0.01, 5.0, "default", Default,
            "velocity σ assumed for a static track when reported in 6D"),
        figure!("know.kf.vertical_speed_sigma", 1.0, "m/s", 0.01, 50.0, "default", Default,
            "vertical speed σ of a surface track when reported in 6D"),
        figure!("know.kf.unknown_alt_sigma", 100.0, "m", 1.0, 5000.0, "default", Default,
            "altitude σ of a fix that measures none"),
        figure!("know.assoc.gate_prob", 0.99, "fraction", 0.9, 0.99999, "default", Default,
            "chi-squared probability inside the association gate"),
        figure!("know.track.confirm_m", 2.0, "count", 1.0, 10.0, "default", Default,
            "M of M-of-N confirmation"),
        figure!("know.track.confirm_n", 3.0, "count", 1.0, 20.0, "default", Default,
            "N of M-of-N confirmation"),
        figure!("know.track.merge_prob", 0.5, "fraction", 0.01, 0.99, "default", Default,
            "chi-squared mass within which two tracks are indistinguishable and merge"),
        figure!("know.track.drop_sigma.land", 1500.0, "m", 50.0, 20_000.0, "R2", Low,
            "drop a ground track whose 1σ error exceeds this"),
        figure!("know.track.drop_sigma.air", 15_000.0, "m", 500.0, 200_000.0, "R2", Low,
            "drop an air track whose 1σ error exceeds this"),
        figure!("know.track.drop_sigma.sea", 10_000.0, "m", 200.0, 100_000.0, "R2", Low,
            "drop a surface track whose 1σ error exceeds this"),
        figure!("know.track.max_coast.land", 1800.0, "s", 60.0, 21_600.0, "R2", Low,
            "ground tracks go stale after tens of minutes to hours"),
        figure!("know.track.max_coast.air", 300.0, "s", 10.0, 1800.0, "R2", Low,
            "air tracks go stale after minutes"),
        figure!("know.track.max_coast.sea", 3600.0, "s", 60.0, 43_200.0, "R2", Low,
            "surface tracks go stale after an hour or so"),
        figure!("know.track.quality_ref_sigma", 5.0, "m", 0.5, 100.0, "R2", Low,
            "1σ at which track quality is 15 (Link 16 TQ 0–15); one step per √2"),
        figure!("know.df.min_crossing", 0.26, "rad", 0.02, 1.0, "default", Default,
            "minimum crossing angle of DF bearings for a fix (15°)"),
        figure!("know.df.max_age", 300.0, "s", 5.0, 3600.0, "P32", Default,
            "how long an unassociated DF bearing waits for a crossing bearing"),
        figure!("know.df.max_range", 80_000.0, "m", 1000.0, 500_000.0, "default", Default,
            "furthest credible DF fix"),
        figure!("know.ci.use_determinant", 0.0, "flag", 0.0, 1.0, "default", Default,
            "covariance intersection minimises the trace (0) or the determinant (1)"),
        figure!("know.cop.gate_prob", 0.99, "fraction", 0.9, 0.99999, "default", Default,
            "chi-squared gate for matching reports from different sources in an HQ's COP"),
        figure!("know.agent.memory", 120.0, "s", 10.0, 7200.0, "default", Default,
            "how long a unit keeps a sighting or a report (TS contactMemoryS)"),
        figure!("know.view.top_k", 3.0, "count", 1.0, 7.0, "default", Default,
            "class hypotheses shown per track in a view"),
    ]
}

impl KnowParams {
    pub fn from_figures(f: &Figures) -> Self {
        let dom = |d: &str| DomainParams {
            accel_psd: f.get(&format!("know.kf.accel_psd.{d}")),
            init_speed_sigma: f.get(&format!("know.kf.init_speed_sigma.{d}")),
            drop_sigma: f.get(&format!("know.track.drop_sigma.{d}")),
            max_coast: SimDuration::from_secs(f.get(&format!("know.track.max_coast.{d}"))),
        };
        KnowParams {
            land: dom("land"),
            air: dom("air"),
            sea: dom("sea"),
            static_psd: f.get("know.kf.static_psd"),
            alt_psd: f.get("know.kf.alt_psd"),
            turn_psd: f.get("know.kf.turn_psd"),
            init_turn_sigma: f.get("know.kf.init_turn_sigma"),
            static_speed_sigma: f.get("know.kf.static_speed_sigma"),
            vertical_speed_sigma: f.get("know.kf.vertical_speed_sigma"),
            unknown_alt_sigma: f.get("know.kf.unknown_alt_sigma"),
            gate_prob: f.get("know.assoc.gate_prob"),
            confirm_m: f.get("know.track.confirm_m").round() as u32,
            confirm_n: f.get("know.track.confirm_n").round() as u32,
            merge_prob: f.get("know.track.merge_prob"),
            quality_ref_sigma: f.get("know.track.quality_ref_sigma"),
            df_min_crossing: f.get("know.df.min_crossing"),
            df_max_age: SimDuration::from_secs(f.get("know.df.max_age")),
            df_max_range: f.get("know.df.max_range"),
            ci_criterion: if f.get("know.ci.use_determinant") >= 0.5 { CiCriterion::Determinant } else { CiCriterion::Trace },
            cop_gate_prob: f.get("know.cop.gate_prob"),
            agent_memory: SimDuration::from_secs(f.get("know.agent.memory")),
            view_top_k: f.get("know.view.top_k").round() as usize,
            confusion: ConfusionModel::from_figures(f),
        }
    }

    /// Built from this crate's declared defaults (tests, tools).
    pub fn defaults() -> Self {
        // The crate's own figures are a valid list by test; this cannot fail.
        let f = Figures::from_list(crate::figures()).unwrap_or_default();
        Self::from_figures(&f)
    }

    pub fn domain(&self, d: Domain) -> &DomainParams {
        match d {
            Domain::Land => &self.land,
            Domain::Air => &self.air,
            Domain::Sea => &self.sea,
        }
    }

    /// Process noise for a track in domain `d`.
    pub fn process_noise(&self, d: Domain) -> ProcessNoise {
        ProcessNoise { accel_psd: self.domain(d).accel_psd, static_psd: self.static_psd, alt_psd: self.alt_psd, turn_psd: self.turn_psd }
    }

    /// Initial-state settings for a track in domain `d`.
    pub fn kf_init(&self, d: Domain, is_static: bool) -> KfInit {
        let fill = if is_static { self.static_speed_sigma } else { self.vertical_speed_sigma };
        KfInit { speed_sigma: self.domain(d).init_speed_sigma, turn_sigma: self.init_turn_sigma, fill_vel_var: fill * fill }
    }

    /// Link-16-style track quality 0–15 from the 1σ horizontal error.
    pub fn quality(&self, sigma: f64) -> u8 {
        if !(sigma.is_finite()) {
            return 0;
        }
        let steps = 2.0 * (sigma.max(1e-9) / self.quality_ref_sigma).log2();
        (15.0 - steps.max(0.0)).round().clamp(0.0, 15.0) as u8
    }
}
