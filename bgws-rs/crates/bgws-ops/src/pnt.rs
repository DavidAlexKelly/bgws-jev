//! Position, navigation and timing (RUST_ENGINE.md §8.7; research R5.3, P15, P16).
//!
//! - **GNSS under jamming:** the effective carrier-to-noise density follows
//!   the standard spectral-separation form
//!   1/(C/N₀)ₑ = 1/(C/N₀) + (J/S)/(Q·R_c). Below the tracking threshold the
//!   receiver loses lock; it reacquires only after C/N₀ has stayed above the
//!   (higher) acquisition threshold for the reacquisition time.
//! - **Spoofing** that out-powers the authentic signal by a capture margin
//!   drags the solution toward the spoofer's offset at a drag rate (a
//!   receiver that is reacquiring is captured at once). Anti-spoof receivers ignore it.
//! - **INS drift** grows from the last good fix by grade:
//!   σ²(t) = σ₀² + q·t + (v·t)² + (½·a·t²)² (random walk, velocity error,
//!   accelerometer bias). Units carry this as their self-position covariance.
//! - **Precision munitions' CEP** follows from the navigation state at
//!   release and in flight, combined with the guidance error; with a bias
//!   (spoofing) the miss distance is Rician. Excalibur-type Pk falling from
//!   >50% to <10% under jamming **[P15]** and jamming radii of ≥25 km per
//!   module **[P16]** are acceptance tests, not inputs.

use crate::math::{bisect, db_to_ratio, ratio_to_db};
use bgws_core::{figure, Enu, Figure, Figures, SimTime};
use serde::{Deserialize, Serialize};

/// Figures for PNT (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("pnt.gnss.signal_dbw", -160.0, "dBW", -166.0, -150.0, "R5.3", High, "GPS L1 C/A at the antenna, about −130 dBm"),
        figure!("pnt.gnss.chip_rate_hz", 1.023e6, "Hz", 1.023e6, 10.23e6, "default", Medium, "C/A chip rate (P(Y)/M 10.23 Mchip/s)"),
        figure!("pnt.gnss.q_factor", 1.0, "1", 0.5, 2.0, "default", Default, "spectral separation factor: 1 narrowband, ~1.5 matched, 2 wideband"),
        figure!("pnt.gnss.cn0_nominal_dbhz", 45.0, "dB-Hz", 35.0, 50.0, "default", Default, "unjammed C/N0, open sky"),
        figure!("pnt.gnss.track_threshold_dbhz", 28.0, "dB-Hz", 20.0, 32.0, "default", Default, "loss-of-lock threshold"),
        figure!("pnt.gnss.acquire_threshold_dbhz", 33.0, "dB-Hz", 28.0, 38.0, "default", Default, "acquisition threshold (hysteresis above tracking)"),
        figure!("pnt.gnss.reacquire_s", 30.0, "s", 1.0, 300.0, "default", Default, "time above acquisition threshold to regain lock (warm start)"),
        figure!("pnt.gnss.fix_sigma_m", 3.0, "m", 1.0, 15.0, "default", Default, "1σ horizontal error of a good GNSS fix per axis"),
        figure!("pnt.gnss.crpa_nulling_db", 30.0, "dB", 0.0, 50.0, "default", Default, "J/S suppression by a CRPA antenna when fitted"),
        figure!("pnt.spoof.capture_margin_db", 3.0, "dB", 0.0, 15.0, "default", Default, "spoofer power over authentic needed to capture tracking"),
        figure!("pnt.spoof.drag_rate_mps", 2.0, "m/s", 0.1, 50.0, "default", Default, "how fast a careful spoofer drags the solution"),
        figure!("pnt.ins.nav.q_m2ps", 0.01, "m2/s", 0.0, 1.0, "default", Default, "navigation-grade INS position random walk"),
        figure!("pnt.ins.nav.v_mps", 0.5, "m/s", 0.05, 2.0, "default", Default, "navigation-grade drift, ~1 nmi/h"),
        figure!("pnt.ins.nav.a_mps2", 0.0, "m/s2", 0.0, 0.001, "default", Default, "navigation-grade residual accel bias (Schuler-bounded)"),
        figure!("pnt.ins.tactical.q_m2ps", 0.1, "m2/s", 0.0, 10.0, "default", Default, "tactical-grade random walk"),
        figure!("pnt.ins.tactical.v_mps", 0.1, "m/s", 0.01, 2.0, "default", Default, "tactical-grade velocity error after alignment"),
        figure!("pnt.ins.tactical.a_mps2", 0.0098, "m/s2", 0.001, 0.05, "default", Default, "tactical-grade accel bias, ~1 mg"),
        figure!("pnt.ins.mems.q_m2ps", 1.0, "m2/s", 0.0, 50.0, "default", Default, "consumer MEMS random walk"),
        figure!("pnt.ins.mems.v_mps", 0.3, "m/s", 0.01, 5.0, "default", Default, "consumer MEMS velocity error"),
        figure!("pnt.ins.mems.a_mps2", 0.049, "m/s2", 0.005, 0.5, "default", Default, "consumer MEMS accel bias, ~5 mg"),
        figure!("pnt.munition.guided_cep_m", 4.0, "m", 1.0, 20.0, "P15", Low, "Excalibur-type CEP with GNSS"),
        figure!("pnt.munition.launch_sigma_m", 20.0, "m", 2.0, 100.0, "default", Default, "1σ per-axis error at launch with no GNSS (gun laying, met)"),
    ]
}

