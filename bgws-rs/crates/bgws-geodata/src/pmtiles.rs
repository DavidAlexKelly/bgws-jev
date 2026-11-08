//! PMTiles v3 (https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md):
//! a 127-byte header, a root directory, leaf directories and tile data, each
//! found by byte range. Reading a box touches the header, a few directories
//! and its tiles, never the whole archive. A writer builds the small local
//! archives a theatre pack carries for the viewer.

use crate::source::RangeSource;
use crate::GeoError;
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

pub const HEADER_LEN: usize = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Unknown,
    None,
    Gzip,
    Brotli,
    Zstd,
}

impl Compression {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Compression::None,
            2 => Compression::Gzip,
            3 => Compression::Brotli,
            4 => Compression::Zstd,
            _ => Compression::Unknown,
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            Compression::Unknown => 0,
            Compression::None => 1,
            Compression::Gzip => 2,
            Compression::Brotli => 3,
            Compression::Zstd => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileType {
    Unknown,
    Mvt,
    Png,
    Jpeg,
    Webp,
    Avif,
}

impl TileType {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => TileType::Mvt,
            2 => TileType::Png,
            3 => TileType::Jpeg,
            4 => TileType::Webp,
            5 => TileType::Avif,
            _ => TileType::Unknown,
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            TileType::Unknown => 0,
            TileType::Mvt => 1,
            TileType::Png => 2,
            TileType::Jpeg => 3,
            TileType::Webp => 4,
            TileType::Avif => 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub root_offset: u64,
    pub root_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub leaf_offset: u64,
    pub leaf_length: u64,
    pub data_offset: u64,
    pub data_length: u64,
    pub addressed_tiles: u64,
    pub tile_entries: u64,
    pub tile_contents: u64,
    pub clustered: bool,
    pub internal_compression: Compression,
    pub tile_compression: Compression,
    pub tile_type: TileType,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// Degrees.
    pub bounds: [f64; 4],
    pub center_zoom: u8,
    pub center: [f64; 2],
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Self, GeoError> {
        if b.len() < HEADER_LEN || &b[0..7] != b"PMTiles" {
            return Err(GeoError::Format("not a PMTiles archive".into()));
        }
        if b[7] != 3 {
            return Err(GeoError::Format(format!("PMTiles version {} (only 3 is read)", b[7])));
        }
        let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        let e7 = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes")) as f64 / 1e7;
        Ok(Header {
            root_offset: u(8),
            root_length: u(16),
            metadata_offset: u(24),
            metadata_length: u(32),
            leaf_offset: u(40),
            leaf_length: u(48),
            data_offset: u(56),
            data_length: u(64),
            addressed_tiles: u(72),
            tile_entries: u(80),
            tile_contents: u(88),
            clustered: b[96] == 1,
            internal_compression: Compression::from_u8(b[97]),
            tile_compression: Compression::from_u8(b[98]),
            tile_type: TileType::from_u8(b[99]),
            min_zoom: b[100],
            max_zoom: b[101],
            bounds: [e7(102), e7(106), e7(110), e7(114)],
            center_zoom: b[118],
            center: [e7(119), e7(123)],
        })
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..7].copy_from_slice(b"PMTiles");
        b[7] = 3;
        for (i, v) in [
            self.root_offset,
            self.root_length,
            self.metadata_offset,
            self.metadata_length,
            self.leaf_offset,
            self.leaf_length,
            self.data_offset,
            self.data_length,
            self.addressed_tiles,
            self.tile_entries,
            self.tile_contents,
        ]
        .iter()
        .enumerate()
        {
            b[8 + 8 * i..16 + 8 * i].copy_from_slice(&v.to_le_bytes());
        }
        b[96] = self.clustered as u8;
        b[97] = self.internal_compression.to_u8();
        b[98] = self.tile_compression.to_u8();
        b[99] = self.tile_type.to_u8();
        b[100] = self.min_zoom;
        b[101] = self.max_zoom;
        let e7 = |v: f64| ((v * 1e7).round() as i32).to_le_bytes();
        b[102..106].copy_from_slice(&e7(self.bounds[0]));
        b[106..110].copy_from_slice(&e7(self.bounds[1]));
        b[110..114].copy_from_slice(&e7(self.bounds[2]));
        b[114..118].copy_from_slice(&e7(self.bounds[3]));
        b[118] = self.center_zoom;
        b[119..123].copy_from_slice(&e7(self.center[0]));
        b[123..127].copy_from_slice(&e7(self.center[1]));
        b
    }
}

