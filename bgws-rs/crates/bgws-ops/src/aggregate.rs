//! Aggregated combat and level of detail (RUST_ENGINE.md §8.13, §6.4; research R8, P14).
//!
//! - **Stochastic Lanchester attrition** between formations: each side's
//!   losses in a step are Poisson with rate k·(enemy effective strength)
//!   for aimed fire (square law), k·(enemy)·(own density) for area fire
//!   (linear law), or one of each (mixed). Posture, terrain, supply and
//!   fatigue modify the rates.
//! - **The coefficients are fitted, not set:** [`fit_poisson`] is the MLE of
//!   a Poisson rate from counts and exposures, with a chi-squared dispersion
//!   goodness-of-fit; [`fit_attrition`] fits Lanchester coefficients from
//!   entity-level engagement samples; [`fit_normal`], [`fit_lognormal`] and
//!   [`fit_exponential`] fit movement and consumption rates with a KS test.
//!   (KS with estimated parameters is conservative — Lilliefors — which is
//!   the safe direction for a regression gate.)
//! - **Formations** carry their members' state as distributions (strength,
//!   stores, fatigue). [`aggregate`] folds entities into a formation;
//!   [`Formation::disaggregate`] samples them back with keyed streams so the
//!   same seed gives the same entities and **killed stays killed**.

use crate::math::{chi2_sf, ks_p_value, normal_cdf};
use bgws_core::{figure, EntityId, Figure, Figures, Seed, Side, StreamKey};
use rand::seq::SliceRandom;
use rand::Rng;
use rand_distr::{Distribution as _, Normal, Poisson};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Figures for aggregated combat (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("agg.k_default_per_h", 0.02, "1/h", 0.0, 1.0, "default", Default, "kills per effective shooter-hour before a fit exists"),
        figure!("agg.posture.attack", 1.0, "1", 0.5, 2.0, "default", Default, "vulnerability of an attacking formation"),
        figure!("agg.posture.hasty_defence", 0.6, "1", 0.2, 1.0, "default", Default, "vulnerability in hasty defence"),
        figure!("agg.posture.prepared_defence", 0.33, "1", 0.1, 1.0, "default", Default, "vulnerability in prepared defence (≈3:1)"),
        figure!("agg.posture.delay", 0.8, "1", 0.2, 1.0, "default", Default, "vulnerability while delaying"),
        figure!("agg.terrain.open", 1.0, "1", 0.5, 1.5, "default", Default, "fire effectiveness in open terrain"),
        figure!("agg.terrain.mixed", 0.8, "1", 0.3, 1.2, "default", Default, "fire effectiveness in mixed terrain"),
        figure!("agg.terrain.close", 0.6, "1", 0.2, 1.0, "default", Default, "fire effectiveness in close terrain"),
        figure!("agg.terrain.urban", 0.5, "1", 0.1, 1.0, "default", Default, "fire effectiveness in urban terrain"),
        figure!("agg.supply_floor", 0.3, "fraction", 0.0, 1.0, "R7", Low, "effectiveness left with no supply"),
        figure!("agg.fatigue_penalty", 0.4, "fraction", 0.0, 1.0, "P31", Low, "effectiveness lost at full fatigue"),
        figure!("agg.fatigue_per_combat_h", 0.02, "1/h", 0.0, 0.2, "P31", Low, "fatigue gained per hour of combat (onset 24–48 h)"),
        figure!("agg.break_fraction", 0.2, "fraction", 0.05, 0.5, "P14", High, "an attack halts at about 20% casualties"),
    ]
}

/// How a side delivers fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireType {
    /// Aimed fire: losses ∝ enemy strength (square law).
    Aimed,
    /// Area fire: losses ∝ enemy strength × own density (linear law).
    Area,
}

/// The law, as the pair of fire types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Law {
    Square,
    Linear,
    /// A fires aimed, B fires area (e.g. guerrilla vs conventional).
    Mixed,
}

