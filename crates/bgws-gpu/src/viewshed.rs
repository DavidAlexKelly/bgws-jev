//! Viewsheds (§11.1): for every grid node, how many observers see a target
//! standing there at a given height. A count rather than a mask, so one pass
//! gives both the union (count > 0) and the overlap of several observers —
//! what exposure-aware routing and quiet skipping want.
//!
//! Each node is tested with the same march as the batched LOS kernel, so a
//! viewshed and a pairwise LOS query never disagree on the CPU.

use crate::grid::HeightGrid;
use crate::los::{los_one, LosParams, LosQuery};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// An observer for a viewshed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewshedObserver {
    pub x: f64,
    pub y: f64,
    /// Eye or sensor height above ground, metres.
    pub agl: f64,
    /// Nodes beyond this ground range are not seen, metres.
    pub max_range: f64,
}

/// Number of observers that see each node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewshed {
    pub width: usize,
    pub height: usize,
    pub count: Vec<u32>,
}

impl Viewshed {
    pub fn seen(&self, i: usize, j: usize) -> bool {
        self.count[j * self.width + i] > 0
    }
    /// Fraction of nodes seen by at least one observer.
    pub fn coverage(&self) -> f64 {
        if self.count.is_empty() {
            return 0.0;
        }
        self.count.iter().filter(|&&c| c > 0).count() as f64 / self.count.len() as f64
    }
}

/// Whether observer `o` sees node `(i, j)` at `target_agl`.
pub fn node_visible(grid: &HeightGrid, o: &ViewshedObserver, i: usize, j: usize, target_agl: f64, p: &LosParams) -> bool {
    let (x, y) = grid.node_pos(i, j);
    if (x - o.x).hypot(y - o.y) > o.max_range {
        return false;
    }
    los_one(grid, &LosQuery { obs_x: o.x, obs_y: o.y, obs_agl: o.agl, tgt_x: x, tgt_y: y, tgt_agl: target_agl }, p).visible
}

/// The CPU twin, parallel over rows.
pub fn viewshed_cpu(grid: &HeightGrid, observers: &[ViewshedObserver], target_agl: f64, p: &LosParams) -> Viewshed {
    let w = grid.width;
    let count: Vec<u32> = (0..grid.height)
        .into_par_iter()
        .flat_map_iter(|j| {
            (0..w).map(move |i| observers.iter().filter(|o| node_visible(grid, o, i, j, target_agl, p)).count() as u32)
        })
        .collect();
    Viewshed { width: w, height: grid.height, count }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_casts_a_shadow() {
        // A 30 m wall across x = 200 m on a 41×21 grid at 10 m.
        let mut h = vec![0f32; 41 * 21];
        for j in 0..21 {
            h[j * 41 + 20] = 30.0;
        }
        let g = HeightGrid::new(41, 21, 10.0, 0.0, 0.0, h).unwrap();
        let o = ViewshedObserver { x: 50.0, y: 100.0, agl: 2.0, max_range: 1e6 };
        let v = viewshed_cpu(&g, &[o], 2.0, &LosParams::default());
        assert!(v.seen(10, 10));
        assert!(v.seen(20, 10), "the wall top is visible");
        assert!(!v.seen(35, 10), "behind the wall is dead ground");
        let two = viewshed_cpu(&g, &[o, ViewshedObserver { x: 350.0, ..o }], 2.0, &LosParams::default());
        assert_eq!(two.count[10 * 41 + 20], 2);
        assert!(two.coverage() > v.coverage());
    }
}
