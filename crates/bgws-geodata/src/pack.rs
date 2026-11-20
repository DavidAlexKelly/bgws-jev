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
    let tree_cover = g.tree_cover;
    if let Some(surface) = &g.surface {
        surface.write(&dir.join("surface.bin"))?;
    }
    let has_surface = g.surface.is_some();
    let world = SynthWorld { terrain: g.terrain, roads: RoadNetwork::default(), rivers: g.rivers, towns, power_lines: Vec::new() };
    let mut meta = TheatrePack::write(dir, name, &world, TILE_INTERVALS, Codec::Lz4).map_err(io)?;
    // The graph is in roads/; the engine builds its network from it.
    let _ = std::fs::remove_file(dir.join("roads.json"));
    meta.roads = None;
    roads.write(&dir.join("roads"))?;
    meta.road_graph = Some("roads".into());
    write_railways(dir, &mut meta, &g.basemap, &g.frame)?;
    let overture = g.stats.get("overture").and_then(|o| o.get("added")).and_then(|n| n.as_u64()).unwrap_or(0) > 0;
    let attribution = if overture { "© OpenStreetMap contributors (ODbL) · Protomaps · buildings also © Overture Maps Foundation (ODbL)" } else { "© OpenStreetMap contributors (ODbL) · Protomaps" };
    if !g.basemap.is_empty() {
        let b = spec.bbox;
        write_archive(&dir.join("basemap.pmtiles"), g.basemap, TileType::Mvt, Compression::Gzip, [b.west, b.south, b.east, b.north], &serde_json::json!({ "name": name, "attribution": attribution, "source": g.planet }))?;
        meta.basemap = Some("basemap.pmtiles".into());
    }
    let b = spec.bbox;
    if !dem.is_empty() {
        write_archive(&dir.join("dem.pmtiles"), dem, TileType::Webp, Compression::None, [b.west, b.south, b.east, b.north], &serde_json::json!({ "name": name, "encoding": "terrarium", "attribution": "© Mapterhorn" }))?;
        meta.dem = Some("dem.pmtiles".into());
    }
    if has_surface {
        meta.surface = Some("surface.bin".into());
    }
    if let Some(c) = &tree_cover {
        meta.tree_cover = Some(write_cover(dir, c)?);
    }
    meta.bbox = Some([b.west, b.south, b.east, b.north]);
    meta.sources = Some(serde_json::json!({
        "builtWith": concat!("bgws-geodata ", env!("CARGO_PKG_VERSION")),
        "osm": { "archive": g.planet, "zoom": g.zoom, "attribution": "© OpenStreetMap contributors, ODbL; Protomaps basemap build" },
        "elevation": { "archive": spec.elevation, "zoom": spec.elevation_zoom, "attribution": "© Mapterhorn (Copernicus GLO-30 and national elevation models)", "groundFilterM": spec.ground_filter_m },
        "cellM": spec.cell_m,
        "ground": g.stats,
        "roads": roads.stats,
    }));
    TheatrePack::write_meta(dir, &meta).map_err(io)?;
    Ok(meta)
}