impl Law {
    pub fn fire_types(self) -> (FireType, FireType) {
        match self {
            Law::Square => (FireType::Aimed, FireType::Aimed),
            Law::Linear => (FireType::Area, FireType::Area),
            Law::Mixed => (FireType::Aimed, FireType::Area),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Posture {
    Attack,
    HastyDefence,
    PreparedDefence,
    Delay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainClass {
    Open,
    Mixed,
    Close,
    Urban,
}

/// A distribution of a member quantity, carried while aggregated.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Spread {
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
}

impl Spread {
    pub fn of(xs: &[f64]) -> Spread {
        if xs.is_empty() {
            return Spread::default();
        }
        let n = xs.len() as f64;
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        Spread { mean, sd: var.sqrt(), min: xs.iter().copied().fold(f64::INFINITY, f64::min), max: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) }
    }
}

/// One entity's state at aggregation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub entity: EntityId,
    pub class: String,
    pub alive: bool,
    /// Stores as a fraction of basic load.
    pub stores: f64,
    pub fatigue: f64,
}

/// A formation in aggregate state (§6.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Formation {
    pub id: u64,
    pub side: Side,
    /// Members alive when aggregated, sorted by id.
    pub alive_at_aggregation: Vec<(EntityId, String)>,
    /// Members already dead when aggregated: never revived.
    pub dead: BTreeSet<EntityId>,
    pub initial_strength: u32,
    pub strength: u32,
    pub stores: Spread,
    pub fatigue: Spread,
    /// Increments on every aggregation, keying disaggregation streams.
    pub epoch: u64,
    pub fire: FireType,
    pub posture: Posture,
    /// Effective firepower per member (1 = reference).
    pub firepower: f64,
}

/// Fold entities into a formation.
pub fn aggregate(id: u64, side: Side, members: &[MemberRecord], epoch: u64, fire: FireType, posture: Posture) -> Formation {
    let mut alive: Vec<&MemberRecord> = members.iter().filter(|m| m.alive).collect();
    alive.sort_by_key(|m| m.entity);
    let dead = members.iter().filter(|m| !m.alive).map(|m| m.entity).collect();
    let stores: Vec<f64> = alive.iter().map(|m| m.stores).collect();
    let fatigue: Vec<f64> = alive.iter().map(|m| m.fatigue).collect();
    Formation {
        id,
        side,
        alive_at_aggregation: alive.iter().map(|m| (m.entity, m.class.clone())).collect(),
        dead,
        initial_strength: alive.len() as u32,
        strength: alive.len() as u32,
        stores: Spread::of(&stores),
        fatigue: Spread::of(&fatigue),
        epoch,
        fire,
        posture,
        firepower: 1.0,
    }
}

impl Formation {
    pub fn losses(&self) -> u32 {
        self.initial_strength - self.strength
    }

    /// Casualty fraction since aggregation.
    pub fn casualty_fraction(&self) -> f64 {
        if self.initial_strength == 0 {
            0.0
        } else {
            self.losses() as f64 / self.initial_strength as f64
        }
    }

    /// Whether it has reached its break point **[P14]**.
    pub fn broken(&self, f: &Figures) -> bool {
        self.casualty_fraction() >= f.get("agg.break_fraction")
    }

    pub fn apply_losses(&mut self, n: u32) {
        self.strength = self.strength.saturating_sub(n);
    }

    /// Consume a fraction of basic load from every member (mean and bounds shift).
    pub fn consume(&mut self, fraction: f64) {
        let d = fraction.max(0.0);
        self.stores.mean = (self.stores.mean - d).max(0.0);
        self.stores.min = (self.stores.min - d).max(0.0);
        self.stores.max = (self.stores.max - d).max(0.0);
    }

    /// Fatigue builds with combat hours.
    pub fn tire(&mut self, combat_h: f64, f: &Figures) {
        let d = combat_h.max(0.0) * f.get("agg.fatigue_per_combat_h");
        self.fatigue.mean = (self.fatigue.mean + d).min(1.0);
        self.fatigue.min = (self.fatigue.min + d).min(1.0);
        self.fatigue.max = (self.fatigue.max + d).min(1.0);
    }

