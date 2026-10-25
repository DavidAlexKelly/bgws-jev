//! People (RUST_ENGINE.md §8.10): suppression, stress and cohesion, morale
//! states as hazards, fatigue and sleep debt, skill and experience, reaction
//! times, and casualties by triage category.
//!
//! - **Suppression** rises with incoming near misses and blasts, decays with
//!   time and is reduced by cover; at full suppression search, accuracy and
//!   movement lose 50–90% **[P31]**. The update is the exact solution of
//!   dS/dt = a(1 − S) − S/τ for constant inputs, so it is step-size free.
//! - **Stress and cohesion** accumulate from casualties, suppression,
//!   isolation, surprise and drone observation; leadership, training,
//!   cohesion and success reduce them. Morale states (steady, shaken, pinned,
//!   broken, routed) change by competing hazards around thresholds (rule 2).
//!   An attacking company halts near 20% casualties **[P14]** — checked by the
//!   test harness below, not imposed.
//! - **Fatigue and sleep debt** over 24–48 h of operations **[P31]**, with recovery.
//! - **Skill and experience** set error budgets; experience grows with
//!   survived engagements **[default]**.
//! - **Reaction times** are lognormal by drill, skill, fatigue and suppression.
//! - **Casualties** by triage category, with time-dependent (Weibull)
//!   mortality without treatment: evacuation delays of 60–120 min matter **[R7]**.

use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use rand_distr::{Distribution, LogNormal};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeopleParams {
    pub sup_gain: f64,
    pub sup_blast_weight: f64,
    pub sup_tau_s: f64,
    pub sup_cover_k: f64,
    pub sup_search_loss: f64,
    pub sup_accuracy_loss: f64,
    pub sup_movement_loss: f64,
    pub sup_pinned: f64,
    pub stress_k_cas: f64,
    pub stress_k_surprise: f64,
    pub stress_k_success: f64,
    pub stress_r_sup: f64,
    pub stress_r_isolated: f64,
    pub stress_r_drone: f64,
    pub stress_tau_recover_s: f64,
    pub m_lead: f64,
    pub m_train: f64,
    pub m_coh: f64,
    pub coh_loss_per_cas: f64,
    pub coh_gain_success: f64,
    pub theta: [f64; 4],
    pub hazard_width: f64,
    pub r_up: f64,
    pub r_down: f64,
    pub fat_debt_rate: f64,
    pub fat_onset_debt_h: f64,
    pub fat_loss_per_day: f64,
    pub fat_sleep_recovery: f64,
    pub fat_phys_gain_combat: f64,
    pub fat_phys_gain_light: f64,
    pub fat_phys_recover: f64,
    pub fat_phys_k: f64,
    pub fat_error_k: f64,
    pub skill_exp_scale: f64,
    pub skill_max_gain: f64,
    pub skill_error_k: f64,
    pub react_median_s: [f64; 3],
    pub react_sigma: f64,
    pub react_skill_k: f64,
    pub react_sup_k: f64,
    /// [cause][killed, T1, T2, T3, T4]
    pub triage_split: [[f64; 5]; 2],
    /// [T1, T2, T3, T4] (η hours, Weibull k)
    pub mortality: [(f64, f64); 4],
}

