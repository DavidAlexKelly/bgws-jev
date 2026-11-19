//! Building footprints from Overture Maps (`docs.overturemaps.org`), for the
//! buildings OpenStreetMap does not have yet: Overture's buildings theme
//! conflates OpenStreetMap with machine-learned footprints from satellite
//! imagery (Microsoft's, Google's) and Esri's community maps, ODbL.
//!
//! The theme is about 500 GeoParquet files on a public S3 bucket, each
//! spatially compact. A build reads:
//! - the bucket's list of files, and the last 64 KB of each, where its
//!   GeoParquet `geo` metadata gives its bounding box (kept as an index, once
//!   per release, in the cache);
//! - the footer of each file that touches the box, whose row groups carry
//!   statistics on each building's `bbox`;
//! - and only the columns it needs (geometry, height, floors, sources) of
//!   the row groups that touch the box, one range request per column chunk.
//!
//! For a 16 × 12 km box that is one file, a 1.4 MB footer and a few tens of MB.

use crate::mercator::BBox;
use crate::source::{open_source, RangeSource};
use crate::GeoError;
use bytes::Bytes;
use parquet::file::reader::{ChunkReader, FileReader, Length};
use parquet::file::serialized_reader::SerializedFileReader;
use parquet::file::statistics::Statistics;
use parquet::record::{Field, Row};
use parquet::schema::types::Type;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Overture's public bucket (S3, us-west-2), read anonymously.
pub const BUCKET: &str = "https://overturemaps-us-west-2.s3.amazonaws.com";
const THEME: &str = "theme=buildings/type=building/";

/// A building's footprint and what Overture knows of its height.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Outer ring then holes, (lon, lat) degrees, closed or not.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Height, m, where a source gives one.
    pub height: Option<f64>,
    pub floors: Option<i32>,
    /// The dataset the footprint came from: "OpenStreetMap",
    /// "Microsoft ML Buildings", "Google Open Buildings", "Esri Community Maps", …
    pub dataset: String,
}

/// One file of the theme, with its bounding box where its metadata gives one.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileEntry {
    key: String,
    size: u64,
    bbox: Option<[f64; 4]>,
}

fn client() -> Result<reqwest::blocking::Client, GeoError> {
    reqwest::blocking::Client::builder().user_agent(concat!("bgws-geodata/", env!("CARGO_PKG_VERSION"))).timeout(std::time::Duration::from_secs(120)).build().map_err(|e| GeoError::Http(e.to_string()))
}

fn get_text(c: &reqwest::blocking::Client, url: &str) -> Result<String, GeoError> {
    let mut last = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500 << attempt));
        }
        match c.get(url).send().and_then(|r| r.error_for_status()).and_then(|r| r.text()) {
            Ok(t) => return Ok(t),
            Err(e) => last = e.to_string(),
        }
    }
    Err(GeoError::Http(format!("{url}: {last}")))
}

/// Every `<tag>…</tag>` in an S3 listing.
fn tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        let s = &rest[a + open.len()..];
        let Some(b) = s.find(&close) else { break };
        out.push(&s[..b]);
        rest = &s[b + close.len()..];
    }
    out
}

/// The newest release on the bucket (`2026-11-18.0`, …).
pub fn latest_release() -> Result<String, GeoError> {
    let xml = get_text(&client()?, &format!("{BUCKET}/?list-type=2&prefix=release/&delimiter=/"))?;
    tags(&xml, "Prefix").into_iter().filter_map(|p| p.strip_prefix("release/")?.strip_suffix('/').map(str::to_string)).filter(|r| !r.is_empty()).max().ok_or_else(|| GeoError::Format("Overture: no releases listed".into()))
}