    /// Recreate the entities (§6.4): which of the members alive at
    /// aggregation were killed is drawn from a keyed stream (formation,
    /// epoch), so a replay gives the same casualties; members dead before
    /// stay dead; stores and fatigue are sampled per member (keyed by
    /// entity) and shifted so their mean matches the aggregate.
    pub fn disaggregate(&self, seed: Seed) -> Vec<MemberRecord> {
        let mut order: Vec<usize> = (0..self.alive_at_aggregation.len()).collect();
        let mut rng = seed.stream(StreamKey::new("agg.disaggregate", self.id, self.epoch, 0));
        order.shuffle(&mut rng);
        let killed: BTreeSet<usize> = order.into_iter().take(self.losses() as usize).collect();
        let draw = |s: &Spread, e: EntityId, purpose: &'static str| -> f64 {
            let mut r = seed.stream(StreamKey::new(purpose, self.id, e.0 as u64, self.epoch as i64));
            let x = if s.sd > 0.0 { Normal::new(s.mean, s.sd).map(|d| d.sample(&mut r)).unwrap_or(s.mean) } else { s.mean };
            x.clamp(s.min, s.max)
        };
        let mut out: Vec<MemberRecord> = Vec::new();
        for (i, (e, class)) in self.alive_at_aggregation.iter().enumerate() {
            let alive = !killed.contains(&i);
            out.push(MemberRecord {
                entity: *e,
                class: class.clone(),
                alive,
                stores: if alive { draw(&self.stores, *e, "agg.stores") } else { 0.0 },
                fatigue: if alive { draw(&self.fatigue, *e, "agg.fatigue") } else { 0.0 },
            });
        }
        // Match the aggregate means over the survivors.
        for (get, spread) in [(0usize, self.stores), (1, self.fatigue)] {
            let alive: Vec<usize> = (0..out.len()).filter(|&i| out[i].alive).collect();
            if alive.is_empty() {
                continue;
            }
            let val = |m: &MemberRecord| if get == 0 { m.stores } else { m.fatigue };
            let mean = alive.iter().map(|&i| val(&out[i])).sum::<f64>() / alive.len() as f64;
            let shift = spread.mean - mean;
            for &i in &alive {
                let v = (val(&out[i]) + shift).clamp(spread.min.min(spread.mean), spread.max.max(spread.mean));
                if get == 0 {
                    out[i].stores = v;
                } else {
                    out[i].fatigue = v;
                }
            }
        }
        for e in &self.dead {
            out.push(MemberRecord { entity: *e, class: String::new(), alive: false, stores: 0.0, fatigue: 0.0 });
        }
        out.sort_by_key(|m| m.entity);
        out
    }
}

/// The circumstances of an aggregate engagement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngagementEnv {
    pub terrain: TerrainClass,
    /// Supply state of A and B, 0–1.
    pub supply: (f64, f64),
}

/// Fitted or default coefficients: kills per effective shooter-hour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttritionCoeffs {
    pub k_a_on_b: f64,
    pub k_b_on_a: f64,
}

/// One step's attrition (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttritionStep {
    pub effective_a: f64,
    pub effective_b: f64,
    /// Loss rates per hour.
    pub rate_a: f64,
    pub rate_b: f64,
    pub expected_a: f64,
    pub expected_b: f64,
    pub losses_a: u32,
    pub losses_b: u32,
}

/// Modifier on a side's effectiveness: terrain × supply × fatigue.
pub fn effectiveness(fm: &Formation, terrain: TerrainClass, supply: f64, f: &Figures) -> f64 {
    let t = match terrain {
        TerrainClass::Open => f.get("agg.terrain.open"),
        TerrainClass::Mixed => f.get("agg.terrain.mixed"),
        TerrainClass::Close => f.get("agg.terrain.close"),
        TerrainClass::Urban => f.get("agg.terrain.urban"),
    };
    let floor = f.get("agg.supply_floor");
    let s = floor + (1.0 - floor) * supply.clamp(0.0, 1.0);
    let fat = 1.0 - f.get("agg.fatigue_penalty") * fm.fatigue.mean.clamp(0.0, 1.0);
    t * s * fat * fm.firepower
}