impl PeopleParams {
    pub fn from_figures(f: &Figures) -> Self {
        let g = |k: &str| f.get(&format!("people.{k}"));
        PeopleParams {
            sup_gain: g("suppression.gain_per_near_miss"),
            sup_blast_weight: g("suppression.blast_weight"),
            sup_tau_s: g("suppression.decay_tau_s"),
            sup_cover_k: g("suppression.cover_k"),
            sup_search_loss: g("suppression.search_loss"),
            sup_accuracy_loss: g("suppression.accuracy_loss"),
            sup_movement_loss: g("suppression.movement_loss"),
            sup_pinned: g("suppression.pinned_level"),
            stress_k_cas: g("stress.k_casualty"),
            stress_k_surprise: g("stress.k_surprise"),
            stress_k_success: g("stress.k_success"),
            stress_r_sup: g("stress.r_suppression"),
            stress_r_isolated: g("stress.r_isolated"),
            stress_r_drone: g("stress.r_drone"),
            stress_tau_recover_s: g("stress.recover_tau_s"),
            m_lead: g("stress.m_leadership"),
            m_train: g("stress.m_training"),
            m_coh: g("stress.m_cohesion"),
            coh_loss_per_cas: g("cohesion.loss_per_casualty"),
            coh_gain_success: g("cohesion.gain_success"),
            theta: ["shaken", "pinned", "broken", "routed"].map(|s| g(&format!("morale.theta.{s}"))),
            hazard_width: g("morale.hazard_width"),
            r_up: g("morale.r_up"),
            r_down: g("morale.r_down"),
            fat_debt_rate: g("fatigue.debt_per_hour_awake"),
            fat_onset_debt_h: g("fatigue.onset_debt_h"),
            fat_loss_per_day: g("fatigue.loss_per_day"),
            fat_sleep_recovery: g("fatigue.sleep_recovery"),
            fat_phys_gain_combat: g("fatigue.physical_gain_combat"),
            fat_phys_gain_light: g("fatigue.physical_gain_light"),
            fat_phys_recover: g("fatigue.physical_recover"),
            fat_phys_k: g("fatigue.physical_k"),
            fat_error_k: g("fatigue.error_k"),
            skill_exp_scale: g("skill.experience_scale"),
            skill_max_gain: g("skill.max_gain"),
            skill_error_k: g("skill.error_k"),
            react_median_s: ["drilled", "practised", "novel"].map(|d| g(&format!("reaction.median_s.{d}"))),
            react_sigma: g("reaction.sigma_log"),
            react_skill_k: g("reaction.skill_k"),
            react_sup_k: g("reaction.suppression_k"),
            triage_split: ["fragment", "bullet"].map(|c| ["killed", "t1", "t2", "t3", "t4"].map(|t| g(&format!("casualty.split.{c}.{t}")))),
            mortality: ["t1", "t2", "t3", "t4"].map(|t| (g(&format!("casualty.mortality.{t}.eta_h")), g(&format!("casualty.mortality.{t}.k")))),
        }
    }
}

// ── Suppression ─────────────────────────────────────────────────────────

/// What is coming in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct IncomingFire {
    /// Rounds per minute passing close (from `direct_fire::small_arms_effect`).
    pub near_misses_per_min: f64,
    /// Shell, bomb and drone bursts per minute close enough to feel.
    pub blasts_per_min: f64,
}

/// A unit's (or a crew's) suppression, 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Suppression {
    pub level: f64,
}

/// How suppression degrades the unit now.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuppressionEffects {
    pub search_mult: f64,
    pub accuracy_mult: f64,
    pub movement_mult: f64,
    pub pinned: bool,
}

impl Suppression {
    /// Advance `dt` s under `fire` with `cover` (0..1). Exact for constant inputs.
    pub fn step(&mut self, dt: f64, fire: &IncomingFire, cover: f64, p: &PeopleParams) {
        let pressure = (fire.near_misses_per_min + p.sup_blast_weight * fire.blasts_per_min) / 60.0;
        let a = p.sup_gain * pressure * (1.0 - p.sup_cover_k * cover.clamp(0.0, 1.0));
        let b = 1.0 / p.sup_tau_s;
        let eq = a / (a + b);
        self.level = (eq + (self.level - eq) * (-(a + b) * dt.max(0.0)).exp()).clamp(0.0, 1.0);
    }

    pub fn effects(&self, p: &PeopleParams) -> SuppressionEffects {
        let s = self.level.clamp(0.0, 1.0);
        let pinned = s >= p.sup_pinned;
        SuppressionEffects {
            search_mult: 1.0 - p.sup_search_loss * s,
            accuracy_mult: 1.0 - p.sup_accuracy_loss * s,
            movement_mult: if pinned { 0.0 } else { 1.0 - p.sup_movement_loss * s },
            pinned,
        }
    }
}

// ── Stress, cohesion and morale ─────────────────────────────────────────

/// Morale states, in order of decay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoraleState {
    Steady,
    Shaken,
    Pinned,
    Broken,
    Routed,
}

