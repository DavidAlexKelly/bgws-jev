//! Kalman filters for tracks (§9.1), in the track store's local ENU frame
//! (§5.2: flat maths only within a neighbourhood).
//!
//! Motion models:
//! - [`MotionModel::Static`]: near-static ground positions (a dug-in battery,
//!   a command post), position as a slow random walk;
//! - [`MotionModel::Cv2`]: constant velocity on the surface (land, sea),
//!   with altitude carried as a separate scalar random walk;
//! - [`MotionModel::Cv3`]: constant velocity in 3D (air);
//! - [`MotionModel::Ct`]: coordinated turn with unknown turn rate ω, an EKF
//!   (manoeuvring aircraft, drones in orbit).
//!
//! Process noise is continuous white-noise acceleration with a power
//! spectral density from the figures. Measurements are position fixes,
//! range-bearing (EKF) and bearing-only (EKF on an existing track; new DF
//! tracks are started by [`crate::df`] from crossing bearings). Updates use
//! the Joseph form so the covariance stays symmetric positive-definite.

use crate::stats::{symmetrize, wrap_pi};
use bgws_core::{Enu, SimTime};
use nalgebra::{DMatrix, DVector, Matrix3, Matrix6, Vector6};
use serde::{Deserialize, Serialize};

/// Which dynamics the filter assumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionModel {
    /// State [e, n, u]: a random walk.
    Static,
    /// State [e, n, ve, vn]; altitude as a scalar beside it.
    Cv2,
    /// State [e, n, u, ve, vn, vu].
    Cv3,
    /// State [e, n, ve, vn, ω]; altitude as a scalar beside it.
    Ct,
}

impl MotionModel {
    /// Length of the state vector.
    pub fn dim(self) -> usize {
        match self {
            MotionModel::Static => 3,
            MotionModel::Cv2 => 4,
            MotionModel::Cv3 => 6,
            MotionModel::Ct => 5,
        }
    }

    /// Index of `u` in the state, if altitude is part of it.
    pub fn u_index(self) -> Option<usize> {
        match self {
            MotionModel::Static | MotionModel::Cv3 => Some(2),
            _ => None,
        }
    }

    /// Indices of (ve, vn, vu) in the state, where present.
    pub fn vel_indices(self) -> (Option<usize>, Option<usize>, Option<usize>) {
        match self {
            MotionModel::Static => (None, None, None),
            MotionModel::Cv2 | MotionModel::Ct => (Some(2), Some(3), None),
            MotionModel::Cv3 => (Some(3), Some(4), Some(5)),
        }
    }
}

/// Process-noise densities for one filter.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessNoise {
    /// Acceleration PSD, m²/s³ (per axis).
    pub accel_psd: f64,
    /// Position random-walk PSD for static tracks, m²/s.
    pub static_psd: f64,
    /// Altitude random-walk PSD for 2D models, m²/s.
    pub alt_psd: f64,
    /// Turn-rate random-walk PSD for the CT model, rad²/s³.
    pub turn_psd: f64,
}

/// One measurement, in the store's ENU frame, with its covariance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measurement {
    /// A position fix (a laser range-finder lase, a radar plot converted, a report).
    Position { pos: Enu, cov: Matrix3<f64> },
    /// Ground range and bearing (radians clockwise from north) from `origin`.
    RangeBearing { origin: Enu, range: f64, bearing: f64, sigma_range: f64, sigma_bearing: f64 },
    /// A bearing only (direction finding, a passive sonar line).
    Bearing { origin: Enu, bearing: f64, sigma_bearing: f64 },
}

impl Measurement {
    /// Degrees of freedom of the innovation against a filter of `model`.
    pub fn dof(&self, model: MotionModel) -> usize {
        match self {
            Measurement::Position { .. } => {
                if model.u_index().is_some() {
                    3
                } else {
                    2
                }
            }
            Measurement::RangeBearing { .. } => 2,
            Measurement::Bearing { .. } => 1,
        }
    }

