//! The runtime tile format and theatre packs (§14.2, §14.3).
//!
//! **Tile** (`.bgwt`): little-endian binary.
//!
//! ```text
//! b"BGWT"              magic
//! u32                  header length H
//! H bytes              JSON header: version, grid spec, frame origin, layers
//! payload              each layer at its offset, raw or LZ4 (lz4_flex block)
//! ```
//!
//! Layers are `elevation` (f32), `land_cover` (u8 class code), `soil` (u8),
//! `wetness` (f32) and `obstacle_gap` (f32), row-major like the grid.
//!
//! **Theatre pack**: a folder with `pack.json` (metadata and the tile index),
//! `tiles/t_<ix>_<iy>.bgwt`, and JSON graphs (`roads.json`, `rivers.json`,
//! `power_lines.json`, `towns.json`). Neighbouring tiles share their edge
//! row and column, so a point on a seam samples the same height from either.
//! Opening a pack reads only `pack.json`; tiles load lazily by area of
//! interest and stay cached until unloaded.

use crate::landcover::LandCover;
use crate::network::{PowerLine, River, RoadNetwork};
use crate::raster::{GridSpec, Raster};
use crate::soil::SoilType;
use crate::synth::{SynthWorld, Town};
use crate::terrain::Terrain;
use bgws_core::Geodetic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const TILE_MAGIC: &[u8; 4] = b"BGWT";
pub const TILE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum TileError {
    #[error("not a BGWS tile (bad magic)")]
    BadMagic,
    #[error("tile is truncated")]
    Truncated,
    #[error("unsupported tile version {0}")]
    Version(u32),
    #[error("bad tile header: {0}")]
    Header(String),
    #[error("layer `{0}` is missing or has the wrong size")]
    Layer(String),
    #[error("layer `{layer}` has an invalid value {value} at {index}")]
    BadValue { layer: String, value: u8, index: usize },
    #[error("decompression failed: {0}")]
    Decompress(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Raw,
    Lz4,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerEntry {
    pub name: String,
    /// "f32" or "u8".
    pub dtype: String,
    pub codec: Codec,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileHeader {
    pub version: u32,
    pub spec: GridSpec,
    pub frame_origin: Geodetic,
    pub layers: Vec<LayerEntry>,
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Encode a terrain grid as a tile.
pub fn write_tile(t: &Terrain, codec: Codec) -> Result<Vec<u8>, TileError> {
    let raw: Vec<(&str, &str, Vec<u8>)> = vec![
        ("elevation", "f32", f32_bytes(&t.elevation.data)),
        ("land_cover", "u8", t.land_cover.data.iter().map(|c| *c as u8).collect()),
        ("soil", "u8", t.soil.data.iter().map(|c| *c as u8).collect()),
        ("wetness", "f32", f32_bytes(&t.wetness.data)),
        ("obstacle_gap", "f32", f32_bytes(&t.obstacle_gap.data)),
    ];
    let mut payload = Vec::new();
    let mut layers = Vec::new();
    for (name, dtype, bytes) in raw {
        let enc = match codec {
            Codec::Raw => bytes.clone(),
            Codec::Lz4 => lz4_flex::block::compress(&bytes),
        };
        layers.push(LayerEntry {
            name: name.into(),
            dtype: dtype.into(),
            codec,
            offset: payload.len() as u64,
            length: enc.len() as u64,
            raw_length: bytes.len() as u64,
        });
        payload.extend_from_slice(&enc);
    }
    let header = serde_json::to_vec(&TileHeader { version: TILE_VERSION, spec: t.spec, frame_origin: t.frame_origin, layers })?;
    let mut out = Vec::with_capacity(8 + header.len() + payload.len());
    out.extend_from_slice(TILE_MAGIC);
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode a tile.
pub fn read_tile(bytes: &[u8]) -> Result<Terrain, TileError> {
    if bytes.len() < 8 {
        return Err(TileError::Truncated);
    }
    if &bytes[0..4] != TILE_MAGIC {
        return Err(TileError::BadMagic);
    }
    let hlen = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let body = 8usize.checked_add(hlen).filter(|&e| e <= bytes.len()).ok_or(TileError::Truncated)?;
    let header: TileHeader = serde_json::from_slice(&bytes[8..body]).map_err(|e| TileError::Header(e.to_string()))?;
    if header.version != TILE_VERSION {
        return Err(TileError::Version(header.version));
    }
    let spec = header.spec;
    if spec.width == 0 || spec.height == 0 || !(spec.cell > 0.0) || spec.width.checked_mul(spec.height).is_none() {
        return Err(TileError::Header("bad grid spec".into()));
    }
    let payload = &bytes[body..];
    let n = spec.len();
    let layer = |name: &str, dtype: &str| -> Result<Vec<u8>, TileError> {
        let l = header.layers.iter().find(|l| l.name == name && l.dtype == dtype).ok_or_else(|| TileError::Layer(name.into()))?;
        let width = if dtype == "f32" { 4 } else { 1 };
        if l.raw_length as usize != n * width {
            return Err(TileError::Layer(name.into()));
        }
        let start = l.offset as usize;
        let end = start.checked_add(l.length as usize).filter(|&e| e <= payload.len()).ok_or(TileError::Truncated)?;
        let data = &payload[start..end];
        let raw = match l.codec {
            Codec::Raw => data.to_vec(),
            Codec::Lz4 => lz4_flex::block::decompress(data, l.raw_length as usize).map_err(|e| TileError::Decompress(e.to_string()))?,
        };
        if raw.len() != n * width {
            return Err(TileError::Layer(name.into()));
        }
        Ok(raw)
    };
    let f32s = |name: &str| -> Result<Vec<f32>, TileError> {
        Ok(layer(name, "f32")?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    };
    let lc = layer("land_cover", "u8")?
        .into_iter()
        .enumerate()
        .map(|(i, v)| LandCover::from_u8(v).ok_or(TileError::BadValue { layer: "land_cover".into(), value: v, index: i }))
        .collect::<Result<Vec<_>, _>>()?;
    let soil = layer("soil", "u8")?
        .into_iter()
        .enumerate()
        .map(|(i, v)| SoilType::from_u8(v).ok_or(TileError::BadValue { layer: "soil".into(), value: v, index: i }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Terrain {
        frame_origin: header.frame_origin,
        spec,
        elevation: Raster { spec, data: f32s("elevation")? },
        land_cover: Raster { spec, data: lc },
        soil: Raster { spec, data: soil },
        wetness: Raster { spec, data: f32s("wetness")? },
        obstacle_gap: Raster { spec, data: f32s("obstacle_gap")? },
    })
}

/// An area of interest in the theatre's ENU frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aoi {
    pub min_e: f64,
    pub min_n: f64,
    pub max_e: f64,
    pub max_n: f64,
}

impl Aoi {
    pub fn around(e: f64, n: f64, r: f64) -> Self {
        Self { min_e: e - r, min_n: n - r, max_e: e + r, max_n: n + r }
    }
    fn intersects(&self, s: &GridSpec) -> bool {
        let (a, b, c, d) = s.bounds();
        !(c < self.min_e || a > self.max_e || d < self.min_n || b > self.max_n)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileEntry {
    pub ix: u32,
    pub iy: u32,
    pub file: String,
    pub spec: GridSpec,
}

/// `pack.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackMeta {
    pub name: String,
    pub version: u32,
    pub frame_origin: Geodetic,
    pub cell: f64,
    /// Node intervals per tile side.
    pub tile_intervals: usize,
    /// The whole theatre's grid.
    pub spec: GridSpec,
    pub tiles: Vec<TileEntry>,
    #[serde(default)]
    pub roads: Option<String>,
    #[serde(default)]
    pub rivers: Option<String>,
    #[serde(default)]
    pub power_lines: Option<String>,
    #[serde(default)]
    pub towns: Option<String>,
}

/// A theatre pack opened from disk, with lazily loaded tiles.
#[derive(Debug)]
pub struct TheatrePack {
    pub dir: PathBuf,
    pub meta: PackMeta,
    cache: BTreeMap<(u32, u32), Terrain>,
}

impl TheatrePack {
    /// Write a synthetic world as a pack, tiling every `tile_intervals` nodes.
    pub fn write(dir: &Path, name: &str, w: &SynthWorld, tile_intervals: usize, codec: Codec) -> Result<PackMeta, TileError> {
        std::fs::create_dir_all(dir.join("tiles"))?;
        let t = &w.terrain;
        let ti = tile_intervals.max(1);
        let mut tiles = Vec::new();
        let (nx, ny) = ((t.spec.width - 1).div_ceil(ti).max(1), (t.spec.height - 1).div_ceil(ti).max(1));
        for iy in 0..ny {
            for ix in 0..nx {
                let sub = t.crop(ix * ti, iy * ti, ti + 1, ti + 1);
                let file = format!("tiles/t_{ix}_{iy}.bgwt");
                std::fs::write(dir.join(&file), write_tile(&sub, codec)?)?;
                tiles.push(TileEntry { ix: ix as u32, iy: iy as u32, file, spec: sub.spec });
            }
        }
        let json = |file: &str, v: &dyn erased::Json| -> Result<Option<String>, TileError> {
            std::fs::write(dir.join(file), v.to_json()?)?;
            Ok(Some(file.to_string()))
        };
        let meta = PackMeta {
            name: name.into(),
            version: TILE_VERSION,
            frame_origin: t.frame_origin,
            cell: t.spec.cell,
            tile_intervals: ti,
            spec: t.spec,
            tiles,
            roads: json("roads.json", &w.roads)?,
            rivers: json("rivers.json", &w.rivers)?,
            power_lines: json("power_lines.json", &w.power_lines)?,
            towns: json("towns.json", &w.towns)?,
        };
        std::fs::write(dir.join("pack.json"), serde_json::to_vec_pretty(&meta)?)?;
        Ok(meta)
    }

    /// Open a pack: reads `pack.json` only.
    pub fn open(dir: &Path) -> Result<Self, TileError> {
        let meta: PackMeta = serde_json::from_slice(&std::fs::read(dir.join("pack.json"))?)?;
        Ok(Self { dir: dir.to_path_buf(), meta, cache: BTreeMap::new() })
    }

    /// Tiles touching an area, sorted by (iy, ix).
    pub fn tiles_in(&self, aoi: &Aoi) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = self.meta.tiles.iter().filter(|t| aoi.intersects(&t.spec)).map(|t| (t.ix, t.iy)).collect();
        v.sort_by_key(|&(x, y)| (y, x));
        v
    }

    /// Load every tile touching `aoi` that is not cached yet; returns how many were read.
    pub fn load_aoi(&mut self, aoi: &Aoi) -> Result<usize, TileError> {
        let mut loaded = 0;
        for key in self.tiles_in(aoi) {
            if self.cache.contains_key(&key) {
                continue;
            }
            let entry = self.meta.tiles.iter().find(|t| (t.ix, t.iy) == key).ok_or_else(|| TileError::Header("tile index".into()))?;
            let t = read_tile(&std::fs::read(self.dir.join(&entry.file))?)?;
            self.cache.insert(key, t);
            loaded += 1;
        }
        Ok(loaded)
    }

    /// Drop cached tiles outside `aoi`.
    pub fn unload_outside(&mut self, aoi: &Aoi) {
        let keep: Vec<(u32, u32)> = self.tiles_in(aoi);
        self.cache.retain(|k, _| keep.contains(k));
    }

    pub fn loaded(&self) -> Vec<(u32, u32)> {
        self.cache.keys().copied().collect()
    }

    pub fn tile(&self, ix: u32, iy: u32) -> Option<&Terrain> {
        self.cache.get(&(ix, iy))
    }

    /// Height from a loaded tile, if one covers the point.
    pub fn height(&self, e: f64, n: f64) -> Option<f64> {
        self.cache.values().find(|t| t.spec.contains(e, n)).map(|t| t.height(e, n))
    }

    /// Load and stitch the tiles covering `aoi` into one terrain grid.
    pub fn assemble(&mut self, aoi: &Aoi) -> Result<Terrain, TileError> {
        self.load_aoi(aoi)?;
        let s = self.meta.spec;
        let (i0, j0) = s.nearest_clamped(aoi.min_e, aoi.min_n);
        let (i1, j1) = s.nearest_clamped(aoi.max_e, aoi.max_n);
        let (w, h) = (i1 - i0 + 1, j1 - j0 + 1);
        let (e0, n0) = s.pos(i0, j0);
        let spec = GridSpec::new(w.max(2), h.max(2), s.cell, e0, n0);
        let ti = self.meta.tile_intervals;
        let mut out = Terrain::flat(self.meta.frame_origin, spec, 0.0);
        for j in 0..spec.height {
            for i in 0..spec.width {
                let (gi, gj) = ((i0 + i).min(s.width - 1), (j0 + j).min(s.height - 1));
                let (tx, ty) = (((gi / ti) as u32).min(self.max_ix()), ((gj / ti) as u32).min(self.max_iy()));
                let t = self.cache.get(&(tx, ty)).ok_or_else(|| TileError::Header(format!("tile {tx},{ty} not loaded")))?;
                let (li, lj) = (gi - tx as usize * ti, gj - ty as usize * ti);
                let k = spec.idx(i, j);
                out.elevation.data[k] = t.elevation.get(li, lj);
                out.land_cover.data[k] = t.land_cover.get(li, lj);
                out.soil.data[k] = t.soil.get(li, lj);
                out.wetness.data[k] = t.wetness.get(li, lj);
                out.obstacle_gap.data[k] = t.obstacle_gap.get(li, lj);
            }
        }
        Ok(out)
    }

    fn max_ix(&self) -> u32 {
        self.meta.tiles.iter().map(|t| t.ix).max().unwrap_or(0)
    }
    fn max_iy(&self) -> u32 {
        self.meta.tiles.iter().map(|t| t.iy).max().unwrap_or(0)
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, f: &Option<String>) -> Result<Option<T>, TileError> {
        match f {
            None => Ok(None),
            Some(f) => Ok(Some(serde_json::from_slice(&std::fs::read(self.dir.join(f))?)?)),
        }
    }
    pub fn roads(&self) -> Result<Option<RoadNetwork>, TileError> {
        self.read_json(&self.meta.roads)
    }
    pub fn rivers(&self) -> Result<Option<Vec<River>>, TileError> {
        self.read_json(&self.meta.rivers)
    }
    pub fn power_lines(&self) -> Result<Option<Vec<PowerLine>>, TileError> {
        self.read_json(&self.meta.power_lines)
    }
    pub fn towns(&self) -> Result<Option<Vec<Town>>, TileError> {
        self.read_json(&self.meta.towns)
    }
}

mod erased {
    /// Object-safe JSON encoding for the pack writer.
    pub trait Json {
        fn to_json(&self) -> Result<Vec<u8>, serde_json::Error>;
    }
    impl<T: serde::Serialize> Json for T {
        fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
            serde_json::to_vec(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{generate, SynthParams};

    fn world() -> SynthWorld {
        let f = crate::default_figures();
        generate(&SynthParams::new(5, 70, 50, 30.0, &f), &f)
    }

    #[test]
    fn tile_round_trip_raw_and_lz4() {
        let w = world();
        for codec in [Codec::Raw, Codec::Lz4] {
            let bytes = write_tile(&w.terrain, codec).unwrap();
            assert_eq!(read_tile(&bytes).unwrap(), w.terrain);
        }
        let raw = write_tile(&w.terrain, Codec::Raw).unwrap();
        let lz = write_tile(&w.terrain, Codec::Lz4).unwrap();
        assert!(lz.len() < raw.len());
        assert!(matches!(read_tile(b"NOPE1234"), Err(TileError::BadMagic)));
        assert!(matches!(read_tile(&raw[..raw.len() - 10]), Err(TileError::Truncated)));
        let mut bad = raw.clone();
        let hlen = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let header: TileHeader = serde_json::from_slice(&raw[8..8 + hlen]).unwrap();
        let lc = header.layers.iter().find(|l| l.name == "land_cover").unwrap();
        bad[8 + hlen + lc.offset as usize] = 200;
        assert!(matches!(read_tile(&bad), Err(TileError::BadValue { .. })));
    }

    #[test]
    fn pack_loads_lazily_and_stitches() {
        let w = world();
        let dir = std::env::temp_dir().join(format!("bgws-env-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let meta = TheatrePack::write(&dir, "test", &w, 16, Codec::Lz4).unwrap();
        assert_eq!(meta.tiles.len(), 5 * 4);
        let mut pack = TheatrePack::open(&dir).unwrap();
        assert!(pack.loaded().is_empty());
        let aoi = Aoi::around(500.0, 500.0, 100.0);
        assert_eq!(pack.load_aoi(&aoi).unwrap(), pack.tiles_in(&aoi).len());
        assert!(pack.loaded().len() < meta.tiles.len());
        assert_eq!(pack.load_aoi(&aoi).unwrap(), 0, "cached");
        let h = pack.height(500.0, 500.0).unwrap();
        assert!((h - w.terrain.height(500.0, 500.0)).abs() < 1e-4);
        // A seam point (node 16 is shared by tiles 0 and 1).
        let seam = 16.0 * 30.0;
        pack.load_aoi(&Aoi::around(seam, 100.0, 10.0)).unwrap();
        assert!((pack.height(seam, 100.0).unwrap() - w.terrain.height(seam, 100.0)).abs() < 1e-4);
        let all = Aoi { min_e: 0.0, min_n: 0.0, max_e: 69.0 * 30.0, max_n: 49.0 * 30.0 };
        let stitched = pack.assemble(&all).unwrap();
        assert_eq!(stitched.elevation, w.terrain.elevation);
        assert_eq!(stitched.land_cover, w.terrain.land_cover);
        let roads = pack.roads().unwrap().unwrap();
        assert_eq!(roads.graph.edge_count(), w.roads.graph.edge_count());
        assert_eq!(pack.towns().unwrap().unwrap().len(), w.towns.len());
        pack.unload_outside(&aoi);
        assert_eq!(pack.loaded(), pack.tiles_in(&aoi));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
