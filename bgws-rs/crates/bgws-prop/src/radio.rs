//! VHF/UHF radio path loss (§8.4, §7.5).
//!
//! - **Free space:** `L = 20·log10(4πd/λ)`.
//! - **Knife-edge diffraction:** the Fresnel–Kirchhoff parameter
//!   `ν = h·√(2/λ·(1/d1 + 1/d2))` and the ITU-R P.526 approximation
//!   `J(ν) = 6.9 + 20·log10(√((ν−0.1)²+1) + ν − 0.1)` for ν > −0.78.
//! - **Multiple edges:** Deygout's method (P.526 §4.5.1): the edge with the
//!   largest ν, then the dominant edge on each side, to a set depth, over
//!   the profile raised by the 4/3-Earth bulge. Like every knife-edge
//!   method it overstates loss for low antennas over smooth ground (where
//!   the ground itself fills the Fresnel zone); the ITM-style model below
//!   covers that regime with its two-ray term.
//! - **Clutter** at a terminal sited below the canopy or roofline, by land
//!   cover, scaled with frequency **[default]**.
//! - **ITM-style irregular terrain:** a simplified Longley–Rice: effective
//!   horizons shrink with terrain irregularity Δh; inside the horizon a
//!   two-ray (plane-Earth) model with Rayleigh roughness reduction; beyond
//!   it a blend of smooth-sphere diffraction (P.526 §3) and a double
//!   knife-edge, weighted by Δh; a linear blend near the horizon keeps it
//!   continuous **[default: simplified implementation]**. Its tests check
//!   the published qualitative behaviour of ITM curves (excess loss rises
//!   with distance and frequency beyond the horizon, higher antennas lose
//!   less, rough terrain pulls the horizon in), not exact ITM numbers.

use bgws_core::geo::EARTH_RADIUS;
use bgws_core::units::SPEED_OF_LIGHT;
use bgws_core::Figures;
pub use bgws_gpu::field::{fspl_db, knife_edge_j};
use bgws_env::{LandCover, Terrain};
use serde::{Deserialize, Serialize};

/// Fresnel–Kirchhoff diffraction parameter for an obstacle `h` metres above
/// the line (negative below) at `d1`, `d2` from the ends.
pub fn fresnel_v(h: f64, d1: f64, d2: f64, lambda: f64) -> f64 {
    h * (2.0 / lambda * (1.0 / d1.max(1e-6) + 1.0 / d2.max(1e-6))).sqrt()
}

/// Radius of the first Fresnel zone at a point.
pub fn fresnel_radius(d1: f64, d2: f64, lambda: f64) -> f64 {
    (lambda * d1 * d2 / (d1 + d2).max(1e-9)).sqrt()
}

/// A terrain profile: distances from the start and ground heights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub d: Vec<f64>,
    pub h: Vec<f64>,
}

impl Profile {
    /// Sample terrain between two points every `step` metres (endpoints included).
    pub fn from_terrain(t: &Terrain, a: (f64, f64), b: (f64, f64), step: f64) -> Self {
        let dist = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (dist / step.max(1.0)).ceil().max(1.0) as usize;
        let (d, h) = (0..=n)
            .map(|s| {
                let f = s as f64 / n as f64;
                (f * dist, t.height(a.0 + f * (b.0 - a.0), a.1 + f * (b.1 - a.1)))
            })
            .unzip();
        Self { d, h }
    }

    pub fn length(&self) -> f64 {
        self.d.last().copied().unwrap_or(0.0)
    }

    /// Terrain irregularity Δh: the interdecile range of heights about a
    /// least-squares line over the middle 80% of the path (ITM's definition).
    pub fn delta_h(&self) -> f64 {
        let len = self.length();
        let pts: Vec<(f64, f64)> = self.d.iter().zip(&self.h).filter(|(d, _)| **d >= 0.1 * len && **d <= 0.9 * len).map(|(d, h)| (*d, *h)).collect();
        if pts.len() < 3 {
            return 0.0;
        }
        let n = pts.len() as f64;
        let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
        let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
        let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        let mut res: Vec<f64> = pts.iter().map(|p| p.1 - (my + slope * (p.0 - mx))).collect();
        res.sort_by(f64::total_cmp);
        let q = |f: f64| res[((res.len() - 1) as f64 * f).round() as usize];
        q(0.9) - q(0.1)
    }
}