/// Vulnerability by posture.
pub fn vulnerability(p: Posture, f: &Figures) -> f64 {
    match p {
        Posture::Attack => f.get("agg.posture.attack"),
        Posture::HastyDefence => f.get("agg.posture.hasty_defence"),
        Posture::PreparedDefence => f.get("agg.posture.prepared_defence"),
        Posture::Delay => f.get("agg.posture.delay"),
    }
}

fn loss_rate(target: &Formation, shooter: &Formation, k: f64, shooter_eff: f64, f: &Figures) -> f64 {
    let fire = shooter.strength as f64 * shooter_eff * k * vulnerability(target.posture, f);
    match shooter.fire {
        FireType::Aimed => fire,
        FireType::Area => fire * target.strength as f64 / target.initial_strength.max(1) as f64,
    }
}

/// Expected loss rates (per hour) for both sides at the current state.
pub fn loss_rates(a: &Formation, b: &Formation, k: &AttritionCoeffs, env: &EngagementEnv, f: &Figures) -> (f64, f64, f64, f64) {
    let ea = effectiveness(a, env.terrain, env.supply.0, f);
    let eb = effectiveness(b, env.terrain, env.supply.1, f);
    (loss_rate(a, b, k.k_b_on_a, eb, f), loss_rate(b, a, k.k_a_on_b, ea, f), ea, eb)
}

/// One stochastic attrition step of `dt_h` hours: Poisson losses, capped at strength.
pub fn attrition_step(a: &mut Formation, b: &mut Formation, k: &AttritionCoeffs, env: &EngagementEnv, dt_h: f64, f: &Figures, rng: &mut impl Rng) -> AttritionStep {
    let (ra, rb, ea, eb) = loss_rates(a, b, k, env, f);
    let draw = |lambda: f64, rng: &mut _| -> u32 {
        if lambda <= 0.0 {
            0
        } else {
            Poisson::new(lambda).map(|p: Poisson<f64>| p.sample(rng) as u32).unwrap_or(0)
        }
    };
    let la = draw(ra * dt_h, rng).min(a.strength);
    let lb = draw(rb * dt_h, rng).min(b.strength);
    a.apply_losses(la);
    b.apply_losses(lb);
    a.tire(dt_h, f);
    b.tire(dt_h, f);
    AttritionStep { effective_a: ea, effective_b: eb, rate_a: ra, rate_b: rb, expected_a: ra * dt_h, expected_b: rb * dt_h, losses_a: la, losses_b: lb }
}

/// Deterministic square-law solution: A(t), B(t) with dA/dt = −k_b·B, dB/dt = −k_a·A.
pub fn square_law(a0: f64, b0: f64, k_a_on_b: f64, k_b_on_a: f64, t: f64) -> (f64, f64) {
    let w = (k_a_on_b * k_b_on_a).sqrt();
    let (c, s) = ((w * t).cosh(), (w * t).sinh());
    (a0 * c - (k_b_on_a / w) * b0 * s, b0 * c - (k_a_on_b / w) * a0 * s)
}

// ───────────────────────────── fitting ─────────────────────────────

#[derive(Debug, Error, PartialEq)]
pub enum FitError {
    #[error("no samples")]
    Empty,
    #[error("counts and exposures differ in length")]
    Mismatch,
    #[error("non-positive total exposure")]
    NoExposure,
    #[error("samples must be positive for this distribution")]
    NonPositive,
}

/// A fitted Poisson rate with goodness of fit (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoissonFit {
    /// MLE: total events / total exposure.
    pub rate: f64,
    pub se: f64,
    pub n: usize,
    pub events: u64,
    pub exposure: f64,
    /// Pearson dispersion statistic Σ(k − λt)²/(λt).
    pub chi2: f64,
    pub dof: f64,
    /// P(χ² ≥ chi2): small means over-dispersed, the Poisson model fits badly.
    pub p_value: f64,
    pub log_likelihood: f64,
}

