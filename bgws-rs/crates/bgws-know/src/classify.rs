//! Classification (§9.1, §9.3): a Bayesian belief over what a track is,
//! updated by every sensor report through that sensor's confusion matrix at
//! the level of look it got (detect, recognise, identify).
//!
//! A decoy looks exactly like the thing it imitates to every sensor and look
//! below an identify-level look by a sensor able to resolve decoys (thermal,
//! multi-band, a close look by eye). Until such a look, a decoy MBT keeps
//! the belief "probable MBT" — the opponent is fooled in the right way
//! **[ref R5.2, P10]** (40% of long-range drones were decoys).
//!
//! The confusion model is shared by the simulation (which draws the reported
//! label from the true class with [`ConfusionModel::draw_report`]) and by the
//! tracker (which updates the belief with the same matrix), so they agree.

use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// The domain a track lives in. Hypotheses, motion noise and drop thresholds depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    Land,
    Air,
    Sea,
}

impl Domain {
    pub const ALL: [Domain; 3] = [Domain::Land, Domain::Air, Domain::Sea];

    pub fn as_str(self) -> &'static str {
        match self {
            Domain::Land => "land",
            Domain::Air => "air",
            Domain::Sea => "sea",
        }
    }

    /// The hypotheses held for a track in this domain, in belief-vector order.
    pub fn hypotheses(self) -> &'static [Hypothesis] {
        use Hypothesis::*;
        match self {
            Domain::Land => &[Mbt, Ifv, Apc, Truck, Artillery, Infantry, LandDecoy],
            Domain::Air => &[Fighter, Bomber, Helicopter, Drone, Missile, AirDecoy],
            Domain::Sea => &[Warship, Merchant, Submarine, Usv],
        }
    }
}

/// What a track might be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hypothesis {
    Mbt,
    Ifv,
    Apc,
    Truck,
    Artillery,
    Infantry,
    /// A land decoy (inflatable or wooden tank, a thermal emitter).
    LandDecoy,
    Fighter,
    Bomber,
    Helicopter,
    Drone,
    Missile,
    /// An air decoy (Gerbera-type drone, towed decoy).
    AirDecoy,
    Warship,
    Merchant,
    Submarine,
    Usv,
}

impl Hypothesis {
    pub fn domain(self) -> Domain {
        use Hypothesis::*;
        match self {
            Mbt | Ifv | Apc | Truck | Artillery | Infantry | LandDecoy => Domain::Land,
            Fighter | Bomber | Helicopter | Drone | Missile | AirDecoy => Domain::Air,
            Warship | Merchant | Submarine | Usv => Domain::Sea,
        }
    }

    pub fn is_decoy(self) -> bool {
        matches!(self, Hypothesis::LandDecoy | Hypothesis::AirDecoy)
    }

    /// What a decoy of this domain imitates **[default]**: a tank on land, a
    /// one-way attack drone in the air.
    pub fn mimic(self) -> Hypothesis {
        match self {
            Hypothesis::LandDecoy => Hypothesis::Mbt,
            Hypothesis::AirDecoy => Hypothesis::Drone,
            h => h,
        }
    }

    /// The look-alike group: confusions mostly happen inside a group.
    fn group(self) -> u8 {
        use Hypothesis::*;
        match self {
            Mbt | Ifv | Apc | LandDecoy => 0,
            Truck => 1,
            Artillery => 2,
            Infantry => 3,
            Fighter | Bomber => 10,
            Helicopter => 11,
            Drone | Missile | AirDecoy => 12,
            Warship | Merchant => 20,
            Submarine => 21,
            Usv => 22,
        }
    }

    fn key(self) -> &'static str {
        use Hypothesis::*;
        match self {
            Mbt => "mbt",
            Ifv => "ifv",
            Apc => "apc",
            Truck => "truck",
            Artillery => "artillery",
            Infantry => "infantry",
            LandDecoy => "decoy",
            Fighter => "fighter",
            Bomber => "bomber",
            Helicopter => "helicopter",
            Drone => "drone",
            Missile => "missile",
            AirDecoy => "decoy",
            Warship => "warship",
            Merchant => "merchant",
            Submarine => "submarine",
            Usv => "usv",
        }
    }

    /// The figure key of this hypothesis' prior.
    pub fn prior_key(self) -> String {
        format!("know.class.prior.{}.{}", self.domain().as_str(), self.key())
    }
}