/// The file's bounding box from its GeoParquet metadata near the end of the
/// file (`"bbox": [xmin, ymin, xmax, ymax]`), if the tail holds it.
fn tail_bbox(tail: &[u8]) -> Option<[f64; 4]> {
    let s = String::from_utf8_lossy(tail);
    let at = s.rfind("\"bbox\": [").or_else(|| s.rfind("\"bbox\":["))?;
    let rest = &s[at..];
    let open = rest.find('[')?;
    let close = rest.find(']')?;
    let v: Vec<f64> = rest[open + 1..close].split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

/// The release's files with their boxes: from the cache, else listed and
/// each file's tail read (about 30 MB once per release).
fn index(release: &str, cache: Option<&Path>, pool: &rayon::ThreadPool, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<Vec<FileEntry>, GeoError> {
    let file = cache.map(|c| c.join("overture").join(release).join("buildings-index.json"));
    if let Some(f) = &file {
        if let Some(v) = std::fs::read(f).ok().and_then(|b| serde_json::from_slice::<Vec<FileEntry>>(&b).ok()) {
            return Ok(v);
        }
    }
    let c = client()?;
    let prefix = format!("release/{release}/{THEME}");
    let mut files: Vec<(String, u64)> = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut url = format!("{BUCKET}/?list-type=2&prefix={prefix}");
        if let Some(t) = &token {
            url.push_str(&format!("&continuation-token={}", t.replace('+', "%2B").replace('/', "%2F").replace('=', "%3D")));
        }
        let xml = get_text(&c, &url)?;
        for item in tags(&xml, "Contents") {
            let (Some(k), Some(s)) = (tags(item, "Key").first().copied(), tags(item, "Size").first().and_then(|s| s.parse::<u64>().ok())) else { continue };
            if k.ends_with(".parquet") {
                files.push((k.to_string(), s));
            }
        }
        token = tags(&xml, "NextContinuationToken").first().map(|t| t.to_string());
        if token.is_none() {
            break;
        }
    }
    if files.is_empty() {
        return Err(GeoError::Format(format!("Overture {release}: no building files")));
    }
    let done = std::sync::atomic::AtomicUsize::new(0);
    let entries: Vec<FileEntry> = pool.install(|| {
        files
            .par_iter()
            .map(|(key, size)| {
                let src = open_source(&format!("{BUCKET}/{key}"), None)?;
                let n = (*size).min(65_536);
                let tail = src.read(size - n, n)?;
                let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress("buildings (Overture): index", k as f64 / files.len() as f64);
                Ok(FileEntry { key: key.clone(), size: *size, bbox: tail_bbox(&tail) })
            })
            .collect::<Result<_, GeoError>>()
    })?;
    if let Some(f) = &file {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, serde_json::to_vec(&entries).unwrap_or_default());
    }
    Ok(entries)
}

/// A remote Parquet file for the `parquet` crate: the ranges it asks for,
/// from column chunks fetched whole beforehand where they are held.
struct Remote {
    src: Box<dyn RangeSource>,
    len: u64,
    held: Mutex<Vec<(u64, Bytes)>>,
}

impl Remote {
    fn hold(&self, start: u64, len: u64) -> Result<(), GeoError> {
        let b = Bytes::from(self.src.read(start, len)?);
        self.held.lock().map_err(|_| GeoError::Io("lock poisoned".into()))?.push((start, b));
        Ok(())
    }
    fn slice(&self, start: u64, len: Option<usize>) -> parquet::errors::Result<Bytes> {
        let want = len.unwrap_or(1) as u64;
        if let Ok(h) = self.held.lock() {
            if let Some((s, b)) = h.iter().find(|(s, b)| start >= *s && start + want <= *s + b.len() as u64) {
                let from = (start - s) as usize;
                let to = len.map(|l| from + l).unwrap_or(b.len());
                return Ok(b.slice(from..to));
            }
        }
        let n = (len.map(|l| l as u64).unwrap_or(65_536)).min(self.len.saturating_sub(start));
        self.src.read(start, n).map(Bytes::from).map_err(|e| parquet::errors::ParquetError::External(Box::new(e)))
    }
}

