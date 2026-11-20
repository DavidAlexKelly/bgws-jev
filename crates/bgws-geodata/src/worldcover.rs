//! Land cover at 10 m from ESA WorldCover (2021, v200, CC BY 4.0): where the
//! trees are, including those OpenStreetMap does not draw as woods (gardens,
//! courtyards, tree-lined streets, scrubland grown over).
//!
//! WorldCover comes as 3° × 3° cloud-optimised GeoTIFFs on a public S3 bucket:
//! one byte a pixel (the class), 1/12000° pixels (about 9 × 5 m at 55°N), in
//! 1024-pixel tiles compressed with deflate. A build reads the file's header
//! and, of its tiles, only those under the box (cached like the other
//! sources): about 1–3 MB for a 16 × 12 km box.

use crate::mercator::BBox;
use crate::source::open_source;
use crate::GeoError;
use std::io::Read;
use std::path::Path;

pub const BASE_URL: &str = "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map";

/// WorldCover's classes.
pub const TREES: u8 = 10;
pub const SHRUBS: u8 = 20;
pub const GRASS: u8 = 30;
pub const CROPS: u8 = 40;
pub const BUILT: u8 = 50;
pub const BARE: u8 = 60;
pub const WATER: u8 = 80;
pub const WETLAND: u8 = 90;

/// Pixels a degree.
const PER_DEG: f64 = 12_000.0;

/// The classes over a box, on WorldCover's own grid (row 0 to the north).
#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    /// The north-west corner of pixel (0, 0), degrees, and a pixel's size.
    pub west: f64,
    pub north: f64,
    pub step: f64,
    pub width: usize,
    pub height: usize,
    /// The class of each pixel, row by row (0: no data).
    pub class: Vec<u8>,
}

impl Cover {
    /// The class at a point (0 off the raster).
    pub fn at(&self, lon: f64, lat: f64) -> u8 {
        let x = ((lon - self.west) / self.step).floor();
        let y = ((self.north - lat) / self.step).floor();
        if x < 0.0 || y < 0.0 || x >= self.width as f64 || y >= self.height as f64 {
            return 0;
        }
        self.class[y as usize * self.width + x as usize]
    }
}

/// The WorldCover file holding a point: named by its south-west corner, 3° steps.
pub fn file_for(lon: f64, lat: f64) -> String {
    let la = (lat / 3.0).floor() as i32 * 3;
    let lo = (lon / 3.0).floor() as i32 * 3;
    format!("ESA_WorldCover_10m_2021_v200_{}{:02}{}{:03}_Map.tif", if la < 0 { 'S' } else { 'N' }, la.abs(), if lo < 0 { 'W' } else { 'E' }, lo.abs())
}

/// The first image of a TIFF: its size, tiles and where they are.
struct Tiff {
    width: usize,
    height: usize,
    tile: usize,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    compression: u16,
    predictor: u16,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Read the first IFD of a little-endian classic TIFF from its first bytes.
fn parse(head: &[u8]) -> Option<Tiff> {
    if head.get(0..4)? != b"II*\0" {
        return None;
    }
    let ifd = le32(head, 4)? as usize;
    let n = le16(head, ifd)? as usize;
    let (mut width, mut height, mut tile, mut compression, mut predictor) = (0, 0, 0, 1, 1);
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        let tag = le16(head, e)?;
        let typ = le16(head, e + 2)?;
        let count = le32(head, e + 4)? as usize;
        let one = || if typ == 3 { le16(head, e + 8).map(|v| v as u64) } else { le32(head, e + 8).map(|v| v as u64) };
        let many = || -> Option<Vec<u64>> {
            let size = if typ == 3 { 2 } else { 4 };
            let at = if count * size <= 4 { e + 8 } else { le32(head, e + 8)? as usize };
            (0..count).map(|k| if typ == 3 { le16(head, at + k * 2).map(|v| v as u64) } else { le32(head, at + k * 4).map(|v| v as u64) }).collect()
        };
        match tag {
            256 => width = one()? as usize,
            257 => height = one()? as usize,
            259 => compression = one()? as u16,
            317 => predictor = one()? as u16,
            322 => tile = one()? as usize,
            324 => offsets = many()?,
            325 => counts = many()?,
            _ => {}
        }
    }
    (width > 0 && tile > 0 && !offsets.is_empty() && offsets.len() == counts.len()).then_some(Tiff { width, height, tile, offsets, counts, compression, predictor })
}