    /// A noisy position fix of `truth` with covariance `cov` (for the simulation).
    pub fn noisy_position(truth: Enu, cov: Matrix3<f64>, rng: &mut impl rand::Rng) -> Option<Measurement> {
        let d = DMatrix::from_iterator(3, 3, cov.iter().copied());
        let n = crate::stats::gaussian_vector(&d, rng)?;
        Some(Measurement::Position { pos: Enu::new(truth.e + n[0], truth.n + n[1], truth.u + n[2]), cov })
    }

    /// A noisy range-bearing of `truth` from `origin`.
    pub fn noisy_range_bearing(origin: Enu, truth: Enu, sigma_range: f64, sigma_bearing: f64, rng: &mut impl rand::Rng) -> Measurement {
        let (de, dn) = (truth.e - origin.e, truth.n - origin.n);
        let range = (de.hypot(dn) + sigma_range * crate::stats::std_normal(rng)).max(0.0);
        let bearing = wrap_pi(de.atan2(dn) + sigma_bearing * crate::stats::std_normal(rng));
        Measurement::RangeBearing { origin, range, bearing, sigma_range, sigma_bearing }
    }

    /// A noisy bearing of `truth` from `origin`.
    pub fn noisy_bearing(origin: Enu, truth: Enu, sigma_bearing: f64, rng: &mut impl rand::Rng) -> Measurement {
        let (de, dn) = (truth.e - origin.e, truth.n - origin.n);
        let bearing = wrap_pi(de.atan2(dn) + sigma_bearing * crate::stats::std_normal(rng));
        Measurement::Bearing { origin, bearing, sigma_bearing }
    }

    /// The position this measurement implies on its own, with covariance
    /// (linearised for range-bearing). `None` for a bearing.
    pub fn as_position(&self, default_u: f64, u_var: f64) -> Option<(Enu, Matrix3<f64>)> {
        match *self {
            Measurement::Position { pos, cov } => Some((pos, cov)),
            Measurement::RangeBearing { origin, range, bearing, sigma_range, sigma_bearing } => {
                let (s, c) = bearing.sin_cos();
                let pos = Enu::new(origin.e + range * s, origin.n + range * c, default_u);
                // J = d(e,n)/d(r,θ) = [[s, r c], [c, −r s]].
                let j = nalgebra::Matrix2::new(s, range * c, c, -range * s);
                let r = nalgebra::Matrix2::new(sigma_range * sigma_range, 0.0, 0.0, sigma_bearing * sigma_bearing);
                let p2 = j * r * j.transpose();
                let mut cov = Matrix3::zeros();
                cov.fixed_view_mut::<2, 2>(0, 0).copy_from(&p2);
                cov[(2, 2)] = u_var;
                Some((pos, cov))
            }
            Measurement::Bearing { .. } => None,
        }
    }
}

/// An innovation and its statistics (for gating and the consistency tests).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Innovation {
    pub y: DVector<f64>,
    pub s: DMatrix<f64>,
    /// Normalised innovation squared, yᵀS⁻¹y (Mahalanobis distance squared).
    pub nis: f64,
    pub dof: usize,
    /// ln det S, for likelihood-based costs.
    pub ln_det_s: f64,
}

/// A track's state in the common 6D form [e, n, u, ve, vn, vu] that reports,
/// knowledge and fusion use, whatever filter produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateEstimate {
    /// The time the estimate is valid for.
    pub time: SimTime,
    pub x: Vector6<f64>,
    pub p: Matrix6<f64>,
}

impl StateEstimate {
    pub fn position(&self) -> Enu {
        Enu::new(self.x[0], self.x[1], self.x[2])
    }
    pub fn velocity(&self) -> Enu {
        Enu::new(self.x[3], self.x[4], self.x[5])
    }
    /// The east/north covariance block.
    pub fn horizontal_cov(&self) -> nalgebra::Matrix2<f64> {
        self.p.fixed_view::<2, 2>(0, 0).into_owned()
    }
    /// The largest 1σ horizontal position error, metres.
    pub fn sigma_max(&self) -> f64 {
        crate::stats::Ellipse::from_cov(&self.horizontal_cov()).semi_major
    }