/// Build a box's ground and roads and write the pack: the whole pre-game step.
pub fn build_pack(dir: &Path, name: &str, spec: &BuildSpec, snap_m: f64, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<PackMeta, GeoError> {
    build_pack_with(dir, name, spec, snap_m, progress, &mut |_| Ok(()))
}

/// Read a pack's railway lines from the basemap it already has (zoom 14
/// tiles over its box) into `railways.json`, without fetching anything:
/// for a pack built before packs carried railways. Returns the lines written.
pub fn railways_from_basemap(dir: &Path) -> Result<usize, GeoError> {
    let io = |e: bgws_env::TileError| GeoError::Io(format!("{e:?}"));
    let mut meta = TheatrePack::open(dir).map_err(io)?.meta;
    let [w, s, e, n] = meta.bbox.ok_or_else(|| GeoError::Format(format!("{}: the pack has no bbox", dir.display())))?;
    let file = meta.basemap.clone().ok_or_else(|| GeoError::Format(format!("{}: the pack has no basemap", dir.display())))?;
    let osm = crate::pmtiles::Reader::open(crate::source::open_source(&dir.join(file).to_string_lossy(), None)?)?;
    let z = osm.header.max_zoom.min(14);
    let mut tiles = Vec::new();
    for (x, y) in crate::mercator::tiles_covering(&crate::mercator::BBox::new(w, s, e, n).grown(500.0), z) {
        if let Some(raw) = osm.tile_raw(z, x, y)? {
            tiles.push(((z, x, y), raw));
        }
    }
    let frame = bgws_core::EnuFrame::new(meta.frame_origin);
    let rails = crate::build::railways_from_tiles(&tiles, osm.header.tile_compression, &frame)?;
    let count = rails.len();
    write_rails(dir, &mut meta, rails)?;
    TheatrePack::write_meta(dir, &meta).map_err(io)?;
    Ok(count)
}

fn write_rails(dir: &Path, meta: &mut PackMeta, rails: Vec<bgws_env::Railway>) -> Result<(), GeoError> {
    if rails.is_empty() {
        meta.railways = None;
        let _ = std::fs::remove_file(dir.join("railways.json"));
        return Ok(());
    }
    std::fs::write(dir.join("railways.json"), serde_json::to_vec(&rails).map_err(|e| GeoError::Format(e.to_string()))?)?;
    meta.railways = Some("railways.json".into());
    Ok(())
}

/// Light woodland on a sight surface wherever WorldCover has trees and the
/// cell has no building and no wood yet (a wood the map has is kept as it
/// is). Returns the cells added.
pub fn add_tree_cover(surface: &mut bgws_env::SightSurface, frame: &bgws_core::EnuFrame, c: &crate::worldcover::Cover) -> usize {
    let v = bgws_env::LandCover::ForestLight as u8 + 1;
    let s = surface.spec;
    let mut added = 0;
    for j in 0..s.height {
        let n = s.origin_n + j as f64 * s.cell;
        // Along a row, longitude and latitude are near enough linear.
        let a = frame.enu_to_geodetic(bgws_core::Enu::new(s.origin_e, n, 0.0));
        let z = frame.enu_to_geodetic(bgws_core::Enu::new(s.origin_e + (s.width.max(2) - 1) as f64 * s.cell, n, 0.0));
        for i in 0..s.width {
            let k = s.idx(i, j);
            if surface.tree[k] != 0 || surface.building_m[k] != 0 {
                continue;
            }
            let t = i as f64 / (s.width.max(2) - 1) as f64;
            if c.at(a.lon + (z.lon - a.lon) * t, a.lat + (z.lat - a.lat) * t) == crate::worldcover::TREES {
                surface.tree[k] = v;
                added += 1;
            }
        }
    }
    added
}

/// Land cover at 10 m as `treecover.png` (ESA WorldCover's classes as grey levels), placed.
fn write_cover(dir: &Path, c: &crate::worldcover::Cover) -> Result<bgws_env::CoverRef, GeoError> {
    crate::preview::write_gray_png(&dir.join("treecover.png"), c.width, c.height, &c.class)?;
    Ok(bgws_env::CoverRef { file: "treecover.png".into(), west: c.west, north: c.north, step: c.step, width: c.width, height: c.height })
}

/// Read where the trees are at 10 m (ESA WorldCover) over a pack's box into
/// `treecover.png`, for the viewer's trees, without rebuilding the rest: for a
/// pack built before packs carried it. The simulation's sight surface gains
/// them too, as a build lays them: light woodland in every 5 m cell under
/// trees that has no building and no mapped wood (hedgerows, tree lines,
/// gardens). Returns the share under trees.
pub fn refresh_tree_cover(dir: &Path, cache: Option<&Path>, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<f64, GeoError> {
    let io = |e: bgws_env::TileError| GeoError::Io(format!("{e:?}"));
    let mut meta = TheatrePack::open(dir).map_err(io)?.meta;
    let [w, s, e, n] = meta.bbox.ok_or_else(|| GeoError::Format(format!("{}: the pack has no bbox", dir.display())))?;
    let c = crate::worldcover::cover(crate::mercator::BBox::new(w, s, e, n).grown(200.0), cache, progress)?;
    meta.tree_cover = Some(write_cover(dir, &c)?);
    if let Some(file) = meta.surface.clone() {
        let path = dir.join(&file);
        let mut surface = bgws_env::SightSurface::read(&path).map_err(|e| GeoError::Io(format!("{}: {e}", path.display())))?;
        let added = add_tree_cover(&mut surface, &bgws_core::EnuFrame::new(meta.frame_origin), &c);
        surface.write(&path).map_err(|e| GeoError::Io(format!("{}: {e}", path.display())))?;
        progress(&format!("sight surface: {added} cells of trees added"), 1.0);
    }
    TheatrePack::write_meta(dir, &meta).map_err(io)?;
    Ok(c.class.iter().filter(|&&v| v == crate::worldcover::TREES).count() as f64 / c.class.len().max(1) as f64)
}

/// The railway lines in the basemap's tiles, as `railways.json` (none: no file).
fn write_railways(dir: &Path, meta: &mut PackMeta, basemap: &[crate::build::StoredTile], frame: &bgws_core::EnuFrame) -> Result<(), GeoError> {
    write_rails(dir, meta, crate::build::railways_from_tiles(basemap, Compression::Gzip, frame)?)
}

/// Fetch a pack's basemap again over its box, up to `max_zoom`, with the
/// buildings' estimated heights: for a pack built before its map went to
/// zoom 15. Everything else in the pack is left as it is. Returns the tiles written.
pub fn refresh_basemap(dir: &Path, max_zoom: u8, planet: Option<&str>, cache: Option<&Path>, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<usize, GeoError> {
    let io = |e: bgws_env::TileError| GeoError::Io(format!("{e:?}"));
    let mut meta = TheatrePack::open(dir).map_err(io)?.meta;
    let [w, s, e, n] = meta.bbox.ok_or_else(|| GeoError::Format(format!("{}: the pack has no bbox (built before packs recorded one)", dir.display())))?;
    let bbox = crate::mercator::BBox::new(w, s, e, n);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let (tiles, source) = crate::build::refetch_basemap(bbox, max_zoom, planet, cache, threads, progress)?;
    let count = tiles.len();
    let frame = bgws_core::EnuFrame::new(meta.frame_origin);
    write_railways(dir, &mut meta, &tiles, &frame)?;
    let tmp = dir.join("basemap.pmtiles.new");
    write_archive(&tmp, tiles, TileType::Mvt, Compression::Gzip, [w, s, e, n], &serde_json::json!({ "name": meta.name, "attribution": "© OpenStreetMap contributors (ODbL) · Protomaps", "source": source }))?;
    std::fs::rename(&tmp, dir.join("basemap.pmtiles")).map_err(|e| GeoError::Io(e.to_string()))?;
    meta.basemap = Some("basemap.pmtiles".into());
    TheatrePack::write_meta(dir, &meta).map_err(io)?;
    Ok(count)
}

/// As [`build_pack`], with `adjust` changing the ground before the road
/// network is made and the pack written (a partner's elevation, land cover
/// or roads in place of ours).
pub fn build_pack_with(dir: &Path, name: &str, spec: &BuildSpec, snap_m: f64, progress: &(dyn Fn(&str, f64) + Sync), adjust: &mut dyn FnMut(&mut crate::build::Ground) -> Result<(), String>) -> Result<PackMeta, GeoError> {
    let mut ground = crate::build::build(spec, progress)?;
    progress("partner terrain", 0.0);
    adjust(&mut ground).map_err(GeoError::Format)?;
    progress("road network", 0.0);
    let roads = crate::roads::build_graph(&ground, snap_m);
    progress("writing the pack", 0.0);
    let meta = write_pack(dir, name, spec, ground, &roads)?;
    progress("done", 1.0);
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{EnuFrame, Geodetic};
    use bgws_env::{GridSpec, LandCover, SightSurface};

    /// Trees from WorldCover go where the surface has neither a building nor
    /// a wood of its own; elsewhere it stays as built.
    #[test]
    fn tree_cover_fills_the_surface_around_buildings_and_woods() {
        let frame = EnuFrame::new(Geodetic::new(54.7, 20.5, 0.0));
        let spec = GridSpec::new(40, 40, 5.0, -100.0, -100.0);
        let mut surface = SightSurface::empty(spec);
        surface.building_m[spec.idx(30, 20)] = 9;
        surface.tree[spec.idx(31, 20)] = LandCover::ForestDense as u8 + 1;
        // Trees over the east half of the box, nothing to the west.
        let step = 1.0 / 12_000.0;
        let (west, north) = (20.49, 54.71);
        let (width, height) = (300, 300);
        let mid = frame.enu_to_geodetic(bgws_core::Enu::new(0.0, 0.0, 0.0)).lon;
        let class: Vec<u8> = (0..width * height).map(|k| if west + (k % width) as f64 * step >= mid { crate::worldcover::TREES } else { crate::worldcover::CROPS }).collect();
        let c = crate::worldcover::Cover { west, north, step, width, height, class };
        let added = add_tree_cover(&mut surface, &frame, &c);
        let light = LandCover::ForestLight as u8 + 1;
        assert_eq!(surface.tree[spec.idx(25, 20)], light, "east, under trees");
        assert_eq!(surface.tree[spec.idx(10, 20)], 0, "west, fields");
        assert_eq!(surface.tree[spec.idx(30, 20)], 0, "a building stays clear");
        assert_eq!(surface.tree[spec.idx(31, 20)], LandCover::ForestDense as u8 + 1, "a mapped wood keeps its class");
        assert!(added > 700 && added < 820, "{added}");
    }
}