/// The file as the reader holds it, shared with the prefetch.
struct Shared(Arc<Remote>);

impl Length for Shared {
    fn len(&self) -> u64 {
        self.0.len
    }
}

impl ChunkReader for Shared {
    type T = std::io::Cursor<Bytes>;
    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        Ok(std::io::Cursor::new(self.0.slice(start, None)?))
    }
    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        self.0.slice(start, Some(length))
    }
}

fn pq(e: parquet::errors::ParquetError) -> GeoError {
    GeoError::Format(format!("Overture parquet: {e}"))
}

fn stat_f64(s: &Statistics, min: bool) -> Option<f64> {
    match s {
        Statistics::Float(v) => (if min { v.min_opt() } else { v.max_opt() }).map(|x| *x as f64),
        Statistics::Double(v) => (if min { v.min_opt() } else { v.max_opt() }).copied(),
        _ => None,
    }
}

/// The columns read: the rest of a building's attributes are left on the server.
const COLUMNS: [&str; 6] = ["bbox", "geometry", "height", "num_floors", "is_underground", "sources"];

fn read_file(e: &FileEntry, b: BBox, cache: Option<&Path>, pool: &rayon::ThreadPool) -> Result<Vec<Building>, GeoError> {
    let src = open_source(&format!("{BUCKET}/{}", e.key), cache)?;
    let remote = Arc::new(Remote { src, len: e.size, held: Mutex::new(Vec::new()) });
    let reader = SerializedFileReader::new(Shared(remote.clone())).map_err(pq)?;
    let meta = reader.metadata();
    let schema = meta.file_metadata().schema_descr();
    // Leaf columns of bbox.{xmin,xmax,ymin,ymax}, for the row groups' statistics.
    let leaf = |path: &str| (0..schema.num_columns()).find(|&i| schema.column(i).path().string() == path);
    let (Some(xmin), Some(xmax), Some(ymin), Some(ymax)) = (leaf("bbox.xmin"), leaf("bbox.xmax"), leaf("bbox.ymin"), leaf("bbox.ymax")) else {
        return Err(GeoError::Format(format!("Overture {}: no bbox columns", e.key)));
    };
    let groups: Vec<usize> = (0..meta.num_row_groups())
        .filter(|&g| {
            let rg = meta.row_group(g);
            let st = |c: usize, min: bool| rg.column(c).statistics().and_then(|s| stat_f64(s, min));
            match (st(xmin, true), st(xmax, false), st(ymin, true), st(ymax, false)) {
                (Some(x0), Some(x1), Some(y0), Some(y1)) => x0 <= b.east && x1 >= b.west && y0 <= b.north && y1 >= b.south,
                _ => true,
            }
        })
        .collect();
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    // The column chunks wanted, fetched whole and in parallel.
    let wanted = |path: &str| COLUMNS.iter().any(|c| path == *c || path.starts_with(&format!("{c}.")));
    let chunks: Vec<(u64, u64)> = groups.iter().flat_map(|&g| meta.row_group(g).columns().iter().filter(|c| wanted(&c.column_path().string())).map(|c| c.byte_range()).collect::<Vec<_>>()).collect();
    pool.install(|| chunks.par_iter().try_for_each(|&(s, l)| remote.hold(s, l)))?;

    let root = meta.file_metadata().schema();
    let fields: Vec<_> = root.get_fields().iter().filter(|f| COLUMNS.contains(&f.name())).cloned().collect();
    let projection = Type::group_type_builder("schema").with_fields(fields).build().map_err(pq)?;
    let mut out = Vec::new();
    for g in groups {
        let rg = reader.get_row_group(g).map_err(pq)?;
        for row in rg.get_row_iter(Some(projection.clone())).map_err(pq)? {
            let row = row.map_err(pq)?;
            if let Some(mut v) = building(&row, b) {
                out.append(&mut v);
            }
        }
    }
    Ok(out)
}

