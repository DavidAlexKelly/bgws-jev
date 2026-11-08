//! Building a theatre's ground from a box: elevation from Mapterhorn's
//! terrarium planet, land cover, water, towns and roads from a Protomaps
//! OpenStreetMap build, onto the engine's ENU grid (plan §7.1, §14.2).
//!
//! Land cover paints polygons in priority order at cell centres (open, then
//! fields, scrub, woods, wetland, towns, then water), then upgrades cells to
//! dense urban by building footprint cover. Copernicus heights are a surface
//! model (they include trees and roofs, smeared over 30 m), while the engine
//! has every building and wood itself (the sight surface). So woods and towns
//! are taken back to bare ground by a morphological opening of the heights
//! (`BuildSpec::ground_filter_m`, 0 to keep raw heights): the lowest ground
//! nearby, then the highest of those, which keeps slopes and drops the
//! bumps that roofs and canopy make.

use crate::elevation::{decode_webp, HeightTile};
use crate::fill::{fill_polygon, ring_area};
use crate::mercator::{tile_lonlat, tile_size_m, tile_xy, tiles_covering, BBox};
use crate::mvt::{self, GeomType};
use crate::pmtiles::{Compression, Reader};
use crate::source::open_source;
use crate::GeoError;
use bgws_core::{EnuFrame, Enu, Geodetic};
use bgws_env::{GridSpec, LandCover, Raster, River, SoilType, Terrain, Town};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

/// The planet archives, as published. Protomaps builds daily under a dated name.
pub const ELEVATION_URL: &str = "https://download.mapterhorn.com/planet.pmtiles";
pub const PROTOMAPS_BUILDS: &str = "https://build.protomaps.com";

#[derive(Clone, Debug)]
pub struct BuildSpec {
    pub bbox: BBox,
    /// Grid spacing, metres.
    pub cell_m: f64,
    /// Zoom read for land cover, water and roads (14: every street, ~1.4 km tiles at 55°N).
    pub vector_zoom: u8,
    /// Zoom read for elevation (12: ~11 m pixels at 55°N).
    pub elevation_zoom: u8,
    /// Highest zoom copied into the pack's basemap.
    pub basemap_max_zoom: u8,
    /// Buildings come from this zoom when the archive has it (Protomaps keeps
    /// every footprint only at its deepest zoom, 15).
    pub building_zoom: u8,
    /// Protomaps archive: URL or path; `None` finds the latest daily build.
    pub planet: Option<String>,
    pub elevation: String,
    pub cache: Option<PathBuf>,
    /// The window the heights under woods and towns are filtered to bare ground over, m (0 keeps them).
    pub ground_filter_m: f64,
    /// Parallel downloads.
    pub threads: usize,
}

impl BuildSpec {
    pub fn new(bbox: BBox) -> Self {
        Self {
            bbox,
            cell_m: 30.0,
            vector_zoom: 14,
            elevation_zoom: 12,
            basemap_max_zoom: 14,
            building_zoom: 15,
            planet: None,
            elevation: ELEVATION_URL.into(),
            cache: default_cache(),
            ground_filter_m: 150.0,
            threads: 16,
        }
    }
}

/// `$BGWS_CACHE`, else the platform's cache folder, under `bgws/pmtiles`.
pub fn default_cache() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("BGWS_CACHE") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let base = if cfg!(target_os = "macos") { home.join("Library/Caches") } else { std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")) };
    Some(base.join("bgws").join("pmtiles"))
}

/// The newest Protomaps daily build that answers, looking back a fortnight.
pub fn latest_planet() -> Result<String, GeoError> {
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| GeoError::Http(e.to_string()))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    for back in 0..15 {
        let (y, m, d) = civil_from_days((now / 86_400) - back);
        let url = format!("{PROTOMAPS_BUILDS}/{y:04}{m:02}{d:02}.pmtiles");
        if let Ok(r) = client.get(&url).header("Range", "bytes=0-126").send() {
            if r.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                return Ok(url);
            }
        }
    }
    Err(GeoError::Http(format!("no Protomaps build answered at {PROTOMAPS_BUILDS} in the last fortnight: pass --planet")))
}

