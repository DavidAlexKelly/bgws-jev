//! A theatre pack's terrain appreciation: what a staff works out about the
//! ground before a fight, done once when the pack is built and kept with it
//! (`analysis.bin`), not again for every decision.
//!
//! - **Exposure** (dead ground): for each cell, the share of the ground
//!   within a radius that a man-high eye there sees. Sight runs both ways,
//!   so it is also how much of that ground can see the cell: low is dead
//!   ground, high is open to view.
//! - **Mobility**, for tracked, wheeled and foot movers: each cell go,
//!   slow-go or no-go, and how wide the go-ground is there (twice the
//!   distance to the nearest ground that is not go).
//! - **Key terrain**: points with what they are and why they matter:
//!   observation posts (ground that sees the most), chokepoints (where wide
//!   corridors narrow to a gap) and river crossings (bridges).
//!
//! File: `BGTA`, a little-endian u32 header length, the JSON header (grids,
//! key terrain, settings), then LZ4 blocks: exposure (u8, ×255), then per
//! class going (u8: 0 no-go, 1 slow-go, 2 go) and corridor width (u16, m).

use crate::raster::GridSpec;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"BGTA";
/// Bumped when the analysis changes, so packs with an older one redo it.
pub const VERSION: u32 = 1;

/// Mover classes, in this order: tracked, wheeled, foot.
pub const CLASSES: [&str; 3] = ["tracked", "wheeled", "foot"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    /// Ground that sees far: an observation post or a position to hold.
    Observation,
    /// A gap where open going narrows between two wide corridors.
    Chokepoint,
    /// A road bridge over a river.
    Crossing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyTerrain {
    pub kind: KeyKind,
    /// Position in the pack's frame, m.
    pub e: f64,
    pub n: f64,
    /// How much it matters, 0–1 within its kind.
    pub score: f64,
    /// Mover classes it concerns (chokepoints), empty for all.
    #[serde(default)]
    pub classes: Vec<String>,
    /// A few words: what it is and why.
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSettings {
    pub exposure_radius_m: f64,
    pub eye_m: f64,
    pub corridor_m: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Appreciation {
    pub version: u32,
    /// The exposure grid (coarser than the terrain's).
    pub exposure_spec: GridSpec,
    pub exposure: Vec<u8>,
    /// The mobility grid (the terrain's).
    pub mobility_spec: GridSpec,
    /// Per class: 0 no-go, 1 slow-go, 2 go.
    pub going: [Vec<u8>; 3],
    /// Per class: width of the go-ground through the cell, m (0 off it).
    pub width_m: [Vec<u16>; 3],
    pub key: Vec<KeyTerrain>,
    pub settings: AnalysisSettings,
}

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    exposure_spec: GridSpec,
    mobility_spec: GridSpec,
    key: Vec<KeyTerrain>,
    settings: AnalysisSettings,
    /// Compressed lengths: exposure, then going and width per class.
    lens: Vec<u64>,
}

fn nearest(s: &GridSpec, e: f64, n: f64) -> Option<usize> {
    s.nearest(e, n).map(|(i, j)| s.idx(i, j))
}

impl Appreciation {
    /// Exposure at a point (0–1), nearest cell.
    pub fn exposure_at(&self, e: f64, n: f64) -> Option<f64> {
        nearest(&self.exposure_spec, e, n).map(|k| self.exposure[k] as f64 / 255.0)
    }

    /// Going class (0 no-go, 1 slow, 2 go) and corridor width for a class at a point.
    pub fn mobility_at(&self, class: usize, e: f64, n: f64) -> Option<(u8, f64)> {
        nearest(&self.mobility_spec, e, n).map(|k| (self.going[class][k], self.width_m[class][k] as f64))
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let mut blocks = vec![lz4_flex::block::compress(&self.exposure)];
        for c in 0..3 {
            blocks.push(lz4_flex::block::compress(&self.going[c]));
            let w: Vec<u8> = self.width_m[c].iter().flat_map(|v| v.to_le_bytes()).collect();
            blocks.push(lz4_flex::block::compress(&w));
        }
        let h = Header {
            version: self.version,
            exposure_spec: self.exposure_spec,
            mobility_spec: self.mobility_spec,
            key: self.key.clone(),
            settings: self.settings.clone(),
            lens: blocks.iter().map(|b| b.len() as u64).collect(),
        };
        let h = serde_json::to_vec(&h).map_err(std::io::Error::other)?;
        let mut out = Vec::with_capacity(8 + h.len() + blocks.iter().map(Vec::len).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(h.len() as u32).to_le_bytes());
        out.extend_from_slice(&h);
        for b in &blocks {
            out.extend_from_slice(b);
        }
        // Aside, then into place: a game opening meanwhile never reads half a file.
        let part = path.with_extension("bin.part");
        std::fs::write(&part, out)?;
        std::fs::rename(&part, path)
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {m}", path.display()));
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(bad("not a terrain analysis"));
        }
        let hl = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let h: Header = serde_json::from_slice(bytes.get(8..8 + hl).ok_or_else(|| bad("truncated header"))?).map_err(|e| bad(&e.to_string()))?;
        if h.lens.len() != 7 {
            return Err(bad("wrong layout"));
        }
        let mut at = 8 + hl;
        let mut next = |len: u64, n: usize| -> std::io::Result<Vec<u8>> {
            let b = bytes.get(at..at + len as usize).ok_or_else(|| bad("truncated"))?;
            at += len as usize;
            let v = lz4_flex::block::decompress(b, n).map_err(|e| bad(&e.to_string()))?;
            if v.len() != n {
                return Err(bad("wrong size"));
            }
            Ok(v)
        };
        let (ne, nm) = (h.exposure_spec.len(), h.mobility_spec.len());
        let exposure = next(h.lens[0], ne)?;
        let mut going: [Vec<u8>; 3] = Default::default();
        let mut width_m: [Vec<u16>; 3] = Default::default();
        for c in 0..3 {
            going[c] = next(h.lens[1 + 2 * c], nm)?;
            width_m[c] = next(h.lens[2 + 2 * c], 2 * nm)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        }
        Ok(Self { version: h.version, exposure_spec: h.exposure_spec, mobility_spec: h.mobility_spec, exposure, going, width_m, key: h.key, settings: h.settings })
    }
}

/// An appreciation seen from a scenario's frame (as `SightView`): points
/// go into the pack's frame by an affine map; key terrain is given in the
/// scenario's frame.
#[derive(Clone, Debug)]
pub struct AppreciationView {
    pub a: Arc<Appreciation>,
    pub to_pack: [f64; 6],
    /// Key terrain with (e, n) in the scenario's frame.
    pub key: Vec<KeyTerrain>,
}

impl AppreciationView {
    pub fn same_frame(a: Arc<Appreciation>) -> Self {
        let key = a.key.clone();
        Self { a, to_pack: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], key }
    }

    fn pack(&self, e: f64, n: f64) -> (f64, f64) {
        let m = &self.to_pack;
        (m[0] * e + m[1] * n + m[2], m[3] * e + m[4] * n + m[5])
    }

    pub fn exposure(&self, e: f64, n: f64) -> Option<f64> {
        let (x, y) = self.pack(e, n);
        self.a.exposure_at(x, y)
    }

    pub fn mobility(&self, class: usize, e: f64, n: f64) -> Option<(u8, f64)> {
        let (x, y) = self.pack(e, n);
        self.a.mobility_at(class, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let es = GridSpec::new(4, 3, 60.0, 0.0, 0.0);
        let ms = GridSpec::new(8, 6, 30.0, 0.0, 0.0);
        let a = Appreciation {
            version: VERSION,
            exposure_spec: es,
            exposure: (0..12).map(|k| k * 20).collect(),
            mobility_spec: ms,
            going: [vec![2; 48], vec![1; 48], vec![0; 48]],
            width_m: [vec![300; 48], vec![1_200; 48], vec![0; 48]],
            key: vec![KeyTerrain { kind: KeyKind::Crossing, e: 30.0, n: 30.0, score: 1.0, classes: vec![], note: "bridge".into() }],
            settings: AnalysisSettings { exposure_radius_m: 2_000.0, eye_m: 2.0, corridor_m: 400.0 },
        };
        let p = std::env::temp_dir().join(format!("bgws-analysis-{}.bin", std::process::id()));
        a.write(&p).unwrap();
        let r = Appreciation::read(&p).unwrap();
        assert_eq!(r, a);
        assert_eq!(r.mobility_at(1, 40.0, 40.0), Some((1, 1_200.0)));
        assert!((r.exposure_at(60.0, 0.0).unwrap() - 20.0 / 255.0).abs() < 1e-9);
        let v = AppreciationView { to_pack: [1.0, 0.0, 60.0, 0.0, 1.0, 0.0], key: vec![], a: Arc::new(r) };
        assert!((v.exposure(0.0, 0.0).unwrap() - 20.0 / 255.0).abs() < 1e-9);
        let _ = std::fs::remove_file(p);
    }
}
