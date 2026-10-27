//! Georeferenced rasters (§7.1): a regular grid of nodes in a local ENU
//! frame. Node `(i, j)` sits at `(origin_e + i·cell, origin_n + j·cell)`,
//! row-major with rows running north — the same convention as
//! `bgws_gpu::HeightGrid`, so terrain converts to the kernel layer without
//! resampling.

use serde::{Deserialize, Serialize};

/// Shape and placement of a grid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub width: usize,
    pub height: usize,
    /// Node spacing, metres.
    pub cell: f64,
    /// East of node (0, 0), metres in the theatre's ENU frame.
    pub origin_e: f64,
    /// North of node (0, 0), metres.
    pub origin_n: f64,
}

impl GridSpec {
    pub fn new(width: usize, height: usize, cell: f64, origin_e: f64, origin_n: f64) -> Self {
        Self { width, height, cell, origin_e, origin_n }
    }
    pub fn len(&self) -> usize {
        self.width * self.height
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn idx(&self, i: usize, j: usize) -> usize {
        j * self.width + i
    }
    #[inline]
    pub fn ij(&self, k: usize) -> (usize, usize) {
        (k % self.width, k / self.width)
    }
    /// Position of node `(i, j)`.
    #[inline]
    pub fn pos(&self, i: usize, j: usize) -> (f64, f64) {
        (self.origin_e + i as f64 * self.cell, self.origin_n + j as f64 * self.cell)
    }
    /// Nearest node to a point, if inside the grid's extent (half a cell slack).
    pub fn nearest(&self, e: f64, n: f64) -> Option<(usize, usize)> {
        let gi = ((e - self.origin_e) / self.cell).round();
        let gj = ((n - self.origin_n) / self.cell).round();
        if gi < 0.0 || gj < 0.0 || gi >= self.width as f64 || gj >= self.height as f64 {
            return None;
        }
        Some((gi as usize, gj as usize))
    }
    /// Nearest node, clamped to the grid.
    pub fn nearest_clamped(&self, e: f64, n: f64) -> (usize, usize) {
        let gi = ((e - self.origin_e) / self.cell).round().clamp(0.0, (self.width - 1) as f64);
        let gj = ((n - self.origin_n) / self.cell).round().clamp(0.0, (self.height - 1) as f64);
        (gi as usize, gj as usize)
    }
    /// Continuous grid coordinates, clamped.
    #[inline]
    pub fn grid_coords(&self, e: f64, n: f64) -> (f64, f64) {
        (
            ((e - self.origin_e) / self.cell).clamp(0.0, (self.width.max(1) - 1) as f64),
            ((n - self.origin_n) / self.cell).clamp(0.0, (self.height.max(1) - 1) as f64),
        )
    }
    /// (min e, min n, max e, max n).
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        (
            self.origin_e,
            self.origin_n,
            self.origin_e + (self.width.saturating_sub(1)) as f64 * self.cell,
            self.origin_n + (self.height.saturating_sub(1)) as f64 * self.cell,
        )
    }
    pub fn contains(&self, e: f64, n: f64) -> bool {
        let (a, b, c, d) = self.bounds();
        e >= a && e <= c && n >= b && n <= d
    }
}

/// A grid of values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Raster<T> {
    pub spec: GridSpec,
    pub data: Vec<T>,
}

impl<T: Copy> Raster<T> {
    pub fn filled(spec: GridSpec, v: T) -> Self {
        Self { spec, data: vec![v; spec.len()] }
    }
    pub fn from_fn(spec: GridSpec, mut f: impl FnMut(usize, usize) -> T) -> Self {
        let mut data = Vec::with_capacity(spec.len());
        for j in 0..spec.height {
            for i in 0..spec.width {
                data.push(f(i, j));
            }
        }
        Self { spec, data }
    }
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> T {
        self.data[self.spec.idx(i, j)]
    }
    #[inline]
    pub fn get_clamped(&self, i: i64, j: i64) -> T {
        let ii = i.clamp(0, self.spec.width as i64 - 1) as usize;
        let jj = j.clamp(0, self.spec.height as i64 - 1) as usize;
        self.get(ii, jj)
    }
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: T) {
        let k = self.spec.idx(i, j);
        self.data[k] = v;
    }
    /// Value at the nearest node (categorical layers).
    pub fn nearest(&self, e: f64, n: f64) -> T {
        let (i, j) = self.spec.nearest_clamped(e, n);
        self.get(i, j)
    }
}

impl<T: Copy + Into<f64>> Raster<T> {
    /// Bilinear sample, clamped to the grid.
    pub fn bilinear(&self, e: f64, n: f64) -> f64 {
        let (gx, gy) = self.spec.grid_coords(e, n);
        let (i0, j0) = (gx.floor() as i64, gy.floor() as i64);
        let (fx, fy) = (gx - i0 as f64, gy - j0 as f64);
        let v = |i, j| -> f64 { self.get_clamped(i, j).into() };
        let a = v(i0, j0) + (v(i0 + 1, j0) - v(i0, j0)) * fx;
        let b = v(i0, j0 + 1) + (v(i0 + 1, j0 + 1) - v(i0, j0 + 1)) * fx;
        a + (b - a) * fy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexing_and_sampling() {
        let s = GridSpec::new(3, 2, 10.0, 100.0, 200.0);
        let r = Raster::from_fn(s, |i, j| (i + 10 * j) as f32);
        assert_eq!(r.get(2, 1), 12.0);
        assert_eq!(s.ij(5), (2, 1));
        assert!((r.bilinear(105.0, 205.0) - 5.5).abs() < 1e-12);
        assert_eq!(s.nearest(96.0, 200.0), Some((0, 0)));
        assert_eq!(s.nearest(80.0, 200.0), None);
        assert_eq!(s.bounds(), (100.0, 200.0, 120.0, 210.0));
    }
}
