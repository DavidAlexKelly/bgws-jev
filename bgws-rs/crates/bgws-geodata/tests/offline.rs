//! A pack built end to end from small archives made here: no network, so it
//! runs in CI. The map has two roads crossing (a main road east–west, a
//! minor one north–south), a wood, a lake, and ground rising to the east.

use bgws_env::{LandCover, RoadClass};
use bgws_geodata::build::BuildSpec;
use bgws_geodata::mercator::{tile_xy, tiles_covering, BBox};
use bgws_geodata::pmtiles::{write_archive, Compression, TileType};

const EXTENT: f64 = 4096.0;
const BOX: BBox = BBox { west: 20.40, south: 54.70, east: 20.43, north: 54.72 };

/// Ground height, m: 10 m in the west rising 1 m per 0.001° east.
fn height(lon: f64) -> f64 {
    10.0 + (lon - BOX.west) * 1_000.0
}

// ── Writing vector tiles ────────────────────────────────────────────────

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    varint(out, ((field as u64) << 3) | 2);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// A feature's geometry commands: a line, or a closed ring.
fn geometry(pts: &[(i64, i64)], ring: bool) -> Vec<u8> {
    let mut c = Vec::new();
    let (mut x, mut y) = (0, 0);
    let pts = if ring { &pts[..pts.len() - 1] } else { pts };
    for (k, &(px, py)) in pts.iter().enumerate() {
        if k == 0 {
            varint(&mut c, (1 << 3) | 1);
        } else if k == 1 {
            varint(&mut c, (((pts.len() - 1) as u64) << 3) | 2);
        }
        varint(&mut c, zz(px - x));
        varint(&mut c, zz(py - y));
        (x, y) = (px, py);
    }
    if ring {
        varint(&mut c, (1 << 3) | 7);
    }
    c
}

/// One layer's features: (geometry type, points, string tags).
struct Feature {
    kind: u64,
    pts: Vec<(i64, i64)>,
    tags: Vec<(&'static str, &'static str)>,
}

fn layer(name: &str, features: &[Feature]) -> Vec<u8> {
    let mut keys: Vec<&str> = Vec::new();
    let mut values: Vec<&str> = Vec::new();
    let mut l = Vec::new();
    bytes(&mut l, 1, name.as_bytes());
    for f in features {
        let mut tags = Vec::new();
        for &(k, v) in &f.tags {
            let ki = keys.iter().position(|x| *x == k).unwrap_or_else(|| {
                keys.push(k);
                keys.len() - 1
            });
            let vi = values.iter().position(|x| *x == v).unwrap_or_else(|| {
                values.push(v);
                values.len() - 1
            });
            varint(&mut tags, ki as u64);
            varint(&mut tags, vi as u64);
        }
        let mut fb = Vec::new();
        bytes(&mut fb, 2, &tags);
        varint(&mut fb, 3 << 3);
        varint(&mut fb, f.kind);
        bytes(&mut fb, 4, &geometry(&f.pts, f.kind == 3));
        bytes(&mut l, 2, &fb);
    }
    for k in keys {
        bytes(&mut l, 3, k.as_bytes());
    }
    for v in values {
        let mut vb = Vec::new();
        bytes(&mut vb, 1, v.as_bytes());
        bytes(&mut l, 4, &vb);
    }
    varint(&mut l, 5 << 3);
    varint(&mut l, EXTENT as u64);
    varint(&mut l, (15 << 3) as u64);
    varint(&mut l, 2);
    l
}