/// A diffracting edge found by Deygout.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnifeEdge {
    pub distance_m: f64,
    /// Height of the (bulge-raised) edge above the local line, metres.
    pub excess_m: f64,
    pub v: f64,
    pub loss_db: f64,
}

/// Deygout multiple knife-edge loss over a profile. `z_tx`/`z_rx` are the
/// antenna heights above the datum; the profile is raised by the
/// effective-Earth bulge with factor `k`; recursion stops at `depth`.
pub fn deygout(p: &Profile, z_tx: f64, z_rx: f64, lambda: f64, k: f64, depth: u32) -> (f64, Vec<KnifeEdge>) {
    let n = p.d.len();
    if n < 3 {
        return (0.0, Vec::new());
    }
    let total = p.length();
    let inv = 1.0 / (2.0 * k * EARTH_RADIUS);
    let mut z: Vec<f64> = (0..n).map(|i| p.h[i] + p.d[i] * (total - p.d[i]) * inv).collect();
    z[0] = z_tx;
    z[n - 1] = z_rx;
    let mut edges = Vec::new();
    let loss = deygout_rec(&p.d, &z, 0, n - 1, lambda, depth, &mut edges);
    edges.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    (loss, edges)
}

fn deygout_rec(d: &[f64], z: &[f64], a: usize, b: usize, lambda: f64, depth: u32, out: &mut Vec<KnifeEdge>) -> f64 {
    if depth == 0 || b <= a + 1 {
        return 0.0;
    }
    let span = d[b] - d[a];
    let mut best: Option<(f64, usize, f64)> = None;
    for i in a + 1..b {
        let t = (d[i] - d[a]) / span;
        let line = z[a] + t * (z[b] - z[a]);
        let h = z[i] - line;
        let v = fresnel_v(h, d[i] - d[a], d[b] - d[i], lambda);
        if best.is_none_or(|x| v > x.0) {
            best = Some((v, i, h));
        }
    }
    let Some((v, i, h)) = best else { return 0.0 };
    if v <= -0.78 {
        return 0.0;
    }
    let j = knife_edge_j(v);
    out.push(KnifeEdge { distance_m: d[i], excess_m: h, v, loss_db: j });
    j + deygout_rec(d, z, a, i, lambda, depth - 1, out) + deygout_rec(d, z, i, b, lambda, depth - 1, out)
}

/// Radio propagation settings, from figures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioParams {
    pub k_factor: f64,
    pub profile_step_m: f64,
    pub deygout_depth: u32,
    /// Clutter loss at 1 GHz for a terminal fully inside each class, dB.
    pub clutter_db_1ghz: [f64; 10],
    pub clutter_freq_exp: f64,
    pub canopy_m: [f64; 10],
    pub itm: ItmParams,
}

impl RadioParams {
    pub fn from_figures(f: &Figures) -> Self {
        let table = bgws_env::LandCoverTable::from_figures(f);
        Self {
            k_factor: f.get("prop.radio.k_factor"),
            profile_step_m: f.get("prop.radio.profile_step_m"),
            deygout_depth: f.get("prop.radio.deygout_depth") as u32,
            clutter_db_1ghz: LandCover::ALL.map(|c| f.get(&format!("prop.clutter.{}_db", c.key()))),
            clutter_freq_exp: f.get("prop.clutter.freq_exp"),
            canopy_m: LandCover::ALL.map(|c| table.get(c).canopy_m),
            itm: ItmParams::from_figures(f),
        }
    }
}

/// Clutter loss for a terminal at `agl` in land cover `lc`, dB: full value
/// at ground level, falling linearly to zero at the canopy/roof height.
pub fn clutter_loss_db(lc: LandCover, agl: f64, f_hz: f64, p: &RadioParams) -> f64 {
    let canopy = p.canopy_m[lc as usize];
    if canopy <= 0.0 || agl >= canopy {
        return 0.0;
    }
    p.clutter_db_1ghz[lc as usize] * (f_hz / 1e9).max(0.01).powf(p.clutter_freq_exp) * (1.0 - agl.max(0.0) / canopy)
}