/// One directory entry: a run of tiles (`run_length` > 0) or a leaf directory (0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub tile_id: u64,
    pub offset: u64,
    pub length: u32,
    pub run_length: u32,
}

/// The tile id of (z, x, y): tiles of lower zooms first, then a Hilbert curve.
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let z = z as u32;
    let acc: u64 = ((1u64 << (2 * z)) - 1) / 3;
    let n = 1u64 << z;
    let (mut x, mut y) = (x as u64, y as u64);
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(x & s > 0);
        let ry = u64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d
}

fn varint(b: &[u8], pos: &mut usize) -> Result<u64, GeoError> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*pos).ok_or_else(|| GeoError::Format("directory ends inside a varint".into()))?;
        *pos += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err(GeoError::Format("varint too long".into()));
        }
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub fn decompress(c: Compression, b: Vec<u8>) -> Result<Vec<u8>, GeoError> {
    match c {
        Compression::None | Compression::Unknown => Ok(b),
        Compression::Gzip => {
            let mut out = Vec::with_capacity(b.len() * 3);
            GzDecoder::new(&b[..]).read_to_end(&mut out).map_err(|e| GeoError::Format(format!("gzip: {e}")))?;
            Ok(out)
        }
        other => Err(GeoError::Format(format!("{other:?} compression is not supported"))),
    }
}

pub fn gzip(b: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(b).expect("in-memory gzip");
    e.finish().expect("in-memory gzip")
}

pub fn parse_directory(b: &[u8]) -> Result<Vec<Entry>, GeoError> {
    let mut p = 0;
    let n = varint(b, &mut p)? as usize;
    let mut es = vec![Entry { tile_id: 0, offset: 0, length: 0, run_length: 0 }; n];
    let mut last = 0u64;
    for e in es.iter_mut() {
        last += varint(b, &mut p)?;
        e.tile_id = last;
    }
    for e in es.iter_mut() {
        e.run_length = varint(b, &mut p)? as u32;
    }
    for e in es.iter_mut() {
        e.length = varint(b, &mut p)? as u32;
    }
    for i in 0..n {
        let v = varint(b, &mut p)?;
        es[i].offset = if v == 0 && i > 0 { es[i - 1].offset + es[i - 1].length as u64 } else { v.saturating_sub(1) };
    }
    Ok(es)
}

pub fn write_directory(es: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, es.len() as u64);
    let mut last = 0;
    for e in es {
        put_varint(&mut out, e.tile_id - last);
        last = e.tile_id;
    }
    for e in es {
        put_varint(&mut out, e.run_length as u64);
    }
    for e in es {
        put_varint(&mut out, e.length as u64);
    }
    for (i, e) in es.iter().enumerate() {
        if i > 0 && e.offset == es[i - 1].offset + es[i - 1].length as u64 {
            put_varint(&mut out, 0);
        } else {
            put_varint(&mut out, e.offset + 1);
        }
    }
    out
}

fn find(es: &[Entry], id: u64) -> Option<Entry> {
    // The last entry with tile_id ≤ id.
    let i = es.partition_point(|e| e.tile_id <= id);
    let e = *es.get(i.checked_sub(1)?)?;
    if e.run_length == 0 || id < e.tile_id + e.run_length as u64 {
        Some(e)
    } else {
        None
    }
}

/// An archive open for reading tiles.
pub struct Reader {
    src: Box<dyn RangeSource>,
    pub header: Header,
    root: Vec<Entry>,
    leaves: Mutex<HashMap<(u64, u32), Arc<Vec<Entry>>>>,
}

impl Reader {
    pub fn open(src: Box<dyn RangeSource>) -> Result<Self, GeoError> {
        let header = Header::parse(&src.read(0, HEADER_LEN as u64)?)?;
        let root_bytes = src.read(header.root_offset, header.root_length)?;
        let root = parse_directory(&decompress(header.internal_compression, root_bytes)?)?;
        Ok(Self { src, header, root, leaves: Mutex::new(HashMap::new()) })
    }

    pub fn name(&self) -> &str {
        self.src.name()
    }