/// The part of a lon/lat box inside a tile, as a clockwise ring in tile pixels.
fn rect(z: u8, x: u32, y: u32, w: f64, s: f64, e: f64, n: f64) -> Option<Vec<(i64, i64)>> {
    let px = |lon: f64, lat: f64| {
        let (tx, ty) = tile_xy(lon, lat, z);
        (((tx - x as f64) * EXTENT).round() as i64, ((ty - y as f64) * EXTENT).round() as i64)
    };
    let (x0, y0) = px(w, n);
    let (x1, y1) = px(e, s);
    let (x0, x1, y0, y1) = (x0.max(0), x1.min(EXTENT as i64), y0.max(0), y1.min(EXTENT as i64));
    (x0 < x1 && y0 < y1).then(|| vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
}

/// The part of a straight line inside a tile (clipped by the tile's box).
fn clip_line(z: u8, x: u32, y: u32, a: (f64, f64), b: (f64, f64)) -> Option<Vec<(i64, i64)>> {
    let p = |q: (f64, f64)| {
        let (tx, ty) = tile_xy(q.0, q.1, z);
        ((tx - x as f64) * EXTENT, (ty - y as f64) * EXTENT)
    };
    let (pa, pb) = (p(a), p(b));
    // Liang–Barsky against [0, EXTENT]².
    let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (pp, q) in [(-dx, pa.0), (dx, EXTENT - pa.0), (-dy, pa.1), (dy, EXTENT - pa.1)] {
        if pp == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / pp;
            if pp < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 < t1).then(|| {
        let at = |t: f64| ((pa.0 + dx * t).round() as i64, (pa.1 + dy * t).round() as i64);
        vec![at(t0), at(t1)]
    })
}

fn vector_tile(z: u8, x: u32, y: u32) -> Vec<u8> {
    // Each road is two ways meeting at the crossing, as in the map data
    // (a junction is a shared vertex; a crossing without one is an overpass).
    let mut roads = Vec::new();
    let x0 = (20.415, 54.71);
    for (a, b) in [((20.395, 54.71), x0), (x0, (20.435, 54.71))] {
        if let Some(pts) = clip_line(z, x, y, a, b) {
            roads.push(Feature { kind: 2, pts, tags: vec![("kind", "major_road"), ("kind_detail", "primary"), ("name", "Main Road")] });
        }
    }
    for (a, b) in [((20.415, 54.695), x0), (x0, (20.415, 54.725))] {
        if let Some(pts) = clip_line(z, x, y, a, b) {
            roads.push(Feature { kind: 2, pts, tags: vec![("kind", "minor_road"), ("kind_detail", "tertiary")] });
        }
    }
    let mut landuse = Vec::new();
    if let Some(pts) = rect(z, x, y, 20.400, 54.712, 20.410, 54.718) {
        landuse.push(Feature { kind: 3, pts, tags: vec![("kind", "forest")] });
    }
    let mut water = Vec::new();
    if let Some(pts) = rect(z, x, y, 20.420, 54.701, 20.428, 54.705) {
        water.push(Feature { kind: 3, pts, tags: vec![("kind", "lake")] });
    }
    let mut tile = Vec::new();
    for (name, f) in [("roads", roads), ("landuse", landuse), ("water", water)] {
        if !f.is_empty() {
            bytes(&mut tile, 3, &layer(name, &f));
        }
    }
    tile
}

// ── Writing elevation tiles ─────────────────────────────────────────────

fn elevation_tile(z: u8, x: u32, _y: u32) -> Vec<u8> {
    let n = 256usize;
    let mut rgb = Vec::with_capacity(n * n * 3);
    let tiles = (1u64 << z) as f64;
    for _j in 0..n {
        for i in 0..n {
            let lon = (x as f64 + (i as f64 + 0.5) / n as f64) / tiles * 360.0 - 180.0;
            let v = height(lon) + 32_768.0;
            rgb.extend_from_slice(&[(v / 256.0).floor() as u8, (v.floor() as u64 % 256) as u8, ((v.fract()) * 256.0) as u8]);
        }
    }
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out).encode(&rgb, n as u32, n as u32, image_webp::ColorType::Rgb8).unwrap();
    out
}