/// CEP of a circular normal with per-axis σ: CEP = σ·√(2 ln 2).
pub const CEP_PER_SIGMA: f64 = 1.177_410_022_515_474_6;

/// GNSS receiver parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GnssParams {
    pub signal_dbw: f64,
    pub chip_rate_hz: f64,
    pub q: f64,
    pub cn0_nominal_dbhz: f64,
    pub track_dbhz: f64,
    pub acquire_dbhz: f64,
    pub reacquire_s: f64,
    pub fix_sigma_m: f64,
    /// Antenna nulling (CRPA) against jamming; 0 if not fitted.
    pub nulling_db: f64,
    pub anti_spoof: bool,
    pub capture_margin_db: f64,
    pub drag_rate_mps: f64,
}

impl GnssParams {
    /// A standard (non-CRPA, no anti-spoof) receiver from figures.
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            signal_dbw: f.get("pnt.gnss.signal_dbw"),
            chip_rate_hz: f.get("pnt.gnss.chip_rate_hz"),
            q: f.get("pnt.gnss.q_factor"),
            cn0_nominal_dbhz: f.get("pnt.gnss.cn0_nominal_dbhz"),
            track_dbhz: f.get("pnt.gnss.track_threshold_dbhz"),
            acquire_dbhz: f.get("pnt.gnss.acquire_threshold_dbhz"),
            reacquire_s: f.get("pnt.gnss.reacquire_s"),
            fix_sigma_m: f.get("pnt.gnss.fix_sigma_m"),
            nulling_db: 0.0,
            anti_spoof: false,
            capture_margin_db: f.get("pnt.spoof.capture_margin_db"),
            drag_rate_mps: f.get("pnt.spoof.drag_rate_mps"),
        }
    }

    /// The same receiver with a CRPA antenna.
    pub fn with_crpa(mut self, f: &Figures) -> Self {
        self.nulling_db = f.get("pnt.gnss.crpa_nulling_db");
        self
    }

    /// Effective C/N₀ (dB-Hz) at a jamming level J (dBW at the antenna).
    pub fn effective_cn0(&self, j_dbw: f64) -> f64 {
        let js = j_dbw - self.nulling_db - self.signal_dbw;
        effective_cn0(self.cn0_nominal_dbhz, js, self.q, self.chip_rate_hz)
    }

    /// The J/S (dB, before nulling) at which the receiver loses lock.
    pub fn js_loss_of_lock_db(&self) -> f64 {
        let inv = 1.0 / db_to_ratio(self.track_dbhz) - 1.0 / db_to_ratio(self.cn0_nominal_dbhz);
        ratio_to_db(inv.max(1e-30) * self.q * self.chip_rate_hz) + self.nulling_db
    }
}