/// A row as buildings (a multipolygon is several), if it touches the box and stands above ground.
fn building(row: &Row, b: BBox) -> Option<Vec<Building>> {
    let (mut geom, mut height, mut floors, mut dataset) = (None, None, None, String::new());
    for (name, f) in row.get_column_iter() {
        match (name.as_str(), f) {
            ("bbox", Field::Group(bb)) => {
                let mut v = [0f64; 4];
                for (n, x) in bb.get_column_iter() {
                    let x = match x {
                        Field::Float(x) => *x as f64,
                        Field::Double(x) => *x,
                        _ => continue,
                    };
                    match n.as_str() {
                        "xmin" => v[0] = x,
                        "xmax" => v[1] = x,
                        "ymin" => v[2] = x,
                        "ymax" => v[3] = x,
                        _ => {}
                    }
                }
                if v[0] > b.east || v[1] < b.west || v[2] > b.north || v[3] < b.south {
                    return None;
                }
            }
            ("geometry", Field::Bytes(g)) => geom = Some(g.data().to_vec()),
            ("height", Field::Double(h)) => height = Some(*h),
            ("height", Field::Float(h)) => height = Some(*h as f64),
            ("num_floors", Field::Int(n)) => floors = Some(*n),
            ("is_underground", Field::Bool(true)) => return None,
            ("sources", Field::ListInternal(l)) => {
                // The geometry's source: the entry for the whole feature (property "").
                for s in l.elements() {
                    let Field::Group(s) = s else { continue };
                    let (mut prop, mut ds) = (None, None);
                    for (n, x) in s.get_column_iter() {
                        match (n.as_str(), x) {
                            ("property", Field::Str(p)) => prop = Some(p.clone()),
                            ("dataset", Field::Str(d)) => ds = Some(d.clone()),
                            _ => {}
                        }
                    }
                    if let Some(d) = ds {
                        if dataset.is_empty() || prop.as_deref().is_none_or(str::is_empty) {
                            dataset = d;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let polys = wkb(&geom?)?;
    Some(polys.into_iter().map(|rings| Building { rings, height: height.filter(|h| *h > 0.0), floors: floors.filter(|n| *n > 0), dataset: dataset.clone() }).collect())
}

/// Polygons (rings of (x, y)) from WKB: Polygon and MultiPolygon, 2D or with Z/M.
pub fn wkb(b: &[u8]) -> Option<Vec<Vec<Vec<[f64; 2]>>>> {
    struct R<'a> {
        b: &'a [u8],
        p: usize,
    }
    impl R<'_> {
        fn u8(&mut self) -> Option<u8> {
            let v = *self.b.get(self.p)?;
            self.p += 1;
            Some(v)
        }
        fn u32(&mut self, le: bool) -> Option<u32> {
            let s: [u8; 4] = self.b.get(self.p..self.p + 4)?.try_into().ok()?;
            self.p += 4;
            Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
        }
        fn f64(&mut self, le: bool) -> Option<f64> {
            let s: [u8; 8] = self.b.get(self.p..self.p + 8)?.try_into().ok()?;
            self.p += 8;
            Some(if le { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
        }
        /// A geometry's header: byte order, base type, coordinates per point.
        fn head(&mut self) -> Option<(bool, u32, usize)> {
            let le = self.u8()? == 1;
            let raw = self.u32(le)?;
            let ewkb = (raw & 0x8000_0000 != 0) as usize + (raw & 0x4000_0000 != 0) as usize;
            let t = raw & 0x0fff_ffff;
            let iso = match t / 1000 {
                1 | 2 => 1,
                3 => 2,
                _ => 0,
            };
            if raw & 0x2000_0000 != 0 {
                self.u32(le)?; // EWKB SRID
            }
            Some((le, t % 1000, 2 + ewkb + iso))
        }
        fn polygon(&mut self, le: bool, dims: usize) -> Option<Vec<Vec<[f64; 2]>>> {
            let rings = self.u32(le)? as usize;
            let mut out = Vec::with_capacity(rings.min(64));
            for _ in 0..rings {
                let n = self.u32(le)? as usize;
                if n > self.b.len() / 16 + 1 {
                    return None;
                }
                let mut r = Vec::with_capacity(n);
                for _ in 0..n {
                    let x = self.f64(le)?;
                    let y = self.f64(le)?;
                    for _ in 2..dims {
                        self.f64(le)?;
                    }
                    r.push([x, y]);
                }
                out.push(r);
            }
            Some(out)
        }
    }
    let mut r = R { b, p: 0 };
    let (le, t, dims) = r.head()?;
    match t {
        3 => Some(vec![r.polygon(le, dims)?]),
        6 => {
            let n = r.u32(le)? as usize;
            let mut out = Vec::new();
            for _ in 0..n.min(10_000) {
                let (le, t, dims) = r.head()?;
                if t != 3 {
                    return None;
                }
                out.push(r.polygon(le, dims)?);
            }
            Some(out)
        }
        _ => None,
    }
}

/// Every Overture building touching `b` (from every source), and the release read.
pub fn buildings(b: BBox, release: Option<&str>, cache: Option<&Path>, threads: usize, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<(Vec<Building>, String), GeoError> {
    let release = match release {
        Some(r) => r.to_string(),
        None => latest_release()?,
    };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build().map_err(|e| GeoError::Io(e.to_string()))?;
    progress("buildings (Overture): index", 0.0);
    let files = index(&release, cache, &pool, progress)?;
    let touching: Vec<&FileEntry> = files.iter().filter(|f| f.bbox.is_none_or(|[x0, y0, x1, y1]| x0 <= b.east && x1 >= b.west && y0 <= b.north && y1 >= b.south)).collect();
    let cache: Option<PathBuf> = cache.map(|c| c.join("overture").join(&release));
    let mut out = Vec::new();
    for (k, f) in touching.iter().enumerate() {
        progress("buildings (Overture): footprints", k as f64 / touching.len() as f64);
        out.append(&mut read_file(f, b, cache.as_deref(), &pool)?);
    }
    progress("buildings (Overture): footprints", 1.0);
    Ok((out, release))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_wkb(le: bool, pts: &[[f64; 2]]) -> Vec<u8> {
        let u = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let f = |v: f64| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut b = vec![u8::from(le)];
        b.extend(u(3));
        b.extend(u(1));
        b.extend(u(pts.len() as u32));
        for p in pts {
            b.extend(f(p[0]));
            b.extend(f(p[1]));
        }
        b
    }

    #[test]
    fn reads_wkb_polygons_and_multipolygons_and_file_boxes() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        assert_eq!(wkb(&ring_wkb(true, &sq)).unwrap(), vec![vec![sq.to_vec()]]);
        assert_eq!(wkb(&ring_wkb(false, &sq)).unwrap(), vec![vec![sq.to_vec()]]);
        let mut multi = vec![1u8];
        multi.extend(6u32.to_le_bytes());
        multi.extend(2u32.to_le_bytes());
        multi.extend(ring_wkb(true, &sq));
        multi.extend(ring_wkb(false, &sq));
        assert_eq!(wkb(&multi).unwrap().len(), 2);
        assert!(wkb(&[1, 1, 0, 0, 0]).is_none(), "a point is not a building");
        assert!(wkb(&ring_wkb(true, &sq)[..20]).is_none(), "cut short");
        let tail = br#"...{"version": "1.1.0", "primary_column": "geometry", "columns": {"geometry": {"encoding": "WKB", "bbox": [19.1, 52.4, 24.2, 56.3]}}}..."#;
        assert_eq!(tail_bbox(tail), Some([19.1, 52.4, 24.2, 56.3]));
        assert_eq!(tags("<a><Key>x</Key><Key>y</Key></a>", "Key"), vec!["x", "y"]);
    }
}