/// How good a look the sensor got (the Johnson levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdLevel {
    /// Something is there; no class information.
    Detect,
    /// Its class (tank vs truck), with confusion inside look-alike groups.
    Recognise,
    /// Its type, reliably; a decoy is exposed only if the sensor can resolve decoys.
    Identify,
}

/// The classification part of a sensor report.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassEvidence {
    pub level: IdLevel,
    /// The label the sensor (or its operator) reported; `None` at detect level.
    pub reported: Option<Hypothesis>,
    /// Whether this sensor can tell a decoy from the real thing at identify level.
    pub resolves_decoys: bool,
}

impl ClassEvidence {
    /// A bare detection: no class information.
    pub fn detect_only() -> Self {
        ClassEvidence { level: IdLevel::Detect, reported: None, resolves_decoys: false }
    }
}

/// The confusion-matrix figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfusionModel {
    pub p_correct_recognise: f64,
    pub p_correct_identify: f64,
    /// Share of wrong labels that fall inside the true class's look-alike group.
    pub group_share: f64,
    /// P(report "decoy" | decoy) for an identify-level look by a resolving sensor.
    pub p_decoy_resolve: f64,
    /// P(report "decoy" | real) for the same look: calling a real tank a decoy.
    pub p_false_decoy: f64,
    /// Floor on every likelihood, so no report can make a hypothesis impossible.
    pub floor: f64,
    /// Prior per domain, in [`Domain::hypotheses`] order, normalised.
    pub priors: Vec<(Domain, Vec<f64>)>,
}

/// Every classification figure.
pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("know.class.p_correct_recognise", 0.7, "fraction", 0.3, 0.99, "default", Default,
            "P(correct label) at a recognise-level look"),
        figure!("know.class.p_correct_identify", 0.92, "fraction", 0.5, 0.999, "default", Default,
            "P(correct label) at an identify-level look"),
        figure!("know.class.group_share", 0.8, "fraction", 0.0, 1.0, "default", Default,
            "share of wrong labels that stay inside the look-alike group (tank/IFV/APC...)"),
        figure!("know.class.p_decoy_resolve", 0.85, "fraction", 0.1, 0.999, "default", Default,
            "P(decoy called a decoy) by an identify-level look from a decoy-resolving sensor"),
        figure!("know.class.p_false_decoy", 0.03, "fraction", 0.0, 0.3, "default", Default,
            "P(real target called a decoy) by the same look"),
        figure!("know.class.floor", 0.01, "fraction", 1e-6, 0.1, "default", Default,
            "floor on every confusion-matrix entry"),
    ];
    use Hypothesis::*;
    let priors: [(Hypothesis, f64, &str, &str); 17] = [
        (Mbt, 0.15, "default", "land prior"),
        (Ifv, 0.20, "default", "land prior"),
        (Apc, 0.15, "default", "land prior"),
        (Truck, 0.25, "default", "land prior"),
        (Artillery, 0.10, "default", "land prior"),
        (Infantry, 0.10, "default", "land prior"),
        (LandDecoy, 0.05, "default", "land prior: decoys among ground contacts"),
        (Fighter, 0.15, "default", "air prior"),
        (Bomber, 0.05, "default", "air prior"),
        (Helicopter, 0.10, "default", "air prior"),
        (Drone, 0.35, "P10", "air prior: long-range drone salvos ~60% strike / 40% decoy"),
        (Missile, 0.10, "default", "air prior"),
        (AirDecoy, 0.25, "P10", "air prior: ~40% of long-range drones were decoys"),
        (Warship, 0.30, "default", "sea prior"),
        (Merchant, 0.50, "default", "sea prior"),
        (Submarine, 0.05, "default", "sea prior"),
        (Usv, 0.15, "R6.3", "sea prior: cheap uncrewed surface craft in the littoral"),
    ];
    for (h, value, src, note) in priors {
        let mut f = figure!(h.prior_key(), value, "fraction", 0.001, 1.0, src, Default, note);
        if src != "default" {
            f.confidence = bgws_core::Confidence::Low;
        }
        v.push(f);
    }
    v
}