/// Days since 1970-01-01 to (year, month, day) (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A road as read from the tiles: its properties and its line in the zoom's
/// global tile grid (tile index × extent + tile coordinate), clipped to its tile.
#[derive(Clone, Debug)]
pub struct RawRoad {
    pub kind: String,
    pub detail: String,
    pub bridge: bool,
    pub tunnel: bool,
    pub link: bool,
    pub name: Option<String>,
    pub line: Vec<[f64; 2]>,
}

/// What a build makes before it is written as a pack.
pub struct Ground {
    pub frame: EnuFrame,
    pub terrain: Terrain,
    pub rivers: Vec<River>,
    pub towns: Vec<Town>,
    pub roads: Vec<RawRoad>,
    pub zoom: u8,
    pub extent: u32,
    /// Basemap tiles, as stored (gzipped vector tiles).
    pub basemap: Vec<((u8, u32, u32), Vec<u8>)>,
    /// Elevation tiles, as stored (terrarium WebP), for the viewer's hillshade.
    pub dem: Vec<((u8, u32, u32), Vec<u8>)>,
    pub planet: String,
    pub stats: serde_json::Value,
    /// Buildings at their height and woods by class, on a fine grid, for sight lines.
    pub surface: Option<bgws_env::SightSurface>,
}

impl Ground {
    /// Global tile-grid coordinates at the build's zoom to ENU metres.
    pub fn grid_to_enu(&self, p: [f64; 2]) -> (f64, f64) {
        let ext = self.extent as f64;
        let (lon, lat) = tile_lonlat(p[0] / ext, p[1] / ext, self.zoom);
        let v = self.frame.geodetic_to_enu(Geodetic::new(lat, lon, 0.0));
        (v.e, v.n)
    }
}

/// How land use reads as engine land cover, and its paint priority.
pub fn landuse_class(kind: &str) -> Option<(LandCover, u8)> {
    use LandCover::*;
    Some(match kind {
        "park" | "garden" | "grass" | "grassland" | "meadow" | "pitch" | "golf_course" | "recreation_ground" | "playground" | "cemetery" | "village_green" | "dog_park" | "allotments" => (Open, 1),
        "farmland" | "farmyard" | "orchard" | "vineyard" | "plant_nursery" | "greenhouse_horticulture" => (Crops, 2),
        "scrub" | "heath" | "shrubbery" => (Scrub, 3),
        "beach" | "sand" | "bare_rock" | "scree" | "quarry" | "landfill" | "brownfield" | "construction" | "glacier" => (Bare, 4),
        "wood" | "forest" | "natural_wood" | "nature_reserve" => (ForestDense, 5),
        "wetland" | "marsh" | "swamp" | "bog" | "fen" | "reedbed" | "wet_meadow" | "mud" => (Wetland, 6),
        "residential" | "suburb" | "neighbourhood" | "school" | "university" | "college" | "kindergarten" | "hospital" | "military" | "naval_base" | "airfield" | "aerodrome" => (UrbanLight, 7),
        "commercial" | "industrial" | "retail" | "railway" | "pedestrian" | "platform" | "parking" | "garages" => (UrbanLight, 7),
        _ => return None,
    })
}

const WATER_PRIORITY: u8 = 9;

/// A building's height when the map gives none, m, by its footprint (m²):
/// a house about two storeys, a larger building three, a block or hall four.
fn default_height(area: f64) -> f64 {
    if area < 120.0 {
        6.0
    } else if area < 600.0 {
        9.0
    } else {
        12.0
    }
}

/// The sight surface's cell, m: as near this as divides the terrain cell.
const SURFACE_CELL_M: f64 = 5.0;