#[test]
fn a_pack_from_local_archives_has_its_ground_roads_and_water() {
    let dir = std::env::temp_dir().join(format!("bgws-offline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let grown = BBox { west: BOX.west - 0.02, south: BOX.south - 0.02, east: BOX.east + 0.02, north: BOX.north + 0.02 };
    let bounds = [grown.west, grown.south, grown.east, grown.north];

    let vz = 14;
    let vtiles: Vec<_> = tiles_covering(&grown, vz).into_iter().map(|(x, y)| ((vz, x, y), vector_tile(vz, x, y))).collect();
    let planet = dir.join("planet.pmtiles");
    write_archive(&planet, vtiles, TileType::Mvt, Compression::None, bounds, &serde_json::json!({})).unwrap();
    let mut etiles = Vec::new();
    for z in 8..=12u8 {
        etiles.extend(tiles_covering(&grown, z).into_iter().map(|(x, y)| ((z, x, y), elevation_tile(z, x, y))));
    }
    let elevation = dir.join("elevation.pmtiles");
    write_archive(&elevation, etiles, TileType::Webp, Compression::None, bounds, &serde_json::json!({})).unwrap();

    let mut spec = BuildSpec::new(BOX);
    spec.planet = Some(planet.to_string_lossy().into_owned());
    spec.elevation = elevation.to_string_lossy().into_owned();
    spec.cache = None;
    spec.threads = 2;
    let pack = dir.join("pack");
    let meta = bgws_geodata::pack::build_pack(&pack, "offline", &spec, 15.0, &|_, _| {}).unwrap();
    assert_eq!(meta.road_graph.as_deref(), Some("roads"));
    for f in ["pack.json", "preview.png", "roads/graph.json", "roads/nodes.bin", "roads/edges.bin"] {
        assert!(pack.join(f).exists(), "{f}");
    }

    // The ground, as the engine reads it back.
    let ground = bgws_geodata::build::build(&spec, &|_, _| {}).unwrap();
    let at = |lon: f64, lat: f64| {
        let v = ground.frame.geodetic_to_enu(bgws_core::Geodetic::new(lat, lon, 0.0));
        (v.e, v.n)
    };
    let t = &ground.terrain;
    let (we, wn) = at(20.402, 54.705);
    let (ee, en) = at(20.428, 54.715);
    let (hw, he) = (t.height(we, wn), t.height(ee, en));
    assert!((hw - height(20.402)).abs() < 2.0 && (he - height(20.428)).abs() < 2.0, "heights {hw:.1} and {he:.1}");
    let (fe, fn_) = at(20.405, 54.715);
    assert_eq!(t.land_cover_at(fe, fn_), LandCover::ForestDense);
    let (le, ln) = at(20.424, 54.703);
    assert_eq!(t.land_cover_at(le, ln), LandCover::Water);
    let (oe, on) = at(20.425, 54.716);
    assert_eq!(t.land_cover_at(oe, on), LandCover::Open);

    // The roads: one graph, the crossing a junction, the main road primary
    // across the box (kept a little beyond it) and joined across tile seams.
    let g = bgws_geodata::roads::RoadGraph::read(&pack.join("roads")).unwrap();
    let primary: f64 = g.edges.iter().filter(|e| e.class == RoadClass::Primary).map(|e| e.length_m).sum();
    let width_m = { let (a, _) = at(BOX.west, 54.71); let (b, _) = at(BOX.east, 54.71); b - a };
    assert!(primary > width_m && primary < 1.4 * width_m, "main road {primary:.0} m across a box {width_m:.0} m wide");
    assert!(g.edges.iter().filter(|e| e.class == RoadClass::Primary).count() <= 2, "one road either side of the crossing: {:?}", g.edges.iter().map(|e| (e.class, e.length_m)).collect::<Vec<_>>());
    let mut degree = vec![0usize; g.nodes.len()];
    for e in &g.edges {
        degree[e.from as usize] += 1;
        degree[e.to as usize] += 1;
    }
    assert!(degree.contains(&4), "the crossing is a junction: {degree:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
