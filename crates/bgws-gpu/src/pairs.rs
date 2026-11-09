//! Candidate pairs (§11.1, §8.2): from entity positions by layer, sensor
//! ranges and fields of regard, the observer–target pairs that are in range
//! and in view. A uniform hash grid keeps the cost near O(n·k) instead of
//! O(n²).
//!
//! "The GPU narrows, the CPU decides" (rule 9): the GPU twin returns a
//! superset with a small slack on range and angle, and the exact f64 test
//! here filters it, so both twins return identical pair lists. Output is
//! sorted by (observer, target).

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// One entity, as an observer (if it has a sensor) and as a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct PairEntity {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Layer index 0..32 (ground, air, surface, subsurface, …).
    pub layer: u32,
    /// Bitmask of target layers this entity's sensor can see; 0 = no sensor.
    pub sees_layers: u32,
    /// Sensor maximum range (3-D slant), metres.
    pub sensor_range: f64,
    /// Sensor boresight, radians clockwise from north.
    pub heading: f64,
    /// Half-width of the azimuth field of regard, radians; ≥ π is all-round.
    pub half_fov: f64,
}

/// A candidate pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidatePair {
    pub observer: u32,
    pub target: u32,
    /// Slant range, rounded to centimetres.
    pub range_m: f64,
}

/// Hash-grid settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairParams {
    /// Hash cell size, metres; 0 picks the largest sensor range.
    pub cell_m: f64,
    /// Relative slack the GPU applies before CPU refinement.
    pub gpu_slack: f64,
}

impl PairParams {
    pub fn from_figures(f: &bgws_core::Figures) -> Self {
        Self { cell_m: f.get("gpu.pairs.hash_cell_m"), gpu_slack: f.get("gpu.pairs.gpu_slack") }
    }
}

impl Default for PairParams {
    fn default() -> Self {
        Self { cell_m: 0.0, gpu_slack: 1e-3 }
    }
}

/// Signed smallest angle difference in (−π, π].
#[inline]
pub fn wrap_angle(a: f64) -> f64 {
    let r = (a + PI).rem_euclid(TAU) - PI;
    if r <= -PI {
        r + TAU
    } else {
        r
    }
}

/// The exact test: does `o` see `t` (layer, range, field of regard)?
/// Returns the slant range when it does.
#[inline]
pub fn pair_test(o: &PairEntity, t: &PairEntity) -> Option<f64> {
    if o.sees_layers & (1u32 << (t.layer & 31)) == 0 {
        return None;
    }
    let (dx, dy, dz) = (t.x - o.x, t.y - o.y, t.z - o.z);
    let r = (dx * dx + dy * dy + dz * dz).sqrt();
    if r > o.sensor_range {
        return None;
    }
    if o.half_fov < PI && (dx != 0.0 || dy != 0.0) {
        let bearing = dx.atan2(dy);
        if wrap_angle(bearing - o.heading).abs() > o.half_fov {
            return None;
        }
    }
    Some(r)
}

#[inline]
fn round_cm(r: f64) -> f64 {
    (r * 100.0).round() / 100.0
}

/// Brute force (the reference the hash grid is tested against).
pub fn candidate_pairs_brute(entities: &[PairEntity]) -> Vec<CandidatePair> {
    let mut out = Vec::new();
    for (i, o) in entities.iter().enumerate() {
        if o.sees_layers == 0 {
            continue;
        }
        for (j, t) in entities.iter().enumerate() {
            if i != j {
                if let Some(r) = pair_test(o, t) {
                    out.push(CandidatePair { observer: i as u32, target: j as u32, range_m: round_cm(r) });
                }
            }
        }
    }
    out
}

/// Refine a candidate list (e.g. from the GPU) with the exact test, sorted.
pub fn refine_pairs(entities: &[PairEntity], mut cand: Vec<(u32, u32)>) -> Vec<CandidatePair> {
    cand.sort_unstable();
    cand.dedup();
    cand.into_iter()
        .filter(|(i, j)| i != j)
        .filter_map(|(i, j)| {
            pair_test(&entities[i as usize], &entities[j as usize])
                .map(|r| CandidatePair { observer: i, target: j, range_m: round_cm(r) })
        })
        .collect()
}

/// The CPU twin: uniform hash grid, parallel over observers.
pub fn candidate_pairs_cpu(entities: &[PairEntity], p: &PairParams) -> Vec<CandidatePair> {
    if entities.is_empty() {
        return Vec::new();
    }
    let max_range = entities.iter().filter(|e| e.sees_layers != 0).map(|e| e.sensor_range).fold(0.0, f64::max);
    if max_range <= 0.0 {
        return Vec::new();
    }
    let cell = if p.cell_m > 0.0 { p.cell_m } else { max_range };
    let key = |e: &PairEntity| ((e.x / cell).floor() as i64, (e.y / cell).floor() as i64);
    // Sorted (cell, index) list: deterministic, no hash iteration.
    let mut bins: Vec<((i64, i64), u32)> = entities.iter().enumerate().map(|(i, e)| (key(e), i as u32)).collect();
    bins.sort_unstable();

    let per_obs: Vec<Vec<CandidatePair>> = entities
        .par_iter()
        .enumerate()
        .map(|(i, o)| {
            let mut out = Vec::new();
            if o.sees_layers == 0 || o.sensor_range <= 0.0 {
                return out;
            }
            let (cx, cy) = key(o);
            let reach = (o.sensor_range / cell).ceil() as i64;
            for gx in cx - reach..=cx + reach {
                let lo = bins.partition_point(|b| b.0 < (gx, cy - reach));
                let hi = bins.partition_point(|b| b.0 <= (gx, cy + reach));
                for &(_, j) in &bins[lo..hi] {
                    if j as usize == i {
                        continue;
                    }
                    if let Some(r) = pair_test(o, &entities[j as usize]) {
                        out.push(CandidatePair { observer: i as u32, target: j, range_m: round_cm(r) });
                    }
                }
            }
            out.sort_unstable_by_key(|c| c.target);
            out
        })
        .collect();
    per_obs.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ent() -> impl Strategy<Value = PairEntity> {
        (-5000.0f64..5000.0, -5000.0f64..5000.0, 0.0f64..500.0, 0u32..3, 0u32..8, 0.0f64..3000.0, 0.0f64..TAU, 0.1f64..4.0)
            .prop_map(|(x, y, z, layer, sees, r, h, f)| PairEntity { x, y, z, layer, sees_layers: sees, sensor_range: r, heading: h, half_fov: f })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn hash_grid_equals_brute_force(es in proptest::collection::vec(ent(), 0..120), cell in 0.0f64..2000.0) {
            let a = candidate_pairs_cpu(&es, &PairParams { cell_m: cell, ..Default::default() });
            let b = candidate_pairs_brute(&es);
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn field_of_regard_and_layers() {
        let o = PairEntity { sees_layers: 0b1, sensor_range: 1000.0, half_fov: 0.5, ..Default::default() };
        let ahead = PairEntity { y: 500.0, ..Default::default() };
        let behind = PairEntity { y: -500.0, ..Default::default() };
        let air = PairEntity { y: 400.0, layer: 1, ..Default::default() };
        let far = PairEntity { y: 1500.0, ..Default::default() };
        let r = candidate_pairs_cpu(&[o, ahead, behind, air, far], &PairParams::default());
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].observer, r[0].target, r[0].range_m), (0, 1, 500.0));
        assert!((wrap_angle(3.0 * PI) - PI).abs() < 1e-12);
    }
}