    /// Predict with constant velocity and white-noise acceleration `q`
    /// (m²/s³) to `to`. Estimates are never predicted backwards.
    pub fn predicted(&self, to: SimTime, q: f64) -> StateEstimate {
        let dt = (to - self.time).secs();
        if dt <= 0.0 {
            return self.clone();
        }
        let mut f = Matrix6::identity();
        let mut qm = Matrix6::zeros();
        for a in 0..3 {
            f[(a, a + 3)] = dt;
            qm[(a, a)] = q * dt.powi(3) / 3.0;
            qm[(a, a + 3)] = q * dt.powi(2) / 2.0;
            qm[(a + 3, a)] = q * dt.powi(2) / 2.0;
            qm[(a + 3, a + 3)] = q * dt;
        }
        let p = f * self.p * f.transpose() + qm;
        StateEstimate { time: to, x: f * self.x, p: 0.5 * (p + p.transpose()) }
    }
}

/// A Kalman filter for one track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kf {
    pub model: MotionModel,
    pub x: DVector<f64>,
    pub p: DMatrix<f64>,
    /// Altitude and its variance for models without `u` in the state.
    pub alt: f64,
    pub alt_var: f64,
    /// Variance given to the velocity components the model does not carry,
    /// when it is expressed as a [`StateEstimate`].
    pub fill_vel_var: f64,
    /// The time the state is valid for.
    pub time: SimTime,
}

/// What the filter needs to start.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KfInit {
    pub speed_sigma: f64,
    pub turn_sigma: f64,
    /// Velocity variance for components a model does not carry.
    pub fill_vel_var: f64,
}

struct Linearised {
    z: DVector<f64>,
    hx: DVector<f64>,
    h: DMatrix<f64>,
    r: DMatrix<f64>,
    angle_rows: Vec<usize>,
}

impl Kf {
    /// Start a filter at a position fix, at rest with velocity uncertainty
    /// `init.speed_sigma` per axis.
    pub fn new(model: MotionModel, time: SimTime, pos: Enu, pos_cov: Matrix3<f64>, init: KfInit) -> Kf {
        let n = model.dim();
        let mut x = DVector::zeros(n);
        let mut p = DMatrix::zeros(n, n);
        x[0] = pos.e;
        x[1] = pos.n;
        for i in 0..2 {
            for j in 0..2 {
                p[(i, j)] = pos_cov[(i, j)];
            }
        }
        if let Some(ui) = model.u_index() {
            x[ui] = pos.u;
            p[(ui, ui)] = pos_cov[(2, 2)];
            p[(0, ui)] = pos_cov[(0, 2)];
            p[(ui, 0)] = pos_cov[(2, 0)];
            p[(1, ui)] = pos_cov[(1, 2)];
            p[(ui, 1)] = pos_cov[(2, 1)];
        }
        let (ve, vn, vu) = model.vel_indices();
        for i in [ve, vn, vu].into_iter().flatten() {
            p[(i, i)] = init.speed_sigma * init.speed_sigma;
        }
        if model == MotionModel::Ct {
            p[(4, 4)] = init.turn_sigma * init.turn_sigma;
        }
        Kf { model, x, p, alt: pos.u, alt_var: pos_cov[(2, 2)], fill_vel_var: init.fill_vel_var, time }
    }

    /// The state transition over `dt` (and the CT Jacobian at the current state).
    fn transition(&self, dt: f64) -> (DVector<f64>, DMatrix<f64>) {
        let n = self.model.dim();
        match self.model {
            MotionModel::Static => (self.x.clone(), DMatrix::identity(n, n)),
            MotionModel::Cv2 | MotionModel::Cv3 => {
                let mut f = DMatrix::identity(n, n);
                let axes = n / 2;
                for a in 0..axes {
                    f[(a, a + axes)] = dt;
                }
                (&f * &self.x, f)
            }
            MotionModel::Ct => {
                let fx = ct_step(&self.x, dt);
                // Central-difference Jacobian: deterministic and exact enough.
                let mut f = DMatrix::zeros(n, n);
                for j in 0..n {
                    let h = 1e-6 * (1.0 + self.x[j].abs());
                    let mut xp = self.x.clone();
                    let mut xm = self.x.clone();
                    xp[j] += h;
                    xm[j] -= h;
                    let col = (ct_step(&xp, dt) - ct_step(&xm, dt)) / (2.0 * h);
                    f.set_column(j, &col);
                }
                (fx, f)
            }
        }
    }