/// 1/(C/N₀)ₑ = 1/(C/N₀) + (J/S)/(Q·R_c), all in dB / dB-Hz.
pub fn effective_cn0(cn0_dbhz: f64, js_db: f64, q: f64, chip_rate_hz: f64) -> f64 {
    let inv = 1.0 / db_to_ratio(cn0_dbhz) + if js_db.is_finite() { db_to_ratio(js_db) / (q * chip_rate_hz) } else { 0.0 };
    ratio_to_db(1.0 / inv)
}

/// The jammer-to-receiver range inside which a GNSS receiver loses lock,
/// for a jammer of `erp_dbw` (in the GNSS band) and a path-loss function of
/// range **[P16]**. `None` if the jammer can't deny even at 1 m.
pub fn gnss_denial_radius(p: &GnssParams, erp_dbw: f64, path_loss_db: impl Fn(f64) -> f64, max_m: f64) -> Option<f64> {
    let thr = p.js_loss_of_lock_db();
    let js = |r: f64| erp_dbw - path_loss_db(r) - p.signal_dbw;
    if js(max_m) >= thr {
        return Some(max_m);
    }
    bisect(1.0, max_m, |r| js(r) - thr)
}

/// A spoofer's signal at a receiver.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpoofInput {
    /// Spoofed power over authentic, dB.
    pub margin_db: f64,
    /// Where the spoofer wants the solution to be, relative to the truth.
    pub target_offset: Enu,
}

/// The receiver's lock state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GnssLock {
    Locked,
    Unlocked { since: SimTime },
    Reacquiring { since: SimTime },
}

/// What the receiver reports this step (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GnssStatus {
    pub at: SimTime,
    pub cn0_dbhz: f64,
    pub lock: GnssLock,
    /// The solution's offset from truth (spoofing); zero when honest.
    pub solution_offset: Enu,
    pub spoofed: bool,
    /// Whether the receiver is producing a fix (locked).
    pub has_fix: bool,
}

/// A GNSS receiver's state machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GnssTracker {
    pub params: GnssParams,
    pub lock: GnssLock,
    pub offset: Enu,
    last: SimTime,
}

impl GnssTracker {
    pub fn new(params: GnssParams, t: SimTime) -> Self {
        Self { params, lock: GnssLock::Locked, offset: Enu::default(), last: t }
    }

    /// Advance to `t` with jamming `j_dbw` at the antenna and an optional spoofer.
    pub fn update(&mut self, t: SimTime, j_dbw: f64, spoof: Option<&SpoofInput>) -> GnssStatus {
        let dt = (t - self.last).secs().max(0.0);
        self.last = t;
        let p = &self.params;
        let captured = spoof.filter(|s| !p.anti_spoof && s.margin_db >= p.capture_margin_db);
        // A capturing spoofer supplies its own (strong) signal: C/N0 doesn't collapse.
        let cn0 = if let Some(s) = captured { effective_cn0(p.cn0_nominal_dbhz + s.margin_db, j_dbw - p.nulling_db - p.signal_dbw - s.margin_db, p.q, p.chip_rate_hz) } else { p.effective_cn0(j_dbw) };
        let was_reacquiring = matches!(self.lock, GnssLock::Reacquiring { .. } | GnssLock::Unlocked { .. });
        self.lock = match self.lock {
            GnssLock::Locked if cn0 < p.track_dbhz => GnssLock::Unlocked { since: t },
            GnssLock::Locked => GnssLock::Locked,
            GnssLock::Unlocked { .. } if cn0 >= p.acquire_dbhz => GnssLock::Reacquiring { since: t },
            GnssLock::Unlocked { since } => GnssLock::Unlocked { since },
            GnssLock::Reacquiring { .. } if cn0 < p.acquire_dbhz => GnssLock::Unlocked { since: t },
            GnssLock::Reacquiring { since } if (t - since).secs() >= p.reacquire_s => GnssLock::Locked,
            GnssLock::Reacquiring { since } => GnssLock::Reacquiring { since },
        };
        // A reacquisition with a spoofer captures the solution at once; else drag it.
        match captured {
            Some(s) if was_reacquiring && self.lock == GnssLock::Locked => self.offset = s.target_offset,
            Some(s) => {
                let d = s.target_offset.sub(self.offset);
                let step = p.drag_rate_mps * dt;
                let n = d.norm();
                self.offset = if n <= step { s.target_offset } else { self.offset.add(d.scale(step / n)) };
            }
            None => self.offset = Enu::default(),
        }
        let has_fix = self.lock == GnssLock::Locked;
        GnssStatus { at: t, cn0_dbhz: cn0, lock: self.lock, solution_offset: self.offset, spoofed: self.offset.norm() > 0.0, has_fix }
    }
}