impl ConfusionModel {
    pub fn from_figures(f: &Figures) -> Self {
        let priors = Domain::ALL
            .iter()
            .map(|&d| {
                let raw: Vec<f64> = d.hypotheses().iter().map(|h| f.get(&h.prior_key())).collect();
                let s: f64 = raw.iter().sum();
                (d, raw.iter().map(|p| p / s).collect())
            })
            .collect();
        ConfusionModel {
            p_correct_recognise: f.get("know.class.p_correct_recognise"),
            p_correct_identify: f.get("know.class.p_correct_identify"),
            group_share: f.get("know.class.group_share"),
            p_decoy_resolve: f.get("know.class.p_decoy_resolve"),
            p_false_decoy: f.get("know.class.p_false_decoy"),
            floor: f.get("know.class.floor"),
            priors,
        }
    }

    /// The prior belief for a domain.
    pub fn prior(&self, domain: Domain) -> ClassBelief {
        let probs = self.priors.iter().find(|(d, _)| *d == domain).map(|(_, p)| p.clone()).unwrap_or_else(|| {
            let n = domain.hypotheses().len();
            vec![1.0 / n as f64; n]
        });
        ClassBelief { domain, probs, reports: 0, best_level: IdLevel::Detect }
    }

    /// One row of the confusion matrix: P(reported = r | truth, look), over
    /// the domain's hypotheses, normalised.
    pub fn row(&self, truth: Hypothesis, level: IdLevel, resolves_decoys: bool) -> Vec<f64> {
        let hyps = truth.domain().hypotheses();
        let n = hyps.len();
        if level == IdLevel::Detect {
            return vec![1.0 / n as f64; n];
        }
        let can_call_decoy = level == IdLevel::Identify && resolves_decoys;
        // A decoy that this look cannot expose behaves exactly like what it imitates.
        let seen_as = if truth.is_decoy() && !can_call_decoy { truth.mimic() } else { truth };
        let pc = if level == IdLevel::Identify { self.p_correct_identify } else { self.p_correct_recognise };
        let mut row = vec![0.0; n];
        let decoy_mass = if can_call_decoy {
            if truth.is_decoy() {
                self.p_decoy_resolve
            } else {
                self.p_false_decoy
            }
        } else {
            0.0
        };
        // The label it is taken for: a resolved decoy otherwise looks like its mimic.
        let looks_like = seen_as.mimic();
        let rest = 1.0 - decoy_mass;
        let others: Vec<usize> = (0..n).filter(|&i| hyps[i] != looks_like && !hyps[i].is_decoy()).collect();
        let in_group: Vec<usize> = others.iter().copied().filter(|&i| hyps[i].group() == looks_like.group()).collect();
        let out_group: Vec<usize> = others.iter().copied().filter(|&i| hyps[i].group() != looks_like.group()).collect();
        let wrong = rest * (1.0 - pc);
        let (g, o) = match (in_group.is_empty(), out_group.is_empty()) {
            (false, false) => (self.group_share, 1.0 - self.group_share),
            (false, true) => (1.0, 0.0),
            (true, false) => (0.0, 1.0),
            (true, true) => (0.0, 0.0),
        };
        for (i, h) in hyps.iter().enumerate() {
            row[i] = if h.is_decoy() {
                decoy_mass
            } else if *h == looks_like {
                rest * pc + if others.is_empty() { wrong } else { 0.0 }
            } else if in_group.contains(&i) {
                wrong * g / in_group.len() as f64
            } else {
                wrong * o / out_group.len().max(1) as f64
            };
        }
        for x in row.iter_mut() {
            *x = x.max(self.floor);
        }
        let s: f64 = row.iter().sum();
        row.iter().map(|x| x / s).collect()
    }