    fn process_noise(&self, dt: f64, q: &ProcessNoise) -> DMatrix<f64> {
        let n = self.model.dim();
        let mut qm = DMatrix::zeros(n, n);
        let wna = |qm: &mut DMatrix<f64>, p: usize, v: usize| {
            let a = q.accel_psd;
            qm[(p, p)] += a * dt.powi(3) / 3.0;
            qm[(p, v)] += a * dt.powi(2) / 2.0;
            qm[(v, p)] += a * dt.powi(2) / 2.0;
            qm[(v, v)] += a * dt;
        };
        match self.model {
            MotionModel::Static => {
                for i in 0..3 {
                    qm[(i, i)] = q.static_psd * dt;
                }
            }
            MotionModel::Cv2 => {
                wna(&mut qm, 0, 2);
                wna(&mut qm, 1, 3);
            }
            MotionModel::Cv3 => {
                for a in 0..3 {
                    wna(&mut qm, a, a + 3);
                }
            }
            MotionModel::Ct => {
                wna(&mut qm, 0, 2);
                wna(&mut qm, 1, 3);
                qm[(4, 4)] = q.turn_psd * dt;
            }
        }
        qm
    }

    /// Predict to `to` (never backwards).
    pub fn predict_to(&mut self, to: SimTime, q: &ProcessNoise) {
        let dt = (to - self.time).secs();
        if dt <= 0.0 {
            return;
        }
        let (fx, f) = self.transition(dt);
        let qm = self.process_noise(dt, q);
        self.x = fx;
        self.p = &f * &self.p * f.transpose() + qm;
        symmetrize(&mut self.p);
        if self.model.u_index().is_none() {
            self.alt_var += q.alt_psd * dt;
        }
        self.time = to;
    }

    fn linearise(&self, m: &Measurement) -> Option<Linearised> {
        let n = self.model.dim();
        match *m {
            Measurement::Position { pos, cov } => {
                let rows: Vec<usize> = match self.model.u_index() {
                    Some(ui) => vec![0, 1, ui],
                    None => vec![0, 1],
                };
                let k = rows.len();
                let zf = [pos.e, pos.n, pos.u];
                let mut h = DMatrix::zeros(k, n);
                let mut r = DMatrix::zeros(k, k);
                for (a, &ri) in rows.iter().enumerate() {
                    h[(a, ri)] = 1.0;
                    for b in 0..k {
                        r[(a, b)] = cov[(a, b)];
                    }
                }
                let z = DVector::from_iterator(k, zf.iter().take(k).copied());
                let hx = &h * &self.x;
                Some(Linearised { z, hx, h, r, angle_rows: vec![] })
            }
            Measurement::RangeBearing { origin, range, bearing, sigma_range, sigma_bearing } => {
                let (de, dn) = (self.x[0] - origin.e, self.x[1] - origin.n);
                let r2 = de * de + dn * dn;
                if r2 < 1e-6 {
                    return None;
                }
                let rr = r2.sqrt();
                let mut h = DMatrix::zeros(2, n);
                h[(0, 0)] = de / rr;
                h[(0, 1)] = dn / rr;
                h[(1, 0)] = dn / r2;
                h[(1, 1)] = -de / r2;
                let z = DVector::from_vec(vec![range, bearing]);
                let hx = DVector::from_vec(vec![rr, de.atan2(dn)]);
                let r = DMatrix::from_diagonal(&DVector::from_vec(vec![sigma_range.powi(2), sigma_bearing.powi(2)]));
                Some(Linearised { z, hx, h, r, angle_rows: vec![1] })
            }
            Measurement::Bearing { origin, bearing, sigma_bearing } => {
                let (de, dn) = (self.x[0] - origin.e, self.x[1] - origin.n);
                let r2 = de * de + dn * dn;
                if r2 < 1e-6 {
                    return None;
                }
                let mut h = DMatrix::zeros(1, n);
                h[(0, 0)] = dn / r2;
                h[(0, 1)] = -de / r2;
                let z = DVector::from_vec(vec![bearing]);
                let hx = DVector::from_vec(vec![de.atan2(dn)]);
                let r = DMatrix::from_element(1, 1, sigma_bearing.powi(2));
                Some(Linearised { z, hx, h, r, angle_rows: vec![0] })
            }
        }
    }