/// INS grades.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsGrade {
    Navigation,
    Tactical,
    Mems,
}

/// INS error growth terms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsModel {
    pub q_m2ps: f64,
    pub v_mps: f64,
    pub a_mps2: f64,
}

impl InsModel {
    pub fn from_figures(g: InsGrade, f: &Figures) -> Self {
        let k = match g {
            InsGrade::Navigation => "nav",
            InsGrade::Tactical => "tactical",
            InsGrade::Mems => "mems",
        };
        Self {
            q_m2ps: f.get(&format!("pnt.ins.{k}.q_m2ps")),
            v_mps: f.get(&format!("pnt.ins.{k}.v_mps")),
            a_mps2: f.get(&format!("pnt.ins.{k}.a_mps2")),
        }
    }

    /// Per-axis 1σ position error `t` seconds after a fix of `sigma0`.
    pub fn sigma_after(&self, sigma0: f64, t: f64) -> f64 {
        let t = t.max(0.0);
        (sigma0 * sigma0 + self.q_m2ps * t + (self.v_mps * t).powi(2) + (0.5 * self.a_mps2 * t * t).powi(2)).sqrt()
    }
}

/// A unit's or munition's navigation state: GNSS + INS.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavState {
    pub ins: InsModel,
    pub last_fix: SimTime,
    pub fix_sigma_m: f64,
    /// Bias the platform doesn't know about (spoofing), metres ENU.
    pub bias: Enu,
}

/// A position belief with its covariance (rule 1: units report this, not truth).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfPosition {
    /// Horizontal covariance, m² (east, north).
    pub covariance: [[f64; 2]; 2],
    pub sigma_m: f64,
    pub cep_m: f64,
    /// The error the platform doesn't know about.
    pub bias: Enu,
    pub since_fix_s: f64,
}

impl NavState {
    pub fn new(ins: InsModel, t: SimTime, fix_sigma_m: f64) -> Self {
        Self { ins, last_fix: t, fix_sigma_m, bias: Enu::default() }
    }

    /// Take a GNSS status: a fix resets the INS (and a spoofed fix plants its bias).
    pub fn update(&mut self, s: &GnssStatus, fix_sigma_m: f64) {
        if s.has_fix {
            self.last_fix = s.at;
            self.fix_sigma_m = fix_sigma_m;
            self.bias = s.solution_offset;
        }
    }

    /// The self-position belief at `t`.
    pub fn self_position(&self, t: SimTime) -> SelfPosition {
        let dt = (t - self.last_fix).secs();
        let s = self.ins.sigma_after(self.fix_sigma_m, dt);
        SelfPosition { covariance: [[s * s, 0.0], [0.0, s * s]], sigma_m: s, cep_m: CEP_PER_SIGMA * s, bias: self.bias, since_fix_s: dt }
    }
}

/// P(miss ≤ r) for a 2-D isotropic normal of per-axis σ offset by `bias`
/// (the Rician CDF), by Simpson integration of the Rice density.
pub fn p_within(r: f64, sigma: f64, bias: f64) -> f64 {
    if r <= 0.0 {
        return 0.0;
    }
    if bias <= 1e-12 {
        return 1.0 - (-r * r / (2.0 * sigma * sigma)).exp();
    }
    let n = 400;
    let h = r / n as f64;
    let s2 = sigma * sigma;
    let pdf = |x: f64| {
        let z = x * bias / s2;
        x / s2 * (-(x - bias).powi(2) / (2.0 * s2)).exp() * bessel_i0e(z)
    };
    let mut sum = pdf(0.0) + pdf(r);
    for i in 1..n {
        let x = i as f64 * h;
        sum += if i % 2 == 1 { 4.0 } else { 2.0 } * pdf(x);
    }
    (sum * h / 3.0).clamp(0.0, 1.0)
}