    /// The likelihood of a report under each hypothesis (a column of the matrix).
    pub fn likelihood(&self, domain: Domain, ev: &ClassEvidence) -> Vec<f64> {
        let hyps = domain.hypotheses();
        let Some(rep) = ev.reported.filter(|r| r.domain() == domain && ev.level != IdLevel::Detect) else {
            return vec![1.0; hyps.len()];
        };
        let ri = hyps.iter().position(|h| *h == rep).unwrap_or(0);
        hyps.iter().map(|&h| self.row(h, ev.level, ev.resolves_decoys)[ri]).collect()
    }

    /// What a sensor reports about a target of class `truth` (for the simulation).
    pub fn draw_report(&self, truth: Hypothesis, level: IdLevel, resolves_decoys: bool, rng: &mut impl Rng) -> ClassEvidence {
        if level == IdLevel::Detect {
            return ClassEvidence { level, reported: None, resolves_decoys };
        }
        let row = self.row(truth, level, resolves_decoys);
        let u: f64 = rng.gen();
        let hyps = truth.domain().hypotheses();
        let mut acc = 0.0;
        let mut pick = hyps[hyps.len() - 1];
        for (i, p) in row.iter().enumerate() {
            acc += p;
            if u < acc {
                pick = hyps[i];
                break;
            }
        }
        ClassEvidence { level, reported: Some(pick), resolves_decoys }
    }
}

/// A belief over the hypotheses of one domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassBelief {
    pub domain: Domain,
    /// In [`Domain::hypotheses`] order; sums to 1.
    pub probs: Vec<f64>,
    /// How many class-bearing reports went in.
    pub reports: u32,
    /// The best look so far.
    pub best_level: IdLevel,
}

impl ClassBelief {
    /// Bayes: posterior ∝ prior × likelihood.
    pub fn update(&mut self, model: &ConfusionModel, ev: &ClassEvidence) {
        if ev.level == IdLevel::Detect || ev.reported.is_none() {
            return;
        }
        let l = model.likelihood(self.domain, ev);
        for (p, li) in self.probs.iter_mut().zip(l) {
            *p *= li;
        }
        self.normalise();
        self.reports += 1;
        self.best_level = self.best_level.max(ev.level);
    }

    fn normalise(&mut self) {
        let s: f64 = self.probs.iter().sum();
        if s > 0.0 && s.is_finite() {
            for p in self.probs.iter_mut() {
                *p /= s;
            }
        } else {
            let n = self.probs.len() as f64;
            self.probs.iter_mut().for_each(|p| *p = 1.0 / n);
        }
    }

    /// Combine two independent beliefs about the same object: a·b / prior.
    pub fn merge_independent(&mut self, other: &ClassBelief, prior: &ClassBelief) {
        if other.domain != self.domain {
            return;
        }
        for i in 0..self.probs.len() {
            self.probs[i] = self.probs[i] * other.probs[i] / prior.probs[i].max(1e-300);
        }
        self.normalise();
        self.reports += other.reports;
        self.best_level = self.best_level.max(other.best_level);
    }

    /// Log-linear pooling with weight ω on self (the discrete analogue of
    /// covariance intersection: safe when the two may share evidence).
    pub fn pool(&mut self, other: &ClassBelief, omega: f64) {
        if other.domain != self.domain {
            return;
        }
        for i in 0..self.probs.len() {
            let (a, b) = (self.probs[i].max(1e-300), other.probs[i].max(1e-300));
            self.probs[i] = (omega * a.ln() + (1.0 - omega) * b.ln()).exp();
        }
        self.normalise();
        self.reports = self.reports.max(other.reports);
        self.best_level = self.best_level.max(other.best_level);
    }

    /// The probability of one hypothesis.
    pub fn prob(&self, h: Hypothesis) -> f64 {
        self.domain.hypotheses().iter().position(|x| *x == h).map(|i| self.probs[i]).unwrap_or(0.0)
    }

    /// The `k` most likely hypotheses, best first (ties in hypothesis order).
    pub fn top_k(&self, k: usize) -> Vec<(Hypothesis, f64)> {
        let mut v: Vec<(Hypothesis, f64)> =
            self.domain.hypotheses().iter().copied().zip(self.probs.iter().copied()).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(k);
        v
    }