impl MoraleState {
    const ORDER: [MoraleState; 5] = [MoraleState::Steady, MoraleState::Shaken, MoraleState::Pinned, MoraleState::Broken, MoraleState::Routed];
    fn idx(self) -> usize {
        self as usize
    }
    /// The attack stops (pinned or worse).
    pub fn halts_attack(self) -> bool {
        self >= MoraleState::Pinned
    }
}

/// Unit quality, each 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitQuality {
    pub leadership: f64,
    pub training: f64,
}

/// What happened to the unit this step.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct StressInputs {
    /// Casualties this step as a fraction of starting strength.
    pub new_casualties: f64,
    /// Current suppression 0..1.
    pub suppression: f64,
    /// No comms and no friends nearby.
    pub isolated: bool,
    /// Surprised this step (ambush, fire from an unseen enemy).
    pub surprised: bool,
    /// Under drone observation.
    pub drone_observed: bool,
    /// A success this step (enemy destroyed, objective taken).
    pub success: bool,
}

/// A unit's morale: stress, cohesion, cumulative casualties and state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Morale {
    pub stress: f64,
    pub cohesion: f64,
    pub casualties: f64,
    pub state: MoraleState,
}

/// A morale change and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoraleChange {
    pub from: MoraleState,
    pub to: MoraleState,
    pub stress: f64,
    pub casualties: f64,
}

impl Morale {
    pub fn new(cohesion: f64) -> Self {
        Morale { stress: 0.0, cohesion: cohesion.clamp(0.0, 1.0), casualties: 0.0, state: MoraleState::Steady }
    }

    /// How much of each stressor gets through: 1 − Σ mᵢ·qualityᵢ.
    pub fn susceptibility(&self, q: &UnitQuality, p: &PeopleParams) -> f64 {
        (1.0 - p.m_lead * q.leadership - p.m_train * q.training - p.m_coh * self.cohesion).max(0.05)
    }

    /// Hazard rates (up, down) out of the current state.
    pub fn hazards(&self, q: &UnitQuality, p: &PeopleParams) -> (f64, f64) {
        let i = self.state.idx();
        let up = if i < 4 { p.r_up * ((self.stress - p.theta[i]) / p.hazard_width).exp() } else { 0.0 };
        let down = if i > 0 { p.r_down * (1.0 + q.leadership) * ((p.theta[i - 1] - self.stress) / p.hazard_width).exp() } else { 0.0 };
        (up.min(1e3), down.min(1e3))
    }

    /// Advance `dt` s. Two uniform draws, always, in fixed order.
    pub fn step(&mut self, dt: f64, inp: &StressInputs, q: &UnitQuality, p: &PeopleParams, rng: &mut impl Rng) -> Option<MoraleChange> {
        let dt = dt.max(0.0);
        let sus = self.susceptibility(q, p);
        self.casualties += inp.new_casualties.max(0.0);
        self.cohesion = (self.cohesion - p.coh_loss_per_cas * inp.new_casualties.max(0.0) + if inp.success { p.coh_gain_success } else { 0.0 }).clamp(0.0, 1.0);
        let impulse = p.stress_k_cas * inp.new_casualties.max(0.0) + if inp.surprised { p.stress_k_surprise } else { 0.0 };
        let rate = p.stress_r_sup * inp.suppression.clamp(0.0, 1.0) + if inp.isolated { p.stress_r_isolated } else { 0.0 } + if inp.drone_observed { p.stress_r_drone } else { 0.0 };
        // Recovery only while not under fire, faster with good leaders.
        let recover = (1.0 - inp.suppression.clamp(0.0, 1.0)) * (1.0 + q.leadership) / p.stress_tau_recover_s;
        let mut s = self.stress + sus * (impulse + rate * dt);
        s *= (-recover * dt).exp();
        if inp.success {
            s -= p.stress_k_success;
        }
        self.stress = s.clamp(0.0, 1.5);
        let (up, down) = self.hazards(q, p);
        let u1: f64 = rng.gen();
        let u2: f64 = rng.gen();
        let total = up + down;
        if total <= 0.0 || u1 >= bgws_core::hazard_probability(total, dt) {
            return None;
        }
        let from = self.state;
        let i = from.idx();
        self.state = if u2 * total < up { MoraleState::ORDER[i + 1] } else { MoraleState::ORDER[i - 1] };
        Some(MoraleChange { from, to: self.state, stress: self.stress, casualties: self.casualties })
    }
}