    pub fn metadata(&self) -> Result<serde_json::Value, GeoError> {
        if self.header.metadata_length == 0 {
            return Ok(serde_json::Value::Null);
        }
        let b = decompress(self.header.internal_compression, self.src.read(self.header.metadata_offset, self.header.metadata_length)?)?;
        serde_json::from_slice(&b).map_err(|e| GeoError::Format(format!("metadata: {e}")))
    }

    fn leaf(&self, offset: u64, length: u32) -> Result<Arc<Vec<Entry>>, GeoError> {
        if let Some(d) = self.leaves.lock().map_err(|_| GeoError::Io("lock".into()))?.get(&(offset, length)) {
            return Ok(d.clone());
        }
        let b = self.src.read(self.header.leaf_offset + offset, length as u64)?;
        let d = Arc::new(parse_directory(&decompress(self.header.internal_compression, b)?)?);
        self.leaves.lock().map_err(|_| GeoError::Io("lock".into()))?.insert((offset, length), d.clone());
        Ok(d)
    }

    /// Where a tile's bytes are, if the archive has it.
    pub fn locate(&self, z: u8, x: u32, y: u32) -> Result<Option<(u64, u32)>, GeoError> {
        let id = tile_id(z, x, y);
        let mut dir: Arc<Vec<Entry>> = Arc::new(self.root.clone());
        // Directories nest at most a few levels deep.
        for _ in 0..4 {
            let Some(e) = find(&dir, id) else { return Ok(None) };
            if e.run_length > 0 {
                return Ok(Some((self.header.data_offset + e.offset, e.length)));
            }
            dir = self.leaf(e.offset, e.length)?;
        }
        Err(GeoError::Format("directories nest too deep".into()))
    }

    /// A tile, decompressed (vector tiles are usually gzipped; images are not).
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, GeoError> {
        match self.locate(z, x, y)? {
            None => Ok(None),
            Some((off, len)) => Ok(Some(decompress(self.header.tile_compression, self.src.read(off, len as u64)?)?)),
        }
    }