    fn innovation_of(&self, l: &Linearised) -> Option<Innovation> {
        let mut y = &l.z - &l.hx;
        for &i in &l.angle_rows {
            y[i] = wrap_pi(y[i]);
        }
        let mut s = &l.h * &self.p * l.h.transpose() + &l.r;
        symmetrize(&mut s);
        let chol = s.clone().cholesky()?;
        let sol = chol.solve(&y);
        let nis = y.dot(&sol);
        let ln_det_s = 2.0 * chol.l().diagonal().iter().map(|d| d.ln()).sum::<f64>();
        Some(Innovation { dof: y.len(), y, s, nis, ln_det_s })
    }

    /// The innovation this measurement would produce, without updating
    /// (gating). `None` if it cannot be linearised (zero range).
    pub fn innovation(&self, m: &Measurement) -> Option<Innovation> {
        self.innovation_of(&self.linearise(m)?)
    }

    /// The Kalman (EKF) update, Joseph form. Returns the innovation used.
    pub fn update(&mut self, m: &Measurement) -> Option<Innovation> {
        let l = self.linearise(m)?;
        let inn = self.innovation_of(&l)?;
        let chol = inn.s.clone().cholesky()?;
        // K = P Hᵀ S⁻¹ = (S⁻¹ H P)ᵀ.
        let k = chol.solve(&(&l.h * &self.p)).transpose();
        self.x += &k * &inn.y;
        let n = self.model.dim();
        let ikh = DMatrix::identity(n, n) - &k * &l.h;
        self.p = &ikh * &self.p * ikh.transpose() + &k * &l.r * k.transpose();
        symmetrize(&mut self.p);
        if let (Measurement::Position { pos, cov }, None) = (m, self.model.u_index()) {
            // Altitude as a scalar filter beside the 2D state.
            let r = cov[(2, 2)];
            if r > 0.0 && r.is_finite() {
                let g = self.alt_var / (self.alt_var + r);
                self.alt += g * (pos.u - self.alt);
                self.alt_var *= 1.0 - g;
            }
        }
        Some(inn)
    }

    /// The estimate in the common 6D form.
    pub fn estimate(&self) -> StateEstimate {
        let mut idx: [Option<usize>; 6] = [Some(0), Some(1), self.model.u_index(), None, None, None];
        let (ve, vn, vu) = self.model.vel_indices();
        idx[3] = ve;
        idx[4] = vn;
        idx[5] = vu;
        let mut x = Vector6::zeros();
        let mut p = Matrix6::zeros();
        for a in 0..6 {
            match idx[a] {
                Some(i) => {
                    x[a] = self.x[i];
                    for b in 0..6 {
                        if let Some(j) = idx[b] {
                            p[(a, b)] = self.p[(i, j)];
                        }
                    }
                }
                None if a == 2 => {
                    x[a] = self.alt;
                    p[(a, a)] = self.alt_var;
                }
                None => p[(a, a)] = self.fill_vel_var,
            }
        }
        StateEstimate { time: self.time, x, p }
    }

    /// Position (east, north, up).
    pub fn position(&self) -> Enu {
        let u = self.model.u_index().map(|i| self.x[i]).unwrap_or(self.alt);
        Enu::new(self.x[0], self.x[1], u)
    }

    /// The turn rate, rad/s, for a CT filter.
    pub fn turn_rate(&self) -> Option<f64> {
        (self.model == MotionModel::Ct).then(|| self.x[4])
    }
}