// ── Fatigue and sleep ───────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkLoad {
    Sleep,
    Rest,
    Light,
    Combat,
}

/// Sleep debt and physical fatigue.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Fatigue {
    /// Hours of sleep owed.
    pub sleep_debt_h: f64,
    /// Physical fatigue 0..1.
    pub physical: f64,
}

/// What fatigue does now.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FatigueEffects {
    /// 0..1 overall effectiveness.
    pub effectiveness: f64,
    pub reaction_mult: f64,
    pub search_mult: f64,
    pub error_mult: f64,
}

impl Fatigue {
    /// Advance `hours` of `load`. Sleep repays debt; other loads accrue it.
    pub fn step(&mut self, hours: f64, load: WorkLoad, p: &PeopleParams) {
        let h = hours.max(0.0);
        match load {
            WorkLoad::Sleep => {
                self.sleep_debt_h = (self.sleep_debt_h - p.fat_sleep_recovery * h).max(0.0);
                self.physical *= (-p.fat_phys_recover * 2.0 * h).exp();
            }
            WorkLoad::Rest => {
                self.sleep_debt_h += p.fat_debt_rate * h;
                self.physical *= (-p.fat_phys_recover * h).exp();
            }
            WorkLoad::Light | WorkLoad::Combat => {
                self.sleep_debt_h += p.fat_debt_rate * h;
                let g = if load == WorkLoad::Combat { p.fat_phys_gain_combat } else { p.fat_phys_gain_light };
                self.physical = 1.0 - (1.0 - self.physical) * (-g * h).exp();
            }
        }
    }

    pub fn effects(&self, p: &PeopleParams) -> FatigueEffects {
        // Effectiveness falls by `loss_per_day` per 24 h awake beyond the onset debt.
        let day_debt = 24.0 * p.fat_debt_rate;
        let cognitive = (1.0 - p.fat_loss_per_day * ((self.sleep_debt_h - p.fat_onset_debt_h).max(0.0) / day_debt)).max(0.1);
        let eff = (cognitive * (1.0 - p.fat_phys_k * self.physical.clamp(0.0, 1.0))).clamp(0.05, 1.0);
        FatigueEffects { effectiveness: eff, reaction_mult: 1.0 / eff, search_mult: eff, error_mult: 1.0 + p.fat_error_k * (1.0 - eff) }
    }
}

// ── Skill and experience ────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// Trained skill 0..1.
    pub base: f64,
    /// Engagements survived.
    pub experience: f64,
}

impl Skill {
    /// Experience closes part of the gap to perfect skill, saturating.
    pub fn effective(&self, p: &PeopleParams) -> f64 {
        let b = self.base.clamp(0.0, 1.0);
        b + (1.0 - b) * p.skill_max_gain * (1.0 - (-self.experience.max(0.0) / p.skill_exp_scale).exp())
    }
    /// Multiplier ≥ 1 on lay and estimation errors (`direct_fire::Shot::skill_mult`).
    pub fn error_mult(&self, p: &PeopleParams) -> f64 {
        1.0 + p.skill_error_k * (1.0 - self.effective(p))
    }
    pub fn after_engagement(&mut self, survived: bool) {
        if survived {
            self.experience += 1.0;
        }
    }
}

// ── Reaction times ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Drill {
    /// A battle drill rehearsed to reflex.
    Drilled,
    Practised,
    /// Something new: think, then act.
    Novel,
}

/// Median reaction time, s.
pub fn reaction_median_s(drill: Drill, skill: f64, fatigue: &FatigueEffects, suppression: f64, p: &PeopleParams) -> f64 {
    p.react_median_s[drill as usize] * (1.0 + p.react_skill_k * (1.0 - skill.clamp(0.0, 1.0))) * fatigue.reaction_mult * (1.0 + p.react_sup_k * suppression.clamp(0.0, 1.0))
}

