//! Against the real planet archives: run with `cargo test -p bgws-geodata -- --ignored`.

use bgws_geodata::{elevation, mercator, mvt, pmtiles, source};

const ELEVATION: &str = "https://download.mapterhorn.com/planet.pmtiles";

fn cache() -> std::path::PathBuf {
    std::env::temp_dir().join("bgws-geodata-live")
}

#[test]
#[ignore]
fn elevation_tile_over_kaliningrad_is_plausible() {
    let r = pmtiles::Reader::open(source::open_source(ELEVATION, Some(&cache())).unwrap()).unwrap();
    assert_eq!(r.header.tile_type, pmtiles::TileType::Webp);
    let (x, y) = mercator::tile_xy(20.5, 54.7, 12);
    let t = elevation::decode_webp(&r.tile(12, x as u32, y as u32).unwrap().expect("a tile")).unwrap();
    let (lo, hi) = t.h.iter().fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
    println!("z12 tile {}×{}: {lo:.1} m to {hi:.1} m", t.size, t.size);
    assert!(lo > -20.0 && hi < 250.0 && hi > lo, "the Kaliningrad lowland");
}

#[test]
#[ignore]
fn vector_tile_over_kaliningrad_has_roads_and_land() {
    let url = std::env::var("BGWS_PLANET").expect("BGWS_PLANET: a Protomaps build URL");
    let r = pmtiles::Reader::open(source::open_source(&url, Some(&cache())).unwrap()).unwrap();
    let (x, y) = mercator::tile_xy(20.5, 54.7, 14);
    let layers = mvt::decode(&r.tile(14, x as u32, y as u32).unwrap().expect("a tile")).unwrap();
    for l in &layers {
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for f in &l.features {
            *kinds.entry(f.str("kind").unwrap_or("?").to_string()).or_default() += 1;
        }
        println!("{} ({}): {:?}", l.name, l.features.len(), kinds);
    }
    assert!(layers.iter().any(|l| l.name == "roads" && !l.features.is_empty()));
}

#[test]
#[ignore]
fn overture_buildings_over_kaliningrad() {
    use bgws_geodata::overture;
    let b = mercator::BBox::new(20.50, 54.70, 20.53, 54.72);
    let t = std::time::Instant::now();
    let (all, release) = overture::buildings(b, None, Some(&cache()), 8, &|_, _| {}).unwrap();
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for x in &all {
        *by.entry(x.dataset.clone()).or_default() += 1;
    }
    let heights = all.iter().filter(|x| x.height.is_some()).count();
    println!("Overture {release}: {} buildings ({heights} with a height) by source {by:?} in {:.1} s", all.len(), t.elapsed().as_secs_f64());
    assert!(all.len() > 500, "a city district has hundreds of buildings");
    assert!(all.iter().all(|x| x.rings[0].len() >= 4));
    assert!(by.contains_key("OpenStreetMap"));
}

#[test]
#[ignore]
fn overture_land_cover_over_kaliningrad() {
    use bgws_geodata::overture;
    let b = mercator::BBox::new(20.44, 54.70, 20.52, 54.73);
    let t = std::time::Instant::now();
    let (all, release) = overture::land_cover(b, None, Some(&cache()), 8, &|_, _| {}).unwrap();
    let mut by: std::collections::BTreeMap<(String, i32, i32), (usize, usize)> = Default::default();
    for x in &all {
        let e = by.entry((x.subtype.clone(), x.min_zoom, x.max_zoom)).or_default();
        e.0 += 1;
        e.1 += x.rings.iter().map(|r| r.len()).sum::<usize>();
    }
    println!("Overture {release}: {} land cover patches in {:.1} s", all.len(), t.elapsed().as_secs_f64());
    for ((s, z0, z1), (n, pts)) in &by {
        println!("  {s:10} zooms {z0:2}-{z1:2}: {n:5} patches, {pts:7} points");
    }
    assert!(all.iter().any(|x| x.subtype == "forest"));
}

#[test]
#[ignore]
fn worldcover_over_kaliningrad() {
    use bgws_geodata::worldcover;
    let b = mercator::BBox::new(20.35, 54.65, 20.6, 54.76);
    let t = std::time::Instant::now();
    let c = worldcover::cover(b, Some(&cache()), &|_, _| {}).unwrap();
    let mut n = [0usize; 256];
    for &v in &c.class {
        n[v as usize] += 1;
    }
    let total = c.class.len() as f64;
    println!("WorldCover {} × {} px in {:.1} s", c.width, c.height, t.elapsed().as_secs_f64());
    for (v, k) in n.iter().enumerate().filter(|x| *x.1 > 0) {
        println!("  class {v:3}: {:.1}%", *k as f64 / total * 100.0);
    }
    // Central Kaliningrad: built up, with trees and water.
    assert!(n[worldcover::TREES as usize] as f64 / total > 0.05);
    assert!(n[worldcover::BUILT as usize] as f64 / total > 0.2);
    // Placed right: where the pack (OpenStreetMap) has water, WorldCover has water too, and not elsewhere.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/kaliningrad");
    if let Ok(mut pack) = bgws_env::TheatrePack::open(&dir) {
        let s = pack.meta.spec;
        let frame = bgws_core::EnuFrame::new(pack.meta.frame_origin);
        let aoi = bgws_env::Aoi { min_e: s.origin_e, min_n: s.origin_n, max_e: s.origin_e + s.cell * (s.width - 1) as f64, max_n: s.origin_n + s.cell * (s.height - 1) as f64 };
        let t = pack.assemble(&aoi).unwrap();
        let (mut wet, mut wet_hit, mut dry, mut dry_hit) = (0, 0, 0, 0);
        for j in 0..t.spec.height {
            for i in 0..t.spec.width {
                let (e, n) = (t.spec.origin_e + i as f64 * t.spec.cell, t.spec.origin_n + j as f64 * t.spec.cell);
                let g = frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0));
                if g.lon < b.west || g.lon > b.east || g.lat < b.south || g.lat > b.north {
                    continue;
                }
                let w = c.at(g.lon, g.lat) == worldcover::WATER;
                if t.land_cover.data[t.spec.idx(i, j)] == bgws_env::LandCover::Water {
                    wet += 1;
                    wet_hit += w as usize;
                } else {
                    dry += 1;
                    dry_hit += w as usize;
                }
            }
        }
        let (a, f) = (wet_hit as f64 / wet as f64, dry_hit as f64 / dry as f64);
        println!("  where the pack has water, WorldCover has water {:.0}%; elsewhere {:.1}%", a * 100.0, f * 100.0);
        assert!(a > 0.7 && f < 0.05, "the rasters line up");
    }
}