/// One coordinated-turn step of [e, n, ve, vn, ω]. ω > 0 turns
/// anticlockwise in the (east, north) plane, i.e. to the left.
fn ct_step(x: &DVector<f64>, dt: f64) -> DVector<f64> {
    let (e, n, ve, vn, w) = (x[0], x[1], x[2], x[3], x[4]);
    let mut out = x.clone();
    if w.abs() < 1e-9 {
        out[0] = e + ve * dt;
        out[1] = n + vn * dt;
        return out;
    }
    let (s, c) = (w * dt).sin_cos();
    out[0] = e + (ve * s - vn * (1.0 - c)) / w;
    out[1] = n + (ve * (1.0 - c) + vn * s) / w;
    out[2] = ve * c - vn * s;
    out[3] = ve * s + vn * c;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{chi2_quantile, gaussian_vector, is_spd, std_normal};
    use approx::assert_relative_eq;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    const Q: ProcessNoise = ProcessNoise { accel_psd: 0.5, static_psd: 0.01, alt_psd: 1.0, turn_psd: 1e-6 };

    fn init(speed: f64) -> KfInit {
        KfInit { speed_sigma: speed, turn_sigma: 0.05, fill_vel_var: 1.0 }
    }

    /// The normalised estimation error squared over the first `dims` state components.
    fn nees(kf: &Kf, truth: &[f64]) -> f64 {
        let n = truth.len();
        let e = DVector::from_iterator(n, (0..n).map(|i| kf.x[i] - truth[i]));
        let p = kf.p.view((0, 0), (n, n)).into_owned();
        e.dot(&p.cholesky().unwrap().solve(&e))
    }

    /// Two-sided 99% bounds on the average of `runs` chi-squared(dof) draws.
    fn bounds(runs: usize, dof: usize) -> (f64, f64) {
        let k = (runs * dof) as f64;
        (chi2_quantile(0.005, k) / runs as f64, chi2_quantile(0.995, k) / runs as f64)
    }

    #[test]
    fn cv2_nees_and_nis_are_consistent() {
        // Monte Carlo: truth follows exactly the filter's model, so the average
        // NEES must sit inside the chi-squared bounds at every checked step.
        let runs = 60;
        let steps = 25;
        let dt = 2.0;
        let sigma = 10.0;
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let mut nees_sum = vec![0.0; steps];
        let mut nis_sum = vec![0.0; steps];
        let rcov = Matrix3::from_diagonal_element(sigma * sigma);
        let speed = 5.0;
        for _ in 0..runs {
            // Truth drawn from the filter's initial distribution.
            let mut t = [0.0, 0.0, speed * std_normal(&mut rng), speed * std_normal(&mut rng)];
            let m0 = Measurement::noisy_position(Enu::new(t[0], t[1], 0.0), rcov, &mut rng).unwrap();
            let Measurement::Position { pos, .. } = m0 else { unreachable!() };
            let mut kf = Kf::new(MotionModel::Cv2, SimTime::ZERO, pos, rcov, init(speed));
            for k in 0..steps {
                // Propagate the truth with the discrete white-noise-acceleration model.
                let qd = {
                    let a = Q.accel_psd;
                    DMatrix::from_row_slice(2, 2, &[a * dt.powi(3) / 3.0, a * dt * dt / 2.0, a * dt * dt / 2.0, a * dt])
                };
                for axis in 0..2 {
                    let w = gaussian_vector(&qd, &mut rng).unwrap();
                    t[axis] += t[axis + 2] * dt + w[0];
                    t[axis + 2] += w[1];
                }
                let time = SimTime::from_secs(dt * (k + 1) as f64);
                kf.predict_to(time, &Q);
                let m = Measurement::noisy_position(Enu::new(t[0], t[1], 0.0), rcov, &mut rng).unwrap();
                let inn = kf.update(&m).unwrap();
                nis_sum[k] += inn.nis;
                nees_sum[k] += nees(&kf, &t);
                assert!(is_spd(&kf.p));
            }
        }
        let (lo4, hi4) = bounds(runs, 4);
        let (lo2, hi2) = bounds(runs, 2);
        let mut out_nees = 0;
        let mut out_nis = 0;
        for k in 0..steps {
            let (a, b) = (nees_sum[k] / runs as f64, nis_sum[k] / runs as f64);
            if !(lo4..=hi4).contains(&a) {
                out_nees += 1;
            }
            if !(lo2..=hi2).contains(&b) {
                out_nis += 1;
            }
        }
        // At 99% two-sided, about 1 in 100 steps may fall out by chance.
        assert!(out_nees <= 2, "NEES out of bounds at {out_nees} of {steps} steps");
        assert!(out_nis <= 2, "NIS out of bounds at {out_nis} of {steps} steps");
        let total: f64 = nees_sum.iter().sum::<f64>() / (runs * steps) as f64;
        assert!((3.0..5.0).contains(&total), "time-averaged NEES {total}");
    }

    #[test]
    fn cv3_and_static_are_consistent() {
        let runs = 50;
        let mut rng = ChaCha8Rng::seed_from_u64(2);
        let rcov = Matrix3::from_diagonal(&nalgebra::Vector3::new(400.0, 400.0, 900.0));
        let (mut sum3, mut sum_s) = (0.0, 0.0);
        let q = ProcessNoise { accel_psd: 0.0, ..Q };
        for _ in 0..runs {
            // A non-manoeuvring aircraft: zero process noise in truth and filter.
            let v = [150.0 * std_normal(&mut rng), 150.0 * std_normal(&mut rng), 10.0 * std_normal(&mut rng)];
            let p0 = Enu::new(0.0, 0.0, 3000.0);
            let m = Measurement::noisy_position(p0, rcov, &mut rng).unwrap();
            let Measurement::Position { pos, .. } = m else { unreachable!() };
            let mut kf = Kf::new(MotionModel::Cv3, SimTime::ZERO, pos, rcov, init(150.0));
            kf.p[(5, 5)] = 100.0;
            let mut t = [p0.e, p0.n, p0.u, v[0], v[1], v[2]];
            for k in 1..=10 {
                for a in 0..3 {
                    t[a] += t[a + 3] * 5.0;
                }
                kf.predict_to(SimTime::from_secs(5.0 * k as f64), &q);
                kf.update(&Measurement::noisy_position(Enu::new(t[0], t[1], t[2]), rcov, &mut rng).unwrap());
            }
            sum3 += nees(&kf, &t);
            // A static position fixed several times.
            let truth = Enu::new(1000.0, -500.0, 50.0);
            let m = Measurement::noisy_position(truth, rcov, &mut rng).unwrap();
            let Measurement::Position { pos, .. } = m else { unreachable!() };
            let mut ks = Kf::new(MotionModel::Static, SimTime::ZERO, pos, rcov, init(0.0));
            for k in 1..=5 {
                ks.predict_to(SimTime::from_secs(k as f64 * 60.0), &ProcessNoise { static_psd: 0.0, ..Q });
                ks.update(&Measurement::noisy_position(truth, rcov, &mut rng).unwrap());
            }
            sum_s += nees(&ks, &[truth.e, truth.n, truth.u]);
        }
        let (lo, hi) = bounds(runs, 6);
        assert!((lo..=hi).contains(&(sum3 / runs as f64)), "CV3 NEES {}", sum3 / runs as f64);
        let (lo, hi) = bounds(runs, 3);
        assert!((lo..=hi).contains(&(sum_s / runs as f64)), "static NEES {}", sum_s / runs as f64);
    }

    #[test]
    fn range_bearing_ekf_is_consistent() {
        let runs = 60;
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let origin = Enu::new(0.0, 0.0, 0.0);
        let (sr, sb) = (15.0, 0.005);
        let q = ProcessNoise { accel_psd: 0.0, ..Q };
        let mut sum = 0.0;
        for _ in 0..runs {
            let mut t = [3000.0, 4000.0, 8.0 * std_normal(&mut rng), 8.0 * std_normal(&mut rng)];
            let m = Measurement::noisy_range_bearing(origin, Enu::new(t[0], t[1], 0.0), sr, sb, &mut rng);
            let (pos, cov) = m.as_position(0.0, 100.0).unwrap();
            let mut kf = Kf::new(MotionModel::Cv2, SimTime::ZERO, pos, cov, init(8.0));
            for k in 1..=15 {
                t[0] += t[2] * 3.0;
                t[1] += t[3] * 3.0;
                kf.predict_to(SimTime::from_secs(3.0 * k as f64), &q);
                let m = Measurement::noisy_range_bearing(origin, Enu::new(t[0], t[1], 0.0), sr, sb, &mut rng);
                kf.update(&m).unwrap();
            }
            sum += nees(&kf, &t);
        }
        let (lo, hi) = bounds(runs, 4);
        let avg = sum / runs as f64;
        // The EKF is only approximately consistent; allow a little margin.
        assert!(avg > lo * 0.8 && avg < hi * 1.2, "range-bearing NEES {avg} vs [{lo}, {hi}]");
    }

    #[test]
    fn ct_ekf_tracks_a_turn_and_learns_omega() {
        let mut rng = ChaCha8Rng::seed_from_u64(4);
        let omega = 0.03; // rad/s: a 3.5-minute circle
        let mut truth = DVector::from_vec(vec![0.0, 0.0, 200.0, 0.0, omega]);
        let rcov = Matrix3::from_diagonal_element(50.0 * 50.0);
        let q = ProcessNoise { accel_psd: 0.1, turn_psd: 1e-6, ..Q };
        let m0 = Measurement::noisy_position(Enu::new(0.0, 0.0, 1000.0), rcov, &mut rng).unwrap();
        let Measurement::Position { pos, .. } = m0 else { unreachable!() };
        let mut kf = Kf::new(MotionModel::Ct, SimTime::ZERO, pos, rcov, KfInit { speed_sigma: 200.0, turn_sigma: 0.05, fill_vel_var: 1.0 });
        let mut cv = Kf::new(MotionModel::Cv2, SimTime::ZERO, pos, rcov, init(200.0));
        let (mut err_ct, mut err_cv) = (0.0, 0.0);
        for k in 1..=60 {
            truth = ct_step(&truth, 2.0);
            let time = SimTime::from_secs(2.0 * k as f64);
            kf.predict_to(time, &q);
            cv.predict_to(time, &ProcessNoise { accel_psd: 0.1, ..Q });
            let m = Measurement::noisy_position(Enu::new(truth[0], truth[1], 1000.0), rcov, &mut rng).unwrap();
            kf.update(&m);
            cv.update(&m);
            assert!(is_spd(&kf.p));
            if k > 30 {
                err_ct += (kf.x[0] - truth[0]).hypot(kf.x[1] - truth[1]);
                err_cv += (cv.x[0] - truth[0]).hypot(cv.x[1] - truth[1]);
            }
        }
        assert_relative_eq!(kf.turn_rate().unwrap(), omega, epsilon = 0.005);
        assert!(err_ct < err_cv, "CT {err_ct} should beat CV {err_cv} in a sustained turn");
        assert_relative_eq!(kf.position().u, 1000.0, epsilon = 30.0);
    }

    #[test]
    fn bearing_update_narrows_cross_range_only() {
        let rcov = Matrix3::from_diagonal_element(1.0e6);
        let mut kf = Kf::new(MotionModel::Static, SimTime::ZERO, Enu::new(0.0, 10_000.0, 0.0), rcov, init(0.0));
        let before = kf.p.clone();
        let inn = kf.update(&Measurement::Bearing { origin: Enu::default(), bearing: 0.0, sigma_bearing: 0.01 }).unwrap();
        assert_eq!(inn.dof, 1);
        // Bearing from the origin due north: east (cross-range) shrinks, north (range) does not.
        assert!(kf.p[(0, 0)] < 0.05 * before[(0, 0)]);
        assert_relative_eq!(kf.p[(1, 1)], before[(1, 1)], max_relative = 1e-6);
    }

    #[test]
    fn estimate_and_prediction() {
        let rcov = Matrix3::from_diagonal_element(25.0);
        let mut kf = Kf::new(MotionModel::Cv2, SimTime::ZERO, Enu::new(1.0, 2.0, 3.0), rcov, init(5.0));
        kf.x[2] = 10.0;
        let e = kf.estimate();
        assert_eq!(e.velocity().e, 10.0);
        assert_eq!(e.p[(5, 5)], 1.0);
        let p = e.predicted(SimTime::from_secs(10.0), 0.5);
        assert_relative_eq!(p.position().e, 101.0);
        assert!(p.sigma_max() > e.sigma_max());
        assert!(is_spd(&DMatrix::from_iterator(6, 6, p.p.iter().copied())));
    }
}
