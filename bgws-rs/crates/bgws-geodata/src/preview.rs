//! A picture of a pack, to check a build by eye: land cover over a
//! hillshade, rivers, and roads by class (bridges in white). One pixel per
//! cell, north up. A small PNG writer, so nothing else is needed.

use crate::roads::RoadGraph;
use bgws_env::{LandCover, RoadClass, River, Terrain, Town};
use std::io::Write;
use std::path::Path;

fn colour(lc: LandCover) -> [u8; 3] {
    match lc {
        LandCover::Open => [196, 214, 160],
        LandCover::Crops => [226, 222, 150],
        LandCover::Scrub => [168, 190, 120],
        LandCover::ForestLight => [96, 156, 84],
        LandCover::ForestDense => [52, 112, 60],
        LandCover::UrbanLight => [200, 186, 176],
        LandCover::UrbanDense => [150, 120, 112],
        LandCover::Water => [96, 150, 214],
        LandCover::Wetland => [124, 176, 168],
        LandCover::Bare => [220, 206, 180],
    }
}

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 3]>,
}

impl Image {
    fn put(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }
    fn line(&mut self, a: (f64, f64), b: (f64, f64), c: [u8; 3], thick: i64) {
        let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for dx in 0..thick {
                for dy in 0..thick {
                    self.put(x.round() as i64 + dx - thick / 2, y.round() as i64 + dy - thick / 2, c);
                }
            }
        }
    }

    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        let mut raw = Vec::with_capacity(self.h * (1 + self.w * 3));
        for row in self.px.chunks(self.w) {
            raw.push(0);
            for p in row {
                raw.extend_from_slice(p);
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw)?;
        let idat = z.finish()?;
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = kind.to_vec();
            c.extend_from_slice(data);
            out.extend_from_slice(&c);
            out.extend_from_slice(&crc32(&c).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(self.w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &idat);
        chunk(&mut out, b"IEND", &[]);
        std::fs::write(path, out)
    }
}

fn crc32(b: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &x in b {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Render a terrain with its rivers, towns and roads.
pub fn render(t: &Terrain, rivers: &[River], towns: &[Town], roads: &RoadGraph) -> Image {
    let s = t.spec;
    let (w, h) = (s.width, s.height);
    let mut img = Image { w, h, px: vec![[0; 3]; w * h] };
    let elev = |i: usize, j: usize| t.elevation.data[s.idx(i.min(w - 1), j.min(h - 1))] as f64;
    for j in 0..h {
        for i in 0..w {
            // Hillshade from the north-west, exaggerated for flat country.
            let dzdx = (elev(i + 1, j) - elev(i.saturating_sub(1), j)) / (2.0 * s.cell);
            let dzdy = (elev(i, j + 1) - elev(i, j.saturating_sub(1))) / (2.0 * s.cell);
            let shade = (1.0 + 4.0 * (-dzdx + dzdy)).clamp(0.55, 1.35);
            let c = colour(t.land_cover.data[s.idx(i, j)]);
            let px = c.map(|v| (v as f64 * shade).clamp(0.0, 255.0) as u8);
            img.px[(h - 1 - j) * w + i] = px;
        }
    }
    let to_px = |e: f64, n: f64| ((e - s.origin_e) / s.cell, (h - 1) as f64 - (n - s.origin_n) / s.cell);
    for r in rivers {
        for seg in r.line.windows(2) {
            img.line(to_px(seg[0].0, seg[0].1), to_px(seg[1].0, seg[1].1), [64, 118, 196], if r.width_m >= 10.0 { 2 } else { 1 });
        }
    }
    let mut order: Vec<&crate::roads::GraphEdge> = roads.edges.iter().collect();
    order.sort_by_key(|e| std::cmp::Reverse(e.class as u8));
    for e in order {
        let (c, thick) = match e.class {
            RoadClass::Motorway => ([214, 64, 64], 3),
            RoadClass::Primary => ([240, 150, 40], 2),
            RoadClass::Secondary => ([70, 70, 70], 1),
            RoadClass::Track => ([140, 100, 60], 1),
        };
        let c = if e.bridge { [255, 255, 255] } else { c };
        for seg in e.geom.windows(2) {
            img.line(to_px(seg[0][0], seg[0][1]), to_px(seg[1][0], seg[1][1]), c, thick);
        }
    }
    for t in towns {
        let (x, y) = to_px(t.e, t.n);
        for dx in -2..=2 {
            for dy in -2..=2 {
                img.put(x.round() as i64 + dx, y.round() as i64 + dy, [20, 20, 20]);
            }
        }
    }
    img
}