/// A radio terminal: ground position and antenna height above ground.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terminal {
    pub e: f64,
    pub n: f64,
    pub agl: f64,
}

/// Basic transmission loss over terrain with its parts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathLoss {
    pub distance_m: f64,
    pub fspl_db: f64,
    pub diffraction_db: f64,
    pub edges: Vec<KnifeEdge>,
    pub clutter_tx_db: f64,
    pub clutter_rx_db: f64,
    /// No edge reaches ν = 0 (first Fresnel zone at least half clear).
    pub line_of_sight: bool,
    pub total_db: f64,
}

/// Point-to-point loss over the terrain: FSPL + Deygout diffraction + clutter.
pub fn terrain_path_loss(t: &Terrain, tx: Terminal, rx: Terminal, f_hz: f64, p: &RadioParams) -> PathLoss {
    let prof = Profile::from_terrain(t, (tx.e, tx.n), (rx.e, rx.n), p.profile_step_m.min(t.spec.cell));
    let lambda = SPEED_OF_LIGHT / f_hz;
    let z_tx = prof.h[0] + tx.agl;
    let z_rx = prof.h[prof.h.len() - 1] + rx.agl;
    let (diff, edges) = deygout(&prof, z_tx, z_rx, lambda, p.k_factor, p.deygout_depth);
    let d = prof.length();
    let slant = (d * d + (z_rx - z_tx).powi(2)).sqrt();
    let fspl = fspl_db(slant, f_hz);
    let ct = clutter_loss_db(t.land_cover_at(tx.e, tx.n), tx.agl, f_hz, p);
    let cr = clutter_loss_db(t.land_cover_at(rx.e, rx.n), rx.agl, f_hz, p);
    PathLoss {
        distance_m: d,
        fspl_db: fspl,
        diffraction_db: diff,
        line_of_sight: edges.iter().all(|e| e.v < 0.0),
        edges,
        clutter_tx_db: ct,
        clutter_rx_db: cr,
        total_db: fspl + diff + ct + cr,
    }
}

/// ITM-style model settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItmParams {
    /// Δh at which the knife-edge and smooth-sphere weights are equal, m.
    pub knife_weight_dh_m: f64,
    /// Fraction of the horizon distance where the LOS→diffraction blend starts.
    pub blend_start: f64,
    /// σ_h = c·Δh for the Rayleigh roughness factor.
    pub sigma_per_dh: f64,
    /// Coefficient of ITM's horizon reduction exp(−c·√(Δh/h_e)).
    pub horizon_dh_coeff: f64,
}

impl ItmParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            knife_weight_dh_m: f.get("prop.itm.knife_weight_dh_m"),
            blend_start: f.get("prop.itm.blend_start"),
            sigma_per_dh: f.get("prop.itm.sigma_per_dh"),
            horizon_dh_coeff: f.get("prop.itm.horizon_dh_coeff"),
        }
    }
}

/// Inputs of the area-mode ITM-style model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItmInput {
    pub distance_m: f64,
    pub f_hz: f64,
    pub h_tx_m: f64,
    pub h_rx_m: f64,
    /// Terrain irregularity Δh, metres.
    pub delta_h_m: f64,
    pub k_factor: f64,
}

/// The ITM-style answer with its parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItmResult {
    pub fspl_db: f64,
    /// Smooth-Earth horizon distances reduced by Δh, metres.
    pub horizon_tx_m: f64,
    pub horizon_rx_m: f64,
    pub line_of_sight: bool,
    pub two_ray_db: f64,
    pub sphere_db: f64,
    pub knife_db: f64,
    /// Weight on the smooth-sphere term.
    pub sphere_weight: f64,
    /// Excess loss over free space (≥ −6 dB).
    pub excess_db: f64,
    pub total_db: f64,
}