/// River width and depth by kind (metres) [default].
fn river_size(kind: &str) -> Option<(f64, f64)> {
    Some(match kind {
        "river" => (40.0, 2.5),
        "canal" => (15.0, 2.0),
        "stream" => (3.0, 0.6),
        "drain" | "ditch" => (2.0, 0.8),
        _ => return None,
    })
}

struct Shape {
    class: LandCover,
    priority: u8,
    rings: Vec<Vec<[f64; 2]>>,
}

struct TileOut {
    shapes: Vec<Shape>,
    /// Building footprint area (m²) at a grid position.
    buildings: Vec<(f64, f64, f64)>,
    /// Building footprints in grid coordinates, with their height (m).
    footprints: Vec<(Vec<Vec<[f64; 2]>>, f32)>,
    rivers: Vec<River>,
    towns: Vec<Town>,
    roads: Vec<RawRoad>,
    extent: u32,
}

fn pool(n: usize) -> Result<rayon::ThreadPool, GeoError> {
    rayon::ThreadPoolBuilder::new().num_threads(n.max(1)).build().map_err(|e| GeoError::Io(e.to_string()))
}

/// Build the ground. `progress(stage, fraction)` reports as it goes.
pub fn build(spec: &BuildSpec, progress: &(dyn Fn(&str, f64) + Sync)) -> Result<Ground, GeoError> {
    let b = spec.bbox;
    let (clon, clat) = b.centre();
    let frame = EnuFrame::new(Geodetic::new(clat, clon, 0.0));
    // The grid covers the box's corners and edge midpoints in ENU.
    let mut lo = (f64::MAX, f64::MAX);
    let mut hi = (f64::MIN, f64::MIN);
    for (lon, lat) in [(b.west, b.south), (b.east, b.south), (b.west, b.north), (b.east, b.north), (clon, b.south), (clon, b.north), (b.west, clat), (b.east, clat)] {
        let v = frame.geodetic_to_enu(Geodetic::new(lat, lon, 0.0));
        lo = (lo.0.min(v.e), lo.1.min(v.n));
        hi = (hi.0.max(v.e), hi.1.max(v.n));
    }
    let cell = spec.cell_m;
    let (w, h) = (((hi.0 - lo.0) / cell).ceil() as usize + 1, ((hi.1 - lo.1) / cell).ceil() as usize + 1);
    if w * h > 400_000_000 {
        return Err(GeoError::Format(format!("{w}×{h} cells is too many: use a smaller box or a larger --cell")));
    }
    let gspec = GridSpec::new(w, h, cell, lo.0, lo.1);
    let net = pool(spec.threads)?;

    // ── Elevation ──────────────────────────────────────────────────────────
    progress("elevation: opening archive", 0.0);
    let elev = Reader::open(open_source(&spec.elevation, spec.cache.as_deref())?)?;
    let ez = spec.elevation_zoom.min(elev.header.max_zoom);
    let etiles = tiles_covering(&b.grown(2.0 * cell), ez);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let fetched: Vec<((u32, u32), Option<HeightTile>)> = net.install(|| {
        etiles
            .par_iter()
            .map(|&(x, y)| {
                let t = elev.tile(ez, x, y)?.map(|bytes| decode_webp(&bytes)).transpose()?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress("elevation: tiles", n as f64 / etiles.len() as f64);
                Ok(((x, y), t))
            })
            .collect::<Result<_, GeoError>>()
    })?;
    let heights: HashMap<(u32, u32), HeightTile> = fetched.into_iter().filter_map(|(k, t)| t.map(|t| (k, t))).collect();
    progress("elevation: sampling", 0.0);
    let mut elevation = Raster::filled(gspec, 0.0f32);
    elevation.data.par_chunks_mut(w).enumerate().for_each(|(j, row)| {
        for (i, v) in row.iter_mut().enumerate() {
            let (e, n) = gspec.pos(i, j);
            let g = frame.enu_to_geodetic(Enu::new(e, n, 0.0));
            let (tx, ty) = tile_xy(g.lon, g.lat, ez);
            if let Some(t) = heights.get(&(tx.floor() as u32, ty.floor() as u32)) {
                let s = t.size as f64;
                *v = t.sample(tx.fract() * s, ty.fract() * s);
            }
        }
    });
    drop(heights);
    // The same tiles as stored, and a few zooms below, for the viewer's hillshade.
    let mut dem_want: Vec<(u8, u32, u32)> = Vec::new();
    for z in ez.saturating_sub(4)..=ez {
        dem_want.extend(tiles_covering(&b.grown(500.0), z).into_iter().map(|(x, y)| (z, x, y)));
    }
    let dem: Vec<((u8, u32, u32), Vec<u8>)> = net.install(|| {
        dem_want
            .par_iter()
            .filter_map(|&(z, x, y)| elev.tile_raw(z, x, y).transpose().map(|r| r.map(|b| ((z, x, y), b))))
            .collect::<Result<_, GeoError>>()
    })?;

    // ── Vector tiles: land cover, water, buildings, towns, roads ──────────
    progress("map data: opening archive", 0.0);
    let planet = match &spec.planet {
        Some(p) => p.clone(),
        None => latest_planet()?,
    };
    let osm = Reader::open(open_source(&planet, spec.cache.as_deref())?)?;
    let vz = spec.vector_zoom.min(osm.header.max_zoom);
    let bz = spec.building_zoom.min(osm.header.max_zoom);
    // Buildings from a deeper zoom when there is one; everything else from vz.
    let deep = bz > vz;
    let vtiles = tiles_covering(&b.grown(200.0), vz);
    let to_grid = |e: f64, n: f64| [(e - gspec.origin_e) / cell, (n - gspec.origin_n) / cell];
    let done = std::sync::atomic::AtomicUsize::new(0);
    let outs: Vec<TileOut> = net.install(|| {
        vtiles
            .par_iter()
            .map(|&(tx, ty)| {
                let out = match osm.tile(vz, tx, ty)? {
                    None => TileOut { shapes: vec![], buildings: vec![], footprints: vec![], rivers: vec![], towns: vec![], roads: vec![], extent: 4096 },
                    Some(bytes) => read_tile(&mvt::decode(&bytes)?, tx, ty, vz, &frame, &to_grid, true, !deep),
                };
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress("map data: tiles", n as f64 / vtiles.len() as f64);
                Ok(out)
            })
            .collect::<Result<_, GeoError>>()
    })?;
    // Every building, from the deeper zoom.
    let bouts: Vec<TileOut> = if deep {
        let btiles = tiles_covering(&b.grown(200.0), bz);
        let done = std::sync::atomic::AtomicUsize::new(0);
        net.install(|| {
            btiles
                .par_iter()
                .map(|&(tx, ty)| {
                    let out = match osm.tile(bz, tx, ty)? {
                        None => TileOut { shapes: vec![], buildings: vec![], footprints: vec![], rivers: vec![], towns: vec![], roads: vec![], extent: 4096 },
                        Some(bytes) => read_tile(&mvt::decode(&bytes)?, tx, ty, bz, &frame, &to_grid, false, true),
                    };
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress("buildings: tiles", n as f64 / btiles.len() as f64);
                    Ok(out)
                })
                .collect::<Result<_, GeoError>>()
        })?
    } else {
        Vec::new()
    };
    let extent = outs.iter().map(|o| o.extent).max().unwrap_or(4096);

    progress("land cover", 0.0);
    let mut cover = Raster::filled(gspec, LandCover::Open);
    let mut prio = vec![0u8; gspec.len()];
    let mut shapes: Vec<&Shape> = outs.iter().flat_map(|o| o.shapes.iter()).collect();
    shapes.sort_by_key(|s| s.priority);
    for s in shapes {
        fill_polygon(&s.rings, w, h, |i, j| {
            let k = gspec.idx(i, j);
            if s.priority >= prio[k] {
                prio[k] = s.priority;
                cover.data[k] = s.class;
            }
        });
    }
    // Building cover: footprint area per cell, then light and dense urban.
    let mut built = vec![0f32; gspec.len()];
    for o in outs.iter().chain(&bouts) {
        for &(gi, gj, area) in &o.buildings {
            let (i, j) = (gi.round(), gj.round());
            if i >= 0.0 && j >= 0.0 && (i as usize) < w && (j as usize) < h {
                built[gspec.idx(i as usize, j as usize)] += area as f32;
            }
        }
    }
    let cell_area = (cell * cell) as f32;
    let mut dense = 0usize;
    for (k, area) in built.iter().enumerate() {
        let frac = area / cell_area;
        if cover.data[k] == LandCover::Water {
            continue;
        }
        if frac > 0.30 {
            cover.data[k] = LandCover::UrbanDense;
            dense += 1;
        } else if frac > 0.06 && !cover.data[k].is_urban() {
            cover.data[k] = LandCover::UrbanLight;
        }
    }
    drop(built);
    drop(prio);

    // ── The sight surface: woods and buildings on a fine grid ─────────────
    progress("sight surface", 0.0);
    let fine = (cell / SURFACE_CELL_M).round().max(1.0) as usize;
    let fspec = GridSpec::new((w - 1) * fine + 1, (h - 1) * fine + 1, cell / fine as f64, lo.0, lo.1);
    let mut surface = bgws_env::SightSurface::empty(fspec);
    let up = |rings: &Vec<Vec<[f64; 2]>>| -> Vec<Vec<[f64; 2]>> { rings.iter().map(|r| r.iter().map(|p| [p[0] * fine as f64, p[1] * fine as f64]).collect()).collect() };
    {
        // Land use in priority order, as for the land cover: a wood, or what overlies it.
        let mut shapes: Vec<&Shape> = outs.iter().flat_map(|o| o.shapes.iter()).collect();
        shapes.sort_by_key(|s| s.priority);
        let mut prio = vec![0u8; fspec.len()];
        for s in shapes {
            let wood = matches!(s.class, LandCover::ForestDense | LandCover::ForestLight | LandCover::Scrub);
            let v = if wood { s.class as u8 + 1 } else { 0 };
            fill_polygon(&up(&s.rings), fspec.width, fspec.height, |i, j| {
                let k = fspec.idx(i, j);
                if s.priority >= prio[k] {
                    prio[k] = s.priority;
                    surface.tree[k] = v;
                }
            });
        }
    }
    let mut footprints = 0usize;
    for o in outs.iter().chain(&bouts) {
        for (rings, hgt) in &o.footprints {
            let v = hgt.round().clamp(1.0, 255.0) as u8;
            footprints += 1;
            fill_polygon(&up(rings), fspec.width, fspec.height, |i, j| {
                let k = fspec.idx(i, j);
                surface.building_m[k] = surface.building_m[k].max(v);
                surface.tree[k] = 0;
            });
        }
    }

    // ── Derived layers ─────────────────────────────────────────────────────
    let figures = bgws_env::default_figures();
    let gap = figures.get("env.synth.dense_forest_gap_m") as f32;
    let mut soil = Raster::filled(gspec, SoilType::Loam);
    let mut wetness = Raster::filled(gspec, 0.25f32);
    let mut obstacle_gap = Raster::filled(gspec, 0.0f32);
    for k in 0..gspec.len() {
        let lc = cover.data[k];
        soil.data[k] = match lc {
            LandCover::Wetland => SoilType::Peat,
            LandCover::Bare => SoilType::Sand,
            LandCover::Water => SoilType::Silt,
            _ => SoilType::Loam,
        };
        wetness.data[k] = match lc {
            LandCover::Water => 1.0,
            LandCover::Wetland => 0.85,
            _ => 0.25,
        };
        if lc == LandCover::ForestDense {
            obstacle_gap.data[k] = gap;
        }
        // The sea and lagoons carry bathymetry: their surface is what is seen across.
        if lc == LandCover::Water && elevation.data[k] < 0.0 {
            elevation.data[k] = 0.0;
        }
    }
    if spec.ground_filter_m > 0.0 {
        progress("bare ground", 0.0);
        bare_ground(&mut elevation, &cover, (spec.ground_filter_m / cell / 2.0).round().max(1.0) as usize);
    }

    // ── Basemap: the same archive, lower zooms and the box, as stored ─────
    progress("basemap", 0.0);
    let mut basemap = Vec::new();
    if osm.header.tile_compression == Compression::Gzip || osm.header.tile_compression == Compression::None {
        let mut want: Vec<(u8, u32, u32)> = Vec::new();
        for z in 0..=spec.basemap_max_zoom.min(osm.header.max_zoom) {
            want.extend(tiles_covering(&b.grown(500.0), z).into_iter().map(|(x, y)| (z, x, y)));
        }
        let done = std::sync::atomic::AtomicUsize::new(0);
        basemap = net.install(|| {
            want.par_iter()
                .filter_map(|&(z, x, y)| {
                    let r = osm.tile_raw(z, x, y).transpose()?;
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress("basemap", n as f64 / want.len() as f64);
                    Some(r.map(|b| ((z, x, y), b)))
                })
                .collect::<Result<Vec<_>, GeoError>>()
        })?;
    }

    let mut rivers: Vec<River> = Vec::new();
    let mut towns: Vec<Town> = Vec::new();
    let mut roads: Vec<RawRoad> = Vec::new();
    for o in outs {
        rivers.extend(o.rivers);
        towns.extend(o.towns);
        roads.extend(o.roads);
    }
    // Towns repeat in every tile's buffer: keep one per name and place.
    towns.sort_by(|a, b| a.name.cmp(&b.name).then(a.e.total_cmp(&b.e)));
    towns.dedup_by(|a, b| a.name == b.name && (a.e - b.e).hypot(a.n - b.n) < 2_000.0);
    towns.retain(|t| gspec.nearest(t.e, t.n).is_some());

    let counts = {
        let mut c = [0usize; 10];
        for v in &cover.data {
            c[*v as usize] += 1;
        }
        LandCover::ALL.iter().map(|lc| (lc.key().to_string(), serde_json::json!(c[*lc as usize] as f64 / gspec.len() as f64))).collect::<serde_json::Map<_, _>>()
    };
    let (emin, emax) = elevation.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let stats = serde_json::json!({
        "grid": [w, h], "cellM": cell, "elevationM": [emin, emax], "landCover": counts, "denseUrbanCells": dense,
        "rivers": rivers.len(), "towns": towns.len(), "roadParts": roads.len(), "basemapTiles": basemap.len(),
        "elevationZoom": ez, "vectorZoom": vz, "surfaceCellM": fspec.cell, "buildingFootprints": footprints,
    });
    let terrain = Terrain { frame_origin: frame.origin, spec: gspec, elevation, land_cover: cover, soil, wetness, obstacle_gap };
    Ok(Ground { frame, terrain, rivers, towns, roads, zoom: vz, extent, basemap, dem, planet, stats, surface: Some(surface) })
}