/// Exponentially scaled modified Bessel function I₀(x)·e^(−x), x ≥ 0
/// (Abramowitz & Stegun 9.8.1–9.8.2).
pub fn bessel_i0e(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let y = (x / 3.75).powi(2);
        let i0 = 1.0 + y * (3.515_622_9 + y * (3.089_942_4 + y * (1.206_749_2 + y * (0.265_973_2 + y * (0.036_076_8 + y * 0.004_581_3)))));
        i0 * (-ax).exp()
    } else {
        let y = 3.75 / ax;
        (0.398_942_28
            + y * (0.013_285_92 + y * (0.002_253_19 + y * (-0.001_575_65 + y * (0.009_162_81 + y * (-0.020_577_06 + y * (0.026_355_37 + y * (-0.016_476_33 + y * 0.003_923_77))))))))
            / ax.sqrt()
    }
}

/// A precision munition's navigation in flight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MunitionNav {
    /// CEP of the guidance and control with perfect navigation.
    pub guidance_cep_m: f64,
    pub ins: InsModel,
    pub flight_time_s: f64,
    /// Seconds before impact that GNSS was lost (None: available throughout;
    /// ≥ flight time: denied from launch).
    pub gnss_lost_before_impact_s: Option<f64>,
    /// Per-axis error at launch when GNSS is unavailable at release.
    pub launch_sigma_m: f64,
    /// Per-axis error of a GNSS fix.
    pub fix_sigma_m: f64,
    /// Spoofing bias carried to impact.
    pub bias_m: f64,
}

/// Accuracy of a precision munition (rule 10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MunitionAccuracy {
    pub guidance_sigma_m: f64,
    pub nav_sigma_m: f64,
    pub total_sigma_m: f64,
    pub bias_m: f64,
    pub cep_m: f64,
    pub inertial_s: f64,
}

impl MunitionNav {
    /// Defaults for an Excalibur-type round from figures.
    pub fn guided_shell(f: &Figures, flight_time_s: f64, gnss_lost_before_impact_s: Option<f64>) -> Self {
        Self {
            guidance_cep_m: f.get("pnt.munition.guided_cep_m"),
            ins: InsModel::from_figures(InsGrade::Tactical, f),
            flight_time_s,
            gnss_lost_before_impact_s,
            launch_sigma_m: f.get("pnt.munition.launch_sigma_m"),
            fix_sigma_m: f.get("pnt.gnss.fix_sigma_m"),
            bias_m: 0.0,
        }
    }

    pub fn accuracy(&self) -> MunitionAccuracy {
        let inertial = self.gnss_lost_before_impact_s.map(|s| s.clamp(0.0, self.flight_time_s)).unwrap_or(0.0);
        let denied_at_launch = inertial >= self.flight_time_s;
        let sigma0 = if denied_at_launch { self.launch_sigma_m } else { self.fix_sigma_m };
        let nav = self.ins.sigma_after(sigma0, inertial);
        let guidance = self.guidance_cep_m / CEP_PER_SIGMA;
        let total = (nav * nav + guidance * guidance).sqrt();
        let cep = if self.bias_m > 0.0 { bisect(0.0, 10.0 * (total + self.bias_m), |r| p_within(r, total, self.bias_m) - 0.5).unwrap_or(f64::NAN) } else { CEP_PER_SIGMA * total };
        MunitionAccuracy { guidance_sigma_m: guidance, nav_sigma_m: nav, total_sigma_m: total, bias_m: self.bias_m, cep_m: cep, inertial_s: inertial }
    }
}

