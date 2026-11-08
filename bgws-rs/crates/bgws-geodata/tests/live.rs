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