/// MLE of a Poisson rate from counts over exposures.
pub fn fit_poisson(counts: &[u64], exposures: &[f64]) -> Result<PoissonFit, FitError> {
    if counts.is_empty() {
        return Err(FitError::Empty);
    }
    if counts.len() != exposures.len() {
        return Err(FitError::Mismatch);
    }
    let events: u64 = counts.iter().sum();
    let exposure: f64 = exposures.iter().sum();
    if exposure <= 0.0 {
        return Err(FitError::NoExposure);
    }
    let rate = events as f64 / exposure;
    let mut chi2 = 0.0;
    let mut ll = 0.0;
    for (k, t) in counts.iter().zip(exposures) {
        let mu = rate * t;
        if mu > 0.0 {
            chi2 += (*k as f64 - mu).powi(2) / mu;
            ll += *k as f64 * mu.ln() - mu - crate::math::ln_gamma(*k as f64 + 1.0);
        }
    }
    let dof = (counts.len() as f64 - 1.0).max(1.0);
    Ok(PoissonFit { rate, se: (events.max(1) as f64).sqrt() / exposure, n: counts.len(), events, exposure, chi2, dof, p_value: chi2_sf(chi2, dof), log_likelihood: ll })
}

/// One entity-level engagement interval, as a fitting sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngagementSample {
    pub losses_a: u64,
    pub losses_b: u64,
    /// Mean strengths over the interval.
    pub strength_a: f64,
    pub strength_b: f64,
    /// Initial strengths (for area fire's density term).
    pub initial_a: f64,
    pub initial_b: f64,
    pub hours: f64,
}

/// Fitted Lanchester coefficients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttritionFit {
    pub law: Law,
    pub a_on_b: PoissonFit,
    pub b_on_a: PoissonFit,
}

impl AttritionFit {
    pub fn coeffs(&self) -> AttritionCoeffs {
        AttritionCoeffs { k_a_on_b: self.a_on_b.rate, k_b_on_a: self.b_on_a.rate }
    }
}

/// Fit k's from samples: losses of A are Poisson with exposure
/// (B strength × hours) for aimed fire, × (A strength / A initial) for area fire.
pub fn fit_attrition(samples: &[EngagementSample], law: Law) -> Result<AttritionFit, FitError> {
    let (fa, fb) = law.fire_types();
    let exp_a: Vec<f64> = samples
        .iter()
        .map(|s| {
            let e = s.strength_b * s.hours;
            if fb == FireType::Area {
                e * s.strength_a / s.initial_a.max(1.0)
            } else {
                e
            }
        })
        .collect();
    let exp_b: Vec<f64> = samples
        .iter()
        .map(|s| {
            let e = s.strength_a * s.hours;
            if fa == FireType::Area {
                e * s.strength_b / s.initial_b.max(1.0)
            } else {
                e
            }
        })
        .collect();
    let la: Vec<u64> = samples.iter().map(|s| s.losses_a).collect();
    let lb: Vec<u64> = samples.iter().map(|s| s.losses_b).collect();
    Ok(AttritionFit { law, b_on_a: fit_poisson(&la, &exp_a)?, a_on_b: fit_poisson(&lb, &exp_b)? })
}

/// A fitted continuous distribution with its KS check.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContinuousFit {
    pub family: String,
    /// (mean, sd) for normal; (μ, σ) of ln x for lognormal; (rate, 0) for exponential.
    pub params: (f64, f64),
    pub n: usize,
    pub ks_d: f64,
    pub ks_p: f64,
}

fn ks(samples: &[f64], cdf: impl Fn(f64) -> f64) -> (f64, f64) {
    let mut xs = samples.to_vec();
    xs.sort_by(|a, b| a.total_cmp(b));
    let n = xs.len() as f64;
    let d = xs.iter().enumerate().map(|(i, x)| {
        let c = cdf(*x);
        (c - i as f64 / n).abs().max(((i + 1) as f64 / n - c).abs())
    });
    let d = d.fold(0.0, f64::max);
    (d, ks_p_value(d, xs.len()))
}