/// Woods and towns back to bare ground: a morphological opening (the lowest
/// height within `r` cells, then the highest of those) over cells with trees
/// or buildings and the cell round them, never raising a cell, then smoothed
/// there. Water is left out of the lowest, so a riverside town is not taken
/// down to the river's surface.
fn bare_ground(elev: &mut Raster<f32>, cover: &Raster<LandCover>, r: usize) {
    let s = elev.spec;
    let (w, h) = (s.width, s.height);
    let covered = |k: usize| matches!(cover.data[k], LandCover::ForestDense | LandCover::ForestLight | LandCover::UrbanLight | LandCover::UrbanDense);
    let mut mask = vec![false; w * h];
    for j in 0..h {
        for i in 0..w {
            if !covered(j * w + i) {
                continue;
            }
            for dj in j.saturating_sub(1)..=(j + 1).min(h - 1) {
                for di in i.saturating_sub(1)..=(i + 1).min(w - 1) {
                    if cover.data[dj * w + di] != LandCover::Water {
                        mask[dj * w + di] = true;
                    }
                }
            }
        }
    }
    if !mask.iter().any(|m| *m) {
        return;
    }
    // A separable min (or max) filter over a square of side 2r + 1.
    let filter = |src: &[f32], max: bool| -> Vec<f32> {
        let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
        let none = if max { f32::MIN } else { f32::MAX };
        let mut rows = vec![none; w * h];
        for j in 0..h {
            for i in 0..w {
                let mut v = none;
                for ii in i.saturating_sub(r)..=(i + r).min(w - 1) {
                    v = pick(v, src[j * w + ii]);
                }
                rows[j * w + i] = v;
            }
        }
        let mut out = vec![none; w * h];
        for j in 0..h {
            for i in 0..w {
                let mut v = none;
                for jj in j.saturating_sub(r)..=(j + r).min(h - 1) {
                    v = pick(v, rows[jj * w + i]);
                }
                out[j * w + i] = v;
            }
        }
        out
    };
    let land: Vec<f32> = elev.data.iter().zip(&cover.data).map(|(&e, &c)| if c == LandCover::Water { f32::MAX } else { e }).collect();
    let low = filter(&land, false);
    // Where the window held only water, keep the height.
    let low: Vec<f32> = low.iter().zip(&elev.data).map(|(&l, &e)| if l == f32::MAX { e } else { l }).collect();
    let opened = filter(&low, true);
    let mut bare = elev.data.clone();
    for k in 0..w * h {
        if mask[k] {
            bare[k] = bare[k].min(opened[k]);
        }
    }
    // Smooth the filtered cells (3 × 3 mean) so the edge of a town is no step.
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            if !mask[k] {
                continue;
            }
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for jj in j.saturating_sub(1)..=(j + 1).min(h - 1) {
                for ii in i.saturating_sub(1)..=(i + 1).min(w - 1) {
                    if cover.data[jj * w + ii] != LandCover::Water {
                        sum += bare[jj * w + ii];
                        n += 1.0;
                    }
                }
            }
            elev.data[k] = if n > 0.0 { (sum / n).min(elev.data[k]) } else { bare[k] };
        }
    }
}