/// Smooth-sphere diffraction loss (ITU-R P.526 §3.1.1, horizontal
/// polarisation over land, β = 1), dB, for distance and heights in metres.
pub fn smooth_sphere_loss_db(d_m: f64, h1: f64, h2: f64, f_hz: f64, k: f64) -> f64 {
    let f_mhz = f_hz / 1e6;
    let ae_km = k * EARTH_RADIUS / 1000.0;
    let x = 2.188 * f_mhz.cbrt() * ae_km.powf(-2.0 / 3.0) * d_m / 1000.0;
    let y = |h: f64| 9.575e-3 * f_mhz.powf(2.0 / 3.0) * ae_km.powf(-1.0 / 3.0) * h.max(0.0);
    let fx = if x >= 1.6 { 11.0 + 10.0 * x.log10() - 17.6 * x } else { -20.0 * x.max(1e-9).log10() - 5.6488 * x.powf(1.425) };
    // Height-gain G(B), B = βY; for B ≤ 2 the 20·log(B + 0.1B³) branch,
    // floored where the surface-impedance term K would take over (K ≪ 1 above 20 MHz).
    let g = |b: f64| {
        if b > 2.0 {
            17.6 * (b - 1.1).sqrt() - 5.0 * (b - 1.1).log10() - 8.0
        } else {
            20.0 * (b + 0.1 * b.powi(3)).max(0.01).log10()
        }
    };
    let g1 = g(y(h1));
    let g2 = g(y(h2));
    -(fx + g1 + g2)
}

/// The simplified ITM-style median basic transmission loss.
pub fn itm_loss(i: &ItmInput, p: &ItmParams) -> ItmResult {
    let lambda = SPEED_OF_LIGHT / i.f_hz;
    let d = i.distance_m.max(1.0);
    let ae = i.k_factor * EARTH_RADIUS;
    let fspl = fspl_db((d * d + (i.h_tx_m - i.h_rx_m).powi(2)).sqrt(), i.f_hz);
    let dh = i.delta_h_m.max(0.0);
    let horizon = |h: f64| (2.0 * ae * h.max(0.5)).sqrt() * (-p.horizon_dh_coeff * (dh / h.max(5.0)).sqrt()).exp();
    let (dl1, dl2) = (horizon(i.h_tx_m), horizon(i.h_rx_m));
    let dl = dl1 + dl2;
    let sigma_h = p.sigma_per_dh * dh;
    let w_sphere = 1.0 / (1.0 + dh / p.knife_weight_dh_m);

    // Two-ray with roughness; median (no nulls) inside the first breakpoint.
    let two_ray = |d: f64| -> f64 {
        let (h1, h2) = (i.h_tx_m.max(0.5), i.h_rx_m.max(0.5));
        let breakpoint = 4.0 * h1 * h2 / lambda;
        if d <= breakpoint {
            return 0.0;
        }
        let sin_psi = (h1 + h2) / d;
        let rho = (-0.5 * (4.0 * std::f64::consts::PI * sigma_h * sin_psi / lambda).powi(2)).exp();
        let phase = 4.0 * std::f64::consts::PI * h1 * h2 / (lambda * d);
        let re = 1.0 - rho * phase.cos();
        let im = rho * phase.sin();
        -20.0 * (re.hypot(im)).max(1e-6).log10()
    };
    let diffraction = |d: f64| -> (f64, f64) {
        let sphere = smooth_sphere_loss_db(d, i.h_tx_m, i.h_rx_m, i.f_hz, i.k_factor).max(0.0);
        let theta = (d - dl).max(0.0) / ae;
        let (d1, d2) = (dl1 + (d - dl).max(0.0) / 2.0, dl2 + (d - dl).max(0.0) / 2.0);
        let v = theta * (2.0 / lambda * d1 * d2 / (d1 + d2)).sqrt();
        (sphere, knife_edge_j(v))
    };
    let (sphere, knife, excess, los) = if d >= dl {
        let (s, k) = diffraction(d);
        (s, k, w_sphere * s + (1.0 - w_sphere) * k, false)
    } else {
        let a_los = two_ray(d);
        let (s, k) = diffraction(dl);
        let a_hor = w_sphere * s + (1.0 - w_sphere) * k;
        let start = p.blend_start * dl;
        let t = ((d - start) / (dl - start).max(1.0)).clamp(0.0, 1.0);
        (s, k, a_los * (1.0 - t) + a_hor.max(a_los) * t, true)
    };
    let excess = excess.max(-6.0);
    ItmResult {
        fspl_db: fspl,
        horizon_tx_m: dl1,
        horizon_rx_m: dl2,
        line_of_sight: los,
        two_ray_db: two_ray(d),
        sphere_db: sphere,
        knife_db: knife,
        sphere_weight: w_sphere,
        excess_db: excess,
        total_db: fspl + excess,
    }
}