/// Normal MLE (e.g. a movement rate) with KS.
pub fn fit_normal(samples: &[f64]) -> Result<ContinuousFit, FitError> {
    if samples.len() < 2 {
        return Err(FitError::Empty);
    }
    let s = Spread::of(samples);
    let sd = s.sd.max(1e-12);
    let (d, p) = ks(samples, |x| normal_cdf((x - s.mean) / sd));
    Ok(ContinuousFit { family: "normal".into(), params: (s.mean, sd), n: samples.len(), ks_d: d, ks_p: p })
}

/// Lognormal MLE (e.g. a consumption rate) with KS.
pub fn fit_lognormal(samples: &[f64]) -> Result<ContinuousFit, FitError> {
    if samples.iter().any(|x| *x <= 0.0) {
        return Err(FitError::NonPositive);
    }
    let logs: Vec<f64> = samples.iter().map(|x| x.ln()).collect();
    let n = fit_normal(&logs)?;
    let (mu, sigma) = n.params;
    let (d, p) = ks(samples, |x| normal_cdf((x.ln() - mu) / sigma));
    Ok(ContinuousFit { family: "lognormal".into(), params: (mu, sigma), n: samples.len(), ks_d: d, ks_p: p })
}

/// Exponential MLE (e.g. times between events) with KS.
pub fn fit_exponential(samples: &[f64]) -> Result<ContinuousFit, FitError> {
    if samples.is_empty() {
        return Err(FitError::Empty);
    }
    if samples.iter().any(|x| *x < 0.0) {
        return Err(FitError::NonPositive);
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    if mean <= 0.0 {
        return Err(FitError::NonPositive);
    }
    let rate = 1.0 / mean;
    let (d, p) = ks(samples, |x| 1.0 - (-rate * x).exp());
    Ok(ContinuousFit { family: "exponential".into(), params: (rate, 0.0), n: samples.len(), ks_d: d, ks_p: p })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    fn figs() -> Figures {
        crate::default_figures()
    }

    fn members(n: u32, dead_every: u32) -> Vec<MemberRecord> {
        (0..n)
            .map(|i| MemberRecord {
                entity: EntityId(i * 3 + 1),
                class: if i % 2 == 0 { "tank".into() } else { "ifv".into() },
                alive: dead_every == 0 || i % dead_every != 0,
                stores: 0.5 + 0.4 * ((i * 7 % 10) as f64 / 10.0),
                fatigue: 0.1 * (i % 3) as f64,
            })
            .collect()
    }

    #[test]
    fn stochastic_mean_follows_square_law() {
        let mut fx = figs();
        fx.set("agg.fatigue_per_combat_h", 0.0).unwrap();
        let env = EngagementEnv { terrain: TerrainClass::Open, supply: (1.0, 1.0) };
        let k = AttritionCoeffs { k_a_on_b: 0.05, k_b_on_a: 0.03 };
        let runs = 300;
        let (mut sa, mut sb) = (0.0, 0.0);
        for r in 0..runs {
            let mut a = aggregate(1, Side::Blue, &members(400, 0), 0, FireType::Aimed, Posture::Attack);
            let mut b = aggregate(2, Side::Red, &members(300, 0), 0, FireType::Aimed, Posture::Attack);
            a.fatigue = Spread::default();
            b.fatigue = Spread::default();
            let mut rng = Seed(r).stream(StreamKey::new("lanchester", r, 0, 0));
            for _ in 0..50 {
                attrition_step(&mut a, &mut b, &k, &env, 0.1, &fx, &mut rng);
            }
            sa += a.strength as f64;
            sb += b.strength as f64;
        }
        let (ea, eb) = square_law(400.0, 300.0, 0.05, 0.03, 5.0);
        assert!((sa / runs as f64 - ea).abs() / ea < 0.02, "{} vs {ea}", sa / runs as f64);
        assert!((sb / runs as f64 - eb).abs() / eb < 0.02, "{} vs {eb}", sb / runs as f64);
    }

    #[test]
    fn modifiers_change_rates() {
        let f = figs();
        let k = AttritionCoeffs { k_a_on_b: 0.05, k_b_on_a: 0.05 };
        let a = aggregate(1, Side::Blue, &members(100, 0), 0, FireType::Aimed, Posture::Attack);
        let mut b = aggregate(2, Side::Red, &members(100, 0), 0, FireType::Aimed, Posture::PreparedDefence);
        let open = EngagementEnv { terrain: TerrainClass::Open, supply: (1.0, 1.0) };
        let (ra, rb, _, _) = loss_rates(&a, &b, &k, &open, &f);
        assert!(rb < ra / 2.5, "prepared defence: {ra} {rb}");
        let urban = EngagementEnv { terrain: TerrainClass::Urban, supply: (0.0, 1.0) };
        let (_, rb2, _, _) = loss_rates(&a, &b, &k, &urban, &f);
        assert!(rb2 < rb * 0.3);
        b.fire = FireType::Area;
        b.posture = Posture::Attack;
        let (ra3, _, _, _) = loss_rates(&a, &b, &k, &open, &f);
        assert!(ra3 > 0.0);
        let mut c = a.clone();
        c.apply_losses(25);
        assert!(c.broken(&f));
    }

    #[test]
    fn poisson_fit_recovers_rate_and_rejects_overdispersion() {
        let mut r = Seed(4).stream(StreamKey::new("fit", 0, 0, 0));
        let true_rate = 0.7;
        let exposures: Vec<f64> = (0..400).map(|i| 2.0 + (i % 5) as f64).collect();
        let counts: Vec<u64> = exposures.iter().map(|t| Poisson::new(true_rate * t).unwrap().sample(&mut r) as u64).collect();
        let fit = fit_poisson(&counts, &exposures).unwrap();
        assert!((fit.rate - true_rate).abs() < 3.0 * fit.se, "{fit:?}");
        assert!(fit.p_value > 0.01, "{fit:?}");
        // Over-dispersed: a gamma-mixed rate.
        let od: Vec<u64> = exposures
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let lam = if i % 2 == 0 { 0.2 } else { 1.2 };
                Poisson::new(lam * t).unwrap().sample(&mut r) as u64
            })
            .collect();
        assert!(fit_poisson(&od, &exposures).unwrap().p_value < 0.01);
        assert_eq!(fit_poisson(&[], &[]), Err(FitError::Empty));
        assert_eq!(fit_poisson(&[1], &[0.0]), Err(FitError::NoExposure));
    }

    #[test]
    fn attrition_fit_from_simulated_engagements() {
        let mut fx = figs();
        fx.set("agg.fatigue_per_combat_h", 0.0).unwrap();
        let env = EngagementEnv { terrain: TerrainClass::Open, supply: (1.0, 1.0) };
        let k = AttritionCoeffs { k_a_on_b: 0.04, k_b_on_a: 0.02 };
        let mut samples = Vec::new();
        for r in 0..200u64 {
            let mut a = aggregate(1, Side::Blue, &members(60, 0), 0, FireType::Aimed, Posture::Attack);
            let mut b = aggregate(2, Side::Red, &members(50, 0), 0, FireType::Aimed, Posture::Attack);
            a.fatigue = Spread::default();
            b.fatigue = Spread::default();
            let mut rng = Seed(8).stream(StreamKey::new("fit.att", r, 0, 0));
            let (sa, sb) = (a.strength as f64, b.strength as f64);
            let st = attrition_step(&mut a, &mut b, &k, &env, 0.5, &fx, &mut rng);
            samples.push(EngagementSample { losses_a: st.losses_a as u64, losses_b: st.losses_b as u64, strength_a: sa, strength_b: sb, initial_a: 60.0, initial_b: 50.0, hours: 0.5 });
        }
        let fit = fit_attrition(&samples, Law::Square).unwrap();
        assert!((fit.coeffs().k_a_on_b - 0.04).abs() < 3.0 * fit.a_on_b.se);
        assert!((fit.coeffs().k_b_on_a - 0.02).abs() < 3.0 * fit.b_on_a.se);
        assert!(fit.a_on_b.p_value > 0.001 && fit.b_on_a.p_value > 0.001);
    }

    #[test]
    fn continuous_fits_and_ks() {
        let mut r = Seed(2).stream(StreamKey::new("ks", 0, 0, 0));
        let speeds: Vec<f64> = (0..500).map(|_| Normal::new(12.0, 3.0).unwrap().sample(&mut r)).collect();
        let n = fit_normal(&speeds).unwrap();
        assert!((n.params.0 - 12.0).abs() < 0.5 && n.ks_p > 0.05, "{n:?}");
        let fuel: Vec<f64> = speeds.iter().map(|x| (x / 6.0).exp()).collect();
        let ln = fit_lognormal(&fuel).unwrap();
        assert!((ln.params.0 - 2.0).abs() < 0.1 && ln.ks_p > 0.05, "{ln:?}");
        let gaps: Vec<f64> = (0..500).map(|_| -(1.0 - r.gen::<f64>()).ln() / 0.5).collect();
        let ex = fit_exponential(&gaps).unwrap();
        assert!((ex.params.0 - 0.5).abs() < 0.06 && ex.ks_p > 0.05, "{ex:?}");
        // The wrong family is rejected.
        assert!(fit_exponential(&speeds).unwrap().ks_p < 0.01);
        assert!(fit_lognormal(&[1.0, -1.0]).is_err());
    }

    #[test]
    fn aggregation_round_trip_keeps_distributions() {
        let ms = members(50, 7);
        let mut fm = aggregate(9, Side::Blue, &ms, 3, FireType::Aimed, Posture::Attack);
        assert_eq!(fm.strength, 42);
        fm.apply_losses(10);
        fm.consume(0.1);
        let back = fm.disaggregate(Seed(77));
        assert_eq!(back.len(), 50);
        let alive: Vec<&MemberRecord> = back.iter().filter(|m| m.alive).collect();
        assert_eq!(alive.len(), 32);
        let mean = alive.iter().map(|m| m.stores).sum::<f64>() / alive.len() as f64;
        assert_relative_eq!(mean, fm.stores.mean, epsilon = 0.02);
        assert_eq!(back, fm.disaggregate(Seed(77)));
        assert_ne!(back, fm.disaggregate(Seed(78)));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn killed_stays_killed(n in 1u32..80, dead_every in 0u32..6, loss_frac in 0.0f64..1.0, seed in 0u64..1_000, rounds in 1usize..4) {
            let ms = members(n, dead_every);
            let mut dead_before: BTreeSet<EntityId> = ms.iter().filter(|m| !m.alive).map(|m| m.entity).collect();
            let mut current = ms;
            for round in 0..rounds {
                let mut fm = aggregate(5, Side::Red, &current, round as u64, FireType::Aimed, Posture::Attack);
                let losses = (fm.strength as f64 * loss_frac).floor() as u32;
                fm.apply_losses(losses);
                let back = fm.disaggregate(Seed(seed));
                prop_assert_eq!(back.len(), n as usize);
                prop_assert_eq!(back.iter().filter(|m| m.alive).count() as u32, fm.strength);
                for m in &back {
                    if dead_before.contains(&m.entity) { prop_assert!(!m.alive); }
                    if m.alive { prop_assert!(m.stores >= 0.0 && m.fatigue >= 0.0); }
                }
                prop_assert_eq!(&back, &fm.disaggregate(Seed(seed)));
                dead_before = back.iter().filter(|m| !m.alive).map(|m| m.entity).collect();
                // Classes survive the round trip for the living.
                for m in back.iter().filter(|m| m.alive) { prop_assert!(!m.class.is_empty()); }
                current = back;
            }
        }
    }
}