/// One vector tile's contribution, in grid and ENU coordinates.
/// `base`: land use, water, places and roads; `buildings`: footprints.
#[allow(clippy::too_many_arguments)]
fn read_tile(layers: &[mvt::Layer], tx: u32, ty: u32, z: u8, frame: &EnuFrame, to_grid: &(dyn Fn(f64, f64) -> [f64; 2] + Sync), base: bool, buildings: bool) -> TileOut {
    let mut out = TileOut { shapes: vec![], buildings: vec![], footprints: vec![], rivers: vec![], towns: vec![], roads: vec![], extent: 4096 };
    let enu = |l: &mvt::Layer, p: [f64; 2]| {
        let ext = l.extent as f64;
        let (lon, lat) = tile_lonlat(tx as f64 + p[0] / ext, ty as f64 + p[1] / ext, z);
        let v = frame.geodetic_to_enu(Geodetic::new(lat, lon, 0.0));
        (v.e, v.n)
    };
    for l in layers {
        out.extent = out.extent.max(l.extent);
        let m_per_unit = tile_size_m(tile_lonlat(tx as f64 + 0.5, ty as f64 + 0.5, z).1, z) / l.extent as f64;
        if (l.name == "buildings" && !buildings) || (l.name != "buildings" && !base) {
            continue;
        }
        for f in &l.features {
            let kind = f.str("kind").unwrap_or("");
            match (l.name.as_str(), f.kind) {
                ("landuse", GeomType::Polygon) | ("water", GeomType::Polygon) => {
                    let class = if l.name == "water" { Some((LandCover::Water, WATER_PRIORITY)) } else { landuse_class(kind) };
                    if let Some((class, priority)) = class {
                        let rings = f.parts.iter().map(|r| r.iter().map(|&p| {
                            let (e, n) = enu(l, p);
                            to_grid(e, n)
                        }).collect()).collect();
                        out.shapes.push(Shape { class, priority, rings });
                    }
                }
                ("buildings", GeomType::Polygon) => {
                    // The footprint at its mapped height, for sight lines.
                    // Mapped, else by footprint: houses low, blocks and halls higher.
                    let area: f64 = f.parts.iter().map(|r| ring_area(r)).sum::<f64>().abs() * m_per_unit * m_per_unit;
                    let height = f.props.get("height").and_then(mvt::Value::as_f64).filter(|h| *h > 0.0).unwrap_or_else(|| default_height(area));
                    let rings: Vec<Vec<[f64; 2]>> = f.parts.iter().map(|r| r.iter().map(|&p| {
                        let (e, n) = enu(l, p);
                        to_grid(e, n)
                    }).collect()).collect();
                    out.footprints.push((rings, height as f32));
                    for r in &f.parts {
                        let area = ring_area(r).abs() * m_per_unit * m_per_unit;
                        if area < 4.0 || r.is_empty() {
                            continue;
                        }
                        let (cx, cy) = r.iter().fold((0.0, 0.0), |(a, b), p| (a + p[0], b + p[1]));
                        let (e, n) = enu(l, [cx / r.len() as f64, cy / r.len() as f64]);
                        let g = to_grid(e, n);
                        out.buildings.push((g[0], g[1], area));
                    }
                }
                ("water", GeomType::Line) => {
                    if let Some((width, depth)) = river_size(kind) {
                        let ext = l.extent as f64;
                        for part in &f.parts {
                            // Only the piece inside this tile, so neighbours do not double up.
                            for piece in crate::fill::clip_line(part, 0.0, 0.0, ext, ext) {
                                out.rivers.push(River { name: f.str("name").unwrap_or(kind).to_string(), line: piece.iter().map(|&p| enu(l, p)).collect(), width_m: width, depth_m: depth, current_m_s: 0.5 });
                            }
                        }
                    }
                }
                ("places", GeomType::Point) => {
                    // Protomaps v4: settlements are `locality`, their size in `kind_detail`.
                    if kind != "locality" {
                        continue;
                    }
                    let radius = match f.str("kind_detail").unwrap_or("") {
                        "city" => 4_000.0,
                        "town" => 1_500.0,
                        "village" => 600.0,
                        "hamlet" | "isolated_dwelling" | "farm" => 250.0,
                        _ => 800.0,
                    };
                    if let (Some(name), Some(p)) = (f.str("name"), f.parts.first().and_then(|p| p.first())) {
                        let (e, n) = enu(l, *p);
                        out.towns.push(Town { name: name.to_string(), e, n, radius_m: radius, road_node: 0 });
                    }
                }
                ("roads", GeomType::Line) => {
                    let ext = l.extent as f64;
                    let (ox, oy) = (tx as f64 * ext, ty as f64 * ext);
                    for part in &f.parts {
                        for piece in crate::fill::clip_line(part, 0.0, 0.0, ext, ext) {
                            out.roads.push(RawRoad {
                                kind: kind.to_string(),
                                detail: f.str("kind_detail").unwrap_or("").to_string(),
                                bridge: f.flag("is_bridge"),
                                tunnel: f.flag("is_tunnel"),
                                link: f.flag("is_link"),
                                name: f.str("name").map(str::to_string),
                                line: piece.iter().map(|p| [p[0] + ox, p[1] + oy]).collect(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_classes() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_765), (2026, 11, 8));
        assert_eq!(landuse_class("wood").map(|c| c.0), Some(LandCover::ForestDense));
        assert_eq!(landuse_class("farmland").map(|c| c.0), Some(LandCover::Crops));
        assert!(landuse_class("unknown").is_none());
        // Water outranks everything painted on land.
        assert!(LandCover::ALL.iter().all(|_| landuse_class("residential").unwrap().1 < WATER_PRIORITY));
    }
}
