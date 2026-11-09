//! Cost → flow fields (§11.1): many movers heading for one goal (a convoy,
//! a formation) share one integration field instead of each running A*.
//!
//! The integration field is the least accumulated cost from every node to
//! the nearest goal, by Dijkstra over the 8-connected grid; an edge costs
//! its length times the mean of its two nodes' per-metre costs. The flow
//! direction at a node points at its cheapest neighbour. Ties break by node
//! index and direction order, so the field is deterministic. This kernel has
//! no GPU version yet: the CPU twin is the implementation (§11.3 fallback).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Direction codes: 0 N, 1 NE, 2 E, 3 SE, 4 S, 5 SW, 6 W, 7 NW.
pub const DIRS: [(i64, i64); 8] = [(0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1)];
/// No direction: a goal, or unreachable.
pub const NO_DIR: u8 = 255;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowField {
    pub width: usize,
    pub height: usize,
    /// Least cost to a goal (cost units); `f64::INFINITY` if unreachable.
    pub integration: Vec<f64>,
    /// Direction code per node (see [`DIRS`]), or [`NO_DIR`].
    pub direction: Vec<u8>,
}

impl FlowField {
    /// The next node from `(i, j)`, if any.
    pub fn next(&self, i: usize, j: usize) -> Option<(usize, usize)> {
        let d = self.direction[j * self.width + i];
        if d == NO_DIR {
            return None;
        }
        let (di, dj) = DIRS[d as usize];
        Some(((i as i64 + di) as usize, (j as i64 + dj) as usize))
    }
}

#[derive(PartialEq)]
struct Item(f64, usize);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on cost, then on index.
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

/// Build a flow field. `cost[k]` is the per-metre cost multiplier of node
/// `k`; zero, negative or non-finite marks it impassable. `cell` is the node
/// spacing in metres.
pub fn flow_field_cpu(width: usize, height: usize, cell: f64, cost: &[f32], goals: &[usize]) -> FlowField {
    let n = width * height;
    let passable = |k: usize| cost.get(k).is_some_and(|&c| c.is_finite() && c > 0.0);
    let mut dist = vec![f64::INFINITY; n];
    let mut heap = BinaryHeap::new();
    let mut gs: Vec<usize> = goals.iter().copied().filter(|&g| g < n && passable(g)).collect();
    gs.sort_unstable();
    gs.dedup();
    for &g in &gs {
        dist[g] = 0.0;
        heap.push(Item(0.0, g));
    }
    while let Some(Item(d, k)) = heap.pop() {
        if d > dist[k] {
            continue;
        }
        let (i, j) = ((k % width) as i64, (k / width) as i64);
        for (di, dj) in DIRS {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= width as i64 || nj >= height as i64 {
                continue;
            }
            let nk = nj as usize * width + ni as usize;
            if !passable(nk) {
                continue;
            }
            let len = if di != 0 && dj != 0 { cell * std::f64::consts::SQRT_2 } else { cell };
            let nd = d + len * 0.5 * (cost[k] as f64 + cost[nk] as f64);
            if nd < dist[nk] {
                dist[nk] = nd;
                heap.push(Item(nd, nk));
            }
        }
    }
    let direction = (0..n)
        .map(|k| {
            if !dist[k].is_finite() || dist[k] == 0.0 {
                return NO_DIR;
            }
            let (i, j) = ((k % width) as i64, (k / width) as i64);
            let mut best = (dist[k], NO_DIR);
            for (c, (di, dj)) in DIRS.iter().enumerate() {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni >= width as i64 || nj >= height as i64 {
                    continue;
                }
                let nd = dist[nj as usize * width + ni as usize];
                if nd < best.0 {
                    best = (nd, c as u8);
                }
            }
            best.1
        })
        .collect();
    FlowField { width, height, integration: dist, direction }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn flows_around_a_wall_to_the_goal() {
        let (w, h) = (20, 20);
        let mut cost = vec![1f32; w * h];
        for j in 0..15 {
            cost[j * w + 10] = 0.0;
        }
        let goal = 5 * w + 18;
        let f = flow_field_cpu(w, h, 10.0, &cost, &[goal]);
        // Follow the arrows from the far side: must arrive, never cross the wall.
        let (mut i, mut j) = (2usize, 5usize);
        let mut steps = 0;
        while let Some(nx) = f.next(i, j) {
            assert!(cost[nx.1 * w + nx.0] > 0.0);
            (i, j) = nx;
            steps += 1;
            assert!(steps < 400);
        }
        assert_eq!(j * w + i, goal);
        // Straight-line neighbour costs.
        assert_relative_eq!(f.integration[5 * w + 17], 10.0);
        assert_relative_eq!(f.integration[6 * w + 17], 10.0 * std::f64::consts::SQRT_2);
        assert!(f.integration[19 * w + 10].is_finite());
    }
}
