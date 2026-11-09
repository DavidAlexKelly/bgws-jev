//! What sight lines are traced through, finer than the terrain: each
//! building's footprint at its height, and the woods by class, on a grid of
//! a few metres (5 m in a 30 m pack). Built from the map data with the pack
//! (`surface.bin`), and read by the line-of-sight code: a line that passes
//! below a roof is blocked, and one that passes through trees counts its
//! length through them toward the chance it is blocked.
//!
//! File: `BGSF`, a little-endian u32 header length, the JSON header (the
//! grid), then the building heights (u8, whole metres) and the tree classes
//! (u8: 0 none, else the land-cover class + 1), each an LZ4 block.

use crate::raster::GridSpec;
use crate::LandCover;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"BGSF";

#[derive(Clone, Debug, PartialEq)]
pub struct SightSurface {
    pub spec: GridSpec,
    /// Building height above the ground, whole metres (0 none).
    pub building_m: Vec<u8>,
    /// The wood in each cell: 0 none, else the land-cover class + 1.
    pub tree: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    spec: GridSpec,
    building_len: u64,
    tree_len: u64,
}

impl SightSurface {
    pub fn empty(spec: GridSpec) -> Self {
        let n = spec.len();
        Self { spec, building_m: vec![0; n], tree: vec![0; n] }
    }

    /// The building height and the wood at a point (nearest cell), in this surface's frame.
    pub fn at(&self, e: f64, n: f64) -> (f64, Option<LandCover>) {
        let s = &self.spec;
        let i = ((e - s.origin_e) / s.cell).round();
        let j = ((n - s.origin_n) / s.cell).round();
        if i < 0.0 || j < 0.0 || i >= s.width as f64 || j >= s.height as f64 {
            return (0.0, None);
        }
        let k = j as usize * s.width + i as usize;
        let tree = self.tree[k].checked_sub(1).and_then(|c| LandCover::ALL.get(c as usize).copied());
        (self.building_m[k] as f64, tree)
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let b = lz4_flex::block::compress(&self.building_m);
        let t = lz4_flex::block::compress(&self.tree);
        let h = serde_json::to_vec(&Header { spec: self.spec, building_len: b.len() as u64, tree_len: t.len() as u64 }).map_err(std::io::Error::other)?;
        let mut out = Vec::with_capacity(8 + h.len() + b.len() + t.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(h.len() as u32).to_le_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(&b);
        out.extend_from_slice(&t);
        std::fs::write(path, out)
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {m}", path.display()));
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(bad("not a sight surface"));
        }
        let hl = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let h: Header = serde_json::from_slice(bytes.get(8..8 + hl).ok_or_else(|| bad("truncated header"))?).map_err(|e| bad(&e.to_string()))?;
        let n = h.spec.len();
        let (b0, b1) = (8 + hl, 8 + hl + h.building_len as usize);
        let t1 = b1 + h.tree_len as usize;
        let building_m = lz4_flex::block::decompress(bytes.get(b0..b1).ok_or_else(|| bad("truncated"))?, n).map_err(|e| bad(&e.to_string()))?;
        let tree = lz4_flex::block::decompress(bytes.get(b1..t1).ok_or_else(|| bad("truncated"))?, n).map_err(|e| bad(&e.to_string()))?;
        if building_m.len() != n || tree.len() != n {
            return Err(bad("wrong size"));
        }
        Ok(Self { spec: h.spec, building_m, tree })
    }
}

/// A surface seen from a scenario's frame: points are carried into the
/// pack's frame by an affine map (the two local frames differ by a shift
/// and a slight turn), so the surface is never re-sampled.
#[derive(Clone, Debug)]
pub struct SightView {
    pub surface: Arc<SightSurface>,
    /// (e, n) in the scenario's frame → [a·e + b·n + c, d·e + f·n + g] in the pack's.
    pub to_pack: [f64; 6],
    /// What still stands of the buildings, by place in the scenario's frame
    /// (×255; none: all of them): a building's height is scaled by it.
    pub standing: Option<Arc<crate::Raster<u8>>>,
}

impl SightView {
    /// The surface in the frame it was built in.
    pub fn same_frame(surface: Arc<SightSurface>) -> Self {
        Self::new(surface, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    }

    pub fn new(surface: Arc<SightSurface>, to_pack: [f64; 6]) -> Self {
        Self { surface, to_pack, standing: None }
    }

    pub fn at(&self, e: f64, n: f64) -> (f64, Option<LandCover>) {
        let m = &self.to_pack;
        let (h, tree) = self.surface.at(m[0] * e + m[1] * n + m[2], m[3] * e + m[4] * n + m[5]);
        match &self.standing {
            Some(r) if h > 0.0 => (h * r.spec.nearest(e, n).map(|(i, j)| r.data[r.spec.idx(i, j)] as f64 / 255.0).unwrap_or(1.0), tree),
            _ => (h, tree),
        }
    }

    /// The surface's cell, metres: how finely lines through it are stepped.
    pub fn cell(&self) -> f64 {
        self.surface.spec.cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_answers_by_cell() {
        let spec = GridSpec::new(40, 30, 5.0, -100.0, -50.0);
        let mut s = SightSurface::empty(spec);
        let k = spec.idx(10, 10);
        s.building_m[k] = 18;
        s.tree[spec.idx(20, 5)] = LandCover::ForestDense as u8 + 1;
        let path = std::env::temp_dir().join(format!("bgws-surface-{}.bin", std::process::id()));
        s.write(&path).unwrap();
        let r = SightSurface::read(&path).unwrap();
        assert_eq!(r, s);
        let (e, n) = spec.pos(10, 10);
        assert_eq!(r.at(e + 2.0, n - 2.0), (18.0, None));
        let (e, n) = spec.pos(20, 5);
        assert_eq!(r.at(e, n).1, Some(LandCover::ForestDense));
        assert_eq!(r.at(-1_000.0, 0.0), (0.0, None));
        // Shifted 100 m east: the scenario's (e, n) is the pack's (e + 100, n).
        let v = SightView::new(Arc::new(r), [1.0, 0.0, 100.0, 0.0, 1.0, 0.0]);
        let (pe, pn) = spec.pos(10, 10);
        assert_eq!(v.at(pe - 100.0, pn).0, 18.0);
        let _ = std::fs::remove_file(path);
    }
}