impl MunitionAccuracy {
    /// Probability of landing within `lethal_radius_m` of the aim point.
    pub fn p_within(&self, lethal_radius_m: f64) -> f64 {
        p_within(lethal_radius_m, self.total_sigma_m, self.bias_m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::free_space_loss_db;
    use approx::assert_relative_eq;

    fn figs() -> Figures {
        crate::default_figures()
    }

    #[test]
    fn effective_cn0_and_threshold_agree() {
        let p = GnssParams::from_figures(&figs());
        assert_relative_eq!(effective_cn0(45.0, f64::NEG_INFINITY, 1.0, 1.023e6), 45.0, epsilon = 1e-9);
        let js = p.js_loss_of_lock_db();
        assert!((30.0..35.0).contains(&js), "{js}");
        assert_relative_eq!(effective_cn0(45.0, js, 1.0, 1.023e6), 28.0, epsilon = 1e-9);
        let crpa = p.clone().with_crpa(&figs());
        assert_relative_eq!(crpa.js_loss_of_lock_db() - js, 30.0, epsilon = 1e-9);
    }

    #[test]
    fn lock_is_lost_and_reacquired_with_hysteresis() {
        let p = GnssParams::from_figures(&figs());
        let mut g = GnssTracker::new(p.clone(), SimTime::ZERO);
        let jam = p.signal_dbw + p.js_loss_of_lock_db() + 3.0;
        assert!(!g.update(SimTime::from_secs_i(1), jam, None).has_fix);
        // Between the thresholds: stays unlocked.
        let mid = p.signal_dbw + p.js_loss_of_lock_db() - 1.0;
        assert!(matches!(g.update(SimTime::from_secs_i(2), mid, None).lock, GnssLock::Unlocked { .. }));
        // Clear: reacquiring, then locked after 30 s.
        assert!(matches!(g.update(SimTime::from_secs_i(3), f64::NEG_INFINITY, None).lock, GnssLock::Reacquiring { .. }));
        assert!(!g.update(SimTime::from_secs_i(20), f64::NEG_INFINITY, None).has_fix);
        assert!(g.update(SimTime::from_secs_i(33), f64::NEG_INFINITY, None).has_fix);
    }

    #[test]
    fn spoofing_drags_the_solution() {
        let f = figs();
        let p = GnssParams::from_figures(&f);
        let mut g = GnssTracker::new(p.clone(), SimTime::ZERO);
        let s = SpoofInput { margin_db: 6.0, target_offset: Enu::new(1000.0, 0.0, 0.0) };
        let st = g.update(SimTime::from_secs_i(100), f64::NEG_INFINITY, Some(&s));
        assert!(st.has_fix && st.spoofed);
        assert_relative_eq!(st.solution_offset.e, 200.0, epsilon = 1e-9);
        let st = g.update(SimTime::from_secs_i(1000), f64::NEG_INFINITY, Some(&s));
        assert_relative_eq!(st.solution_offset.e, 1000.0, epsilon = 1e-9);
        // The nav state believes it, and carries the bias.
        let mut nav = NavState::new(InsModel::from_figures(InsGrade::Tactical, &f), SimTime::ZERO, 3.0);
        nav.update(&st, 3.0);
        assert_relative_eq!(nav.self_position(SimTime::from_secs_i(1000)).bias.e, 1000.0);
        // An anti-spoof receiver ignores it.
        let mut safe = GnssTracker::new(GnssParams { anti_spoof: true, ..p }, SimTime::ZERO);
        assert!(!safe.update(SimTime::from_secs_i(100), f64::NEG_INFINITY, Some(&s)).spoofed);
    }

    #[test]
    fn ins_drift_grows_by_grade() {
        let f = figs();
        let nav = InsModel::from_figures(InsGrade::Navigation, &f);
        let tac = InsModel::from_figures(InsGrade::Tactical, &f);
        let mems = InsModel::from_figures(InsGrade::Mems, &f);
        for t in [60.0, 600.0] {
            assert!(nav.sigma_after(3.0, t) < tac.sigma_after(3.0, t) || t < 100.0);
            assert!(tac.sigma_after(3.0, t) < mems.sigma_after(3.0, t));
        }
        // Navigation grade ≈ 1 nmi/h.
        let one_hour = nav.sigma_after(3.0, 3600.0);
        assert!((1500.0..2500.0).contains(&one_hour), "{one_hour}");
        let s = NavState::new(tac, SimTime::ZERO, 3.0).self_position(SimTime::from_secs_i(120));
        assert_relative_eq!(s.cep_m, s.sigma_m * CEP_PER_SIGMA);
        assert!(s.sigma_m > 3.0);
    }

    #[test]
    fn rician_reduces_to_rayleigh_and_bessel_is_right() {
        assert_relative_eq!(p_within(5.0, 3.0, 1e-9), 1.0 - (-25.0f64 / 18.0).exp(), epsilon = 1e-9);
        assert_relative_eq!(p_within(5.0, 3.0, 1e-3), 1.0 - (-25.0f64 / 18.0).exp(), epsilon = 1e-4);
        assert_relative_eq!(bessel_i0e(1.0), 1.266_065_877_8 * (-1.0f64).exp(), epsilon = 1e-6);
        assert_relative_eq!(bessel_i0e(10.0), 2_815.716_628 * (-10.0f64).exp(), max_relative = 1e-5);
        // A large bias pushes everything outside.
        assert!(p_within(5.0, 3.0, 50.0) < 1e-6);
    }

    /// Acceptance [P15]: an Excalibur-type round's Pk is above 50% with GNSS
    /// and below 10% when GNSS is denied for the whole flight. (Point target,
    /// 7 m lethal radius, 70 s flight; Pk here is P(within lethal radius).)
    #[test]
    fn excalibur_pk_collapses_under_jamming() {
        let f = figs();
        let lethal = 7.0;
        let clear = MunitionNav::guided_shell(&f, 70.0, None).accuracy();
        let terminal = MunitionNav::guided_shell(&f, 70.0, Some(20.0)).accuracy();
        let denied = MunitionNav::guided_shell(&f, 70.0, Some(70.0)).accuracy();
        let (pc, pt, pd) = (clear.p_within(lethal), terminal.p_within(lethal), denied.p_within(lethal));
        assert!(pc > 0.5, "clear {pc} {clear:?}");
        assert!(pd < 0.1, "denied {pd} {denied:?}");
        assert!(pc >= pt && pt > pd);
        let mut spoofed = MunitionNav::guided_shell(&f, 70.0, None);
        spoofed.bias_m = 30.0;
        let sp = spoofed.accuracy();
        assert!(sp.cep_m > 25.0 && sp.p_within(lethal) < 0.05);
    }

    /// A ground-level two-ray path loss: free space to the breakpoint
    /// 4πh₁h₂/λ, then 40 dB/decade (ground wave).
    fn two_ray(d: f64, f_hz: f64, h1: f64, h2: f64) -> f64 {
        let lambda = bgws_core::units::SPEED_OF_LIGHT / f_hz;
        let dc = 4.0 * std::f64::consts::PI * h1 * h2 / lambda;
        if d <= dc {
            free_space_loss_db(d, f_hz)
        } else {
            free_space_loss_db(dc, f_hz) + 40.0 * (d / dc).log10()
        }
    }

    /// Acceptance [P16]: a Pole-21-type module (≈20 dBW ERP in L1 from a
    /// 30 m mast) denies a ground receiver at 2 m to at least 25 km; the
    /// radius emerges from power, path loss and the receiver.
    #[test]
    fn pole21_module_denies_beyond_25_km() {
        let p = GnssParams::from_figures(&figs());
        let l1 = 1.575_42e9;
        let r = gnss_denial_radius(&p, 20.0, |d| two_ray(d, l1, 30.0, 2.0), 1e6).unwrap();
        assert!(r >= 25_000.0, "{r}");
        assert!(r < 200_000.0, "{r}");
        // A drone at 300 m altitude is denied much further away (free space).
        let air = gnss_denial_radius(&p, 20.0, |d| two_ray(d, l1, 30.0, 300.0), 1e7).unwrap();
        assert!(air > r);
        // A CRPA shrinks it a lot.
        let crpa = gnss_denial_radius(&p.clone().with_crpa(&figs()), 20.0, |d| two_ray(d, l1, 30.0, 2.0), 1e6).unwrap();
        assert!(crpa < r / 4.0);
    }
}
