//! The plain height grid every terrain kernel reads (§11.3): a row-major
//! array of node heights at a fixed spacing, in a local east/north frame.
//! No environment types cross into this crate; `bgws-prop` converts its
//! terrain into one of these.
//!
//! Node `(i, j)` sits at `(origin_x + i·cell, origin_y + j·cell)`; row `j`
//! runs east, rows run north. Heights between nodes are bilinear, and points
//! outside the grid clamp to its edge. The WGSL kernels use exactly the same
//! convention, so CPU and GPU twins sample the same surface.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum GridError {
    #[error("grid is {width}×{height} but has {len} heights")]
    WrongLength { width: usize, height: usize, len: usize },
    #[error("grid needs at least 2×2 nodes and a positive spacing")]
    Degenerate,
}

/// Heights (metres above the datum) on a regular grid of nodes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeightGrid {
    pub width: usize,
    pub height: usize,
    /// Node spacing, metres.
    pub cell: f64,
    /// East coordinate of node (0, 0), metres.
    pub origin_x: f64,
    /// North coordinate of node (0, 0), metres.
    pub origin_y: f64,
    /// Row-major heights, `heights[j * width + i]`.
    pub heights: Vec<f32>,
}

impl HeightGrid {
    pub fn new(width: usize, height: usize, cell: f64, origin_x: f64, origin_y: f64, heights: Vec<f32>) -> Result<Self, GridError> {
        if width < 2 || height < 2 || cell <= 0.0 || !cell.is_finite() {
            return Err(GridError::Degenerate);
        }
        if heights.len() != width * height {
            return Err(GridError::WrongLength { width, height, len: heights.len() });
        }
        Ok(Self { width, height, cell, origin_x, origin_y, heights })
    }

    /// A flat grid at height `h`.
    pub fn flat(width: usize, height: usize, cell: f64, h: f32) -> Self {
        Self { width, height, cell, origin_x: 0.0, origin_y: 0.0, heights: vec![h; width * height] }
    }

    #[inline]
    pub fn node(&self, i: i64, j: i64) -> f64 {
        let ii = i.clamp(0, self.width as i64 - 1) as usize;
        let jj = j.clamp(0, self.height as i64 - 1) as usize;
        self.heights[jj * self.width + ii] as f64
    }

    /// Position of node `(i, j)`.
    pub fn node_pos(&self, i: usize, j: usize) -> (f64, f64) {
        (self.origin_x + i as f64 * self.cell, self.origin_y + j as f64 * self.cell)
    }

    /// Bilinear height at an east/north point, clamped to the grid.
    #[inline]
    pub fn height_at(&self, x: f64, y: f64) -> f64 {
        let gx = ((x - self.origin_x) / self.cell).clamp(0.0, (self.width - 1) as f64);
        let gy = ((y - self.origin_y) / self.cell).clamp(0.0, (self.height - 1) as f64);
        let i0 = gx.floor() as i64;
        let j0 = gy.floor() as i64;
        let fx = gx - i0 as f64;
        let fy = gy - j0 as f64;
        let h00 = self.node(i0, j0);
        let h10 = self.node(i0 + 1, j0);
        let h01 = self.node(i0, j0 + 1);
        let h11 = self.node(i0 + 1, j0 + 1);
        let a = h00 + (h10 - h00) * fx;
        let b = h01 + (h11 - h01) * fx;
        a + (b - a) * fy
    }

    /// Extent of the grid: (min x, min y, max x, max y).
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        (
            self.origin_x,
            self.origin_y,
            self.origin_x + (self.width - 1) as f64 * self.cell,
            self.origin_y + (self.height - 1) as f64 * self.cell,
        )
    }

    pub fn len(&self) -> usize {
        self.heights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn bilinear_and_clamp() {
        let g = HeightGrid::new(2, 2, 10.0, 0.0, 0.0, vec![0.0, 10.0, 20.0, 30.0]).unwrap();
        assert_relative_eq!(g.height_at(5.0, 5.0), 15.0);
        assert_relative_eq!(g.height_at(10.0, 0.0), 10.0);
        assert_relative_eq!(g.height_at(-50.0, 100.0), 20.0);
        assert!(HeightGrid::new(2, 2, 10.0, 0.0, 0.0, vec![0.0; 3]).is_err());
    }
}