/// The classes over `b`. Where the box crosses files, each is read; a file
/// that is not there (open sea) leaves its part as no data.
pub fn cover(b: BBox, cache: Option<&Path>, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<Cover, GeoError> {
    let step = 1.0 / PER_DEG;
    let x0 = (b.west * PER_DEG).floor();
    let y0 = (b.north * PER_DEG).ceil();
    let width = ((b.east * PER_DEG).ceil() - x0) as usize;
    let height = (y0 - (b.south * PER_DEG).floor()) as usize;
    if width == 0 || height == 0 || width * height > 200_000_000 {
        return Err(GeoError::Format(format!("WorldCover: a box of {width} × {height} pixels")));
    }
    let mut out = Cover { west: x0 / PER_DEG, north: y0 / PER_DEG, step, width, height, class: vec![0; width * height] };
    // Each 3° file the box touches.
    let mut files = Vec::new();
    let mut la = (b.south / 3.0).floor() * 3.0;
    while la < b.north {
        let mut lo = (b.west / 3.0).floor() * 3.0;
        while lo < b.east {
            files.push((lo, la));
            lo += 3.0;
        }
        la += 3.0;
    }
    for (k, &(lo, la)) in files.iter().enumerate() {
        progress("tree cover (WorldCover)", k as f64 / files.len() as f64);
        let url = format!("{BASE_URL}/{}", file_for(lo + 0.5, la + 0.5));
        let src = open_source(&url, cache)?;
        let head = match src.read(0, 65_536) {
            Ok(h) => h,
            // No file: open sea, or past the poles.
            Err(GeoError::Http(e)) if e.contains("404") || e.contains("403") => continue,
            Err(e) => return Err(e),
        };
        let t = parse(&head).ok_or_else(|| GeoError::Format(format!("{url}: not a tiled TIFF this reads")))?;
        if t.compression != 8 && t.compression != 32946 {
            return Err(GeoError::Format(format!("{url}: compression {} (deflate only)", t.compression)));
        }
        let across = t.width.div_ceil(t.tile);
        // This file's pixel (0, 0) is its north-west corner: (lo, la + 3).
        let fx0 = (lo * PER_DEG).round() as i64;
        let fy0 = ((la + 3.0) * PER_DEG).round() as i64;
        // The box in this file's pixels.
        let bx0 = (x0 as i64 - fx0).max(0);
        let bx1 = (x0 as i64 + width as i64 - fx0).min(t.width as i64);
        let by0 = (fy0 - y0 as i64).max(0);
        let by1 = (fy0 - y0 as i64 + height as i64).min(t.height as i64);
        if bx0 >= bx1 || by0 >= by1 {
            continue;
        }
        let ts = t.tile as i64;
        for ty in by0 / ts..=(by1 - 1) / ts {
            for tx in bx0 / ts..=(bx1 - 1) / ts {
                let i = ty as usize * across + tx as usize;
                let (Some(&off), Some(&len)) = (t.offsets.get(i), t.counts.get(i)) else { continue };
                if len == 0 {
                    continue;
                }
                let raw = src.read(off, len)?;
                let mut px = Vec::with_capacity(t.tile * t.tile);
                flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut px).map_err(|e| GeoError::Format(format!("{url}: tile {i}: {e}")))?;
                if px.len() < t.tile * t.tile {
                    return Err(GeoError::Format(format!("{url}: tile {i} is short")));
                }
                if t.predictor == 2 {
                    for row in px.chunks_mut(t.tile) {
                        for c in 1..row.len() {
                            row[c] = row[c].wrapping_add(row[c - 1]);
                        }
                    }
                }
                // Copy the part under the box.
                for py in 0..ts {
                    let fy = ty * ts + py;
                    if fy < by0 || fy >= by1 {
                        continue;
                    }
                    let oy = (fy - fy0 + y0 as i64) as usize;
                    for pxx in 0..ts {
                        let fx = tx * ts + pxx;
                        if fx < bx0 || fx >= bx1 {
                            continue;
                        }
                        let ox = (fx + fx0 - x0 as i64) as usize;
                        out.class[oy * width + ox] = px[(py * ts + pxx) as usize];
                    }
                }
            }
        }
    }
    progress("tree cover (WorldCover)", 1.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_by_their_south_west_corner() {
        assert_eq!(file_for(20.5, 54.7), "ESA_WorldCover_10m_2021_v200_N54E018_Map.tif");
        assert_eq!(file_for(-3.2, 40.4), "ESA_WorldCover_10m_2021_v200_N39W006_Map.tif");
        assert_eq!(file_for(151.2, -33.9), "ESA_WorldCover_10m_2021_v200_S36E150_Map.tif");
    }

    #[test]
    fn reads_a_tiled_tiff_header() {
        // A minimal header: 2048 × 1024, 1024-pixel tiles, deflate, two tiles.
        let mut h = b"II*\0".to_vec();
        h.extend(8u32.to_le_bytes());
        let entries: [(u16, u16, u32, u32); 6] = [(256, 3, 1, 2048), (257, 3, 1, 1024), (259, 3, 1, 8), (322, 3, 1, 1024), (324, 4, 2, 200), (325, 4, 2, 208)];
        h.extend((entries.len() as u16).to_le_bytes());
        for (tag, typ, n, v) in entries {
            h.extend(tag.to_le_bytes());
            h.extend(typ.to_le_bytes());
            h.extend(n.to_le_bytes());
            h.extend(v.to_le_bytes());
        }
        h.resize(200, 0);
        h.extend(1000u32.to_le_bytes());
        h.extend(5000u32.to_le_bytes());
        h.extend(77u32.to_le_bytes());
        h.extend(88u32.to_le_bytes());
        let t = parse(&h).unwrap();
        assert_eq!((t.width, t.height, t.tile, t.compression), (2048, 1024, 1024, 8));
        assert_eq!(t.offsets, vec![1000, 5000]);
        assert_eq!(t.counts, vec![77, 88]);
    }
}