    /// The most likely hypothesis and its probability.
    pub fn top(&self) -> (Hypothesis, f64) {
        self.top_k(1)[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use rand::SeedableRng;

    fn model() -> ConfusionModel {
        ConfusionModel::from_figures(&Figures::from_list(figures()).unwrap())
    }

    fn ev(level: IdLevel, r: Hypothesis, resolves: bool) -> ClassEvidence {
        ClassEvidence { level, reported: Some(r), resolves_decoys: resolves }
    }

    #[test]
    fn rows_are_distributions() {
        let m = model();
        for d in Domain::ALL {
            for &h in d.hypotheses() {
                for level in [IdLevel::Detect, IdLevel::Recognise, IdLevel::Identify] {
                    for res in [false, true] {
                        let r = m.row(h, level, res);
                        assert_relative_eq!(r.iter().sum::<f64>(), 1.0, epsilon = 1e-12);
                        assert!(r.iter().all(|x| *x > 0.0));
                    }
                }
            }
        }
    }

    #[test]
    fn decoy_stays_probable_mbt_until_identified_by_a_resolving_sensor() {
        let m = model();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let mut b = m.prior(Domain::Land);
        // Ten recognise-level looks and five identify-level looks by a sensor
        // that cannot resolve decoys: the decoy reports as what it imitates.
        for _ in 0..10 {
            let e = m.draw_report(Hypothesis::LandDecoy, IdLevel::Recognise, false, &mut rng);
            b.update(&m, &e);
        }
        for _ in 0..5 {
            b.update(&m, &m.draw_report(Hypothesis::LandDecoy, IdLevel::Identify, false, &mut rng));
        }
        let (top, p) = b.top();
        assert_eq!(top, Hypothesis::Mbt);
        assert!(p > 0.6, "probable MBT, p = {p}");
        // The decoy share is exactly prior-proportional: no look could tell them apart.
        assert_relative_eq!(b.prob(Hypothesis::LandDecoy) / b.prob(Hypothesis::Mbt), 0.05 / 0.15, epsilon = 1e-9);
        // A good look: identify-level from a thermal sight that resolves decoys.
        b.update(&m, &ev(IdLevel::Identify, Hypothesis::LandDecoy, true));
        b.update(&m, &ev(IdLevel::Identify, Hypothesis::LandDecoy, true));
        assert_eq!(b.top().0, Hypothesis::LandDecoy);
    }

    #[test]
    fn recognise_confuses_within_group() {
        let m = model();
        let r = m.row(Hypothesis::Mbt, IdLevel::Recognise, false);
        let hyps = Domain::Land.hypotheses();
        let at = |h| r[hyps.iter().position(|x| *x == h).unwrap()];
        assert!(at(Hypothesis::Ifv) > at(Hypothesis::Truck));
        assert!(at(Hypothesis::Mbt) > 0.6);
        let mut b = m.prior(Domain::Land);
        b.update(&m, &ev(IdLevel::Identify, Hypothesis::Truck, false));
        assert_eq!(b.top().0, Hypothesis::Truck);
        let top = b.top_k(3);
        assert_eq!(top.len(), 3);
        assert!(top[0].1 >= top[1].1 && top[1].1 >= top[2].1);
        // Detect-level reports carry no class information.
        let before = b.clone();
        b.update(&m, &ClassEvidence::detect_only());
        assert_eq!(b, before);
    }

    #[test]
    fn pooling_same_belief_is_identity_but_independent_merge_sharpens() {
        let m = model();
        let mut a = m.prior(Domain::Air);
        a.update(&m, &ev(IdLevel::Recognise, Hypothesis::Fighter, false));
        let mut pooled = a.clone();
        pooled.pool(&a, 0.5);
        for (x, y) in pooled.probs.iter().zip(&a.probs) {
            assert_relative_eq!(x, y, epsilon = 1e-12);
        }
        let mut merged = a.clone();
        merged.merge_independent(&a, &m.prior(Domain::Air));
        assert!(merged.prob(Hypothesis::Fighter) > a.prob(Hypothesis::Fighter));
    }
}