/// ITM-style loss over an actual terrain path (Δh from the profile).
pub fn itm_over_terrain(t: &Terrain, tx: Terminal, rx: Terminal, f_hz: f64, p: &RadioParams) -> ItmResult {
    let prof = Profile::from_terrain(t, (tx.e, tx.n), (rx.e, rx.n), p.profile_step_m.max(t.spec.cell));
    itm_loss(
        &ItmInput { distance_m: prof.length(), f_hz, h_tx_m: tx.agl, h_rx_m: rx.agl, delta_h_m: prof.delta_h(), k_factor: p.k_factor },
        &p.itm,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use bgws_core::Geodetic;
    use bgws_env::{GridSpec, Raster};

    fn rp() -> RadioParams {
        RadioParams::from_figures(&crate::default_figures())
    }

    #[test]
    fn fspl_known_values() {
        // 1 km at 1 GHz: 92.45 dB; 10 km at 100 MHz: 92.45 dB too; 36 000 km at 12 GHz ≈ 205.2 dB.
        assert_relative_eq!(fspl_db(1_000.0, 1e9), 92.45, epsilon = 0.01);
        assert_relative_eq!(fspl_db(10_000.0, 1e8), 92.45, epsilon = 0.01);
        assert_relative_eq!(fspl_db(36_000e3, 12e9), 205.15, epsilon = 0.05);
    }

    #[test]
    fn j_of_v_matches_p526() {
        // P.526 Fig. 9 / the approximation: J(−0.78)=0, J(0)≈6.0, J(1)≈13.9, J(2)≈19.0 (exact Fresnel 13.9/19.0), J(3)≈22.0.
        assert_relative_eq!(knife_edge_j(-0.78), 0.0);
        assert_relative_eq!(knife_edge_j(0.0), 6.03, epsilon = 0.05);
        assert_relative_eq!(knife_edge_j(1.0), 13.96, epsilon = 0.1);
        assert_relative_eq!(knife_edge_j(2.0), 19.04, epsilon = 0.1);
        assert_relative_eq!(knife_edge_j(3.0), 22.35, epsilon = 0.15);
        // Large ν: J → 13 + 20 log10 ν (the approximation sits ~0.15 dB under it at ν = 10).
        assert_relative_eq!(knife_edge_j(10.0), 13.0 + 20.0, epsilon = 0.3);
        // ν for a 10 m edge midway on a 10 km path at 300 MHz: 10·√(2·(2/5000)) = 0.283.
        assert_relative_eq!(fresnel_v(10.0, 5_000.0, 5_000.0, 1.0), 0.2828, epsilon = 1e-4);
    }

    #[test]
    fn deygout_single_and_double_edges() {
        // 3 GHz, 40 m masts 10 km apart: flat ground clears the Fresnel zone.
        let lambda = 0.1;
        let mut p = Profile { d: (0..=100).map(|i| i as f64 * 100.0).collect(), h: vec![0.0; 101] };
        assert_eq!(deygout(&p, 40.0, 40.0, lambda, 1e9, 3).0, 0.0);
        p.h[50] = 60.0;
        let (one, e1) = deygout(&p, 40.0, 40.0, lambda, 1e9, 3);
        assert_eq!(e1.len(), 1);
        assert_relative_eq!(one, knife_edge_j(fresnel_v(20.0, 5000.0, 5000.0, lambda)), epsilon = 1e-9);
        p.h[25] = 55.0;
        let (two, e2) = deygout(&p, 40.0, 40.0, lambda, 1e9, 3);
        assert!(e2.len() >= 2 && two > one, "{two} vs {one}");
    }

    #[test]
    fn terrain_path_and_clutter() {
        let p = rp();
        let spec = GridSpec::new(201, 3, 50.0, 0.0, 0.0);
        let mut t = bgws_env::Terrain::flat(Geodetic::default(), spec, 100.0);
        let open = terrain_path_loss(&t, Terminal { e: 0.0, n: 50.0, agl: 40.0 }, Terminal { e: 5_000.0, n: 50.0, agl: 40.0 }, 3e9, &p);
        assert!(open.line_of_sight && (open.total_db - open.fspl_db).abs() < 0.5);
        t.elevation = Raster::from_fn(spec, |i, _| if i == 50 { 180.0 } else { 100.0 });
        let hill = terrain_path_loss(&t, Terminal { e: 0.0, n: 50.0, agl: 40.0 }, Terminal { e: 5_000.0, n: 50.0, agl: 40.0 }, 3e9, &p);
        assert!(!hill.line_of_sight && hill.diffraction_db > 20.0, "{hill:?}");
        t.land_cover = Raster::filled(spec, LandCover::ForestDense);
        let wood = terrain_path_loss(&t, Terminal { e: 0.0, n: 50.0, agl: 2.0 }, Terminal { e: 2_000.0, n: 50.0, agl: 30.0 }, 3e9, &p);
        assert!(wood.clutter_tx_db > 5.0 && wood.clutter_rx_db == 0.0);
    }

    #[test]
    fn itm_qualitative_behaviour() {
        let p = rp().itm;
        let base = ItmInput { distance_m: 2_000.0, f_hz: 100e6, h_tx_m: 10.0, h_rx_m: 10.0, delta_h_m: 90.0, k_factor: 4.0 / 3.0 };
        // Short range: close to free space.
        let near = itm_loss(&base, &p);
        assert!(near.line_of_sight && near.excess_db.abs() < 6.5, "{near:?}");
        // Beyond the horizon excess loss rises with distance…
        let mut last = f64::NEG_INFINITY;
        for km in [30.0, 50.0, 80.0, 120.0, 200.0] {
            let r = itm_loss(&ItmInput { distance_m: km * 1000.0, ..base }, &p);
            assert!(r.excess_db > last, "{km} km: {r:?}");
            last = r.excess_db;
        }
        // …a plausible magnitude at 50 km, 100 MHz, 10 m antennas, Δh = 90 m,
        let mid = itm_loss(&ItmInput { distance_m: 50_000.0, ..base }, &p);
        assert!((125.0..175.0).contains(&mid.total_db), "{mid:?}");
        // …and more with frequency; higher antennas lose less.
        let hi_f = itm_loss(&ItmInput { distance_m: 50_000.0, f_hz: 1e9, ..base }, &p);
        assert!(hi_f.excess_db > mid.excess_db);
        let tall = itm_loss(&ItmInput { distance_m: 50_000.0, h_tx_m: 100.0, ..base }, &p);
        assert!(tall.total_db < mid.total_db - 5.0);
        // Rough terrain pulls the effective horizon in.
        let smooth = itm_loss(&ItmInput { delta_h_m: 0.0, ..base }, &p);
        assert!(smooth.horizon_tx_m > near.horizon_tx_m);
        assert!(near.sphere_weight < smooth.sphere_weight);
    }

    #[test]
    fn delta_h_of_profiles() {
        let flat = Profile { d: (0..100).map(|i| i as f64 * 10.0).collect(), h: (0..100).map(|i| i as f64 * 0.5).collect() };
        assert!(flat.delta_h() < 1e-9, "a tilted plane is smooth");
        let rough = Profile { d: flat.d.clone(), h: (0..100).map(|i| if i % 2 == 0 { 0.0 } else { 50.0 }).collect() };
        assert_relative_eq!(rough.delta_h(), 50.0, epsilon = 2.0);
    }
}