/// Draw a reaction time (lognormal around the median).
pub fn sample_reaction_s(median_s: f64, p: &PeopleParams, rng: &mut impl Rng) -> f64 {
    LogNormal::new(median_s.max(1e-6).ln(), p.react_sigma).map(|d| d.sample(rng)).unwrap_or(median_s)
}

// ── Casualties ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Triage {
    Killed,
    /// T1: needs surgery within hours.
    Immediate,
    /// T2: can wait.
    Delayed,
    /// T3: walking wounded.
    Minimal,
    /// T4: unlikely to survive.
    Expectant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WoundCause {
    /// Shell, bomb or drone fragments and blast.
    Fragment,
    Bullet,
}

/// Classify one casualty (one uniform draw).
pub fn classify_casualty(cause: WoundCause, p: &PeopleParams, rng: &mut impl Rng) -> Triage {
    let split = p.triage_split[cause as usize];
    let total: f64 = split.iter().sum();
    let mut u = rng.gen::<f64>() * total;
    let order = [Triage::Killed, Triage::Immediate, Triage::Delayed, Triage::Minimal, Triage::Expectant];
    for (t, w) in order.iter().zip(split) {
        if u < w {
            return *t;
        }
        u -= w;
    }
    Triage::Minimal
}

/// P(dies before treatment) after `delay_s` without care: Weibull 1 − exp(−(t/η)^k).
pub fn p_die_untreated(t: Triage, delay_s: f64, p: &PeopleParams) -> f64 {
    let i = match t {
        Triage::Killed => return 1.0,
        Triage::Immediate => 0,
        Triage::Delayed => 1,
        Triage::Minimal => 2,
        Triage::Expectant => 3,
    };
    let (eta_h, k) = p.mortality[i];
    let h = delay_s.max(0.0) / 3600.0;
    -(-(h / eta_h).powf(k)).exp_m1()
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("people.suppression.gain_per_near_miss", 0.05, "1/round", 0.005, 0.5, "default", Default, "suppression drive per near miss"),
        figure!("people.suppression.blast_weight", 10.0, "rounds", 1.0, 50.0, "default", Default, "one nearby burst counts as this many near misses"),
        figure!("people.suppression.decay_tau_s", 60.0, "s", 10.0, 600.0, "default", Default, "suppression recovery time constant"),
        figure!("people.suppression.cover_k", 0.6, "fraction", 0.0, 0.9, "default", Default, "suppression drive removed by full cover"),
        figure!("people.suppression.search_loss", 0.8, "fraction", 0.5, 0.9, "P31", Low, "search rate lost at full suppression"),
        figure!("people.suppression.accuracy_loss", 0.7, "fraction", 0.5, 0.9, "P31", Low, "return-fire effectiveness lost at full suppression"),
        figure!("people.suppression.movement_loss", 0.9, "fraction", 0.5, 1.0, "P31", Low, "movement lost at full suppression"),
        figure!("people.suppression.pinned_level", 0.7, "fraction", 0.4, 0.95, "default", Default, "suppression at which movement stops"),
        figure!("people.stress.k_casualty", 3.0, "1/fraction", 0.5, 8.0, "P14", Low, "stress per fraction of strength lost (calibrated to the ~20% halt)"),
        figure!("people.stress.k_surprise", 0.15, "1", 0.0, 0.5, "default", Default, "stress impulse from surprise"),
        figure!("people.stress.k_success", 0.1, "1", 0.0, 0.5, "default", Default, "stress relief from a success"),
        figure!("people.stress.r_suppression", 2.0e-4, "1/s", 0.0, 2e-3, "default", Default, "stress rate under full suppression"),
        figure!("people.stress.r_isolated", 5.0e-5, "1/s", 0.0, 1e-3, "default", Default, "stress rate when isolated"),
        figure!("people.stress.r_drone", 5.0e-5, "1/s", 0.0, 1e-3, "default", Default, "stress rate under drone observation"),
        figure!("people.stress.recover_tau_s", 3600.0, "s", 600.0, 86400.0, "default", Default, "stress recovery time constant out of contact"),
        figure!("people.stress.m_leadership", 0.3, "fraction", 0.0, 0.5, "default", Default, "stress shielded by leadership"),
        figure!("people.stress.m_training", 0.2, "fraction", 0.0, 0.5, "default", Default, "stress shielded by training"),
        figure!("people.stress.m_cohesion", 0.2, "fraction", 0.0, 0.5, "default", Default, "stress shielded by cohesion"),
        figure!("people.cohesion.loss_per_casualty", 1.0, "1/fraction", 0.0, 3.0, "default", Default, "cohesion lost per fraction of strength lost"),
        figure!("people.cohesion.gain_success", 0.05, "1", 0.0, 0.3, "default", Default, "cohesion gained per success"),
        figure!("people.morale.theta.shaken", 0.3, "1", 0.1, 0.6, "default", Default, "stress threshold: shaken"),
        figure!("people.morale.theta.pinned", 0.5, "1", 0.2, 0.8, "P14", Low, "stress threshold: pinned (the attack halts)"),
        figure!("people.morale.theta.broken", 0.75, "1", 0.4, 1.0, "default", Default, "stress threshold: broken"),
        figure!("people.morale.theta.routed", 0.95, "1", 0.6, 1.3, "default", Default, "stress threshold: routed"),
        figure!("people.morale.hazard_width", 0.05, "1", 0.01, 0.2, "default", Default, "stress scale of the transition hazards"),
        figure!("people.morale.r_up", 0.0167, "1/s", 0.001, 0.1, "default", Default, "transition rate at the threshold (worse)"),
        figure!("people.morale.r_down", 0.0028, "1/s", 0.0001, 0.05, "default", Default, "recovery rate one band below (better)"),
        figure!("people.fatigue.debt_per_hour_awake", 0.333, "h/h", 0.2, 0.5, "default", Default, "sleep owed per hour awake (8 h per 24 h)"),
        figure!("people.fatigue.onset_debt_h", 8.0, "h", 4.0, 16.0, "P31", Low, "debt at which effectiveness starts falling (24 h awake)"),
        figure!("people.fatigue.loss_per_day", 0.25, "fraction", 0.1, 0.5, "P31", Low, "effectiveness lost per further 24 h awake"),
        figure!("people.fatigue.sleep_recovery", 1.0, "h/h", 0.5, 2.0, "default", Default, "debt repaid per hour of sleep"),
        figure!("people.fatigue.physical_gain_combat", 0.25, "1/h", 0.05, 1.0, "default", Default, "physical fatigue rate in combat"),
        figure!("people.fatigue.physical_gain_light", 0.01, "1/h", 0.0, 0.3, "default", Default, "physical fatigue rate on light duty"),
        figure!("people.fatigue.physical_recover", 0.3, "1/h", 0.05, 1.0, "default", Default, "physical recovery rate at rest"),
        figure!("people.fatigue.physical_k", 0.3, "fraction", 0.0, 0.6, "default", Default, "effectiveness lost at full physical fatigue"),
        figure!("people.fatigue.error_k", 1.0, "1", 0.0, 3.0, "default", Default, "error multiplier per unit of lost effectiveness"),
        figure!("people.skill.experience_scale", 5.0, "engagements", 1.0, 30.0, "default", Default, "engagements to gain ~63% of the experience benefit"),
        figure!("people.skill.max_gain", 0.5, "fraction", 0.0, 1.0, "default", Default, "share of the skill gap experience can close"),
        figure!("people.skill.error_k", 1.5, "1", 0.0, 4.0, "default", Default, "error multiplier per unit of missing skill"),
        figure!("people.reaction.median_s.drilled", 3.0, "s", 0.5, 10.0, "default", Default, "median reaction, rehearsed drill"),
        figure!("people.reaction.median_s.practised", 8.0, "s", 2.0, 30.0, "default", Default, "median reaction, practised task"),
        figure!("people.reaction.median_s.novel", 30.0, "s", 5.0, 300.0, "default", Default, "median reaction, novel situation"),
        figure!("people.reaction.sigma_log", 0.5, "1", 0.1, 1.2, "default", Default, "lognormal spread of reaction times"),
        figure!("people.reaction.skill_k", 1.0, "1", 0.0, 3.0, "default", Default, "reaction slows by this per unit of missing skill"),
        figure!("people.reaction.suppression_k", 2.0, "1", 0.0, 5.0, "P31", Low, "reaction slows by this at full suppression"),
    ];
    let split = [("fragment", [0.2, 0.2, 0.3, 0.27, 0.03]), ("bullet", [0.25, 0.25, 0.25, 0.22, 0.03])];
    for (c, vals) in split {
        for (t, x) in ["killed", "t1", "t2", "t3", "t4"].iter().zip(vals) {
            v.push(figure!(format!("people.casualty.split.{c}.{t}"), x, "fraction", 0.0, 1.0, "default", Default, "casualty split by triage category"));
        }
    }
    let mort = [("t1", 2.0, 1.5), ("t2", 24.0, 1.5), ("t3", 500.0, 1.0), ("t4", 0.75, 1.2)];
    for (t, eta, k) in mort {
        v.push(figure!(format!("people.casualty.mortality.{t}.eta_h"), eta, "h", 0.1, 2000.0, "R7", Low, "Weibull scale of untreated mortality"));
        v.push(figure!(format!("people.casualty.mortality.{t}.k"), k, "1", 0.5, 4.0, "R7", Low, "Weibull shape of untreated mortality"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{figs, rng};
    use approx::assert_relative_eq;
    use proptest::prelude::*;

    #[test]
    fn suppression_rises_decays_and_cover_helps() {
        let p = PeopleParams::from_figures(&figs());
        let fire = IncomingFire { near_misses_per_min: 60.0, blasts_per_min: 0.0 };
        let mut open = Suppression::default();
        let mut covered = Suppression::default();
        open.step(120.0, &fire, 0.0, &p);
        covered.step(120.0, &fire, 1.0, &p);
        assert!(open.level > 0.6 && covered.level < open.level);
        // Step-size independent.
        let mut fine = Suppression::default();
        for _ in 0..120 {
            fine.step(1.0, &fire, 0.0, &p);
        }
        assert_relative_eq!(fine.level, open.level, epsilon = 1e-12);
        let e = open.effects(&p);
        assert!(e.search_mult < 0.6 && e.accuracy_mult < 0.6);
        open.step(600.0, &IncomingFire::default(), 0.0, &p);
        assert!(open.level < 0.01);
        let full = Suppression { level: 1.0 }.effects(&p);
        assert!(full.pinned && full.movement_mult == 0.0);
        assert!((0.1..=0.5).contains(&full.search_mult), "P31: −50–90%");
    }

    /// The attacking-company harness: casualties arrive as a hazard, the
    /// company is suppressed and watched by drones; record the casualty
    /// fraction when the attack halts (pinned or worse).
    fn attack(seed: u64, p: &PeopleParams) -> Option<f64> {
        let mut r = rng(seed);
        let q = UnitQuality { leadership: 0.5, training: 0.5 };
        let mut m = Morale::new(0.6);
        let strength = 120.0;
        let mut alive = strength;
        let loss_rate = 0.005 / 60.0; // 0.5% of strength per minute
        let dt = 5.0;
        for _ in 0..(4 * 3600 / 5) {
            let pl = bgws_core::hazard_probability(loss_rate, dt);
            let lost = rand_distr::Binomial::new(alive as u64, pl).map(|b| b.sample(&mut r)).unwrap_or(0) as f64;
            alive -= lost;
            let inp = StressInputs { new_casualties: lost / strength, suppression: 0.4, drone_observed: true, ..Default::default() };
            m.step(dt, &inp, &q, p, &mut r);
            if m.state.halts_attack() {
                return Some(1.0 - alive / strength);
            }
        }
        None
    }

    #[test]
    fn acceptance_p14_attack_halts_near_20_percent() {
        let p = PeopleParams::from_figures(&figs());
        let mut halts: Vec<f64> = (0..200).filter_map(|s| attack(s, &p)).collect();
        assert!(halts.len() >= 190, "nearly every attack halts: {}", halts.len());
        halts.sort_by(|a, b| a.total_cmp(b));
        let median = halts[halts.len() / 2];
        eprintln!("P14 harness: {} halts, median casualties at halt {median:.3}", halts.len());
        assert!((0.12..=0.28).contains(&median), "P14: median halt at {median}");
        let beyond_40 = halts.iter().filter(|&&c| c > 0.4).count();
        assert!(beyond_40 <= 4, "few attacks carry on past 40%: {beyond_40}");
        let early = halts.iter().filter(|&&c| c < 0.05).count();
        assert!(early <= 10, "few attacks halt with almost no losses: {early}");
    }

    #[test]
    fn good_units_hold_longer_and_recover() {
        let p = PeopleParams::from_figures(&figs());
        let elite = UnitQuality { leadership: 1.0, training: 1.0 };
        let green = UnitQuality { leadership: 0.0, training: 0.0 };
        let m = Morale::new(1.0);
        assert!(m.susceptibility(&elite, &p) < Morale::new(0.0).susceptibility(&green, &p));
        // A shaken unit out of contact recovers to steady.
        let mut r = rng(11);
        let mut m = Morale { stress: 0.4, cohesion: 0.6, casualties: 0.1, state: MoraleState::Shaken };
        for _ in 0..7200 {
            m.step(1.0, &StressInputs::default(), &elite, &p, &mut r);
        }
        assert_eq!(m.state, MoraleState::Steady);
    }

    #[test]
    fn fatigue_onset_24_to_48h_and_recovery() {
        let p = PeopleParams::from_figures(&figs());
        let mut f = Fatigue::default();
        f.step(20.0, WorkLoad::Light, &p);
        let e20 = f.effects(&p).effectiveness;
        f.step(28.0, WorkLoad::Light, &p);
        let e48 = f.effects(&p).effectiveness;
        assert!(e20 > 0.85 && e48 < 0.8 && e48 > 0.5, "{e20} {e48}");
        f.step(10.0, WorkLoad::Sleep, &p);
        assert!(f.effects(&p).effectiveness > e48);
        assert!(f.effects(&p).reaction_mult >= 1.0);
    }

    #[test]
    fn skill_reaction_and_casualties() {
        let p = PeopleParams::from_figures(&figs());
        let mut s = Skill { base: 0.4, experience: 0.0 };
        let before = s.error_mult(&p);
        for _ in 0..10 {
            s.after_engagement(true);
        }
        assert!(s.error_mult(&p) < before && s.effective(&p) <= 1.0);
        let fresh = Fatigue::default().effects(&p);
        let drilled = reaction_median_s(Drill::Drilled, 0.8, &fresh, 0.0, &p);
        let novel = reaction_median_s(Drill::Novel, 0.8, &fresh, 0.0, &p);
        let shot_at = reaction_median_s(Drill::Drilled, 0.8, &fresh, 1.0, &p);
        assert!(drilled < novel && drilled < shot_at);
        let mut r = rng(5);
        let mut xs: Vec<f64> = (0..2001).map(|_| sample_reaction_s(drilled, &p, &mut r)).collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        assert_relative_eq!(xs[1000], drilled, max_relative = 0.08);
        // R7: 60 vs 120 minutes without care.
        let t1_60 = p_die_untreated(Triage::Immediate, 3600.0, &p);
        let t1_120 = p_die_untreated(Triage::Immediate, 7200.0, &p);
        assert!(t1_120 > t1_60 * 1.8 && t1_60 > 0.2, "{t1_60} {t1_120}");
        assert!(p_die_untreated(Triage::Minimal, 7200.0, &p) < 0.01);
        let n = 5000;
        let killed = (0..n).filter(|_| classify_casualty(WoundCause::Fragment, &p, &mut r) == Triage::Killed).count();
        assert_relative_eq!(killed as f64 / n as f64, 0.2, epsilon = 0.02);
    }

    proptest! {
        #[test]
        fn suppression_stays_in_unit_interval(nm in 0.0f64..1000.0, cover in 0.0f64..1.0, dt in 0.0f64..1000.0, s0 in 0.0f64..1.0) {
            let p = PeopleParams::from_figures(&figs());
            let mut s = Suppression { level: s0 };
            s.step(dt, &IncomingFire { near_misses_per_min: nm, blasts_per_min: 0.0 }, cover, &p);
            prop_assert!((0.0..=1.0).contains(&s.level));
        }
    }
}
