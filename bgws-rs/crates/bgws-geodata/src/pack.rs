//! A real-ground build written as a theatre pack: the engine's terrain tiles,
//! the road graph, the viewer's basemap, and where it all came from.

use crate::build::{BuildSpec, Ground};
use crate::pmtiles::{write_archive, Compression, TileType};
use crate::roads::RoadGraph;
use crate::GeoError;
use bgws_env::{Codec, PackMeta, RoadNetwork, SynthWorld, TheatrePack};
use std::path::Path;

/// Node intervals per terrain tile side (128 × 30 m ≈ 3.8 km tiles).
pub const TILE_INTERVALS: usize = 128;

pub fn write_pack(dir: &Path, name: &str, spec: &BuildSpec, g: Ground, roads: &RoadGraph) -> Result<PackMeta, GeoError> {
    let io = |e: bgws_env::TileError| GeoError::Io(format!("{e:?}"));
    std::fs::create_dir_all(dir)?;
    crate::preview::render(&g.terrain, &g.rivers, &g.towns, roads).write_png(&dir.join("preview.png"))?;
    let (towns, dem) = (g.towns, g.dem);
    let world = SynthWorld { terrain: g.terrain, roads: RoadNetwork::default(), rivers: g.rivers, towns, power_lines: Vec::new() };
    let mut meta = TheatrePack::write(dir, name, &world, TILE_INTERVALS, Codec::Lz4).map_err(io)?;
    // The graph is in roads/; the engine builds its network from it.
    let _ = std::fs::remove_file(dir.join("roads.json"));
    meta.roads = None;
    roads.write(&dir.join("roads"))?;
    meta.road_graph = Some("roads".into());
    if !g.basemap.is_empty() {
        let b = spec.bbox;
        write_archive(
            &dir.join("basemap.pmtiles"),
            g.basemap,
            TileType::Mvt,
            Compression::Gzip,
            [b.west, b.south, b.east, b.north],
            &serde_json::json!({ "name": name, "attribution": "© OpenStreetMap contributors (ODbL) · Protomaps", "source": g.planet }),
        )?;
        meta.basemap = Some("basemap.pmtiles".into());
    }
    let b = spec.bbox;
    if !dem.is_empty() {
        write_archive(&dir.join("dem.pmtiles"), dem, TileType::Webp, Compression::None, [b.west, b.south, b.east, b.north], &serde_json::json!({ "name": name, "encoding": "terrarium", "attribution": "© Mapterhorn" }))?;
        meta.dem = Some("dem.pmtiles".into());
    }
    meta.bbox = Some([b.west, b.south, b.east, b.north]);
    meta.sources = Some(serde_json::json!({
        "builtWith": concat!("bgws-geodata ", env!("CARGO_PKG_VERSION")),
        "osm": { "archive": g.planet, "zoom": g.zoom, "attribution": "© OpenStreetMap contributors, ODbL; Protomaps basemap build" },
        "elevation": { "archive": spec.elevation, "zoom": spec.elevation_zoom, "attribution": "© Mapterhorn (Copernicus GLO-30 and national elevation models)", "dsmCanopyFraction": spec.dsm_canopy_fraction },
        "cellM": spec.cell_m,
        "ground": g.stats,
        "roads": roads.stats,
    }));
    TheatrePack::write_meta(dir, &meta).map_err(io)?;
    Ok(meta)
}

/// Build a box's ground and roads and write the pack: the whole pre-game step.
pub fn build_pack(dir: &Path, name: &str, spec: &BuildSpec, snap_m: f64, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<PackMeta, GeoError> {
    let ground = crate::build::build(spec, progress)?;
    progress("road network", 0.0);
    let roads = crate::roads::build_graph(&ground, snap_m);
    progress("writing the pack", 0.0);
    let meta = write_pack(dir, name, spec, ground, &roads)?;
    progress("done", 1.0);
    Ok(meta)
}