    /// A tile's bytes as stored, for copying into another archive.
    pub fn tile_raw(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, GeoError> {
        match self.locate(z, x, y)? {
            None => Ok(None),
            Some((off, len)) => Ok(Some(self.src.read(off, len as u64)?)),
        }
    }
}

/// Write an archive from tiles already in their stored encoding (the same
/// compression for all). Identical tiles (sea, empty land) are stored once.
pub fn write_archive(
    path: &std::path::Path,
    mut tiles: Vec<((u8, u32, u32), Vec<u8>)>,
    tile_type: TileType,
    tile_compression: Compression,
    bounds: [f64; 4],
    metadata: &serde_json::Value,
) -> Result<(), GeoError> {
    tiles.sort_by_key(|((z, x, y), _)| tile_id(*z, *x, *y));
    let mut data: Vec<u8> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: HashMap<u64, (u64, u32)> = HashMap::new();
    let (mut min_z, mut max_z) = (u8::MAX, 0);
    let mut contents = 0u64;
    for ((z, x, y), b) in &tiles {
        min_z = min_z.min(*z);
        max_z = max_z.max(*z);
        let id = tile_id(*z, *x, *y);
        let h = bgws_core::rng::fnv1a(b) ^ (b.len() as u64).rotate_left(32);
        let (offset, length) = match seen.get(&h) {
            Some(&loc) => loc,
            None => {
                let loc = (data.len() as u64, b.len() as u32);
                data.extend_from_slice(b);
                seen.insert(h, loc);
                contents += 1;
                loc
            }
        };
        // Runs of the same content at consecutive ids share an entry.
        if let Some(last) = entries.last_mut() {
            if last.offset == offset && last.length == length && last.tile_id + last.run_length as u64 == id {
                last.run_length += 1;
                continue;
            }
        }
        entries.push(Entry { tile_id: id, offset, length, run_length: 1 });
    }
    // A root directory that fits the first 16 KiB, with leaves if it does not.
    let (root, leaves) = {
        let whole = gzip(&write_directory(&entries));
        if whole.len() + HEADER_LEN <= 16_384 {
            (whole, Vec::new())
        } else {
            let mut leaf_size = 4096;
            loop {
                let mut leaves = Vec::new();
                let mut root_entries = Vec::new();
                for chunk in entries.chunks(leaf_size) {
                    let b = gzip(&write_directory(chunk));
                    root_entries.push(Entry { tile_id: chunk[0].tile_id, offset: leaves.len() as u64, length: b.len() as u32, run_length: 0 });
                    leaves.extend_from_slice(&b);
                }
                let root = gzip(&write_directory(&root_entries));
                if root.len() + HEADER_LEN <= 16_384 {
                    break (root, leaves);
                }
                leaf_size *= 2;
            }
        }
    };
    let meta = gzip(serde_json::to_string(metadata).unwrap_or_default().as_bytes());
    let root_offset = HEADER_LEN as u64;
    let metadata_offset = root_offset + root.len() as u64;
    let leaf_offset = metadata_offset + meta.len() as u64;
    let data_offset = leaf_offset + leaves.len() as u64;
    let header = Header {
        root_offset,
        root_length: root.len() as u64,
        metadata_offset,
        metadata_length: meta.len() as u64,
        leaf_offset,
        leaf_length: leaves.len() as u64,
        data_offset,
        data_length: data.len() as u64,
        addressed_tiles: tiles.len() as u64,
        tile_entries: entries.len() as u64,
        tile_contents: contents,
        clustered: true,
        internal_compression: Compression::Gzip,
        tile_compression,
        tile_type,
        min_zoom: if tiles.is_empty() { 0 } else { min_z },
        max_zoom: max_z,
        bounds,
        center_zoom: max_z.saturating_sub(2),
        center: [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0],
    };
    let mut out = Vec::with_capacity(data_offset as usize + data.len());
    out.extend_from_slice(&header.to_bytes());
    out.extend_from_slice(&root);
    out.extend_from_slice(&meta);
    out.extend_from_slice(&leaves);
    out.extend_from_slice(&data);
    std::fs::write(path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_ids_follow_the_spec() {
        assert_eq!(tile_id(0, 0, 0), 0);
        // Zoom 1, in Hilbert order.
        assert_eq!(tile_id(1, 0, 0), 1);
        assert_eq!(tile_id(1, 0, 1), 2);
        assert_eq!(tile_id(1, 1, 1), 3);
        assert_eq!(tile_id(1, 1, 0), 4);
        assert_eq!(tile_id(2, 0, 0), 5);
        // The last tile of zoom 12 is just before the first of zoom 13.
        let first13 = tile_id(13, 0, 0);
        assert_eq!(first13, (4u64.pow(13) - 1) / 3);
        let mut ids: Vec<u64> = (0..4u32).flat_map(|x| (0..4u32).map(move |y| tile_id(2, x, y))).collect();
        ids.sort_unstable();
        assert_eq!(ids, (5..21).collect::<Vec<_>>(), "zoom 2 covers 5..21 once each");
    }

    #[test]
    fn directories_round_trip() {
        let es = vec![
            Entry { tile_id: 0, offset: 0, length: 10, run_length: 1 },
            Entry { tile_id: 3, offset: 10, length: 5, run_length: 2 },
            Entry { tile_id: 9, offset: 0, length: 10, run_length: 1 },
        ];
        assert_eq!(parse_directory(&write_directory(&es)).unwrap(), es);
        assert_eq!(find(&es, 4).unwrap().tile_id, 3);
        assert!(find(&es, 5).is_none());
    }

    #[test]
    fn an_archive_written_reads_back() {
        let dir = std::env::temp_dir().join(format!("bgws-pmtiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.pmtiles");
        // Enough tiles to need leaf directories.
        let mut tiles = Vec::new();
        for x in 0..256u32 {
            for y in 0..64u32 {
                tiles.push(((8u8, x, y), format!("tile {x} {y} {}", (x * y) % 7).into_bytes()));
            }
        }
        tiles.push(((0, 0, 0), b"world".to_vec()));
        write_archive(&p, tiles, TileType::Mvt, Compression::None, [-180.0, -85.0, 180.0, 85.0], &serde_json::json!({ "name": "t" })).unwrap();
        let r = Reader::open(Box::new(crate::source::FileSource::open(&p).unwrap())).unwrap();
        assert_eq!((r.header.min_zoom, r.header.max_zoom), (0, 8));
        assert_eq!(r.tile(0, 0, 0).unwrap().unwrap(), b"world");
        assert_eq!(r.tile(8, 200, 33).unwrap().unwrap(), b"tile 200 33 6");
        assert!(r.tile(8, 200, 100).unwrap().is_none());
        assert_eq!(r.metadata().unwrap()["name"], "t");
    }
}
